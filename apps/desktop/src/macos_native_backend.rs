//! macOS composition of ScreenCaptureKit with the owned GStreamer recorder.
//!
//! This module is deliberately narrower than the provider-neutral capture
//! contracts: it records one opaque display, window, or region target, embeds
//! the cursor, excludes the entire current Frame application where the native
//! target permits it, optionally captures exact 48 kHz stereo
//! system audio, and exports an Editable WebM. Camera, microphone, pause, and
//! distribution-master paths stay disabled in the desktop runtime.

#![forbid(unsafe_code)]

mod av_worker;

use std::{
    collections::BTreeMap,
    fmt,
    io::{Read, Seek, SeekFrom, Write},
    mem,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU16, Ordering},
        mpsc::{SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use frame_macos_av_capture::{
    MacOsSystemAudioDiagnostics, MacOsSystemAudioError, MacOsSystemAudioSource, SYSTEM_AUDIO_FORMAT,
};
#[cfg(test)]
use frame_macos_screen_capture::MacOsCaptureStopError;
use frame_macos_screen_capture::{
    MacOsCaptureConfig, MacOsCaptureDiagnostics, MacOsCaptureError,
    MacOsNormalizedScreenCaptureSource, MacOsRegionSelection, MacOsScreenCaptureSource,
};
#[cfg(test)]
use frame_media::ScreenRecordingArtifact;
use frame_media::{
    ColorSpace, CursorCaptureMode, DisplayGeometryTransform, FrameMemory, LogicalRect,
    PermissionPreflight, PixelFormat, ProtectedContentPolicy, Rotation, ScreenAudioRecording,
    ScreenAudioRecordingArtifact, ScreenRecording, ScreenRecordingError, ScreenRecordingSpec,
    ScreenSessionId, ScreenSourceInstanceId, ScreenTargetBinding, ScreenTargetDescriptor,
    ScreenTargetKind, ScreenTargetSnapshot, SystemAudioRecordingSpec, VideoFrameSpec,
    preflight_screen_recording_runtime,
};

use self::av_worker::{
    AvWorkerTelemetry, SharedClockNormalizer, calibrate_av_startup, classify_audio_stop,
    classify_screen_stop, run_av_capture_worker,
};
use crate::native_screen_worker::{
    CompletedRecordingArtifact, NativeScreenSource, ScreenWorkerStart, WorkerCompletion,
    WorkerControl, WorkerOutcome,
};
use ring::{
    digest::{Context as Sha256Context, SHA256},
    rand::{SecureRandom, SystemRandom},
};
use rustix::io::Errno;
use zeroize::Zeroizing;

use crate::{
    CAPTURE_TARGET_CATALOG_VERSION, CaptureTargetCatalog, CaptureTargetKind, CaptureTargetSummary,
    DurableAvSettingsStore, NativeCaptureArtifact, NativeCaptureStartRequest, NativeDesktopBackend,
    NativeDesktopBackendError, NativeEditableWebmExportOutcome, NativeEditableWebmExportRequest,
    NativePermissionOutcome, NativeRecordingCancelOutcome, NativeRecordingControlRequest,
    NativeRecordingMeter, NativeRecordingStartOutcome, NativeRecordingStopOutcome,
    NativeRecordingTerminalFailure, NativeRegionDefinitionOutcome, NativeRegionDefinitionRequest,
    NativeTargetSelectionOutcome, NativeTargetSelectionRequest, PathUse,
    rooted_io::{FileIdentity, RootedDir, RootedFile, RootedIoError},
};

const TOKEN_RANDOM_BYTES: usize = 16;
const WORKER_CONTROL_CAPACITY: usize = 1;
const WORKER_START_CAPACITY: usize = 1;
const WORKER_IDLE_POLL: Duration = Duration::from_millis(2);
const MAX_TOKEN_ATTEMPTS: usize = 8;
const FILE_IO_BUFFER_BYTES: usize = 64 * 1_024;
const RECORDINGS_DIRECTORY: &str = "recordings";
const EXPORT_STAGING_DIRECTORY: &str = ".frame-staging";

#[derive(Clone)]
struct CatalogTarget {
    summary: CaptureTargetSummary,
    descriptor: ScreenTargetDescriptor,
}

impl fmt::Debug for CatalogTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogTarget")
            .field("summary", &self.summary)
            .field("descriptor", &self.descriptor)
            .finish()
    }
}

struct ActiveRecording {
    token: String,
    control: SyncSender<WorkerControl>,
    worker: JoinHandle<WorkerCompletion>,
    output: PendingRecordingOutput,
    system_audio_meter: Arc<AtomicU16>,
}

struct PendingRecordingOutput {
    staging_relative: PathBuf,
    final_relative: PathBuf,
    identity: FileIdentity,
}

impl fmt::Debug for PendingRecordingOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingRecordingOutput")
            .field("paths", &"<redacted>")
            .field("identity", &"<redacted>")
            .finish()
    }
}

struct SessionSource {
    source: MacOsScreenCaptureSource,
    system_audio: MacOsSystemAudioSource,
    observed_topology_generation: Option<u64>,
    snapshot: Option<ScreenTargetSnapshot>,
}

enum CaptureLifecycle {
    Ready(Box<SessionSource>),
    Recording(ActiveRecording),
    Poisoned,
}

impl fmt::Debug for ActiveRecording {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActiveRecording")
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct StoredArtifact {
    token: String,
    revision: u64,
    source: PathBuf,
    source_relative: PathBuf,
    source_identity: FileIdentity,
    source_bytes: u64,
    source_sha256: String,
    export: PathBuf,
    export_relative: PathBuf,
}

impl fmt::Debug for StoredArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredArtifact")
            .field("token", &"<redacted>")
            .field("revision", &self.revision)
            .field("source", &"<redacted>")
            .field("export", &"<redacted>")
            .finish()
    }
}

impl From<ScreenAudioRecordingArtifact> for CompletedRecordingArtifact {
    fn from(artifact: ScreenAudioRecordingArtifact) -> Self {
        Self {
            path: artifact.path,
            bytes: artifact.bytes,
            sha256: artifact.sha256,
            duration_ns: artifact.video_duration_ns.max(artifact.audio_duration_ns),
        }
    }
}

impl NativeScreenSource for MacOsNormalizedScreenCaptureSource {
    type RawSource = MacOsScreenCaptureSource;
    type Diagnostics = MacOsCaptureDiagnostics;

    fn normalize(
        source: Self::RawSource,
        snapshot: ScreenTargetSnapshot,
    ) -> Result<Self, NativeDesktopBackendError> {
        Self::new(source, snapshot).map_err(map_capture_error)
    }

    fn native_is_running(&self) -> bool {
        self.raw_source().is_running()
    }

    fn diagnostics(&self) -> Self::Diagnostics {
        self.raw_source().diagnostics()
    }

    fn diagnostics_failed(baseline: Self::Diagnostics, current: Self::Diagnostics) -> bool {
        diagnostics_failed(baseline, current)
    }

    fn protected_content_policy() -> ProtectedContentPolicy {
        ProtectedContentPolicy::FailSession
    }
}

enum WorkerStart {
    ScreenOnly(Box<ScreenWorkerStart<MacOsNormalizedScreenCaptureSource>>),
    ScreenAudio(Box<ScreenAudioWorkerStart>),
}

struct ScreenAudioWorkerStart {
    source: MacOsScreenCaptureSource,
    system_audio: MacOsSystemAudioSource,
    recording: ScreenAudioRecording,
    timestamps: SharedClockNormalizer,
    screen_diagnostic_baseline: MacOsCaptureDiagnostics,
    audio_diagnostic_baseline: MacOsSystemAudioDiagnostics,
}

enum PendingRecordingGraph {
    ScreenOnly(ScreenRecording),
    ScreenAudio(ScreenAudioRecording),
}

/// Production native backend selected by the release macOS composition root.
pub struct MacOsNativeDesktopBackend {
    capture: CaptureLifecycle,
    installation_secret: Zeroizing<[u8; 32]>,
    media_root: PathBuf,
    media_directory: RootedDir,
    recordings_root: PathBuf,
    recordings_directory: RootedDir,
    export_root: PathBuf,
    export_directory: RootedDir,
    export_staging_root: PathBuf,
    export_staging_directory: RootedDir,
    catalog_generation: u64,
    stable_tokens: BTreeMap<ScreenTargetBinding, String>,
    catalog: BTreeMap<String, CatalogTarget>,
    selected_token: Option<String>,
    artifact_revision: u64,
    artifact: Option<StoredArtifact>,
}

impl MacOsNativeDesktopBackend {
    pub fn new(
        media_root: impl Into<PathBuf>,
        export_root: impl Into<PathBuf>,
    ) -> Result<Self, NativeDesktopBackendError> {
        let (media_root, media_directory) = bind_or_create_root(media_root.into(), true)?;
        let (export_root, export_directory) = bind_or_create_root(export_root.into(), false)?;
        if media_root == export_root {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let recordings_root = media_root.join(RECORDINGS_DIRECTORY);
        let recordings_directory = match media_directory.create_private_dir(RECORDINGS_DIRECTORY) {
            Ok(directory) => directory,
            Err(RootedIoError::EntryExists) => media_directory
                .open_dir(RECORDINGS_DIRECTORY)
                .map_err(map_rooted_io_error)?,
            Err(error) => return Err(map_rooted_io_error(error)),
        };
        recordings_directory
            .ensure_private_mode()
            .map_err(map_rooted_io_error)?;
        let export_staging_root = export_root.join(EXPORT_STAGING_DIRECTORY);
        let export_staging_directory =
            match export_directory.create_private_dir(EXPORT_STAGING_DIRECTORY) {
                Ok(directory) => directory,
                Err(RootedIoError::EntryExists) => export_directory
                    .open_dir(EXPORT_STAGING_DIRECTORY)
                    .map_err(map_rooted_io_error)?,
                Err(error) => return Err(map_rooted_io_error(error)),
            };
        export_staging_directory
            .ensure_private_mode()
            .map_err(map_rooted_io_error)?;
        ensure_visible_directory(&media_directory, &media_root)?;
        ensure_visible_directory(&recordings_directory, &recordings_root)?;
        ensure_visible_directory(&export_directory, &export_root)?;
        ensure_visible_directory(&export_staging_directory, &export_staging_root)?;
        preflight_screen_recording_runtime().map_err(map_recording_error)?;

        let app_data_root = media_root
            .parent()
            .filter(|parent| parent != &media_root)
            .ok_or(NativeDesktopBackendError::Filesystem)?;
        let app_data_directory = RootedDir::bind(app_data_root).map_err(map_rooted_io_error)?;
        let mut settings_store = DurableAvSettingsStore::open(&app_data_directory)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        let installation_secret = settings_store
            .load_or_create_installation_secret()
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        let installation_secret = Zeroizing::new(*installation_secret.as_bytes());
        let source = new_session_source(&installation_secret)?;
        Ok(Self {
            capture: CaptureLifecycle::Ready(Box::new(source)),
            installation_secret,
            media_root,
            media_directory,
            recordings_root,
            recordings_directory,
            export_root,
            export_directory,
            export_staging_root,
            export_staging_directory,
            // Each source incarnation owns one externally monotonic catalog
            // generation even though its native topology counter starts over.
            catalog_generation: 1,
            stable_tokens: BTreeMap::new(),
            catalog: BTreeMap::new(),
            selected_token: None,
            artifact_revision: 0,
            artifact: None,
        })
    }

    fn source_mut(&mut self) -> Result<&mut MacOsScreenCaptureSource, NativeDesktopBackendError> {
        match &mut self.capture {
            CaptureLifecycle::Ready(session) => Ok(&mut session.source),
            CaptureLifecycle::Recording(_) => Err(NativeDesktopBackendError::Busy),
            CaptureLifecycle::Poisoned => Err(NativeDesktopBackendError::Unavailable),
        }
    }

    fn clear_catalog(&mut self) {
        self.stable_tokens.clear();
        self.catalog.clear();
        self.selected_token = None;
    }

    fn ensure_media_directories_visible(&self) -> Result<(), NativeDesktopBackendError> {
        ensure_visible_directory(&self.media_directory, &self.media_root)?;
        ensure_visible_directory(&self.recordings_directory, &self.recordings_root)
    }

    fn ensure_export_directories_visible(&self) -> Result<(), NativeDesktopBackendError> {
        ensure_visible_directory(&self.export_directory, &self.export_root)?;
        ensure_visible_directory(&self.export_staging_directory, &self.export_staging_root)
    }

    fn advance_catalog_generation(&mut self) -> Result<(), NativeDesktopBackendError> {
        let Some(generation) = self.catalog_generation.checked_add(1) else {
            self.capture = CaptureLifecycle::Poisoned;
            self.clear_catalog();
            return Err(NativeDesktopBackendError::Internal);
        };
        self.catalog_generation = generation;
        self.clear_catalog();
        Ok(())
    }

    /// Retire the only source allowed to participate in the completed session.
    /// A failed native stop is quarantined instead of being relabelled as safe.
    fn retire_session(&mut self, teardown_confirmed: bool) -> bool {
        if self.advance_catalog_generation().is_err() || !teardown_confirmed {
            self.capture = CaptureLifecycle::Poisoned;
            return false;
        }
        match new_session_source(&self.installation_secret) {
            Ok(source) => {
                self.capture = CaptureLifecycle::Ready(Box::new(source));
                true
            }
            Err(_) => {
                self.capture = CaptureLifecycle::Poisoned;
                false
            }
        }
    }

    fn fresh_token(&self, prefix: &str) -> Result<String, NativeDesktopBackendError> {
        let random = SystemRandom::new();
        for _ in 0..MAX_TOKEN_ATTEMPTS {
            let bytes: [u8; TOKEN_RANDOM_BYTES] = random_array(&random)?;
            let token = format!("{prefix}-{}", encode_hex(&bytes));
            let catalog_collision = self.catalog.contains_key(&token)
                || self
                    .stable_tokens
                    .values()
                    .any(|existing| existing == &token);
            let active_collision = matches!(
                &self.capture,
                CaptureLifecycle::Recording(active) if active.token == token
            );
            let artifact_collision = self
                .artifact
                .as_ref()
                .is_some_and(|artifact| artifact.token == token);
            if !catalog_collision && !active_collision && !artifact_collision {
                return Ok(token);
            }
        }
        Err(NativeDesktopBackendError::Internal)
    }

    fn install_target_snapshot(
        &mut self,
        snapshot: ScreenTargetSnapshot,
        previous_topology_generation: Option<u64>,
    ) -> Result<CaptureTargetCatalog, NativeDesktopBackendError> {
        if previous_topology_generation
            .is_some_and(|generation| generation != snapshot.generation())
        {
            self.advance_catalog_generation()?;
        }

        let mut next_catalog = BTreeMap::new();
        for (index, target) in snapshot.targets().iter().enumerate() {
            let binding = target.binding();
            let token = if let Some(existing) = self.stable_tokens.get(&binding) {
                existing.clone()
            } else {
                let fresh = self.fresh_token(target_token_prefix(target.kind()))?;
                self.stable_tokens.insert(binding, fresh.clone());
                fresh
            };
            let ordinal =
                u16::try_from(index + 1).map_err(|_| NativeDesktopBackendError::Internal)?;
            let summary = target_summary(token.clone(), ordinal, target, snapshot.targets())?;
            next_catalog.insert(
                token,
                CatalogTarget {
                    summary,
                    descriptor: target.clone(),
                },
            );
        }
        self.catalog = next_catalog;
        if self
            .selected_token
            .as_ref()
            .is_some_and(|token| !self.catalog.contains_key(token))
        {
            self.selected_token = None;
        }
        let catalog = CaptureTargetCatalog {
            schema_version: CAPTURE_TARGET_CATALOG_VERSION,
            generation: self.catalog_generation,
            targets: self
                .catalog
                .values()
                .map(|target| target.summary.clone())
                .collect(),
        };
        catalog
            .validate_enumeration()
            .map_err(|_| NativeDesktopBackendError::Internal)?;
        Ok(catalog)
    }

    fn take_worker(
        &mut self,
        expected_token: &str,
        command: WorkerControl,
    ) -> Result<(String, PendingRecordingOutput, WorkerCompletion), NativeDesktopBackendError> {
        let active = match &self.capture {
            CaptureLifecycle::Recording(active) => active,
            CaptureLifecycle::Ready(_) => {
                return Err(NativeDesktopBackendError::TargetUnavailable);
            }
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        if active.token != expected_token {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        match active.control.try_send(command) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => return Err(NativeDesktopBackendError::Busy),
        }
        let CaptureLifecycle::Recording(active) =
            mem::replace(&mut self.capture, CaptureLifecycle::Poisoned)
        else {
            return Err(NativeDesktopBackendError::Internal);
        };
        let token = active.token;
        let output = active.output;
        let completion = match active.worker.join() {
            Ok(completion) => completion,
            Err(_) => WorkerCompletion {
                outcome: WorkerOutcome::Failed {
                    error: NativeDesktopBackendError::Internal,
                    teardown_confirmed: false,
                },
            },
        };
        Ok((token, output, completion))
    }

    fn cleanup_recording_output(&self, output: &PendingRecordingOutput) {
        let _ = self
            .recordings_directory
            .cleanup_file_if_identity(&output.staging_relative, output.identity);
        let _ = self
            .recordings_directory
            .cleanup_file_if_identity(&output.final_relative, output.identity);
    }

    fn publish_recording_artifact(
        &self,
        output: &PendingRecordingOutput,
        artifact: &CompletedRecordingArtifact,
    ) -> Result<(), NativeDesktopBackendError> {
        self.ensure_media_directories_visible()?;
        let expected_path = self.recordings_root.join(&output.final_relative);
        if artifact.path != expected_path {
            self.cleanup_recording_output(output);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let mut staging = self
            .recordings_directory
            .open_regular_file(&output.staging_relative)
            .map_err(map_rooted_io_error)?;
        let metadata = staging.metadata();
        if metadata.identity() != output.identity || metadata.size_bytes() != artifact.bytes {
            drop(staging);
            self.cleanup_recording_output(output);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        staging.sync().map_err(map_rooted_io_error)?;
        let refreshed = staging.refresh_metadata().map_err(map_rooted_io_error)?;
        if refreshed.identity() != output.identity || refreshed.size_bytes() != artifact.bytes {
            drop(staging);
            self.cleanup_recording_output(output);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let published = self
            .recordings_directory
            .publish_file_if_identity(
                &output.staging_relative,
                output.identity,
                &output.final_relative,
            )
            .map_err(|error| {
                self.cleanup_recording_output(output);
                map_rooted_io_error(error)
            })?;
        if published.identity() != output.identity || published.size_bytes() != artifact.bytes {
            self.cleanup_recording_output(output);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        if self.ensure_media_directories_visible().is_err() {
            drop(staging);
            self.cleanup_recording_output(output);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        Ok(())
    }

    fn seal_artifact(
        &mut self,
        recording_token: String,
        artifact: CompletedRecordingArtifact,
    ) -> Result<NativeCaptureArtifact, NativeDesktopBackendError> {
        self.ensure_media_directories_visible()?;
        self.ensure_export_directories_visible()?;
        self.artifact_revision = self
            .artifact_revision
            .checked_add(1)
            .ok_or(NativeDesktopBackendError::Internal)?;
        let artifact_token = self.fresh_token("artifact")?;
        let source_relative = PathBuf::from(format!("{recording_token}.webm"));
        let expected_source = self.recordings_root.join(&source_relative);
        if artifact.path != expected_source {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let mut source_file = self
            .recordings_directory
            .open_regular_file(&source_relative)
            .map_err(map_rooted_io_error)?;
        let source_metadata = source_file.metadata();
        if source_metadata.size_bytes() != artifact.bytes {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let source_sha256 = sha256_rooted_file(
            &mut source_file,
            source_metadata.identity(),
            source_metadata.size_bytes(),
        )?;
        if source_sha256 != artifact.sha256 {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        self.ensure_media_directories_visible()?;
        self.ensure_export_directories_visible()?;
        let export_relative = PathBuf::from(format!("Frame-{artifact_token}.webm"));
        let export_path = self.export_root.join(&export_relative);
        let source_text = path_text(&artifact.path)?;
        let export_text = path_text(&export_path)?;
        let duration_ms = artifact.duration_ns.div_ceil(1_000_000).max(1);
        let response = NativeCaptureArtifact {
            recording_token,
            artifact_token: artifact_token.clone(),
            artifact_revision: self.artifact_revision,
            duration_ms,
            bytes_written: artifact.bytes,
            media_path: source_text,
            editable_webm_output_path: Some(export_text),
        };
        self.artifact = Some(StoredArtifact {
            token: artifact_token,
            revision: self.artifact_revision,
            source: artifact.path,
            source_relative,
            source_identity: source_metadata.identity(),
            source_bytes: source_metadata.size_bytes(),
            source_sha256,
            export: export_path,
            export_relative,
        });
        Ok(response)
    }
}

impl fmt::Debug for MacOsNativeDesktopBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacOsNativeDesktopBackend")
            .field(
                "capture",
                &match &self.capture {
                    CaptureLifecycle::Ready(_) => "ready",
                    CaptureLifecycle::Recording(_) => "recording",
                    CaptureLifecycle::Poisoned => "poisoned",
                },
            )
            .field("catalog_generation", &self.catalog_generation)
            .field("catalog_size", &self.catalog.len())
            .field(
                "selected",
                &self.selected_token.as_ref().map(|_| "<redacted>"),
            )
            .field("artifact", &self.artifact)
            .finish_non_exhaustive()
    }
}

impl NativeDesktopBackend for MacOsNativeDesktopBackend {
    fn prepare_capture(&mut self) -> Result<NativePermissionOutcome, NativeDesktopBackendError> {
        let source = self.source_mut()?;
        let permission = match source.preflight_permission() {
            PermissionPreflight::Granted => PermissionPreflight::Granted,
            PermissionPreflight::PromptRequired => source.request_permission(),
            denied => denied,
        };
        Ok(match permission {
            PermissionPreflight::Granted => NativePermissionOutcome::Granted,
            PermissionPreflight::PromptRequired
            | PermissionPreflight::Denied(_)
            | PermissionPreflight::Restricted
            | PermissionPreflight::Revoked(_) => NativePermissionOutcome::Denied,
        })
    }

    fn enumerate_targets(&mut self) -> Result<CaptureTargetCatalog, NativeDesktopBackendError> {
        let (snapshot, previous_topology_generation) = match &mut self.capture {
            CaptureLifecycle::Ready(session) => {
                let snapshot = session
                    .source
                    .enumerate_targets(&[])
                    .map_err(map_capture_error)?;
                let previous = session.observed_topology_generation;
                if previous.is_some_and(|generation| generation > snapshot.generation()) {
                    return Err(NativeDesktopBackendError::Internal);
                }
                session.observed_topology_generation = Some(snapshot.generation());
                session.snapshot = Some(snapshot.clone());
                (snapshot, previous)
            }
            CaptureLifecycle::Recording(_) => return Err(NativeDesktopBackendError::Busy),
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        self.install_target_snapshot(snapshot, previous_topology_generation)
    }

    fn select_target(
        &mut self,
        request: &NativeTargetSelectionRequest,
    ) -> Result<NativeTargetSelectionOutcome, NativeDesktopBackendError> {
        match &self.capture {
            CaptureLifecycle::Ready(_) => {}
            CaptureLifecycle::Recording(_) => return Err(NativeDesktopBackendError::Busy),
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        }
        if request.catalog_generation != self.catalog_generation {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        let target = self
            .catalog
            .get(&request.target.token)
            .filter(|target| target.summary == request.target)
            .ok_or(NativeDesktopBackendError::TargetUnavailable)?;
        self.selected_token = Some(target.summary.token.clone());
        Ok(NativeTargetSelectionOutcome {
            catalog_generation: self.catalog_generation,
            target_token: target.summary.token.clone(),
        })
    }

    fn define_region(
        &mut self,
        request: &NativeRegionDefinitionRequest,
    ) -> Result<NativeRegionDefinitionOutcome, NativeDesktopBackendError> {
        if request.catalog_generation != self.catalog_generation {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        let display = self
            .catalog
            .get(&request.display.token)
            .filter(|target| {
                target.summary == request.display
                    && target.descriptor.kind() == ScreenTargetKind::Display
            })
            .cloned()
            .ok_or(NativeDesktopBackendError::TargetUnavailable)?;
        let display_bounds = display.descriptor.logical_bounds();
        let x = i64::from(display_bounds.x())
            .checked_add(i64::from(request.x))
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(NativeDesktopBackendError::TargetUnavailable)?;
        let y = i64::from(display_bounds.y())
            .checked_add(i64::from(request.y))
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(NativeDesktopBackendError::TargetUnavailable)?;
        let logical_bounds = LogicalRect::new(x, y, request.width, request.height)
            .map_err(|_| NativeDesktopBackendError::TargetUnavailable)?;
        if !display_bounds.contains_rect(logical_bounds) {
            return Err(NativeDesktopBackendError::TargetUnavailable);
        }
        let region_selection =
            MacOsRegionSelection::new(display.descriptor.binding(), logical_bounds)
                .map_err(map_capture_error)?;
        let (snapshot, previous_topology_generation) = match &mut self.capture {
            CaptureLifecycle::Ready(session) => {
                let snapshot = session
                    .source
                    .enumerate_targets(&[region_selection])
                    .map_err(map_capture_error)?;
                let previous = session.observed_topology_generation;
                if previous.is_some_and(|generation| generation > snapshot.generation()) {
                    return Err(NativeDesktopBackendError::Internal);
                }
                session.observed_topology_generation = Some(snapshot.generation());
                session.snapshot = Some(snapshot.clone());
                (snapshot, previous)
            }
            CaptureLifecycle::Recording(_) => return Err(NativeDesktopBackendError::Busy),
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        let catalog = self.install_target_snapshot(snapshot, previous_topology_generation)?;
        let region = self
            .catalog
            .values()
            .find(|target| {
                target.descriptor.kind() == ScreenTargetKind::Region
                    && target.descriptor.logical_bounds() == logical_bounds
            })
            .map(|target| target.summary.clone())
            .ok_or(NativeDesktopBackendError::Internal)?;
        self.selected_token = Some(region.token.clone());
        Ok(NativeRegionDefinitionOutcome { catalog, region })
    }

    fn start_recording(
        &mut self,
        request: &NativeCaptureStartRequest,
    ) -> Result<NativeRecordingStartOutcome, NativeDesktopBackendError> {
        self.ensure_media_directories_visible()?;
        self.ensure_export_directories_visible()?;
        match &self.capture {
            CaptureLifecycle::Ready(_) => {}
            CaptureLifecycle::Recording(_) => return Err(NativeDesktopBackendError::Busy),
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        }
        if !request.exclude_frame_windows {
            return Err(NativeDesktopBackendError::Unavailable);
        }
        if request.catalog_generation != self.catalog_generation {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        if self.selected_token.as_deref() != Some(request.target.token.as_str()) {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        let target = self
            .catalog
            .get(&request.target.token)
            .filter(|target| target.summary == request.target)
            .cloned()
            .ok_or(NativeDesktopBackendError::TargetUnavailable)?;
        let (width, height) = bounded_recording_dimensions(
            request.target.width_pixels,
            request.target.height_pixels,
        )?;
        let frame_duration = 1_000_000_000_u64
            .checked_div(u64::from(request.frame_rate))
            .filter(|duration| *duration > 0)
            .ok_or(NativeDesktopBackendError::Internal)?;
        let frame_spec = VideoFrameSpec {
            width,
            height,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: frame_duration,
            memory: FrameMemory::Cpu,
        };
        let recording_spec = ScreenRecordingSpec::new(frame_spec).map_err(map_recording_error)?;
        let screen_session_id = ScreenSessionId::from_csprng(random_array(&SystemRandom::new())?)
            .map_err(|_| NativeDesktopBackendError::Internal)?;
        let recording_token = self.fresh_token("recording")?;
        let final_relative = PathBuf::from(format!("{recording_token}.webm"));
        let staging_relative = PathBuf::from(format!(".{recording_token}.partial"));
        let session = match mem::replace(&mut self.capture, CaptureLifecycle::Poisoned) {
            CaptureLifecycle::Ready(session) => session,
            CaptureLifecycle::Recording(active) => {
                self.capture = CaptureLifecycle::Recording(active);
                return Err(NativeDesktopBackendError::Busy);
            }
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        let SessionSource {
            mut source,
            mut system_audio,
            observed_topology_generation,
            snapshot,
        } = *session;

        let screen_diagnostic_baseline = source.diagnostics();
        let audio_diagnostic_baseline = system_audio.diagnostics();
        let mut system_audio_included = if should_attempt_system_audio(
            request.system_audio_enabled,
            system_audio.preflight_permission(),
        ) {
            match system_audio.start() {
                Ok(()) => true,
                Err(error) if system_audio_source_reusable_after_start_error(error) => {
                    eprintln!(
                        "Frame system audio is unavailable; continuing with screen-only capture: {error}"
                    );
                    false
                }
                Err(error) => {
                    self.capture = CaptureLifecycle::Poisoned;
                    return Err(map_system_audio_error(error));
                }
            }
        } else {
            false
        };

        let staging = match self.recordings_directory.create_new_file(&staging_relative) {
            Ok(staging) => staging,
            Err(error) => {
                let audio_teardown_confirmed = if system_audio_included {
                    stop_unowned_system_audio(&mut system_audio, "recording output creation failed")
                } else {
                    true
                };
                if audio_teardown_confirmed {
                    self.capture = CaptureLifecycle::Ready(Box::new(SessionSource {
                        source,
                        system_audio,
                        observed_topology_generation,
                        snapshot,
                    }));
                }
                return Err(map_rooted_io_error(error));
            }
        };
        let mut staging_identity = staging.metadata().identity();
        let recording_path = self.recordings_root.join(&final_relative);
        let recording = if system_audio_included {
            SystemAudioRecordingSpec::new(SYSTEM_AUDIO_FORMAT).and_then(|audio_spec| {
                ScreenAudioRecording::start_preopened(
                    &recording_path,
                    staging.into_file(),
                    recording_spec,
                    audio_spec,
                )
                .map(PendingRecordingGraph::ScreenAudio)
            })
        } else {
            ScreenRecording::start_preopened(&recording_path, staging.into_file(), recording_spec)
                .map(PendingRecordingGraph::ScreenOnly)
        };
        let recording = match recording {
            Ok(recording) => recording,
            Err(error) if system_audio_included && audio_graph_unavailable(&error) => {
                let primary_error = map_recording_error(error);
                let audio_teardown_confirmed =
                    stop_unowned_system_audio(&mut system_audio, "A/V recorder graph unavailable");
                let _ = self
                    .recordings_directory
                    .cleanup_file_if_identity(&staging_relative, staging_identity);
                if !audio_teardown_confirmed {
                    self.capture = CaptureLifecycle::Poisoned;
                    return Err(primary_error);
                }
                eprintln!(
                    "Frame A/V recorder graph is unavailable; continuing with screen-only capture"
                );
                let fallback_staging =
                    match self.recordings_directory.create_new_file(&staging_relative) {
                        Ok(staging) => staging,
                        Err(error) => {
                            self.capture = CaptureLifecycle::Ready(Box::new(SessionSource {
                                source,
                                system_audio,
                                observed_topology_generation,
                                snapshot,
                            }));
                            return Err(map_rooted_io_error(error));
                        }
                    };
                staging_identity = fallback_staging.metadata().identity();
                match ScreenRecording::start_preopened(
                    &recording_path,
                    fallback_staging.into_file(),
                    recording_spec,
                ) {
                    Ok(recording) => {
                        system_audio_included = false;
                        PendingRecordingGraph::ScreenOnly(recording)
                    }
                    Err(error) => {
                        let _ = self
                            .recordings_directory
                            .cleanup_file_if_identity(&staging_relative, staging_identity);
                        self.capture = CaptureLifecycle::Ready(Box::new(SessionSource {
                            source,
                            system_audio,
                            observed_topology_generation,
                            snapshot,
                        }));
                        return Err(map_recording_error(error));
                    }
                }
            }
            Err(error) => {
                let primary_error = map_recording_error(error);
                let audio_teardown_confirmed = if system_audio_included {
                    stop_unowned_system_audio(&mut system_audio, "recorder graph creation failed")
                } else {
                    true
                };
                let _ = self
                    .recordings_directory
                    .cleanup_file_if_identity(&staging_relative, staging_identity);
                if audio_teardown_confirmed {
                    self.capture = CaptureLifecycle::Ready(Box::new(SessionSource {
                        source,
                        system_audio,
                        observed_topology_generation,
                        snapshot,
                    }));
                }
                return Err(primary_error);
            }
        };
        let output = PendingRecordingOutput {
            staging_relative,
            final_relative,
            identity: staging_identity,
        };
        let worker_start_value = match recording {
            PendingRecordingGraph::ScreenOnly(recording) => {
                let Some(snapshot) = snapshot else {
                    let teardown_confirmed = abort_unowned_recording(
                        recording,
                        NativeDesktopBackendError::StaleCatalog,
                        "normalized capture snapshot was unavailable",
                    );
                    self.cleanup_recording_output(&output);
                    let _ = self.retire_session(teardown_confirmed);
                    return Err(NativeDesktopBackendError::StaleCatalog);
                };
                match ScreenWorkerStart::prepare(
                    source,
                    snapshot,
                    target.descriptor,
                    frame_spec,
                    recording,
                    screen_session_id,
                ) {
                    Ok(start) => WorkerStart::ScreenOnly(Box::new(start)),
                    Err(failure) => {
                        self.cleanup_recording_output(&output);
                        let _ = self.retire_session(failure.teardown_confirmed);
                        return Err(failure.error);
                    }
                }
            }
            PendingRecordingGraph::ScreenAudio(mut recording) => {
                let capture_config = match MacOsCaptureConfig::new(
                    target.descriptor.binding(),
                    frame_spec,
                    CursorCaptureMode::EmbeddedInFrame,
                ) {
                    Ok(config) => config,
                    Err(error) => {
                        let primary_error = map_capture_error(error);
                        let recorder_teardown_confirmed = recording.abort().is_ok();
                        let audio_teardown_confirmed = stop_unowned_system_audio(
                            &mut system_audio,
                            "native A/V capture configuration failed",
                        );
                        if recorder_teardown_confirmed && audio_teardown_confirmed {
                            self.capture = CaptureLifecycle::Ready(Box::new(SessionSource {
                                source,
                                system_audio,
                                observed_topology_generation,
                                snapshot,
                            }));
                        }
                        self.cleanup_recording_output(&output);
                        return Err(primary_error);
                    }
                };
                if let Err(error) = source.start(capture_config) {
                    let primary_error = map_capture_error(error);
                    let recorder_teardown_confirmed = match recording.abort() {
                        Ok(()) => true,
                        Err(teardown) => {
                            eprintln!(
                                "Frame A/V recorder teardown failed after native capture start failed while preserving primary error {primary_error}: {teardown}"
                            );
                            false
                        }
                    };
                    let audio_teardown_confirmed = stop_unowned_system_audio(
                        &mut system_audio,
                        "native A/V capture start failed",
                    );
                    if capture_start_resources_reusable(error, recorder_teardown_confirmed)
                        && audio_teardown_confirmed
                    {
                        self.capture = CaptureLifecycle::Ready(Box::new(SessionSource {
                            source,
                            system_audio,
                            observed_topology_generation,
                            snapshot,
                        }));
                    } else {
                        self.capture = CaptureLifecycle::Poisoned;
                    }
                    self.cleanup_recording_output(&output);
                    return Err(primary_error);
                }
                let timestamps =
                    match calibrate_av_startup(&mut source, &mut system_audio, &mut recording) {
                        Ok(timestamps) => timestamps,
                        Err(primary_error) => {
                            let (_, screen_teardown_confirmed, _) =
                                classify_screen_stop(source.stop_and_drain_frames());
                            let (_, audio_teardown_confirmed, _) =
                                classify_audio_stop(system_audio.stop_and_drain_chunks());
                            let recorder_teardown_confirmed = recording.abort().is_ok();
                            let teardown_confirmed = all_av_teardown_confirmed(
                                screen_teardown_confirmed,
                                audio_teardown_confirmed,
                                recorder_teardown_confirmed,
                            );
                            if teardown_confirmed {
                                self.capture = CaptureLifecycle::Ready(Box::new(SessionSource {
                                    source,
                                    system_audio,
                                    observed_topology_generation,
                                    snapshot,
                                }));
                            } else {
                                self.capture = CaptureLifecycle::Poisoned;
                            }
                            self.cleanup_recording_output(&output);
                            return Err(primary_error);
                        }
                    };
                WorkerStart::ScreenAudio(Box::new(ScreenAudioWorkerStart {
                    source,
                    system_audio,
                    recording,
                    timestamps,
                    screen_diagnostic_baseline,
                    audio_diagnostic_baseline,
                }))
            }
        };

        let (control, receiver) = sync_channel(WORKER_CONTROL_CAPACITY);
        let system_audio_meter = Arc::new(AtomicU16::new(0));
        let worker_system_audio_meter = Arc::clone(&system_audio_meter);
        // Keep both native authorities on this thread until worker creation is
        // confirmed. A failed startup send returns the tuple for explicit
        // teardown instead of hiding graph failure in Drop.
        let (worker_start, worker_start_receiver) = sync_channel(WORKER_START_CAPACITY);
        let worker = thread::Builder::new()
            .name("frame-macos-screen-recorder".into())
            .spawn(move || {
                let Ok(worker_start) = worker_start_receiver.recv() else {
                    return WorkerCompletion {
                        outcome: WorkerOutcome::Failed {
                            error: NativeDesktopBackendError::Internal,
                            teardown_confirmed: false,
                        },
                    };
                };
                match worker_start {
                    WorkerStart::ScreenOnly(start) => start.run(receiver),
                    WorkerStart::ScreenAudio(start) => {
                        let ScreenAudioWorkerStart {
                            source,
                            system_audio,
                            recording,
                            timestamps,
                            screen_diagnostic_baseline,
                            audio_diagnostic_baseline,
                        } = *start;
                        run_av_capture_worker(
                            source,
                            system_audio,
                            recording,
                            timestamps,
                            receiver,
                            AvWorkerTelemetry {
                                screen_diagnostic_baseline,
                                audio_diagnostic_baseline,
                                system_audio_meter: worker_system_audio_meter,
                            },
                        )
                    }
                }
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(_) => {
                let _ = teardown_worker_start(
                    worker_start_value,
                    NativeDesktopBackendError::Internal,
                    "native recording worker spawn failed",
                );
                let _ = self.advance_catalog_generation();
                self.capture = CaptureLifecycle::Poisoned;
                self.cleanup_recording_output(&output);
                return Err(NativeDesktopBackendError::Internal);
            }
        };
        if let Err(error) = worker_start.send(worker_start_value) {
            let _ = worker.join();
            let _ = teardown_worker_start(
                error.0,
                NativeDesktopBackendError::Internal,
                "native recording worker startup handoff failed",
            );
            let _ = self.advance_catalog_generation();
            self.capture = CaptureLifecycle::Poisoned;
            self.cleanup_recording_output(&output);
            return Err(NativeDesktopBackendError::Internal);
        }
        self.capture = CaptureLifecycle::Recording(ActiveRecording {
            token: recording_token.clone(),
            control,
            worker,
            output,
            system_audio_meter,
        });
        self.artifact = None;
        Ok(NativeRecordingStartOutcome {
            catalog_generation: self.catalog_generation,
            target_token: request.target.token.clone(),
            recording_token,
            system_audio_included,
        })
    }

    fn poll_recording_terminal_failure(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<Option<NativeRecordingTerminalFailure>, NativeDesktopBackendError> {
        let active = match &self.capture {
            CaptureLifecycle::Recording(active) => active,
            CaptureLifecycle::Ready(_) => return Err(NativeDesktopBackendError::TargetUnavailable),
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        if active.token != request.recording_token {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        if !active.worker.is_finished() {
            return Ok(None);
        }

        let CaptureLifecycle::Recording(active) =
            mem::replace(&mut self.capture, CaptureLifecycle::Poisoned)
        else {
            return Err(NativeDesktopBackendError::Internal);
        };
        let token = active.token;
        let output = active.output;
        let outcome = active.worker.join().map_or(
            WorkerOutcome::Failed {
                error: NativeDesktopBackendError::Internal,
                teardown_confirmed: false,
            },
            |completion| completion.outcome,
        );
        let teardown_confirmed = outcome.teardown_confirmed();
        let error = match outcome {
            WorkerOutcome::Failed { error, .. } => error,
            WorkerOutcome::Finished(_) | WorkerOutcome::Cancelled => {
                NativeDesktopBackendError::Internal
            }
        };
        self.cleanup_recording_output(&output);
        self.artifact = None;
        let _ = self.retire_session(teardown_confirmed);
        Ok(Some(NativeRecordingTerminalFailure {
            recording_token: token,
            error,
            teardown_confirmed,
        }))
    }

    fn poll_recording_meter(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingMeter, NativeDesktopBackendError> {
        let active = match &self.capture {
            CaptureLifecycle::Recording(active) => active,
            CaptureLifecycle::Ready(_) => return Err(NativeDesktopBackendError::TargetUnavailable),
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        if active.token != request.recording_token {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        Ok(NativeRecordingMeter {
            system_audio_basis_points: active.system_audio_meter.load(Ordering::Acquire),
        })
    }

    fn stop_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingStopOutcome, NativeDesktopBackendError> {
        let (recording_token, output, completion) =
            self.take_worker(&request.recording_token, WorkerControl::Stop)?;
        let teardown_confirmed = completion.outcome.teardown_confirmed();
        let outcome = match completion.outcome {
            WorkerOutcome::Finished(artifact) => {
                let sealed = self
                    .publish_recording_artifact(&output, &artifact)
                    .and_then(|()| self.seal_artifact(recording_token, artifact));
                match sealed {
                    Ok(artifact) => NativeRecordingStopOutcome::Sealed(artifact),
                    Err(error) => {
                        self.cleanup_recording_output(&output);
                        NativeRecordingStopOutcome::Failed(NativeRecordingTerminalFailure {
                            recording_token: request.recording_token.clone(),
                            error,
                            teardown_confirmed: true,
                        })
                    }
                }
            }
            WorkerOutcome::Cancelled => {
                self.cleanup_recording_output(&output);
                NativeRecordingStopOutcome::Failed(NativeRecordingTerminalFailure {
                    recording_token,
                    error: NativeDesktopBackendError::Cancelled,
                    teardown_confirmed: true,
                })
            }
            WorkerOutcome::Failed {
                error,
                teardown_confirmed,
            } => {
                self.cleanup_recording_output(&output);
                NativeRecordingStopOutcome::Failed(NativeRecordingTerminalFailure {
                    recording_token,
                    error,
                    teardown_confirmed,
                })
            }
        };
        let _ = self.retire_session(teardown_confirmed);
        Ok(outcome)
    }

    fn cancel_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingCancelOutcome, NativeDesktopBackendError> {
        let (recording_token, output, completion) =
            self.take_worker(&request.recording_token, WorkerControl::Cancel)?;
        let teardown_confirmed = completion.outcome.teardown_confirmed();
        self.cleanup_recording_output(&output);
        let outcome = match completion.outcome {
            WorkerOutcome::Cancelled => NativeRecordingCancelOutcome::Cancelled { recording_token },
            WorkerOutcome::Finished(_) => {
                NativeRecordingCancelOutcome::Failed(NativeRecordingTerminalFailure {
                    recording_token,
                    error: NativeDesktopBackendError::Internal,
                    teardown_confirmed: true,
                })
            }
            WorkerOutcome::Failed {
                error,
                teardown_confirmed,
            } => NativeRecordingCancelOutcome::Failed(NativeRecordingTerminalFailure {
                recording_token,
                error,
                teardown_confirmed,
            }),
        };
        let _ = self.retire_session(teardown_confirmed);
        self.artifact = None;
        Ok(outcome)
    }

    fn export_editable_webm(
        &mut self,
        request: &NativeEditableWebmExportRequest,
    ) -> Result<NativeEditableWebmExportOutcome, NativeDesktopBackendError> {
        self.ensure_media_directories_visible()?;
        self.ensure_export_directories_visible()?;
        if !request.source_media_path.requires_no_follow()
            || !request.output_path.requires_no_follow()
            || request.source_media_path.usage() != PathUse::MediaRead
            || request.output_path.usage() != PathUse::ExportWrite
        {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let artifact = self
            .artifact
            .as_ref()
            .filter(|artifact| {
                artifact.token == request.artifact_token
                    && artifact.revision == request.artifact_revision
                    && artifact.source == request.source_media_path.as_path()
                    && artifact.export == request.output_path.as_path()
            })
            .cloned()
            .ok_or(NativeDesktopBackendError::StaleCatalog)?;
        let mut source = self
            .recordings_directory
            .open_regular_file(&artifact.source_relative)
            .map_err(map_rooted_io_error)?;
        if source.metadata().identity() != artifact.source_identity
            || source.metadata().size_bytes() != artifact.source_bytes
        {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let (staging_relative, mut staging) =
            create_export_staging(&self.export_staging_directory, &artifact.token)?;
        let staging_identity = staging.metadata().identity();
        let copied = match copy_rooted_file_with_sha256(
            &mut source,
            &mut staging,
            artifact.source_identity,
            artifact.source_bytes,
            &artifact.source_sha256,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                drop(staging);
                let _ = self
                    .export_staging_directory
                    .cleanup_file_if_identity(&staging_relative, staging_identity);
                return Err(error);
            }
        };
        let staging_valid = staging
            .sync()
            .and_then(|()| staging.refresh_metadata())
            .is_ok_and(|metadata| {
                metadata.identity() == staging_identity && metadata.size_bytes() == copied
            });
        if !staging_valid {
            drop(staging);
            let _ = self
                .export_staging_directory
                .cleanup_file_if_identity(&staging_relative, staging_identity);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let published = self
            .export_staging_directory
            .publish_file_to_root_if_identity(
                &staging_relative,
                staging_identity,
                &self.export_directory,
                &artifact.export_relative,
            )
            .map_err(|error| {
                let _ = self
                    .export_staging_directory
                    .cleanup_file_if_identity(&staging_relative, staging_identity);
                // Publication can fail after the no-replace rename (for
                // example, while verifying or syncing it). In that case the
                // same identity is now reachable at the final name.
                let _ = self
                    .export_directory
                    .cleanup_file_if_identity(&artifact.export_relative, staging_identity);
                map_rooted_io_error(error)
            })?;
        if published.identity() != staging_identity || published.size_bytes() != copied {
            let _ = self
                .export_directory
                .cleanup_file_if_identity(&artifact.export_relative, staging_identity);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        if verify_published_rooted_file(
            &self.export_directory,
            &artifact.export_relative,
            staging_identity,
            copied,
            &artifact.source_sha256,
        )
        .is_err()
            || self.ensure_media_directories_visible().is_err()
            || self.ensure_export_directories_visible().is_err()
        {
            drop(staging);
            let _ = self
                .export_directory
                .cleanup_file_if_identity(&artifact.export_relative, staging_identity);
            return Err(NativeDesktopBackendError::Filesystem);
        }
        Ok(NativeEditableWebmExportOutcome {
            artifact_token: artifact.token,
            artifact_revision: artifact.revision,
            bytes_written: copied,
        })
    }
}

impl Drop for MacOsNativeDesktopBackend {
    fn drop(&mut self) {
        let capture = mem::replace(&mut self.capture, CaptureLifecycle::Poisoned);
        if let CaptureLifecycle::Recording(active) = capture {
            let ActiveRecording {
                control,
                worker,
                output,
                ..
            } = active;
            let _ = control.try_send(WorkerControl::Cancel);
            let _ = worker.join();
            self.cleanup_recording_output(&output);
        }
    }
}

const fn recording_finish_teardown_confirmed(error: &ScreenRecordingError) -> bool {
    !matches!(
        error,
        ScreenRecordingError::TeardownUnconfirmed(_)
            | ScreenRecordingError::OperationAndTeardown { .. }
    )
}

fn diagnostic_delta(
    baseline: MacOsCaptureDiagnostics,
    current: MacOsCaptureDiagnostics,
) -> Result<MacOsCaptureDiagnostics, NativeDesktopBackendError> {
    Ok(MacOsCaptureDiagnostics {
        dropped_callback_frames: current
            .dropped_callback_frames
            .checked_sub(baseline.dropped_callback_frames)
            .ok_or(NativeDesktopBackendError::Internal)?,
        callback_frames_after_stop: current
            .callback_frames_after_stop
            .checked_sub(baseline.callback_frames_after_stop)
            .ok_or(NativeDesktopBackendError::Internal)?,
        ignored_non_content_samples: current
            .ignored_non_content_samples
            .checked_sub(baseline.ignored_non_content_samples)
            .ok_or(NativeDesktopBackendError::Internal)?,
        invalid_samples: current
            .invalid_samples
            .checked_sub(baseline.invalid_samples)
            .ok_or(NativeDesktopBackendError::Internal)?,
        duration_fallbacks: current
            .duration_fallbacks
            .checked_sub(baseline.duration_fallbacks)
            .ok_or(NativeDesktopBackendError::Internal)?,
        timestamp_discontinuities: current
            .timestamp_discontinuities
            .checked_sub(baseline.timestamp_discontinuities)
            .ok_or(NativeDesktopBackendError::Internal)?,
        unexpected_native_stops: current
            .unexpected_native_stops
            .checked_sub(baseline.unexpected_native_stops)
            .ok_or(NativeDesktopBackendError::Internal)?,
    })
}

const fn diagnostics_have_terminal_fault(diagnostics: &MacOsCaptureDiagnostics) -> bool {
    diagnostics.dropped_callback_frames > 0
        || diagnostics.invalid_samples > 0
        || diagnostics.unexpected_native_stops > 0
}

fn diagnostics_failed(baseline: MacOsCaptureDiagnostics, current: MacOsCaptureDiagnostics) -> bool {
    diagnostic_delta(baseline, current)
        .map(|delta| diagnostics_have_terminal_fault(&delta))
        .unwrap_or(true)
}

fn system_audio_diagnostic_delta(
    baseline: MacOsSystemAudioDiagnostics,
    current: MacOsSystemAudioDiagnostics,
) -> Result<MacOsSystemAudioDiagnostics, NativeDesktopBackendError> {
    Ok(MacOsSystemAudioDiagnostics {
        dropped_callback_chunks: current
            .dropped_callback_chunks
            .checked_sub(baseline.dropped_callback_chunks)
            .ok_or(NativeDesktopBackendError::Internal)?,
        callback_chunks_after_stop: current
            .callback_chunks_after_stop
            .checked_sub(baseline.callback_chunks_after_stop)
            .ok_or(NativeDesktopBackendError::Internal)?,
        invalid_callback_chunks: current
            .invalid_callback_chunks
            .checked_sub(baseline.invalid_callback_chunks)
            .ok_or(NativeDesktopBackendError::Internal)?,
        unexpected_native_stops: current
            .unexpected_native_stops
            .checked_sub(baseline.unexpected_native_stops)
            .ok_or(NativeDesktopBackendError::Internal)?,
    })
}

const fn system_audio_diagnostics_have_terminal_fault(
    diagnostics: &MacOsSystemAudioDiagnostics,
) -> bool {
    diagnostics.dropped_callback_chunks > 0
        || diagnostics.invalid_callback_chunks > 0
        || diagnostics.unexpected_native_stops > 0
}

fn system_audio_diagnostics_failed(
    baseline: MacOsSystemAudioDiagnostics,
    current: MacOsSystemAudioDiagnostics,
) -> bool {
    system_audio_diagnostic_delta(baseline, current)
        .map(|delta| system_audio_diagnostics_have_terminal_fault(&delta))
        .unwrap_or(true)
}

fn target_summary(
    token: String,
    ordinal: u16,
    target: &ScreenTargetDescriptor,
    targets: &[ScreenTargetDescriptor],
) -> Result<CaptureTargetSummary, NativeDesktopBackendError> {
    let kind = match target.kind() {
        ScreenTargetKind::Display => CaptureTargetKind::Display,
        ScreenTargetKind::Window => CaptureTargetKind::Window,
        ScreenTargetKind::Region => CaptureTargetKind::Region,
    };
    let (width_pixels, height_pixels, scale_numerator, scale_denominator, rotation_degrees) =
        match target.kind() {
            ScreenTargetKind::Display => {
                let transform = target
                    .display_transform()
                    .ok_or(NativeDesktopBackendError::Internal)?;
                let physical = transform.physical_bounds();
                let scale = transform.scale();
                (
                    physical.width(),
                    physical.height(),
                    u16::try_from(scale.numerator())
                        .map_err(|_| NativeDesktopBackendError::Internal)?,
                    u16::try_from(scale.denominator())
                        .map_err(|_| NativeDesktopBackendError::Internal)?,
                    rotation_degrees(transform),
                )
            }
            ScreenTargetKind::Window => {
                let logical = target.logical_bounds();
                (logical.width(), logical.height(), 1, 1, 0)
            }
            ScreenTargetKind::Region => {
                let display_binding = target
                    .containing_display_binding()
                    .ok_or(NativeDesktopBackendError::Internal)?;
                let transform = targets
                    .iter()
                    .find(|candidate| candidate.binding() == display_binding)
                    .and_then(ScreenTargetDescriptor::display_transform)
                    .ok_or(NativeDesktopBackendError::Internal)?;
                let physical = transform
                    .logical_rect_to_physical(target.logical_bounds())
                    .map_err(|_| NativeDesktopBackendError::Internal)?;
                let scale = transform.scale();
                (
                    physical.width(),
                    physical.height(),
                    u16::try_from(scale.numerator())
                        .map_err(|_| NativeDesktopBackendError::Internal)?,
                    u16::try_from(scale.denominator())
                        .map_err(|_| NativeDesktopBackendError::Internal)?,
                    rotation_degrees(transform),
                )
            }
        };
    let summary = CaptureTargetSummary {
        token,
        kind,
        ordinal,
        width_pixels,
        height_pixels,
        scale_numerator,
        scale_denominator,
        rotation_degrees,
    };
    summary
        .validate()
        .map_err(|_| NativeDesktopBackendError::Internal)?;
    Ok(summary)
}

const fn target_token_prefix(kind: ScreenTargetKind) -> &'static str {
    match kind {
        ScreenTargetKind::Display => "display",
        ScreenTargetKind::Window => "window",
        ScreenTargetKind::Region => "region",
    }
}

const fn rotation_degrees(transform: DisplayGeometryTransform) -> u16 {
    match transform.rotation() {
        Rotation::Degrees0 => 0,
        Rotation::Degrees90 => 90,
        Rotation::Degrees180 => 180,
        Rotation::Degrees270 => 270,
    }
}

fn bounded_recording_dimensions(
    width: u32,
    height: u32,
) -> Result<(u32, u32), NativeDesktopBackendError> {
    if width == 0 || height == 0 {
        return Err(NativeDesktopBackendError::TargetUnavailable);
    }
    let (max_width, max_height) = if width >= height {
        (1_920_u32, 1_080_u32)
    } else {
        (1_080_u32, 1_920_u32)
    };
    if width <= max_width && height <= max_height {
        return Ok((width, height));
    }
    let width_limited =
        u64::from(width) * u64::from(max_height) > u64::from(height) * u64::from(max_width);
    let (numerator, denominator) = if width_limited {
        (u64::from(max_width), u64::from(width))
    } else {
        (u64::from(max_height), u64::from(height))
    };
    let scaled_width = (u64::from(width) * numerator / denominator).max(1);
    let scaled_height = (u64::from(height) * numerator / denominator).max(1);
    Ok((
        u32::try_from(scaled_width).map_err(|_| NativeDesktopBackendError::Internal)?,
        u32::try_from(scaled_height).map_err(|_| NativeDesktopBackendError::Internal)?,
    ))
}

fn bind_or_create_root(
    path: PathBuf,
    private: bool,
) -> Result<(PathBuf, RootedDir), NativeDesktopBackendError> {
    if !path.is_absolute() || path.to_str().is_none() {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    let directory = match RootedDir::bind(&path) {
        Ok(directory) => directory,
        Err(RootedIoError::Io {
            source: Errno::NOENT,
            ..
        }) => {
            let parent = path
                .parent()
                .filter(|parent| parent != &path)
                .ok_or(NativeDesktopBackendError::Filesystem)?;
            let leaf = path
                .file_name()
                .ok_or(NativeDesktopBackendError::Filesystem)?;
            let (_, parent) = bind_or_create_root(parent.to_path_buf(), false)?;
            match parent.create_private_dir(Path::new(leaf)) {
                Ok(directory) => directory,
                Err(RootedIoError::EntryExists) => {
                    RootedDir::bind(&path).map_err(map_rooted_io_error)?
                }
                Err(error) => return Err(map_rooted_io_error(error)),
            }
        }
        Err(error) => return Err(map_rooted_io_error(error)),
    };
    if private {
        directory
            .ensure_private_mode()
            .map_err(map_rooted_io_error)?;
    }
    ensure_visible_directory(&directory, &path)?;
    Ok((path, directory))
}

fn ensure_visible_directory(
    directory: &RootedDir,
    path: &Path,
) -> Result<(), NativeDesktopBackendError> {
    if directory
        .matches_visible_path(path)
        .map_err(map_rooted_io_error)?
    {
        Ok(())
    } else {
        Err(NativeDesktopBackendError::Filesystem)
    }
}

fn path_text(path: &Path) -> Result<String, NativeDesktopBackendError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or(NativeDesktopBackendError::Filesystem)
}

fn sha256_rooted_file(
    source: &mut RootedFile,
    expected_identity: FileIdentity,
    expected_bytes: u64,
) -> Result<String, NativeDesktopBackendError> {
    source
        .file_mut()
        .seek(SeekFrom::Start(0))
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let mut digest = Sha256Context::new(&SHA256);
    let mut buffer = [0_u8; FILE_IO_BUFFER_BYTES];
    let mut total_bytes = 0_u64;
    loop {
        let read = source
            .file_mut()
            .read(&mut buffer)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        if read == 0 {
            break;
        }
        let read_bytes = u64::try_from(read).map_err(|_| NativeDesktopBackendError::Filesystem)?;
        total_bytes = total_bytes
            .checked_add(read_bytes)
            .filter(|total| *total <= expected_bytes)
            .ok_or(NativeDesktopBackendError::Filesystem)?;
        digest.update(&buffer[..read]);
    }
    let refreshed = source.refresh_metadata().map_err(map_rooted_io_error)?;
    if total_bytes != expected_bytes
        || refreshed.identity() != expected_identity
        || refreshed.size_bytes() != expected_bytes
    {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(encode_hex(digest.finish().as_ref()))
}

fn copy_rooted_file_with_sha256(
    source: &mut RootedFile,
    staging: &mut RootedFile,
    expected_source_identity: FileIdentity,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<u64, NativeDesktopBackendError> {
    let staging_identity = staging.metadata().identity();
    let mut digest = Sha256Context::new(&SHA256);
    let mut buffer = [0_u8; FILE_IO_BUFFER_BYTES];
    let mut total_bytes = 0_u64;
    loop {
        let read = source
            .file_mut()
            .read(&mut buffer)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        if read == 0 {
            break;
        }
        let read_bytes = u64::try_from(read).map_err(|_| NativeDesktopBackendError::Filesystem)?;
        total_bytes = total_bytes
            .checked_add(read_bytes)
            .filter(|total| *total <= expected_bytes)
            .ok_or(NativeDesktopBackendError::Filesystem)?;
        staging
            .file_mut()
            .write_all(&buffer[..read])
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        digest.update(&buffer[..read]);
    }

    let source_metadata = source.refresh_metadata().map_err(map_rooted_io_error)?;
    let staging_metadata = staging.refresh_metadata().map_err(map_rooted_io_error)?;
    let copied_sha256 = encode_hex(digest.finish().as_ref());
    if total_bytes != expected_bytes
        || copied_sha256 != expected_sha256
        || source_metadata.identity() != expected_source_identity
        || source_metadata.size_bytes() != expected_bytes
        || staging_metadata.identity() != staging_identity
        || staging_metadata.size_bytes() != total_bytes
    {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(total_bytes)
}

fn verify_published_rooted_file(
    directory: &RootedDir,
    relative: &Path,
    expected_identity: FileIdentity,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<(), NativeDesktopBackendError> {
    let mut published = directory
        .open_regular_file(relative)
        .map_err(map_rooted_io_error)?;
    if published.metadata().identity() != expected_identity
        || published.metadata().size_bytes() != expected_bytes
        || sha256_rooted_file(&mut published, expected_identity, expected_bytes)? != expected_sha256
    {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    drop(published);
    let rebound = directory
        .open_regular_file(relative)
        .map_err(map_rooted_io_error)?;
    if rebound.metadata().identity() != expected_identity
        || rebound.metadata().size_bytes() != expected_bytes
    {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(())
}

fn create_export_staging(
    export_directory: &RootedDir,
    artifact_token: &str,
) -> Result<(PathBuf, RootedFile), NativeDesktopBackendError> {
    let random = SystemRandom::new();
    for _ in 0..MAX_TOKEN_ATTEMPTS {
        let nonce: [u8; TOKEN_RANDOM_BYTES] = random_array(&random)?;
        let relative = PathBuf::from(format!(
            ".frame-export-{artifact_token}-{}.webm",
            encode_hex(&nonce)
        ));
        match export_directory.create_new_file(&relative) {
            Ok(file) => return Ok((relative, file)),
            Err(RootedIoError::EntryExists) => {}
            Err(error) => return Err(map_rooted_io_error(error)),
        }
    }
    Err(NativeDesktopBackendError::Filesystem)
}

fn map_rooted_io_error(_error: RootedIoError) -> NativeDesktopBackendError {
    NativeDesktopBackendError::Filesystem
}

fn new_session_source(
    installation_secret: &[u8; 32],
) -> Result<SessionSource, NativeDesktopBackendError> {
    let random = SystemRandom::new();
    let source_instance = ScreenSourceInstanceId::new(random_array(&random)?)
        .map_err(|_| NativeDesktopBackendError::Internal)?;
    let source_secret = Zeroizing::new(random_array(&random)?);
    let source = MacOsScreenCaptureSource::new(source_instance, *source_secret)
        .map_err(map_capture_error)?;
    let system_audio =
        MacOsSystemAudioSource::new(*installation_secret).map_err(map_system_audio_error)?;
    Ok(SessionSource {
        source,
        system_audio,
        observed_topology_generation: None,
        snapshot: None,
    })
}

fn random_array<const N: usize>(
    random: &SystemRandom,
) -> Result<[u8; N], NativeDesktopBackendError> {
    let mut bytes = [0_u8; N];
    random
        .fill(&mut bytes)
        .map_err(|_| NativeDesktopBackendError::Internal)?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(NativeDesktopBackendError::Internal);
    }
    Ok(bytes)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn map_capture_error(error: MacOsCaptureError) -> NativeDesktopBackendError {
    match error {
        MacOsCaptureError::PermissionDenied => NativeDesktopBackendError::PermissionDenied,
        MacOsCaptureError::AlreadyRunning => NativeDesktopBackendError::Busy,
        MacOsCaptureError::StaleOrForeignTarget => NativeDesktopBackendError::StaleCatalog,
        MacOsCaptureError::TargetNoLongerAvailable | MacOsCaptureError::ContentUnavailable => {
            NativeDesktopBackendError::TargetUnavailable
        }
        MacOsCaptureError::DisplayCatalogUnavailable
        | MacOsCaptureError::ShareableContentUnavailable => NativeDesktopBackendError::Unavailable,
        _ => NativeDesktopBackendError::Internal,
    }
}

const fn map_system_audio_error(error: MacOsSystemAudioError) -> NativeDesktopBackendError {
    match error {
        MacOsSystemAudioError::PermissionDenied => NativeDesktopBackendError::PermissionDenied,
        MacOsSystemAudioError::AlreadyRunning => NativeDesktopBackendError::Busy,
        MacOsSystemAudioError::ShareableContentUnavailable
        | MacOsSystemAudioError::NoDisplayAvailable => NativeDesktopBackendError::Unavailable,
        _ => NativeDesktopBackendError::Internal,
    }
}

const fn system_audio_source_reusable_after_start_error(error: MacOsSystemAudioError) -> bool {
    matches!(
        error,
        MacOsSystemAudioError::PermissionDenied
            | MacOsSystemAudioError::ShareableContentUnavailable
            | MacOsSystemAudioError::NoDisplayAvailable
            | MacOsSystemAudioError::NativeOperationCapacityUnavailable
            | MacOsSystemAudioError::NativeOperationWorkerUnavailable
            | MacOsSystemAudioError::OutputHandlerRegistrationFailed
            | MacOsSystemAudioError::CaptureStartFailed
    )
}

const fn should_attempt_system_audio(requested: bool, permission: PermissionPreflight) -> bool {
    requested && matches!(permission, PermissionPreflight::Granted)
}

const fn all_av_teardown_confirmed(screen: bool, system_audio: bool, recorder: bool) -> bool {
    screen && system_audio && recorder
}

const fn capture_source_reusable_after_start_error(error: MacOsCaptureError) -> bool {
    !matches!(
        error,
        MacOsCaptureError::CaptureStartTeardownUnconfirmed
            | MacOsCaptureError::CaptureTeardownUnconfirmed
    )
}

const fn capture_start_resources_reusable(
    capture_error: MacOsCaptureError,
    recorder_teardown_confirmed: bool,
) -> bool {
    capture_source_reusable_after_start_error(capture_error) && recorder_teardown_confirmed
}

fn abort_unowned_recording(
    recording: ScreenRecording,
    primary_error: NativeDesktopBackendError,
    context: &str,
) -> bool {
    match recording.abort() {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "Frame recorder teardown failed after {context} while preserving primary error {primary_error}: {error}"
            );
            false
        }
    }
}

fn stop_unowned_system_audio(system_audio: &mut MacOsSystemAudioSource, context: &str) -> bool {
    match system_audio.stop_and_drain_chunks() {
        Ok(_) => true,
        Err(error) => {
            let teardown_confirmed = error.capture_teardown_confirmed();
            eprintln!("Frame system-audio teardown failed after {context}: {error}");
            teardown_confirmed
        }
    }
}

fn teardown_worker_start(
    worker_start: WorkerStart,
    primary_error: NativeDesktopBackendError,
    context: &str,
) -> bool {
    match worker_start {
        WorkerStart::ScreenOnly(start) => {
            let teardown_confirmed = start.teardown();
            if !teardown_confirmed {
                eprintln!(
                    "Frame normalized worker teardown failed after {context} while preserving primary error {primary_error}"
                );
            }
            teardown_confirmed
        }
        WorkerStart::ScreenAudio(start) => {
            let ScreenAudioWorkerStart {
                mut source,
                mut system_audio,
                recording,
                ..
            } = *start;
            let (_, screen_teardown_confirmed, _) =
                classify_screen_stop(source.stop_and_drain_frames());
            let (_, audio_teardown_confirmed, _) =
                classify_audio_stop(system_audio.stop_and_drain_chunks());
            let recording_teardown_confirmed = recording.abort().is_ok();
            all_av_teardown_confirmed(
                screen_teardown_confirmed,
                audio_teardown_confirmed,
                recording_teardown_confirmed,
            )
        }
    }
}

#[cfg(test)]
fn map_capture_stop_error(error: MacOsCaptureStopError) -> (NativeDesktopBackendError, bool) {
    let teardown_confirmed = error.capture_teardown_confirmed();
    (
        map_capture_error(error.into_capture_error()),
        teardown_confirmed,
    )
}

#[cfg(test)]
fn failed_worker_teardown_outcome(
    primary_error: NativeDesktopBackendError,
    capture_stopped: Result<(), MacOsCaptureStopError>,
    recording_stopped: Result<(), ScreenRecordingError>,
) -> WorkerOutcome {
    let capture_teardown_confirmed = match capture_stopped {
        Ok(()) => true,
        Err(error) => {
            let teardown_confirmed = error.capture_teardown_confirmed();
            eprintln!(
                "Frame native capture teardown failed while preserving primary error {primary_error}: {error}"
            );
            teardown_confirmed
        }
    };
    let recording_teardown_confirmed = match recording_stopped {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "Frame recorder teardown failed while preserving primary error {primary_error}: {error}"
            );
            false
        }
    };
    WorkerOutcome::Failed {
        error: primary_error,
        teardown_confirmed: capture_teardown_confirmed && recording_teardown_confirmed,
    }
}

#[cfg(test)]
fn capture_stop_failure_outcome(
    error: MacOsCaptureStopError,
    recording_teardown_confirmed: bool,
) -> WorkerOutcome {
    let (error, capture_teardown_confirmed) = map_capture_stop_error(error);
    WorkerOutcome::Failed {
        error,
        teardown_confirmed: capture_teardown_confirmed && recording_teardown_confirmed,
    }
}

fn map_recording_error(error: ScreenRecordingError) -> NativeDesktopBackendError {
    match error {
        ScreenRecordingError::Cancelled => NativeDesktopBackendError::Cancelled,
        ScreenRecordingError::OutputExists
        | ScreenRecordingError::OutputOwnership
        | ScreenRecordingError::ResourceLimit
        | ScreenRecordingError::Filesystem(_) => NativeDesktopBackendError::Filesystem,
        ScreenRecordingError::MissingFactory
        | ScreenRecordingError::Runtime(_)
        | ScreenRecordingError::UntrustedFactory => NativeDesktopBackendError::Unavailable,
        _ => NativeDesktopBackendError::Internal,
    }
}

const fn audio_graph_unavailable(error: &ScreenRecordingError) -> bool {
    matches!(
        error,
        ScreenRecordingError::MissingFactory
            | ScreenRecordingError::Runtime(_)
            | ScreenRecordingError::UntrustedFactory
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::symlink;
    use std::time::Instant;

    use crate::{PathPolicy, RootAccess};

    use super::*;

    const EXPORT_FIXTURE_BYTES: &[u8] = b"verified editable WebM fixture";
    const TEST_INSTALLATION_SECRET: [u8; 32] = [0x5a; 32];
    const WORKER_COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);
    const WORKER_COMPLETION_POLL: Duration = Duration::from_millis(1);

    fn sha256_bytes(bytes: &[u8]) -> String {
        let mut digest = Sha256Context::new(&SHA256);
        digest.update(bytes);
        encode_hex(digest.finish().as_ref())
    }

    fn wait_for_terminal_failure(
        backend: &mut MacOsNativeDesktopBackend,
        request: &NativeRecordingControlRequest,
        context: &str,
    ) -> NativeRecordingTerminalFailure {
        let deadline = Instant::now()
            .checked_add(WORKER_COMPLETION_TIMEOUT)
            .expect("bounded test deadline");
        loop {
            if let Some(failure) = backend
                .poll_recording_terminal_failure(request)
                .expect(context)
            {
                return failure;
            }
            let now = Instant::now();
            assert!(now < deadline, "{context} timed out");
            thread::park_timeout(
                WORKER_COMPLETION_POLL.min(deadline.saturating_duration_since(now)),
            );
        }
    }

    fn assert_send<T: Send>() {}

    #[test]
    fn finish_error_classification_preserves_teardown_authority() {
        let teardown_only =
            ScreenRecordingError::TeardownUnconfirmed(Box::new(ScreenRecordingError::Pipeline));
        let operation_and_teardown = ScreenRecordingError::OperationAndTeardown {
            operation: Box::new(ScreenRecordingError::Timeout),
            teardown: Box::new(ScreenRecordingError::Pipeline),
        };

        assert!(!recording_finish_teardown_confirmed(&teardown_only));
        assert!(!recording_finish_teardown_confirmed(
            &operation_and_teardown,
        ));
        assert!(recording_finish_teardown_confirmed(
            &ScreenRecordingError::InvalidOutput,
        ));
    }

    #[test]
    fn audio_graph_fallback_is_limited_to_runtime_availability_failures() {
        assert!(audio_graph_unavailable(
            &ScreenRecordingError::MissingFactory
        ));
        assert!(audio_graph_unavailable(
            &ScreenRecordingError::UntrustedFactory
        ));
        assert!(!audio_graph_unavailable(&ScreenRecordingError::Pipeline));
        assert!(!audio_graph_unavailable(&ScreenRecordingError::Filesystem(
            std::io::Error::other("test")
        )));
        assert!(!audio_graph_unavailable(
            &ScreenRecordingError::ResourceLimit
        ));
    }

    #[test]
    fn capture_stop_error_classification_preserves_full_teardown_authority() {
        let native = capture_stop_failure_outcome(
            MacOsCaptureStopError::NativeStopUnconfirmed(MacOsCaptureError::CaptureStopFailed),
            true,
        );
        let tail_without_recorder = capture_stop_failure_outcome(
            MacOsCaptureStopError::TailProcessingFailed(MacOsCaptureError::InvalidSampleBuffer),
            false,
        );
        let callbacks_with_recorder = capture_stop_failure_outcome(
            MacOsCaptureStopError::CallbackQuiescenceUnconfirmed(
                MacOsCaptureError::OutputHandlerRemovalFailed,
            ),
            true,
        );
        let capture_with_recorder = capture_stop_failure_outcome(
            MacOsCaptureStopError::CaptureFailedAfterTeardown(
                MacOsCaptureError::UnexpectedStreamStop,
            ),
            true,
        );
        let tail_with_recorder = capture_stop_failure_outcome(
            MacOsCaptureStopError::TailProcessingFailed(MacOsCaptureError::InvalidSampleBuffer),
            true,
        );

        for outcome in [native, callbacks_with_recorder, tail_without_recorder] {
            assert!(matches!(
                outcome,
                WorkerOutcome::Failed {
                    error: NativeDesktopBackendError::Internal,
                    teardown_confirmed: false,
                }
            ));
        }
        assert!(matches!(
            capture_with_recorder,
            WorkerOutcome::Failed {
                error: NativeDesktopBackendError::Internal,
                teardown_confirmed: true,
            }
        ));
        assert!(matches!(
            tail_with_recorder,
            WorkerOutcome::Failed {
                error: NativeDesktopBackendError::Internal,
                teardown_confirmed: true,
            }
        ));
    }

    #[test]
    fn capture_start_reuse_requires_both_capture_and_recorder_teardown() {
        assert!(!capture_source_reusable_after_start_error(
            MacOsCaptureError::CaptureStartTeardownUnconfirmed,
        ));
        assert!(!capture_source_reusable_after_start_error(
            MacOsCaptureError::CaptureTeardownUnconfirmed,
        ));
        assert!(capture_source_reusable_after_start_error(
            MacOsCaptureError::PermissionDenied,
        ));
        assert!(capture_start_resources_reusable(
            MacOsCaptureError::PermissionDenied,
            true,
        ));
        assert!(!capture_start_resources_reusable(
            MacOsCaptureError::PermissionDenied,
            false,
        ));
        assert!(!capture_start_resources_reusable(
            MacOsCaptureError::CaptureStartTeardownUnconfirmed,
            true,
        ));

        assert!(!should_attempt_system_audio(
            false,
            PermissionPreflight::Granted
        ));
        assert!(!should_attempt_system_audio(
            true,
            PermissionPreflight::PromptRequired
        ));
        assert!(should_attempt_system_audio(
            true,
            PermissionPreflight::Granted
        ));
        assert!(system_audio_source_reusable_after_start_error(
            MacOsSystemAudioError::PermissionDenied
        ));
        assert!(system_audio_source_reusable_after_start_error(
            MacOsSystemAudioError::ShareableContentUnavailable
        ));
        assert!(!system_audio_source_reusable_after_start_error(
            MacOsSystemAudioError::AlreadyRunning
        ));
        assert!(!system_audio_source_reusable_after_start_error(
            MacOsSystemAudioError::CaptureStartTeardownUnconfirmed
        ));
    }

    #[test]
    fn av_teardown_confirmation_requires_both_native_sources_and_recorder() {
        for screen in [false, true] {
            for system_audio in [false, true] {
                for recorder in [false, true] {
                    assert_eq!(
                        all_av_teardown_confirmed(screen, system_audio, recorder),
                        screen && system_audio && recorder
                    );
                }
            }
        }
    }

    #[test]
    fn worker_failure_preserves_primary_error_across_teardown_matrix() {
        let primary_error = NativeDesktopBackendError::TargetUnavailable;
        let outcomes = [
            failed_worker_teardown_outcome(primary_error, Ok(()), Ok(())),
            failed_worker_teardown_outcome(
                primary_error,
                Err(MacOsCaptureStopError::NativeStopUnconfirmed(
                    MacOsCaptureError::CaptureStopFailed,
                )),
                Ok(()),
            ),
            failed_worker_teardown_outcome(
                primary_error,
                Err(MacOsCaptureStopError::CallbackQuiescenceUnconfirmed(
                    MacOsCaptureError::OutputHandlerRemovalFailed,
                )),
                Ok(()),
            ),
            failed_worker_teardown_outcome(
                primary_error,
                Err(MacOsCaptureStopError::TailProcessingFailed(
                    MacOsCaptureError::InvalidSampleBuffer,
                )),
                Ok(()),
            ),
            failed_worker_teardown_outcome(
                primary_error,
                Err(MacOsCaptureStopError::CaptureFailedAfterTeardown(
                    MacOsCaptureError::UnexpectedStreamStop,
                )),
                Ok(()),
            ),
            failed_worker_teardown_outcome(
                primary_error,
                Ok(()),
                Err(ScreenRecordingError::Pipeline),
            ),
            failed_worker_teardown_outcome(
                primary_error,
                Err(MacOsCaptureStopError::TailProcessingFailed(
                    MacOsCaptureError::InvalidSampleBuffer,
                )),
                Err(ScreenRecordingError::Pipeline),
            ),
            failed_worker_teardown_outcome(
                primary_error,
                Err(MacOsCaptureStopError::NativeStopUnconfirmed(
                    MacOsCaptureError::CaptureStopFailed,
                )),
                Err(ScreenRecordingError::Pipeline),
            ),
        ];
        let expected_teardown = [true, false, false, true, true, false, false, false];

        for (outcome, expected_teardown) in outcomes.into_iter().zip(expected_teardown) {
            let WorkerOutcome::Failed {
                error,
                teardown_confirmed,
            } = outcome
            else {
                panic!("worker failure teardown must remain failed");
            };
            assert_eq!(error, primary_error);
            assert_eq!(teardown_confirmed, expected_teardown);
        }
    }

    fn test_rooted_directory() -> RootedDir {
        RootedDir::bind("/private/tmp").expect("test root should bind without symlinks")
    }

    fn create_pending_output(
        backend: &MacOsNativeDesktopBackend,
        label: &str,
        bytes: &[u8],
    ) -> PendingRecordingOutput {
        let nonce: [u8; TOKEN_RANDOM_BYTES] =
            random_array(&SystemRandom::new()).expect("test random nonce");
        let suffix = encode_hex(&nonce);
        let staging_relative = PathBuf::from(format!(".{label}-{suffix}.partial"));
        let final_relative = PathBuf::from(format!("{label}-{suffix}.webm"));
        let mut file = backend
            .recordings_directory
            .create_new_file(&staging_relative)
            .expect("create pending recording output");
        file.file_mut()
            .write_all(bytes)
            .expect("write pending recording output");
        file.sync().expect("sync pending recording output");
        let identity = file
            .refresh_metadata()
            .expect("refresh pending recording output")
            .identity();
        drop(file);
        PendingRecordingOutput {
            staging_relative,
            final_relative,
            identity,
        }
    }

    fn install_failed_worker(
        backend: &mut MacOsNativeDesktopBackend,
        label: &str,
        error: NativeDesktopBackendError,
    ) -> NativeRecordingControlRequest {
        let output = create_pending_output(backend, label, b"unconfirmed worker output");
        let recording_token = format!("recording-{label}");
        let (control, receiver) = sync_channel(WORKER_CONTROL_CAPACITY);
        let worker = thread::spawn(move || {
            let _ = receiver.recv();
            WorkerCompletion {
                outcome: WorkerOutcome::Failed {
                    error,
                    teardown_confirmed: false,
                },
            }
        });
        backend.capture = CaptureLifecycle::Recording(ActiveRecording {
            token: recording_token.clone(),
            control,
            worker,
            output,
            system_audio_meter: Arc::new(AtomicU16::new(0)),
        });
        NativeRecordingControlRequest { recording_token }
    }

    struct ExportFixture {
        root: PathBuf,
        source: PathBuf,
        output: PathBuf,
        backend: MacOsNativeDesktopBackend,
        request: NativeEditableWebmExportRequest,
    }

    impl ExportFixture {
        fn new(label: &str) -> Self {
            let nonce: [u8; TOKEN_RANDOM_BYTES] =
                random_array(&SystemRandom::new()).expect("test random nonce");
            let root = PathBuf::from("/private/tmp").join(format!(
                "frame-native-export-{label}-{}",
                encode_hex(&nonce)
            ));
            let media_root = root.join("media");
            let export_root = root.join("exports");
            fs::create_dir(&root).expect("fixture root");
            fs::create_dir(&media_root).expect("fixture media root");
            fs::create_dir(&export_root).expect("fixture export root");
            let media_directory = RootedDir::bind(&media_root).expect("bind media root");
            let recordings_root = media_root.join(RECORDINGS_DIRECTORY);
            let recordings_directory = media_directory
                .create_private_dir("recordings")
                .expect("create recordings root");
            let export_directory = RootedDir::bind(&export_root).expect("bind export root");
            let export_staging_root = export_root.join(EXPORT_STAGING_DIRECTORY);
            let export_staging_directory = export_directory
                .create_private_dir(EXPORT_STAGING_DIRECTORY)
                .expect("create export staging root");
            let source_relative = PathBuf::from("recording-token-test.webm");
            let source = recordings_root.join(&source_relative);
            let mut source_file = recordings_directory
                .create_new_file(&source_relative)
                .expect("create source artifact");
            source_file
                .file_mut()
                .write_all(EXPORT_FIXTURE_BYTES)
                .expect("write source artifact");
            source_file.sync().expect("sync source artifact");
            let source_metadata = source_file
                .refresh_metadata()
                .expect("refresh source metadata");
            drop(source_file);

            let export_relative = PathBuf::from("Frame-artifact-token-test.webm");
            let output = export_root.join(&export_relative);
            let policy = PathPolicy::empty()
                .allow_root(
                    &media_root,
                    RootAccess {
                        read: true,
                        write: false,
                        delete: false,
                    },
                )
                .expect("media policy")
                .allow_root(
                    &export_root,
                    RootAccess {
                        read: false,
                        write: true,
                        delete: false,
                    },
                )
                .expect("export policy");
            let request = NativeEditableWebmExportRequest {
                artifact_token: "artifact-token-test".into(),
                artifact_revision: 1,
                source_media_path: policy
                    .validate(
                        source.to_str().expect("utf-8 source path"),
                        PathUse::MediaRead,
                    )
                    .expect("validated source path"),
                output_path: policy
                    .validate(
                        output.to_str().expect("utf-8 output path"),
                        PathUse::ExportWrite,
                    )
                    .expect("validated output path"),
            };
            let backend = MacOsNativeDesktopBackend {
                capture: CaptureLifecycle::Ready(Box::new(
                    new_session_source(&TEST_INSTALLATION_SECRET).expect("test source"),
                )),
                installation_secret: Zeroizing::new(TEST_INSTALLATION_SECRET),
                media_root,
                media_directory,
                recordings_root,
                recordings_directory,
                export_root,
                export_directory,
                export_staging_root,
                export_staging_directory,
                catalog_generation: 1,
                stable_tokens: BTreeMap::new(),
                catalog: BTreeMap::new(),
                selected_token: None,
                artifact_revision: 1,
                artifact: Some(StoredArtifact {
                    token: "artifact-token-test".into(),
                    revision: 1,
                    source: source.clone(),
                    source_relative,
                    source_identity: source_metadata.identity(),
                    source_bytes: source_metadata.size_bytes(),
                    source_sha256: sha256_bytes(EXPORT_FIXTURE_BYTES),
                    export: output.clone(),
                    export_relative,
                }),
            };
            Self {
                root,
                source,
                output,
                backend,
                request,
            }
        }
    }

    impl Drop for ExportFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn backend_and_worker_are_sendable() {
        assert_send::<MacOsNativeDesktopBackend>();
        assert_send::<ScreenRecording>();
    }

    #[test]
    fn dimensions_preserve_orientation_inside_the_1080p_ceiling() {
        assert_eq!(
            bounded_recording_dimensions(1_920, 1_080),
            Ok((1_920, 1_080))
        );
        assert_eq!(
            bounded_recording_dimensions(1_080, 1_920),
            Ok((1_080, 1_920))
        );
        assert_eq!(
            bounded_recording_dimensions(3_840, 2_160),
            Ok((1_920, 1_080))
        );
        assert_eq!(
            bounded_recording_dimensions(2_160, 3_840),
            Ok((1_080, 1_920))
        );
    }

    #[test]
    fn zero_sized_native_geometry_fails_closed() {
        assert_eq!(
            bounded_recording_dimensions(0, 1_080),
            Err(NativeDesktopBackendError::TargetUnavailable)
        );
    }

    #[test]
    fn diagnostics_are_session_deltas_and_only_loss_faults_are_terminal() {
        let baseline = MacOsCaptureDiagnostics {
            dropped_callback_frames: 3,
            callback_frames_after_stop: 5,
            ignored_non_content_samples: 7,
            invalid_samples: 11,
            duration_fallbacks: 13,
            timestamp_discontinuities: 17,
            unexpected_native_stops: 19,
        };
        let non_terminal = MacOsCaptureDiagnostics {
            dropped_callback_frames: 3,
            callback_frames_after_stop: 6,
            ignored_non_content_samples: 9,
            invalid_samples: 11,
            duration_fallbacks: 16,
            timestamp_discontinuities: 21,
            unexpected_native_stops: 19,
        };
        let delta = diagnostic_delta(baseline, non_terminal).expect("monotonic delta");
        assert_eq!(delta.callback_frames_after_stop, 1);
        assert_eq!(delta.ignored_non_content_samples, 2);
        assert_eq!(delta.duration_fallbacks, 3);
        assert_eq!(delta.timestamp_discontinuities, 4);
        assert!(!diagnostics_have_terminal_fault(&delta));

        for terminal in [
            MacOsCaptureDiagnostics {
                dropped_callback_frames: 4,
                ..non_terminal
            },
            MacOsCaptureDiagnostics {
                invalid_samples: 12,
                ..non_terminal
            },
            MacOsCaptureDiagnostics {
                unexpected_native_stops: 20,
                ..non_terminal
            },
        ] {
            assert!(diagnostics_failed(baseline, terminal));
        }
        assert_eq!(
            diagnostic_delta(non_terminal, baseline),
            Err(NativeDesktopBackendError::Internal)
        );
    }

    #[test]
    fn confirmed_retirement_rotates_identity_and_unconfirmed_retirement_poisoned() {
        let session = new_session_source(&TEST_INSTALLATION_SECRET).expect("first source");
        let first_identity = session.source.source_instance();
        let mut backend = MacOsNativeDesktopBackend {
            capture: CaptureLifecycle::Ready(Box::new(session)),
            installation_secret: Zeroizing::new(TEST_INSTALLATION_SECRET),
            media_root: PathBuf::from("/private/tmp"),
            media_directory: test_rooted_directory(),
            recordings_root: PathBuf::from("/private/tmp"),
            recordings_directory: test_rooted_directory(),
            export_root: PathBuf::from("/private/tmp"),
            export_directory: test_rooted_directory(),
            export_staging_root: PathBuf::from("/private/tmp"),
            export_staging_directory: test_rooted_directory(),
            catalog_generation: 7,
            stable_tokens: BTreeMap::new(),
            catalog: BTreeMap::new(),
            selected_token: Some("display-stale-token".into()),
            artifact_revision: 0,
            artifact: None,
        };

        assert!(backend.retire_session(true));
        assert_eq!(backend.catalog_generation, 8);
        assert!(backend.selected_token.is_none());
        let CaptureLifecycle::Ready(session) = &backend.capture else {
            panic!("confirmed teardown must install a ready source");
        };
        assert_ne!(session.source.source_instance(), first_identity);

        assert!(!backend.retire_session(false));
        assert_eq!(backend.catalog_generation, 9);
        assert!(matches!(backend.capture, CaptureLifecycle::Poisoned));
    }

    #[test]
    fn wrong_recording_token_does_not_consume_active_worker() {
        let nonce: [u8; TOKEN_RANDOM_BYTES] =
            random_array(&SystemRandom::new()).expect("test random nonce");
        let staging_relative = PathBuf::from(format!(
            "frame-worker-token-test-{}.partial",
            encode_hex(&nonce)
        ));
        let recordings_directory = test_rooted_directory();
        let staging = recordings_directory
            .create_new_file(&staging_relative)
            .expect("pending worker output");
        let output = PendingRecordingOutput {
            final_relative: PathBuf::from(format!(
                "frame-worker-token-test-{}.webm",
                encode_hex(&nonce)
            )),
            staging_relative,
            identity: staging.metadata().identity(),
        };
        drop(staging);
        let (control, receiver) = sync_channel(WORKER_CONTROL_CAPACITY);
        let worker = thread::spawn(move || {
            drop(receiver);
            WorkerCompletion {
                outcome: WorkerOutcome::Cancelled,
            }
        });
        let mut backend = MacOsNativeDesktopBackend {
            capture: CaptureLifecycle::Recording(ActiveRecording {
                token: "recording-correct-token".into(),
                control,
                worker,
                output,
                system_audio_meter: Arc::new(AtomicU16::new(0)),
            }),
            installation_secret: Zeroizing::new(TEST_INSTALLATION_SECRET),
            media_root: PathBuf::from("/private/tmp"),
            media_directory: test_rooted_directory(),
            recordings_root: PathBuf::from("/private/tmp"),
            recordings_directory,
            export_root: PathBuf::from("/private/tmp"),
            export_directory: test_rooted_directory(),
            export_staging_root: PathBuf::from("/private/tmp"),
            export_staging_directory: test_rooted_directory(),
            catalog_generation: 4,
            stable_tokens: BTreeMap::new(),
            catalog: BTreeMap::new(),
            selected_token: None,
            artifact_revision: 0,
            artifact: None,
        };

        assert!(matches!(
            backend.take_worker("recording-wrong-token", WorkerControl::Stop),
            Err(NativeDesktopBackendError::StaleCatalog)
        ));
        assert!(matches!(backend.capture, CaptureLifecycle::Recording(_)));
        let (token, output, completion) = backend
            .take_worker("recording-correct-token", WorkerControl::Cancel)
            .expect("correct token consumes worker");
        assert_eq!(token, "recording-correct-token");
        assert!(matches!(completion.outcome, WorkerOutcome::Cancelled));
        backend.cleanup_recording_output(&output);
    }

    #[test]
    fn live_worker_poll_is_observational_and_terminal_poll_cleans_output() {
        let mut fixture = ExportFixture::new("worker-poll");
        let output = create_pending_output(&fixture.backend, "worker-poll", b"partial pixels");
        let staging_path = fixture
            .backend
            .recordings_root
            .join(&output.staging_relative);
        let (control, receiver) = sync_channel(WORKER_CONTROL_CAPACITY);
        let worker = thread::spawn(move || {
            let _ = receiver.recv();
            WorkerCompletion {
                outcome: WorkerOutcome::Failed {
                    error: NativeDesktopBackendError::Filesystem,
                    teardown_confirmed: true,
                },
            }
        });
        fixture.backend.capture = CaptureLifecycle::Recording(ActiveRecording {
            token: "recording-poll-token".into(),
            control,
            worker,
            output,
            system_audio_meter: Arc::new(AtomicU16::new(0)),
        });
        let request = NativeRecordingControlRequest {
            recording_token: "recording-poll-token".into(),
        };

        assert_eq!(
            fixture.backend.poll_recording_terminal_failure(&request),
            Ok(None)
        );
        let CaptureLifecycle::Recording(active) = &fixture.backend.capture else {
            panic!("a live worker poll must preserve capture authority");
        };
        active
            .control
            .try_send(WorkerControl::Cancel)
            .expect("release worker fixture");

        let failure =
            wait_for_terminal_failure(&mut fixture.backend, &request, "bounded worker poll");
        assert_eq!(failure.recording_token, "recording-poll-token");
        assert_eq!(failure.error, NativeDesktopBackendError::Filesystem);
        assert!(failure.teardown_confirmed);
        assert!(!staging_path.exists());
        assert!(matches!(
            fixture.backend.capture,
            CaptureLifecycle::Ready(_)
        ));
    }

    #[test]
    fn unconfirmed_retirement_preserves_primary_errors_across_terminal_controls() {
        let mut polled = ExportFixture::new("worker-poll-unconfirmed");
        let request = install_failed_worker(
            &mut polled.backend,
            "poll-unconfirmed",
            NativeDesktopBackendError::Filesystem,
        );
        let CaptureLifecycle::Recording(active) = &polled.backend.capture else {
            panic!("poll fixture must own the active worker");
        };
        active
            .control
            .try_send(WorkerControl::Cancel)
            .expect("release poll worker fixture");
        let failure =
            wait_for_terminal_failure(&mut polled.backend, &request, "poll failed worker");
        assert_eq!(failure.error, NativeDesktopBackendError::Filesystem);
        assert!(!failure.teardown_confirmed);
        assert!(matches!(polled.backend.capture, CaptureLifecycle::Poisoned));

        let mut stopped = ExportFixture::new("worker-stop-unconfirmed");
        let request = install_failed_worker(
            &mut stopped.backend,
            "stop-unconfirmed",
            NativeDesktopBackendError::TargetUnavailable,
        );
        let outcome = stopped
            .backend
            .stop_recording(&request)
            .expect("stop failed worker");
        let NativeRecordingStopOutcome::Failed(failure) = outcome else {
            panic!("unconfirmed worker stop must fail");
        };
        assert_eq!(failure.error, NativeDesktopBackendError::TargetUnavailable);
        assert!(!failure.teardown_confirmed);
        assert!(matches!(
            stopped.backend.capture,
            CaptureLifecycle::Poisoned
        ));

        let mut cancelled = ExportFixture::new("worker-cancel-unconfirmed");
        let request = install_failed_worker(
            &mut cancelled.backend,
            "cancel-unconfirmed",
            NativeDesktopBackendError::Internal,
        );
        let outcome = cancelled
            .backend
            .cancel_recording(&request)
            .expect("cancel failed worker");
        let NativeRecordingCancelOutcome::Failed(failure) = outcome else {
            panic!("unconfirmed worker cancellation must fail");
        };
        assert_eq!(failure.error, NativeDesktopBackendError::Internal);
        assert!(!failure.teardown_confirmed);
        assert!(matches!(
            cancelled.backend.capture,
            CaptureLifecycle::Poisoned
        ));
    }

    #[test]
    fn verified_preopened_recording_is_published_only_after_finish() {
        let fixture = ExportFixture::new("recording-publish");
        let bytes = b"verified descriptor recording";
        let output = create_pending_output(&fixture.backend, "recording-publish", bytes);
        let staging_path = fixture
            .backend
            .recordings_root
            .join(&output.staging_relative);
        let final_path = fixture.backend.recordings_root.join(&output.final_relative);
        let artifact: CompletedRecordingArtifact = ScreenRecordingArtifact {
            path: final_path.clone(),
            bytes: u64::try_from(bytes.len()).expect("bounded fixture"),
            sha256: sha256_bytes(bytes),
            submitted_frames: 1,
            encoded_frames: 1,
            first_pts_ns: 0,
            end_pts_ns: 33_333_333,
            encoded_duration_ns: 33_333_333,
        }
        .into();

        assert!(!final_path.exists());
        fixture
            .backend
            .publish_recording_artifact(&output, &artifact)
            .expect("publish verified recording identity");
        assert!(!staging_path.exists());
        assert_eq!(fs::read(&final_path).expect("published recording"), bytes);
        fixture.backend.cleanup_recording_output(&output);
        assert!(!final_path.exists());
    }

    #[test]
    fn rooted_export_copies_and_atomically_publishes_the_sealed_identity() {
        let mut fixture = ExportFixture::new("success");
        let outcome = fixture
            .backend
            .export_editable_webm(&fixture.request)
            .expect("rooted export");
        assert_eq!(outcome.artifact_token, "artifact-token-test");
        assert_eq!(outcome.artifact_revision, 1);
        assert_eq!(outcome.bytes_written, 30);
        assert_eq!(
            fs::read(&fixture.output).expect("published export"),
            EXPORT_FIXTURE_BYTES
        );
    }

    #[test]
    fn sealing_recomputes_and_retains_the_recorder_digest() {
        let mut fixture = ExportFixture::new("seal-digest");
        fixture.backend.artifact = None;
        fixture.backend.artifact_revision = 0;
        let source_sha256 = sha256_bytes(EXPORT_FIXTURE_BYTES);
        let artifact = ScreenRecordingArtifact {
            path: fixture.source.clone(),
            bytes: u64::try_from(EXPORT_FIXTURE_BYTES.len()).expect("fixture length"),
            sha256: source_sha256.clone(),
            submitted_frames: 2,
            encoded_frames: 2,
            first_pts_ns: 0,
            end_pts_ns: 1_000_000_000,
            encoded_duration_ns: 1_000_000_000,
        };

        fixture
            .backend
            .seal_artifact("recording-token-test".into(), artifact.into())
            .expect("matching rooted digest seals");
        let stored = fixture.backend.artifact.as_ref().expect("sealed artifact");
        assert_eq!(stored.source_sha256, source_sha256);
        assert_eq!(
            stored.source_bytes,
            u64::try_from(EXPORT_FIXTURE_BYTES.len()).expect("fixture length")
        );
    }

    #[test]
    fn sealing_rejects_a_same_inode_equal_size_digest_mismatch() {
        let mut fixture = ExportFixture::new("seal-mutation");
        let sealed = fixture
            .backend
            .artifact
            .as_ref()
            .expect("sealed fixture artifact")
            .clone();
        fixture.backend.artifact = None;
        fixture.backend.artifact_revision = 0;
        fs::write(&fixture.source, vec![b'x'; EXPORT_FIXTURE_BYTES.len()])
            .expect("overwrite source in place");
        let mutated = fixture
            .backend
            .recordings_directory
            .open_regular_file(&sealed.source_relative)
            .expect("reopen mutated source");
        assert_eq!(mutated.metadata().identity(), sealed.source_identity);
        assert_eq!(mutated.metadata().size_bytes(), sealed.source_bytes);
        let artifact = ScreenRecordingArtifact {
            path: fixture.source.clone(),
            bytes: sealed.source_bytes,
            sha256: sealed.source_sha256,
            submitted_frames: 2,
            encoded_frames: 2,
            first_pts_ns: 0,
            end_pts_ns: 1_000_000_000,
            encoded_duration_ns: 1_000_000_000,
        };

        assert_eq!(
            fixture
                .backend
                .seal_artifact("recording-token-test".into(), artifact.into()),
            Err(NativeDesktopBackendError::Filesystem)
        );
        assert!(fixture.backend.artifact.is_none());
    }

    #[test]
    fn rooted_export_rejects_same_inode_equal_size_source_mutation() {
        let mut fixture = ExportFixture::new("same-inode-mutation");
        let sealed = fixture
            .backend
            .artifact
            .as_ref()
            .expect("sealed fixture artifact")
            .clone();
        fs::write(&fixture.source, vec![b'x'; EXPORT_FIXTURE_BYTES.len()])
            .expect("overwrite source in place");
        let mutated = fixture
            .backend
            .recordings_directory
            .open_regular_file(&sealed.source_relative)
            .expect("reopen mutated source");
        assert_eq!(mutated.metadata().identity(), sealed.source_identity);
        assert_eq!(mutated.metadata().size_bytes(), sealed.source_bytes);

        assert_eq!(
            fixture.backend.export_editable_webm(&fixture.request),
            Err(NativeDesktopBackendError::Filesystem)
        );
        assert!(!fixture.output.exists());
        assert_eq!(
            fs::read_dir(&fixture.backend.export_staging_root)
                .expect("read private export staging root")
                .count(),
            0,
            "digest failure must remove its private staging file"
        );
    }

    #[test]
    fn rooted_export_rejects_replaced_source_and_existing_destination() {
        let mut replaced = ExportFixture::new("replaced-source");
        fs::rename(&replaced.source, replaced.source.with_extension("old"))
            .expect("retire original source path");
        fs::write(&replaced.source, b"replacement").expect("install replacement source");
        assert_eq!(
            replaced.backend.export_editable_webm(&replaced.request),
            Err(NativeDesktopBackendError::Filesystem)
        );
        assert!(!replaced.output.exists());

        let mut existing = ExportFixture::new("existing-output");
        fs::write(&existing.output, b"keep existing output").expect("existing output");
        assert_eq!(
            existing.backend.export_editable_webm(&existing.request),
            Err(NativeDesktopBackendError::Filesystem)
        );
        assert_eq!(
            fs::read(&existing.output).expect("preserved output"),
            b"keep existing output"
        );
        assert_eq!(
            fs::read_dir(&existing.backend.export_staging_root)
                .expect("read private export staging root")
                .count(),
            0,
            "destination conflict must not leak a private staging file"
        );
    }

    #[test]
    fn rooted_export_rejects_visible_directory_replacements() {
        let mut recordings = ExportFixture::new("replaced-recordings-root");
        let moved_recordings = recordings.backend.media_root.join("recordings-moved");
        fs::rename(&recordings.backend.recordings_root, &moved_recordings)
            .expect("move pinned recordings root");
        fs::create_dir(&recordings.backend.recordings_root)
            .expect("install replacement recordings root");
        assert_eq!(
            recordings.backend.export_editable_webm(&recordings.request),
            Err(NativeDesktopBackendError::Filesystem)
        );
        assert!(!recordings.output.exists());

        let mut exports = ExportFixture::new("replaced-export-root");
        let moved_exports = exports.root.join("exports-moved");
        fs::rename(&exports.backend.export_root, &moved_exports).expect("move pinned export root");
        fs::create_dir(&exports.backend.export_root).expect("install replacement export root");
        assert_eq!(
            exports.backend.export_editable_webm(&exports.request),
            Err(NativeDesktopBackendError::Filesystem)
        );
        assert!(!exports.output.exists());

        let mut staging = ExportFixture::new("replaced-export-staging-root");
        let moved_staging = staging.backend.export_root.join(".frame-staging-moved");
        fs::rename(&staging.backend.export_staging_root, &moved_staging)
            .expect("move pinned export staging root");
        fs::create_dir(&staging.backend.export_staging_root)
            .expect("install replacement export staging root");
        assert_eq!(
            staging.backend.export_editable_webm(&staging.request),
            Err(NativeDesktopBackendError::Filesystem)
        );
        assert!(!staging.output.exists());
    }

    #[test]
    fn retained_export_descriptor_detects_equal_size_post_publish_mutation() {
        let fixture = ExportFixture::new("post-publish-mutation");
        let (staging_relative, mut staging) =
            create_export_staging(&fixture.backend.export_staging_directory, "mutation")
                .expect("create private export staging file");
        staging
            .file_mut()
            .write_all(EXPORT_FIXTURE_BYTES)
            .expect("write private export staging file");
        staging.sync().expect("sync private export staging file");
        let metadata = staging
            .refresh_metadata()
            .expect("refresh private export staging file");
        let output_relative = PathBuf::from("mutation.webm");
        fixture
            .backend
            .export_staging_directory
            .publish_file_to_root_if_identity(
                &staging_relative,
                metadata.identity(),
                &fixture.backend.export_directory,
                &output_relative,
            )
            .expect("publish retained staging identity");
        let output = fixture.backend.export_root.join(&output_relative);
        fs::write(&output, vec![b'x'; EXPORT_FIXTURE_BYTES.len()])
            .expect("overwrite published inode with equal-size bytes");

        assert_eq!(
            verify_published_rooted_file(
                &fixture.backend.export_directory,
                &output_relative,
                metadata.identity(),
                metadata.size_bytes(),
                &sha256_bytes(EXPORT_FIXTURE_BYTES),
            ),
            Err(NativeDesktopBackendError::Filesystem)
        );
        fixture
            .backend
            .export_directory
            .cleanup_file_if_identity(&output_relative, metadata.identity())
            .expect("clean mutated published fixture");
    }

    #[test]
    fn published_export_verification_rejects_final_leaf_replacement() {
        let fixture = ExportFixture::new("post-publish-replacement");
        let (staging_relative, mut staging) =
            create_export_staging(&fixture.backend.export_staging_directory, "replacement")
                .expect("create private export staging file");
        staging
            .file_mut()
            .write_all(EXPORT_FIXTURE_BYTES)
            .expect("write private export staging file");
        staging.sync().expect("sync private export staging file");
        let metadata = staging
            .refresh_metadata()
            .expect("refresh private export staging file");
        let output_relative = PathBuf::from("replacement.webm");
        fixture
            .backend
            .export_staging_directory
            .publish_file_to_root_if_identity(
                &staging_relative,
                metadata.identity(),
                &fixture.backend.export_directory,
                &output_relative,
            )
            .expect("publish retained staging identity");
        let output = fixture.backend.export_root.join(&output_relative);
        let moved = fixture.backend.export_root.join("replacement-moved.webm");
        fs::rename(&output, &moved).expect("move published output leaf");
        fs::write(&output, EXPORT_FIXTURE_BYTES).expect("install equal-byte replacement leaf");

        assert_eq!(
            verify_published_rooted_file(
                &fixture.backend.export_directory,
                &output_relative,
                metadata.identity(),
                metadata.size_bytes(),
                &sha256_bytes(EXPORT_FIXTURE_BYTES),
            ),
            Err(NativeDesktopBackendError::Filesystem),
            "an equal-byte replacement must still fail the published inode identity check"
        );
        fs::remove_file(output).expect("remove replacement leaf");
        fs::remove_file(moved).expect("remove moved published fixture");
    }

    #[test]
    fn root_creation_rejects_symlinked_parent_without_side_effects() {
        let nonce: [u8; TOKEN_RANDOM_BYTES] =
            random_array(&SystemRandom::new()).expect("test random nonce");
        let root = PathBuf::from("/private/tmp")
            .join(format!("frame-native-root-bind-{}", encode_hex(&nonce)));
        let real_parent = root.join("real");
        let linked_parent = root.join("linked");
        fs::create_dir(&root).expect("create root fixture");
        fs::create_dir(&real_parent).expect("create real parent");
        symlink(&real_parent, &linked_parent).expect("create parent symlink");
        let requested = linked_parent.join("media");

        assert!(matches!(
            bind_or_create_root(requested, true),
            Err(NativeDesktopBackendError::Filesystem)
        ));
        assert!(
            !real_parent.join("media").exists(),
            "a rejected symlinked parent must not receive a created directory"
        );
        fs::remove_dir_all(&root).expect("remove root fixture");
    }

    #[test]
    fn root_creation_safely_creates_missing_private_ancestors() {
        let nonce: [u8; TOKEN_RANDOM_BYTES] =
            random_array(&SystemRandom::new()).expect("test random nonce");
        let root = PathBuf::from("/private/tmp")
            .join(format!("frame-native-root-create-{}", encode_hex(&nonce)));
        fs::create_dir(&root).expect("create root fixture");
        let requested = root.join("application/support/media");

        let (visible, directory) =
            bind_or_create_root(requested.clone(), true).expect("create nested rooted directory");
        assert_eq!(visible, requested);
        assert!(
            directory
                .matches_visible_path(&visible)
                .expect("rebind created visible root")
        );
        fs::remove_dir_all(&root).expect("remove root fixture");
    }
}
