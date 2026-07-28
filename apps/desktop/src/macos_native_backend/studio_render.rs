//! Durable asynchronous macOS Studio export coordination.
//!
//! The renderer owns only descriptor-rooted staging/output identities and a
//! bounded worker result channel. The provider-neutral coordinator remains the
//! authority for journal fences, progress ordering, cancellation, cleanup,
//! and terminal receipts.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, TryRecvError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use frame_media::{
    AssetChecksum, CancellationToken, CodecLicense, DurableStudioJournal, EncoderBackend,
    ExportProfile as MediaExportProfile, FilesystemStudioJournalStore, FrameRate,
    JournalAdvanceRequest, JournalBoundary, MediaContainer, NativeStudioEditedExportArtifact,
    PendingRender, ReceiptKind, RenderCapabilities, RenderEvent, RenderEventKind, RenderPhase,
    RenderPostcondition, RenderSessionState, RenderStartOutcome, Resolution, StudioAudioCodec,
    StudioError, StudioExportId, StudioOperationId, StudioProjectManifest, StudioRenderCoordinator,
    StudioRenderGraphSpec, StudioRenderTicket, StudioRendererPort, StudioSourceName,
    StudioSourceSet, StudioTimelineCompiler, StudioVideoCodec, TimelineSource, preflight_render,
    strong_sha256,
};

use super::{
    PreparedStudioExport, map_rooted_io_error, sha256_rooted_file,
    studio_recorder::{random_export_id, random_operation_id, random_worker_id},
    verify_published_rooted_file,
};
use crate::{
    ExportProfile, NativeDesktopBackendError, NativeStudioExportOutcome,
    NativeStudioExportPollOutcome, NativeStudioExportRequest, NativeStudioExportStartOutcome,
    rooted_io::{FileIdentity, RootedDir, RootedFile},
};

const MAX_RENDER_EVENTS: usize = 8;
const RENDER_DEADLINE: Duration = Duration::from_secs(24 * 60 * 60);
const CANCEL_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
struct WorkerProgress {
    phase: RenderPhase,
    basis_points: u16,
}

struct ReservedRender {
    prepared: PreparedStudioExport,
    staging_relative: PathBuf,
    staging_path: PathBuf,
    output_relative: PathBuf,
    staging: RootedFile,
}

impl std::fmt::Debug for ReservedRender {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReservedRender")
            .field("prepared", &self.prepared)
            .field("paths", &"<redacted>")
            .field("identity", &"<redacted>")
            .finish()
    }
}

struct RendererSession {
    project_id: frame_media::StudioProjectId,
    fence: u64,
    render_spec_digest: frame_media::Sha256Digest,
    output_name: StudioSourceName,
    staging_relative: PathBuf,
    staging_path: PathBuf,
    output_relative: PathBuf,
    staging: Option<RootedFile>,
    staging_identity: FileIdentity,
    published: bool,
    cancellation: CancellationToken,
    progress: Arc<Mutex<WorkerProgress>>,
    last_progress: WorkerProgress,
    sequence: u64,
    completion: Receiver<Result<NativeStudioEditedExportArtifact, NativeDesktopBackendError>>,
    worker: Option<JoinHandle<()>>,
    postcondition: RenderPostcondition,
}

impl std::fmt::Debug for RendererSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RendererSession")
            .field("project_id", &self.project_id)
            .field("fence", &self.fence)
            .field("output_name", &self.output_name)
            .field("published", &self.published)
            .field("postcondition", &self.postcondition)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct MacOsStudioRenderer {
    capabilities: RenderCapabilities,
    export_directory: RootedDir,
    staging_directory: RootedDir,
    reservations: BTreeMap<StudioExportId, ReservedRender>,
    sessions: BTreeMap<StudioExportId, RendererSession>,
}

impl MacOsStudioRenderer {
    fn new(export_root: &Path, staging_root: &Path) -> Result<Self, NativeDesktopBackendError> {
        Ok(Self {
            capabilities: software_capabilities(),
            export_directory: RootedDir::bind(export_root).map_err(map_rooted_io_error)?,
            staging_directory: RootedDir::bind(staging_root).map_err(map_rooted_io_error)?,
            reservations: BTreeMap::new(),
            sessions: BTreeMap::new(),
        })
    }

    fn reserve(
        &mut self,
        export_id: StudioExportId,
        prepared: PreparedStudioExport,
        staging_relative: PathBuf,
        staging_path: PathBuf,
        output_relative: PathBuf,
    ) -> Result<(), NativeDesktopBackendError> {
        if self.reservations.contains_key(&export_id) || self.sessions.contains_key(&export_id) {
            return Err(NativeDesktopBackendError::Busy);
        }
        let staging = self
            .staging_directory
            .create_new_file(&staging_relative)
            .map_err(map_rooted_io_error)?;
        self.reservations.insert(
            export_id,
            ReservedRender {
                prepared,
                staging_relative,
                staging_path,
                output_relative,
                staging,
            },
        );
        Ok(())
    }

    fn abort_reservation(&mut self, export_id: StudioExportId) {
        let Some(reserved) = self.reservations.remove(&export_id) else {
            return;
        };
        let identity = reserved.staging.metadata().identity();
        drop(reserved.staging);
        let _ = self
            .staging_directory
            .cleanup_file_if_identity(reserved.staging_relative, identity);
    }

    fn release_session(&mut self, export_id: StudioExportId) {
        self.reservations.remove(&export_id);
        self.sessions.remove(&export_id);
    }

    fn terminal_event(
        session: &mut RendererSession,
        export_id: StudioExportId,
        kind: RenderEventKind,
    ) -> Result<RenderEvent, StudioError> {
        session.sequence = session
            .sequence
            .checked_add(1)
            .ok_or(StudioError::StaleRenderCallback)?;
        Ok(RenderEvent {
            project_id: session.project_id,
            export_id,
            fence: session.fence,
            render_spec_digest: session.render_spec_digest,
            sequence: session.sequence,
            kind,
        })
    }
}

impl StudioRendererPort for MacOsStudioRenderer {
    fn capabilities(&mut self) -> Result<RenderCapabilities, StudioError> {
        Ok(self.capabilities.clone())
    }

    fn start(&mut self, ticket: StudioRenderTicket) -> Result<RenderStartOutcome, StudioError> {
        let export_id = ticket.export_id();
        let reserved = self
            .reservations
            .remove(&export_id)
            .ok_or(StudioError::RenderReservationRequired)?;
        if ticket.graph().preflight.selected_backend != EncoderBackend::Software
            || ticket.output_name().as_str()
                != reserved
                    .output_relative
                    .to_str()
                    .ok_or(StudioError::InvalidSourceName)?
        {
            self.reservations.insert(export_id, reserved);
            return Err(StudioError::InvalidRenderTicket);
        }

        let ReservedRender {
            prepared,
            staging_relative,
            staging_path,
            output_relative,
            staging,
        } = reserved;
        let staging_identity = staging.metadata().identity();
        let output = staging.file().try_clone();
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let progress = Arc::new(Mutex::new(WorkerProgress {
            phase: RenderPhase::Preparing,
            basis_points: 0,
        }));
        let worker_progress = Arc::clone(&progress);
        let (completion_sender, completion) = sync_channel(1);
        let worker_staging_path = staging_path.clone();
        self.sessions.insert(
            export_id,
            RendererSession {
                project_id: ticket.project_id(),
                fence: ticket.expected_fence(),
                render_spec_digest: ticket.render_spec_digest(),
                output_name: ticket.output_name().clone(),
                staging_relative,
                staging_path,
                output_relative,
                staging: Some(staging),
                staging_identity,
                published: false,
                cancellation,
                progress,
                last_progress: WorkerProgress {
                    phase: RenderPhase::Preparing,
                    basis_points: 0,
                },
                sequence: 0,
                completion,
                worker: None,
                postcondition: RenderPostcondition::Running {
                    fence: ticket.expected_fence(),
                    render_spec_digest: ticket.render_spec_digest(),
                },
            },
        );

        let Ok(output) = output else {
            let _ = completion_sender.send(Err(NativeDesktopBackendError::Filesystem));
            return Ok(RenderStartOutcome::Accepted);
        };
        let worker = thread::Builder::new()
            .name("frame-studio-render".into())
            .spawn(move || {
                let result = prepared.render_preopened_with_progress(
                    &worker_staging_path,
                    output,
                    &worker_cancellation,
                    |update| {
                        if let Ok(mut current) = worker_progress.lock() {
                            *current = WorkerProgress {
                                phase: update.phase,
                                basis_points: update.basis_points,
                            };
                        }
                    },
                );
                let _ = completion_sender.send(result);
            });
        if let Ok(worker) = worker {
            self.sessions
                .get_mut(&export_id)
                .ok_or(StudioError::UnknownExport)?
                .worker = Some(worker);
        }
        Ok(RenderStartOutcome::Accepted)
    }
    fn poll(
        &mut self,
        export_id: StudioExportId,
        maximum_events: usize,
        _wait: Duration,
    ) -> Result<Vec<RenderEvent>, StudioError> {
        if maximum_events == 0 {
            return Err(StudioError::UnboundedRendererEvents);
        }
        let Self {
            export_directory,
            staging_directory,
            sessions,
            ..
        } = self;
        let session = sessions
            .get_mut(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        if !matches!(session.postcondition, RenderPostcondition::Running { .. }) {
            return Ok(Vec::new());
        }

        let progress = *session
            .progress
            .lock()
            .map_err(|_| StudioError::StorageIo)?;
        let mut events = Vec::with_capacity(2);
        if progress.basis_points > session.last_progress.basis_points
            || progress.phase != session.last_progress.phase
        {
            session.sequence = session
                .sequence
                .checked_add(1)
                .ok_or(StudioError::StaleRenderCallback)?;
            events.push(RenderEvent {
                project_id: session.project_id,
                export_id,
                fence: session.fence,
                render_spec_digest: session.render_spec_digest,
                sequence: session.sequence,
                kind: RenderEventKind::Progress {
                    phase: progress.phase,
                    basis_points: progress.basis_points,
                },
            });
            session.last_progress = progress;
        }

        let completion = match session.completion.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(NativeDesktopBackendError::Internal)),
        };
        let Some(completion) = completion else {
            return Ok(events);
        };
        if let Some(worker) = session.worker.take() {
            let _ = worker.join();
        }

        let terminal = match completion {
            Ok(artifact) => {
                match finalize_output(staging_directory, export_directory, session, &artifact) {
                    Ok((checksum, bytes)) => RenderEventKind::Committed {
                        output_checksum: checksum,
                        output_bytes: bytes,
                    },
                    Err(error) => {
                        eprintln!("Frame Studio export finalization failed: {error}");
                        session.postcondition = RenderPostcondition::Partial {
                            fence: session.fence,
                            render_spec_digest: session.render_spec_digest,
                        };
                        RenderEventKind::Failed {
                            safe_code: "filesystem",
                            hardware_failure: false,
                        }
                    }
                }
            }
            Err(NativeDesktopBackendError::Cancelled) => {
                session.postcondition = RenderPostcondition::Partial {
                    fence: session.fence,
                    render_spec_digest: session.render_spec_digest,
                };
                RenderEventKind::Cancelled
            }
            Err(error) => {
                session.postcondition = RenderPostcondition::Partial {
                    fence: session.fence,
                    render_spec_digest: session.render_spec_digest,
                };
                RenderEventKind::Failed {
                    safe_code: safe_render_failure(error),
                    hardware_failure: false,
                }
            }
        };
        events.push(Self::terminal_event(session, export_id, terminal)?);
        if events.len() > maximum_events {
            return Err(StudioError::RendererEventOverflow);
        }
        Ok(events)
    }

    fn probe(&mut self, export_id: StudioExportId) -> Result<RenderPostcondition, StudioError> {
        self.sessions
            .get(&export_id)
            .map(|session| session.postcondition.clone())
            .ok_or(StudioError::UnknownExport)
    }

    fn cancel(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        deadline: Duration,
    ) -> Result<(), StudioError> {
        let session = self
            .sessions
            .get_mut(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        if session.fence != expected_fence {
            return Err(StudioError::StaleRenderCallback);
        }
        session.cancellation.cancel();
        match session.completion.recv_timeout(deadline) {
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                return Err(StudioError::RenderDeadlineExceeded);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
        }
        if let Some(worker) = session.worker.take() {
            worker.join().map_err(|_| StudioError::StorageIo)?;
        }
        session.postcondition = RenderPostcondition::Partial {
            fence: session.fence,
            render_spec_digest: session.render_spec_digest,
        };
        Ok(())
    }

    fn cleanup_partial(
        &mut self,
        export_id: StudioExportId,
        expected_fence: u64,
        expected_render_spec_digest: frame_media::Sha256Digest,
        output_name: &StudioSourceName,
    ) -> Result<(), StudioError> {
        let session = self
            .sessions
            .get_mut(&export_id)
            .ok_or(StudioError::UnknownExport)?;
        if session.fence != expected_fence
            || session.render_spec_digest != expected_render_spec_digest
            || &session.output_name != output_name
        {
            return Err(StudioError::StaleRenderCallback);
        }
        if session.postcondition == RenderPostcondition::Absent {
            return Ok(());
        }
        if matches!(session.postcondition, RenderPostcondition::Committed { .. }) {
            return Err(StudioError::CommittedRenderCannotBeCancelled);
        }
        drop(session.staging.take());
        let cleanup = if session.published {
            self.export_directory
                .cleanup_file_if_identity(&session.output_relative, session.staging_identity)
        } else {
            self.staging_directory
                .cleanup_file_if_identity(&session.staging_relative, session.staging_identity)
        };
        cleanup.map_err(|_| StudioError::PartialCleanupUnconfirmed)?;
        session.postcondition = RenderPostcondition::Absent;
        Ok(())
    }
}

fn finalize_output(
    staging_directory: &RootedDir,
    export_directory: &RootedDir,
    session: &mut RendererSession,
    artifact: &NativeStudioEditedExportArtifact,
) -> Result<(AssetChecksum, u64), &'static str> {
    if artifact.path != session.staging_path || artifact.bytes == 0 {
        return Err("artifact identity");
    }
    let staging = session.staging.as_mut().ok_or("staging authority")?;
    let metadata = staging.refresh_metadata().map_err(|_| "staging metadata")?;
    if metadata.identity() != session.staging_identity || metadata.size_bytes() != artifact.bytes {
        return Err("staging postcondition");
    }
    if sha256_rooted_file(staging, metadata.identity(), metadata.size_bytes())
        .map_err(|_| "staging checksum")?
        != artifact.sha256
    {
        return Err("staging checksum mismatch");
    }
    staging.sync().map_err(|_| "staging sync")?;
    let published = staging_directory
        .publish_file_to_root_if_identity(
            &session.staging_relative,
            session.staging_identity,
            export_directory,
            &session.output_relative,
        )
        .map_err(|_| "staging publication")?;
    session.published = true;
    if published.identity() != session.staging_identity
        || published.size_bytes() != artifact.bytes
        || verify_published_rooted_file(
            export_directory,
            &session.output_relative,
            session.staging_identity,
            artifact.bytes,
            &artifact.sha256,
        )
        .is_err()
    {
        return Err("published postcondition");
    }
    let checksum =
        AssetChecksum::from_hex(&artifact.sha256).map_err(|_| "published checksum encoding")?;
    session.postcondition = RenderPostcondition::Committed {
        fence: session.fence,
        render_spec_digest: session.render_spec_digest,
        output_checksum: checksum,
        output_bytes: artifact.bytes,
    };
    Ok((checksum, artifact.bytes))
}

#[derive(Debug, Clone)]
struct ActiveRender {
    export_id: StudioExportId,
    operation_id: StudioOperationId,
    project_revision: u64,
    profile: ExportProfile,
}

#[derive(Debug)]
pub(super) struct NativeStudioRenderController {
    projects_root: PathBuf,
    staging_root: PathBuf,
    coordinator: StudioRenderCoordinator<MacOsStudioRenderer>,
    active: Option<ActiveRender>,
}

impl NativeStudioRenderController {
    pub(super) fn new(
        projects_root: &Path,
        export_root: &Path,
        staging_root: &Path,
    ) -> Result<Self, NativeDesktopBackendError> {
        let renderer = MacOsStudioRenderer::new(export_root, staging_root)?;
        let coordinator = StudioRenderCoordinator::new(renderer, MAX_RENDER_EVENTS, Vec::new())
            .map_err(map_studio_error)?;
        Ok(Self {
            projects_root: projects_root.to_path_buf(),
            staging_root: staging_root.to_path_buf(),
            coordinator,
            active: None,
        })
    }

    pub(super) const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        &mut self,
        request: &NativeStudioExportRequest,
        manifest: &StudioProjectManifest,
        prepared: PreparedStudioExport,
        output_relative: PathBuf,
        staging_relative: PathBuf,
    ) -> Result<NativeStudioExportStartOutcome, NativeDesktopBackendError> {
        if self.active.is_some() {
            return Err(NativeDesktopBackendError::Busy);
        }
        let timeline = TimelineSource::from_assets(&manifest.assets).map_err(map_studio_error)?;
        let plan = StudioTimelineCompiler::compile(&timeline, &manifest.edits)
            .map_err(map_studio_error)?;
        let sources =
            StudioSourceSet::from_project(manifest, &timeline).map_err(map_studio_error)?;
        let capabilities = software_capabilities();
        let preflight = preflight_render(media_profile(request.profile), &capabilities)
            .map_err(map_studio_error)?;
        let graph =
            StudioRenderGraphSpec::compile(sources, plan, preflight).map_err(map_studio_error)?;
        let output_name = StudioSourceName::new(
            output_relative
                .to_str()
                .ok_or(NativeDesktopBackendError::Filesystem)?
                .to_owned(),
        )
        .map_err(map_studio_error)?;
        let export_id = random_export_id()?;
        let operation_id = random_operation_id()?;

        let store =
            FilesystemStudioJournalStore::new(&self.projects_root).map_err(map_studio_error)?;
        let mut journal =
            DurableStudioJournal::open(store, manifest.id).map_err(map_studio_error)?;
        journal
            .take_ownership(
                journal.snapshot().revision,
                journal.snapshot().fence,
                random_worker_id()?,
            )
            .map_err(map_studio_error)?;
        let ticket = StudioRenderTicket::new(
            manifest.id,
            export_id,
            operation_id,
            journal.snapshot().fence,
            output_name.clone(),
            graph,
            RENDER_DEADLINE,
        )
        .map_err(map_studio_error)?;
        let pending = PendingRender::new(
            operation_id,
            export_id,
            ticket.expected_fence(),
            ticket.graph().sources.digest(),
            ticket.graph().edit_plan_digest(),
            ticket.graph().preflight.profile.profile,
            output_name,
        )
        .map_err(map_studio_error)?;
        let command_digest = render_prepared_digest(b"frame-render-prepared-command-v1", &pending);
        let outcome_digest = render_prepared_digest(b"frame-render-prepared-outcome-v1", &pending);
        journal
            .advance(JournalAdvanceRequest {
                expected_revision: journal.snapshot().revision,
                expected_fence: journal.snapshot().fence,
                operation_id,
                command_digest,
                boundary: JournalBoundary::RenderPrepared,
                pending_asset: None,
                pending_edit: None,
                pending_render: Some(pending),
                receipt_kind: ReceiptKind::RenderPrepared,
                outcome_digest,
            })
            .map_err(map_studio_error)?;
        let dispatch = journal
            .into_render_authorization()
            .map_err(map_studio_error)?
            .bind(ticket)
            .map_err(map_studio_error)?;
        let staging_path = self.staging_root.join(&staging_relative);
        self.coordinator.renderer_mut().reserve(
            export_id,
            prepared,
            staging_relative,
            staging_path,
            output_relative,
        )?;
        let state = match self.coordinator.start(dispatch) {
            Ok(state) => state,
            Err(error) => {
                self.coordinator.renderer_mut().abort_reservation(export_id);
                return Err(map_studio_error(error));
            }
        };
        if state != RenderSessionState::Running {
            return Err(NativeDesktopBackendError::Internal);
        }
        self.active = Some(ActiveRender {
            export_id,
            operation_id,
            project_revision: request.project_revision,
            profile: request.profile,
        });
        Ok(NativeStudioExportStartOutcome::Running {
            progress_basis_points: 0,
        })
    }

    pub(super) fn poll(
        &mut self,
        request: &NativeStudioExportRequest,
    ) -> Result<NativeStudioExportPollOutcome, NativeDesktopBackendError> {
        let active = self
            .active
            .clone()
            .ok_or(NativeDesktopBackendError::Unavailable)?;
        validate_active(&active, request)?;
        let state = match self.coordinator.poll(active.export_id, Duration::ZERO) {
            Ok(state) => state,
            Err(error) => {
                let native_error = map_studio_error(error);
                let cleanup_confirmed = self
                    .coordinator
                    .cancel_and_cleanup(active.export_id, CANCEL_DEADLINE)
                    .and_then(|()| {
                        self.coordinator.release_terminal(active.export_id)?;
                        self.coordinator
                            .renderer_mut()
                            .release_session(active.export_id);
                        Ok(())
                    })
                    .is_ok();
                if cleanup_confirmed {
                    self.active = None;
                }
                return Ok(NativeStudioExportPollOutcome::Failed {
                    project_revision: active.project_revision,
                    profile: active.profile,
                    error: native_error,
                    cleanup_confirmed,
                });
            }
        };
        let progress = self
            .coordinator
            .progress(active.export_id)
            .map_err(map_studio_error)?;
        match state {
            RenderSessionState::Running => Ok(NativeStudioExportPollOutcome::Running {
                project_revision: active.project_revision,
                profile: active.profile,
                progress_basis_points: progress.basis_points,
            }),
            RenderSessionState::Committed => {
                let receipt = self
                    .coordinator
                    .receipt(active.operation_id)
                    .cloned()
                    .ok_or(NativeDesktopBackendError::Internal)?;
                self.release(active.export_id)?;
                Ok(NativeStudioExportPollOutcome::Completed(
                    NativeStudioExportOutcome {
                        project_revision: active.project_revision,
                        profile: active.profile,
                        bytes_written: receipt.output_bytes,
                        sha256: receipt.output_checksum.to_hex(),
                    },
                ))
            }
            RenderSessionState::Cancelled => {
                self.release(active.export_id)?;
                Ok(NativeStudioExportPollOutcome::Failed {
                    project_revision: active.project_revision,
                    profile: active.profile,
                    error: NativeDesktopBackendError::Cancelled,
                    cleanup_confirmed: true,
                })
            }
            RenderSessionState::Failed => {
                let failure = progress.failure_code.unwrap_or("render_failed");
                self.release(active.export_id)?;
                let error = match failure {
                    "backend_unavailable" => NativeDesktopBackendError::Unavailable,
                    "filesystem" => NativeDesktopBackendError::Filesystem,
                    "invalid_project" => NativeDesktopBackendError::InvalidEdit,
                    _ => NativeDesktopBackendError::Internal,
                };
                Ok(NativeStudioExportPollOutcome::Failed {
                    project_revision: active.project_revision,
                    profile: active.profile,
                    error,
                    cleanup_confirmed: true,
                })
            }
        }
    }

    pub(super) fn cancel(
        &mut self,
        request: &NativeStudioExportRequest,
    ) -> Result<(), NativeDesktopBackendError> {
        let active = self
            .active
            .clone()
            .ok_or(NativeDesktopBackendError::Unavailable)?;
        validate_active(&active, request)?;
        self.coordinator
            .cancel_and_cleanup(active.export_id, CANCEL_DEADLINE)
            .map_err(map_studio_error)?;
        self.release(active.export_id)
    }

    fn release(&mut self, export_id: StudioExportId) -> Result<(), NativeDesktopBackendError> {
        self.coordinator
            .release_terminal(export_id)
            .map_err(map_studio_error)?;
        self.coordinator.renderer_mut().release_session(export_id);
        self.active = None;
        Ok(())
    }
}

fn validate_active(
    active: &ActiveRender,
    request: &NativeStudioExportRequest,
) -> Result<(), NativeDesktopBackendError> {
    if active.project_revision != request.project_revision || active.profile != request.profile {
        return Err(NativeDesktopBackendError::StaleCatalog);
    }
    Ok(())
}

fn render_prepared_digest(domain: &[u8], pending: &PendingRender) -> frame_media::Sha256Digest {
    let mut bytes = Vec::with_capacity(256);
    bytes.extend_from_slice(domain);
    bytes.push(0);
    bytes.extend_from_slice(pending.source_set_digest.to_hex().as_bytes());
    bytes.extend_from_slice(pending.plan_digest.to_hex().as_bytes());
    bytes.extend_from_slice(pending.render_spec_digest.to_hex().as_bytes());
    bytes.extend_from_slice(pending.output_name.as_str().as_bytes());
    strong_sha256(&bytes)
}

fn media_profile(profile: ExportProfile) -> MediaExportProfile {
    match profile {
        ExportProfile::DistributionMp4 => MediaExportProfile::DistributionMaster,
        ExportProfile::EditableWebm => MediaExportProfile::NativeHighQualityWebM,
        ExportProfile::Archive => MediaExportProfile::NativeArchiveLossless,
    }
}

fn software_capabilities() -> RenderCapabilities {
    RenderCapabilities {
        contract_version: frame_media::STUDIO_RENDER_PROTOCOL_VERSION,
        containers: BTreeSet::from([
            MediaContainer::Mp4,
            MediaContainer::WebM,
            MediaContainer::Matroska,
        ]),
        hardware_video: BTreeSet::new(),
        software_video: BTreeSet::from([
            StudioVideoCodec::H264Avc,
            StudioVideoCodec::Vp8,
            StudioVideoCodec::Ffv1,
        ]),
        audio: BTreeSet::from([
            StudioAudioCodec::AacLowComplexity,
            StudioAudioCodec::Opus,
            StudioAudioCodec::Flac,
        ]),
        licenses: BTreeSet::from([CodecLicense::H264Encode, CodecLicense::AacEncode]),
        maximum_resolution: Resolution {
            width: 3_840,
            height: 2_160,
        },
        maximum_frame_rate: FrameRate {
            numerator: 60,
            denominator: 1,
        },
        bounded_renderer_queue: true,
        cancellation: true,
        postcondition_probe: true,
        exact_partial_cleanup: true,
    }
}

const fn safe_render_failure(error: NativeDesktopBackendError) -> &'static str {
    match error {
        NativeDesktopBackendError::Unavailable => "backend_unavailable",
        NativeDesktopBackendError::Filesystem => "filesystem",
        NativeDesktopBackendError::InvalidEdit
        | NativeDesktopBackendError::StaleCatalog
        | NativeDesktopBackendError::TargetUnavailable => "invalid_project",
        NativeDesktopBackendError::PermissionDenied => "permission_denied",
        NativeDesktopBackendError::Cancelled => "cancelled",
        NativeDesktopBackendError::Busy | NativeDesktopBackendError::Internal => "render_failed",
    }
}

fn map_studio_error(error: StudioError) -> NativeDesktopBackendError {
    match error {
        StudioError::StorageIo
        | StudioError::UnsafeStoragePath
        | StudioError::PartialCleanupUnconfirmed => NativeDesktopBackendError::Filesystem,
        StudioError::StaleJournal
        | StudioError::StaleRenderCallback
        | StudioError::IdempotencyConflict => NativeDesktopBackendError::StaleCatalog,
        StudioError::UnsupportedRenderProfile
        | StudioError::MissingCodecLicense(_)
        | StudioError::IncompatibleRenderer
        | StudioError::RendererCapabilityChanged => NativeDesktopBackendError::Unavailable,
        StudioError::OutputTargetBusy
        | StudioError::RenderConcurrencyLimit
        | StudioError::ActiveRenderCannotBeReleased => NativeDesktopBackendError::Busy,
        StudioError::RenderDeadlineExceeded => NativeDesktopBackendError::Busy,
        StudioError::CommittedRenderCannotBeCancelled => NativeDesktopBackendError::Cancelled,
        StudioError::InvalidRenderGraph
        | StudioError::InvalidRenderTicket
        | StudioError::InvalidSourceName
        | StudioError::InvalidSourceSet
        | StudioError::SourceSetTimelineMismatch
        | StudioError::InvalidCompiledPlan
        | StudioError::CorruptCompiledPlan
        | StudioError::InvalidExportProfile => NativeDesktopBackendError::InvalidEdit,
        _ => NativeDesktopBackendError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use frame_media::{
        AssetCommitState, AudioSampleFormat, EditSpec, FilesystemStudioOriginalStore,
        MediaContainer, NativeStudioTrackRole, PixelFormat, STUDIO_ASSET_VERSION,
        STUDIO_JOURNAL_VERSION, STUDIO_PROJECT_VERSION, StudioAsset, StudioAssetCodec,
        StudioAssetEncoding, StudioAssetId, StudioAssetRawCaps, StudioAudioRawCaps,
        StudioJournalSnapshot, StudioOperationReceipt, StudioProjectId, StudioState,
        StudioVideoRawCaps, StudioWorkerId, TempAssetCommitTicket, TimeBase, TrackKind,
        commit_verified_temporary, record_synthetic_studio_tracks,
    };

    use super::*;
    use crate::{PathPolicy, PathUse, RootAccess};

    #[test]
    fn production_coordinator_commits_and_cancels_exact_preopened_outputs() {
        let temporary = tempfile::tempdir().expect("fixture root");
        let fixture_root = fs::canonicalize(temporary.path()).expect("canonical fixture");
        let projects_root = fixture_root.join("projects");
        let studio_root = fixture_root.join("studio");
        let export_root = fixture_root.join("exports");
        fs::create_dir(&projects_root).expect("projects root");
        fs::create_dir(&export_root).expect("exports root");
        let export_directory = RootedDir::bind(&export_root).expect("rooted exports");
        let staging_root = export_root.join(".frame-staging");
        export_directory
            .create_private_dir(".frame-staging")
            .expect("private staging root");

        let encoded = record_synthetic_studio_tracks(
            &fixture_root.join("encoded"),
            Duration::from_secs(2),
            &CancellationToken::new(),
        )
        .expect("synthetic aligned sources");
        let track = |role| {
            encoded
                .iter()
                .find(|track| track.role == role)
                .expect("requested role")
                .path
                .clone()
        };
        let project_id = StudioProjectId::from_csprng([41; 16]).expect("project ID");
        let mut original_store =
            FilesystemStudioOriginalStore::new(&studio_root).expect("original store");
        let manifest = StudioProjectManifest {
            version: STUDIO_PROJECT_VERSION,
            id: project_id,
            revision: 1,
            state: StudioState::Editing,
            assets: vec![
                commit_asset(
                    &mut original_store,
                    project_id,
                    &track(NativeStudioTrackRole::Screen),
                    42,
                    TrackKind::Screen,
                ),
                commit_asset(
                    &mut original_store,
                    project_id,
                    &track(NativeStudioTrackRole::SystemAudio),
                    43,
                    TrackKind::SystemAudio,
                ),
            ],
            edits: EditSpec::default(),
        };
        create_recording_stopped_journal(&projects_root, project_id);
        let studio_directory = RootedDir::bind(&studio_root).expect("rooted Studio originals");
        let mut controller =
            NativeStudioRenderController::new(&projects_root, &export_root, &staging_root)
                .expect("native render controller");

        let completed_request = export_request(&export_root, "studio-completed.webm");
        let prepared = PreparedStudioExport::prepare(
            &studio_root,
            &studio_directory,
            &manifest,
            ExportProfile::EditableWebm,
        )
        .expect("prepared completed render");
        assert!(matches!(
            controller
                .start(
                    &completed_request,
                    &manifest,
                    prepared,
                    PathBuf::from("studio-completed.webm"),
                    PathBuf::from(".frame-completed.webm"),
                )
                .expect("start completed render"),
            NativeStudioExportStartOutcome::Running { .. }
        ));
        let mut last_progress = 0;
        let deadline = Instant::now() + Duration::from_secs(30);
        let completed = loop {
            assert!(Instant::now() < deadline, "bounded render deadline");
            match controller
                .poll(&completed_request)
                .expect("poll completed render")
            {
                NativeStudioExportPollOutcome::Running {
                    progress_basis_points,
                    ..
                } => {
                    assert!(progress_basis_points >= last_progress);
                    last_progress = progress_basis_points;
                    std::thread::sleep(Duration::from_millis(10));
                }
                NativeStudioExportPollOutcome::Completed(outcome) => break outcome,
                NativeStudioExportPollOutcome::Failed { error, .. } => {
                    panic!("production render failed: {error}")
                }
            }
        };
        assert!(completed.bytes_written > 0);
        assert_eq!(completed.sha256.len(), 64);
        assert!(export_root.join("studio-completed.webm").is_file());
        assert_eq!(
            DurableStudioJournal::open(
                FilesystemStudioJournalStore::new(&projects_root).expect("journal store"),
                project_id,
            )
            .expect("committed journal")
            .snapshot()
            .boundary,
            JournalBoundary::RenderCommitted
        );

        let cancelled_request = export_request(&export_root, "studio-cancelled.webm");
        let prepared = PreparedStudioExport::prepare(
            &studio_root,
            &studio_directory,
            &manifest,
            ExportProfile::EditableWebm,
        )
        .expect("prepared cancelled render");
        controller
            .start(
                &cancelled_request,
                &manifest,
                prepared,
                PathBuf::from("studio-cancelled.webm"),
                PathBuf::from(".frame-cancelled.webm"),
            )
            .expect("start cancelled render");
        controller
            .cancel(&cancelled_request)
            .expect("cancel and clean exact render");
        assert!(!export_root.join("studio-cancelled.webm").exists());
        assert!(
            fs::read_dir(&staging_root)
                .expect("read staging root")
                .next()
                .is_none()
        );
        assert_eq!(
            DurableStudioJournal::open(
                FilesystemStudioJournalStore::new(&projects_root).expect("journal store"),
                project_id,
            )
            .expect("cancelled journal")
            .snapshot()
            .boundary,
            JournalBoundary::RenderCancelled
        );
    }

    fn export_request(export_root: &Path, name: &str) -> NativeStudioExportRequest {
        let policy = PathPolicy::empty()
            .allow_root(
                export_root,
                RootAccess {
                    read: false,
                    write: true,
                    delete: false,
                },
            )
            .expect("export policy");
        let output = export_root.join(name);
        NativeStudioExportRequest {
            project_revision: 1,
            output_path: policy
                .validate(
                    output.to_str().expect("UTF-8 fixture output"),
                    PathUse::ExportWrite,
                )
                .expect("validated export output"),
            profile: ExportProfile::EditableWebm,
        }
    }

    fn create_recording_stopped_journal(projects_root: &Path, project_id: StudioProjectId) {
        let operation_id = StudioOperationId::from_csprng([44; 16]).expect("operation ID");
        let snapshot = StudioJournalSnapshot {
            version: STUDIO_JOURNAL_VERSION,
            project_id,
            revision: 1,
            fence: 1,
            owner: StudioWorkerId::from_csprng([45; 16]).expect("worker ID"),
            boundary: JournalBoundary::RecordingStopped,
            last_operation_id: Some(operation_id),
            pending_asset: None,
            pending_edit: None,
            pending_render: None,
            receipts: BTreeMap::from([(
                operation_id,
                StudioOperationReceipt {
                    operation_id,
                    kind: ReceiptKind::RecordingStopped,
                    command_digest: strong_sha256(b"recording stopped command"),
                    outcome_digest: strong_sha256(b"recording stopped outcome"),
                },
            )]),
        };
        DurableStudioJournal::create(
            FilesystemStudioJournalStore::new(projects_root).expect("journal store"),
            snapshot,
        )
        .expect("recording-stopped journal");
    }

    fn commit_asset(
        store: &mut FilesystemStudioOriginalStore,
        project_id: StudioProjectId,
        source: &Path,
        marker: u8,
        track: TrackKind,
    ) -> StudioAsset {
        let bytes = fs::read(source).expect("encoded source");
        let time_base = TimeBase::new(1).expect("time base");
        let encoding = match track {
            TrackKind::Screen => StudioAssetEncoding::Encoded {
                container: MediaContainer::WebM,
                codec: StudioAssetCodec::Video(StudioVideoCodec::Vp8),
                raw_caps: StudioAssetRawCaps::Video(StudioVideoRawCaps {
                    width: 320,
                    height: 180,
                    frame_rate: FrameRate {
                        numerator: 30,
                        denominator: 1,
                    },
                    pixel_format: PixelFormat::Bgra8,
                }),
                time_base: TimeBase::new(90_000).expect("video time base"),
            },
            TrackKind::SystemAudio => StudioAssetEncoding::Encoded {
                container: MediaContainer::WebM,
                codec: StudioAssetCodec::Audio(StudioAudioCodec::Opus),
                raw_caps: StudioAssetRawCaps::Audio(StudioAudioRawCaps {
                    sample_rate: 48_000,
                    channels: 2,
                    sample_format: AudioSampleFormat::Float32,
                }),
                time_base: TimeBase::new(48_000).expect("audio time base"),
            },
            TrackKind::Camera | TrackKind::Microphone => unreachable!("fixture track"),
        };
        let temporary = StudioAsset {
            version: STUDIO_ASSET_VERSION,
            id: StudioAssetId::from_csprng([marker; 16]).expect("asset ID"),
            track,
            source_name: StudioSourceName::new(format!("{track:?}.webm").to_ascii_lowercase())
                .expect("source name"),
            byte_len: u64::try_from(bytes.len()).expect("asset length"),
            start: frame_media::RationalTime::new(0, time_base),
            duration: frame_media::RationalTime::new(2, time_base),
            checksum: AssetChecksum::from_content(&bytes),
            commit_state: AssetCommitState::Temporary,
            encoding,
        };
        store
            .stage_temporary_bytes(project_id, &temporary, &bytes)
            .expect("stage original");
        commit_verified_temporary(
            store,
            TempAssetCommitTicket::new(
                project_id,
                StudioOperationId::from_csprng([marker + 10; 16]).expect("operation ID"),
                1,
                temporary,
            )
            .expect("commit ticket"),
        )
        .expect("durable original")
    }
}
