//! Private output ownership and bounded verification for screen recordings.
//!
//! Unix recording graphs use `fdsink` and `fdsrc` against a retained descriptor.
//! Managed path outputs are still confined to a freshly created mode-0700
//! staging directory beside the final destination. The final pathname is
//! introduced with `hard_link` only after the staging inode has been decoded,
//! counted, timed, hashed, and synced. Cleanup only unlinks that same identified
//! inode inside the private directory.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gst::prelude::*;
use gstreamer as gst;
use same_file::Handle;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{ScreenRecordingError, require_trusted, set_null};
use crate::CancellationToken;

const PRIVATE_DIRECTORY_ATTEMPTS: usize = 8;
const VERIFICATION_TIMEOUT: Duration = Duration::from_secs(120);
const BUS_POLL: Duration = Duration::from_millis(25);
const WEBM_PROBE_BYTES: u64 = 1024 * 1024;
const MINIMUM_WEBM_BYTES: u64 = 128;
const TIMELINE_TOLERANCE_NS: u64 = 2_000_000;

#[cfg(unix)]
pub(super) const VERIFICATION_FACTORIES: &[&str] =
    &["fdsrc", "matroskademux", "identity", "vp8dec", "fakesink"];

#[cfg(not(unix))]
pub(super) const VERIFICATION_FACTORIES: &[&str] =
    &["filesrc", "matroskademux", "identity", "vp8dec", "fakesink"];

#[cfg(unix)]
pub(super) const AV_VERIFICATION_FACTORIES: &[&str] = &[
    "fdsrc",
    "matroskademux",
    "identity",
    "vp8dec",
    "opusdec",
    "fakesink",
];

#[cfg(not(unix))]
pub(super) const AV_VERIFICATION_FACTORIES: &[&str] = &[
    "filesrc",
    "matroskademux",
    "identity",
    "vp8dec",
    "opusdec",
    "fakesink",
];

#[derive(Debug, Clone, Copy)]
pub(super) struct ExpectedVideo {
    pub frames: u64,
    pub duration_ns: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ExpectedAudio {
    pub duration_ns: u64,
}

#[derive(Debug)]
pub(super) struct VerifiedWebm {
    pub bytes: u64,
    pub sha256: Option<String>,
    pub encoded_frames: u64,
    pub first_pts_ns: u64,
    pub end_pts_ns: u64,
    pub encoded_duration_ns: u64,
}

#[derive(Debug)]
pub(super) struct VerifiedAvWebm {
    pub bytes: u64,
    pub sha256: String,
    pub encoded_video_frames: u64,
    pub decoded_audio_buffers: u64,
    pub video_duration_ns: u64,
    pub audio_duration_ns: u64,
}

/// Owns the only directory in which a recording/export writer may create data.
pub(super) struct OutputReservation {
    final_path: PathBuf,
    staging_directory: PathBuf,
    staging_path: PathBuf,
    identity: Option<Handle>,
    sync_file: Option<File>,
    committed: bool,
}

impl std::fmt::Debug for OutputReservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OutputReservation")
            .field("materialized", &self.identity.is_some())
            .field("committed", &self.committed)
            .field("paths", &"<redacted>")
            .finish()
    }
}

impl OutputReservation {
    /// Reserves the staging inode before handing its private pathname to
    /// `filesink`. The retained writable handle is later used for durability.
    pub(super) fn for_filesink(final_path: PathBuf) -> Result<Self, ScreenRecordingError> {
        let mut reservation = Self::new(final_path)?;
        reservation.create_staging_file()?;
        Ok(reservation)
    }

    /// Reserves only the private directory for an existing writer that rejects
    /// pre-existing output files. Call [`Self::adopt_created`] after it returns.
    pub(super) fn for_external_writer(final_path: PathBuf) -> Result<Self, ScreenRecordingError> {
        Self::new(final_path)
    }

    fn new(final_path: PathBuf) -> Result<Self, ScreenRecordingError> {
        if final_path.file_name().is_none() {
            return Err(ScreenRecordingError::InvalidConfiguration);
        }
        reject_existing_path(&final_path)?;
        let parent = final_path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        ensure_output_parent(parent)?;
        let staging_directory = create_private_staging_directory(parent)?;
        let staging_path = staging_directory.join("artifact.webm");
        Ok(Self {
            final_path,
            staging_directory,
            staging_path,
            identity: None,
            sync_file: None,
            committed: false,
        })
    }

    #[must_use]
    pub(super) fn staging_path(&self) -> &Path {
        &self.staging_path
    }

    pub(super) fn retained_file(&self) -> Result<&File, ScreenRecordingError> {
        self.sync_file
            .as_ref()
            .ok_or(ScreenRecordingError::OutputOwnership)
    }

    pub(super) fn retained_file_mut(&mut self) -> Result<&mut File, ScreenRecordingError> {
        self.sync_file
            .as_mut()
            .ok_or(ScreenRecordingError::OutputOwnership)
    }

    fn create_staging_file(&mut self) -> Result<(), ScreenRecordingError> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&self.staging_path)
            .map_err(ScreenRecordingError::Filesystem)?;
        self.adopt_file(file)
    }

    /// Captures an identity and writable durability handle for a file created
    /// by a writer inside this reservation's private directory.
    pub(super) fn adopt_created(&mut self) -> Result<(), ScreenRecordingError> {
        if self.identity.is_some() {
            return self.verify_staging_identity();
        }
        let metadata =
            fs::symlink_metadata(&self.staging_path).map_err(ScreenRecordingError::Filesystem)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ScreenRecordingError::OutputOwnership);
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.staging_path)
            .map_err(ScreenRecordingError::Filesystem)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(ScreenRecordingError::Filesystem)?;
            if file
                .metadata()
                .map_err(ScreenRecordingError::Filesystem)?
                .permissions()
                .mode()
                & 0o777
                != 0o600
            {
                return Err(ScreenRecordingError::OutputOwnership);
            }
        }
        self.adopt_file(file)
    }

    fn adopt_file(&mut self, file: File) -> Result<(), ScreenRecordingError> {
        let identity_file = file.try_clone().map_err(ScreenRecordingError::Filesystem)?;
        let identity =
            Handle::from_file(identity_file).map_err(ScreenRecordingError::Filesystem)?;
        self.sync_file = Some(file);
        self.identity = Some(identity);
        self.verify_staging_identity()
    }

    pub(super) fn verify_staging_identity(&self) -> Result<(), ScreenRecordingError> {
        let expected = self
            .identity
            .as_ref()
            .ok_or(ScreenRecordingError::OutputOwnership)?;
        let metadata =
            fs::symlink_metadata(&self.staging_path).map_err(ScreenRecordingError::Filesystem)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ScreenRecordingError::OutputOwnership);
        }
        let actual =
            Handle::from_path(&self.staging_path).map_err(ScreenRecordingError::Filesystem)?;
        if actual != *expected {
            return Err(ScreenRecordingError::OutputOwnership);
        }
        Ok(())
    }

    /// Atomically introduces the verified inode at the caller's destination.
    pub(super) fn commit(&mut self) -> Result<PathBuf, ScreenRecordingError> {
        self.verify_staging_identity()?;
        reject_existing_path(&self.final_path)?;
        fs::hard_link(&self.staging_path, &self.final_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ScreenRecordingError::OutputExists
            } else {
                ScreenRecordingError::Filesystem(error)
            }
        })?;

        if let Err(error) = self.finish_commit() {
            self.remove_final_if_owned();
            return Err(error);
        }
        self.committed = true;
        Ok(self.final_path.clone())
    }

    fn finish_commit(&mut self) -> Result<(), ScreenRecordingError> {
        if !self.path_has_expected_identity(&self.final_path)? {
            return Err(ScreenRecordingError::OutputOwnership);
        }
        self.sync_file
            .as_ref()
            .ok_or(ScreenRecordingError::OutputOwnership)?
            .sync_all()
            .map_err(ScreenRecordingError::Filesystem)?;
        sync_parent_directory(&self.final_path)?;
        self.remove_staging_if_owned()?;
        fs::remove_dir(&self.staging_directory).map_err(ScreenRecordingError::Filesystem)?;
        sync_parent_directory(&self.final_path)?;
        if !self.path_has_expected_identity(&self.final_path)? {
            return Err(ScreenRecordingError::OutputOwnership);
        }
        Ok(())
    }

    fn path_has_expected_identity(&self, path: &Path) -> Result<bool, ScreenRecordingError> {
        let Some(expected) = self.identity.as_ref() else {
            return Ok(false);
        };
        let metadata = fs::symlink_metadata(path).map_err(ScreenRecordingError::Filesystem)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Ok(false);
        }
        let actual = Handle::from_path(path).map_err(ScreenRecordingError::Filesystem)?;
        Ok(actual == *expected)
    }

    fn remove_staging_if_owned(&mut self) -> Result<(), ScreenRecordingError> {
        if !self.path_has_expected_identity(&self.staging_path)? {
            return Err(ScreenRecordingError::OutputOwnership);
        }
        fs::remove_file(&self.staging_path).map_err(ScreenRecordingError::Filesystem)
    }

    fn remove_final_if_owned(&self) {
        if self
            .path_has_expected_identity(&self.final_path)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.final_path);
            let _ = sync_parent_directory(&self.final_path);
        }
    }

    fn cleanup_private_staging(&mut self) {
        if self.identity.is_some() {
            if self
                .path_has_expected_identity(&self.staging_path)
                .unwrap_or(false)
            {
                let _ = fs::remove_file(&self.staging_path);
            }
        } else if fs::symlink_metadata(&self.staging_path).is_ok() {
            // The directory itself is backend-owned and inaccessible to other
            // users. Removing a pathname here cannot follow a symlink target.
            let _ = fs::remove_file(&self.staging_path);
        }
        let _ = fs::remove_dir(&self.staging_directory);
    }
}

impl Drop for OutputReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.remove_final_if_owned();
            self.cleanup_private_staging();
        }
    }
}

pub(super) fn preflight_verification() -> Result<(), ScreenRecordingError> {
    for name in VERIFICATION_FACTORIES {
        if gst::ElementFactory::find(name).is_none() {
            return Err(ScreenRecordingError::MissingFactory);
        }
    }
    let pipeline = build_verifier_pipeline()?;
    require_trusted(&pipeline)
}

pub(super) fn preflight_av_verification() -> Result<(), ScreenRecordingError> {
    for name in AV_VERIFICATION_FACTORIES {
        if gst::ElementFactory::find(name).is_none() {
            return Err(ScreenRecordingError::MissingFactory);
        }
    }
    let pipeline = build_av_verifier_pipeline()?;
    require_trusted(&pipeline)
}

pub(super) fn verify_playable_webm(
    output: &Path,
    cancellation: &CancellationToken,
    expected: Option<ExpectedVideo>,
    compute_hash: bool,
) -> Result<VerifiedWebm, ScreenRecordingError> {
    let metadata = fs::symlink_metadata(output).map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < MINIMUM_WEBM_BYTES
    {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    let mut file = File::open(output).map_err(ScreenRecordingError::Filesystem)?;
    let verified = verify_playable_webm_file_with_path(
        &mut file,
        Some(output),
        cancellation,
        expected,
        compute_hash,
    )?;
    if verified.bytes != metadata.len() {
        return Err(ScreenRecordingError::OutputOwnership);
    }
    Ok(verified)
}

pub(super) fn verify_playable_webm_file(
    file: &mut File,
    cancellation: &CancellationToken,
    expected: Option<ExpectedVideo>,
    compute_hash: bool,
) -> Result<VerifiedWebm, ScreenRecordingError> {
    verify_playable_webm_file_with_path(file, None, cancellation, expected, compute_hash)
}

#[cfg(not(unix))]
pub(super) fn verify_playable_av_webm(
    output: &Path,
    cancellation: &CancellationToken,
    expected_video: ExpectedVideo,
    expected_audio: ExpectedAudio,
) -> Result<VerifiedAvWebm, ScreenRecordingError> {
    let metadata = fs::symlink_metadata(output).map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() < MINIMUM_WEBM_BYTES
    {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    let mut file = File::open(output).map_err(ScreenRecordingError::Filesystem)?;
    let verified = verify_playable_av_webm_file_with_path(
        &mut file,
        Some(output),
        cancellation,
        expected_video,
        expected_audio,
    )?;
    if verified.bytes != metadata.len() {
        return Err(ScreenRecordingError::OutputOwnership);
    }
    Ok(verified)
}

pub(super) fn verify_playable_av_webm_file(
    file: &mut File,
    cancellation: &CancellationToken,
    expected_video: ExpectedVideo,
    expected_audio: ExpectedAudio,
) -> Result<VerifiedAvWebm, ScreenRecordingError> {
    verify_playable_av_webm_file_with_path(file, None, cancellation, expected_video, expected_audio)
}

fn verify_playable_av_webm_file_with_path(
    file: &mut File,
    path: Option<&Path>,
    cancellation: &CancellationToken,
    expected_video: ExpectedVideo,
    expected_audio: ExpectedAudio,
) -> Result<VerifiedAvWebm, ScreenRecordingError> {
    let started = Instant::now();
    check_budget(cancellation, started)?;
    let metadata = file.metadata().map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.len() < MINIMUM_WEBM_BYTES {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    verify_av_webm_markers(file, metadata.len(), cancellation, started)?;
    let encoded = inspect_encoded_av(file, path, cancellation, started)?;
    if encoded.video.frames != expected_video.frames
        || encoded
            .video
            .duration_ns
            .abs_diff(expected_video.duration_ns)
            > TIMELINE_TOLERANCE_NS
        || encoded
            .audio
            .duration_ns
            .abs_diff(expected_audio.duration_ns)
            > 25_000_000
    {
        return Err(ScreenRecordingError::FrameLoss);
    }
    let sha256 = sha256_file(file, cancellation, started)?;
    check_budget(cancellation, started)?;
    let final_metadata = file.metadata().map_err(ScreenRecordingError::Filesystem)?;
    if !final_metadata.file_type().is_file() || final_metadata.len() != metadata.len() {
        return Err(ScreenRecordingError::OutputOwnership);
    }
    Ok(VerifiedAvWebm {
        bytes: metadata.len(),
        sha256,
        encoded_video_frames: encoded.video.frames,
        decoded_audio_buffers: encoded.audio.frames,
        video_duration_ns: encoded.video.duration_ns,
        audio_duration_ns: encoded.audio.duration_ns,
    })
}

fn verify_playable_webm_file_with_path(
    file: &mut File,
    path: Option<&Path>,
    cancellation: &CancellationToken,
    expected: Option<ExpectedVideo>,
    compute_hash: bool,
) -> Result<VerifiedWebm, ScreenRecordingError> {
    let started = Instant::now();
    check_budget(cancellation, started)?;
    let metadata = file.metadata().map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_file() || metadata.len() < MINIMUM_WEBM_BYTES {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    verify_webm_markers(file, metadata.len(), cancellation, started)?;
    let encoded = inspect_encoded_video(file, path, cancellation, started)?;
    if let Some(expected) = expected
        && (encoded.frames != expected.frames
            || encoded.duration_ns.abs_diff(expected.duration_ns) > TIMELINE_TOLERANCE_NS)
    {
        return Err(ScreenRecordingError::FrameLoss);
    }
    let sha256 = compute_hash
        .then(|| sha256_file(file, cancellation, started))
        .transpose()?;
    check_budget(cancellation, started)?;
    let final_metadata = file.metadata().map_err(ScreenRecordingError::Filesystem)?;
    if !final_metadata.file_type().is_file() || final_metadata.len() != metadata.len() {
        return Err(ScreenRecordingError::OutputOwnership);
    }
    Ok(VerifiedWebm {
        bytes: metadata.len(),
        sha256,
        encoded_frames: encoded.frames,
        first_pts_ns: encoded.first_pts_ns,
        end_pts_ns: encoded.end_pts_ns,
        encoded_duration_ns: encoded.duration_ns,
    })
}

fn verify_webm_markers(
    file: &mut File,
    bytes: u64,
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<(), ScreenRecordingError> {
    let probe_len = usize::try_from(bytes.min(WEBM_PROBE_BYTES))
        .map_err(|_| ScreenRecordingError::InvalidOutput)?;
    let mut prefix = vec![0_u8; probe_len];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut prefix))
        .map_err(ScreenRecordingError::Filesystem)?;
    check_budget(cancellation, started)?;
    let has_ebml_header = prefix.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]);
    let has_webm_doctype = prefix.windows(4).any(|window| window == b"webm");
    let has_vp8_track = prefix.windows(5).any(|window| window == b"V_VP8");
    if !has_ebml_header || !has_webm_doctype || !has_vp8_track {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    Ok(())
}

fn verify_av_webm_markers(
    file: &mut File,
    bytes: u64,
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<(), ScreenRecordingError> {
    verify_webm_markers(file, bytes, cancellation, started)?;
    let probe_len = usize::try_from(bytes.min(WEBM_PROBE_BYTES))
        .map_err(|_| ScreenRecordingError::InvalidOutput)?;
    let mut prefix = vec![0_u8; probe_len];
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.read_exact(&mut prefix))
        .map_err(ScreenRecordingError::Filesystem)?;
    check_budget(cancellation, started)?;
    if !prefix.windows(6).any(|window| window == b"A_OPUS") {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    Ok(())
}

#[derive(Default)]
struct EncodedVideoProbe {
    frames: AtomicU64,
    first_pts_ns: AtomicU64,
    end_pts_ns: AtomicU64,
    invalid_timing: AtomicBool,
}

#[derive(Debug, Clone, Copy)]
struct EncodedVideo {
    frames: u64,
    first_pts_ns: u64,
    end_pts_ns: u64,
    duration_ns: u64,
}

#[derive(Debug, Clone, Copy)]
struct EncodedAv {
    video: EncodedVideo,
    audio: EncodedVideo,
}

fn inspect_encoded_video(
    file: &mut File,
    _path: Option<&Path>,
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<EncodedVideo, ScreenRecordingError> {
    preflight_verification()?;
    let pipeline = build_verifier_pipeline()?;
    let source = pipeline
        .by_name("screen_verify_source")
        .ok_or(ScreenRecordingError::Pipeline)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        file.seek(SeekFrom::Start(0))
            .map_err(ScreenRecordingError::Filesystem)?;
        source.set_property("fd", file.as_raw_fd());
    }
    #[cfg(not(unix))]
    {
        source.set_property(
            "location",
            _path.ok_or(ScreenRecordingError::InvalidConfiguration)?,
        );
    }
    require_trusted(&pipeline)?;
    let counter = pipeline
        .by_name("screen_verify_counter")
        .ok_or(ScreenRecordingError::Pipeline)?;
    let pad = counter
        .static_pad("src")
        .ok_or(ScreenRecordingError::Pipeline)?;
    let probe = Arc::new(EncodedVideoProbe {
        first_pts_ns: AtomicU64::new(u64::MAX),
        ..EncodedVideoProbe::default()
    });
    let callback_probe = Arc::clone(&probe);
    pad.add_probe(gst::PadProbeType::BUFFER, move |_, information| {
        if let Some(gst::PadProbeData::Buffer(buffer)) = information.data.as_ref() {
            let _ = callback_probe.frames.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |current| current.checked_add(1),
            );
            match (buffer.pts(), buffer.duration()) {
                (Some(pts), Some(duration)) => {
                    let pts_ns = pts.nseconds();
                    let Some(end_ns) = pts_ns.checked_add(duration.nseconds()) else {
                        callback_probe.invalid_timing.store(true, Ordering::Relaxed);
                        return gst::PadProbeReturn::Ok;
                    };
                    callback_probe
                        .first_pts_ns
                        .fetch_min(pts_ns, Ordering::Relaxed);
                    callback_probe
                        .end_pts_ns
                        .fetch_max(end_ns, Ordering::Relaxed);
                }
                _ => callback_probe.invalid_timing.store(true, Ordering::Relaxed),
            }
        }
        gst::PadProbeReturn::Ok
    })
    .ok_or(ScreenRecordingError::Pipeline)?;

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|_| ScreenRecordingError::Pipeline)?;
    let terminal = wait_for_verifier(&pipeline, cancellation, started);
    let teardown = set_null(&pipeline);
    terminal?;
    teardown?;
    require_trusted(&pipeline)?;

    let frames = probe.frames.load(Ordering::Relaxed);
    let first_pts_ns = probe.first_pts_ns.load(Ordering::Relaxed);
    let end_pts_ns = probe.end_pts_ns.load(Ordering::Relaxed);
    if frames == 0
        || first_pts_ns == u64::MAX
        || end_pts_ns <= first_pts_ns
        || probe.invalid_timing.load(Ordering::Relaxed)
    {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    Ok(EncodedVideo {
        frames,
        first_pts_ns,
        end_pts_ns,
        duration_ns: end_pts_ns - first_pts_ns,
    })
}

fn inspect_encoded_av(
    file: &mut File,
    _path: Option<&Path>,
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<EncodedAv, ScreenRecordingError> {
    preflight_av_verification()?;
    let pipeline = build_av_verifier_pipeline()?;
    let source = pipeline
        .by_name("screen_av_verify_source")
        .ok_or(ScreenRecordingError::Pipeline)?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        file.seek(SeekFrom::Start(0))
            .map_err(ScreenRecordingError::Filesystem)?;
        source.set_property("fd", file.as_raw_fd());
    }
    #[cfg(not(unix))]
    source.set_property(
        "location",
        _path.ok_or(ScreenRecordingError::InvalidConfiguration)?,
    );
    require_trusted(&pipeline)?;
    let video = install_timing_probe(&pipeline, "screen_av_verify_video")?;
    let audio = install_timing_probe(&pipeline, "screen_av_verify_audio")?;

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|_| ScreenRecordingError::Pipeline)?;
    let terminal = wait_for_verifier(&pipeline, cancellation, started);
    let teardown = set_null(&pipeline);
    terminal?;
    teardown?;
    require_trusted(&pipeline)?;

    Ok(EncodedAv {
        video: timing_probe_result(&video)?,
        audio: timing_probe_result(&audio)?,
    })
}

fn install_timing_probe(
    pipeline: &gst::Pipeline,
    element_name: &str,
) -> Result<Arc<EncodedVideoProbe>, ScreenRecordingError> {
    let element = pipeline
        .by_name(element_name)
        .ok_or(ScreenRecordingError::Pipeline)?;
    let pad = element
        .static_pad("src")
        .ok_or(ScreenRecordingError::Pipeline)?;
    let probe = Arc::new(EncodedVideoProbe {
        first_pts_ns: AtomicU64::new(u64::MAX),
        ..EncodedVideoProbe::default()
    });
    let callback_probe = Arc::clone(&probe);
    pad.add_probe(gst::PadProbeType::BUFFER, move |_, information| {
        if let Some(gst::PadProbeData::Buffer(buffer)) = information.data.as_ref() {
            let _ = callback_probe.frames.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |current| current.checked_add(1),
            );
            match (buffer.pts(), buffer.duration()) {
                (Some(pts), Some(duration)) => {
                    let pts_ns = pts.nseconds();
                    let Some(end_ns) = pts_ns.checked_add(duration.nseconds()) else {
                        callback_probe.invalid_timing.store(true, Ordering::Relaxed);
                        return gst::PadProbeReturn::Ok;
                    };
                    callback_probe
                        .first_pts_ns
                        .fetch_min(pts_ns, Ordering::Relaxed);
                    callback_probe
                        .end_pts_ns
                        .fetch_max(end_ns, Ordering::Relaxed);
                }
                _ => callback_probe.invalid_timing.store(true, Ordering::Relaxed),
            }
        }
        gst::PadProbeReturn::Ok
    })
    .ok_or(ScreenRecordingError::Pipeline)?;
    Ok(probe)
}

fn timing_probe_result(probe: &EncodedVideoProbe) -> Result<EncodedVideo, ScreenRecordingError> {
    let frames = probe.frames.load(Ordering::Relaxed);
    let first_pts_ns = probe.first_pts_ns.load(Ordering::Relaxed);
    let end_pts_ns = probe.end_pts_ns.load(Ordering::Relaxed);
    if frames == 0
        || first_pts_ns == u64::MAX
        || end_pts_ns <= first_pts_ns
        || probe.invalid_timing.load(Ordering::Relaxed)
    {
        return Err(ScreenRecordingError::InvalidOutput);
    }
    Ok(EncodedVideo {
        frames,
        first_pts_ns,
        end_pts_ns,
        duration_ns: end_pts_ns - first_pts_ns,
    })
}

fn build_verifier_pipeline() -> Result<gst::Pipeline, ScreenRecordingError> {
    #[cfg(unix)]
    let description = concat!(
        "fdsrc name=screen_verify_source ! matroskademux ",
        "! identity name=screen_verify_counter ! vp8dec ! fakesink sync=false"
    );
    #[cfg(not(unix))]
    let description = concat!(
        "filesrc name=screen_verify_source ! matroskademux ",
        "! identity name=screen_verify_counter ! vp8dec ! fakesink sync=false"
    );
    gst::parse::launch(description)
        .map_err(|_| ScreenRecordingError::Pipeline)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| ScreenRecordingError::Pipeline)
}

fn build_av_verifier_pipeline() -> Result<gst::Pipeline, ScreenRecordingError> {
    #[cfg(unix)]
    let description = concat!(
        "fdsrc name=screen_av_verify_source ! matroskademux name=demux ",
        "demux. ! queue ! video/x-vp8 ! vp8dec ! identity name=screen_av_verify_video ! fakesink sync=false ",
        "demux. ! queue ! audio/x-opus ! opusdec ! identity name=screen_av_verify_audio ! fakesink sync=false"
    );
    #[cfg(not(unix))]
    let description = concat!(
        "filesrc name=screen_av_verify_source ! matroskademux name=demux ",
        "demux. ! queue ! video/x-vp8 ! vp8dec ! identity name=screen_av_verify_video ! fakesink sync=false ",
        "demux. ! queue ! audio/x-opus ! opusdec ! identity name=screen_av_verify_audio ! fakesink sync=false"
    );
    gst::parse::launch(description)
        .map_err(|_| ScreenRecordingError::Pipeline)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| ScreenRecordingError::Pipeline)
}

fn wait_for_verifier(
    pipeline: &gst::Pipeline,
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<(), ScreenRecordingError> {
    let bus = pipeline.bus().ok_or(ScreenRecordingError::Pipeline)?;
    loop {
        check_budget(cancellation, started)?;
        let Some(message) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(BUS_POLL.as_millis() as u64),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) else {
            continue;
        };
        match message.view() {
            gst::MessageView::Eos(_) => return Ok(()),
            gst::MessageView::Error(_) => return Err(ScreenRecordingError::Pipeline),
            _ => {}
        }
    }
}

fn sha256_file(
    file: &mut File,
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<String, ScreenRecordingError> {
    file.seek(SeekFrom::Start(0))
        .map_err(ScreenRecordingError::Filesystem)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        check_budget(cancellation, started)?;
        let read = file
            .read(&mut buffer)
            .map_err(ScreenRecordingError::Filesystem)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn check_budget(
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<(), ScreenRecordingError> {
    if cancellation.is_cancelled() {
        return Err(ScreenRecordingError::Cancelled);
    }
    if started.elapsed() >= VERIFICATION_TIMEOUT {
        return Err(ScreenRecordingError::Timeout);
    }
    Ok(())
}

fn reject_existing_path(path: &Path) -> Result<(), ScreenRecordingError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(ScreenRecordingError::OutputExists),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ScreenRecordingError::Filesystem(error)),
    }
}

fn ensure_output_parent(path: &Path) -> Result<(), ScreenRecordingError> {
    let _newly_created = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(ScreenRecordingError::OutputOwnership);
            }
            false
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(ScreenRecordingError::Filesystem)?;
            true
        }
        Err(error) => return Err(ScreenRecordingError::Filesystem(error)),
    };
    #[cfg(unix)]
    if _newly_created {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(ScreenRecordingError::Filesystem)?;
    }
    let metadata = fs::symlink_metadata(path).map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(ScreenRecordingError::OutputOwnership);
    }
    Ok(())
}

fn create_private_staging_directory(parent: &Path) -> Result<PathBuf, ScreenRecordingError> {
    for _ in 0..PRIVATE_DIRECTORY_ATTEMPTS {
        let path = parent.join(format!(".frame-screen-{}", Uuid::new_v4().simple()));
        #[cfg(unix)]
        let mut builder = fs::DirBuilder::new();
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => {
                verify_private_directory(&path)?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(ScreenRecordingError::Filesystem(error)),
        }
    }
    Err(ScreenRecordingError::OutputOwnership)
}

fn verify_private_directory(path: &Path) -> Result<(), ScreenRecordingError> {
    let metadata = fs::symlink_metadata(path).map_err(ScreenRecordingError::Filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(ScreenRecordingError::OutputOwnership);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ScreenRecordingError::OutputOwnership);
        }
    }
    Ok(())
}

fn sync_parent_directory(path: &Path) -> Result<(), ScreenRecordingError> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(ScreenRecordingError::Filesystem)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    // Rust's standard library does not expose a portable Windows directory
    // flush. The writable artifact handle is still flushed above; Windows
    // skips only the directory-entry durability strengthening.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_never_exposes_or_replaces_final_path_before_commit() {
        let directory = tempfile::tempdir().expect("private test directory");
        let final_path = directory.path().join("final.webm");
        let reservation =
            OutputReservation::for_filesink(final_path.clone()).expect("staging reservation");
        assert!(!final_path.exists());
        assert!(reservation.staging_path().is_file());
        drop(reservation);
        assert!(!final_path.exists());
    }

    #[test]
    fn preexisting_symlink_is_never_treated_as_an_available_output() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let directory = tempfile::tempdir().expect("private test directory");
            let target = directory.path().join("target");
            let output = directory.path().join("output");
            fs::write(&target, b"preserve").expect("target fixture");
            symlink(&target, &output).expect("symlink fixture");
            assert!(matches!(
                OutputReservation::for_filesink(output),
                Err(ScreenRecordingError::OutputExists)
            ));
            assert_eq!(fs::read(target).expect("preserved target"), b"preserve");
        }
    }
}
