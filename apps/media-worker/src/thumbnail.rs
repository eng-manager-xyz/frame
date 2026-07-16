use std::{
    env, fmt, fs,
    path::Path,
    time::{Duration, Instant},
};
#[cfg(test)]
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

use frame_media::{
    CancellationToken, RuntimeDiagnostics, diagnose_runtime,
    pipeline_has_trusted_factory_provenance, runtime_manifest,
};
use gst::prelude::*;
use gstreamer as gst;
#[cfg(test)]
use uuid::Uuid;

use crate::protocol::{MAX_OUTPUT_BYTES, MAX_SOURCE_BYTES};

const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(10);
const PIPELINE_TIMEOUT: Duration = Duration::from_secs(45);
const BUS_POLL_MS: u64 = 50;
const MAX_DURATION_NS: u64 = 4 * 60 * 60 * 1_000_000_000;
const MAX_WIDTH: i32 = 7_680;
const MAX_HEIGHT: i32 = 4_320;
const MAX_PIXELS: i64 = 7_680_i64 * 4_320_i64;
const THUMBNAIL_FACTORIES: [&str; 8] = [
    "filesrc",
    "decodebin",
    "queue",
    "videoconvert",
    "videoscale",
    "pngenc",
    "fakesink",
    "filesink",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThumbnailError {
    InvalidInput,
    MissingRuntime,
    Pipeline,
    Timeout,
    Cancelled,
    ResourceLimit,
    #[allow(dead_code)]
    InvalidOutput,
}

impl fmt::Display for ThumbnailError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput => "the native thumbnail input is invalid",
            Self::MissingRuntime => "the native thumbnail runtime is unavailable",
            Self::Pipeline => "the native thumbnail pipeline failed",
            Self::Timeout => "the native thumbnail pipeline timed out",
            Self::Cancelled => "the native thumbnail pipeline was cancelled",
            Self::ResourceLimit => "the native thumbnail operation exceeded a resource limit",
            Self::InvalidOutput => "the native thumbnail output is invalid",
        })
    }
}

impl std::error::Error for ThumbnailError {}

pub(crate) struct ReadyGstreamerRuntime {
    thumbnail_factories_missing: usize,
}

pub(crate) fn thumbnail_runtime_capability(
    diagnostics: &RuntimeDiagnostics,
) -> Option<ReadyGstreamerRuntime> {
    #[cfg(not(test))]
    if !thumbnail_sandbox_supported() || !thumbnail_sandbox_ready() {
        return None;
    }
    if !diagnostics.is_ready() {
        return None;
    }
    let manifest = runtime_manifest();
    let thumbnail_factories_missing = THUMBNAIL_FACTORIES
        .iter()
        .filter(|required| {
            !manifest
                .factories
                .iter()
                .any(|declared| declared.factory == **required)
                || !diagnostics
                    .factories
                    .iter()
                    .any(|factory| factory.factory == **required && factory.available)
        })
        .count();
    Some(ReadyGstreamerRuntime {
        thumbnail_factories_missing,
    })
}

pub(crate) const fn thumbnail_factory_count() -> usize {
    THUMBNAIL_FACTORIES.len()
}

pub(crate) fn ensure_thumbnail_runtime() -> Result<(), ThumbnailError> {
    let diagnostics = diagnose_runtime();
    let runtime =
        thumbnail_runtime_capability(&diagnostics).ok_or(ThumbnailError::MissingRuntime)?;
    if thumbnail_runtime_missing_factories(&runtime) != 0 {
        return Err(ThumbnailError::MissingRuntime);
    }
    Ok(())
}

pub(crate) const fn thumbnail_runtime_missing_factories(runtime: &ReadyGstreamerRuntime) -> usize {
    runtime.thumbnail_factories_missing
}

#[cfg(all(not(test), unix))]
fn thumbnail_sandbox_ready() -> bool {
    let shell = fs::metadata("/bin/sh").is_ok_and(|metadata| metadata.is_file());
    let ps = fs::metadata("/bin/ps").is_ok_and(|metadata| metadata.is_file())
        && resident_set_bytes(std::process::id()).is_some();
    shell && ps
}

const fn thumbnail_sandbox_supported() -> bool {
    cfg!(unix)
}

#[cfg(all(not(test), not(unix)))]
const fn thumbnail_sandbox_ready() -> bool {
    false
}

#[cfg(test)]
pub(crate) fn render_thumbnail_v1(
    source: &[u8],
    output_limit: u64,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, ThumbnailError> {
    if source.is_empty() || source.len() as u64 > MAX_SOURCE_BYTES {
        return Err(ThumbnailError::InvalidInput);
    }
    if output_limit == 0 || output_limit > MAX_OUTPUT_BYTES {
        return Err(ThumbnailError::ResourceLimit);
    }
    if cancellation.is_cancelled() {
        return Err(ThumbnailError::Cancelled);
    }

    let scratch = ScratchDirectory::create()?;
    let source_path = scratch.path().join("source.media");
    let output_path = scratch.path().join("thumbnail.png");
    write_source(&source_path, source)?;
    run_child_pipeline(&source_path, &output_path, output_limit, cancellation)?;
    read_output(&output_path, output_limit)
}

#[cfg(all(not(test), unix))]
fn resident_set_bytes(process_id: u32) -> Option<u64> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &process_id.to_string()])
        .env_clear()
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 64 {
        return None;
    }
    let kibibytes = std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1_024)
}

pub(crate) fn run_thumbnail_child(
    source_path: &Path,
    output_path: &Path,
    output_limit: u64,
) -> Result<(), ThumbnailError> {
    #[cfg(not(test))]
    if !thumbnail_sandbox_supported() || !thumbnail_sandbox_ready() {
        return Err(ThumbnailError::MissingRuntime);
    }
    validate_child_paths(source_path, output_path)?;
    run_child_pipeline(
        source_path,
        output_path,
        output_limit,
        &CancellationToken::new(),
    )
}

fn run_child_pipeline(
    source_path: &Path,
    output_path: &Path,
    output_limit: u64,
    cancellation: &CancellationToken,
) -> Result<(), ThumbnailError> {
    if output_limit == 0 || output_limit > MAX_OUTPUT_BYTES {
        return Err(ThumbnailError::ResourceLimit);
    }
    ensure_thumbnail_runtime()?;
    preflight_source(source_path, cancellation)?;
    run_pipeline(source_path, output_path, cancellation)
}

fn validate_child_paths(source_path: &Path, output_path: &Path) -> Result<(), ThumbnailError> {
    if source_path.file_name().and_then(|name| name.to_str()) != Some("source.media")
        || output_path.file_name().and_then(|name| name.to_str()) != Some("thumbnail.png")
        || output_path.exists()
    {
        return Err(ThumbnailError::InvalidInput);
    }
    let source_metadata =
        fs::symlink_metadata(source_path).map_err(|_| ThumbnailError::InvalidInput)?;
    if !source_metadata.file_type().is_file()
        || source_metadata.file_type().is_symlink()
        || source_metadata.len() == 0
        || source_metadata.len() > MAX_SOURCE_BYTES
    {
        return Err(ThumbnailError::InvalidInput);
    }
    let source = fs::canonicalize(source_path).map_err(|_| ThumbnailError::InvalidInput)?;
    let parent = source.parent().ok_or(ThumbnailError::InvalidInput)?;
    let output_parent = output_path.parent().ok_or(ThumbnailError::InvalidInput)?;
    let output_parent =
        fs::canonicalize(output_parent).map_err(|_| ThumbnailError::InvalidInput)?;
    let temporary_root =
        fs::canonicalize(env::temp_dir()).map_err(|_| ThumbnailError::InvalidInput)?;
    let directory_name = parent
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ThumbnailError::InvalidInput)?;
    let suffix = directory_name
        .strip_prefix("frame-media-worker-job-")
        .ok_or(ThumbnailError::InvalidInput)?;
    if parent != output_parent
        || !parent.starts_with(temporary_root)
        || suffix.len() != 32
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ThumbnailError::InvalidInput);
    }
    let parent_metadata = fs::symlink_metadata(parent).map_err(|_| ThumbnailError::InvalidInput)?;
    if !parent_metadata.file_type().is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(ThumbnailError::InvalidInput);
    }
    Ok(())
}

fn preflight_source(
    source_path: &Path,
    cancellation: &CancellationToken,
) -> Result<(), ThumbnailError> {
    let element = gst::parse::launch(
        "filesrc name=source ! decodebin ! queue max-size-buffers=2 max-size-bytes=0 \
         max-size-time=0 ! videoconvert ! video/x-raw,format=RGBA ! \
         fakesink name=probe sync=false num-buffers=1",
    )
    .map_err(|_| ThumbnailError::InvalidInput)?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| ThumbnailError::Pipeline)?;
    let source = pipeline.by_name("source").ok_or(ThumbnailError::Pipeline)?;
    let probe = pipeline.by_name("probe").ok_or(ThumbnailError::Pipeline)?;
    source.set_property("location", source_path);
    let bus = pipeline.bus().ok_or(ThumbnailError::Pipeline)?;
    if pipeline.set_state(gst::State::Playing).is_err() {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(ThumbnailError::InvalidInput);
    }

    let started = Instant::now();
    let terminal = loop {
        if cancellation.is_cancelled() {
            break Err(ThumbnailError::Cancelled);
        }
        if started.elapsed() >= PREFLIGHT_TIMEOUT {
            break Err(ThumbnailError::Timeout);
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(BUS_POLL_MS),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => break Ok(()),
            gst::MessageView::Error(_) => break Err(ThumbnailError::InvalidInput),
            _ => continue,
        }
    };
    let validated = terminal.and_then(|()| {
        if !pipeline_has_trusted_factory_provenance(&pipeline) {
            return Err(ThumbnailError::MissingRuntime);
        }
        let duration = pipeline
            .query_duration::<gst::ClockTime>()
            .ok_or(ThumbnailError::InvalidInput)?;
        let duration_ns = duration.nseconds();
        let pad = probe
            .static_pad("sink")
            .ok_or(ThumbnailError::InvalidInput)?;
        let caps = pad.current_caps().ok_or(ThumbnailError::InvalidInput)?;
        let structure = caps.structure(0).ok_or(ThumbnailError::InvalidInput)?;
        let width = structure
            .get::<i32>("width")
            .map_err(|_| ThumbnailError::InvalidInput)?;
        let height = structure
            .get::<i32>("height")
            .map_err(|_| ThumbnailError::InvalidInput)?;
        let pixels = i64::from(width)
            .checked_mul(i64::from(height))
            .ok_or(ThumbnailError::ResourceLimit)?;
        if duration_ns == 0
            || duration_ns > MAX_DURATION_NS
            || !(1..=MAX_WIDTH).contains(&width)
            || !(1..=MAX_HEIGHT).contains(&height)
            || pixels > MAX_PIXELS
        {
            return Err(ThumbnailError::ResourceLimit);
        }
        Ok(())
    });
    let teardown = pipeline.set_state(gst::State::Null);
    match (validated, teardown) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(_)) => Err(ThumbnailError::Pipeline),
        (Ok(()), Ok(_)) => Ok(()),
    }
}

fn run_pipeline(
    source_path: &Path,
    output_path: &Path,
    cancellation: &CancellationToken,
) -> Result<(), ThumbnailError> {
    ensure_thumbnail_runtime()?;

    let element = gst::parse::launch(
        "filesrc name=source ! decodebin ! videoconvert ! videoscale ! \
         video/x-raw,width=640,height=360,pixel-aspect-ratio=1/1 ! \
         pngenc snapshot=true ! filesink name=output",
    )
    .map_err(|_| ThumbnailError::Pipeline)?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| ThumbnailError::Pipeline)?;
    let source = pipeline.by_name("source").ok_or(ThumbnailError::Pipeline)?;
    let output = pipeline.by_name("output").ok_or(ThumbnailError::Pipeline)?;
    source.set_property("location", source_path);
    output.set_property("location", output_path);
    let bus = pipeline.bus().ok_or(ThumbnailError::Pipeline)?;
    if pipeline.set_state(gst::State::Playing).is_err() {
        let _ = pipeline.set_state(gst::State::Null);
        return Err(ThumbnailError::Pipeline);
    }

    let started = Instant::now();
    let terminal = loop {
        if cancellation.is_cancelled() {
            break Err(ThumbnailError::Cancelled);
        }
        if started.elapsed() >= PIPELINE_TIMEOUT {
            break Err(ThumbnailError::Timeout);
        }
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(BUS_POLL_MS),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => break Ok(()),
            gst::MessageView::Error(_) => break Err(ThumbnailError::Pipeline),
            _ => continue,
        }
    };
    let terminal = terminal.and_then(|()| {
        pipeline_has_trusted_factory_provenance(&pipeline)
            .then_some(())
            .ok_or(ThumbnailError::MissingRuntime)
    });
    let teardown = pipeline.set_state(gst::State::Null);
    match (terminal, teardown) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(_)) => Err(ThumbnailError::Pipeline),
        (Ok(()), Ok(_)) => Ok(()),
    }
}

#[cfg(test)]
fn write_source(path: &Path, source: &[u8]) -> Result<(), ThumbnailError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| ThumbnailError::ResourceLimit)?;
    file.write_all(source)
        .map_err(|_| ThumbnailError::ResourceLimit)?;
    file.sync_all().map_err(|_| ThumbnailError::ResourceLimit)
}

#[cfg(test)]
fn read_output(path: &Path, output_limit: u64) -> Result<Vec<u8>, ThumbnailError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ThumbnailError::InvalidOutput)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > output_limit
    {
        return Err(ThumbnailError::InvalidOutput);
    }
    let mut file = File::open(path).map_err(|_| ThumbnailError::InvalidOutput)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(output_limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| ThumbnailError::InvalidOutput)?;
    if bytes.is_empty()
        || bytes.len() as u64 > output_limit
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err(ThumbnailError::InvalidOutput);
    }
    Ok(bytes)
}

#[cfg(test)]
struct ScratchDirectory {
    path: PathBuf,
}

#[cfg(test)]
impl ScratchDirectory {
    fn create() -> Result<Self, ThumbnailError> {
        let path = std::env::temp_dir().join(format!(
            "frame-media-worker-job-{}",
            Uuid::now_v7().simple()
        ));
        fs::create_dir(&path).map_err(|_| ThumbnailError::ResourceLimit)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .map_err(|_| ThumbnailError::ResourceLimit)?;
        }
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_operation_never_creates_a_pipeline() {
        let cancellation = CancellationToken::new();
        assert!(cancellation.cancel());
        assert_eq!(
            render_thumbnail_v1(b"not media", 1_024, &cancellation),
            Err(ThumbnailError::Cancelled)
        );
    }

    #[test]
    fn bounds_are_enforced_before_media_parsing() {
        let cancellation = CancellationToken::new();
        assert_eq!(
            render_thumbnail_v1(&[], 1_024, &cancellation),
            Err(ThumbnailError::InvalidInput)
        );
        assert_eq!(
            render_thumbnail_v1(b"x", MAX_OUTPUT_BYTES + 1, &cancellation),
            Err(ThumbnailError::ResourceLimit)
        );
    }

    #[test]
    fn real_gstreamer_profile_produces_a_bounded_png() {
        let fixture = std::env::temp_dir().join(format!(
            "frame-media-worker-fixture-{}.webm",
            Uuid::now_v7().simple()
        ));
        frame_media::record_synthetic_webm(&fixture).expect("synthetic fixture");
        let source = fs::read(&fixture).expect("fixture bytes");
        let _ = fs::remove_file(&fixture);
        let output = render_thumbnail_v1(&source, MAX_OUTPUT_BYTES, &CancellationToken::new())
            .expect("thumbnail");
        assert!(output.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!((1..=MAX_OUTPUT_BYTES as usize).contains(&output.len()));
    }

    #[test]
    fn consumer_sandbox_support_is_an_explicit_platform_contract() {
        assert_eq!(thumbnail_sandbox_supported(), cfg!(unix));
    }
}
