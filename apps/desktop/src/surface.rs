//! Backend-truth-driven product surface for the desktop recorder and editor.
//!
//! A Leptos/Tauri renderer can treat this module as its UI port: it renders the
//! returned accessibility model, forwards keyboard and focus events, executes
//! scoped commands, and applies scoped backend events. No project paths,
//! tenant identifiers, or backend error strings are copied into render state.

use std::fmt;

use thiserror::Error;

use crate::{
    accessibility::{
        ACCESSIBILITY_MODEL_VERSION, AccessibilityModel, Control, ControlRole, DialogFocusModel,
        Key, KeyChord, KeyboardAction, Landmark, LandmarkRole, LiveRegion, Modifiers, Shortcut,
    },
    ipc::{EditorMutation, SessionId},
    workflow::{
        BackendEvent, BackendEventEnvelope, DesktopWorkflow, EditorOperation, EditorState,
        ExportState, IntentKind, RecorderState, SafeFailureCode, UiIntent, WorkflowArea,
        WorkflowError,
    },
};

pub const DESKTOP_SURFACE_VERSION: u16 = 1;

const RECORDER_TAB: &str = "nav-recorder";
const EDITOR_TAB: &str = "nav-editor";
const RECORDER_PRIMARY: &str = "recorder-primary";
const RECORDER_PAUSE: &str = "recorder-pause";
const RECORDER_CANCEL: &str = "recorder-cancel";
const EDITOR_OPEN: &str = "editor-open";
const SELECTION_START: &str = "selection-start";
const SELECTION_END: &str = "selection-end";
const EDITOR_APPLY: &str = "editor-apply-selection";
const EDITOR_SAVE: &str = "editor-save";
const EDITOR_EXPORT: &str = "editor-export";
const EXPORT_PROGRESS: &str = "export-progress";
const EXPORT_CANCEL: &str = "export-cancel";
const FAILURE_RETRY: &str = "failure-retry";
const FAILURE_DISMISS: &str = "failure-dismiss";

fn valid_private_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
}

macro_rules! private_identifier {
    ($name:ident, $label:literal) => {
        #[derive(Clone, PartialEq, Eq)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, SurfaceError> {
                let value = value.into();
                if !valid_private_identifier(&value) {
                    return Err(SurfaceError::InvalidPrivateIdentifier);
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($label, "(<redacted>)"))
            }
        }
    };
}

private_identifier!(TenantId, "TenantId");
private_identifier!(ProjectHandle, "ProjectHandle");
private_identifier!(ExportDestinationHandle, "ExportDestinationHandle");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopScreen {
    Recorder,
    Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutPlatform {
    MacOs,
    WindowsOrLinux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionHandle {
    Start,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorSelection {
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
}

impl EditorSelection {
    fn full_duration(duration_ms: u64) -> Result<Self, SurfaceError> {
        if duration_ms == 0 {
            return Err(SurfaceError::SelectionUnavailable);
        }
        Ok(Self {
            start_ms: 0,
            end_ms: duration_ms,
            duration_ms,
        })
    }

    fn adjust(&mut self, handle: SelectionHandle, delta_ms: i64) {
        match handle {
            SelectionHandle::Start => {
                self.start_ms = offset(self.start_ms, delta_ms).min(self.end_ms.saturating_sub(1));
            }
            SelectionHandle::End => {
                self.end_ms = offset(self.end_ms, delta_ms)
                    .max(self.start_ms.saturating_add(1))
                    .min(self.duration_ms);
            }
        }
    }

    fn is_full_duration(self) -> bool {
        self.start_ms == 0 && self.end_ms == self.duration_ms
    }
}

fn offset(value: u64, delta: i64) -> u64 {
    if delta.is_negative() {
        value.saturating_sub(delta.unsigned_abs())
    } else {
        value.saturating_add(delta.unsigned_abs())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum SurfaceCommand {
    RecorderStart,
    RecorderPause,
    RecorderResume,
    RecorderStop,
    RecorderCancel,
    RecorderRecover,
    EditorOpen {
        project: ProjectHandle,
    },
    EditorApply {
        base_revision: u64,
        mutation: EditorMutation,
    },
    EditorSave {
        expected_revision: u64,
    },
    ExportStart {
        project_revision: u64,
        destination: ExportDestinationHandle,
    },
    ExportCancel,
}

impl fmt::Debug for SurfaceCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecorderStart => formatter.write_str("RecorderStart"),
            Self::RecorderPause => formatter.write_str("RecorderPause"),
            Self::RecorderResume => formatter.write_str("RecorderResume"),
            Self::RecorderStop => formatter.write_str("RecorderStop"),
            Self::RecorderCancel => formatter.write_str("RecorderCancel"),
            Self::RecorderRecover => formatter.write_str("RecorderRecover"),
            Self::EditorOpen { project } => formatter
                .debug_struct("EditorOpen")
                .field("project", project)
                .finish(),
            Self::EditorApply {
                base_revision,
                mutation,
            } => formatter
                .debug_struct("EditorApply")
                .field("base_revision", base_revision)
                .field("mutation", mutation)
                .finish(),
            Self::EditorSave { expected_revision } => formatter
                .debug_struct("EditorSave")
                .field("expected_revision", expected_revision)
                .finish(),
            Self::ExportStart {
                project_revision,
                destination,
            } => formatter
                .debug_struct("ExportStart")
                .field("project_revision", project_revision)
                .field("destination", destination)
                .finish(),
            Self::ExportCancel => formatter.write_str("ExportCancel"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ScopedSurfaceCommand {
    tenant_id: TenantId,
    intent: UiIntent,
    command: SurfaceCommand,
}

impl ScopedSurfaceCommand {
    #[must_use]
    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    #[must_use]
    pub fn intent(&self) -> &UiIntent {
        &self.intent
    }

    #[must_use]
    pub fn command(&self) -> &SurfaceCommand {
        &self.command
    }
}

impl fmt::Debug for ScopedSurfaceCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedSurfaceCommand")
            .field("tenant_id", &"<redacted>")
            .field("intent", &self.intent)
            .field("command", &self.command)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ScopedBackendEvent {
    tenant_id: TenantId,
    envelope: BackendEventEnvelope,
}

impl ScopedBackendEvent {
    #[must_use]
    pub fn new(tenant_id: TenantId, envelope: BackendEventEnvelope) -> Self {
        Self {
            tenant_id,
            envelope,
        }
    }
}

impl fmt::Debug for ScopedBackendEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedBackendEvent")
            .field("tenant_id", &"<redacted>")
            .field("protocol_version", &self.envelope.protocol_version)
            .field("sequence", &self.envelope.sequence)
            .field("event", &self.envelope.event)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOperation {
    OpenProject,
    ApplyEdit,
    SaveProject,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceFailure {
    pub operation: FailureOperation,
    pub code: SafeFailureCode,
    pub retryable: bool,
}

impl SurfaceFailure {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self.operation {
            FailureOperation::OpenProject => {
                "The project could not be opened. The source project was not changed."
            }
            FailureOperation::ApplyEdit => {
                "The edit could not be applied. The last confirmed project revision is unchanged."
            }
            FailureOperation::SaveProject => {
                "The project could not be saved. Unsaved edits are still present."
            }
            FailureOperation::Export => {
                "The export could not be completed. The project was not changed."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceEffect {
    Command(ScopedSurfaceCommand),
    FocusChanged { control_id: String },
    SelectionChanged(EditorSelection),
    ChooseProject,
    ChooseExportDestination { project_revision: u64 },
    FailureDismissed,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceSnapshot {
    pub version: u16,
    pub screen: DesktopScreen,
    pub focused_control_id: Option<String>,
    pub announcement: String,
    pub recorder: RecorderState,
    pub editor: EditorState,
    pub export: ExportState,
    pub selection: Option<EditorSelection>,
    pub failure: Option<SurfaceFailure>,
}

#[derive(Clone)]
pub struct DesktopProductSurface {
    tenant_id: TenantId,
    platform: ShortcutPlatform,
    workflow: DesktopWorkflow,
    screen: DesktopScreen,
    focused_control_id: Option<String>,
    selection: Option<EditorSelection>,
    failure: Option<SurfaceFailure>,
    failure_restore_focus_id: Option<String>,
    announcement: String,
    next_intent_sequence: u64,
}

impl fmt::Debug for DesktopProductSurface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DesktopProductSurface")
            .field("tenant_id", &"<redacted>")
            .field("snapshot", &self.snapshot())
            .field("workflow", &self.workflow)
            .finish()
    }
}

impl DesktopProductSurface {
    #[must_use]
    pub fn new(tenant_id: TenantId, session_id: SessionId, platform: ShortcutPlatform) -> Self {
        Self {
            tenant_id,
            platform,
            workflow: DesktopWorkflow::new(session_id),
            screen: DesktopScreen::Recorder,
            focused_control_id: Some(RECORDER_PRIMARY.into()),
            selection: None,
            failure: None,
            failure_restore_focus_id: None,
            announcement: "Recorder ready.".into(),
            next_intent_sequence: 0,
        }
    }

    #[must_use]
    pub fn workflow(&self) -> &DesktopWorkflow {
        &self.workflow
    }

    #[must_use]
    pub fn snapshot(&self) -> SurfaceSnapshot {
        SurfaceSnapshot {
            version: DESKTOP_SURFACE_VERSION,
            screen: self.screen,
            focused_control_id: self.focused_control_id.clone(),
            announcement: self.announcement.clone(),
            recorder: self.workflow.recorder(),
            editor: self.workflow.editor(),
            export: self.workflow.export(),
            selection: self.selection,
            failure: self.failure,
        }
    }

    #[must_use]
    pub fn accessibility_model(&self) -> AccessibilityModel {
        let mut controls = Vec::new();
        let mut order = 1;
        push_control(
            &mut controls,
            &mut order,
            ControlSpec::button(
                RECORDER_TAB,
                "Recorder",
                true,
                Some(KeyboardAction::FocusRecorder),
            )
            .role(ControlRole::Tab),
        );
        push_control(
            &mut controls,
            &mut order,
            ControlSpec::button(
                EDITOR_TAB,
                "Editor",
                true,
                Some(KeyboardAction::FocusEditor),
            )
            .role(ControlRole::Tab),
        );

        match self.screen {
            DesktopScreen::Recorder => self.recorder_controls(&mut controls, &mut order),
            DesktopScreen::Editor => self.editor_controls(&mut controls, &mut order),
        }

        let dialog = self.failure.map(|failure| {
            for control in &mut controls {
                control.disabled = true;
                control.focus_order = None;
            }
            if failure.retryable {
                push_control(
                    &mut controls,
                    &mut order,
                    ControlSpec::button(
                        FAILURE_RETRY,
                        "Retry operation",
                        true,
                        Some(KeyboardAction::RetryLastOperation),
                    ),
                );
            }
            push_control(
                &mut controls,
                &mut order,
                ControlSpec::button(
                    FAILURE_DISMISS,
                    "Dismiss error",
                    true,
                    Some(KeyboardAction::DismissDialog),
                ),
            );
            DialogFocusModel {
                open: true,
                focus_trapped: true,
                initial_focus_id: Some(
                    if failure.retryable {
                        FAILURE_RETRY
                    } else {
                        FAILURE_DISMISS
                    }
                    .into(),
                ),
                restore_focus_id: self
                    .failure_restore_focus_id
                    .clone()
                    .or_else(|| Some(active_tab(self.screen).into())),
                dismissible: true,
            }
        });

        let mut landmarks = vec![
            Landmark {
                id: "desktop-navigation".into(),
                role: LandmarkRole::Navigation,
                label: "Desktop workspace".into(),
                live: LiveRegion::Off,
            },
            Landmark {
                id: "desktop-main".into(),
                role: LandmarkRole::Main,
                label: match self.screen {
                    DesktopScreen::Recorder => "Recorder",
                    DesktopScreen::Editor => "Editor",
                }
                .into(),
                live: LiveRegion::Off,
            },
            Landmark {
                id: "operation-status".into(),
                role: LandmarkRole::Status,
                label: "Operation status".into(),
                live: if self.failure.is_some() {
                    LiveRegion::Assertive
                } else {
                    LiveRegion::Polite
                },
            },
        ];
        if self.failure.is_some() {
            landmarks.push(Landmark {
                id: "failure-dialog".into(),
                role: LandmarkRole::Dialog,
                label: "Operation failed".into(),
                live: LiveRegion::Assertive,
            });
        }

        AccessibilityModel {
            version: ACCESSIBILITY_MODEL_VERSION,
            landmarks,
            controls,
            shortcuts: shortcuts(self.platform),
            dialog,
        }
    }

    fn recorder_controls(&self, controls: &mut Vec<Control>, order: &mut u16) {
        let pending = self
            .workflow
            .pending_intent(WorkflowArea::Recorder)
            .is_some();
        let (primary_name, primary_enabled, primary_action) = match self.workflow.recorder() {
            RecorderState::Idle | RecorderState::Ready => (
                "Start recording",
                !pending,
                Some(KeyboardAction::StartStopRecording),
            ),
            RecorderState::Preparing => ("Starting recording", false, None),
            RecorderState::Recording | RecorderState::Paused => (
                "Stop recording",
                !pending,
                Some(KeyboardAction::StartStopRecording),
            ),
            RecorderState::Recoverable => (
                "Recover recording",
                !pending,
                Some(KeyboardAction::OpenRecovery),
            ),
            RecorderState::Failed {
                retryable: true, ..
            } => (
                "Retry recording",
                !pending,
                Some(KeyboardAction::StartStopRecording),
            ),
            RecorderState::Failed {
                retryable: false, ..
            } => ("Recording unavailable", false, None),
        };
        push_control(
            controls,
            order,
            ControlSpec::button(
                RECORDER_PRIMARY,
                primary_name,
                primary_enabled,
                primary_action,
            ),
        );

        let can_pause = matches!(
            self.workflow.recorder(),
            RecorderState::Recording | RecorderState::Paused
        );
        let pause_name = if self.workflow.recorder() == RecorderState::Paused {
            "Resume recording"
        } else {
            "Pause recording"
        };
        push_control(
            controls,
            order,
            ControlSpec::button(
                RECORDER_PAUSE,
                pause_name,
                can_pause && !pending,
                Some(KeyboardAction::PauseResumeRecording),
            )
            .visible(can_pause),
        );
        let can_cancel = matches!(
            self.workflow.recorder(),
            RecorderState::Preparing
                | RecorderState::Recording
                | RecorderState::Paused
                | RecorderState::Recoverable
        );
        push_control(
            controls,
            order,
            ControlSpec::button(
                RECORDER_CANCEL,
                "Cancel recording",
                can_cancel && !pending,
                Some(KeyboardAction::Cancel),
            )
            .visible(can_cancel),
        );
    }

    fn editor_controls(&self, controls: &mut Vec<Control>, order: &mut u16) {
        let editor_pending = self.workflow.pending_intent(WorkflowArea::Editor).is_some();
        match self.workflow.editor() {
            EditorState::Closed | EditorState::Failed { .. } => push_control(
                controls,
                order,
                ControlSpec::button(
                    EDITOR_OPEN,
                    "Open project",
                    !editor_pending,
                    Some(KeyboardAction::OpenProject),
                ),
            ),
            EditorState::Loading => push_control(
                controls,
                order,
                ControlSpec::progress(EDITOR_OPEN, "Loading project", "Loading"),
            ),
            EditorState::Ready {
                revision,
                duration_ms,
                dirty,
            } => {
                let selection = self.selection.unwrap_or(EditorSelection {
                    start_ms: 0,
                    end_ms: duration_ms,
                    duration_ms,
                });
                push_control(
                    controls,
                    order,
                    ControlSpec::slider(
                        SELECTION_START,
                        "Selection start",
                        format_time(selection.start_ms),
                        !editor_pending,
                    ),
                );
                push_control(
                    controls,
                    order,
                    ControlSpec::slider(
                        SELECTION_END,
                        "Selection end",
                        format_time(selection.end_ms),
                        !editor_pending,
                    ),
                );
                push_control(
                    controls,
                    order,
                    ControlSpec::button(
                        EDITOR_APPLY,
                        "Trim to selection",
                        !editor_pending && !selection.is_full_duration(),
                        Some(KeyboardAction::ApplySelection),
                    ),
                );
                push_control(
                    controls,
                    order,
                    ControlSpec::button(
                        EDITOR_SAVE,
                        if dirty {
                            "Save project"
                        } else {
                            "Project saved"
                        },
                        dirty && !editor_pending,
                        Some(KeyboardAction::SaveProject),
                    ),
                );
                let export_available = !dirty
                    && !editor_pending
                    && self.workflow.pending_intent(WorkflowArea::Export).is_none()
                    && matches!(
                        self.workflow.export(),
                        ExportState::Idle
                            | ExportState::Completed { .. }
                            | ExportState::Failed {
                                retryable: true,
                                ..
                            }
                    );
                push_control(
                    controls,
                    order,
                    ControlSpec::button(
                        EDITOR_EXPORT,
                        "Export project",
                        export_available,
                        Some(KeyboardAction::Export),
                    ),
                );

                match self.workflow.export() {
                    ExportState::Running {
                        progress_basis_points,
                        ..
                    } => {
                        push_control(
                            controls,
                            order,
                            ControlSpec::progress(
                                EXPORT_PROGRESS,
                                "Export progress",
                                &format!("{} percent", u32::from(progress_basis_points) / 100),
                            ),
                        );
                        push_control(
                            controls,
                            order,
                            ControlSpec::button(
                                EXPORT_CANCEL,
                                "Cancel export",
                                self.workflow.pending_intent(WorkflowArea::Export).is_none(),
                                Some(KeyboardAction::Cancel),
                            ),
                        );
                    }
                    ExportState::Cancelling { .. } => push_control(
                        controls,
                        order,
                        ControlSpec::progress(EXPORT_PROGRESS, "Export cancellation", "Cancelling"),
                    ),
                    _ => {}
                }

                let _ = revision;
            }
        }
    }

    pub fn handle_key(&mut self, chord: KeyChord) -> Result<SurfaceEffect, SurfaceError> {
        let chord = normalize_chord(chord);
        if chord.key == Key::Tab {
            return self.move_focus(chord.modifiers.shift);
        }

        if self.failure.is_some() {
            return match chord.key {
                Key::Escape => self.dismiss_failure(),
                Key::Enter | Key::Space => self.activate_focused(),
                _ => Ok(SurfaceEffect::None),
            };
        }

        if matches!(chord.key, Key::ArrowLeft | Key::ArrowRight)
            && matches!(
                self.focused_control_id.as_deref(),
                Some(SELECTION_START | SELECTION_END)
            )
        {
            let handle = if self.focused_control_id.as_deref() == Some(SELECTION_START) {
                SelectionHandle::Start
            } else {
                SelectionHandle::End
            };
            let magnitude = if chord.modifiers.shift { 10_000 } else { 1_000 };
            let delta = if chord.key == Key::ArrowLeft {
                -magnitude
            } else {
                magnitude
            };
            return self.adjust_selection(handle, delta);
        }

        if matches!(chord.key, Key::Enter | Key::Space) {
            return self.activate_focused();
        }
        if let Some(action) = self.accessibility_model().keyboard_action(chord) {
            return self.perform_action(action);
        }
        Ok(SurfaceEffect::None)
    }

    pub fn accept_focus(&mut self, control_id: &str) -> Result<SurfaceEffect, SurfaceError> {
        if !self.focusable_ids().iter().any(|id| id == control_id) {
            return Err(SurfaceError::ControlNotFocusable);
        }
        self.focused_control_id = Some(control_id.into());
        Ok(SurfaceEffect::FocusChanged {
            control_id: control_id.into(),
        })
    }

    pub fn activate_focused(&mut self) -> Result<SurfaceEffect, SurfaceError> {
        let focused = self
            .focused_control_id
            .as_deref()
            .ok_or(SurfaceError::ControlNotFocusable)?;
        let action = self
            .accessibility_model()
            .controls
            .iter()
            .find(|control| control.id == focused && control.visible && !control.disabled)
            .and_then(|control| control.action)
            .ok_or(SurfaceError::ActionUnavailable)?;
        self.perform_action(action)
    }

    pub fn open_editor(&mut self, project: ProjectHandle) -> Result<SurfaceEffect, SurfaceError> {
        self.queue_command(
            IntentKind::EditorOpen,
            SurfaceCommand::EditorOpen { project },
        )
    }

    pub fn request_export(
        &mut self,
        destination: ExportDestinationHandle,
    ) -> Result<SurfaceEffect, SurfaceError> {
        let EditorState::Ready {
            revision,
            dirty: false,
            ..
        } = self.workflow.editor()
        else {
            return Err(SurfaceError::ActionUnavailable);
        };
        let effect = self.queue_command(
            IntentKind::ExportStart {
                project_revision: revision,
            },
            SurfaceCommand::ExportStart {
                project_revision: revision,
                destination,
            },
        )?;
        self.clear_failure_for_retry();
        Ok(effect)
    }

    pub fn adjust_selection(
        &mut self,
        handle: SelectionHandle,
        delta_ms: i64,
    ) -> Result<SurfaceEffect, SurfaceError> {
        if !matches!(self.workflow.editor(), EditorState::Ready { .. })
            || self.workflow.pending_intent(WorkflowArea::Editor).is_some()
        {
            return Err(SurfaceError::SelectionUnavailable);
        }
        let selection = self
            .selection
            .as_mut()
            .ok_or(SurfaceError::SelectionUnavailable)?;
        selection.adjust(handle, delta_ms);
        self.announcement = match handle {
            SelectionHandle::Start => {
                format!("Selection starts at {}.", format_time(selection.start_ms))
            }
            SelectionHandle::End => {
                format!("Selection ends at {}.", format_time(selection.end_ms))
            }
        };
        Ok(SurfaceEffect::SelectionChanged(*selection))
    }

    pub fn apply_backend(&mut self, scoped: ScopedBackendEvent) -> Result<(), SurfaceError> {
        if scoped.tenant_id != self.tenant_id {
            return Err(SurfaceError::TenantMismatch);
        }
        let event = scoped.envelope.event.clone();
        self.workflow.apply_backend(scoped.envelope)?;
        match event {
            BackendEvent::RecorderPreparing { .. } => {
                self.announcement = "Preparing recording.".into();
            }
            BackendEvent::RecorderStarted => {
                self.announcement = "Recording started.".into();
            }
            BackendEvent::RecorderPaused { .. } => {
                self.announcement = "Recording paused.".into();
            }
            BackendEvent::RecorderResumed { .. } => {
                self.announcement = "Recording resumed.".into();
            }
            BackendEvent::RecorderStopped { recoverable, .. } => {
                self.announcement = if recoverable {
                    "Recording stopped. Recovery is available."
                } else {
                    "Recording stopped."
                }
                .into();
            }
            BackendEvent::RecorderCancelled { .. } => {
                self.announcement = "Recording cancelled.".into();
            }
            BackendEvent::RecorderFailed { .. } => {
                self.announcement =
                    "Recording failed. No private diagnostic details were shown.".into();
            }
            BackendEvent::EditorLoaded { duration_ms, .. } => {
                self.selection = Some(EditorSelection::full_duration(duration_ms)?);
                self.failure = None;
                self.announcement = "Project loaded.".into();
            }
            BackendEvent::EditorApplied { .. } => {
                self.failure = None;
                self.announcement = "Edit applied. Project has unsaved changes.".into();
            }
            BackendEvent::EditorSaved { .. } => {
                self.failure = None;
                self.announcement = "Project saved.".into();
            }
            BackendEvent::EditorFailed { code } => {
                let operation = self
                    .workflow
                    .editor_operation_failure()
                    .map(|failure| match failure.operation {
                        EditorOperation::Apply => FailureOperation::ApplyEdit,
                        EditorOperation::Save => FailureOperation::SaveProject,
                    })
                    .unwrap_or(FailureOperation::OpenProject);
                let retryable = matches!(
                    (operation, code),
                    (
                        FailureOperation::ApplyEdit | FailureOperation::SaveProject,
                        SafeFailureCode::DiskFull | SafeFailureCode::BackendUnavailable
                    )
                );
                self.raise_failure(SurfaceFailure {
                    operation,
                    code,
                    retryable,
                });
            }
            BackendEvent::ExportStarted { .. } => {
                self.failure = None;
                self.announcement = "Export started.".into();
            }
            BackendEvent::ExportProgress {
                progress_basis_points,
            } => {
                self.announcement = format!(
                    "Export {} percent complete.",
                    u32::from(progress_basis_points) / 100
                );
            }
            BackendEvent::ExportCompleted => {
                self.failure = None;
                self.announcement = "Export completed.".into();
            }
            BackendEvent::ExportCancelled => {
                self.failure = None;
                self.announcement = "Export cancelled.".into();
            }
            BackendEvent::ExportFailed { code, retryable } => {
                self.raise_failure(SurfaceFailure {
                    operation: FailureOperation::Export,
                    code,
                    retryable,
                });
            }
            _ => {}
        }
        self.reconcile_focus();
        Ok(())
    }

    fn perform_action(&mut self, action: KeyboardAction) -> Result<SurfaceEffect, SurfaceError> {
        match action {
            KeyboardAction::StartStopRecording => match self.workflow.recorder() {
                RecorderState::Idle
                | RecorderState::Ready
                | RecorderState::Failed {
                    retryable: true, ..
                } => self.queue_command(IntentKind::RecorderStart, SurfaceCommand::RecorderStart),
                RecorderState::Recording | RecorderState::Paused => {
                    self.queue_command(IntentKind::RecorderStop, SurfaceCommand::RecorderStop)
                }
                _ => Err(SurfaceError::ActionUnavailable),
            },
            KeyboardAction::PauseResumeRecording => match self.workflow.recorder() {
                RecorderState::Recording => {
                    self.queue_command(IntentKind::RecorderPause, SurfaceCommand::RecorderPause)
                }
                RecorderState::Paused => {
                    self.queue_command(IntentKind::RecorderResume, SurfaceCommand::RecorderResume)
                }
                _ => Err(SurfaceError::ActionUnavailable),
            },
            KeyboardAction::Cancel => match self.screen {
                DesktopScreen::Recorder => {
                    self.queue_command(IntentKind::RecorderCancel, SurfaceCommand::RecorderCancel)
                }
                DesktopScreen::Editor
                    if matches!(self.workflow.export(), ExportState::Running { .. }) =>
                {
                    self.queue_command(IntentKind::ExportCancel, SurfaceCommand::ExportCancel)
                }
                DesktopScreen::Editor => Err(SurfaceError::ActionUnavailable),
            },
            KeyboardAction::OpenRecovery => {
                self.queue_command(IntentKind::RecorderRecover, SurfaceCommand::RecorderRecover)
            }
            KeyboardAction::OpenProject if self.screen == DesktopScreen::Editor => {
                Ok(SurfaceEffect::ChooseProject)
            }
            KeyboardAction::ApplySelection if self.screen == DesktopScreen::Editor => {
                self.apply_selection()
            }
            KeyboardAction::SaveProject if self.screen == DesktopScreen::Editor => {
                self.request_save()
            }
            KeyboardAction::Export => {
                if self.screen != DesktopScreen::Editor {
                    return Err(SurfaceError::ActionUnavailable);
                }
                let EditorState::Ready {
                    revision,
                    dirty: false,
                    ..
                } = self.workflow.editor()
                else {
                    return Err(SurfaceError::ActionUnavailable);
                };
                if self.workflow.pending_intent(WorkflowArea::Export).is_some()
                    || !matches!(
                        self.workflow.export(),
                        ExportState::Idle
                            | ExportState::Completed { .. }
                            | ExportState::Failed {
                                retryable: true,
                                ..
                            }
                    )
                {
                    return Err(SurfaceError::ActionUnavailable);
                }
                Ok(SurfaceEffect::ChooseExportDestination {
                    project_revision: revision,
                })
            }
            KeyboardAction::RetryLastOperation => self.retry_failure(),
            KeyboardAction::FocusRecorder => self.change_screen(DesktopScreen::Recorder),
            KeyboardAction::FocusEditor => self.change_screen(DesktopScreen::Editor),
            KeyboardAction::DismissDialog => self.dismiss_failure(),
            KeyboardAction::OpenProject
            | KeyboardAction::ApplySelection
            | KeyboardAction::SaveProject
            | KeyboardAction::Upload => Err(SurfaceError::ActionUnavailable),
        }
    }

    fn apply_selection(&mut self) -> Result<SurfaceEffect, SurfaceError> {
        let EditorState::Ready { revision, .. } = self.workflow.editor() else {
            return Err(SurfaceError::SelectionUnavailable);
        };
        let selection = self.selection.ok_or(SurfaceError::SelectionUnavailable)?;
        if selection.is_full_duration() {
            return Err(SurfaceError::ActionUnavailable);
        }
        self.queue_command(
            IntentKind::EditorApply {
                base_revision: revision,
            },
            SurfaceCommand::EditorApply {
                base_revision: revision,
                mutation: EditorMutation::Trim {
                    start_ms: selection.start_ms,
                    end_ms: selection.end_ms,
                },
            },
        )
    }

    fn request_save(&mut self) -> Result<SurfaceEffect, SurfaceError> {
        let EditorState::Ready {
            revision,
            dirty: true,
            ..
        } = self.workflow.editor()
        else {
            return Err(SurfaceError::ActionUnavailable);
        };
        let effect = self.queue_command(
            IntentKind::EditorSave {
                expected_revision: revision,
            },
            SurfaceCommand::EditorSave {
                expected_revision: revision,
            },
        )?;
        self.clear_failure_for_retry();
        Ok(effect)
    }

    fn retry_failure(&mut self) -> Result<SurfaceEffect, SurfaceError> {
        let failure = self.failure.ok_or(SurfaceError::ActionUnavailable)?;
        if !failure.retryable {
            return Err(SurfaceError::ActionUnavailable);
        }
        match failure.operation {
            FailureOperation::SaveProject => self.request_save(),
            FailureOperation::Export => {
                let EditorState::Ready { revision, .. } = self.workflow.editor() else {
                    return Err(SurfaceError::ActionUnavailable);
                };
                self.clear_failure_for_retry();
                Ok(SurfaceEffect::ChooseExportDestination {
                    project_revision: revision,
                })
            }
            FailureOperation::ApplyEdit => {
                let effect = self.apply_selection()?;
                self.clear_failure_for_retry();
                Ok(effect)
            }
            FailureOperation::OpenProject => Err(SurfaceError::ActionUnavailable),
        }
    }

    fn change_screen(&mut self, screen: DesktopScreen) -> Result<SurfaceEffect, SurfaceError> {
        self.screen = screen;
        self.failure = None;
        let focusable = self.focusable_ids();
        let preferred = match screen {
            DesktopScreen::Recorder => RECORDER_PRIMARY,
            DesktopScreen::Editor => match self.workflow.editor() {
                EditorState::Ready { .. } => SELECTION_START,
                _ => EDITOR_OPEN,
            },
        };
        let control_id = if focusable.iter().any(|id| id == preferred) {
            preferred.into()
        } else {
            self.fallback_focus(&focusable)
                .ok_or(SurfaceError::ControlNotFocusable)?
        };
        self.focused_control_id = Some(control_id.clone());
        Ok(SurfaceEffect::FocusChanged { control_id })
    }

    fn move_focus(&mut self, backwards: bool) -> Result<SurfaceEffect, SurfaceError> {
        let ids = self.focusable_ids();
        if ids.is_empty() {
            self.focused_control_id = None;
            return Err(SurfaceError::ControlNotFocusable);
        }
        let current = self
            .focused_control_id
            .as_ref()
            .and_then(|focused| ids.iter().position(|id| id == focused));
        let index = match (current, backwards) {
            (Some(0), true) | (None, true) => ids.len() - 1,
            (Some(index), true) => index - 1,
            (Some(index), false) => (index + 1) % ids.len(),
            (None, false) => 0,
        };
        let control_id = ids[index].clone();
        self.focused_control_id = Some(control_id.clone());
        Ok(SurfaceEffect::FocusChanged { control_id })
    }

    fn focusable_ids(&self) -> Vec<String> {
        let mut controls = self
            .accessibility_model()
            .controls
            .into_iter()
            .filter_map(|control| {
                (control.visible && !control.disabled).then_some((control.focus_order?, control.id))
            })
            .collect::<Vec<_>>();
        controls.sort_by_key(|(order, _)| *order);
        controls.into_iter().map(|(_, id)| id).collect()
    }

    fn queue_command(
        &mut self,
        kind: IntentKind,
        command: SurfaceCommand,
    ) -> Result<SurfaceEffect, SurfaceError> {
        let sequence = self
            .next_intent_sequence
            .checked_add(1)
            .ok_or(SurfaceError::IntentSequenceExhausted)?;
        let intent = UiIntent::new(format!("ui-{sequence:016x}"), kind)?;
        self.workflow.request(intent.clone())?;
        self.next_intent_sequence = sequence;
        Ok(SurfaceEffect::Command(ScopedSurfaceCommand {
            tenant_id: self.tenant_id.clone(),
            intent,
            command,
        }))
    }

    fn raise_failure(&mut self, failure: SurfaceFailure) {
        self.failure_restore_focus_id = self
            .focused_control_id
            .clone()
            .or_else(|| Some(active_tab(self.screen).into()));
        self.failure = Some(failure);
        self.announcement = failure.message().into();
        self.focused_control_id = Some(
            if failure.retryable {
                FAILURE_RETRY
            } else {
                FAILURE_DISMISS
            }
            .into(),
        );
    }

    fn dismiss_failure(&mut self) -> Result<SurfaceEffect, SurfaceError> {
        if self.failure.take().is_none() {
            return Err(SurfaceError::ActionUnavailable);
        }
        self.announcement = "Error dismissed.".into();
        let restore = self.failure_restore_focus_id.take();
        let ids = self.focusable_ids();
        self.focused_control_id = restore
            .filter(|id| ids.contains(id))
            .or_else(|| self.fallback_focus(&ids));
        Ok(SurfaceEffect::FailureDismissed)
    }

    fn clear_failure_for_retry(&mut self) {
        if self.failure.take().is_none() {
            return;
        }
        self.announcement = "Retrying operation.".into();
        let restore = self.failure_restore_focus_id.take();
        let ids = self.focusable_ids();
        self.focused_control_id = restore
            .filter(|id| ids.contains(id))
            .or_else(|| self.fallback_focus(&ids));
    }

    fn reconcile_focus(&mut self) {
        let ids = self.focusable_ids();
        if self
            .focused_control_id
            .as_ref()
            .is_none_or(|focused| !ids.contains(focused))
        {
            self.focused_control_id = self.fallback_focus(&ids);
        }
    }

    fn fallback_focus(&self, ids: &[String]) -> Option<String> {
        let active = active_tab(self.screen);
        ids.iter()
            .find(|id| id.as_str() == active)
            .cloned()
            .or_else(|| ids.first().cloned())
    }
}

fn active_tab(screen: DesktopScreen) -> &'static str {
    match screen {
        DesktopScreen::Recorder => RECORDER_TAB,
        DesktopScreen::Editor => EDITOR_TAB,
    }
}

fn normalize_chord(mut chord: KeyChord) -> KeyChord {
    if let Key::Character(character) = chord.key {
        chord.key = Key::Character(character.to_ascii_lowercase());
    }
    chord
}

fn shortcuts(platform: ShortcutPlatform) -> Vec<Shortcut> {
    let primary = |shift| Modifiers {
        control: platform == ShortcutPlatform::WindowsOrLinux,
        alt: false,
        shift,
        command: platform == ShortcutPlatform::MacOs,
    };
    vec![
        Shortcut {
            chord: KeyChord {
                modifiers: primary(true),
                key: Key::Character('r'),
            },
            action: KeyboardAction::StartStopRecording,
            global: true,
        },
        Shortcut {
            chord: KeyChord {
                modifiers: primary(true),
                key: Key::Character('p'),
            },
            action: KeyboardAction::PauseResumeRecording,
            global: true,
        },
        Shortcut {
            chord: KeyChord {
                modifiers: primary(false),
                key: Key::Character('s'),
            },
            action: KeyboardAction::SaveProject,
            global: false,
        },
        Shortcut {
            chord: KeyChord {
                modifiers: primary(true),
                key: Key::Character('e'),
            },
            action: KeyboardAction::Export,
            global: false,
        },
        Shortcut {
            chord: KeyChord {
                modifiers: Modifiers {
                    control: false,
                    alt: true,
                    shift: false,
                    command: false,
                },
                key: Key::Character('1'),
            },
            action: KeyboardAction::FocusRecorder,
            global: false,
        },
        Shortcut {
            chord: KeyChord {
                modifiers: Modifiers {
                    control: false,
                    alt: true,
                    shift: false,
                    command: false,
                },
                key: Key::Character('2'),
            },
            action: KeyboardAction::FocusEditor,
            global: false,
        },
        Shortcut {
            chord: KeyChord {
                modifiers: Modifiers::none(),
                key: Key::Escape,
            },
            action: KeyboardAction::DismissDialog,
            global: false,
        },
    ]
}

fn format_time(milliseconds: u64) -> String {
    let hours = milliseconds / 3_600_000;
    let minutes = (milliseconds / 60_000) % 60;
    let seconds = (milliseconds / 1_000) % 60;
    let millis = milliseconds % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

struct ControlSpec {
    id: &'static str,
    role: ControlRole,
    name: String,
    value_text: Option<String>,
    icon_only: bool,
    visible: bool,
    enabled: bool,
    action: Option<KeyboardAction>,
}

impl ControlSpec {
    fn button(id: &'static str, name: &str, enabled: bool, action: Option<KeyboardAction>) -> Self {
        Self {
            id,
            role: ControlRole::Button,
            name: name.into(),
            value_text: None,
            icon_only: false,
            visible: true,
            enabled,
            action,
        }
    }

    fn slider(id: &'static str, name: &str, value_text: String, enabled: bool) -> Self {
        Self {
            id,
            role: ControlRole::Slider,
            name: name.into(),
            value_text: Some(value_text),
            icon_only: false,
            visible: true,
            enabled,
            action: None,
        }
    }

    fn progress(id: &'static str, name: &str, value_text: &str) -> Self {
        Self {
            id,
            role: ControlRole::Progress,
            name: name.into(),
            value_text: Some(value_text.into()),
            icon_only: false,
            visible: true,
            enabled: false,
            action: None,
        }
    }

    fn role(mut self, role: ControlRole) -> Self {
        self.role = role;
        self
    }

    fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

fn push_control(controls: &mut Vec<Control>, order: &mut u16, spec: ControlSpec) {
    let focus_order =
        (spec.visible && spec.enabled && spec.role != ControlRole::Progress).then(|| {
            let current = *order;
            *order = order.saturating_add(1);
            current
        });
    controls.push(Control {
        id: spec.id.into(),
        role: spec.role,
        accessible_name: spec.name,
        value_text: spec.value_text,
        icon_only: spec.icon_only,
        visible: spec.visible,
        disabled: !spec.enabled,
        focus_order,
        action: spec.action,
    });
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError {
    #[error("private identifier is invalid")]
    InvalidPrivateIdentifier,
    #[error("backend event belongs to a different tenant")]
    TenantMismatch,
    #[error("surface intent sequence is exhausted")]
    IntentSequenceExhausted,
    #[error("control is not currently focusable")]
    ControlNotFocusable,
    #[error("action is unavailable in the backend-confirmed state")]
    ActionUnavailable,
    #[error("editor selection is unavailable")]
    SelectionUnavailable,
    #[error(transparent)]
    Workflow(#[from] WorkflowError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{BackendEventEnvelope, WORKFLOW_PROTOCOL_VERSION};

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("tenant")
    }

    fn session() -> SessionId {
        SessionId::new("surface-session-1").expect("session")
    }

    fn surface() -> DesktopProductSurface {
        DesktopProductSurface::new(
            tenant("tenant-a"),
            session(),
            ShortcutPlatform::WindowsOrLinux,
        )
    }

    fn event(tenant_id: &str, sequence: u64, event: BackendEvent) -> ScopedBackendEvent {
        ScopedBackendEvent::new(
            tenant(tenant_id),
            BackendEventEnvelope {
                protocol_version: WORKFLOW_PROTOCOL_VERSION,
                session_id: session(),
                sequence,
                event,
            },
        )
    }

    fn key(modifiers: Modifiers, key: Key) -> KeyChord {
        KeyChord { modifiers, key }
    }

    fn primary(shift: bool, character: char) -> KeyChord {
        key(
            Modifiers {
                control: true,
                alt: false,
                shift,
                command: false,
            },
            Key::Character(character),
        )
    }

    fn open_loaded_editor(surface: &mut DesktopProductSurface) {
        surface
            .handle_key(key(
                Modifiers {
                    control: false,
                    alt: true,
                    shift: false,
                    command: false,
                },
                Key::Character('2'),
            ))
            .expect("focus editor");
        let effect = surface
            .open_editor(ProjectHandle::new("project-handle-1").expect("project"))
            .expect("open");
        assert!(matches!(effect, SurfaceEffect::Command(_)));
        surface
            .apply_backend(event(
                "tenant-a",
                1,
                BackendEvent::EditorLoading {
                    intent_id: "ui-0000000000000001".into(),
                },
            ))
            .expect("loading");
        surface
            .apply_backend(event(
                "tenant-a",
                2,
                BackendEvent::EditorLoaded {
                    revision: 1,
                    duration_ms: 60_000,
                },
            ))
            .expect("loaded");
    }

    fn make_dirty(surface: &mut DesktopProductSurface) {
        surface
            .accept_focus(SELECTION_START)
            .expect("selection focus");
        surface
            .handle_key(key(Modifiers::none(), Key::ArrowRight))
            .expect("move selection");
        surface.accept_focus(EDITOR_APPLY).expect("apply focus");
        let effect = surface.activate_focused().expect("apply");
        assert!(matches!(effect, SurfaceEffect::Command(_)));
        surface
            .apply_backend(event(
                "tenant-a",
                3,
                BackendEvent::EditorApplied {
                    intent_id: "ui-0000000000000002".into(),
                    revision: 2,
                },
            ))
            .expect("applied");
    }

    #[test]
    fn recorder_shortcut_waits_for_backend_truth_and_tab_order_is_deterministic() {
        let mut surface = surface();
        assert!(surface.accessibility_model().validate().passed());
        assert_eq!(
            surface
                .handle_key(key(Modifiers::none(), Key::Tab))
                .expect("tab"),
            SurfaceEffect::FocusChanged {
                control_id: RECORDER_TAB.into()
            }
        );

        let effect = surface
            .handle_key(primary(true, 'R'))
            .expect("record shortcut");
        assert!(matches!(
            effect,
            SurfaceEffect::Command(ScopedSurfaceCommand {
                command: SurfaceCommand::RecorderStart,
                ..
            })
        ));
        assert_eq!(surface.snapshot().recorder, RecorderState::Idle);
        surface
            .apply_backend(event(
                "tenant-a",
                1,
                BackendEvent::RecorderPreparing {
                    intent_id: "ui-0000000000000001".into(),
                },
            ))
            .expect("preparing");
        surface
            .apply_backend(event("tenant-a", 2, BackendEvent::RecorderStarted))
            .expect("started");
        assert_eq!(surface.snapshot().recorder, RecorderState::Recording);
        assert!(surface.accessibility_model().validate().passed());
    }

    #[test]
    fn keyboard_editor_exposes_numeric_timeline_alternative() {
        let mut surface = surface();
        open_loaded_editor(&mut surface);
        surface.accept_focus(SELECTION_START).expect("focus");
        let effect = surface
            .handle_key(key(Modifiers::none(), Key::ArrowRight))
            .expect("adjust");
        assert_eq!(
            effect,
            SurfaceEffect::SelectionChanged(EditorSelection {
                start_ms: 1_000,
                end_ms: 60_000,
                duration_ms: 60_000,
            })
        );
        let start = surface
            .accessibility_model()
            .controls
            .into_iter()
            .find(|control| control.id == SELECTION_START)
            .expect("start control");
        assert_eq!(start.role, ControlRole::Slider);
        assert_eq!(start.value_text.as_deref(), Some("00:00:01.000"));
        assert!(surface.accessibility_model().validate().passed());
    }

    #[test]
    fn save_failure_preserves_dirty_revision_traps_focus_and_can_retry() {
        let mut surface = surface();
        open_loaded_editor(&mut surface);
        make_dirty(&mut surface);

        surface
            .handle_key(key(
                Modifiers {
                    control: false,
                    alt: true,
                    shift: false,
                    command: false,
                },
                Key::Character('1'),
            ))
            .expect("focus recorder");
        assert_eq!(
            surface.handle_key(primary(false, 's')),
            Err(SurfaceError::ActionUnavailable)
        );
        assert_eq!(
            surface
                .handle_key(key(
                    Modifiers {
                        control: false,
                        alt: true,
                        shift: false,
                        command: false,
                    },
                    Key::Character('2'),
                ))
                .expect("focus editor"),
            SurfaceEffect::FocusChanged {
                control_id: SELECTION_START.into()
            }
        );

        let save = surface
            .handle_key(primary(false, 's'))
            .expect("save shortcut");
        assert!(matches!(save, SurfaceEffect::Command(_)));
        surface
            .apply_backend(event(
                "tenant-a",
                4,
                BackendEvent::EditorFailed {
                    code: SafeFailureCode::DiskFull,
                },
            ))
            .expect("save failed");
        assert_eq!(
            surface.snapshot().editor,
            EditorState::Ready {
                revision: 2,
                duration_ms: 60_000,
                dirty: true,
            }
        );
        assert_eq!(
            surface.snapshot().failure,
            Some(SurfaceFailure {
                operation: FailureOperation::SaveProject,
                code: SafeFailureCode::DiskFull,
                retryable: true,
            })
        );
        let model = surface.accessibility_model();
        assert!(model.validate().passed());
        assert!(model.dialog.as_ref().is_some_and(|dialog| {
            dialog.focus_trapped && dialog.initial_focus_id.as_deref() == Some(FAILURE_RETRY)
        }));
        assert_eq!(
            surface
                .handle_key(key(Modifiers::none(), Key::Tab))
                .expect("dialog tab"),
            SurfaceEffect::FocusChanged {
                control_id: FAILURE_DISMISS.into()
            }
        );
        assert_eq!(
            surface
                .handle_key(key(
                    Modifiers {
                        control: false,
                        alt: false,
                        shift: true,
                        command: false,
                    },
                    Key::Tab,
                ))
                .expect("dialog reverse tab"),
            SurfaceEffect::FocusChanged {
                control_id: FAILURE_RETRY.into()
            }
        );

        let retry = surface
            .handle_key(key(Modifiers::none(), Key::Enter))
            .expect("retry");
        assert!(matches!(
            retry,
            SurfaceEffect::Command(ScopedSurfaceCommand {
                command: SurfaceCommand::EditorSave {
                    expected_revision: 2
                },
                ..
            })
        ));
        assert_eq!(
            surface.snapshot().focused_control_id.as_deref(),
            Some(EDITOR_TAB)
        );
        surface
            .apply_backend(event(
                "tenant-a",
                5,
                BackendEvent::EditorSaved {
                    intent_id: "ui-0000000000000004".into(),
                    revision: 2,
                },
            ))
            .expect("saved");
        assert!(matches!(
            surface.snapshot().editor,
            EditorState::Ready { dirty: false, .. }
        ));
    }

    #[test]
    fn failed_edit_preserves_revision_and_retries_the_same_keyboard_selection() {
        let mut surface = surface();
        open_loaded_editor(&mut surface);
        surface.accept_focus(SELECTION_START).expect("focus");
        surface
            .handle_key(key(Modifiers::none(), Key::ArrowRight))
            .expect("adjust");
        surface.accept_focus(EDITOR_APPLY).expect("apply focus");
        surface.activate_focused().expect("apply");
        surface
            .apply_backend(event(
                "tenant-a",
                3,
                BackendEvent::EditorFailed {
                    code: SafeFailureCode::BackendUnavailable,
                },
            ))
            .expect("apply failed");
        assert_eq!(
            surface.snapshot().editor,
            EditorState::Ready {
                revision: 1,
                duration_ms: 60_000,
                dirty: false,
            }
        );
        let retry = surface
            .handle_key(key(Modifiers::none(), Key::Enter))
            .expect("retry");
        assert!(matches!(
            retry,
            SurfaceEffect::Command(ScopedSurfaceCommand {
                command: SurfaceCommand::EditorApply {
                    base_revision: 1,
                    mutation: EditorMutation::Trim {
                        start_ms: 1_000,
                        end_ms: 60_000,
                    },
                },
                ..
            })
        ));
        surface
            .apply_backend(event(
                "tenant-a",
                4,
                BackendEvent::EditorApplied {
                    intent_id: "ui-0000000000000003".into(),
                    revision: 2,
                },
            ))
            .expect("applied");
        assert!(matches!(
            surface.snapshot().editor,
            EditorState::Ready {
                revision: 2,
                dirty: true,
                ..
            }
        ));
    }

    #[test]
    fn nonretryable_export_failure_has_dismiss_only_and_blocks_restart() {
        let mut surface = surface();
        open_loaded_editor(&mut surface);
        let choose = surface
            .handle_key(primary(true, 'e'))
            .expect("choose export");
        assert_eq!(
            choose,
            SurfaceEffect::ChooseExportDestination {
                project_revision: 1
            }
        );
        surface
            .request_export(
                ExportDestinationHandle::new("destination-handle-1").expect("destination"),
            )
            .expect("export request");
        surface
            .apply_backend(event(
                "tenant-a",
                3,
                BackendEvent::ExportStarted {
                    intent_id: "ui-0000000000000002".into(),
                    project_revision: 1,
                },
            ))
            .expect("started");
        surface
            .apply_backend(event(
                "tenant-a",
                4,
                BackendEvent::ExportFailed {
                    code: SafeFailureCode::UnsupportedProject,
                    retryable: false,
                },
            ))
            .expect("failed");
        let model = surface.accessibility_model();
        assert!(model.validate().passed());
        assert_eq!(
            model.dialog.and_then(|dialog| dialog.initial_focus_id),
            Some(FAILURE_DISMISS.into())
        );
        assert!(!model.controls.iter().any(|control| {
            control.id == FAILURE_RETRY && control.visible && !control.disabled
        }));

        surface
            .handle_key(key(Modifiers::none(), Key::Enter))
            .expect("dismiss");
        assert_eq!(
            surface.handle_key(primary(true, 'e')),
            Err(SurfaceError::ActionUnavailable)
        );
    }

    #[test]
    fn cross_tenant_event_is_atomic_and_debug_output_redacts_private_values() {
        let private_tenant = "tenant-private-acme";
        let private_project = "project-private-launch";
        let mut surface =
            DesktopProductSurface::new(tenant(private_tenant), session(), ShortcutPlatform::MacOs);
        surface
            .handle_key(key(
                Modifiers {
                    control: false,
                    alt: true,
                    shift: false,
                    command: false,
                },
                Key::Character('2'),
            ))
            .expect("editor");
        let command = surface
            .open_editor(ProjectHandle::new(private_project).expect("project"))
            .expect("open");
        let rendered_command = format!("{command:?}");
        assert!(!rendered_command.contains(private_tenant));
        assert!(!rendered_command.contains(private_project));
        assert!(!format!("{surface:?}").contains(private_tenant));

        let wrong_tenant = event(
            "tenant-b",
            1,
            BackendEvent::EditorLoading {
                intent_id: "ui-0000000000000001".into(),
            },
        );
        assert_eq!(
            surface.apply_backend(wrong_tenant),
            Err(SurfaceError::TenantMismatch)
        );
        assert_eq!(surface.workflow().last_backend_sequence(), 0);
        assert_eq!(surface.workflow().editor(), EditorState::Closed);
    }

    #[test]
    fn private_handles_and_shortcuts_are_strictly_validated() {
        assert_eq!(
            ProjectHandle::new("../private/project"),
            Err(SurfaceError::InvalidPrivateIdentifier)
        );
        for platform in [ShortcutPlatform::MacOs, ShortcutPlatform::WindowsOrLinux] {
            let model = DesktopProductSurface::new(tenant("tenant-a"), session(), platform)
                .accessibility_model();
            assert!(
                model.validate().passed(),
                "{platform:?}: {:?}",
                model.validate()
            );
        }
    }
}
