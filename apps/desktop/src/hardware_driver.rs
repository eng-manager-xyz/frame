//! Protected-runner display evidence produced by the signed Frame executable.
//!
//! This mode is unreachable during ordinary application startup. The
//! certificate-signed bundle executable must receive an exact command flag,
//! protected-runner marker, and per-run CSPRNG token before it can touch the
//! dedicated data root.

use std::{
    collections::BTreeSet,
    env,
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use frame_desktop_core::{
    CaptureTargetKind, MacOsNativeDesktopBackend, NativeCaptureArtifact, NativeCaptureStartRequest,
    NativeDesktopBackend, NativeEditableWebmExportRequest, NativePermissionOutcome,
    NativeRecordingCancelOutcome, NativeRecordingControlRequest, NativeRecordingStopOutcome,
    NativeTargetSelectionRequest, PathPolicy, PathUse, RecorderMode, RootAccess,
};
use frame_macos_screen_capture::MacOsScreenCaptureSource;
use frame_media::{PermissionPreflight, ScreenSourceInstanceId};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;

const DRIVER_FLAG: &str = "--frame-hardware-driver";
const TOKEN_HEX_BYTES: usize = 64;
const SHA256_HEX_BYTES: usize = 64;
const RECORD_SECONDS: u64 = 2;
const CANCEL_MILLISECONDS: u64 = 500;
const DISCOVERER_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_PROBE_OUTPUT_BYTES: u64 = 64 * 1_024;
const MAX_DIRECTORY_ENTRIES: usize = 256;
const EVIDENCE_FILE: &str = "evidence.json";
const BUNDLE_IDENTIFIER: &str = "xyz.engmanager.frame";

#[derive(Debug)]
struct DriverArguments {
    data_root: PathBuf,
    source_sha: String,
    run_id: String,
    signing_team: String,
    binary_sha256: String,
    designated_requirement_sha256: String,
    bundle_identifier: String,
}

#[derive(Serialize)]
struct HardwareEvidence<'a> {
    schema_version: u8,
    evidence_class: &'static str,
    full_product_gate: &'static str,
    capability: &'static str,
    platform: &'static str,
    adapter: &'static str,
    source_sha: &'a str,
    run_id: &'a str,
    bundle_identifier: &'a str,
    signing_team_id: &'a str,
    binary_sha256: &'a str,
    designated_requirement_sha256: &'a str,
    cases: HardwareCases,
}

#[derive(Serialize)]
struct HardwareCases {
    screen_capture_preauthorized: bool,
    display_catalog_and_selection: bool,
    display_capture: bool,
    frame_application_exclusion_filter: bool,
    stop_and_playable_webm: bool,
    export_and_playable_webm: bool,
    cancel_partial_cleanup: bool,
}

pub fn run_if_requested() -> Option<Result<(), &'static str>> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    if arguments.next().as_deref() != Some(OsStr::new(DRIVER_FLAG)) {
        return None;
    }
    Some(run(arguments))
}

fn run(arguments: impl Iterator<Item = OsString>) -> Result<(), &'static str> {
    require_protected_environment()?;
    let arguments = parse_arguments(arguments)?;
    validate_data_root(&arguments.data_root)?;
    preflight_screen_capture_access()?;
    frame_desktop_core::bootstrap_desktop_gstreamer().map_err(|_| "GStreamer bootstrap failed")?;

    let projects = arguments.data_root.join("projects");
    let media = arguments.data_root.join("media");
    let exports = arguments.data_root.join("exports");
    let mut backend = MacOsNativeDesktopBackend::new(&projects, &media, &exports)
        .map_err(|_| "native desktop backend initialization failed")?;
    require_permission(&mut backend)?;
    let (generation, display) = select_first_display(&mut backend)?;
    let first = start_display_recording(&mut backend, generation, display)?;
    thread::sleep(Duration::from_secs(RECORD_SECONDS));
    if backend
        .poll_recording_terminal_failure(&NativeRecordingControlRequest {
            recording_token: first.clone(),
        })
        .map_err(|_| "recording health probe failed")?
        .is_some()
    {
        return Err("recording failed before stop");
    }
    let artifact = stop_recording(&mut backend, &first)?;
    validate_capture_artifact(&artifact)?;
    discover_playable_webm(
        Path::new(&artifact.media_path),
        &arguments.data_root,
        "capture",
    )?;
    export_artifact(&mut backend, &artifact, &media, &exports)?;
    let export_path = artifact
        .editable_webm_output_path
        .as_deref()
        .ok_or("capture artifact omitted export path")?;
    discover_playable_webm(Path::new(export_path), &arguments.data_root, "export")?;

    let recordings = media.join("recordings");
    let staging = exports.join(".frame-staging");
    let recordings_before = directory_snapshot(&recordings, false)?;
    let staging_before = directory_snapshot(&staging, true)?;
    require_permission(&mut backend)?;
    let (generation, display) = select_first_display(&mut backend)?;
    let second = start_display_recording(&mut backend, generation, display)?;
    thread::sleep(Duration::from_millis(CANCEL_MILLISECONDS));
    cancel_recording(&mut backend, &second)?;
    if directory_snapshot(&recordings, false)? != recordings_before
        || directory_snapshot(&staging, true)? != staging_before
    {
        return Err("cancel left an unexpected partial artifact");
    }

    write_evidence(&arguments)
}

fn require_protected_environment() -> Result<(), &'static str> {
    if env::var("FRAME_REAL_HARDWARE").as_deref() != Ok("1") {
        return Err("FRAME_REAL_HARDWARE=1 is required");
    }
    let token =
        env::var("FRAME_HARDWARE_DRIVER_TOKEN").map_err(|_| "hardware driver token is required")?;
    if !is_lower_hex(&token, TOKEN_HEX_BYTES) {
        return Err("hardware driver token is malformed");
    }
    Ok(())
}

fn parse_arguments(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<DriverArguments, &'static str> {
    let data_root = PathBuf::from(next_value(&mut arguments, "--data-root")?);
    let source_sha = string_value(next_value(&mut arguments, "--source-sha")?)?;
    let run_id = string_value(next_value(&mut arguments, "--run-id")?)?;
    let signing_team = string_value(next_value(&mut arguments, "--signing-team")?)?;
    let binary_sha256 = string_value(next_value(&mut arguments, "--binary-sha256")?)?;
    let designated_requirement_sha256 = string_value(next_value(
        &mut arguments,
        "--designated-requirement-sha256",
    )?)?;
    let bundle_identifier = string_value(next_value(&mut arguments, "--bundle-identifier")?)?;
    if arguments.next().is_some() {
        return Err("unexpected hardware driver argument");
    }
    if !is_lower_hex(&source_sha, 40)
        || !is_run_id(&run_id)
        || !is_signing_team(&signing_team)
        || !is_lower_hex(&binary_sha256, SHA256_HEX_BYTES)
        || !is_lower_hex(&designated_requirement_sha256, SHA256_HEX_BYTES)
        || bundle_identifier != BUNDLE_IDENTIFIER
    {
        return Err("hardware driver metadata is malformed");
    }
    Ok(DriverArguments {
        data_root,
        source_sha,
        run_id,
        signing_team,
        binary_sha256,
        designated_requirement_sha256,
        bundle_identifier,
    })
}

fn next_value(
    arguments: &mut impl Iterator<Item = OsString>,
    expected_name: &'static str,
) -> Result<OsString, &'static str> {
    if arguments.next().as_deref() != Some(OsStr::new(expected_name)) {
        return Err("hardware driver argument order is invalid");
    }
    arguments
        .next()
        .ok_or("hardware driver argument value is missing")
}

fn string_value(value: OsString) -> Result<String, &'static str> {
    value
        .into_string()
        .map_err(|_| "hardware driver argument is not UTF-8")
}

fn validate_data_root(root: &Path) -> Result<(), &'static str> {
    if !root.is_absolute() || root == Path::new("/") {
        return Err("hardware data root must be absolute and non-root");
    }
    let metadata = fs::symlink_metadata(root).map_err(|_| "hardware data root is unavailable")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("hardware data root must be a non-symlink directory");
    }
    if root.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) || root
        .canonicalize()
        .map_err(|_| "hardware data root cannot be resolved")?
        != root
    {
        return Err("hardware data root must be canonical");
    }
    Ok(())
}

fn preflight_screen_capture_access() -> Result<(), &'static str> {
    let random = SystemRandom::new();
    let mut source_id = [0_u8; 16];
    let mut secret = [0_u8; 32];
    random
        .fill(&mut source_id)
        .and_then(|()| random.fill(&mut secret))
        .map_err(|_| "screen preflight CSPRNG failed")?;
    let source_id =
        ScreenSourceInstanceId::new(source_id).map_err(|_| "screen source identity is invalid")?;
    let mut source = MacOsScreenCaptureSource::new(source_id, secret)
        .map_err(|_| "screen preflight source failed")?;
    if source.preflight_permission() != PermissionPreflight::Granted {
        return Err("screen capture was not preauthorized for the signed Frame identity");
    }
    Ok(())
}

fn require_permission(backend: &mut MacOsNativeDesktopBackend) -> Result<(), &'static str> {
    if backend
        .prepare_capture()
        .map_err(|_| "native permission check failed")?
        != NativePermissionOutcome::Granted
    {
        return Err("native screen permission is not granted");
    }
    Ok(())
}

fn select_first_display(
    backend: &mut MacOsNativeDesktopBackend,
) -> Result<(u64, frame_desktop_core::CaptureTargetSummary), &'static str> {
    let catalog = backend
        .enumerate_targets()
        .map_err(|_| "display enumeration failed")?;
    catalog
        .validate_enumeration()
        .map_err(|_| "display catalog is invalid")?;
    let display = catalog
        .targets
        .iter()
        .find(|target| target.kind == CaptureTargetKind::Display)
        .cloned()
        .ok_or("no display target is available")?;
    let selected = backend
        .select_target(&NativeTargetSelectionRequest {
            catalog_generation: catalog.generation,
            target: display.clone(),
        })
        .map_err(|_| "display selection failed")?;
    if selected.catalog_generation != catalog.generation || selected.target_token != display.token {
        return Err("display selection response is invalid");
    }
    Ok((catalog.generation, display))
}

fn start_display_recording(
    backend: &mut MacOsNativeDesktopBackend,
    catalog_generation: u64,
    target: frame_desktop_core::CaptureTargetSummary,
) -> Result<String, &'static str> {
    let expected_target = target.token.clone();
    let outcome = backend
        .start_recording(&NativeCaptureStartRequest {
            catalog_generation,
            target,
            mode: RecorderMode::Instant,
            frame_rate: 30,
            exclude_frame_windows: true,
            system_audio_enabled: false,
            microphone_enabled: false,
            camera_enabled: false,
        })
        .map_err(|_| "display recording failed to start")?;
    if outcome.catalog_generation != catalog_generation
        || outcome.target_token != expected_target
        || outcome.recording_token.is_empty()
        || outcome.system_audio_included
        || outcome.microphone_included
        || outcome.camera_included
    {
        return Err("display recording start response is invalid");
    }
    Ok(outcome.recording_token)
}

fn stop_recording(
    backend: &mut MacOsNativeDesktopBackend,
    recording_token: &str,
) -> Result<NativeCaptureArtifact, &'static str> {
    match backend
        .stop_recording(&NativeRecordingControlRequest {
            recording_token: recording_token.to_owned(),
        })
        .map_err(|_| "display recording stop failed")?
    {
        NativeRecordingStopOutcome::Sealed(artifact)
            if artifact.recording_token == recording_token =>
        {
            Ok(artifact)
        }
        NativeRecordingStopOutcome::Sealed(_) | NativeRecordingStopOutcome::Failed(_) => {
            Err("display recording did not seal")
        }
    }
}

fn validate_capture_artifact(artifact: &NativeCaptureArtifact) -> Result<(), &'static str> {
    if artifact.artifact_revision == 0
        || artifact.duration_ms < 500
        || artifact.bytes_written == 0
        || artifact.studio_project_path.is_some()
    {
        return Err("sealed capture artifact is incomplete");
    }
    validate_regular_webm(Path::new(&artifact.media_path), artifact.bytes_written)?;
    let export = artifact
        .editable_webm_output_path
        .as_deref()
        .ok_or("sealed capture has no export path")?;
    if Path::new(export).extension() != Some(OsStr::new("webm")) {
        return Err("sealed capture export is not WebM");
    }
    Ok(())
}

fn export_artifact(
    backend: &mut MacOsNativeDesktopBackend,
    artifact: &NativeCaptureArtifact,
    media_root: &Path,
    export_root: &Path,
) -> Result<(), &'static str> {
    let policy = PathPolicy::empty()
        .allow_root(
            media_root,
            RootAccess {
                read: true,
                write: false,
                delete: false,
            },
        )
        .and_then(|policy| {
            policy.allow_root(
                export_root,
                RootAccess {
                    read: false,
                    write: true,
                    delete: false,
                },
            )
        })
        .map_err(|_| "artifact path policy is invalid")?;
    let source = policy
        .validate(&artifact.media_path, PathUse::MediaRead)
        .map_err(|_| "capture media escaped its root")?;
    let export_text = artifact
        .editable_webm_output_path
        .as_deref()
        .ok_or("capture export path is absent")?;
    let output = policy
        .validate(export_text, PathUse::ExportWrite)
        .map_err(|_| "capture export escaped its root")?;
    let result = backend
        .export_editable_webm(&NativeEditableWebmExportRequest {
            artifact_token: artifact.artifact_token.clone(),
            artifact_revision: artifact.artifact_revision,
            source_media_path: source,
            output_path: output,
        })
        .map_err(|_| "editable WebM export failed")?;
    if result.artifact_token != artifact.artifact_token
        || result.artifact_revision != artifact.artifact_revision
        || result.bytes_written != artifact.bytes_written
    {
        return Err("editable WebM export response is invalid");
    }
    validate_regular_webm(Path::new(export_text), result.bytes_written)
}

fn validate_regular_webm(path: &Path, expected_bytes: u64) -> Result<(), &'static str> {
    if !path.is_absolute() || path.extension() != Some(OsStr::new("webm")) {
        return Err("media artifact path is invalid");
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "media artifact is unavailable")?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != expected_bytes
        || expected_bytes == 0
    {
        return Err("media artifact identity or size is invalid");
    }
    Ok(())
}

fn discover_playable_webm(media: &Path, data_root: &Path, label: &str) -> Result<(), &'static str> {
    let probe_path = data_root.join(format!(".{label}-discoverer.txt"));
    let probe = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
        .map_err(|_| "cannot create bounded discoverer output")?;
    let error_probe = probe
        .try_clone()
        .map_err(|_| "cannot duplicate discoverer output")?;
    let mut child = Command::new("gst-discoverer-1.0")
        .arg(media)
        .stdin(Stdio::null())
        .stdout(Stdio::from(probe))
        .stderr(Stdio::from(error_probe))
        .spawn()
        .map_err(|_| "cannot launch gst-discoverer-1.0")?;
    let deadline = Instant::now()
        .checked_add(DISCOVERER_TIMEOUT)
        .ok_or("discoverer deadline overflow")?;
    let status = loop {
        match child
            .try_wait()
            .map_err(|_| "cannot poll gst-discoverer-1.0")?
        {
            Some(status) => break status,
            None if Instant::now() < deadline => thread::sleep(PROCESS_POLL_INTERVAL),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&probe_path);
                return Err("gst-discoverer-1.0 timed out");
            }
        }
    };
    let result = read_probe_result(&probe_path, status.success());
    let _ = fs::remove_file(&probe_path);
    result
}

fn read_probe_result(path: &Path, success: bool) -> Result<(), &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "discoverer output is unavailable")?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_PROBE_OUTPUT_BYTES
    {
        return Err("discoverer output is invalid");
    }
    let mut output = String::new();
    File::open(path)
        .and_then(|file| {
            file.take(MAX_PROBE_OUTPUT_BYTES)
                .read_to_string(&mut output)
        })
        .map_err(|_| "discoverer output is unreadable")?;
    let normalized = output.to_ascii_lowercase();
    if !success || !normalized.contains("duration:") || !normalized.contains("video") {
        return Err("GStreamer did not discover playable video");
    }
    Ok(())
}

fn cancel_recording(
    backend: &mut MacOsNativeDesktopBackend,
    recording_token: &str,
) -> Result<(), &'static str> {
    match backend
        .cancel_recording(&NativeRecordingControlRequest {
            recording_token: recording_token.to_owned(),
        })
        .map_err(|_| "display recording cancel failed")?
    {
        NativeRecordingCancelOutcome::Cancelled {
            recording_token: observed,
        } if observed == recording_token => Ok(()),
        NativeRecordingCancelOutcome::Cancelled { .. }
        | NativeRecordingCancelOutcome::Failed(_) => Err("display recording did not cancel"),
    }
}

fn directory_snapshot(
    path: &Path,
    require_empty: bool,
) -> Result<BTreeSet<OsString>, &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "artifact directory is unavailable")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("artifact directory is not a trusted directory");
    }
    let mut entries = BTreeSet::new();
    for entry in fs::read_dir(path).map_err(|_| "artifact directory cannot be read")? {
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            return Err("artifact directory exceeds its entry bound");
        }
        let entry = entry.map_err(|_| "artifact directory entry is unavailable")?;
        let entry_metadata = entry
            .file_type()
            .map_err(|_| "artifact directory entry type is unavailable")?;
        if entry_metadata.is_symlink() || !entry_metadata.is_file() {
            return Err("artifact directory contains an unexpected entry");
        }
        entries.insert(entry.file_name());
    }
    if require_empty && !entries.is_empty() {
        return Err("artifact staging directory is not empty");
    }
    Ok(entries)
}

fn write_evidence(arguments: &DriverArguments) -> Result<(), &'static str> {
    let evidence = HardwareEvidence {
        schema_version: 1,
        evidence_class: "macos_display_capture_partial",
        full_product_gate: "not_claimed",
        capability: "macos_display_webm_v1",
        platform: "macos",
        adapter: "native_macos_display",
        source_sha: &arguments.source_sha,
        run_id: &arguments.run_id,
        bundle_identifier: &arguments.bundle_identifier,
        signing_team_id: &arguments.signing_team,
        binary_sha256: &arguments.binary_sha256,
        designated_requirement_sha256: &arguments.designated_requirement_sha256,
        cases: HardwareCases {
            screen_capture_preauthorized: true,
            display_catalog_and_selection: true,
            display_capture: true,
            frame_application_exclusion_filter: true,
            stop_and_playable_webm: true,
            export_and_playable_webm: true,
            cancel_partial_cleanup: true,
        },
    };
    let encoded =
        serde_json::to_vec_pretty(&evidence).map_err(|_| "hardware evidence cannot be encoded")?;
    if encoded.len() > 16 * 1_024 {
        return Err("hardware evidence exceeds its size bound");
    }
    let output = arguments.data_root.join(EVIDENCE_FILE);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| "hardware evidence file cannot be created")?;
    file.write_all(&encoded)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|_| "hardware evidence file cannot be committed")
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

fn is_signing_team(value: &str) -> bool {
    value.len() == 10
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{is_lower_hex, is_run_id, is_signing_team};

    #[test]
    fn metadata_validators_are_exact() {
        assert!(is_lower_hex(&"a".repeat(64), 64));
        assert!(!is_lower_hex(&"A".repeat(64), 64));
        assert!(!is_lower_hex(&"a".repeat(63), 64));
        assert!(is_run_id("123456:7"));
        assert!(!is_run_id("../hardware"));
        assert!(is_signing_team("ABCDE12345"));
        assert!(!is_signing_team("abcde12345"));
    }
}
