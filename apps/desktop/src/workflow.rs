use std::{
    collections::{HashMap, VecDeque},
    fmt,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ipc::SessionId;

pub const WORKFLOW_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowArea {
    Recorder,
    Devices,
    Recovery,
    Editor,
    Export,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeFailureCode {
    PermissionDenied,
    DeviceLost,
    DiskFull,
    NetworkUnavailable,
    InvalidProject,
    UnsupportedProject,
    ExportFailed,
    UploadFailed,
    BackendUnavailable,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RecorderState {
    Idle,
    Preparing,
    Recording,
    Paused,
    Recoverable,
    Ready,
    Failed {
        code: SafeFailureCode,
        retryable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCounts {
    pub displays: u16,
    pub microphones: u16,
    pub system_audio_sources: u16,
    pub cameras: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "data", rename_all = "snake_case")]
pub enum DeviceState {
    Unknown,
    Enumerating,
    Ready(DeviceCounts),
    PermissionDenied,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RecoveryState {
    Hidden,
    Scanning,
    Available { projects: u16 },
    Opening,
    Opened,
    Failed { code: SafeFailureCode },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EditorState {
    Closed,
    Loading,
    Ready {
        revision: u64,
        duration_ms: u64,
        dirty: bool,
    },
    Failed {
        code: SafeFailureCode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorOperation {
    Apply,
    Save,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorOperationFailure {
    pub operation: EditorOperation,
    pub code: SafeFailureCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ExportState {
    Idle,
    Running {
        project_revision: u64,
        progress_basis_points: u16,
    },
    Cancelling {
        project_revision: u64,
    },
    Completed {
        project_revision: u64,
    },
    Failed {
        code: SafeFailureCode,
        retryable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UploadState {
    Idle,
    Uploading {
        verified_parts: u32,
        total_parts: u32,
    },
    Paused {
        verified_parts: u32,
        total_parts: u32,
        reason: UploadPauseReason,
    },
    Finalizing,
    Completed,
    Failed {
        code: SafeFailureCode,
        retryable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadPauseReason {
    User,
    Offline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    RecorderStart,
    RecorderPause,
    RecorderResume,
    RecorderStop,
    RecorderCancel,
    RecorderRecover,
    DevicesRefresh,
    DeviceSelect,
    RecoveryScan,
    RecoveryOpen,
    RecoveryDiscard,
    EditorOpen,
    EditorApply { base_revision: u64 },
    EditorSave { expected_revision: u64 },
    ExportStart { project_revision: u64 },
    CaptureExportStart { artifact_revision: u64 },
    ExportCancel,
    UploadStart,
    UploadPause,
    UploadResume,
    UploadCancel,
}

impl IntentKind {
    #[must_use]
    pub const fn area(self) -> WorkflowArea {
        match self {
            Self::RecorderStart
            | Self::RecorderPause
            | Self::RecorderResume
            | Self::RecorderStop
            | Self::RecorderCancel
            | Self::RecorderRecover => WorkflowArea::Recorder,
            Self::DevicesRefresh | Self::DeviceSelect => WorkflowArea::Devices,
            Self::RecoveryScan | Self::RecoveryOpen | Self::RecoveryDiscard => {
                WorkflowArea::Recovery
            }
            Self::EditorOpen | Self::EditorApply { .. } | Self::EditorSave { .. } => {
                WorkflowArea::Editor
            }
            Self::ExportStart { .. } | Self::CaptureExportStart { .. } | Self::ExportCancel => {
                WorkflowArea::Export
            }
            Self::UploadStart | Self::UploadPause | Self::UploadResume | Self::UploadCancel => {
                WorkflowArea::Upload
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct UiIntent {
    id: String,
    pub kind: IntentKind,
}

impl UiIntent {
    pub fn new(id: impl Into<String>, kind: IntentKind) -> Result<Self, WorkflowError> {
        let id = id.into();
        if !valid_intent_id(&id) {
            return Err(WorkflowError::InvalidIntent);
        }
        Ok(Self { id, kind })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Debug for UiIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UiIntent")
            .field("id", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

fn valid_intent_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendEventEnvelope {
    pub protocol_version: u16,
    pub session_id: SessionId,
    pub sequence: u64,
    pub event: BackendEvent,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum BackendEvent {
    RecorderPreparing {
        intent_id: String,
    },
    RecorderStarted,
    RecorderPaused {
        intent_id: String,
    },
    RecorderResumed {
        intent_id: String,
    },
    RecorderStopped {
        intent_id: String,
        recoverable: bool,
    },
    RecorderCancelled {
        intent_id: String,
    },
    RecorderFailed {
        code: SafeFailureCode,
        retryable: bool,
    },
    DevicesEnumerating {
        intent_id: String,
    },
    DevicesReady {
        counts: DeviceCounts,
    },
    DeviceSelected {
        intent_id: String,
    },
    DevicePermissionDenied,
    DeviceLost,
    RecoveryScanning {
        intent_id: String,
    },
    RecoveryAvailable {
        projects: u16,
    },
    RecoveryOpening {
        intent_id: String,
    },
    RecoveryOpened,
    RecoveryDiscarded {
        intent_id: String,
        remaining: u16,
    },
    RecoveryFailed {
        code: SafeFailureCode,
    },
    EditorLoading {
        intent_id: String,
    },
    EditorLoaded {
        revision: u64,
        duration_ms: u64,
    },
    EditorApplied {
        intent_id: String,
        revision: u64,
    },
    EditorSaved {
        intent_id: String,
        revision: u64,
    },
    EditorFailed {
        code: SafeFailureCode,
    },
    ExportStarted {
        intent_id: String,
        project_revision: u64,
    },
    ExportProgress {
        progress_basis_points: u16,
    },
    ExportCancelling {
        intent_id: String,
    },
    ExportCompleted,
    ExportCancelled,
    ExportFailed {
        code: SafeFailureCode,
        retryable: bool,
    },
    UploadStarted {
        intent_id: String,
        total_parts: u32,
    },
    UploadProgress {
        verified_parts: u32,
        total_parts: u32,
    },
    UploadPaused {
        intent_id: String,
    },
    UploadOffline,
    UploadResumed {
        intent_id: String,
    },
    UploadFinalizing,
    UploadCompleted,
    UploadCancelled {
        intent_id: String,
    },
    UploadFailed {
        code: SafeFailureCode,
        retryable: bool,
    },
}

impl BackendEvent {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::RecorderPreparing { .. } => "recorder_preparing",
            Self::RecorderStarted => "recorder_started",
            Self::RecorderPaused { .. } => "recorder_paused",
            Self::RecorderResumed { .. } => "recorder_resumed",
            Self::RecorderStopped { .. } => "recorder_stopped",
            Self::RecorderCancelled { .. } => "recorder_cancelled",
            Self::RecorderFailed { .. } => "recorder_failed",
            Self::DevicesEnumerating { .. } => "devices_enumerating",
            Self::DevicesReady { .. } => "devices_ready",
            Self::DeviceSelected { .. } => "device_selected",
            Self::DevicePermissionDenied => "device_permission_denied",
            Self::DeviceLost => "device_lost",
            Self::RecoveryScanning { .. } => "recovery_scanning",
            Self::RecoveryAvailable { .. } => "recovery_available",
            Self::RecoveryOpening { .. } => "recovery_opening",
            Self::RecoveryOpened => "recovery_opened",
            Self::RecoveryDiscarded { .. } => "recovery_discarded",
            Self::RecoveryFailed { .. } => "recovery_failed",
            Self::EditorLoading { .. } => "editor_loading",
            Self::EditorLoaded { .. } => "editor_loaded",
            Self::EditorApplied { .. } => "editor_applied",
            Self::EditorSaved { .. } => "editor_saved",
            Self::EditorFailed { .. } => "editor_failed",
            Self::ExportStarted { .. } => "export_started",
            Self::ExportProgress { .. } => "export_progress",
            Self::ExportCancelling { .. } => "export_cancelling",
            Self::ExportCompleted => "export_completed",
            Self::ExportCancelled => "export_cancelled",
            Self::ExportFailed { .. } => "export_failed",
            Self::UploadStarted { .. } => "upload_started",
            Self::UploadProgress { .. } => "upload_progress",
            Self::UploadPaused { .. } => "upload_paused",
            Self::UploadOffline => "upload_offline",
            Self::UploadResumed { .. } => "upload_resumed",
            Self::UploadFinalizing => "upload_finalizing",
            Self::UploadCompleted => "upload_completed",
            Self::UploadCancelled { .. } => "upload_cancelled",
            Self::UploadFailed { .. } => "upload_failed",
        }
    }

    fn intent_id(&self) -> Option<&str> {
        match self {
            Self::RecorderPreparing { intent_id }
            | Self::RecorderPaused { intent_id }
            | Self::RecorderResumed { intent_id }
            | Self::RecorderStopped { intent_id, .. }
            | Self::RecorderCancelled { intent_id }
            | Self::DevicesEnumerating { intent_id }
            | Self::DeviceSelected { intent_id }
            | Self::RecoveryScanning { intent_id }
            | Self::RecoveryOpening { intent_id }
            | Self::RecoveryDiscarded { intent_id, .. }
            | Self::EditorLoading { intent_id }
            | Self::EditorApplied { intent_id, .. }
            | Self::EditorSaved { intent_id, .. }
            | Self::ExportStarted { intent_id, .. }
            | Self::ExportCancelling { intent_id }
            | Self::UploadStarted { intent_id, .. }
            | Self::UploadPaused { intent_id }
            | Self::UploadResumed { intent_id }
            | Self::UploadCancelled { intent_id } => Some(intent_id),
            _ => None,
        }
    }
}

impl fmt::Debug for BackendEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BackendEvent")
            .field(&self.name())
            .finish()
    }
}

#[derive(Clone)]
pub struct DesktopWorkflow {
    session_id: SessionId,
    last_backend_sequence: u64,
    pending: HashMap<WorkflowArea, UiIntent>,
    superseded_intents: VecDeque<String>,
    recorder: RecorderState,
    devices: DeviceState,
    recovery: RecoveryState,
    editor: EditorState,
    editor_operation_failure: Option<EditorOperationFailure>,
    export: ExportState,
    upload: UploadState,
}

impl fmt::Debug for DesktopWorkflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopWorkflow")
            .field("session_id", &"<redacted>")
            .field("last_backend_sequence", &self.last_backend_sequence)
            .field("pending_areas", &self.pending.keys().collect::<Vec<_>>())
            .field("superseded_count", &self.superseded_intents.len())
            .field("recorder", &self.recorder)
            .field("devices", &self.devices)
            .field("recovery", &self.recovery)
            .field("editor", &self.editor)
            .field("editor_operation_failure", &self.editor_operation_failure)
            .field("export", &self.export)
            .field("upload", &self.upload)
            .finish()
    }
}

impl DesktopWorkflow {
    #[must_use]
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            last_backend_sequence: 0,
            pending: HashMap::new(),
            superseded_intents: VecDeque::new(),
            recorder: RecorderState::Idle,
            devices: DeviceState::Unknown,
            recovery: RecoveryState::Hidden,
            editor: EditorState::Closed,
            editor_operation_failure: None,
            export: ExportState::Idle,
            upload: UploadState::Idle,
        }
    }

    #[must_use]
    pub const fn recorder(&self) -> RecorderState {
        self.recorder
    }

    #[must_use]
    pub const fn devices(&self) -> DeviceState {
        self.devices
    }

    #[must_use]
    pub const fn recovery(&self) -> RecoveryState {
        self.recovery
    }

    #[must_use]
    pub const fn editor(&self) -> EditorState {
        self.editor
    }

    #[must_use]
    pub const fn editor_operation_failure(&self) -> Option<EditorOperationFailure> {
        self.editor_operation_failure
    }

    #[must_use]
    pub const fn export(&self) -> ExportState {
        self.export
    }

    #[must_use]
    pub const fn upload(&self) -> UploadState {
        self.upload
    }

    #[must_use]
    pub const fn last_backend_sequence(&self) -> u64 {
        self.last_backend_sequence
    }

    #[must_use]
    pub fn pending_intent(&self, area: WorkflowArea) -> Option<&UiIntent> {
        self.pending.get(&area)
    }

    /// Records UI intent without changing backend-confirmed state. The shell can
    /// render the pending intent separately, but must not claim success until a
    /// matching backend event arrives.
    pub fn request(&mut self, intent: UiIntent) -> Result<(), WorkflowError> {
        let area = intent.kind.area();
        if self.pending.contains_key(&area) {
            return Err(WorkflowError::Busy(area));
        }
        self.validate_intent(intent.kind)?;
        if matches!(
            intent.kind,
            IntentKind::EditorOpen | IntentKind::EditorApply { .. } | IntentKind::EditorSave { .. }
        ) {
            self.editor_operation_failure = None;
        }
        self.pending.insert(area, intent);
        Ok(())
    }

    /// Applies one backend event atomically. Validation runs on a clone so an
    /// invalid, stale, or out-of-order event cannot partially mutate UI state.
    pub fn apply_backend(&mut self, envelope: BackendEventEnvelope) -> Result<(), WorkflowError> {
        if envelope.protocol_version != WORKFLOW_PROTOCOL_VERSION {
            return Err(WorkflowError::UnsupportedProtocol);
        }
        if envelope.session_id != self.session_id {
            return Err(WorkflowError::SessionMismatch);
        }
        let expected = self
            .last_backend_sequence
            .checked_add(1)
            .ok_or(WorkflowError::SequenceOverflow)?;
        if envelope.sequence < expected {
            return Err(WorkflowError::StaleBackendEvent);
        }
        if envelope.sequence > expected {
            return Err(WorkflowError::BackendSequenceGap);
        }

        let mut next = self.clone();
        next.apply_event(envelope.event)?;
        next.last_backend_sequence = envelope.sequence;
        *self = next;
        Ok(())
    }

    fn validate_intent(&self, intent: IntentKind) -> Result<(), WorkflowError> {
        let valid = match intent {
            IntentKind::RecorderStart => matches!(
                self.recorder,
                RecorderState::Idle
                    | RecorderState::Ready
                    | RecorderState::Failed {
                        retryable: true,
                        ..
                    }
            ),
            IntentKind::RecorderRecover => self.recorder == RecorderState::Recoverable,
            IntentKind::RecorderPause => self.recorder == RecorderState::Recording,
            IntentKind::RecorderResume => self.recorder == RecorderState::Paused,
            IntentKind::RecorderStop => {
                matches!(
                    self.recorder,
                    RecorderState::Recording | RecorderState::Paused
                )
            }
            IntentKind::RecorderCancel => matches!(
                self.recorder,
                RecorderState::Preparing
                    | RecorderState::Recording
                    | RecorderState::Paused
                    | RecorderState::Recoverable
            ),
            IntentKind::DevicesRefresh => !matches!(self.devices, DeviceState::Enumerating),
            IntentKind::DeviceSelect => matches!(self.devices, DeviceState::Ready(_)),
            IntentKind::RecoveryScan => !matches!(
                self.recovery,
                RecoveryState::Scanning | RecoveryState::Opening
            ),
            IntentKind::RecoveryOpen => {
                matches!(self.recovery, RecoveryState::Available { projects } if projects > 0)
            }
            IntentKind::RecoveryDiscard => {
                matches!(self.recovery, RecoveryState::Available { projects } if projects > 0)
            }
            IntentKind::EditorOpen => matches!(
                self.editor,
                EditorState::Closed | EditorState::Failed { .. }
            ),
            IntentKind::EditorApply { base_revision } => matches!(
                self.editor,
                EditorState::Ready { revision, .. } if revision == base_revision
            ),
            IntentKind::EditorSave { expected_revision } => matches!(
                self.editor,
                EditorState::Ready { revision, dirty: true, .. } if revision == expected_revision
            ),
            IntentKind::ExportStart { project_revision } => {
                matches!(self.editor, EditorState::Ready { revision, .. } if revision == project_revision)
                    && matches!(
                        self.export,
                        ExportState::Idle
                            | ExportState::Completed { .. }
                            | ExportState::Failed {
                                retryable: true,
                                ..
                            }
                    )
            }
            IntentKind::CaptureExportStart { artifact_revision } => {
                artifact_revision > 0
                    && matches!(
                        self.export,
                        ExportState::Idle
                            | ExportState::Completed { .. }
                            | ExportState::Failed {
                                retryable: true,
                                ..
                            }
                    )
            }
            IntentKind::ExportCancel => matches!(self.export, ExportState::Running { .. }),
            IntentKind::UploadStart => matches!(
                self.upload,
                UploadState::Idle
                    | UploadState::Completed
                    | UploadState::Failed {
                        retryable: true,
                        ..
                    }
            ),
            IntentKind::UploadPause => matches!(self.upload, UploadState::Uploading { .. }),
            IntentKind::UploadResume => matches!(self.upload, UploadState::Paused { .. }),
            IntentKind::UploadCancel => matches!(
                self.upload,
                UploadState::Uploading { .. }
                    | UploadState::Paused { .. }
                    | UploadState::Finalizing
            ),
        };
        if valid {
            Ok(())
        } else {
            Err(WorkflowError::InvalidIntentForState(intent.area()))
        }
    }

    fn apply_event(&mut self, event: BackendEvent) -> Result<(), WorkflowError> {
        if let Some(intent_id) = event.intent_id().map(str::to_owned)
            && self.consume_superseded(&intent_id)
        {
            return Ok(());
        }
        match event {
            BackendEvent::RecorderPreparing { intent_id } => {
                self.consume_intent(
                    WorkflowArea::Recorder,
                    &intent_id,
                    &[IntentKind::RecorderStart, IntentKind::RecorderRecover],
                )?;
                self.recorder = RecorderState::Preparing;
            }
            BackendEvent::RecorderStarted => {
                require(matches!(self.recorder, RecorderState::Preparing))?;
                self.recorder = RecorderState::Recording;
            }
            BackendEvent::RecorderPaused { intent_id } => {
                require(self.recorder == RecorderState::Recording)?;
                self.consume_intent(
                    WorkflowArea::Recorder,
                    &intent_id,
                    &[IntentKind::RecorderPause],
                )?;
                self.recorder = RecorderState::Paused;
            }
            BackendEvent::RecorderResumed { intent_id } => {
                require(self.recorder == RecorderState::Paused)?;
                self.consume_intent(
                    WorkflowArea::Recorder,
                    &intent_id,
                    &[IntentKind::RecorderResume],
                )?;
                self.recorder = RecorderState::Recording;
            }
            BackendEvent::RecorderStopped {
                intent_id,
                recoverable,
            } => {
                require(matches!(
                    self.recorder,
                    RecorderState::Recording | RecorderState::Paused
                ))?;
                self.consume_intent(
                    WorkflowArea::Recorder,
                    &intent_id,
                    &[IntentKind::RecorderStop],
                )?;
                self.recorder = if recoverable {
                    RecorderState::Recoverable
                } else {
                    RecorderState::Ready
                };
            }
            BackendEvent::RecorderCancelled { intent_id } => {
                self.consume_intent(
                    WorkflowArea::Recorder,
                    &intent_id,
                    &[IntentKind::RecorderCancel],
                )?;
                self.recorder = RecorderState::Idle;
            }
            BackendEvent::RecorderFailed { code, retryable } => {
                self.supersede_area(WorkflowArea::Recorder);
                self.recorder = RecorderState::Failed { code, retryable };
            }
            BackendEvent::DevicesEnumerating { intent_id } => {
                self.consume_intent(
                    WorkflowArea::Devices,
                    &intent_id,
                    &[IntentKind::DevicesRefresh],
                )?;
                self.devices = DeviceState::Enumerating;
            }
            BackendEvent::DevicesReady { counts } => {
                require(self.devices == DeviceState::Enumerating)?;
                self.devices = DeviceState::Ready(counts);
            }
            BackendEvent::DeviceSelected { intent_id } => {
                require(matches!(self.devices, DeviceState::Ready(_)))?;
                self.consume_intent(
                    WorkflowArea::Devices,
                    &intent_id,
                    &[IntentKind::DeviceSelect],
                )?;
            }
            BackendEvent::DevicePermissionDenied => {
                self.supersede_area(WorkflowArea::Devices);
                self.devices = DeviceState::PermissionDenied;
                if matches!(
                    self.recorder,
                    RecorderState::Preparing | RecorderState::Recording
                ) {
                    self.supersede_area(WorkflowArea::Recorder);
                    self.recorder = RecorderState::Failed {
                        code: SafeFailureCode::PermissionDenied,
                        retryable: true,
                    };
                }
            }
            BackendEvent::DeviceLost => {
                self.supersede_area(WorkflowArea::Devices);
                self.devices = DeviceState::Unavailable;
                if matches!(
                    self.recorder,
                    RecorderState::Recording | RecorderState::Paused
                ) {
                    self.supersede_area(WorkflowArea::Recorder);
                    self.recorder = RecorderState::Recoverable;
                }
            }
            BackendEvent::RecoveryScanning { intent_id } => {
                self.consume_intent(
                    WorkflowArea::Recovery,
                    &intent_id,
                    &[IntentKind::RecoveryScan],
                )?;
                self.recovery = RecoveryState::Scanning;
            }
            BackendEvent::RecoveryAvailable { projects } => {
                require(self.recovery == RecoveryState::Scanning)?;
                self.recovery = if projects == 0 {
                    RecoveryState::Hidden
                } else {
                    RecoveryState::Available { projects }
                };
            }
            BackendEvent::RecoveryOpening { intent_id } => {
                self.consume_intent(
                    WorkflowArea::Recovery,
                    &intent_id,
                    &[IntentKind::RecoveryOpen],
                )?;
                self.recovery = RecoveryState::Opening;
            }
            BackendEvent::RecoveryOpened => {
                require(self.recovery == RecoveryState::Opening)?;
                self.recovery = RecoveryState::Opened;
            }
            BackendEvent::RecoveryDiscarded {
                intent_id,
                remaining,
            } => {
                self.consume_intent(
                    WorkflowArea::Recovery,
                    &intent_id,
                    &[IntentKind::RecoveryDiscard],
                )?;
                self.recovery = if remaining == 0 {
                    RecoveryState::Hidden
                } else {
                    RecoveryState::Available {
                        projects: remaining,
                    }
                };
            }
            BackendEvent::RecoveryFailed { code } => {
                self.supersede_area(WorkflowArea::Recovery);
                self.recovery = RecoveryState::Failed { code };
            }
            BackendEvent::EditorLoading { intent_id } => {
                self.consume_intent(WorkflowArea::Editor, &intent_id, &[IntentKind::EditorOpen])?;
                self.editor = EditorState::Loading;
                self.editor_operation_failure = None;
            }
            BackendEvent::EditorLoaded {
                revision,
                duration_ms,
            } => {
                require(self.editor == EditorState::Loading && revision > 0 && duration_ms > 0)?;
                self.editor = EditorState::Ready {
                    revision,
                    duration_ms,
                    dirty: false,
                };
                self.editor_operation_failure = None;
            }
            BackendEvent::EditorApplied {
                intent_id,
                revision,
            } => {
                let pending = self.pending_intent_kind(WorkflowArea::Editor, &intent_id)?;
                let IntentKind::EditorApply { base_revision } = pending else {
                    return Err(WorkflowError::IntentMismatch);
                };
                let EditorState::Ready {
                    revision: current,
                    duration_ms,
                    ..
                } = self.editor
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                require(current == base_revision && revision == current.saturating_add(1))?;
                self.pending.remove(&WorkflowArea::Editor);
                self.editor = EditorState::Ready {
                    revision,
                    duration_ms,
                    dirty: true,
                };
                self.editor_operation_failure = None;
            }
            BackendEvent::EditorSaved {
                intent_id,
                revision,
            } => {
                let pending = self.pending_intent_kind(WorkflowArea::Editor, &intent_id)?;
                let IntentKind::EditorSave { expected_revision } = pending else {
                    return Err(WorkflowError::IntentMismatch);
                };
                let EditorState::Ready {
                    revision: current,
                    duration_ms,
                    dirty: true,
                } = self.editor
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                require(current == expected_revision && revision == current)?;
                self.pending.remove(&WorkflowArea::Editor);
                self.editor = EditorState::Ready {
                    revision,
                    duration_ms,
                    dirty: false,
                };
                self.editor_operation_failure = None;
            }
            BackendEvent::EditorFailed { code } => {
                let operation =
                    self.pending
                        .get(&WorkflowArea::Editor)
                        .and_then(|intent| match intent.kind {
                            IntentKind::EditorApply { .. } => Some(EditorOperation::Apply),
                            IntentKind::EditorSave { .. } => Some(EditorOperation::Save),
                            _ => None,
                        });
                self.supersede_area(WorkflowArea::Editor);
                if let Some(operation) = operation
                    && matches!(self.editor, EditorState::Ready { .. })
                {
                    // An edit or save operation can fail without invalidating the
                    // last backend-confirmed project revision. Preserve that
                    // revision (and its dirty bit) so the user can recover, save
                    // elsewhere, or retry after resolving the safe failure.
                    self.editor_operation_failure =
                        Some(EditorOperationFailure { operation, code });
                } else {
                    self.editor = EditorState::Failed { code };
                    self.editor_operation_failure = None;
                }
            }
            BackendEvent::ExportStarted {
                intent_id,
                project_revision,
            } => {
                let pending = self.pending_intent_kind(WorkflowArea::Export, &intent_id)?;
                require(
                    pending == IntentKind::ExportStart { project_revision }
                        || pending
                            == IntentKind::CaptureExportStart {
                                artifact_revision: project_revision,
                            },
                )?;
                self.pending.remove(&WorkflowArea::Export);
                self.export = ExportState::Running {
                    project_revision,
                    progress_basis_points: 0,
                };
            }
            BackendEvent::ExportProgress {
                progress_basis_points,
            } => {
                let ExportState::Running {
                    project_revision,
                    progress_basis_points: current,
                } = self.export
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                require(progress_basis_points <= 10_000 && progress_basis_points >= current)?;
                self.export = ExportState::Running {
                    project_revision,
                    progress_basis_points,
                };
            }
            BackendEvent::ExportCancelling { intent_id } => {
                let ExportState::Running {
                    project_revision, ..
                } = self.export
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                self.consume_intent(
                    WorkflowArea::Export,
                    &intent_id,
                    &[IntentKind::ExportCancel],
                )?;
                self.export = ExportState::Cancelling { project_revision };
            }
            BackendEvent::ExportCompleted => {
                let ExportState::Running {
                    project_revision,
                    progress_basis_points: 10_000,
                } = self.export
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                self.export = ExportState::Completed { project_revision };
            }
            BackendEvent::ExportCancelled => {
                require(matches!(self.export, ExportState::Cancelling { .. }))?;
                self.export = ExportState::Idle;
            }
            BackendEvent::ExportFailed { code, retryable } => {
                self.supersede_area(WorkflowArea::Export);
                self.export = ExportState::Failed { code, retryable };
            }
            BackendEvent::UploadStarted {
                intent_id,
                total_parts,
            } => {
                require(total_parts > 0)?;
                self.consume_intent(WorkflowArea::Upload, &intent_id, &[IntentKind::UploadStart])?;
                self.upload = UploadState::Uploading {
                    verified_parts: 0,
                    total_parts,
                };
            }
            BackendEvent::UploadProgress {
                verified_parts,
                total_parts,
            } => {
                let (current, expected_total) = upload_progress(self.upload)?;
                require(
                    total_parts == expected_total
                        && verified_parts >= current
                        && verified_parts <= total_parts,
                )?;
                self.upload = UploadState::Uploading {
                    verified_parts,
                    total_parts,
                };
            }
            BackendEvent::UploadPaused { intent_id } => {
                let (verified_parts, total_parts) = match self.upload {
                    UploadState::Uploading {
                        verified_parts,
                        total_parts,
                    }
                    | UploadState::Paused {
                        verified_parts,
                        total_parts,
                        ..
                    } => (verified_parts, total_parts),
                    _ => return Err(WorkflowError::InvalidBackendTransition),
                };
                self.consume_intent(WorkflowArea::Upload, &intent_id, &[IntentKind::UploadPause])?;
                self.upload = UploadState::Paused {
                    verified_parts,
                    total_parts,
                    reason: UploadPauseReason::User,
                };
            }
            BackendEvent::UploadOffline => {
                let UploadState::Uploading {
                    verified_parts,
                    total_parts,
                } = self.upload
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                self.supersede_if(WorkflowArea::Upload, IntentKind::UploadPause);
                self.upload = UploadState::Paused {
                    verified_parts,
                    total_parts,
                    reason: UploadPauseReason::Offline,
                };
            }
            BackendEvent::UploadResumed { intent_id } => {
                let UploadState::Paused {
                    verified_parts,
                    total_parts,
                    ..
                } = self.upload
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                self.consume_intent(
                    WorkflowArea::Upload,
                    &intent_id,
                    &[IntentKind::UploadResume],
                )?;
                self.upload = UploadState::Uploading {
                    verified_parts,
                    total_parts,
                };
            }
            BackendEvent::UploadFinalizing => {
                let UploadState::Uploading {
                    verified_parts,
                    total_parts,
                } = self.upload
                else {
                    return Err(WorkflowError::InvalidBackendTransition);
                };
                require(verified_parts == total_parts)?;
                self.upload = UploadState::Finalizing;
            }
            BackendEvent::UploadCompleted => {
                require(self.upload == UploadState::Finalizing)?;
                self.upload = UploadState::Completed;
            }
            BackendEvent::UploadCancelled { intent_id } => {
                self.consume_intent(
                    WorkflowArea::Upload,
                    &intent_id,
                    &[IntentKind::UploadCancel],
                )?;
                self.upload = UploadState::Idle;
            }
            BackendEvent::UploadFailed { code, retryable } => {
                self.supersede_area(WorkflowArea::Upload);
                self.upload = UploadState::Failed { code, retryable };
            }
        }
        Ok(())
    }

    fn pending_intent_kind(
        &self,
        area: WorkflowArea,
        intent_id: &str,
    ) -> Result<IntentKind, WorkflowError> {
        let pending = self
            .pending
            .get(&area)
            .ok_or(WorkflowError::NoPendingIntent)?;
        if pending.id() != intent_id {
            return Err(WorkflowError::IntentMismatch);
        }
        Ok(pending.kind)
    }

    fn consume_intent(
        &mut self,
        area: WorkflowArea,
        intent_id: &str,
        allowed: &[IntentKind],
    ) -> Result<IntentKind, WorkflowError> {
        let kind = self.pending_intent_kind(area, intent_id)?;
        if !allowed.contains(&kind) {
            return Err(WorkflowError::IntentMismatch);
        }
        self.pending.remove(&area);
        Ok(kind)
    }

    fn supersede_area(&mut self, area: WorkflowArea) {
        if let Some(intent) = self.pending.remove(&area) {
            if self.superseded_intents.len() == 128 {
                self.superseded_intents.pop_front();
            }
            self.superseded_intents.push_back(intent.id);
        }
    }

    fn supersede_if(&mut self, area: WorkflowArea, kind: IntentKind) {
        if self
            .pending
            .get(&area)
            .is_some_and(|intent| intent.kind == kind)
        {
            self.supersede_area(area);
        }
    }

    fn consume_superseded(&mut self, intent_id: &str) -> bool {
        let Some(index) = self
            .superseded_intents
            .iter()
            .position(|candidate| candidate == intent_id)
        else {
            return false;
        };
        self.superseded_intents.remove(index);
        true
    }
}

fn upload_progress(state: UploadState) -> Result<(u32, u32), WorkflowError> {
    match state {
        UploadState::Uploading {
            verified_parts,
            total_parts,
        } => Ok((verified_parts, total_parts)),
        _ => Err(WorkflowError::InvalidBackendTransition),
    }
}

fn require(condition: bool) -> Result<(), WorkflowError> {
    if condition {
        Ok(())
    } else {
        Err(WorkflowError::InvalidBackendTransition)
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowError {
    #[error("UI intent identifier is invalid")]
    InvalidIntent,
    #[error("workflow area {0:?} already has a pending intent")]
    Busy(WorkflowArea),
    #[error("UI intent is invalid for the confirmed backend state in {0:?}")]
    InvalidIntentForState(WorkflowArea),
    #[error("workflow protocol version is unsupported")]
    UnsupportedProtocol,
    #[error("workflow session does not match")]
    SessionMismatch,
    #[error("backend event is stale or replayed")]
    StaleBackendEvent,
    #[error("backend event sequence has a gap")]
    BackendSequenceGap,
    #[error("backend event sequence overflowed")]
    SequenceOverflow,
    #[error("backend transition is invalid")]
    InvalidBackendTransition,
    #[error("no matching UI intent is pending")]
    NoPendingIntent,
    #[error("backend response does not match the pending UI intent")]
    IntentMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionId {
        SessionId::new("desktop-session-1").expect("session")
    }

    fn event(sequence: u64, event: BackendEvent) -> BackendEventEnvelope {
        BackendEventEnvelope {
            protocol_version: WORKFLOW_PROTOCOL_VERSION,
            session_id: session(),
            sequence,
            event,
        }
    }

    fn intent(id: &str, kind: IntentKind) -> UiIntent {
        UiIntent::new(id, kind).expect("intent")
    }

    #[test]
    fn recorder_ui_waits_for_backend_truth() {
        let mut model = DesktopWorkflow::new(session());
        model
            .request(intent("start-1", IntentKind::RecorderStart))
            .expect("request");
        assert_eq!(model.recorder(), RecorderState::Idle);
        assert!(model.pending_intent(WorkflowArea::Recorder).is_some());

        model
            .apply_backend(event(
                1,
                BackendEvent::RecorderPreparing {
                    intent_id: "start-1".into(),
                },
            ))
            .expect("preparing");
        assert_eq!(model.recorder(), RecorderState::Preparing);
        model
            .apply_backend(event(2, BackendEvent::RecorderStarted))
            .expect("started");
        assert_eq!(model.recorder(), RecorderState::Recording);
    }

    #[test]
    fn stale_or_gapped_backend_events_do_not_mutate_state() {
        let mut model = DesktopWorkflow::new(session());
        assert_eq!(
            model.apply_backend(event(2, BackendEvent::RecorderStarted)),
            Err(WorkflowError::BackendSequenceGap)
        );
        assert_eq!(model.last_backend_sequence(), 0);
        assert_eq!(model.recorder(), RecorderState::Idle);
    }

    #[test]
    fn device_loss_overrides_pending_recorder_action_truthfully() {
        let mut model = DesktopWorkflow::new(session());
        model
            .request(intent("start-1", IntentKind::RecorderStart))
            .expect("start request");
        model
            .apply_backend(event(
                1,
                BackendEvent::RecorderPreparing {
                    intent_id: "start-1".into(),
                },
            ))
            .expect("preparing");
        model
            .apply_backend(event(2, BackendEvent::RecorderStarted))
            .expect("started");
        model
            .request(intent("pause-1", IntentKind::RecorderPause))
            .expect("pause request");
        model
            .apply_backend(event(3, BackendEvent::DeviceLost))
            .expect("lost");
        assert_eq!(model.recorder(), RecorderState::Recoverable);
        assert!(model.pending_intent(WorkflowArea::Recorder).is_none());
        model
            .apply_backend(event(
                4,
                BackendEvent::RecorderPaused {
                    intent_id: "pause-1".into(),
                },
            ))
            .expect("late superseded response");
        assert_eq!(model.recorder(), RecorderState::Recoverable);
    }

    #[test]
    fn stale_editor_revision_is_rejected_before_intent_is_pending() {
        let mut model = DesktopWorkflow::new(session());
        model.editor = EditorState::Ready {
            revision: 4,
            duration_ms: 1_000,
            dirty: false,
        };
        assert_eq!(
            model.request(intent(
                "edit-1",
                IntentKind::EditorApply { base_revision: 3 }
            )),
            Err(WorkflowError::InvalidIntentForState(WorkflowArea::Editor))
        );
        assert!(model.pending_intent(WorkflowArea::Editor).is_none());
    }

    #[test]
    fn export_progress_regression_is_atomic() {
        let mut model = DesktopWorkflow::new(session());
        model.export = ExportState::Running {
            project_revision: 7,
            progress_basis_points: 5_000,
        };
        assert_eq!(
            model.apply_backend(event(
                1,
                BackendEvent::ExportProgress {
                    progress_basis_points: 4_000
                }
            )),
            Err(WorkflowError::InvalidBackendTransition)
        );
        assert_eq!(
            model.export(),
            ExportState::Running {
                project_revision: 7,
                progress_basis_points: 5_000
            }
        );
        assert_eq!(model.last_backend_sequence(), 0);
    }

    #[test]
    fn upload_resume_preserves_verified_parts() {
        let mut model = DesktopWorkflow::new(session());
        model.upload = UploadState::Uploading {
            verified_parts: 2,
            total_parts: 5,
        };
        model
            .apply_backend(event(1, BackendEvent::UploadOffline))
            .expect("offline");
        model
            .request(intent("resume-1", IntentKind::UploadResume))
            .expect("resume intent");
        model
            .apply_backend(event(
                2,
                BackendEvent::UploadResumed {
                    intent_id: "resume-1".into(),
                },
            ))
            .expect("resumed");
        assert_eq!(
            model.upload(),
            UploadState::Uploading {
                verified_parts: 2,
                total_parts: 5
            }
        );
    }

    #[test]
    fn user_upload_pause_is_backend_confirmed() {
        let mut model = DesktopWorkflow::new(session());
        model.upload = UploadState::Uploading {
            verified_parts: 3,
            total_parts: 8,
        };
        model
            .request(intent("pause-upload", IntentKind::UploadPause))
            .expect("pause intent");
        assert!(matches!(model.upload(), UploadState::Uploading { .. }));
        model
            .apply_backend(event(
                1,
                BackendEvent::UploadPaused {
                    intent_id: "pause-upload".into(),
                },
            ))
            .expect("paused");
        assert_eq!(
            model.upload(),
            UploadState::Paused {
                verified_parts: 3,
                total_parts: 8,
                reason: UploadPauseReason::User,
            }
        );
    }

    #[test]
    fn workflow_debug_redacts_session_and_intent_ids() {
        let mut model = DesktopWorkflow::new(session());
        model
            .request(intent("private-intent", IntentKind::RecorderStart))
            .expect("request");
        let rendered = format!("{model:?}");
        assert!(!rendered.contains("desktop-session-1"));
        assert!(!rendered.contains("private-intent"));

        let event = BackendEvent::RecorderPreparing {
            intent_id: "private-backend-intent".into(),
        };
        assert!(!format!("{event:?}").contains("private-backend-intent"));
    }
}
