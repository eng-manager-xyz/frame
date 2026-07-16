use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

pub const ACCESSIBILITY_MODEL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandmarkRole {
    Banner,
    Navigation,
    Main,
    Complementary,
    ContentInfo,
    Status,
    Dialog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveRegion {
    Off,
    Polite,
    Assertive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Landmark {
    pub id: String,
    pub role: LandmarkRole,
    pub label: String,
    pub live: LiveRegion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlRole {
    Button,
    Checkbox,
    Radio,
    Slider,
    Textbox,
    Tab,
    MenuItem,
    Link,
    Progress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardAction {
    StartStopRecording,
    PauseResumeRecording,
    Cancel,
    SaveProject,
    Export,
    Upload,
    FocusRecorder,
    FocusEditor,
    OpenRecovery,
    DismissDialog,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Control {
    pub id: String,
    pub role: ControlRole,
    pub accessible_name: String,
    pub value_text: Option<String>,
    pub icon_only: bool,
    pub visible: bool,
    pub disabled: bool,
    /// Positive DOM-equivalent tab order. `None` means deliberately not focusable.
    pub focus_order: Option<u16>,
    pub action: Option<KeyboardAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Modifiers {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub command: bool,
}

impl Modifiers {
    #[must_use]
    pub const fn none() -> Self {
        Self {
            control: false,
            alt: false,
            shift: false,
            command: false,
        }
    }

    #[must_use]
    pub const fn has_command_modifier(self) -> bool {
        self.control || self.alt || self.command
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Character(char),
    Enter,
    Space,
    Escape,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Function(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyChord {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl KeyChord {
    pub fn validate(self) -> Result<Self, AccessibilityError> {
        match self.key {
            Key::Character(character) if character.is_ascii_alphanumeric() => {}
            Key::Character(_) => return Err(AccessibilityError::InvalidShortcut),
            Key::Function(number) if (1..=24).contains(&number) => {}
            Key::Function(_) => return Err(AccessibilityError::InvalidShortcut),
            _ => {}
        }
        if matches!(self.key, Key::Character(_)) && !self.modifiers.has_command_modifier() {
            return Err(AccessibilityError::UnmodifiedCharacterShortcut);
        }
        // Reserve operating-system quit/close shortcuts.
        if matches!(self.key, Key::Character('q' | 'Q')) && self.modifiers.command
            || matches!(self.key, Key::Function(4)) && self.modifiers.alt
        {
            return Err(AccessibilityError::ReservedShortcut);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcut {
    pub chord: KeyChord,
    pub action: KeyboardAction,
    pub global: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogFocusModel {
    pub open: bool,
    pub focus_trapped: bool,
    pub initial_focus_id: Option<String>,
    pub restore_focus_id: Option<String>,
    pub dismissible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessibilityModel {
    pub version: u16,
    pub landmarks: Vec<Landmark>,
    pub controls: Vec<Control>,
    pub shortcuts: Vec<Shortcut>,
    pub dialog: Option<DialogFocusModel>,
}

impl AccessibilityModel {
    #[must_use]
    pub fn validate(&self) -> AccessibilityReport {
        let mut errors = Vec::new();
        if self.version != ACCESSIBILITY_MODEL_VERSION {
            errors.push(AccessibilityError::UnsupportedVersion);
        }

        let main_count = self
            .landmarks
            .iter()
            .filter(|landmark| landmark.role == LandmarkRole::Main)
            .count();
        if main_count != 1 {
            errors.push(AccessibilityError::MainLandmarkCount);
        }

        let mut ids = HashSet::new();
        for landmark in &self.landmarks {
            validate_identifier_and_label(&landmark.id, &landmark.label, &mut errors);
            if !ids.insert(landmark.id.as_str()) {
                errors.push(AccessibilityError::DuplicateId);
            }
            if landmark.role == LandmarkRole::Status && landmark.live == LiveRegion::Off {
                errors.push(AccessibilityError::StatusNotLive);
            }
        }

        let mut focus_orders = HashSet::new();
        for control in &self.controls {
            validate_identifier_and_label(&control.id, &control.accessible_name, &mut errors);
            if !ids.insert(control.id.as_str()) {
                errors.push(AccessibilityError::DuplicateId);
            }
            if control.visible && !control.disabled {
                if control.role != ControlRole::Progress && control.focus_order.is_none() {
                    errors.push(AccessibilityError::MissingFocusOrder);
                }
                if control.focus_order == Some(0) {
                    errors.push(AccessibilityError::InvalidFocusOrder);
                }
                if let Some(order) = control.focus_order
                    && !focus_orders.insert(order)
                {
                    errors.push(AccessibilityError::DuplicateFocusOrder);
                }
                if control.icon_only && !valid_label(&control.accessible_name) {
                    errors.push(AccessibilityError::IconMissingName);
                }
            }
            if matches!(control.role, ControlRole::Slider | ControlRole::Progress)
                && control.visible
                && control
                    .value_text
                    .as_deref()
                    .is_none_or(|value| !valid_label(value))
            {
                errors.push(AccessibilityError::ValueTextMissing);
            }
        }

        let mut shortcuts = HashMap::new();
        for shortcut in &self.shortcuts {
            if let Err(error) = shortcut.chord.validate() {
                errors.push(error);
                continue;
            }
            if shortcuts.insert(shortcut.chord, shortcut.action).is_some() {
                errors.push(AccessibilityError::DuplicateShortcut);
            }
        }

        if let Some(dialog) = &self.dialog
            && dialog.open
        {
            if !dialog.focus_trapped {
                errors.push(AccessibilityError::DialogFocusNotTrapped);
            }
            if !dialog
                .initial_focus_id
                .as_deref()
                .is_some_and(|id| ids.contains(id))
            {
                errors.push(AccessibilityError::DialogInitialFocusMissing);
            }
            if !dialog
                .restore_focus_id
                .as_deref()
                .is_some_and(|id| ids.contains(id))
            {
                errors.push(AccessibilityError::DialogRestoreFocusMissing);
            }
            if dialog.dismissible
                && !self.shortcuts.iter().any(|shortcut| {
                    shortcut.action == KeyboardAction::DismissDialog
                        && shortcut.chord.key == Key::Escape
                        && shortcut.chord.modifiers == Modifiers::none()
                })
            {
                errors.push(AccessibilityError::DialogEscapeMissing);
            }
        }

        errors.sort_unstable();
        errors.dedup();
        AccessibilityReport { errors }
    }

    #[must_use]
    pub fn keyboard_action(&self, chord: KeyChord) -> Option<KeyboardAction> {
        self.shortcuts
            .iter()
            .find(|shortcut| shortcut.chord == chord)
            .map(|shortcut| shortcut.action)
    }
}

fn validate_identifier_and_label(id: &str, label: &str, errors: &mut Vec<AccessibilityError>) {
    if !valid_identifier(id) {
        errors.push(AccessibilityError::InvalidId);
    }
    if !valid_label(label) {
        errors.push(AccessibilityError::MissingAccessibleName);
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_".contains(character))
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 160 && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityReport {
    pub errors: Vec<AccessibilityError>,
}

impl AccessibilityReport {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AccessibilityError {
    UnsupportedVersion,
    MainLandmarkCount,
    DuplicateId,
    InvalidId,
    MissingAccessibleName,
    StatusNotLive,
    MissingFocusOrder,
    InvalidFocusOrder,
    DuplicateFocusOrder,
    IconMissingName,
    ValueTextMissing,
    InvalidShortcut,
    UnmodifiedCharacterShortcut,
    ReservedShortcut,
    DuplicateShortcut,
    DialogFocusNotTrapped,
    DialogInitialFocusMissing,
    DialogRestoreFocusMissing,
    DialogEscapeMissing,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(key: char) -> KeyChord {
        KeyChord {
            modifiers: Modifiers {
                control: true,
                alt: false,
                shift: false,
                command: false,
            },
            key: Key::Character(key),
        }
    }

    fn valid_model() -> AccessibilityModel {
        AccessibilityModel {
            version: ACCESSIBILITY_MODEL_VERSION,
            landmarks: vec![
                Landmark {
                    id: "main".into(),
                    role: LandmarkRole::Main,
                    label: "Recorder".into(),
                    live: LiveRegion::Off,
                },
                Landmark {
                    id: "status".into(),
                    role: LandmarkRole::Status,
                    label: "Recording status".into(),
                    live: LiveRegion::Polite,
                },
            ],
            controls: vec![
                Control {
                    id: "record".into(),
                    role: ControlRole::Button,
                    accessible_name: "Start recording".into(),
                    value_text: None,
                    icon_only: true,
                    visible: true,
                    disabled: false,
                    focus_order: Some(1),
                    action: Some(KeyboardAction::StartStopRecording),
                },
                Control {
                    id: "cancel".into(),
                    role: ControlRole::Button,
                    accessible_name: "Cancel".into(),
                    value_text: None,
                    icon_only: false,
                    visible: true,
                    disabled: false,
                    focus_order: Some(2),
                    action: Some(KeyboardAction::Cancel),
                },
            ],
            shortcuts: vec![
                Shortcut {
                    chord: chord('r'),
                    action: KeyboardAction::StartStopRecording,
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
            ],
            dialog: Some(DialogFocusModel {
                open: true,
                focus_trapped: true,
                initial_focus_id: Some("record".into()),
                restore_focus_id: Some("cancel".into()),
                dismissible: true,
            }),
        }
    }

    #[test]
    fn complete_model_passes_and_resolves_shortcuts() {
        let model = valid_model();
        assert!(model.validate().passed());
        assert_eq!(
            model.keyboard_action(chord('r')),
            Some(KeyboardAction::StartStopRecording)
        );
    }

    #[test]
    fn icon_only_controls_require_names() {
        let mut model = valid_model();
        model.controls[0].accessible_name.clear();
        let report = model.validate();
        assert!(
            report
                .errors
                .contains(&AccessibilityError::MissingAccessibleName)
        );
        assert!(report.errors.contains(&AccessibilityError::IconMissingName));
    }

    #[test]
    fn visible_enabled_controls_require_focus_order() {
        let mut model = valid_model();
        model.controls[0].focus_order = None;
        assert!(
            model
                .validate()
                .errors
                .contains(&AccessibilityError::MissingFocusOrder)
        );
    }

    #[test]
    fn shortcut_conflicts_are_rejected() {
        let mut model = valid_model();
        model.shortcuts.push(Shortcut {
            chord: chord('r'),
            action: KeyboardAction::Export,
            global: false,
        });
        assert!(
            model
                .validate()
                .errors
                .contains(&AccessibilityError::DuplicateShortcut)
        );
    }

    #[test]
    fn modal_dialog_requires_focus_trap_restore_and_escape() {
        let mut model = valid_model();
        model.shortcuts.clear();
        model.dialog = Some(DialogFocusModel {
            open: true,
            focus_trapped: false,
            initial_focus_id: Some("missing".into()),
            restore_focus_id: None,
            dismissible: true,
        });
        let errors = model.validate().errors;
        assert!(errors.contains(&AccessibilityError::DialogFocusNotTrapped));
        assert!(errors.contains(&AccessibilityError::DialogInitialFocusMissing));
        assert!(errors.contains(&AccessibilityError::DialogRestoreFocusMissing));
        assert!(errors.contains(&AccessibilityError::DialogEscapeMissing));
    }

    #[test]
    fn unmodified_character_shortcuts_are_rejected() {
        assert_eq!(
            KeyChord {
                modifiers: Modifiers::none(),
                key: Key::Character('r')
            }
            .validate(),
            Err(AccessibilityError::UnmodifiedCharacterShortcut)
        );
    }
}
