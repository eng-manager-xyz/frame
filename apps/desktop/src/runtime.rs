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

use crate::{
    ipc::{
        CaptureTargetKind, CommandOutcome, DeviceClass, IpcCommand, IpcError, LifecycleAction,
        PathPolicy, PublicErrorCode, RecorderMode, RequestEnvelope, ResponseEnvelope, RootAccess,
        ScopeRegistry, SessionId, UpdateAction, WindowId, WindowRole, WindowScope, decode_request,
    },
    workflow::{
        BackendEvent, BackendEventEnvelope, DesktopWorkflow, DeviceCounts, DeviceState,
        EditorState, ExportState, IntentKind, RecorderState, RecoveryState, SafeFailureCode,
        UiIntent, UploadState, WORKFLOW_PROTOCOL_VERSION, WorkflowError,
    },
};

pub const DESKTOP_RUNTIME_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopAdapterKind {
    Unavailable,
    DeterministicFake,
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
    pub permission: PermissionState,
    pub meter: AudioMeterSnapshot,
    pub recorder_configuration: RecorderConfiguration,
    pub selected_sources: SelectedSources,
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
    StateConfirmed { operation_revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopDispatch {
    pub response: ResponseEnvelope,
    pub events: Vec<DesktopEventEnvelope>,
    pub snapshot: DesktopRuntimeSnapshot,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DesktopRoots {
    projects: String,
    media: String,
    exports: String,
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
    permission: PermissionState,
    meter: AudioMeterSnapshot,
    recorder_configuration: RecorderConfiguration,
    selected_sources: SelectedSources,
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
            settings: DesktopSettingsSnapshot {
                revision: 1,
                mode: RecorderMode::Instant,
                frame_rate: 30,
                microphone_enabled: true,
                system_audio_enabled: true,
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
            permission: self.permission,
            meter: self.meter,
            recorder_configuration: self.recorder_configuration,
            selected_sources: self.selected_sources,
            settings: self.settings,
            lifecycle: self.lifecycle,
            update: self.update,
            crash_recovery_reported: self.crash_recovery_reported,
            legacy_desktop_selectable: true,
            announcement: self.announcement.clone(),
        }
    }

    pub fn dispatch_json(&mut self, json: &str) -> Result<DesktopDispatch, DesktopRuntimeError> {
        let request = decode_request(json)?;
        self.dispatch(request)
    }

    pub fn dispatch(
        &mut self,
        request: RequestEnvelope,
    ) -> Result<DesktopDispatch, DesktopRuntimeError> {
        let accepted = self.registry.accept(request)?;
        let owner = self.owner_for(&accepted.request)?;
        let response_scope = &accepted.request;
        let mut candidate = self.clone();
        match candidate.execute(owner, &accepted.request.command) {
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
}

impl DesktopRuntimeError {
    #[must_use]
    pub const fn public_code(&self) -> PublicErrorCode {
        match self {
            Self::Ipc(error) => error.public_code(),
            Self::FakeAdapterRequired => PublicErrorCode::Unavailable,
            Self::Workflow(_)
            | Self::RevisionOverflow
            | Self::SequenceOverflow
            | Self::ScopeInvariant => PublicErrorCode::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{EditorMutation, ExportProfile, IPC_PROTOCOL_VERSION, RequestId};

    fn runtime() -> DesktopRuntime {
        DesktopRuntime::new(
            DesktopAdapterKind::DeterministicFake,
            DesktopRoots::new("/frame/projects", "/frame/media", "/frame/exports"),
            "test-1",
        )
        .expect("runtime")
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
                    project_path: "/frame/projects/demo.frame".into(),
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
                    output_path: "/frame/exports/demo.mp4".into(),
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
                    source_path: "/frame/media/demo.mp4".into(),
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
                    project_path: "/frame/projects/demo.frame".into(),
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
                    project_path: "/private/secret.frame".into(),
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
        let mut runtime = DesktopRuntime::new(
            DesktopAdapterKind::Unavailable,
            DesktopRoots::new("/frame/projects", "/frame/media", "/frame/exports"),
            "release-1",
        )
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
}
