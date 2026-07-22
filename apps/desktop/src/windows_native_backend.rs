//! Windows composition of Graphics Capture with the owned GStreamer recorder.
//!
//! The Tauri composition root enables capture protection on every Frame
//! window before constructing this backend. The backend then records an opaque
//! display, window, or region through the shared normalized capture worker and
//! exports the verified Editable WebM without exposing native target identity.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    mem,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::mpsc::{SyncSender, TrySendError, sync_channel},
    thread::{self, JoinHandle},
};

use frame_media::{
    ColorSpace, DisplayGeometryTransform, FrameMemory, LogicalRect, PermissionPreflight,
    PixelFormat, ProtectedContentPolicy, Rotation, ScreenRecording, ScreenRecordingError,
    ScreenRecordingSpec, ScreenSessionId, ScreenSourceInstanceId, ScreenTargetBinding,
    ScreenTargetDescriptor, ScreenTargetKind, ScreenTargetSnapshot, VideoFrameSpec,
    preflight_screen_recording_runtime,
};
use frame_windows_screen_capture::{
    WindowsCaptureDiagnostics, WindowsCaptureError, WindowsNormalizedScreenCaptureSource,
    WindowsRegionSelection, WindowsScreenCaptureSource,
};
use frame_windows_secure_spool::{
    WindowsPublishError, create_private_file, enforce_private_permissions, metadata_is_indirect,
    publish_file,
};
use ring::{
    digest::{Context as Sha256Context, SHA256},
    rand::{SecureRandom, SystemRandom},
};
use zeroize::Zeroizing;

use crate::{
    CAPTURE_TARGET_CATALOG_VERSION, CaptureTargetCatalog, CaptureTargetKind, CaptureTargetSummary,
    NativeCaptureArtifact, NativeCaptureStartRequest, NativeDesktopBackend,
    NativeDesktopBackendError, NativeEditableWebmExportOutcome, NativeEditableWebmExportRequest,
    NativePermissionOutcome, NativeRecordingCancelOutcome, NativeRecordingControlRequest,
    NativeRecordingStartOutcome, NativeRecordingStopOutcome, NativeRecordingTerminalFailure,
    NativeRegionDefinitionOutcome, NativeRegionDefinitionRequest, NativeTargetSelectionOutcome,
    NativeTargetSelectionRequest,
    native_screen_worker::{
        CompletedRecordingArtifact, NativeScreenSource, ScreenWorkerStart, WorkerCompletion,
        WorkerControl, WorkerOutcome,
    },
};

const TOKEN_RANDOM_BYTES: usize = 16;
const MAX_TOKEN_ATTEMPTS: usize = 8;
const WORKER_CONTROL_CAPACITY: usize = 1;
const WORKER_START_CAPACITY: usize = 1;
const RECORDINGS_DIRECTORY: &str = "recordings";
const FILE_IO_BUFFER_BYTES: usize = 64 * 1024;
// FILE_FLAG_OPEN_REPARSE_POINT: open the final component itself instead of
// following a junction or symlink. Ancestors are protected app-data folders.
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[derive(Clone)]
struct CatalogTarget {
    summary: CaptureTargetSummary,
    descriptor: ScreenTargetDescriptor,
}

struct SessionSource {
    source: WindowsScreenCaptureSource,
    observed_topology_generation: Option<u64>,
    snapshot: Option<ScreenTargetSnapshot>,
}

struct ActiveRecording {
    token: String,
    control: SyncSender<WorkerControl>,
    worker: JoinHandle<WorkerCompletion>,
    expected_path: PathBuf,
}

enum CaptureLifecycle {
    Ready(Box<SessionSource>),
    Recording(ActiveRecording),
    Poisoned,
}

#[derive(Clone)]
struct StoredArtifact {
    token: String,
    revision: u64,
    source: PathBuf,
    source_bytes: u64,
    source_sha256: String,
    export: PathBuf,
}

/// Production Windows backend. Construction fails unless the composition root
/// has already confirmed that Frame's own window is excluded from WGC.
pub struct WindowsNativeDesktopBackend {
    capture: CaptureLifecycle,
    source_secret: Zeroizing<[u8; 32]>,
    recordings_root: PathBuf,
    export_root: PathBuf,
    catalog_generation: u64,
    stable_tokens: BTreeMap<ScreenTargetBinding, String>,
    catalog: BTreeMap<String, CatalogTarget>,
    selected_token: Option<String>,
    artifact_revision: u64,
    artifact: Option<StoredArtifact>,
}

impl WindowsNativeDesktopBackend {
    pub fn new(
        media_root: impl Into<PathBuf>,
        export_root: impl Into<PathBuf>,
        frame_windows_excluded: bool,
    ) -> Result<Self, NativeDesktopBackendError> {
        if !frame_windows_excluded {
            return Err(NativeDesktopBackendError::Unavailable);
        }
        let media_root = prepare_private_directory(media_root.into())?;
        let recordings_root = prepare_private_directory(media_root.join(RECORDINGS_DIRECTORY))?;
        let export_root = prepare_private_directory(export_root.into())?;
        if media_root == export_root || recordings_root == export_root {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        preflight_screen_recording_runtime().map_err(map_recording_error)?;
        let source_secret = Zeroizing::new(random_array(&SystemRandom::new())?);
        let capture = CaptureLifecycle::Ready(Box::new(new_session_source(&source_secret)?));
        Ok(Self {
            capture,
            source_secret,
            recordings_root,
            export_root,
            catalog_generation: 1,
            stable_tokens: BTreeMap::new(),
            catalog: BTreeMap::new(),
            selected_token: None,
            artifact_revision: 0,
            artifact: None,
        })
    }

    fn source_mut(&mut self) -> Result<&mut WindowsScreenCaptureSource, NativeDesktopBackendError> {
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

    fn retire_session(&mut self, teardown_confirmed: bool) -> bool {
        if self.advance_catalog_generation().is_err() || !teardown_confirmed {
            self.capture = CaptureLifecycle::Poisoned;
            return false;
        }
        match new_session_source(&self.source_secret) {
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
            let collision = self.catalog.contains_key(&token)
                || self.stable_tokens.values().any(|value| value == &token)
                || matches!(&self.capture, CaptureLifecycle::Recording(active) if active.token == token)
                || self
                    .artifact
                    .as_ref()
                    .is_some_and(|artifact| artifact.token == token);
            if !collision {
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
            let token = if let Some(token) = self.stable_tokens.get(&binding) {
                token.clone()
            } else {
                let token = self.fresh_token(target_token_prefix(target.kind()))?;
                self.stable_tokens.insert(binding, token.clone());
                token
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
    ) -> Result<(String, PathBuf, WorkerCompletion), NativeDesktopBackendError> {
        let active = match &self.capture {
            CaptureLifecycle::Recording(active) => active,
            CaptureLifecycle::Ready(_) => return Err(NativeDesktopBackendError::TargetUnavailable),
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
        let completion = active.worker.join().unwrap_or(WorkerCompletion {
            outcome: WorkerOutcome::Failed {
                error: NativeDesktopBackendError::Internal,
                teardown_confirmed: false,
            },
        });
        Ok((active.token, active.expected_path, completion))
    }

    fn seal_artifact(
        &mut self,
        recording_token: String,
        expected_path: &Path,
        artifact: CompletedRecordingArtifact,
    ) -> Result<NativeCaptureArtifact, NativeDesktopBackendError> {
        if artifact.path != expected_path
            || artifact.path.parent() != Some(self.recordings_root.as_path())
        {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        let metadata = fs::symlink_metadata(&artifact.path)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        if !metadata.is_file()
            || metadata_is_indirect(&metadata)
            || metadata.len() != artifact.bytes
            || hash_file_no_reparse(&artifact.path, artifact.bytes)? != artifact.sha256
        {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        self.artifact_revision = self
            .artifact_revision
            .checked_add(1)
            .ok_or(NativeDesktopBackendError::Internal)?;
        let token = self.fresh_token("artifact")?;
        let export = self.export_root.join(format!("Frame-{token}.webm"));
        let response = NativeCaptureArtifact {
            recording_token,
            artifact_token: token.clone(),
            artifact_revision: self.artifact_revision,
            duration_ms: artifact.duration_ns.div_ceil(1_000_000).max(1),
            bytes_written: artifact.bytes,
            media_path: path_text(&artifact.path)?,
            editable_webm_output_path: Some(path_text(&export)?),
        };
        self.artifact = Some(StoredArtifact {
            token,
            revision: self.artifact_revision,
            source: artifact.path,
            source_bytes: artifact.bytes,
            source_sha256: artifact.sha256,
            export,
        });
        Ok(response)
    }
}

impl NativeDesktopBackend for WindowsNativeDesktopBackend {
    fn prepare_capture(&mut self) -> Result<NativePermissionOutcome, NativeDesktopBackendError> {
        let source = self.source_mut()?;
        let permission = match source.preflight_permission().map_err(map_capture_error)? {
            PermissionPreflight::PromptRequired => {
                source.request_permission().map_err(map_capture_error)?
            }
            permission => permission,
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
        let (snapshot, previous) = match &mut self.capture {
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
        self.install_target_snapshot(snapshot, previous)
    }

    fn select_target(
        &mut self,
        request: &NativeTargetSelectionRequest,
    ) -> Result<NativeTargetSelectionOutcome, NativeDesktopBackendError> {
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
        let region = WindowsRegionSelection::new(display.descriptor.binding(), logical_bounds)
            .map_err(map_capture_error)?;
        let (snapshot, previous) = match &mut self.capture {
            CaptureLifecycle::Ready(session) => {
                let snapshot = session
                    .source
                    .enumerate_targets(&[region])
                    .map_err(map_capture_error)?;
                let previous = session.observed_topology_generation;
                session.observed_topology_generation = Some(snapshot.generation());
                session.snapshot = Some(snapshot.clone());
                (snapshot, previous)
            }
            CaptureLifecycle::Recording(_) => return Err(NativeDesktopBackendError::Busy),
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        let catalog = self.install_target_snapshot(snapshot, previous)?;
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
        if !request.exclude_frame_windows {
            return Err(NativeDesktopBackendError::Unavailable);
        }
        if request.system_audio_enabled {
            return Err(NativeDesktopBackendError::Unavailable);
        }
        if request.catalog_generation != self.catalog_generation
            || self.selected_token.as_deref() != Some(request.target.token.as_str())
        {
            return Err(NativeDesktopBackendError::StaleCatalog);
        }
        let target = self
            .catalog
            .get(&request.target.token)
            .filter(|target| target.summary == request.target)
            .cloned()
            .ok_or(NativeDesktopBackendError::TargetUnavailable)?;
        let frame_duration = 1_000_000_000_u64
            .checked_div(u64::from(request.frame_rate))
            .filter(|duration| *duration > 0)
            .ok_or(NativeDesktopBackendError::Internal)?;
        let frame_spec = VideoFrameSpec {
            width: request.target.width_pixels,
            height: request.target.height_pixels,
            pixel_format: PixelFormat::Bgra8,
            color_space: ColorSpace::Srgb,
            nominal_frame_duration_ns: frame_duration,
            memory: FrameMemory::Cpu,
        };
        let spec = ScreenRecordingSpec::new(frame_spec).map_err(map_recording_error)?;
        let recording_token = self.fresh_token("recording")?;
        let output_path = self.recordings_root.join(format!("{recording_token}.webm"));
        let recording = ScreenRecording::start(&output_path, spec).map_err(map_recording_error)?;
        let session_id = ScreenSessionId::from_csprng(random_array(&SystemRandom::new())?)
            .map_err(|_| NativeDesktopBackendError::Internal)?;
        let session = match mem::replace(&mut self.capture, CaptureLifecycle::Poisoned) {
            CaptureLifecycle::Ready(session) => session,
            CaptureLifecycle::Recording(active) => {
                self.capture = CaptureLifecycle::Recording(active);
                return Err(NativeDesktopBackendError::Busy);
            }
            CaptureLifecycle::Poisoned => return Err(NativeDesktopBackendError::Unavailable),
        };
        let SessionSource {
            source,
            observed_topology_generation: _,
            snapshot,
        } = *session;
        let Some(snapshot) = snapshot else {
            let _ = recording.abort();
            let _ = self.retire_session(true);
            return Err(NativeDesktopBackendError::StaleCatalog);
        };
        let start = match ScreenWorkerStart::<WindowsNormalizedScreenCaptureSource>::prepare(
            source,
            snapshot,
            target.descriptor,
            frame_spec,
            recording,
            session_id,
        ) {
            Ok(start) => start,
            Err(failure) => {
                let _ = self.retire_session(failure.teardown_confirmed);
                return Err(failure.error);
            }
        };
        let (control, receiver) = sync_channel(WORKER_CONTROL_CAPACITY);
        let (start_sender, start_receiver) = sync_channel::<
            ScreenWorkerStart<WindowsNormalizedScreenCaptureSource>,
        >(WORKER_START_CAPACITY);
        let worker = thread::Builder::new()
            .name("frame-windows-screen-recorder".into())
            .spawn(move || {
                let Ok(start) = start_receiver.recv() else {
                    return WorkerCompletion {
                        outcome: WorkerOutcome::Failed {
                            error: NativeDesktopBackendError::Internal,
                            teardown_confirmed: false,
                        },
                    };
                };
                start.run(receiver)
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(_) => {
                let teardown_confirmed = start.teardown();
                let _ = self.retire_session(teardown_confirmed);
                return Err(NativeDesktopBackendError::Internal);
            }
        };
        if let Err(error) = start_sender.send(start) {
            let teardown_confirmed = error.0.teardown();
            let _ = worker.join();
            let _ = self.retire_session(teardown_confirmed);
            return Err(NativeDesktopBackendError::Internal);
        }
        self.capture = CaptureLifecycle::Recording(ActiveRecording {
            token: recording_token.clone(),
            control,
            worker,
            expected_path: output_path,
        });
        self.artifact = None;
        Ok(NativeRecordingStartOutcome {
            catalog_generation: self.catalog_generation,
            target_token: request.target.token.clone(),
            recording_token,
            system_audio_included: false,
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
        let outcome = active.worker.join().unwrap_or(WorkerCompletion {
            outcome: WorkerOutcome::Failed {
                error: NativeDesktopBackendError::Internal,
                teardown_confirmed: false,
            },
        });
        let (error, teardown_confirmed) = match outcome.outcome {
            WorkerOutcome::Failed {
                error,
                teardown_confirmed,
            } => (error, teardown_confirmed),
            WorkerOutcome::Cancelled | WorkerOutcome::Finished(_) => {
                (NativeDesktopBackendError::Internal, false)
            }
        };
        let _ = self.retire_session(teardown_confirmed);
        Ok(Some(NativeRecordingTerminalFailure {
            recording_token: active.token,
            error,
            teardown_confirmed,
        }))
    }

    fn stop_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingStopOutcome, NativeDesktopBackendError> {
        let (token, expected_path, completion) =
            self.take_worker(&request.recording_token, WorkerControl::Stop)?;
        let teardown_confirmed = completion.outcome.teardown_confirmed();
        let retired = self.retire_session(teardown_confirmed);
        match completion.outcome {
            WorkerOutcome::Finished(artifact) if retired => {
                match self.seal_artifact(token.clone(), &expected_path, artifact) {
                    Ok(artifact) => Ok(NativeRecordingStopOutcome::Sealed(artifact)),
                    Err(error) => Ok(NativeRecordingStopOutcome::Failed(
                        NativeRecordingTerminalFailure {
                            recording_token: token,
                            error,
                            teardown_confirmed: true,
                        },
                    )),
                }
            }
            WorkerOutcome::Failed {
                error,
                teardown_confirmed,
            } => Ok(NativeRecordingStopOutcome::Failed(
                NativeRecordingTerminalFailure {
                    recording_token: token,
                    error,
                    teardown_confirmed,
                },
            )),
            WorkerOutcome::Cancelled | WorkerOutcome::Finished(_) => Ok(
                NativeRecordingStopOutcome::Failed(NativeRecordingTerminalFailure {
                    recording_token: token,
                    error: NativeDesktopBackendError::Internal,
                    teardown_confirmed,
                }),
            ),
        }
    }

    fn cancel_recording(
        &mut self,
        request: &NativeRecordingControlRequest,
    ) -> Result<NativeRecordingCancelOutcome, NativeDesktopBackendError> {
        let (token, _, completion) =
            self.take_worker(&request.recording_token, WorkerControl::Cancel)?;
        let teardown_confirmed = completion.outcome.teardown_confirmed();
        let retired = self.retire_session(teardown_confirmed);
        match completion.outcome {
            WorkerOutcome::Cancelled if retired => Ok(NativeRecordingCancelOutcome::Cancelled {
                recording_token: token,
            }),
            WorkerOutcome::Failed {
                error,
                teardown_confirmed,
            } => Ok(NativeRecordingCancelOutcome::Failed(
                NativeRecordingTerminalFailure {
                    recording_token: token,
                    error,
                    teardown_confirmed,
                },
            )),
            WorkerOutcome::Finished(_) | WorkerOutcome::Cancelled => Ok(
                NativeRecordingCancelOutcome::Failed(NativeRecordingTerminalFailure {
                    recording_token: token,
                    error: NativeDesktopBackendError::Internal,
                    teardown_confirmed,
                }),
            ),
        }
    }

    fn export_editable_webm(
        &mut self,
        request: &NativeEditableWebmExportRequest,
    ) -> Result<NativeEditableWebmExportOutcome, NativeDesktopBackendError> {
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
        let nonce: [u8; TOKEN_RANDOM_BYTES] = random_array(&SystemRandom::new())?;
        let staging = self.export_root.join(format!(
            ".frame-export-{}-{}.partial",
            artifact.token,
            encode_hex(&nonce)
        ));
        let mut source = open_regular_file_no_reparse(&artifact.source)?;
        let mut destination =
            create_private_file(&staging).map_err(|_| NativeDesktopBackendError::Filesystem)?;
        let copy_result = copy_and_hash(
            &mut source,
            &mut destination,
            artifact.source_bytes,
            &artifact.source_sha256,
        );
        let copied = match copy_result {
            Ok(copied) => copied,
            Err(error) => {
                drop(destination);
                let _ = fs::remove_file(&staging);
                return Err(error);
            }
        };
        destination
            .sync_all()
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        drop(destination);
        publish_file(&staging, &artifact.export).map_err(|error| {
            let _ = fs::remove_file(&staging);
            match error {
                WindowsPublishError::AlreadyExists | WindowsPublishError::Failed => {
                    NativeDesktopBackendError::Filesystem
                }
            }
        })?;
        if hash_file_no_reparse(&artifact.export, copied)? != artifact.source_sha256 {
            return Err(NativeDesktopBackendError::Filesystem);
        }
        Ok(NativeEditableWebmExportOutcome {
            artifact_token: artifact.token,
            artifact_revision: artifact.revision,
            bytes_written: copied,
        })
    }
}

impl Drop for WindowsNativeDesktopBackend {
    fn drop(&mut self) {
        let capture = mem::replace(&mut self.capture, CaptureLifecycle::Poisoned);
        if let CaptureLifecycle::Recording(active) = capture {
            let _ = active.control.try_send(WorkerControl::Cancel);
            let _ = active.worker.join();
        }
    }
}

impl fmt::Debug for WindowsNativeDesktopBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowsNativeDesktopBackend")
            .field("catalog_generation", &self.catalog_generation)
            .field("catalog_size", &self.catalog.len())
            .field(
                "selected",
                &self.selected_token.as_ref().map(|_| "<redacted>"),
            )
            .field("artifact", &self.artifact.as_ref().map(|_| "<redacted>"))
            .finish_non_exhaustive()
    }
}

impl NativeScreenSource for WindowsNormalizedScreenCaptureSource {
    type RawSource = WindowsScreenCaptureSource;
    type Diagnostics = WindowsCaptureDiagnostics;

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
        let terminal_delta = current
            .dropped_callback_frames
            .checked_sub(baseline.dropped_callback_frames)
            .zip(
                current
                    .invalid_native_frames
                    .checked_sub(baseline.invalid_native_frames),
            )
            .zip(
                current
                    .target_closed_events
                    .checked_sub(baseline.target_closed_events),
            )
            .zip(
                current
                    .unexpected_native_stops
                    .checked_sub(baseline.unexpected_native_stops),
            );
        terminal_delta
            .map(|(((dropped, invalid), closed), stops)| {
                dropped > 0 || invalid > 0 || closed > 0 || stops > 0
            })
            .unwrap_or(true)
    }

    fn protected_content_policy() -> ProtectedContentPolicy {
        ProtectedContentPolicy::RequirePlatformRedaction
    }
}

fn prepare_private_directory(path: PathBuf) -> Result<PathBuf, NativeDesktopBackendError> {
    if !path.is_absolute() || path.to_str().is_none() {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    fs::create_dir_all(&path).map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if !metadata.is_dir() || metadata_is_indirect(&metadata) {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    enforce_private_permissions(&path).map_err(|_| NativeDesktopBackendError::Filesystem)?;
    Ok(path)
}

fn new_session_source(secret: &[u8; 32]) -> Result<SessionSource, NativeDesktopBackendError> {
    let random = SystemRandom::new();
    let source_instance = ScreenSourceInstanceId::new(random_array(&random)?)
        .map_err(|_| NativeDesktopBackendError::Internal)?;
    let per_session_secret: [u8; 32] = random_array(&random)?;
    let mut derived = [0_u8; 32];
    for (index, byte) in derived.iter_mut().enumerate() {
        *byte = per_session_secret[index] ^ secret[index];
    }
    let source =
        WindowsScreenCaptureSource::new(source_instance, derived).map_err(map_capture_error)?;
    Ok(SessionSource {
        source,
        observed_topology_generation: None,
        snapshot: None,
    })
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
                let display = target
                    .containing_display_binding()
                    .and_then(|binding| {
                        targets
                            .iter()
                            .find(|candidate| candidate.binding() == binding)
                    })
                    .and_then(ScreenTargetDescriptor::display_transform)
                    .ok_or(NativeDesktopBackendError::Internal)?;
                let physical = display
                    .logical_rect_to_physical(target.logical_bounds())
                    .map_err(|_| NativeDesktopBackendError::Internal)?;
                let scale = display.scale();
                (
                    physical.width(),
                    physical.height(),
                    u16::try_from(scale.numerator())
                        .map_err(|_| NativeDesktopBackendError::Internal)?,
                    u16::try_from(scale.denominator())
                        .map_err(|_| NativeDesktopBackendError::Internal)?,
                    rotation_degrees(display),
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

fn path_text(path: &Path) -> Result<String, NativeDesktopBackendError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or(NativeDesktopBackendError::Filesystem)
}

fn open_regular_file_no_reparse(path: &Path) -> Result<File, NativeDesktopBackendError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let metadata = file
        .metadata()
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    if !metadata.is_file() || metadata_is_indirect(&metadata) {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(file)
}

fn hash_file_no_reparse(
    path: &Path,
    expected_bytes: u64,
) -> Result<String, NativeDesktopBackendError> {
    let mut file = open_regular_file_no_reparse(path)?;
    hash_open_file(&mut file, expected_bytes)
}

fn hash_open_file(
    file: &mut File,
    expected_bytes: u64,
) -> Result<String, NativeDesktopBackendError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let mut digest = Sha256Context::new(&SHA256);
    let mut buffer = [0_u8; FILE_IO_BUFFER_BYTES];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).map_err(|_| NativeDesktopBackendError::Filesystem)?)
            .filter(|total| *total <= expected_bytes)
            .ok_or(NativeDesktopBackendError::Filesystem)?;
        digest.update(&buffer[..read]);
    }
    if total != expected_bytes {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(encode_hex(digest.finish().as_ref()))
}

fn copy_and_hash(
    source: &mut File,
    destination: &mut File,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<u64, NativeDesktopBackendError> {
    source
        .seek(SeekFrom::Start(0))
        .map_err(|_| NativeDesktopBackendError::Filesystem)?;
    let mut digest = Sha256Context::new(&SHA256);
    let mut buffer = [0_u8; FILE_IO_BUFFER_BYTES];
    let mut total = 0_u64;
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).map_err(|_| NativeDesktopBackendError::Filesystem)?)
            .filter(|total| *total <= expected_bytes)
            .ok_or(NativeDesktopBackendError::Filesystem)?;
        destination
            .write_all(&buffer[..read])
            .map_err(|_| NativeDesktopBackendError::Filesystem)?;
        digest.update(&buffer[..read]);
    }
    if total != expected_bytes || encode_hex(digest.finish().as_ref()) != expected_sha256 {
        return Err(NativeDesktopBackendError::Filesystem);
    }
    Ok(total)
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

fn map_capture_error(error: WindowsCaptureError) -> NativeDesktopBackendError {
    match error {
        WindowsCaptureError::AdapterUnavailable
        | WindowsCaptureError::CaptureStartTimedOut
        | WindowsCaptureError::CaptureStopTimedOut => NativeDesktopBackendError::Unavailable,
        WindowsCaptureError::AlreadyRunning => NativeDesktopBackendError::Busy,
        WindowsCaptureError::StaleOrForeignTarget
        | WindowsCaptureError::StaleOrForeignRegionDisplay
        | WindowsCaptureError::StaleTargetTopology => NativeDesktopBackendError::StaleCatalog,
        WindowsCaptureError::TargetNoLongerAvailable
        | WindowsCaptureError::UnexpectedStreamStop => NativeDesktopBackendError::TargetUnavailable,
        _ => NativeDesktopBackendError::Internal,
    }
}
