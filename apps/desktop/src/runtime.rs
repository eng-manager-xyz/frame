//! Versioned, fail-closed desktop command runtime.
//!
//! The native shell owns this state. WebView code receives only opaque window
//! scopes and redacted snapshots, submits the checked [`RequestEnvelope`]
//! protocol, and renders backend-confirmed transitions. The deterministic fake
//! adapter is intentionally explicit and is used by the hermetic journey; a
//! release build never selects it implicitly.

use std::{fmt, path::Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use frame_client::{InstantUiErrorCodeV1, InstantUiPhaseV1, InstantUiProgressV1};

use crate::{
    instant_finalize_service::{
        INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION, InstantFinalizeCapabilityState,
        InstantFinalizeHandle, InstantFinalizeRegistrationV1,
    },
    ipc::{
        CaptureTargetKind, CommandOutcome, DeviceClass, ExportProfile, IpcCommand, IpcError,
        LifecycleAction, PathPolicy, PathUse, PublicErrorCode, RecorderMode, RequestEnvelope,
        ResponseEnvelope, RootAccess, ScopeRegistry, SessionId, UpdateAction, ValidatedPath,
        WindowId, WindowRole, WindowScope, decode_request, valid_opaque_id,
    },
    native_backend::{
        CAPTURE_ARTIFACT_SUMMARY_VERSION, CaptureArtifactSummary, CaptureTargetCatalog,
        CaptureTargetSummary, NativeCaptureArtifact, NativeCaptureStartRequest,
        NativeDesktopBackend, NativeDesktopBackendError, NativeEditableWebmExportRequest,
        NativePermissionOutcome, NativeRecordingCancelOutcome, NativeRecordingControlRequest,
        NativeRecordingStartOutcome, NativeRecordingStopOutcome, NativeRecordingTerminalFailure,
        NativeTargetSelectionOutcome, NativeTargetSelectionRequest,
    },
    workflow::{
        BackendEvent, BackendEventEnvelope, DesktopWorkflow, DeviceCounts, DeviceState,
        EditorState, ExportState, IntentKind, RecorderState, RecoveryState, SafeFailureCode,
        UiIntent, UploadState, WORKFLOW_PROTOCOL_VERSION, WorkflowError,
    },
};

pub const DESKTOP_RUNTIME_VERSION: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopAdapterKind {
    Unavailable,
    DeterministicFake,
    NativeMacOs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    NotDetermined,
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioMeterSnapshot {
    pub microphone_basis_points: u16,
    pub system_audio_basis_points: u16,
    pub camera_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecorderConfiguration {
    pub mode: RecorderMode,
    pub countdown_seconds: u8,
    pub exclude_frame_windows: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedSources {
    pub target: Option<CaptureTargetKind>,
    pub display_selected: bool,
    pub microphone_selected: bool,
    pub system_audio_selected: bool,
    pub camera_selected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopSettingsSnapshot {
    pub revision: u64,
    pub mode: RecorderMode,
    pub frame_rate: u16,
    pub microphone_enabled: bool,
    pub system_audio_enabled: bool,
    pub camera_enabled: bool,
    pub reduced_motion: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleSnapshot {
    pub main_visible: bool,
    pub overlay_visible: bool,
    pub target_picker_visible: bool,
    pub hotkeys_registered: bool,
    pub frame_windows_excluded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateState {
    Current { revision: u64 },
    Available { revision: u64 },
    ReadyToRelaunch { revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopRuntimeSnapshot {
    pub version: u16,
    pub operation_revision: u64,
    pub adapter: DesktopAdapterKind,
    pub recorder: RecorderState,
    pub devices: DeviceState,
    pub recovery: RecoveryState,
    pub editor: EditorState,
    pub export: ExportState,
    pub upload: UploadState,
    pub instant_finalize: InstantFinalizeCapabilityState,
    pub instant_finalize_handle: Option<InstantFinalizeHandle>,
    pub instant_finalize_next_sequence: Option<u64>,
    pub instant_progress: Option<InstantUiProgressV1>,
    pub permission: PermissionState,
    pub meter: AudioMeterSnapshot,
    pub recorder_configuration: RecorderConfiguration,
    pub selected_sources: SelectedSources,
    #[serde(default = "CaptureTargetCatalog::empty")]
    pub capture_targets: CaptureTargetCatalog,
    #[serde(default)]
    pub capture_artifact: Option<CaptureArtifactSummary>,
    pub settings: DesktopSettingsSnapshot,
    pub lifecycle: LifecycleSnapshot,
    pub update: UpdateState,
    pub crash_recovery_reported: bool,
    pub legacy_desktop_selectable: bool,
    pub announcement: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopWindowContext {
    pub role: WindowRole,
    pub window_id: WindowId,
    pub session_id: SessionId,
}

impl fmt::Debug for DesktopWindowContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopWindowContext")
            .field("role", &self.role)
            .field("window_id", &"<redacted>")
            .field("session_id", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopBootstrap {
    pub runtime_version: u16,
    pub contexts: Vec<DesktopWindowContext>,
    pub fake_journey_paths: Option<FakeJourneyPaths>,
    pub snapshot: DesktopRuntimeSnapshot,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeJourneyPaths {
    pub project: String,
    pub media: String,
    pub export: String,
}

impl fmt::Debug for FakeJourneyPaths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeJourneyPaths")
            .field("project", &"<redacted>")
            .field("media", &"<redacted>")
            .field("export", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopEventEnvelope {
    pub protocol_version: u16,
    pub event_sequence: u64,
    pub owner: WindowRole,
    pub event: DesktopRuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum DesktopRuntimeEvent {
    Backend(BackendEvent),
    InstantProgress(InstantUiProgressV1),
    StateConfirmed { operation_revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopDispatch {
    pub response: ResponseEnvelope,
    pub events: Vec<DesktopEventEnvelope>,
    pub snapshot: DesktopRuntimeSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstantFinalizeUiUpdate {
    pub runtime_version: u16,
    pub command_protocol_version: u16,
    pub command_sequence: u64,
    pub operation_revision: u64,
    pub events: Vec<DesktopEventEnvelope>,
    pub progress: InstantUiProgressV1,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DesktopRoots {
    projects: String,
    media: String,
    exports: String,
}

#[derive(Clone, PartialEq, Eq)]
struct NativeRecordingAuthority {
    recording_token: String,
    catalog_generation: u64,
    target_token: String,
}

impl fmt::Debug for NativeRecordingAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeRecordingAuthority")
            .field("recording_token", &"<redacted>")
            .field("catalog_generation", &self.catalog_generation)
            .field("target_token", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct NativeArtifactAuthority {
    summary: CaptureArtifactSummary,
    media_path: ValidatedPath,
    export_path: Option<ValidatedPath>,
}

impl fmt::Debug for NativeArtifactAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeArtifactAuthority")
            .field("summary", &self.summary)
            .field("media_path", &self.media_path)
            .field("export_path", &self.export_path)
            .finish()
    }
}

impl DesktopRoots {
    #[must_use]
    pub fn new(
        projects: impl Into<String>,
        media: impl Into<String>,
        exports: impl Into<String>,
    ) -> Self {
        Self {
            projects: projects.into(),
            media: media.into(),
            exports: exports.into(),
        }
    }
}

impl fmt::Debug for DesktopRoots {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopRoots")
            .field("projects", &"<redacted>")
            .field("media", &"<redacted>")
            .field("exports", &"<redacted>")
            .finish()
    }
}

#[derive(Clone)]
pub struct DesktopRuntime {
    adapter: DesktopAdapterKind,
    registry: ScopeRegistry,
    contexts: Vec<DesktopWindowContext>,
    fake_journey_paths: Option<FakeJourneyPaths>,
    backend_session: SessionId,
    backend_sequence: u64,
    event_sequence: u64,
    operation_revision: u64,
    workflow: DesktopWorkflow,
    instant_finalize: InstantFinalizeCapabilityState,
    instant_finalize_handle: Option<InstantFinalizeHandle>,
    instant_finalize_last_sequence: u64,
    instant_progress: Option<InstantUiProgressV1>,
    permission: PermissionState,
    meter: AudioMeterSnapshot,
    recorder_configuration: RecorderConfiguration,
    selected_sources: SelectedSources,
    capture_targets: CaptureTargetCatalog,
    selected_capture_target: Option<CaptureTargetSummary>,
    native_recording: Option<NativeRecordingAuthority>,
    native_artifact: Option<NativeArtifactAuthority>,
    native_media_paths: PathPolicy,
    native_export_paths: PathPolicy,
    settings: DesktopSettingsSnapshot,
    lifecycle: LifecycleSnapshot,
    update: UpdateState,
    recovery_projects: u16,
    crash_recovery_reported: bool,
    announcement: String,
}

impl fmt::Debug for DesktopRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopRuntime")
            .field("adapter", &self.adapter)
            .field("snapshot", &self.snapshot())
            .field("contexts", &self.contexts)
            .finish_non_exhaustive()
    }
}

impl DesktopRuntime {
    pub fn new(
        adapter: DesktopAdapterKind,
        roots: DesktopRoots,
        session_nonce: &str,
    ) -> Result<Self, DesktopRuntimeError> {
        let backend_session = SessionId::new(format!("backend-{session_nonce}"))?;
        let native_media_paths = PathPolicy::empty().allow_root(
            &roots.media,
            RootAccess {
                read: true,
                write: false,
                delete: false,
            },
        )?;
        let native_export_paths = PathPolicy::empty().allow_root(
            &roots.exports,
            RootAccess {
                read: true,
                write: true,
                delete: false,
            },
        )?;
        let mut registry = ScopeRegistry::new();
        let mut contexts = Vec::new();
        for role in [
            WindowRole::Main,
            WindowRole::Recorder,
            WindowRole::Recovery,
            WindowRole::Editor,
            WindowRole::Export,
            WindowRole::Settings,
            WindowRole::Overlay,
        ] {
            let label = role_label(role);
            let context = DesktopWindowContext {
                role,
                window_id: WindowId::new(format!("{label}-{session_nonce}"))?,
                session_id: SessionId::new(format!("{label}-session-{session_nonce}"))?,
            };
            registry.register(WindowScope {
                window_id: context.window_id.clone(),
                session_id: context.session_id.clone(),
                role,
                paths: paths_for(role, &roots)?,
            })?;
            contexts.push(context);
        }

        let recovery_projects = u16::from(adapter == DesktopAdapterKind::DeterministicFake);
        let fake_journey_paths =
            (adapter == DesktopAdapterKind::DeterministicFake).then(|| FakeJourneyPaths {
                project: Path::new(&roots.projects)
                    .join("demo.frame")
                    .to_string_lossy()
                    .into_owned(),
                media: Path::new(&roots.media)
                    .join("demo.mp4")
                    .to_string_lossy()
                    .into_owned(),
                export: Path::new(&roots.exports)
                    .join("demo.mp4")
                    .to_string_lossy()
                    .into_owned(),
            });
        Ok(Self {
            adapter,
            registry,
            contexts,
            fake_journey_paths,
            backend_session: backend_session.clone(),
            backend_sequence: 0,
            event_sequence: 0,
            operation_revision: 1,
            workflow: DesktopWorkflow::new(backend_session),
            instant_finalize: InstantFinalizeCapabilityState::NotConfigured,
            instant_finalize_handle: None,
            instant_finalize_last_sequence: 0,
            instant_progress: None,
            permission: PermissionState::NotDetermined,
            meter: AudioMeterSnapshot {
                microphone_basis_points: 0,
                system_audio_basis_points: 0,
                camera_active: false,
            },
            recorder_configuration: RecorderConfiguration {
                mode: RecorderMode::Instant,
                countdown_seconds: 3,
                exclude_frame_windows: true,
            },
            selected_sources: SelectedSources {
                target: None,
                display_selected: false,
                microphone_selected: false,
                system_audio_selected: false,
                camera_selected: false,
            },
            capture_targets: CaptureTargetCatalog::empty(),
            selected_capture_target: None,
            native_recording: None,
            native_artifact: None,
            native_media_paths,
            native_export_paths,
            settings: DesktopSettingsSnapshot {
                revision: 1,
                mode: RecorderMode::Instant,
                frame_rate: 30,
                microphone_enabled: adapter != DesktopAdapterKind::NativeMacOs,
                system_audio_enabled: adapter != DesktopAdapterKind::NativeMacOs,
                camera_enabled: false,
                reduced_motion: false,
            },
            lifecycle: LifecycleSnapshot {
                main_visible: true,
                overlay_visible: false,
                target_picker_visible: false,
                hotkeys_registered: false,
                frame_windows_excluded: true,
            },
            update: UpdateState::Current { revision: 1 },
            recovery_projects,
            crash_recovery_reported: false,
            announcement: match adapter {
                DesktopAdapterKind::Unavailable => {
                    "Native adapter unavailable. Recording remains disabled.".into()
                }
                DesktopAdapterKind::DeterministicFake => {
                    "Deterministic fake desktop backend ready.".into()
                }
                DesktopAdapterKind::NativeMacOs => {
                    "Native macOS display capture is ready for permission setup.".into()
                }
            },
        })
    }

    #[must_use]
    pub fn bootstrap(&self) -> DesktopBootstrap {
        DesktopBootstrap {
            runtime_version: DESKTOP_RUNTIME_VERSION,
            contexts: self.contexts.clone(),
            fake_journey_paths: self.fake_journey_paths.clone(),
            snapshot: self.snapshot(),
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> DesktopRuntimeSnapshot {
        DesktopRuntimeSnapshot {
            version: DESKTOP_RUNTIME_VERSION,
            operation_revision: self.operation_revision,
            adapter: self.adapter,
            recorder: self.workflow.recorder(),
            devices: self.workflow.devices(),
            recovery: self.workflow.recovery(),
            editor: self.workflow.editor(),
            export: self.workflow.export(),
            upload: self.workflow.upload(),
            instant_finalize: self.instant_finalize,
            instant_finalize_handle: self.instant_finalize_handle.clone(),
            instant_finalize_next_sequence: (self.instant_finalize
                == InstantFinalizeCapabilityState::Available
                && self.instant_finalize_handle.is_some())
            .then(|| self.instant_finalize_last_sequence.checked_add(1))
            .flatten(),
            instant_progress: self.instant_progress,
            permission: self.permission,
            meter: self.meter,
            recorder_configuration: self.recorder_configuration,
            selected_sources: self.selected_sources,
            capture_targets: self.capture_targets.clone(),
            capture_artifact: self
                .native_artifact
                .as_ref()
                .map(|artifact| artifact.summary.clone()),
            settings: self.settings,
            lifecycle: self.lifecycle,
            update: self.update,
            crash_recovery_reported: self.crash_recovery_reported,
            legacy_desktop_selectable: true,
            announcement: self.announcement.clone(),
        }
    }

    /// Binds an opaque handle minted by the native finalize service to this
    /// public-safe UI model. The production release does not call this method
    /// until an authenticated native session/journal owner exists.
    pub fn bind_native_instant_finalize(
        &mut self,
        registration: InstantFinalizeRegistrationV1,
    ) -> Result<(), DesktopRuntimeError> {
        registration
            .progress
            .validate()
            .map_err(|_| DesktopRuntimeError::InvalidInstantProgress)?;
        if self.instant_finalize_handle.is_some()
            || registration.progress.phase != InstantUiPhaseV1::Finalizing
        {
            return Err(DesktopRuntimeError::InstantFinalizeAuthorityMismatch);
        }
        self.instant_finalize = InstantFinalizeCapabilityState::Available;
        self.instant_finalize_handle = Some(registration.handle);
        self.instant_finalize_last_sequence = 0;
        self.instant_progress = Some(registration.progress);
        self.announcement = instant_progress_announcement(registration.progress).into();
        Ok(())
    }

    /// Checks the currently bound opaque authority and the exact next command
    /// sequence before native network I/O. The composition root must release
    /// its runtime lock after this check and revalidate again when applying the
    /// result.
    pub fn preflight_instant_finalize(
        &self,
        handle: &InstantFinalizeHandle,
        command_sequence: u64,
    ) -> Result<(), DesktopRuntimeError> {
        if self.instant_finalize != InstantFinalizeCapabilityState::Available
            || self.instant_finalize_handle.as_ref() != Some(handle)
        {
            return Err(DesktopRuntimeError::InstantFinalizeAuthorityMismatch);
        }
        let expected = self
            .instant_finalize_last_sequence
            .checked_add(1)
            .ok_or(DesktopRuntimeError::SequenceOverflow)?;
        if command_sequence != expected {
            return Err(DesktopRuntimeError::InstantFinalizeAuthorityMismatch);
        }
        Ok(())
    }

    /// Applies only the progress returned for the already-bound opaque handle.
    /// The service separately revalidates native request digest and generation
    /// after network I/O; this method cannot receive those private identities.
    pub fn apply_instant_finalize_progress(
        &mut self,
        handle: &InstantFinalizeHandle,
        command_sequence: u64,
        progress: InstantUiProgressV1,
    ) -> Result<InstantFinalizeUiUpdate, DesktopRuntimeError> {
        progress
            .validate()
            .map_err(|_| DesktopRuntimeError::InvalidInstantProgress)?;
        self.preflight_instant_finalize(handle, command_sequence)?;
        let mut candidate = self.clone();
        candidate.instant_finalize_last_sequence = command_sequence;
        candidate.instant_progress = Some(progress);
        if matches!(
            progress.phase,
            InstantUiPhaseV1::ShareReady
                | InstantUiPhaseV1::Cancelled
                | InstantUiPhaseV1::RecoveryRequired
        ) {
            candidate.instant_finalize_handle = None;
        }
        candidate.operation_revision = candidate
            .operation_revision
            .checked_add(1)
            .ok_or(DesktopRuntimeError::RevisionOverflow)?;
        candidate.announcement = instant_progress_announcement(progress).into();
        let events = vec![
            candidate.wrap_event(
                WindowRole::Recorder,
                DesktopRuntimeEvent::InstantProgress(progress),
            )?,
            candidate.wrap_event(
                WindowRole::Recorder,
                DesktopRuntimeEvent::StateConfirmed {
                    operation_revision: candidate.operation_revision,
                },
            )?,
        ];
        let update = InstantFinalizeUiUpdate {
            runtime_version: DESKTOP_RUNTIME_VERSION,
            command_protocol_version: INSTANT_FINALIZE_COMMAND_PROTOCOL_VERSION,
            command_sequence,
            operation_revision: candidate.operation_revision,
            events,
            progress,
        };
        *self = candidate;
        Ok(update)
    }

    /// Commits a stable terminal projection after native authority is sealed
    /// by a non-retryable provider/receipt failure. It consumes the expected
    /// command sequence and removes the WebView handle so retry cannot appear
    /// available against a permanently terminal native context.
    pub fn disable_native_instant_finalize(
        &mut self,
        handle: &InstantFinalizeHandle,
        command_sequence: u64,
    ) -> Result<InstantFinalizeUiUpdate, DesktopRuntimeError> {
        let progress = InstantUiProgressV1::new(
            InstantUiPhaseV1::RecoveryRequired,
            None,
            false,
            Some(InstantUiErrorCodeV1::RecordingRecoveryRequired),
        )
        .map_err(|_| DesktopRuntimeError::InvalidInstantProgress)?;
        self.apply_instant_finalize_progress(handle, command_sequence, progress)
    }

    pub fn dispatch_json(&mut self, json: &str) -> Result<DesktopDispatch, DesktopRuntimeError> {
        let request = decode_request(json)?;
        self.dispatch(request)
    }

    /// Decodes the same fail-closed IPC envelope, then invokes an explicitly
    /// injected native backend only after scope, replay, and path checks pass.
    pub fn dispatch_native_json<B: NativeDesktopBackend>(
        &mut self,
        json: &str,
        backend: &mut B,
    ) -> Result<DesktopDispatch, DesktopRuntimeError> {
        let request = decode_request(json)?;
        self.dispatch_native(request, backend)
    }

    pub fn dispatch(
        &mut self,
        request: RequestEnvelope,
    ) -> Result<DesktopDispatch, DesktopRuntimeError> {
        let accepted = self.registry.accept(request)?;
        let owner = self.owner_for(&accepted.request)?;
        let mut candidate = self.clone();
        let result = candidate.execute(owner, &accepted.request.command);
        self.finish_dispatch(&accepted.request, owner, candidate, result)
    }

    /// Native capture entry point. This is separate from [`Self::dispatch`] so
    /// a runtime without an injected platform capability remains fail-closed.
    pub fn dispatch_native<B: NativeDesktopBackend>(
        &mut self,
        request: RequestEnvelope,
        backend: &mut B,
    ) -> Result<DesktopDispatch, DesktopRuntimeError> {
        let accepted = self.registry.accept(request)?;
        let owner = self.owner_for(&accepted.request)?;
        let mut candidate = self.clone();
        let result = if candidate.adapter == DesktopAdapterKind::NativeMacOs {
            candidate.execute_native(
                owner,
                &accepted.request.command,
                accepted.validated_path.as_ref(),
                backend,
            )
        } else {
            Err(ExecutionFailure::unavailable())
        };
        self.finish_dispatch(&accepted.request, owner, candidate, result)
    }

    fn finish_dispatch(
        &mut self,
        response_scope: &RequestEnvelope,
        owner: WindowRole,
        mut candidate: Self,
        result: Result<Vec<BackendEvent>, ExecutionFailure>,
    ) -> Result<DesktopDispatch, DesktopRuntimeError> {
        match result {
            Ok(backend_events) => {
                candidate.operation_revision = candidate
                    .operation_revision
                    .checked_add(1)
                    .ok_or(DesktopRuntimeError::RevisionOverflow)?;
                let response = ResponseEnvelope {
                    protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
                    request_id: response_scope.request_id.clone(),
                    window_id: response_scope.window_id.clone(),
                    session_id: response_scope.session_id.clone(),
                    sequence: response_scope.sequence,
                    outcome: CommandOutcome::Ok { revision: None },
                };
                candidate.registry.accept_response(response.clone())?;
                let mut events = backend_events
                    .into_iter()
                    .map(|event| candidate.wrap_event(owner, DesktopRuntimeEvent::Backend(event)))
                    .collect::<Result<Vec<_>, _>>()?;
                events.push(candidate.wrap_event(
                    owner,
                    DesktopRuntimeEvent::StateConfirmed {
                        operation_revision: candidate.operation_revision,
                    },
                )?);
                let snapshot = candidate.snapshot();
                *self = candidate;
                Ok(DesktopDispatch {
                    response,
                    events,
                    snapshot,
                })
            }
            Err(failure) => {
                self.announcement = failure.announcement.into();
                self.operation_revision = self
                    .operation_revision
                    .checked_add(1)
                    .ok_or(DesktopRuntimeError::RevisionOverflow)?;
                let response = ResponseEnvelope {
                    protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
                    request_id: response_scope.request_id.clone(),
                    window_id: response_scope.window_id.clone(),
                    session_id: response_scope.session_id.clone(),
                    sequence: response_scope.sequence,
                    outcome: CommandOutcome::Error {
                        code: failure.code,
                        retryable: failure.retryable,
                    },
                };
                self.registry.accept_response(response.clone())?;
                let events = vec![self.wrap_event(
                    owner,
                    DesktopRuntimeEvent::StateConfirmed {
                        operation_revision: self.operation_revision,
                    },
                )?];
                Ok(DesktopDispatch {
                    response,
                    events,
                    snapshot: self.snapshot(),
                })
            }
        }
    }

    /// Advances long-running fake export/upload work without introducing a
    /// production IPC escape hatch. Test harnesses call this between commands.
    pub fn advance_fake(&mut self) -> Result<Vec<DesktopEventEnvelope>, DesktopRuntimeError> {
        self.require_fake()?;
        let mut backend_events = Vec::new();
        if matches!(self.workflow.export(), ExportState::Running { .. }) {
            backend_events.push(BackendEvent::ExportProgress {
                progress_basis_points: 10_000,
            });
            backend_events.push(BackendEvent::ExportCompleted);
        }
        if let UploadState::Uploading { total_parts, .. } = self.workflow.upload() {
            backend_events.push(BackendEvent::UploadProgress {
                verified_parts: total_parts,
                total_parts,
            });
            backend_events.push(BackendEvent::UploadFinalizing);
            backend_events.push(BackendEvent::UploadCompleted);
        }
        self.apply_unsolicited(&backend_events)?;
        self.operation_revision = self
            .operation_revision
            .checked_add(1)
            .ok_or(DesktopRuntimeError::RevisionOverflow)?;
        self.announcement = "Background desktop work completed.".into();
        backend_events
            .into_iter()
            .map(|event| self.wrap_event(WindowRole::Main, DesktopRuntimeEvent::Backend(event)))
            .collect()
    }

    /// Deterministic fault used by the fake device-loss/recovery journey.
    pub fn simulate_fake_device_loss(&mut self) -> Result<(), DesktopRuntimeError> {
        self.require_fake()?;
        self.apply_unsolicited(&[BackendEvent::DeviceLost])?;
        self.recovery_projects = self.recovery_projects.saturating_add(1);
        self.permission = PermissionState::NotDetermined;
        self.meter = AudioMeterSnapshot {
            microphone_basis_points: 0,
            system_audio_basis_points: 0,
            camera_active: false,
        };
        self.operation_revision = self
            .operation_revision
            .checked_add(1)
            .ok_or(DesktopRuntimeError::RevisionOverflow)?;
        self.announcement = "Capture device lost. A recoverable recording is available.".into();
        Ok(())
    }

    /// Models process restart/window reconstruction while preserving the native
    /// journal as authority. It never claims that recording continued.
    pub fn simulate_fake_restart(&mut self) -> Result<(), DesktopRuntimeError> {
        self.require_fake()?;
        if matches!(
            self.workflow.recorder(),
            RecorderState::Recording | RecorderState::Paused
        ) {
            self.apply_unsolicited(&[BackendEvent::DeviceLost])?;
            self.recovery_projects = self.recovery_projects.saturating_add(1);
        }
        self.lifecycle.main_visible = true;
        self.lifecycle.overlay_visible = false;
        self.lifecycle.target_picker_visible = false;
        self.crash_recovery_reported = true;
        self.operation_revision = self
            .operation_revision
            .checked_add(1)
            .ok_or(DesktopRuntimeError::RevisionOverflow)?;
        self.announcement = "Desktop restarted. Backend journal state was restored.".into();
        Ok(())
    }

    fn execute_native<B: NativeDesktopBackend>(
        &mut self,
        owner: WindowRole,
        command: &IpcCommand,
        validated_path: Option<&ValidatedPath>,
        backend: &mut B,
    ) -> Result<Vec<BackendEvent>, ExecutionFailure> {
        match command {
            IpcCommand::RecorderPrepare => {
                if !matches!(
                    self.workflow.recorder(),
                    RecorderState::Idle | RecorderState::Ready | RecorderState::Failed { .. }
                ) {
                    return Err(ExecutionFailure::conflict(
                        "Screen recording permission cannot change during capture.",
                    ));
                }
                let events = match backend
                    .prepare_display_capture()
                    .map_err(ExecutionFailure::native_backend)?
                {
                    NativePermissionOutcome::Granted => {
                        self.permission = PermissionState::Granted;
                        self.announcement =
                            "Native screen recording permission was confirmed.".into();
                        Vec::new()
                    }
                    NativePermissionOutcome::Denied => {
                        self.permission = PermissionState::Denied;
                        self.selected_capture_target = None;
                        self.selected_sources.target = None;
                        self.selected_sources.display_selected = false;
                        self.announcement =
                            "Screen recording permission was denied by macOS.".into();
                        let event = BackendEvent::DevicePermissionDenied;
                        self.apply_unsolicited(std::slice::from_ref(&event))
                            .map_err(|_| ExecutionFailure::internal())?;
                        vec![event]
                    }
                };
                Ok(events)
            }
            IpcCommand::RecorderPoll => {
                if self.workflow.recorder() != RecorderState::Recording {
                    return Err(ExecutionFailure::conflict(
                        "Native recording health can only be polled during capture.",
                    ));
                }
                let recording = self
                    .native_recording
                    .clone()
                    .ok_or_else(ExecutionFailure::internal)?;
                let failure = backend
                    .poll_recording_terminal_failure(&NativeRecordingControlRequest {
                        recording_token: recording.recording_token.clone(),
                    })
                    .map_err(ExecutionFailure::native_backend)?;
                let Some(failure) = failure else {
                    return Ok(Vec::new());
                };
                if failure.recording_token != recording.recording_token
                    || !valid_opaque_id(&failure.recording_token)
                {
                    return Err(ExecutionFailure::invalid_backend_response());
                }

                let (code, retryable) =
                    native_terminal_failure_state(&failure, &recording.recording_token);
                let event = BackendEvent::RecorderFailed { code, retryable };
                self.apply_unsolicited(std::slice::from_ref(&event))
                    .map_err(|_| ExecutionFailure::internal())?;
                self.clear_native_recording_session();
                self.native_artifact = None;
                self.announcement =
                    "Native recording failed and its capture session was retired.".into();
                Ok(vec![event])
            }
            IpcCommand::DeviceEnumerate {
                class: DeviceClass::Display,
            } => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.preflight_transition(&intent_id, IntentKind::DevicesRefresh)?;
                let catalog = backend
                    .enumerate_displays()
                    .map_err(ExecutionFailure::native_backend)?;
                catalog
                    .validate_enumeration()
                    .map_err(|_| ExecutionFailure::invalid_backend_response())?;
                if catalog
                    .targets
                    .iter()
                    .any(|target| target.kind != CaptureTargetKind::Display)
                {
                    return Err(ExecutionFailure::invalid_backend_response());
                }
                if catalog.generation < self.capture_targets.generation
                    || (catalog.generation == self.capture_targets.generation
                        && catalog != self.capture_targets)
                {
                    return Err(ExecutionFailure::invalid_backend_response());
                }
                let display_count = u16::try_from(catalog.targets.len())
                    .map_err(|_| ExecutionFailure::invalid_backend_response())?;
                let keep_selection =
                    self.selected_capture_target
                        .as_ref()
                        .is_some_and(|selected| {
                            self.capture_targets.generation == catalog.generation
                                && catalog.targets.iter().any(|target| target == selected)
                        });
                let events = self.transition(
                    &intent_id,
                    IntentKind::DevicesRefresh,
                    vec![
                        BackendEvent::DevicesEnumerating {
                            intent_id: intent_id.clone(),
                        },
                        BackendEvent::DevicesReady {
                            counts: DeviceCounts {
                                displays: display_count,
                                microphones: 0,
                                system_audio_sources: 0,
                                cameras: 0,
                            },
                        },
                    ],
                )?;
                self.capture_targets = catalog;
                if !keep_selection {
                    self.selected_capture_target = None;
                    self.selected_sources.target = None;
                    self.selected_sources.display_selected = false;
                }
                self.announcement = "Native display catalog refreshed.".into();
                Ok(events)
            }
            IpcCommand::CaptureTargetSelect {
                kind: CaptureTargetKind::Display,
                target_token,
            } => {
                let target = self
                    .capture_targets
                    .targets
                    .iter()
                    .find(|target| {
                        target.kind == CaptureTargetKind::Display && target.token == *target_token
                    })
                    .cloned()
                    .ok_or_else(|| {
                        ExecutionFailure::conflict(
                            "The display catalog changed. Refresh displays and select again.",
                        )
                    })?;
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.preflight_transition(&intent_id, IntentKind::DeviceSelect)?;
                let outcome = backend
                    .select_display(&NativeTargetSelectionRequest {
                        catalog_generation: self.capture_targets.generation,
                        target: target.clone(),
                    })
                    .map_err(ExecutionFailure::native_backend)?;
                validate_selection_outcome(
                    &outcome,
                    self.capture_targets.generation,
                    &target.token,
                )?;
                let events = self.transition(
                    &intent_id,
                    IntentKind::DeviceSelect,
                    vec![BackendEvent::DeviceSelected {
                        intent_id: intent_id.clone(),
                    }],
                )?;
                self.selected_capture_target = Some(target);
                self.selected_sources.target = Some(CaptureTargetKind::Display);
                self.selected_sources.display_selected = true;
                self.announcement = "Native display selected by opaque token.".into();
                Ok(events)
            }
            IpcCommand::RecorderStart { intent_id } => {
                if self.permission != PermissionState::Granted {
                    return Err(ExecutionFailure::invalid(
                        "Confirm macOS screen recording permission before recording.",
                    ));
                }
                if self.settings.microphone_enabled
                    || self.settings.camera_enabled
                    || self.selected_sources.microphone_selected
                    || self.selected_sources.system_audio_selected
                    || self.selected_sources.camera_selected
                {
                    return Err(ExecutionFailure::invalid(
                        "Native capture supports display video with optional system audio only.",
                    ));
                }
                let target = self
                    .selected_capture_target
                    .as_ref()
                    .filter(|selected| {
                        selected.kind == CaptureTargetKind::Display
                            && self
                                .capture_targets
                                .targets
                                .iter()
                                .any(|current| current == *selected)
                    })
                    .cloned()
                    .ok_or_else(|| {
                        ExecutionFailure::conflict(
                            "The selected display is stale. Refresh displays and select again.",
                        )
                    })?;
                self.preflight_transition(intent_id, IntentKind::RecorderStart)?;
                let outcome = backend
                    .start_display_recording(&NativeCaptureStartRequest {
                        catalog_generation: self.capture_targets.generation,
                        target: target.clone(),
                        frame_rate: self.settings.frame_rate,
                        exclude_frame_windows: self.recorder_configuration.exclude_frame_windows,
                        system_audio_enabled: self.settings.system_audio_enabled,
                    })
                    .map_err(ExecutionFailure::native_backend)?;
                validate_start_outcome(
                    &outcome,
                    self.capture_targets.generation,
                    &target.token,
                    self.settings.system_audio_enabled,
                )?;
                let events = self.transition(
                    intent_id,
                    IntentKind::RecorderStart,
                    vec![
                        BackendEvent::RecorderPreparing {
                            intent_id: intent_id.clone(),
                        },
                        BackendEvent::RecorderStarted,
                    ],
                )?;
                self.native_recording = Some(NativeRecordingAuthority {
                    recording_token: outcome.recording_token,
                    catalog_generation: outcome.catalog_generation,
                    target_token: outcome.target_token,
                });
                self.native_artifact = None;
                self.lifecycle.overlay_visible = true;
                self.meter = AudioMeterSnapshot {
                    microphone_basis_points: 0,
                    system_audio_basis_points: 0,
                    camera_active: false,
                };
                self.announcement = if outcome.system_audio_included {
                    "Native display and system-audio recording started."
                } else if self.settings.system_audio_enabled {
                    "System audio was unavailable; verified screen-only recording started."
                } else {
                    "Native display recording started."
                }
                .into();
                Ok(events)
            }
            IpcCommand::RecorderStop { intent_id } => {
                let recording = self
                    .native_recording
                    .clone()
                    .ok_or_else(|| ExecutionFailure::conflict("No native recording is active."))?;
                self.preflight_transition(intent_id, IntentKind::RecorderStop)?;
                let outcome = backend
                    .stop_recording(&NativeRecordingControlRequest {
                        recording_token: recording.recording_token.clone(),
                    })
                    .map_err(ExecutionFailure::native_backend)?;
                match outcome {
                    NativeRecordingStopOutcome::Sealed(artifact) => {
                        let Ok(authority) = self.validate_native_artifact(&recording, artifact)
                        else {
                            return self.commit_native_recording_failure(
                                intent_id,
                                IntentKind::RecorderStop,
                                SafeFailureCode::Internal,
                                false,
                            );
                        };
                        let events = self.transition(
                            intent_id,
                            IntentKind::RecorderStop,
                            vec![BackendEvent::RecorderStopped {
                                intent_id: intent_id.clone(),
                                recoverable: false,
                            }],
                        )?;
                        self.clear_native_recording_session();
                        self.native_artifact = Some(authority);
                        self.announcement =
                            "Native recording stopped and its artifact was sealed.".into();
                        Ok(events)
                    }
                    NativeRecordingStopOutcome::Failed(failure) => {
                        let (code, retryable) =
                            native_terminal_failure_state(&failure, &recording.recording_token);
                        self.commit_native_recording_failure(
                            intent_id,
                            IntentKind::RecorderStop,
                            code,
                            retryable,
                        )
                    }
                }
            }
            IpcCommand::RecorderCancel { intent_id } => {
                let recording = self
                    .native_recording
                    .clone()
                    .ok_or_else(|| ExecutionFailure::conflict("No native recording is active."))?;
                self.preflight_transition(intent_id, IntentKind::RecorderCancel)?;
                let outcome = backend
                    .cancel_recording(&NativeRecordingControlRequest {
                        recording_token: recording.recording_token.clone(),
                    })
                    .map_err(ExecutionFailure::native_backend)?;
                match outcome {
                    NativeRecordingCancelOutcome::Cancelled { recording_token }
                        if recording_token == recording.recording_token
                            && valid_opaque_id(&recording_token) =>
                    {
                        let events = self.transition(
                            intent_id,
                            IntentKind::RecorderCancel,
                            vec![BackendEvent::RecorderCancelled {
                                intent_id: intent_id.clone(),
                            }],
                        )?;
                        self.clear_native_recording_session();
                        self.native_artifact = None;
                        self.announcement = "Native recording cancelled.".into();
                        Ok(events)
                    }
                    NativeRecordingCancelOutcome::Cancelled { .. } => self
                        .commit_native_recording_failure(
                            intent_id,
                            IntentKind::RecorderCancel,
                            SafeFailureCode::Internal,
                            false,
                        ),
                    NativeRecordingCancelOutcome::Failed(failure) => {
                        let (code, retryable) =
                            native_terminal_failure_state(&failure, &recording.recording_token);
                        self.commit_native_recording_failure(
                            intent_id,
                            IntentKind::RecorderCancel,
                            code,
                            retryable,
                        )
                    }
                }
            }
            IpcCommand::ExportStart {
                project_revision,
                profile: ExportProfile::EditableWebm,
                ..
            } => {
                let output_path = validated_path.ok_or_else(ExecutionFailure::internal)?;
                let artifact = self.native_artifact.clone().ok_or_else(|| {
                    ExecutionFailure::conflict(
                        "Stop and seal a native recording before exporting it.",
                    )
                })?;
                let expected_output = artifact.export_path.as_ref().ok_or_else(|| {
                    ExecutionFailure::conflict(
                        "This recording has no approved editable WebM destination.",
                    )
                })?;
                if artifact.summary.artifact_revision != *project_revision
                    || output_path.as_path() != expected_output.as_path()
                {
                    return Err(ExecutionFailure::conflict(
                        "The recording artifact or export destination changed. Refresh and retry.",
                    ));
                }
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.preflight_transition(
                    &intent_id,
                    IntentKind::CaptureExportStart {
                        artifact_revision: *project_revision,
                    },
                )?;
                let outcome = backend
                    .export_editable_webm(&NativeEditableWebmExportRequest {
                        artifact_token: artifact.summary.artifact_token.clone(),
                        artifact_revision: artifact.summary.artifact_revision,
                        source_media_path: artifact.media_path,
                        output_path: output_path.clone(),
                    })
                    .map_err(ExecutionFailure::native_backend)?;
                if outcome.artifact_token != artifact.summary.artifact_token
                    || outcome.artifact_revision != artifact.summary.artifact_revision
                    || outcome.bytes_written == 0
                {
                    return Err(ExecutionFailure::invalid_backend_response());
                }
                let events = self.transition(
                    &intent_id,
                    IntentKind::CaptureExportStart {
                        artifact_revision: *project_revision,
                    },
                    vec![
                        BackendEvent::ExportStarted {
                            intent_id: intent_id.clone(),
                            project_revision: *project_revision,
                        },
                        BackendEvent::ExportProgress {
                            progress_basis_points: 10_000,
                        },
                        BackendEvent::ExportCompleted,
                    ],
                )?;
                self.announcement = "Editable WebM export completed.".into();
                Ok(events)
            }
            IpcCommand::SettingsApply {
                expected_revision,
                mode,
                frame_rate,
                microphone_enabled,
                system_audio_enabled,
                camera_enabled,
                reduced_motion,
            } => {
                if *microphone_enabled || *camera_enabled {
                    return Err(ExecutionFailure::unavailable());
                }
                if self.settings.revision != *expected_revision {
                    return Err(ExecutionFailure::conflict(
                        "Settings changed in another window. Refresh and retry.",
                    ));
                }
                self.settings = DesktopSettingsSnapshot {
                    revision: expected_revision.saturating_add(1),
                    mode: *mode,
                    frame_rate: *frame_rate,
                    microphone_enabled: false,
                    system_audio_enabled: *system_audio_enabled,
                    camera_enabled: false,
                    reduced_motion: *reduced_motion,
                };
                self.recorder_configuration.mode = *mode;
                self.announcement = "Native system-audio preference saved.".into();
                Ok(Vec::new())
            }
            _ => Err(ExecutionFailure::unavailable()),
        }
    }

    fn execute(
        &mut self,
        owner: WindowRole,
        command: &IpcCommand,
    ) -> Result<Vec<BackendEvent>, ExecutionFailure> {
        match command {
            IpcCommand::WindowOpen { role } => {
                if !self.contexts.iter().any(|context| context.role == *role) {
                    return Err(ExecutionFailure::forbidden());
                }
                self.lifecycle.main_visible = true;
                self.announcement = "Desktop window opened from backend state.".into();
                Ok(Vec::new())
            }
            IpcCommand::RecorderPrepare => {
                self.require_fake_execution()?;
                self.permission = PermissionState::Granted;
                self.announcement = "Capture permissions confirmed.".into();
                Ok(Vec::new())
            }
            IpcCommand::RecorderPoll => Err(ExecutionFailure::unavailable()),
            IpcCommand::RecorderStart { intent_id } => {
                if self.adapter != DesktopAdapterKind::DeterministicFake {
                    return self.transition(
                        intent_id,
                        IntentKind::RecorderStart,
                        vec![BackendEvent::RecorderFailed {
                            code: SafeFailureCode::BackendUnavailable,
                            retryable: true,
                        }],
                    );
                }
                if self.permission != PermissionState::Granted
                    || self.selected_sources.target.is_none()
                {
                    return Err(ExecutionFailure::invalid(
                        "Choose a capture target and confirm permissions before recording.",
                    ));
                }
                self.meter = AudioMeterSnapshot {
                    microphone_basis_points: if self.settings.microphone_enabled {
                        4_200
                    } else {
                        0
                    },
                    system_audio_basis_points: if self.settings.system_audio_enabled {
                        3_100
                    } else {
                        0
                    },
                    camera_active: self.settings.camera_enabled,
                };
                self.lifecycle.overlay_visible = true;
                self.announcement = "Recording started after backend confirmation.".into();
                self.transition(
                    intent_id,
                    IntentKind::RecorderStart,
                    vec![
                        BackendEvent::RecorderPreparing {
                            intent_id: intent_id.clone(),
                        },
                        BackendEvent::RecorderStarted,
                    ],
                )
            }
            IpcCommand::RecorderPause { intent_id } => {
                self.announcement = "Recording paused after backend confirmation.".into();
                self.transition(
                    intent_id,
                    IntentKind::RecorderPause,
                    vec![BackendEvent::RecorderPaused {
                        intent_id: intent_id.clone(),
                    }],
                )
            }
            IpcCommand::RecorderResume { intent_id } => {
                self.announcement = "Recording resumed after backend confirmation.".into();
                self.transition(
                    intent_id,
                    IntentKind::RecorderResume,
                    vec![BackendEvent::RecorderResumed {
                        intent_id: intent_id.clone(),
                    }],
                )
            }
            IpcCommand::RecorderStop { intent_id } => {
                self.lifecycle.overlay_visible = false;
                self.meter = AudioMeterSnapshot {
                    microphone_basis_points: 0,
                    system_audio_basis_points: 0,
                    camera_active: false,
                };
                self.announcement = "Recording stopped and project is ready.".into();
                self.transition(
                    intent_id,
                    IntentKind::RecorderStop,
                    vec![BackendEvent::RecorderStopped {
                        intent_id: intent_id.clone(),
                        recoverable: false,
                    }],
                )
            }
            IpcCommand::RecorderCancel { intent_id } => {
                self.lifecycle.overlay_visible = false;
                self.meter = AudioMeterSnapshot {
                    microphone_basis_points: 0,
                    system_audio_basis_points: 0,
                    camera_active: false,
                };
                self.announcement = "Recording cancelled after backend confirmation.".into();
                self.transition(
                    intent_id,
                    IntentKind::RecorderCancel,
                    vec![BackendEvent::RecorderCancelled {
                        intent_id: intent_id.clone(),
                    }],
                )
            }
            IpcCommand::DeviceEnumerate { .. } => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                if self.adapter == DesktopAdapterKind::DeterministicFake {
                    self.permission = PermissionState::Granted;
                    self.announcement = "Fake capture devices enumerated.".into();
                    self.transition(
                        &intent_id,
                        IntentKind::DevicesRefresh,
                        vec![
                            BackendEvent::DevicesEnumerating {
                                intent_id: intent_id.clone(),
                            },
                            BackendEvent::DevicesReady {
                                counts: DeviceCounts {
                                    displays: 2,
                                    microphones: 2,
                                    system_audio_sources: 1,
                                    cameras: 1,
                                },
                            },
                        ],
                    )
                } else {
                    self.permission = PermissionState::NotDetermined;
                    Err(ExecutionFailure::unavailable())
                }
            }
            IpcCommand::DeviceSelect {
                class,
                device_token: _,
            } => {
                self.require_fake_execution()?;
                let intent_id = current_intent_id(owner, self.operation_revision);
                match class {
                    DeviceClass::Display => self.selected_sources.display_selected = true,
                    DeviceClass::Microphone => self.selected_sources.microphone_selected = true,
                    DeviceClass::SystemAudio => {
                        self.selected_sources.system_audio_selected = true;
                    }
                    DeviceClass::Camera => self.selected_sources.camera_selected = true,
                }
                self.announcement = "Capture source selected.".into();
                self.transition(
                    &intent_id,
                    IntentKind::DeviceSelect,
                    vec![BackendEvent::DeviceSelected {
                        intent_id: intent_id.clone(),
                    }],
                )
            }
            IpcCommand::RecoveryScan => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.announcement = if self.recovery_projects == 0 {
                    "No recoverable projects were found."
                } else {
                    "Recoverable projects found."
                }
                .into();
                self.transition(
                    &intent_id,
                    IntentKind::RecoveryScan,
                    vec![
                        BackendEvent::RecoveryScanning {
                            intent_id: intent_id.clone(),
                        },
                        BackendEvent::RecoveryAvailable {
                            projects: self.recovery_projects,
                        },
                    ],
                )
            }
            IpcCommand::RecoveryInspect { .. } => {
                self.announcement = "Recovery project inspected without modifying it.".into();
                Ok(Vec::new())
            }
            IpcCommand::RecoveryOpen { .. } => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.announcement = "Recovered project opened from a preserved copy.".into();
                self.transition(
                    &intent_id,
                    IntentKind::RecoveryOpen,
                    vec![
                        BackendEvent::RecoveryOpening {
                            intent_id: intent_id.clone(),
                        },
                        BackendEvent::RecoveryOpened,
                    ],
                )
            }
            IpcCommand::RecoveryDiscard { .. } => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.recovery_projects = self.recovery_projects.saturating_sub(1);
                self.announcement = "Recovery copy discarded after explicit confirmation.".into();
                self.transition(
                    &intent_id,
                    IntentKind::RecoveryDiscard,
                    vec![BackendEvent::RecoveryDiscarded {
                        intent_id: intent_id.clone(),
                        remaining: self.recovery_projects,
                    }],
                )
            }
            IpcCommand::EditorOpen { .. } => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.announcement = "Project loaded from backend revision 1.".into();
                self.transition(
                    &intent_id,
                    IntentKind::EditorOpen,
                    vec![
                        BackendEvent::EditorLoading {
                            intent_id: intent_id.clone(),
                        },
                        BackendEvent::EditorLoaded {
                            revision: 1,
                            duration_ms: 90_000,
                        },
                    ],
                )
            }
            IpcCommand::EditorApply { base_revision, .. } => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.announcement = "Edit applied to a new backend revision.".into();
                self.transition(
                    &intent_id,
                    IntentKind::EditorApply {
                        base_revision: *base_revision,
                    },
                    vec![BackendEvent::EditorApplied {
                        intent_id: intent_id.clone(),
                        revision: base_revision.saturating_add(1),
                    }],
                )
            }
            IpcCommand::EditorSave { expected_revision } => {
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.announcement = "Project revision saved.".into();
                self.transition(
                    &intent_id,
                    IntentKind::EditorSave {
                        expected_revision: *expected_revision,
                    },
                    vec![BackendEvent::EditorSaved {
                        intent_id: intent_id.clone(),
                        revision: *expected_revision,
                    }],
                )
            }
            IpcCommand::ExportStart {
                project_revision, ..
            } => {
                self.require_fake_execution()?;
                let intent_id = current_intent_id(owner, self.operation_revision);
                self.announcement = "Export is 25 percent complete.".into();
                self.transition(
                    &intent_id,
                    IntentKind::ExportStart {
                        project_revision: *project_revision,
                    },
                    vec![
                        BackendEvent::ExportStarted {
                            intent_id: intent_id.clone(),
                            project_revision: *project_revision,
                        },
                        BackendEvent::ExportProgress {
                            progress_basis_points: 2_500,
                        },
                    ],
                )
            }
            IpcCommand::ExportCancel { intent_id } => {
                self.announcement = "Export cancelled.".into();
                self.transition(
                    intent_id,
                    IntentKind::ExportCancel,
                    vec![
                        BackendEvent::ExportCancelling {
                            intent_id: intent_id.clone(),
                        },
                        BackendEvent::ExportCancelled,
                    ],
                )
            }
            IpcCommand::UploadStart { upload_intent, .. } => {
                self.require_fake_execution()?;
                self.announcement = "Upload started with verified multipart progress.".into();
                self.transition(
                    upload_intent,
                    IntentKind::UploadStart,
                    vec![
                        BackendEvent::UploadStarted {
                            intent_id: upload_intent.clone(),
                            total_parts: 4,
                        },
                        BackendEvent::UploadProgress {
                            verified_parts: 1,
                            total_parts: 4,
                        },
                    ],
                )
            }
            IpcCommand::UploadPause { intent_id } => {
                self.announcement = "Upload paused after verified parts were journaled.".into();
                self.transition(
                    intent_id,
                    IntentKind::UploadPause,
                    vec![BackendEvent::UploadPaused {
                        intent_id: intent_id.clone(),
                    }],
                )
            }
            IpcCommand::UploadResume { intent_id } => {
                self.announcement = "Upload resumed from verified parts.".into();
                self.transition(
                    intent_id,
                    IntentKind::UploadResume,
                    vec![BackendEvent::UploadResumed {
                        intent_id: intent_id.clone(),
                    }],
                )
            }
            IpcCommand::UploadCancel { intent_id } => {
                self.announcement = "Upload cancelled and pending parts were released.".into();
                self.transition(
                    intent_id,
                    IntentKind::UploadCancel,
                    vec![BackendEvent::UploadCancelled {
                        intent_id: intent_id.clone(),
                    }],
                )
            }
            IpcCommand::RecorderConfigure {
                mode,
                countdown_seconds,
                exclude_frame_windows,
            } => {
                if !matches!(
                    self.workflow.recorder(),
                    RecorderState::Idle | RecorderState::Ready | RecorderState::Failed { .. }
                ) {
                    return Err(ExecutionFailure::conflict(
                        "Recorder settings cannot change during capture.",
                    ));
                }
                self.recorder_configuration = RecorderConfiguration {
                    mode: *mode,
                    countdown_seconds: *countdown_seconds,
                    exclude_frame_windows: *exclude_frame_windows,
                };
                self.lifecycle.frame_windows_excluded = *exclude_frame_windows;
                self.announcement = "Recorder configuration confirmed.".into();
                Ok(Vec::new())
            }
            IpcCommand::CaptureTargetSelect { kind, .. } => {
                self.require_fake_execution()?;
                self.selected_sources.target = Some(*kind);
                self.lifecycle.target_picker_visible = false;
                self.announcement = "Capture target selected by opaque token.".into();
                Ok(Vec::new())
            }
            IpcCommand::SettingsApply {
                expected_revision,
                mode,
                frame_rate,
                microphone_enabled,
                system_audio_enabled,
                camera_enabled,
                reduced_motion,
            } => {
                if self.settings.revision != *expected_revision {
                    return Err(ExecutionFailure::conflict(
                        "Settings changed in another window. Refresh and retry.",
                    ));
                }
                self.settings = DesktopSettingsSnapshot {
                    revision: expected_revision.saturating_add(1),
                    mode: *mode,
                    frame_rate: *frame_rate,
                    microphone_enabled: *microphone_enabled,
                    system_audio_enabled: *system_audio_enabled,
                    camera_enabled: *camera_enabled,
                    reduced_motion: *reduced_motion,
                };
                self.recorder_configuration.mode = *mode;
                self.announcement = "Desktop settings saved at a new revision.".into();
                Ok(Vec::new())
            }
            IpcCommand::PresetApply {
                preset_token,
                expected_settings_revision,
            } => {
                if self.settings.revision != *expected_settings_revision {
                    return Err(ExecutionFailure::conflict(
                        "Preset was based on stale settings.",
                    ));
                }
                match preset_token.as_str() {
                    "preset-balanced" => {
                        self.settings.frame_rate = 30;
                        self.settings.camera_enabled = false;
                    }
                    "preset-quality" => {
                        self.settings.frame_rate = 60;
                        self.settings.camera_enabled = true;
                    }
                    _ => return Err(ExecutionFailure::invalid("Preset is not approved.")),
                }
                self.settings.revision = self.settings.revision.saturating_add(1);
                self.announcement = "Approved recorder preset applied.".into();
                Ok(Vec::new())
            }
            IpcCommand::Lifecycle { action } => {
                self.require_fake_execution()?;
                match action {
                    LifecycleAction::RegisterHotkeys => self.lifecycle.hotkeys_registered = true,
                    LifecycleAction::ShowMainWindow | LifecycleAction::ReopenWindow => {
                        self.lifecycle.main_visible = true;
                    }
                    LifecycleAction::HideMainWindow | LifecycleAction::CloseWindow => {
                        self.lifecycle.main_visible = false;
                    }
                    LifecycleAction::ShowOverlay => self.lifecycle.overlay_visible = true,
                    LifecycleAction::HideOverlay => self.lifecycle.overlay_visible = false,
                    LifecycleAction::ShowTargetPicker => {
                        self.lifecycle.target_picker_visible = true;
                    }
                    LifecycleAction::HideTargetPicker => {
                        self.lifecycle.target_picker_visible = false;
                    }
                }
                self.announcement = "Desktop lifecycle transition confirmed.".into();
                Ok(Vec::new())
            }
            IpcCommand::Update {
                action,
                expected_revision,
            } => {
                self.require_fake_execution()?;
                let current_revision = match self.update {
                    UpdateState::Current { revision }
                    | UpdateState::Available { revision }
                    | UpdateState::ReadyToRelaunch { revision } => revision,
                };
                if current_revision != *expected_revision {
                    return Err(ExecutionFailure::conflict(
                        "Update state changed. Refresh and retry.",
                    ));
                }
                self.update = match (*action, self.update) {
                    (UpdateAction::Check, UpdateState::Current { revision }) => {
                        UpdateState::Available { revision }
                    }
                    (UpdateAction::Install, UpdateState::Available { revision }) => {
                        UpdateState::ReadyToRelaunch { revision }
                    }
                    (UpdateAction::Relaunch, UpdateState::ReadyToRelaunch { revision }) => {
                        UpdateState::Current {
                            revision: revision.saturating_add(1),
                        }
                    }
                    _ => {
                        return Err(ExecutionFailure::conflict(
                            "Update action is not valid in the current state.",
                        ));
                    }
                };
                self.announcement = "Update lifecycle confirmed by the backend.".into();
                Ok(Vec::new())
            }
        }
    }

    fn transition(
        &mut self,
        intent_id: &str,
        intent_kind: IntentKind,
        events: Vec<BackendEvent>,
    ) -> Result<Vec<BackendEvent>, ExecutionFailure> {
        let mut next = self.workflow.clone();
        let intent = UiIntent::new(intent_id, intent_kind).map_err(ExecutionFailure::workflow)?;
        next.request(intent).map_err(ExecutionFailure::workflow)?;
        let mut sequence = self.backend_sequence;
        for event in &events {
            sequence = sequence
                .checked_add(1)
                .ok_or_else(ExecutionFailure::internal)?;
            next.apply_backend(BackendEventEnvelope {
                protocol_version: WORKFLOW_PROTOCOL_VERSION,
                session_id: self.backend_session.clone(),
                sequence,
                event: event.clone(),
            })
            .map_err(ExecutionFailure::workflow)?;
        }
        self.workflow = next;
        self.backend_sequence = sequence;
        Ok(events)
    }

    fn preflight_transition(
        &self,
        intent_id: &str,
        intent_kind: IntentKind,
    ) -> Result<(), ExecutionFailure> {
        let mut next = self.workflow.clone();
        let intent = UiIntent::new(intent_id, intent_kind).map_err(ExecutionFailure::workflow)?;
        next.request(intent).map_err(ExecutionFailure::workflow)
    }

    fn validate_native_artifact(
        &self,
        recording: &NativeRecordingAuthority,
        artifact: NativeCaptureArtifact,
    ) -> Result<NativeArtifactAuthority, ExecutionFailure> {
        if artifact.recording_token != recording.recording_token
            || !valid_opaque_id(&artifact.artifact_token)
            || artifact.artifact_revision == 0
            || artifact.duration_ms == 0
            || artifact.bytes_written == 0
        {
            return Err(ExecutionFailure::invalid_backend_response());
        }
        let media_path = self
            .native_media_paths
            .validate(&artifact.media_path, PathUse::MediaRead)
            .map_err(|_| ExecutionFailure::invalid_backend_response())?;
        let export_path = artifact
            .editable_webm_output_path
            .as_deref()
            .map(|path| {
                let validated = self
                    .native_export_paths
                    .validate(path, PathUse::ExportWrite)
                    .map_err(|_| ExecutionFailure::invalid_backend_response())?;
                if validated
                    .as_path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_none_or(|extension| !extension.eq_ignore_ascii_case("webm"))
                {
                    return Err(ExecutionFailure::invalid_backend_response());
                }
                Ok(validated)
            })
            .transpose()?;
        if export_path
            .as_ref()
            .is_some_and(|output| output.as_path() == media_path.as_path())
        {
            return Err(ExecutionFailure::invalid_backend_response());
        }
        let summary = CaptureArtifactSummary {
            schema_version: CAPTURE_ARTIFACT_SUMMARY_VERSION,
            artifact_token: artifact.artifact_token,
            artifact_revision: artifact.artifact_revision,
            duration_ms: artifact.duration_ms,
            bytes_written: artifact.bytes_written,
            editable_webm_output_path: export_path
                .as_ref()
                .map(|path| path.as_path().to_string_lossy().into_owned()),
        };
        Ok(NativeArtifactAuthority {
            summary,
            media_path,
            export_path,
        })
    }

    fn commit_native_recording_failure(
        &mut self,
        intent_id: &str,
        intent_kind: IntentKind,
        code: SafeFailureCode,
        retryable: bool,
    ) -> Result<Vec<BackendEvent>, ExecutionFailure> {
        let events = self.transition(
            intent_id,
            intent_kind,
            vec![BackendEvent::RecorderFailed { code, retryable }],
        )?;
        self.clear_native_recording_session();
        self.native_artifact = None;
        self.announcement = "Native recording failed and its capture session was retired.".into();
        Ok(events)
    }

    fn clear_native_recording_session(&mut self) {
        self.native_recording = None;
        self.lifecycle.overlay_visible = false;
        self.meter = AudioMeterSnapshot {
            microphone_basis_points: 0,
            system_audio_basis_points: 0,
            camera_active: false,
        };
        self.capture_targets.targets.clear();
        self.selected_capture_target = None;
        self.selected_sources.target = None;
        self.selected_sources.display_selected = false;
    }

    fn apply_unsolicited(&mut self, events: &[BackendEvent]) -> Result<(), DesktopRuntimeError> {
        let mut next = self.workflow.clone();
        let mut sequence = self.backend_sequence;
        for event in events {
            sequence = sequence
                .checked_add(1)
                .ok_or(DesktopRuntimeError::SequenceOverflow)?;
            next.apply_backend(BackendEventEnvelope {
                protocol_version: WORKFLOW_PROTOCOL_VERSION,
                session_id: self.backend_session.clone(),
                sequence,
                event: event.clone(),
            })?;
        }
        self.workflow = next;
        self.backend_sequence = sequence;
        Ok(())
    }

    fn owner_for(&self, request: &RequestEnvelope) -> Result<WindowRole, DesktopRuntimeError> {
        self.contexts
            .iter()
            .find(|context| {
                context.window_id == request.window_id && context.session_id == request.session_id
            })
            .map(|context| context.role)
            .ok_or(DesktopRuntimeError::ScopeInvariant)
    }

    fn wrap_event(
        &mut self,
        owner: WindowRole,
        event: DesktopRuntimeEvent,
    ) -> Result<DesktopEventEnvelope, DesktopRuntimeError> {
        self.event_sequence = self
            .event_sequence
            .checked_add(1)
            .ok_or(DesktopRuntimeError::SequenceOverflow)?;
        Ok(DesktopEventEnvelope {
            protocol_version: DESKTOP_RUNTIME_VERSION,
            event_sequence: self.event_sequence,
            owner,
            event,
        })
    }

    fn require_fake(&self) -> Result<(), DesktopRuntimeError> {
        if self.adapter == DesktopAdapterKind::DeterministicFake {
            Ok(())
        } else {
            Err(DesktopRuntimeError::FakeAdapterRequired)
        }
    }

    fn require_fake_execution(&self) -> Result<(), ExecutionFailure> {
        if self.adapter == DesktopAdapterKind::DeterministicFake {
            Ok(())
        } else {
            Err(ExecutionFailure::unavailable())
        }
    }
}

#[must_use]
pub const fn instant_progress_announcement(progress: InstantUiProgressV1) -> &'static str {
    if let Some(error) = progress.error {
        return instant_error_message(error);
    }
    match progress.phase {
        InstantUiPhaseV1::Recording => "Instant recording is being captured locally.",
        InstantUiPhaseV1::LocallyRecoverable => {
            "Instant recording is safe locally and waiting for the network."
        }
        InstantUiPhaseV1::Uploading => "Instant recording is uploading.",
        InstantUiPhaseV1::Finalizing => "Instant recording is finalizing for sharing.",
        InstantUiPhaseV1::ShareReady => "Instant recording is ready to share.",
        InstantUiPhaseV1::Cancelled => "Instant recording was cancelled.",
        InstantUiPhaseV1::RecoveryRequired => "Instant recording needs recovery.",
    }
}

#[must_use]
pub const fn instant_error_message(error: InstantUiErrorCodeV1) -> &'static str {
    match error {
        InstantUiErrorCodeV1::LocalStorageFull => {
            "Local storage is full. Free space before continuing."
        }
        InstantUiErrorCodeV1::LocalStorageUnavailable => "Local recording storage is unavailable.",
        InstantUiErrorCodeV1::NetworkOffline => {
            "The network is offline. Your recording remains stored locally."
        }
        InstantUiErrorCodeV1::UploadDelayed => {
            "Upload is delayed. Frame will retry without duplicating verified data."
        }
        InstantUiErrorCodeV1::UploadExpired => {
            "Upload authorization expired. Retry to request fresh authorization."
        }
        InstantUiErrorCodeV1::FinalizeDelayed => {
            "Sharing is delayed. Retry is safe and will reuse the same native request."
        }
        InstantUiErrorCodeV1::RecordingRecoveryRequired => {
            "The recording is safe locally and needs recovery."
        }
        InstantUiErrorCodeV1::RecordingCancelled => "The recording was cancelled.",
        InstantUiErrorCodeV1::RecordingFailed => {
            "The recording could not be completed. Open recovery for available media."
        }
    }
}

fn paths_for(role: WindowRole, roots: &DesktopRoots) -> Result<PathPolicy, IpcError> {
    let read = RootAccess {
        read: true,
        write: false,
        delete: false,
    };
    let recovery = RootAccess {
        read: true,
        write: false,
        delete: true,
    };
    let write = RootAccess {
        read: true,
        write: true,
        delete: false,
    };
    match role {
        WindowRole::Recorder => PathPolicy::empty().allow_root(&roots.media, read),
        WindowRole::Recovery => PathPolicy::empty().allow_root(&roots.projects, recovery),
        WindowRole::Editor => PathPolicy::empty()
            .allow_root(&roots.projects, read)?
            .allow_root(&roots.exports, write)?
            .allow_root(&roots.media, read),
        WindowRole::Export => PathPolicy::empty().allow_root(&roots.exports, write),
        WindowRole::Main | WindowRole::Settings | WindowRole::Overlay => Ok(PathPolicy::empty()),
    }
}

fn role_label(role: WindowRole) -> &'static str {
    match role {
        WindowRole::Main => "main",
        WindowRole::Recorder => "recorder",
        WindowRole::Recovery => "recovery",
        WindowRole::Editor => "editor",
        WindowRole::Export => "export",
        WindowRole::Settings => "settings",
        WindowRole::Overlay => "overlay",
    }
}

fn current_intent_id(owner: WindowRole, revision: u64) -> String {
    format!("runtime-{}-{revision:016x}", role_label(owner))
}

fn validate_selection_outcome(
    outcome: &NativeTargetSelectionOutcome,
    catalog_generation: u64,
    target_token: &str,
) -> Result<(), ExecutionFailure> {
    if outcome.catalog_generation != catalog_generation
        || outcome.target_token != target_token
        || !valid_opaque_id(&outcome.target_token)
    {
        Err(ExecutionFailure::invalid_backend_response())
    } else {
        Ok(())
    }
}

fn validate_start_outcome(
    outcome: &NativeRecordingStartOutcome,
    catalog_generation: u64,
    target_token: &str,
    system_audio_requested: bool,
) -> Result<(), ExecutionFailure> {
    if outcome.catalog_generation != catalog_generation
        || outcome.target_token != target_token
        || !valid_opaque_id(&outcome.target_token)
        || !valid_opaque_id(&outcome.recording_token)
        || (outcome.system_audio_included && !system_audio_requested)
    {
        Err(ExecutionFailure::invalid_backend_response())
    } else {
        Ok(())
    }
}

fn native_terminal_failure_state(
    failure: &NativeRecordingTerminalFailure,
    recording_token: &str,
) -> (SafeFailureCode, bool) {
    if failure.recording_token != recording_token
        || !valid_opaque_id(&failure.recording_token)
        || !failure.teardown_confirmed
    {
        return (SafeFailureCode::BackendUnavailable, false);
    }
    match failure.error {
        NativeDesktopBackendError::PermissionDenied => (SafeFailureCode::PermissionDenied, true),
        NativeDesktopBackendError::StaleCatalog | NativeDesktopBackendError::TargetUnavailable => {
            (SafeFailureCode::DeviceLost, true)
        }
        NativeDesktopBackendError::Unavailable => (SafeFailureCode::BackendUnavailable, false),
        NativeDesktopBackendError::Filesystem => (SafeFailureCode::DiskFull, true),
        NativeDesktopBackendError::Cancelled => (SafeFailureCode::Cancelled, false),
        NativeDesktopBackendError::Busy | NativeDesktopBackendError::Internal => {
            (SafeFailureCode::Internal, false)
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ExecutionFailure {
    code: PublicErrorCode,
    retryable: bool,
    announcement: &'static str,
}

impl ExecutionFailure {
    const fn invalid(announcement: &'static str) -> Self {
        Self {
            code: PublicErrorCode::InvalidRequest,
            retryable: false,
            announcement,
        }
    }

    const fn forbidden() -> Self {
        Self {
            code: PublicErrorCode::Forbidden,
            retryable: false,
            announcement: "This window does not own the requested operation.",
        }
    }

    const fn conflict(announcement: &'static str) -> Self {
        Self {
            code: PublicErrorCode::Conflict,
            retryable: true,
            announcement,
        }
    }

    const fn unavailable() -> Self {
        Self {
            code: PublicErrorCode::Unavailable,
            retryable: true,
            announcement: "The native adapter is unavailable. No operation was started.",
        }
    }

    const fn invalid_backend_response() -> Self {
        Self {
            code: PublicErrorCode::Internal,
            retryable: false,
            announcement: "The native backend returned an invalid confirmation.",
        }
    }

    const fn internal() -> Self {
        Self {
            code: PublicErrorCode::Internal,
            retryable: false,
            announcement: "The desktop operation could not be completed.",
        }
    }

    fn workflow(error: WorkflowError) -> Self {
        match error {
            WorkflowError::Busy(_)
            | WorkflowError::InvalidIntentForState(_)
            | WorkflowError::IntentMismatch
            | WorkflowError::NoPendingIntent => {
                Self::conflict("Backend state changed before this operation. Refresh and retry.")
            }
            _ => Self::internal(),
        }
    }

    const fn native_backend(error: NativeDesktopBackendError) -> Self {
        match error {
            NativeDesktopBackendError::Unavailable => Self::unavailable(),
            NativeDesktopBackendError::Busy => Self {
                code: PublicErrorCode::Busy,
                retryable: true,
                announcement: "The native capture backend is busy.",
            },
            NativeDesktopBackendError::PermissionDenied => Self {
                code: PublicErrorCode::Forbidden,
                retryable: true,
                announcement: "macOS screen recording permission is required.",
            },
            NativeDesktopBackendError::StaleCatalog
            | NativeDesktopBackendError::TargetUnavailable => {
                Self::conflict("The display catalog changed. Refresh displays and select again.")
            }
            NativeDesktopBackendError::Cancelled => Self {
                code: PublicErrorCode::Cancelled,
                retryable: false,
                announcement: "The native operation was cancelled.",
            },
            NativeDesktopBackendError::Filesystem => Self {
                code: PublicErrorCode::Internal,
                retryable: true,
                announcement: "The native recording file could not be completed.",
            },
            NativeDesktopBackendError::Internal => Self::internal(),
        }
    }
}

#[derive(Debug, Error)]
pub enum DesktopRuntimeError {
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
    #[error("desktop operation revision overflowed")]
    RevisionOverflow,
    #[error("desktop event sequence overflowed")]
    SequenceOverflow,
    #[error("accepted request lost its window scope")]
    ScopeInvariant,
    #[error("the deterministic fake adapter is required")]
    FakeAdapterRequired,
    #[error("the Instant progress contract is invalid")]
    InvalidInstantProgress,
    #[error("the Instant finalize handle does not match native authority")]
    InstantFinalizeAuthorityMismatch,
}

impl DesktopRuntimeError {
    #[must_use]
    pub const fn public_code(&self) -> PublicErrorCode {
        match self {
            Self::Ipc(error) => error.public_code(),
            Self::FakeAdapterRequired => PublicErrorCode::Unavailable,
            Self::InstantFinalizeAuthorityMismatch => PublicErrorCode::Conflict,
            Self::Workflow(_)
            | Self::RevisionOverflow
            | Self::SequenceOverflow
            | Self::ScopeInvariant
            | Self::InvalidInstantProgress => PublicErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::ipc::{EditorMutation, ExportProfile, IPC_PROTOCOL_VERSION, RequestId};
    use crate::native_backend::NativeEditableWebmExportOutcome;

    #[derive(Debug)]
    struct TestNativeBackend {
        catalog: CaptureTargetCatalog,
        calls: Vec<&'static str>,
        permission: NativePermissionOutcome,
        select_generation_override: Option<u64>,
        start_error: Option<NativeDesktopBackendError>,
        poll_error: Option<NativeDesktopBackendError>,
        poll_failure: Option<NativeRecordingTerminalFailure>,
        poll_request_matches_recording: bool,
        stop_error: Option<NativeDesktopBackendError>,
        stop_failure: Option<NativeRecordingTerminalFailure>,
        stop_artifact: NativeCaptureArtifact,
        cancel_error: Option<NativeDesktopBackendError>,
        cancel_failure: Option<NativeRecordingTerminalFailure>,
        cancel_token_override: Option<String>,
        export_calls: usize,
    }

    impl TestNativeBackend {
        fn new() -> Self {
            Self {
                catalog: native_catalog(1, "display-token-1"),
                calls: Vec::new(),
                permission: NativePermissionOutcome::Granted,
                select_generation_override: None,
                start_error: None,
                poll_error: None,
                poll_failure: None,
                poll_request_matches_recording: true,
                stop_error: None,
                stop_failure: None,
                stop_artifact: NativeCaptureArtifact {
                    recording_token: "recording-token-1".into(),
                    artifact_token: "artifact-token-1".into(),
                    artifact_revision: 11,
                    duration_ms: 2_000,
                    bytes_written: 512_000,
                    media_path: absolute_test_path(&["frame", "media", "capture.webm"]),
                    editable_webm_output_path: Some(absolute_test_path(&[
                        "frame",
                        "exports",
                        "capture-editable.webm",
                    ])),
                },
                cancel_error: None,
                cancel_failure: None,
                cancel_token_override: None,
                export_calls: 0,
            }
        }

        fn call_count(&self, name: &str) -> usize {
            self.calls.iter().filter(|call| **call == name).count()
        }
    }

    impl NativeDesktopBackend for TestNativeBackend {
        fn prepare_display_capture(
            &mut self,
        ) -> Result<NativePermissionOutcome, NativeDesktopBackendError> {
            self.calls.push("prepare");
            Ok(self.permission)
        }

        fn enumerate_displays(
            &mut self,
        ) -> Result<CaptureTargetCatalog, NativeDesktopBackendError> {
            self.calls.push("enumerate");
            Ok(self.catalog.clone())
        }

        fn select_display(
            &mut self,
            request: &NativeTargetSelectionRequest,
        ) -> Result<NativeTargetSelectionOutcome, NativeDesktopBackendError> {
            self.calls.push("select");
            Ok(NativeTargetSelectionOutcome {
                catalog_generation: self
                    .select_generation_override
                    .unwrap_or(request.catalog_generation),
                target_token: request.target.token.clone(),
            })
        }

        fn start_display_recording(
            &mut self,
            request: &NativeCaptureStartRequest,
        ) -> Result<NativeRecordingStartOutcome, NativeDesktopBackendError> {
            self.calls.push("start");
            if let Some(error) = self.start_error {
                return Err(error);
            }
            Ok(NativeRecordingStartOutcome {
                catalog_generation: request.catalog_generation,
                target_token: request.target.token.clone(),
                recording_token: "recording-token-1".into(),
                system_audio_included: request.system_audio_enabled,
            })
        }

        fn stop_recording(
            &mut self,
            _request: &NativeRecordingControlRequest,
        ) -> Result<NativeRecordingStopOutcome, NativeDesktopBackendError> {
            self.calls.push("stop");
            if let Some(error) = self.stop_error {
                return Err(error);
            }
            Ok(match self.stop_failure.clone() {
                Some(failure) => NativeRecordingStopOutcome::Failed(failure),
                None => NativeRecordingStopOutcome::Sealed(self.stop_artifact.clone()),
            })
        }

        fn poll_recording_terminal_failure(
            &mut self,
            request: &NativeRecordingControlRequest,
        ) -> Result<Option<NativeRecordingTerminalFailure>, NativeDesktopBackendError> {
            self.calls.push("poll");
            self.poll_request_matches_recording &=
                request.recording_token == self.stop_artifact.recording_token;
            if let Some(error) = self.poll_error {
                return Err(error);
            }
            Ok(self.poll_failure.clone())
        }

        fn cancel_recording(
            &mut self,
            request: &NativeRecordingControlRequest,
        ) -> Result<NativeRecordingCancelOutcome, NativeDesktopBackendError> {
            self.calls.push("cancel");
            if let Some(error) = self.cancel_error {
                return Err(error);
            }
            Ok(match self.cancel_failure.clone() {
                Some(failure) => NativeRecordingCancelOutcome::Failed(failure),
                None => NativeRecordingCancelOutcome::Cancelled {
                    recording_token: self
                        .cancel_token_override
                        .clone()
                        .unwrap_or_else(|| request.recording_token.clone()),
                },
            })
        }

        fn export_editable_webm(
            &mut self,
            request: &NativeEditableWebmExportRequest,
        ) -> Result<NativeEditableWebmExportOutcome, NativeDesktopBackendError> {
            self.calls.push("export");
            self.export_calls += 1;
            Ok(NativeEditableWebmExportOutcome {
                artifact_token: request.artifact_token.clone(),
                artifact_revision: request.artifact_revision,
                bytes_written: 256_000,
            })
        }
    }

    fn absolute_test_path(components: &[&str]) -> String {
        #[cfg(windows)]
        let mut path = PathBuf::from(r"C:\");
        #[cfg(not(windows))]
        let mut path = PathBuf::from("/");
        path.extend(components);
        path.to_string_lossy().into_owned()
    }

    fn roots() -> DesktopRoots {
        DesktopRoots::new(
            absolute_test_path(&["frame", "projects"]),
            absolute_test_path(&["frame", "media"]),
            absolute_test_path(&["frame", "exports"]),
        )
    }

    fn native_catalog(generation: u64, token: &str) -> CaptureTargetCatalog {
        CaptureTargetCatalog {
            schema_version: crate::native_backend::CAPTURE_TARGET_CATALOG_VERSION,
            generation,
            targets: vec![CaptureTargetSummary {
                token: token.into(),
                kind: CaptureTargetKind::Display,
                ordinal: 1,
                width_pixels: 1_920,
                height_pixels: 1_080,
                scale_numerator: 2,
                scale_denominator: 1,
                rotation_degrees: 0,
            }],
        }
    }

    fn runtime() -> DesktopRuntime {
        DesktopRuntime::new(DesktopAdapterKind::DeterministicFake, roots(), "test-1")
            .expect("runtime")
    }

    fn native_runtime() -> DesktopRuntime {
        DesktopRuntime::new(DesktopAdapterKind::NativeMacOs, roots(), "native-test-1")
            .expect("native runtime")
    }

    fn context(runtime: &DesktopRuntime, role: WindowRole) -> DesktopWindowContext {
        runtime
            .bootstrap()
            .contexts
            .into_iter()
            .find(|context| context.role == role)
            .expect("context")
    }

    fn request(
        runtime: &DesktopRuntime,
        role: WindowRole,
        sequence: u64,
        id: &str,
        command: IpcCommand,
    ) -> RequestEnvelope {
        let context = context(runtime, role);
        RequestEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: RequestId::new(id).expect("request"),
            window_id: context.window_id,
            session_id: context.session_id,
            sequence,
            command,
        }
    }

    fn prepare_native_recording(runtime: &mut DesktopRuntime, backend: &mut TestNativeBackend) {
        for (sequence, id, command) in [
            (
                1,
                "native-enumerate",
                IpcCommand::DeviceEnumerate {
                    class: DeviceClass::Display,
                },
            ),
            (
                2,
                "native-select",
                IpcCommand::CaptureTargetSelect {
                    kind: CaptureTargetKind::Display,
                    target_token: "display-token-1".into(),
                },
            ),
            (3, "native-prepare", IpcCommand::RecorderPrepare),
        ] {
            let envelope = request(runtime, WindowRole::Recorder, sequence, id, command);
            let dispatch = runtime
                .dispatch_native(envelope, backend)
                .expect("native preparation command");
            ok(&dispatch);
        }
    }

    fn ok(dispatch: &DesktopDispatch) {
        assert!(matches!(
            dispatch.response.outcome,
            CommandOutcome::Ok { .. }
        ));
        assert!(
            dispatch
                .events
                .windows(2)
                .all(|events| events[0].event_sequence < events[1].event_sequence)
        );
    }

    fn finalize_handle(marker: char) -> InstantFinalizeHandle {
        serde_json::from_value(serde_json::Value::String(marker.to_string().repeat(64)))
            .expect("opaque finalize handle")
    }

    fn finalizing_progress() -> InstantUiProgressV1 {
        InstantUiProgressV1::new(InstantUiPhaseV1::Finalizing, None, false, None)
            .expect("finalizing progress")
    }

    fn bind_finalize(runtime: &mut DesktopRuntime, marker: char) -> InstantFinalizeHandle {
        let handle = finalize_handle(marker);
        runtime
            .bind_native_instant_finalize(InstantFinalizeRegistrationV1 {
                handle: handle.clone(),
                progress: finalizing_progress(),
            })
            .expect("bind native finalize");
        handle
    }

    #[test]
    fn deterministic_backend_drives_record_edit_export_and_upload_truth() {
        let mut runtime = runtime();
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                1,
                "devices",
                IpcCommand::DeviceEnumerate {
                    class: DeviceClass::Display,
                },
            ))
            .expect("devices"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                2,
                "target",
                IpcCommand::CaptureTargetSelect {
                    kind: CaptureTargetKind::Display,
                    target_token: "fake-display-1".into(),
                },
            ))
            .expect("target"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                3,
                "prepare",
                IpcCommand::RecorderPrepare,
            ))
            .expect("prepare"));
        let started = runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                4,
                "record-start",
                IpcCommand::RecorderStart {
                    intent_id: "record-start".into(),
                },
            ))
            .expect("start");
        ok(&started);
        assert_eq!(started.snapshot.recorder, RecorderState::Recording);
        assert!(started.snapshot.lifecycle.overlay_visible);

        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                5,
                "record-pause",
                IpcCommand::RecorderPause {
                    intent_id: "record-pause".into(),
                },
            ))
            .expect("pause"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                6,
                "record-resume",
                IpcCommand::RecorderResume {
                    intent_id: "record-resume".into(),
                },
            ))
            .expect("resume"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                7,
                "record-stop",
                IpcCommand::RecorderStop {
                    intent_id: "record-stop".into(),
                },
            ))
            .expect("stop"));

        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Editor,
                1,
                "editor-open",
                IpcCommand::EditorOpen {
                    project_path: absolute_test_path(&["frame", "projects", "demo.frame"]),
                },
            ))
            .expect("open"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Editor,
                2,
                "editor-trim",
                IpcCommand::EditorApply {
                    base_revision: 1,
                    mutation: EditorMutation::Trim {
                        start_ms: 1_000,
                        end_ms: 80_000,
                    },
                },
            ))
            .expect("trim"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Editor,
                3,
                "editor-save",
                IpcCommand::EditorSave {
                    expected_revision: 2,
                },
            ))
            .expect("save"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Editor,
                4,
                "export-start",
                IpcCommand::ExportStart {
                    project_revision: 2,
                    output_path: absolute_test_path(&["frame", "exports", "demo.mp4"]),
                    profile: ExportProfile::DistributionMp4,
                },
            ))
            .expect("export"));
        ok(&runtime
            .dispatch(request(
                &runtime,
                WindowRole::Editor,
                5,
                "upload-start",
                IpcCommand::UploadStart {
                    source_path: absolute_test_path(&["frame", "media", "demo.mp4"]),
                    upload_intent: "upload-start".into(),
                },
            ))
            .expect("upload"));
        runtime.advance_fake().expect("advance");
        let snapshot = runtime.snapshot();
        assert!(matches!(snapshot.export, ExportState::Completed { .. }));
        assert_eq!(snapshot.upload, UploadState::Completed);
    }

    #[test]
    fn stale_cross_window_unapproved_and_out_of_scope_requests_fail_closed() {
        let mut runtime = runtime();
        runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                1,
                "accepted-once",
                IpcCommand::RecorderPrepare,
            ))
            .expect("accepted");
        assert!(matches!(
            runtime.dispatch(request(
                &runtime,
                WindowRole::Recorder,
                1,
                "replayed",
                IpcCommand::RecorderPrepare,
            )),
            Err(DesktopRuntimeError::Ipc(IpcError::Replay))
        ));
        assert!(matches!(
            runtime.dispatch(request(
                &runtime,
                WindowRole::Recorder,
                2,
                "accepted-once",
                IpcCommand::RecorderPrepare,
            )),
            Err(DesktopRuntimeError::Ipc(IpcError::DuplicateRequestId))
        ));
        assert!(matches!(
            runtime.dispatch(request(
                &runtime,
                WindowRole::Recovery,
                1,
                "cross-window-editor",
                IpcCommand::EditorOpen {
                    project_path: absolute_test_path(&["frame", "projects", "demo.frame"]),
                },
            )),
            Err(DesktopRuntimeError::Ipc(IpcError::CommandOutOfScope))
        ));
        assert!(matches!(
            runtime.dispatch(request(
                &runtime,
                WindowRole::Editor,
                1,
                "outside-root",
                IpcCommand::EditorOpen {
                    project_path: absolute_test_path(&["private", "secret.frame"]),
                },
            )),
            Err(DesktopRuntimeError::Ipc(IpcError::PathOutOfScope))
        ));
        assert!(matches!(
            runtime.dispatch_json("{private"),
            Err(DesktopRuntimeError::Ipc(IpcError::MalformedEnvelope))
        ));
    }

    #[test]
    fn device_loss_restart_and_update_relaunch_preserve_backend_truth() {
        let mut runtime = runtime();
        for (sequence, id, command) in [
            (
                1,
                "devices",
                IpcCommand::DeviceEnumerate {
                    class: DeviceClass::Display,
                },
            ),
            (
                2,
                "target",
                IpcCommand::CaptureTargetSelect {
                    kind: CaptureTargetKind::Display,
                    target_token: "fake-display-1".into(),
                },
            ),
            (3, "prepare", IpcCommand::RecorderPrepare),
            (
                4,
                "start",
                IpcCommand::RecorderStart {
                    intent_id: "start".into(),
                },
            ),
        ] {
            runtime
                .dispatch(request(
                    &runtime,
                    WindowRole::Recorder,
                    sequence,
                    id,
                    command,
                ))
                .expect("recorder step");
        }
        runtime.simulate_fake_device_loss().expect("device loss");
        assert_eq!(runtime.snapshot().recorder, RecorderState::Recoverable);
        runtime.simulate_fake_restart().expect("restart");
        assert!(runtime.snapshot().crash_recovery_reported);
        assert_ne!(runtime.snapshot().recorder, RecorderState::Recording);

        for (sequence, id, action) in [
            (1, "update-check", UpdateAction::Check),
            (2, "update-install", UpdateAction::Install),
            (3, "update-relaunch", UpdateAction::Relaunch),
        ] {
            runtime
                .dispatch(request(
                    &runtime,
                    WindowRole::Main,
                    sequence,
                    id,
                    IpcCommand::Update {
                        action,
                        expected_revision: 1,
                    },
                ))
                .expect("update step");
        }
        assert_eq!(
            runtime.snapshot().update,
            UpdateState::Current { revision: 2 }
        );
        assert_eq!(runtime.snapshot().recorder, RecorderState::Recoverable);
    }

    #[test]
    fn release_adapter_never_claims_capture_or_os_lifecycle_success() {
        let mut runtime =
            DesktopRuntime::new(DesktopAdapterKind::Unavailable, roots(), "release-1")
                .expect("runtime");
        let dispatch = runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                1,
                "prepare",
                IpcCommand::RecorderPrepare,
            ))
            .expect("bounded response");
        assert_eq!(
            dispatch.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Unavailable,
                retryable: true,
            }
        );
        assert_ne!(dispatch.snapshot.recorder, RecorderState::Recording);
        assert!(!dispatch.snapshot.lifecycle.hotkeys_registered);
    }

    #[test]
    fn additive_capture_snapshot_fields_default_for_existing_runtime_v2_payloads() {
        let runtime = runtime();
        let mut value = serde_json::to_value(runtime.snapshot()).expect("snapshot json");
        let object = value.as_object_mut().expect("snapshot object");
        object.remove("capture_targets");
        object.remove("capture_artifact");
        let decoded: DesktopRuntimeSnapshot =
            serde_json::from_value(value).expect("compatible v2 snapshot");
        assert_eq!(decoded.capture_targets, CaptureTargetCatalog::empty());
        assert!(decoded.capture_artifact.is_none());
    }

    #[test]
    fn rejected_execution_discards_candidate_state_atomically() {
        let mut runtime = runtime();
        let before = runtime.snapshot();
        let dispatch = runtime
            .dispatch(request(
                &runtime,
                WindowRole::Recorder,
                1,
                "premature-device-select",
                IpcCommand::DeviceSelect {
                    class: DeviceClass::Microphone,
                    device_token: "fake-microphone-1".into(),
                },
            ))
            .expect("bounded error response");
        assert!(matches!(
            dispatch.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Conflict,
                ..
            }
        ));
        assert_eq!(
            dispatch.snapshot.selected_sources, before.selected_sources,
            "candidate source selection must not leak through a rejected workflow transition"
        );
        assert_eq!(dispatch.snapshot.devices, before.devices);
    }

    #[test]
    fn injected_native_backend_confirms_display_capture_and_editable_webm_export() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);

        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "native-start",
            IpcCommand::RecorderStart {
                intent_id: "native-start".into(),
            },
        );
        let started = runtime
            .dispatch_native(start, &mut backend)
            .expect("native start");
        ok(&started);
        assert_eq!(started.snapshot.recorder, RecorderState::Recording);
        assert!(started.snapshot.capture_artifact.is_none());

        let stop = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "native-stop",
            IpcCommand::RecorderStop {
                intent_id: "native-stop".into(),
            },
        );
        let stopped = runtime
            .dispatch_native(stop, &mut backend)
            .expect("native stop");
        ok(&stopped);
        assert_eq!(stopped.snapshot.recorder, RecorderState::Ready);
        let artifact = stopped
            .snapshot
            .capture_artifact
            .expect("sealed artifact summary");
        assert_eq!(artifact.artifact_revision, 11);
        assert_eq!(artifact.schema_version, CAPTURE_ARTIFACT_SUMMARY_VERSION);
        assert!(artifact.editable_webm_output_path.is_some());

        let export = request(
            &runtime,
            WindowRole::Export,
            1,
            "native-export",
            IpcCommand::ExportStart {
                project_revision: artifact.artifact_revision,
                output_path: artifact
                    .editable_webm_output_path
                    .expect("approved export path"),
                profile: ExportProfile::EditableWebm,
            },
        );
        let exported = runtime
            .dispatch_native(export, &mut backend)
            .expect("native export");
        ok(&exported);
        assert_eq!(
            exported.snapshot.export,
            ExportState::Completed {
                project_revision: 11,
            }
        );
        assert_eq!(
            backend.calls,
            ["enumerate", "select", "prepare", "start", "stop", "export"]
        );
    }

    #[test]
    fn native_target_selection_rejects_stale_token_and_generation() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        let enumerate = request(
            &runtime,
            WindowRole::Recorder,
            1,
            "catalog-one",
            IpcCommand::DeviceEnumerate {
                class: DeviceClass::Display,
            },
        );
        ok(&runtime
            .dispatch_native(enumerate, &mut backend)
            .expect("first catalog"));

        backend.catalog = native_catalog(2, "display-token-2");
        let enumerate = request(
            &runtime,
            WindowRole::Recorder,
            2,
            "catalog-two",
            IpcCommand::DeviceEnumerate {
                class: DeviceClass::Display,
            },
        );
        ok(&runtime
            .dispatch_native(enumerate, &mut backend)
            .expect("second catalog"));

        let stale_token = request(
            &runtime,
            WindowRole::Recorder,
            3,
            "stale-token",
            IpcCommand::CaptureTargetSelect {
                kind: CaptureTargetKind::Display,
                target_token: "display-token-1".into(),
            },
        );
        let rejected = runtime
            .dispatch_native(stale_token, &mut backend)
            .expect("bounded stale-token response");
        assert!(matches!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Conflict,
                ..
            }
        ));
        assert_eq!(backend.call_count("select"), 0);

        backend.select_generation_override = Some(1);
        let stale_generation = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "stale-generation",
            IpcCommand::CaptureTargetSelect {
                kind: CaptureTargetKind::Display,
                target_token: "display-token-2".into(),
            },
        );
        let rejected = runtime
            .dispatch_native(stale_generation, &mut backend)
            .expect("bounded stale-generation response");
        assert_eq!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Internal,
                retryable: false,
            }
        );
        assert!(rejected.snapshot.selected_sources.target.is_none());
        assert_eq!(backend.call_count("select"), 1);
    }

    #[test]
    fn native_backend_error_never_creates_optimistic_recording_success() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        backend.start_error = Some(NativeDesktopBackendError::Busy);

        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "busy-start",
            IpcCommand::RecorderStart {
                intent_id: "busy-start".into(),
            },
        );
        let rejected = runtime
            .dispatch_native(start, &mut backend)
            .expect("bounded busy response");
        assert_eq!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Busy,
                retryable: true,
            }
        );
        assert_eq!(rejected.snapshot.recorder, RecorderState::Idle);
        assert!(rejected.snapshot.capture_artifact.is_none());
        assert!(runtime.native_recording.is_none());
        assert_eq!(backend.call_count("start"), 1);
    }

    #[test]
    fn native_recorder_poll_is_bounded_to_an_active_recording() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);

        let premature = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "premature-native-poll",
            IpcCommand::RecorderPoll,
        );
        let rejected = runtime
            .dispatch_native(premature, &mut backend)
            .expect("bounded poll response");
        assert_eq!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Conflict,
                retryable: true,
            }
        );
        assert_eq!(backend.call_count("poll"), 0);
        assert_eq!(rejected.snapshot.recorder, RecorderState::Idle);
    }

    #[test]
    fn native_recorder_poll_without_failure_preserves_recording_authority() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "poll-healthy-start",
            IpcCommand::RecorderStart {
                intent_id: "poll-healthy-start".into(),
            },
        );
        ok(&runtime
            .dispatch_native(start, &mut backend)
            .expect("native start"));

        let poll = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "poll-healthy",
            IpcCommand::RecorderPoll,
        );
        let healthy = runtime
            .dispatch_native(poll, &mut backend)
            .expect("healthy poll");
        ok(&healthy);
        assert_eq!(healthy.snapshot.recorder, RecorderState::Recording);
        assert!(runtime.native_recording.is_some());
        assert_eq!(backend.call_count("poll"), 1);
        assert!(backend.poll_request_matches_recording);
        assert!(healthy.events.iter().all(|event| !matches!(
            &event.event,
            DesktopRuntimeEvent::Backend(BackendEvent::RecorderFailed { .. })
        )));
    }

    #[test]
    fn native_recorder_poll_reconciles_terminal_worker_failure_unsolicited() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "poll-failure-start",
            IpcCommand::RecorderStart {
                intent_id: "poll-failure-start".into(),
            },
        );
        ok(&runtime
            .dispatch_native(start, &mut backend)
            .expect("native start"));
        backend.poll_failure = Some(NativeRecordingTerminalFailure {
            recording_token: "recording-token-1".into(),
            error: NativeDesktopBackendError::Filesystem,
            teardown_confirmed: true,
        });

        let poll = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "poll-failure",
            IpcCommand::RecorderPoll,
        );
        let failed = runtime
            .dispatch_native(poll, &mut backend)
            .expect("terminal poll response");
        ok(&failed);
        assert_eq!(
            failed.snapshot.recorder,
            RecorderState::Failed {
                code: SafeFailureCode::DiskFull,
                retryable: true,
            }
        );
        assert!(failed.events.iter().any(|event| matches!(
            &event.event,
            DesktopRuntimeEvent::Backend(BackendEvent::RecorderFailed {
                code: SafeFailureCode::DiskFull,
                retryable: true,
            })
        )));
        assert!(runtime.native_recording.is_none());
        assert!(failed.snapshot.capture_targets.targets.is_empty());
        assert!(failed.snapshot.selected_sources.target.is_none());
        assert!(!failed.snapshot.lifecycle.overlay_visible);
        assert!(failed.snapshot.capture_artifact.is_none());
        assert_eq!(backend.call_count("poll"), 1);
        assert!(backend.poll_request_matches_recording);
    }

    #[test]
    fn native_recorder_poll_rejects_failure_for_another_recording() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "poll-stale-start",
            IpcCommand::RecorderStart {
                intent_id: "poll-stale-start".into(),
            },
        );
        ok(&runtime
            .dispatch_native(start, &mut backend)
            .expect("native start"));
        backend.poll_failure = Some(NativeRecordingTerminalFailure {
            recording_token: "stale-recording-token".into(),
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: true,
        });

        let poll = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "poll-stale",
            IpcCommand::RecorderPoll,
        );
        let rejected = runtime
            .dispatch_native(poll, &mut backend)
            .expect("bounded invalid-backend response");
        assert_eq!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Internal,
                retryable: false,
            }
        );
        assert_eq!(rejected.snapshot.recorder, RecorderState::Recording);
        assert!(runtime.native_recording.is_some());
        assert_eq!(backend.call_count("poll"), 1);
    }

    #[test]
    fn native_terminal_stop_failure_clears_consumed_authority_and_catalog() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "terminal-failure-start",
            IpcCommand::RecorderStart {
                intent_id: "terminal-failure-start".into(),
            },
        );
        ok(&runtime
            .dispatch_native(start, &mut backend)
            .expect("native start"));
        backend.stop_failure = Some(NativeRecordingTerminalFailure {
            recording_token: "recording-token-1".into(),
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: true,
        });

        let stop = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "terminal-failure-stop",
            IpcCommand::RecorderStop {
                intent_id: "terminal-failure-stop".into(),
            },
        );
        let failed = runtime
            .dispatch_native(stop, &mut backend)
            .expect("terminal failure response");
        ok(&failed);
        assert_eq!(
            failed.snapshot.recorder,
            RecorderState::Failed {
                code: SafeFailureCode::Internal,
                retryable: false,
            }
        );
        assert!(runtime.native_recording.is_none());
        assert!(failed.snapshot.capture_targets.targets.is_empty());
        assert_eq!(failed.snapshot.capture_targets.generation, 1);
        assert!(failed.snapshot.selected_sources.target.is_none());
        assert!(!failed.snapshot.lifecycle.overlay_visible);

        let cancel = request(
            &runtime,
            WindowRole::Recorder,
            6,
            "terminal-failure-cancel",
            IpcCommand::RecorderCancel {
                intent_id: "terminal-failure-cancel".into(),
            },
        );
        let rejected = runtime
            .dispatch_native(cancel, &mut backend)
            .expect("bounded no-authority response");
        assert!(matches!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Conflict,
                ..
            }
        ));
        assert_eq!(backend.call_count("cancel"), 0);
    }

    #[test]
    fn native_unconfirmed_cancel_transitions_to_nonretryable_backend_failure() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "unconfirmed-cancel-start",
            IpcCommand::RecorderStart {
                intent_id: "unconfirmed-cancel-start".into(),
            },
        );
        ok(&runtime
            .dispatch_native(start, &mut backend)
            .expect("native start"));
        backend.cancel_failure = Some(NativeRecordingTerminalFailure {
            recording_token: "recording-token-1".into(),
            error: NativeDesktopBackendError::Internal,
            teardown_confirmed: false,
        });

        let cancel = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "unconfirmed-cancel",
            IpcCommand::RecorderCancel {
                intent_id: "unconfirmed-cancel".into(),
            },
        );
        let failed = runtime
            .dispatch_native(cancel, &mut backend)
            .expect("terminal cancel failure response");
        ok(&failed);
        assert_eq!(
            failed.snapshot.recorder,
            RecorderState::Failed {
                code: SafeFailureCode::BackendUnavailable,
                retryable: false,
            }
        );
        assert!(runtime.native_recording.is_none());
        assert!(failed.snapshot.capture_targets.targets.is_empty());
    }

    #[test]
    fn native_busy_stop_error_preserves_active_recording_authority() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "busy-stop-start",
            IpcCommand::RecorderStart {
                intent_id: "busy-stop-start".into(),
            },
        );
        ok(&runtime
            .dispatch_native(start, &mut backend)
            .expect("native start"));
        let before = runtime.snapshot();
        backend.stop_error = Some(NativeDesktopBackendError::Busy);

        let stop = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "busy-stop",
            IpcCommand::RecorderStop {
                intent_id: "busy-stop".into(),
            },
        );
        let rejected = runtime
            .dispatch_native(stop, &mut backend)
            .expect("bounded busy response");
        assert_eq!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Busy,
                retryable: true,
            }
        );
        assert_eq!(rejected.snapshot.recorder, RecorderState::Recording);
        assert_eq!(rejected.snapshot.capture_targets, before.capture_targets);
        assert_eq!(rejected.snapshot.selected_sources, before.selected_sources);
        assert!(runtime.native_recording.is_some());
    }

    #[test]
    fn native_permission_denial_is_a_confirmed_state_not_recording_success() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        for (sequence, id, command) in [
            (
                1,
                "denied-enumerate",
                IpcCommand::DeviceEnumerate {
                    class: DeviceClass::Display,
                },
            ),
            (
                2,
                "denied-select",
                IpcCommand::CaptureTargetSelect {
                    kind: CaptureTargetKind::Display,
                    target_token: "display-token-1".into(),
                },
            ),
        ] {
            let envelope = request(&runtime, WindowRole::Recorder, sequence, id, command);
            ok(&runtime
                .dispatch_native(envelope, &mut backend)
                .expect("native setup"));
        }
        backend.permission = NativePermissionOutcome::Denied;
        let prepare = request(
            &runtime,
            WindowRole::Recorder,
            3,
            "permission-denied",
            IpcCommand::RecorderPrepare,
        );
        let denied = runtime
            .dispatch_native(prepare, &mut backend)
            .expect("confirmed denial");
        ok(&denied);
        assert_eq!(denied.snapshot.permission, PermissionState::Denied);
        assert_eq!(denied.snapshot.devices, DeviceState::PermissionDenied);
        assert!(denied.snapshot.selected_sources.target.is_none());
        assert_eq!(denied.snapshot.recorder, RecorderState::Idle);
    }

    #[test]
    fn native_artifact_and_export_paths_remain_scoped() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        prepare_native_recording(&mut runtime, &mut backend);
        let start = request(
            &runtime,
            WindowRole::Recorder,
            4,
            "scoped-start",
            IpcCommand::RecorderStart {
                intent_id: "scoped-start".into(),
            },
        );
        ok(&runtime
            .dispatch_native(start, &mut backend)
            .expect("native start"));

        backend.stop_artifact.media_path =
            absolute_test_path(&["private", "outside", "capture.webm"]);
        let stop = request(
            &runtime,
            WindowRole::Recorder,
            5,
            "unscoped-artifact",
            IpcCommand::RecorderStop {
                intent_id: "unscoped-artifact".into(),
            },
        );
        let rejected = runtime
            .dispatch_native(stop, &mut backend)
            .expect("bounded invalid artifact response");
        ok(&rejected);
        assert_eq!(
            rejected.snapshot.recorder,
            RecorderState::Failed {
                code: SafeFailureCode::Internal,
                retryable: false,
            }
        );
        assert!(runtime.native_recording.is_none());
        assert!(rejected.snapshot.capture_artifact.is_none());

        let export = request(
            &runtime,
            WindowRole::Export,
            1,
            "outside-export",
            IpcCommand::ExportStart {
                project_revision: 11,
                output_path: absolute_test_path(&["private", "outside", "capture.webm"]),
                profile: ExportProfile::EditableWebm,
            },
        );
        assert!(matches!(
            runtime.dispatch_native(export, &mut backend),
            Err(DesktopRuntimeError::Ipc(IpcError::PathOutOfScope))
        ));
        assert_eq!(backend.export_calls, 0);
    }

    #[test]
    fn native_adapter_without_injection_keeps_dispatch_json_fail_closed() {
        let mut runtime = native_runtime();
        let envelope = request(
            &runtime,
            WindowRole::Recorder,
            1,
            "no-injection",
            IpcCommand::RecorderPrepare,
        );
        let json = serde_json::to_string(&envelope).expect("request json");
        let rejected = runtime.dispatch_json(&json).expect("bounded response");
        assert_eq!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Unavailable,
                retryable: true,
            }
        );
        assert_eq!(rejected.snapshot.permission, PermissionState::NotDetermined);
    }

    #[test]
    fn native_backend_rejects_window_audio_camera_pause_and_mp4_capabilities() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();

        for (sequence, id, command) in [
            (
                1,
                "native-microphones",
                IpcCommand::DeviceEnumerate {
                    class: DeviceClass::Microphone,
                },
            ),
            (
                2,
                "native-window",
                IpcCommand::CaptureTargetSelect {
                    kind: CaptureTargetKind::Window,
                    target_token: "window-token-1".into(),
                },
            ),
            (
                3,
                "native-pause",
                IpcCommand::RecorderPause {
                    intent_id: "native-pause".into(),
                },
            ),
        ] {
            let envelope = request(&runtime, WindowRole::Recorder, sequence, id, command);
            let rejected = runtime
                .dispatch_native(envelope, &mut backend)
                .expect("bounded unsupported response");
            assert_eq!(
                rejected.response.outcome,
                CommandOutcome::Error {
                    code: PublicErrorCode::Unavailable,
                    retryable: true,
                }
            );
        }

        let settings = request(
            &runtime,
            WindowRole::Settings,
            1,
            "native-av-settings",
            IpcCommand::SettingsApply {
                expected_revision: 1,
                mode: RecorderMode::Instant,
                frame_rate: 30,
                microphone_enabled: true,
                system_audio_enabled: true,
                camera_enabled: true,
                reduced_motion: false,
            },
        );
        let rejected = runtime
            .dispatch_native(settings, &mut backend)
            .expect("bounded unsupported settings response");
        assert!(matches!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Unavailable,
                ..
            }
        ));
        assert!(!rejected.snapshot.settings.microphone_enabled);
        assert!(!rejected.snapshot.settings.system_audio_enabled);
        assert!(!rejected.snapshot.settings.camera_enabled);

        let mp4 = request(
            &runtime,
            WindowRole::Export,
            1,
            "native-mp4",
            IpcCommand::ExportStart {
                project_revision: 1,
                output_path: absolute_test_path(&["frame", "exports", "capture.mp4"]),
                profile: ExportProfile::DistributionMp4,
            },
        );
        let rejected = runtime
            .dispatch_native(mp4, &mut backend)
            .expect("bounded unsupported MP4 response");
        assert!(matches!(
            rejected.response.outcome,
            CommandOutcome::Error {
                code: PublicErrorCode::Unavailable,
                ..
            }
        ));
        assert!(backend.calls.is_empty());
    }

    #[test]
    fn native_backend_accepts_system_audio_without_enabling_unimplemented_sources() {
        let mut runtime = native_runtime();
        let mut backend = TestNativeBackend::new();
        let settings = request(
            &runtime,
            WindowRole::Settings,
            1,
            "native-system-audio-settings",
            IpcCommand::SettingsApply {
                expected_revision: 1,
                mode: RecorderMode::Instant,
                frame_rate: 30,
                microphone_enabled: false,
                system_audio_enabled: true,
                camera_enabled: false,
                reduced_motion: false,
            },
        );

        let applied = runtime
            .dispatch_native(settings, &mut backend)
            .expect("bounded native system-audio settings response");

        assert!(matches!(
            applied.response.outcome,
            CommandOutcome::Ok { .. }
        ));
        assert!(!applied.snapshot.settings.microphone_enabled);
        assert!(applied.snapshot.settings.system_audio_enabled);
        assert!(!applied.snapshot.settings.camera_enabled);
    }

    #[test]
    fn native_start_response_cannot_add_unrequested_system_audio() {
        let outcome = NativeRecordingStartOutcome {
            catalog_generation: 7,
            target_token: "display-token-1".into(),
            recording_token: "recording-token-1".into(),
            system_audio_included: true,
        };

        assert!(
            validate_start_outcome(&outcome, 7, "display-token-1", false).is_err(),
            "the backend must not add an unrequested native source"
        );
        assert!(validate_start_outcome(&outcome, 7, "display-token-1", true).is_ok());
    }

    #[test]
    fn instant_finalize_requires_exact_sequence_and_never_regresses_terminal_progress() {
        let mut runtime = runtime();
        let handle = bind_finalize(&mut runtime, 'a');
        runtime.instant_finalize_last_sequence = 1;
        let ready =
            InstantUiProgressV1::new(InstantUiPhaseV1::ShareReady, Some(10_000), false, None)
                .expect("share ready");
        runtime
            .apply_instant_finalize_progress(&handle, 2, ready)
            .expect("newest completion");
        let terminal = runtime.snapshot();
        assert!(matches!(
            runtime.apply_instant_finalize_progress(&handle, 1, finalizing_progress()),
            Err(DesktopRuntimeError::InstantFinalizeAuthorityMismatch)
        ));
        assert_eq!(runtime.snapshot(), terminal);
        assert_eq!(terminal.instant_progress, Some(ready));
        assert!(terminal.instant_finalize_handle.is_none());
        assert!(terminal.instant_finalize_next_sequence.is_none());
    }

    #[test]
    fn instant_finalize_bootstrap_advances_the_native_command_sequence() {
        let mut runtime = runtime();
        let handle = bind_finalize(&mut runtime, 'b');
        runtime
            .apply_instant_finalize_progress(&handle, 1, finalizing_progress())
            .expect("pending finalize");
        let bootstrap = runtime.bootstrap();
        assert_eq!(bootstrap.snapshot.instant_finalize_next_sequence, Some(2));
        assert_eq!(
            bootstrap.snapshot.instant_finalize,
            InstantFinalizeCapabilityState::Available
        );
    }

    #[test]
    fn instant_finalize_overflow_failures_leave_runtime_state_atomic() {
        let mut revision_runtime = runtime();
        let revision_handle = bind_finalize(&mut revision_runtime, 'c');
        revision_runtime.operation_revision = u64::MAX;
        let before_revision = revision_runtime.snapshot();
        assert!(matches!(
            revision_runtime.apply_instant_finalize_progress(
                &revision_handle,
                1,
                finalizing_progress(),
            ),
            Err(DesktopRuntimeError::RevisionOverflow)
        ));
        assert_eq!(revision_runtime.snapshot(), before_revision);

        let mut event_runtime = runtime();
        let event_handle = bind_finalize(&mut event_runtime, 'd');
        event_runtime.event_sequence = u64::MAX;
        let before_event = event_runtime.snapshot();
        assert!(matches!(
            event_runtime.apply_instant_finalize_progress(&event_handle, 1, finalizing_progress(),),
            Err(DesktopRuntimeError::SequenceOverflow)
        ));
        assert_eq!(event_runtime.snapshot(), before_event);
    }

    #[test]
    fn instant_finalize_update_shape_is_versioned_and_contains_no_native_authority() {
        let mut runtime = runtime();
        let handle = bind_finalize(&mut runtime, 'e');
        let update = runtime
            .apply_instant_finalize_progress(&handle, 1, finalizing_progress())
            .expect("UI update");
        let json = serde_json::to_value(&update).expect("update JSON");
        assert_eq!(
            json,
            serde_json::json!({
                "runtime_version": 2,
                "command_protocol_version": 1,
                "command_sequence": 1,
                "operation_revision": 2,
                "events": [
                    {
                        "protocol_version": 2,
                        "event_sequence": 1,
                        "owner": "recorder",
                        "event": {
                            "event": "instant_progress",
                            "data": {
                                "schema_version": 1,
                                "phase": "finalizing",
                                "progress_basis_points": null,
                                "retrying": false,
                                "error": null
                            }
                        }
                    },
                    {
                        "protocol_version": 2,
                        "event_sequence": 2,
                        "owner": "recorder",
                        "event": {
                            "event": "state_confirmed",
                            "data": { "operation_revision": 2 }
                        }
                    }
                ],
                "progress": {
                    "schema_version": 1,
                    "phase": "finalizing",
                    "progress_basis_points": null,
                    "retrying": false,
                    "error": null
                }
            })
        );
        let encoded = serde_json::to_string(&update).expect("encoded update");
        assert!(!encoded.contains(&"e".repeat(64)));
        for forbidden in [
            "bearer",
            "tenant_id",
            "video_id",
            "request_sha256",
            "publication_id",
            "upload_id",
            "object_version",
        ] {
            assert!(!encoded.contains(forbidden));
        }
    }

    #[test]
    fn permanent_finalize_failure_disables_retry_and_clears_webview_handle() {
        let mut runtime = runtime();
        let handle = bind_finalize(&mut runtime, 'f');
        let retrying = InstantUiProgressV1::new(
            InstantUiPhaseV1::Finalizing,
            None,
            true,
            Some(InstantUiErrorCodeV1::FinalizeDelayed),
        )
        .expect("retrying progress");
        runtime
            .apply_instant_finalize_progress(&handle, 1, retrying)
            .expect("transient status");
        let update = runtime
            .disable_native_instant_finalize(&handle, 2)
            .expect("terminal status");
        assert_eq!(update.progress.phase, InstantUiPhaseV1::RecoveryRequired);
        assert!(!update.progress.retrying);
        let snapshot = runtime.snapshot();
        assert!(snapshot.instant_finalize_handle.is_none());
        assert!(snapshot.instant_finalize_next_sequence.is_none());
        assert_eq!(snapshot.instant_progress, Some(update.progress));
    }

    #[test]
    fn rebinding_live_finalize_authority_is_rejected() {
        let mut runtime = runtime();
        bind_finalize(&mut runtime, 'a');
        let second = InstantFinalizeRegistrationV1 {
            handle: finalize_handle('b'),
            progress: finalizing_progress(),
        };
        assert!(matches!(
            runtime.bind_native_instant_finalize(second),
            Err(DesktopRuntimeError::InstantFinalizeAuthorityMismatch)
        ));
    }
}
