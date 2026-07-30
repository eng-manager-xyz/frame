//! Protected full-product lifecycle evidence from the signed Frame application.
//!
//! Unlike the narrow macOS display driver, this mode launches the real Tauri
//! application and exercises its physical windows, global shortcuts, tray,
//! close interception, monitor placement, and native screen-capture backend.
//! It is unreachable during ordinary startup: the exact command flag, protected
//! runner marker, and per-run CSPRNG token are all required.

use std::{
    collections::BTreeSet,
    env,
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant},
};

use frame_desktop_core::{
    CaptureTargetKind, CaptureTargetSummary, LifecycleAction, NativeCaptureArtifact,
    NativeCaptureStartRequest, NativeDesktopBackend, NativePermissionOutcome,
    NativeRecordingControlRequest, NativeRecordingStopOutcome, NativeTargetSelectionRequest,
    RecorderMode,
};
use frame_media::{CancellationToken, decode_studio_preview_frame};
use ring::rand::{SecureRandom, SystemRandom};
use serde::Serialize;
use tauri::{AppHandle, Manager, Monitor, PhysicalPosition};

use super::{
    MAIN_WINDOW_LABEL, NativeDesktopState, OVERLAY_WINDOW_LABEL, TARGET_PICKER_WINDOW_LABEL,
    all_shortcuts_registered, apply_lifecycle, auxiliary_window_position, handle_shortcut,
    require_window, shell_shortcuts, tray_selection, window_is_visible,
};

const DRIVER_FLAG: &str = "--frame-product-hardware-driver";
const TOKEN_HEX_BYTES: usize = 64;
const SHA256_HEX_BYTES: usize = 64;
const BUNDLE_IDENTIFIER: &str = "xyz.engmanager.frame";
const EVIDENCE_FILE: &str = "evidence.json";
const MAX_MONITORS: usize = 2;
const MAX_EVIDENCE_BYTES: usize = 16 * 1_024;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const WINDOW_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const WINDOW_SETTLE: Duration = Duration::from_millis(300);
const WEBVIEW_SETTLE: Duration = Duration::from_secs(1);
const RECORD_DURATION: Duration = Duration::from_secs(2);
const PREVIEW_POSITION: Duration = Duration::from_secs(1);
const SENTINEL_TOLERANCE: u8 = 8;
const MAX_SENTINEL_PIXELS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HardwareTopology {
    SingleStandard,
    DualMixedScale,
    Rotated,
}

impl HardwareTopology {
    fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "single-standard" => Ok(Self::SingleStandard),
            "dual-mixed-scale" => Ok(Self::DualMixedScale),
            "rotated" => Ok(Self::Rotated),
            _ => Err("hardware topology is unsupported"),
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::SingleStandard => "single-standard",
            Self::DualMixedScale => "dual-mixed-scale",
            Self::Rotated => "rotated",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProductHardwareConfiguration {
    data_root: PathBuf,
    source_sha: String,
    run_id: String,
    topology: HardwareTopology,
    signing_identity: String,
    binary_sha256: String,
    signature_binding_sha256: String,
}

impl ProductHardwareConfiguration {
    pub(super) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

#[derive(Debug, Serialize)]
struct ProductHardwareEvidence<'a> {
    schema_version: u8,
    evidence_class: &'static str,
    capability: &'static str,
    platform: &'static str,
    topology: &'static str,
    adapter: &'static str,
    source_sha: &'a str,
    run_id: &'a str,
    application_id: &'static str,
    signing_identity: &'a str,
    binary_sha256: &'a str,
    signature_binding_sha256: &'a str,
    cases: ProductHardwareCases,
    measurements: TopologyMeasurements,
}

#[derive(Debug, Serialize)]
struct ProductHardwareCases {
    signed_native_application: bool,
    native_capture_adapter: bool,
    three_content_protected_windows: bool,
    global_hotkey_registration_and_handler: bool,
    tray_registration_and_handler: bool,
    overlay_target_picker_close_reopen: bool,
    monitor_relative_window_placement: bool,
    randomized_physical_window_exclusion: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct TopologyMeasurements {
    monitor_count: u16,
    distinct_scale_count: u16,
    rotated_display_count: u16,
}

#[derive(Debug, Clone, Copy)]
struct SentinelColor {
    red: u8,
    green: u8,
    blue: u8,
}

pub(super) fn configuration_if_requested()
-> Option<Result<ProductHardwareConfiguration, &'static str>> {
    let mut arguments = env::args_os();
    let _program = arguments.next();
    if arguments.next().as_deref() != Some(OsStr::new(DRIVER_FLAG)) {
        return None;
    }
    Some(parse_configuration(arguments))
}

fn parse_configuration(
    mut arguments: impl Iterator<Item = OsString>,
) -> Result<ProductHardwareConfiguration, &'static str> {
    require_protected_environment()?;
    let data_root = PathBuf::from(next_value(&mut arguments, "--data-root")?);
    let source_sha = string_value(next_value(&mut arguments, "--source-sha")?)?;
    let run_id = string_value(next_value(&mut arguments, "--run-id")?)?;
    let platform = string_value(next_value(&mut arguments, "--platform")?)?;
    let topology =
        HardwareTopology::parse(&string_value(next_value(&mut arguments, "--topology")?)?)?;
    let signing_identity = string_value(next_value(&mut arguments, "--signing-identity")?)?;
    let binary_sha256 = string_value(next_value(&mut arguments, "--binary-sha256")?)?;
    let signature_binding_sha256 =
        string_value(next_value(&mut arguments, "--signature-binding-sha256")?)?;
    let application_id = string_value(next_value(&mut arguments, "--application-id")?)?;
    if arguments.next().is_some() {
        return Err("unexpected product hardware driver argument");
    }
    if platform != platform_name()
        || application_id != BUNDLE_IDENTIFIER
        || !is_lower_hex(&source_sha, 40)
        || !is_run_id(&run_id)
        || !valid_signing_identity(&signing_identity)
        || !is_lower_hex(&binary_sha256, SHA256_HEX_BYTES)
        || !is_lower_hex(&signature_binding_sha256, SHA256_HEX_BYTES)
    {
        return Err("product hardware driver metadata is malformed");
    }
    validate_data_root(&data_root)?;
    Ok(ProductHardwareConfiguration {
        data_root,
        source_sha,
        run_id,
        topology,
        signing_identity,
        binary_sha256,
        signature_binding_sha256,
    })
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

fn next_value(
    arguments: &mut impl Iterator<Item = OsString>,
    expected_name: &'static str,
) -> Result<OsString, &'static str> {
    if arguments.next().as_deref() != Some(OsStr::new(expected_name)) {
        return Err("product hardware driver argument order is invalid");
    }
    arguments
        .next()
        .ok_or("product hardware driver argument value is missing")
}

fn string_value(value: OsString) -> Result<String, &'static str> {
    value
        .into_string()
        .map_err(|_| "product hardware driver argument is not UTF-8")
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

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

#[cfg(target_os = "macos")]
fn valid_signing_identity(value: &str) -> bool {
    value.len() == 10
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

#[cfg(target_os = "windows")]
fn valid_signing_identity(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
}

#[cfg(target_os = "macos")]
const fn platform_name() -> &'static str {
    "macos"
}

#[cfg(target_os = "windows")]
const fn platform_name() -> &'static str {
    "windows"
}

#[cfg(target_os = "macos")]
const fn adapter_name() -> &'static str {
    "native_macos_display"
}

#[cfg(target_os = "windows")]
const fn adapter_name() -> &'static str {
    "native_windows_display_window_region"
}

pub(super) fn launch(app: AppHandle, configuration: ProductHardwareConfiguration) {
    thread::spawn(move || {
        let result = run(&app, &configuration);
        match result {
            Ok(()) => app.exit(0),
            Err(error) => {
                eprintln!("Frame protected product hardware driver failed: {error}");
                process::exit(1);
            }
        }
    });
}

fn run(app: &AppHandle, configuration: &ProductHardwareConfiguration) -> Result<(), &'static str> {
    wait_until(STARTUP_TIMEOUT, || {
        [
            MAIN_WINDOW_LABEL,
            OVERLAY_WINDOW_LABEL,
            TARGET_PICKER_WINDOW_LABEL,
        ]
        .into_iter()
        .all(|label| app.get_webview_window(label).is_some())
            && app.try_state::<NativeDesktopState>().is_some()
    })?;
    let monitors = app
        .available_monitors()
        .map_err(|_| "monitor enumeration failed")?;
    let (monitor_count, distinct_scale_count) = coarse_monitor_measurements(&monitors)?;
    verify_product_lifecycle(app)?;
    verify_monitor_placement(app, &monitors)?;
    let rotated_display_count = verify_physical_window_exclusion(
        app,
        configuration.topology,
        monitor_count,
        distinct_scale_count,
        &monitors,
    )?;
    let measurements = TopologyMeasurements {
        monitor_count,
        distinct_scale_count,
        rotated_display_count,
    };
    validate_topology(configuration.topology, measurements)?;
    write_evidence(configuration, measurements)
}

fn coarse_monitor_measurements(monitors: &[Monitor]) -> Result<(u16, u16), &'static str> {
    if monitors.is_empty() || monitors.len() > MAX_MONITORS {
        return Err("protected topology must contain one or two monitors");
    }
    let mut scales = BTreeSet::new();
    for monitor in monitors {
        let scale = monitor.scale_factor();
        if !scale.is_finite() || !(0.5..=8.0).contains(&scale) {
            return Err("monitor scale factor is outside the protected bounds");
        }
        #[allow(clippy::cast_possible_truncation)]
        let milli_scale = (scale * 1_000.0).round() as u32;
        scales.insert(milli_scale);
        if monitor.size().width == 0 || monitor.size().height == 0 {
            return Err("monitor dimensions are empty");
        }
    }
    Ok((
        u16::try_from(monitors.len()).map_err(|_| "monitor count overflow")?,
        u16::try_from(scales.len()).map_err(|_| "monitor scale count overflow")?,
    ))
}

fn validate_topology(
    topology: HardwareTopology,
    measurements: TopologyMeasurements,
) -> Result<(), &'static str> {
    let valid = match topology {
        HardwareTopology::SingleStandard => {
            measurements.monitor_count == 1
                && measurements.distinct_scale_count == 1
                && measurements.rotated_display_count == 0
        }
        HardwareTopology::DualMixedScale => {
            measurements.monitor_count == 2
                && measurements.distinct_scale_count == 2
                && measurements.rotated_display_count == 0
        }
        HardwareTopology::Rotated => {
            (1..=2).contains(&measurements.monitor_count) && measurements.rotated_display_count >= 1
        }
    };
    if valid {
        Ok(())
    } else {
        Err("physical monitor topology does not match the requested matrix cell")
    }
}

fn verify_product_lifecycle(app: &AppHandle) -> Result<(), &'static str> {
    let state = app
        .try_state::<NativeDesktopState>()
        .ok_or("native desktop state is unavailable")?;
    if !state.frame_windows_excluded {
        return Err("Frame windows are not capture protected");
    }
    if !all_shortcuts_registered(app) {
        return Err("global shortcuts are not registered");
    }
    if state
        .tray
        .lock()
        .map_err(|_| "tray state lock is poisoned")?
        .is_none()
    {
        return Err("tray is not registered");
    }
    for label in [
        MAIN_WINDOW_LABEL,
        OVERLAY_WINDOW_LABEL,
        TARGET_PICKER_WINDOW_LABEL,
    ] {
        require_window(app, label).map_err(|_| "a product window is unavailable")?;
    }

    for action in [
        LifecycleAction::ShowMainWindow,
        LifecycleAction::ShowOverlay,
        LifecycleAction::ShowTargetPicker,
    ] {
        apply_lifecycle(app, &state, action, MAIN_WINDOW_LABEL)
            .map_err(|_| "show lifecycle action failed")?;
    }
    require_visibility(app, MAIN_WINDOW_LABEL, true)?;
    require_visibility(app, OVERLAY_WINDOW_LABEL, true)?;
    require_visibility(app, TARGET_PICKER_WINDOW_LABEL, true)?;

    let shortcuts = shell_shortcuts();
    for (shortcut, label) in shortcuts.into_iter().zip([
        MAIN_WINDOW_LABEL,
        TARGET_PICKER_WINDOW_LABEL,
        OVERLAY_WINDOW_LABEL,
    ]) {
        handle_shortcut(app, &shortcut);
        require_visibility(app, label, false)?;
        handle_shortcut(app, &shortcut);
        require_visibility(app, label, true)?;
    }

    for id in [
        "frame-show-main",
        "frame-show-target-picker",
        "frame-show-overlay",
    ] {
        let super::TraySelection::Lifecycle(action) =
            tray_selection(id).ok_or("tray action is unavailable")?
        else {
            return Err("tray lifecycle action is invalid");
        };
        apply_lifecycle(app, &state, action, MAIN_WINDOW_LABEL)
            .map_err(|_| "tray lifecycle handler failed")?;
    }

    for label in [
        MAIN_WINDOW_LABEL,
        OVERLAY_WINDOW_LABEL,
        TARGET_PICKER_WINDOW_LABEL,
    ] {
        let window = require_window(app, label).map_err(|_| "product window is unavailable")?;
        window
            .close()
            .map_err(|_| "physical close request failed")?;
        wait_until(WINDOW_TIMEOUT, || {
            app.get_webview_window(label).is_some() && !window_is_visible(app, label)
        })?;
        let action = match label {
            MAIN_WINDOW_LABEL => LifecycleAction::ReopenWindow,
            OVERLAY_WINDOW_LABEL => LifecycleAction::ShowOverlay,
            TARGET_PICKER_WINDOW_LABEL => LifecycleAction::ShowTargetPicker,
            _ => return Err("closed product window label is invalid"),
        };
        apply_lifecycle(app, &state, action, label)
            .map_err(|_| "closed product window did not reopen")?;
        require_visibility(app, label, true)?;
    }
    Ok(())
}

fn verify_monitor_placement(app: &AppHandle, monitors: &[Monitor]) -> Result<(), &'static str> {
    let main =
        require_window(app, MAIN_WINDOW_LABEL).map_err(|_| "main product window is unavailable")?;
    let overlay = require_window(app, OVERLAY_WINDOW_LABEL)
        .map_err(|_| "overlay product window is unavailable")?;
    let picker = require_window(app, TARGET_PICKER_WINDOW_LABEL)
        .map_err(|_| "target-picker product window is unavailable")?;
    let state = app
        .try_state::<NativeDesktopState>()
        .ok_or("native desktop state is unavailable")?;

    for monitor in monitors {
        let origin = monitor.position();
        main.set_position(PhysicalPosition::new(
            origin.x.saturating_add(32),
            origin.y.saturating_add(32),
        ))
        .map_err(|_| "main window monitor move failed")?;
        thread::sleep(WINDOW_SETTLE);
        let current = main
            .current_monitor()
            .map_err(|_| "main window monitor lookup failed")?
            .ok_or("main window has no current monitor")?;
        if current.position() != monitor.position() {
            return Err("main window did not enter the requested monitor");
        }
        for (label, window) in [
            (OVERLAY_WINDOW_LABEL, &overlay),
            (TARGET_PICKER_WINDOW_LABEL, &picker),
        ] {
            let action = if label == OVERLAY_WINDOW_LABEL {
                LifecycleAction::ShowOverlay
            } else {
                LifecycleAction::ShowTargetPicker
            };
            apply_lifecycle(app, &state, action, MAIN_WINDOW_LABEL)
                .map_err(|_| "auxiliary window placement action failed")?;
            let window_size = window
                .outer_size()
                .map_err(|_| "auxiliary window size is unavailable")?;
            let expected = auxiliary_window_position(
                label,
                (monitor.position().x, monitor.position().y),
                (monitor.size().width, monitor.size().height),
                (window_size.width, window_size.height),
            )
            .ok_or("auxiliary placement policy rejected a product window")?;
            let actual = window
                .outer_position()
                .map_err(|_| "auxiliary window position is unavailable")?;
            if (actual.x, actual.y) != expected {
                return Err("auxiliary window placement drifted from monitor-relative policy");
            }
        }
    }
    Ok(())
}

fn verify_physical_window_exclusion(
    app: &AppHandle,
    topology: HardwareTopology,
    monitor_count: u16,
    distinct_scale_count: u16,
    monitors: &[Monitor],
) -> Result<u16, &'static str> {
    let colors = generate_sentinel_colors()?;
    install_sentinel(
        app,
        OVERLAY_WINDOW_LABEL,
        colors[0],
        monitors.first().ok_or("primary monitor is unavailable")?,
        (64, 64),
    )?;
    let picker_monitor = if monitors.len() == 2 {
        &monitors[1]
    } else {
        &monitors[0]
    };
    install_sentinel(
        app,
        TARGET_PICKER_WINDOW_LABEL,
        colors[1],
        picker_monitor,
        (96, 96),
    )?;
    thread::sleep(WEBVIEW_SETTLE);

    let state = app
        .try_state::<NativeDesktopState>()
        .ok_or("native desktop state is unavailable")?;
    #[cfg(target_os = "macos")]
    let backend = state
        .native_backend
        .as_ref()
        .ok_or("native macOS backend is unavailable")?;
    #[cfg(target_os = "windows")]
    let backend = state
        .native_backend
        .as_ref()
        .ok_or("native Windows backend is unavailable")?;
    let mut backend = backend
        .lock()
        .map_err(|_| "native backend lock is poisoned")?;
    let (artifact, rotated_display_count) =
        capture_exclusion_frame(&mut *backend, topology, monitor_count, distinct_scale_count)?;
    let preview = decode_studio_preview_frame(
        Path::new(&artifact.media_path),
        PREVIEW_POSITION,
        &CancellationToken::new(),
    )
    .map_err(|_| "captured display frame could not be decoded")?;
    if preview.width != 320 || preview.height != 180 || preview.rgb.len() != 320 * 180 * 3 {
        return Err("decoded exclusion frame has an invalid shape");
    }
    for color in colors {
        if sentinel_pixel_count(&preview.rgb, color) > MAX_SENTINEL_PIXELS {
            return Err("a randomized Frame window sentinel leaked into screen capture");
        }
    }
    Ok(rotated_display_count)
}

fn generate_sentinel_colors() -> Result<[SentinelColor; 2], &'static str> {
    let random = SystemRandom::new();
    let mut bytes = [0_u8; 6];
    random
        .fill(&mut bytes)
        .map_err(|_| "sentinel CSPRNG failed")?;
    let mapped = bytes.map(|value| 32_u8.saturating_add(value % 192));
    let first = SentinelColor {
        red: mapped[0],
        green: mapped[1],
        blue: mapped[2],
    };
    let mut second = SentinelColor {
        red: mapped[3],
        green: mapped[4],
        blue: mapped[5],
    };
    if color_distance(first, second) < 96 {
        second = SentinelColor {
            red: 255_u8.saturating_sub(first.red),
            green: 255_u8.saturating_sub(first.green),
            blue: 255_u8.saturating_sub(first.blue),
        };
    }
    Ok([first, second])
}

fn color_distance(left: SentinelColor, right: SentinelColor) -> u16 {
    u16::from(left.red.abs_diff(right.red))
        + u16::from(left.green.abs_diff(right.green))
        + u16::from(left.blue.abs_diff(right.blue))
}

fn install_sentinel(
    app: &AppHandle,
    label: &str,
    color: SentinelColor,
    monitor: &Monitor,
    offset: (i32, i32),
) -> Result<(), &'static str> {
    let window = require_window(app, label).map_err(|_| "sentinel window is unavailable")?;
    let script = format!(
        "(()=>{{const apply=()=>{{document.documentElement.style.background='rgb({},{},{})';\
         document.documentElement.style.color='rgb({},{},{})';\
         if(document.body){{document.body.replaceChildren();document.body.style.margin='0';\
         document.body.style.minHeight='100vh';document.body.style.background='rgb({},{},{})';}}}};\
         if(document.readyState==='loading'){{addEventListener('DOMContentLoaded',apply,{{once:true}})}}\
         else{{apply()}}}})();",
        color.red,
        color.green,
        color.blue,
        color.red,
        color.green,
        color.blue,
        color.red,
        color.green,
        color.blue,
    );
    window
        .eval(script)
        .map_err(|_| "sentinel WebView injection failed")?;
    window
        .set_always_on_top(true)
        .map_err(|_| "sentinel window could not be raised")?;
    window
        .show()
        .map_err(|_| "sentinel window could not be shown")?;
    window
        .set_position(PhysicalPosition::new(
            monitor.position().x.saturating_add(offset.0),
            monitor.position().y.saturating_add(offset.1),
        ))
        .map_err(|_| "sentinel window could not be positioned")?;
    Ok(())
}

fn capture_exclusion_frame<B: NativeDesktopBackend>(
    backend: &mut B,
    topology: HardwareTopology,
    monitor_count: u16,
    distinct_scale_count: u16,
) -> Result<(NativeCaptureArtifact, u16), &'static str> {
    if backend
        .prepare_capture()
        .map_err(|_| "native permission check failed")?
        != NativePermissionOutcome::Granted
    {
        return Err("native screen permission is not granted");
    }
    let catalog = backend
        .enumerate_targets()
        .map_err(|_| "display enumeration failed")?;
    catalog
        .validate_enumeration()
        .map_err(|_| "display catalog is invalid")?;
    let displays = catalog
        .targets
        .iter()
        .filter(|target| target.kind == CaptureTargetKind::Display)
        .collect::<Vec<_>>();
    let rotated_display_count = u16::try_from(
        displays
            .iter()
            .filter(|target| matches!(target.rotation_degrees, 90 | 270))
            .count(),
    )
    .map_err(|_| "rotated display count overflow")?;
    validate_topology(
        topology,
        TopologyMeasurements {
            monitor_count,
            distinct_scale_count,
            rotated_display_count,
        },
    )?;
    if displays.len() != usize::from(monitor_count) {
        return Err("native display catalog disagrees with the window-system monitor count");
    }
    let display = displays
        .first()
        .copied()
        .cloned()
        .ok_or("no display capture target is available")?;
    select_target(backend, catalog.generation, &display)?;
    let recording = start_recording(backend, catalog.generation, display)?;
    thread::sleep(RECORD_DURATION);
    if backend
        .poll_recording_terminal_failure(&NativeRecordingControlRequest {
            recording_token: recording.clone(),
        })
        .map_err(|_| "recording health probe failed")?
        .is_some()
    {
        return Err("recording failed before exclusion validation");
    }
    let artifact = match backend
        .stop_recording(&NativeRecordingControlRequest {
            recording_token: recording.clone(),
        })
        .map_err(|_| "display recording stop failed")?
    {
        NativeRecordingStopOutcome::Sealed(artifact)
            if artifact.recording_token == recording
                && artifact.duration_ms >= 500
                && artifact.bytes_written > 0 =>
        {
            artifact
        }
        NativeRecordingStopOutcome::Sealed(_) | NativeRecordingStopOutcome::Failed(_) => {
            return Err("display recording did not seal a valid artifact");
        }
    };
    let media = Path::new(&artifact.media_path);
    let metadata =
        fs::symlink_metadata(media).map_err(|_| "captured display artifact is unavailable")?;
    if !media.is_absolute()
        || media.extension() != Some(OsStr::new("webm"))
        || metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() != artifact.bytes_written
    {
        return Err("captured display artifact is invalid");
    }
    Ok((artifact, rotated_display_count))
}

fn select_target<B: NativeDesktopBackend>(
    backend: &mut B,
    catalog_generation: u64,
    display: &CaptureTargetSummary,
) -> Result<(), &'static str> {
    let selected = backend
        .select_target(&NativeTargetSelectionRequest {
            catalog_generation,
            target: display.clone(),
        })
        .map_err(|_| "display selection failed")?;
    if selected.catalog_generation != catalog_generation || selected.target_token != display.token {
        return Err("display selection response is invalid");
    }
    Ok(())
}

fn start_recording<B: NativeDesktopBackend>(
    backend: &mut B,
    catalog_generation: u64,
    display: CaptureTargetSummary,
) -> Result<String, &'static str> {
    let expected_target = display.token.clone();
    let outcome = backend
        .start_recording(&NativeCaptureStartRequest {
            catalog_generation,
            target: display,
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

fn sentinel_pixel_count(rgb: &[u8], color: SentinelColor) -> usize {
    rgb.chunks_exact(3)
        .filter(|pixel| {
            pixel[0].abs_diff(color.red) <= SENTINEL_TOLERANCE
                && pixel[1].abs_diff(color.green) <= SENTINEL_TOLERANCE
                && pixel[2].abs_diff(color.blue) <= SENTINEL_TOLERANCE
        })
        .count()
}

fn require_visibility(app: &AppHandle, label: &str, expected: bool) -> Result<(), &'static str> {
    wait_until(WINDOW_TIMEOUT, || window_is_visible(app, label) == expected)
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> Result<(), &'static str> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or("hardware driver deadline overflow")?;
    loop {
        if condition() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("hardware driver condition timed out");
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn write_evidence(
    configuration: &ProductHardwareConfiguration,
    measurements: TopologyMeasurements,
) -> Result<(), &'static str> {
    let evidence = ProductHardwareEvidence {
        schema_version: 1,
        evidence_class: "desktop_product_hardware_matrix_cell",
        capability: "desktop_lifecycle_matrix_v1",
        platform: platform_name(),
        topology: configuration.topology.as_str(),
        adapter: adapter_name(),
        source_sha: &configuration.source_sha,
        run_id: &configuration.run_id,
        application_id: BUNDLE_IDENTIFIER,
        signing_identity: &configuration.signing_identity,
        binary_sha256: &configuration.binary_sha256,
        signature_binding_sha256: &configuration.signature_binding_sha256,
        cases: ProductHardwareCases {
            signed_native_application: true,
            native_capture_adapter: true,
            three_content_protected_windows: true,
            global_hotkey_registration_and_handler: true,
            tray_registration_and_handler: true,
            overlay_target_picker_close_reopen: true,
            monitor_relative_window_placement: true,
            randomized_physical_window_exclusion: true,
        },
        measurements,
    };
    let payload =
        serde_json::to_vec(&evidence).map_err(|_| "hardware evidence serialization failed")?;
    if payload.len() > MAX_EVIDENCE_BYTES {
        return Err("hardware evidence exceeds its size bound");
    }
    let output = configuration.data_root.join(EVIDENCE_FILE);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| "hardware evidence output is unavailable")?;
    file.write_all(&payload)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|_| "hardware evidence write failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_cells_are_exact_and_coarse() {
        assert!(
            validate_topology(
                HardwareTopology::SingleStandard,
                TopologyMeasurements {
                    monitor_count: 1,
                    distinct_scale_count: 1,
                    rotated_display_count: 0,
                }
            )
            .is_ok()
        );
        assert!(
            validate_topology(
                HardwareTopology::DualMixedScale,
                TopologyMeasurements {
                    monitor_count: 2,
                    distinct_scale_count: 2,
                    rotated_display_count: 0,
                }
            )
            .is_ok()
        );
        assert!(
            validate_topology(
                HardwareTopology::Rotated,
                TopologyMeasurements {
                    monitor_count: 2,
                    distinct_scale_count: 1,
                    rotated_display_count: 1,
                }
            )
            .is_ok()
        );
        assert!(
            validate_topology(
                HardwareTopology::DualMixedScale,
                TopologyMeasurements {
                    monitor_count: 2,
                    distinct_scale_count: 1,
                    rotated_display_count: 0,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn sentinel_detection_is_tolerant_but_bounded() {
        let color = SentinelColor {
            red: 90,
            green: 120,
            blue: 180,
        };
        assert_eq!(
            sentinel_pixel_count(&[90, 120, 180, 80, 110, 170], color),
            2
        );
        assert_eq!(sentinel_pixel_count(&[0, 0, 0, 255, 255, 255], color), 0);
    }
}
