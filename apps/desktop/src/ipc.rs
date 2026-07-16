use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

pub const IPC_PROTOCOL_VERSION: u16 = 1;
const MAX_PATH_BYTES: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopShell {
    Tauri2LeptosCsr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecorderAdapterState {
    NotSelected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorAdapterState {
    RevisionFencedCore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShellCapabilities {
    pub protocol_version: u16,
    pub shell: DesktopShell,
    pub backend_truth: bool,
    pub recorder_adapter: RecorderAdapterState,
    pub editor_adapter: EditorAdapterState,
}

impl ShellCapabilities {
    #[must_use]
    pub const fn current() -> Self {
        Self {
            protocol_version: IPC_PROTOCOL_VERSION,
            shell: DesktopShell::Tauri2LeptosCsr,
            backend_truth: true,
            recorder_adapter: RecorderAdapterState::NotSelected,
            editor_adapter: EditorAdapterState::RevisionFencedCore,
        }
    }

    #[must_use]
    pub const fn is_current_backend_truth(self) -> bool {
        self.protocol_version == IPC_PROTOCOL_VERSION
            && matches!(self.shell, DesktopShell::Tauri2LeptosCsr)
            && self.backend_truth
    }
}

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IpcError> {
                let value = value.into();
                if !valid_opaque_id(&value) {
                    return Err(IpcError::InvalidIdentifier);
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
                formatter.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

opaque_id!(RequestId);
opaque_id!(WindowId);
opaque_id!(SessionId);

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRole {
    Main,
    Recorder,
    Recovery,
    Editor,
    Export,
    Settings,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    WindowOpen,
    RecorderPrepare,
    RecorderStart,
    RecorderPause,
    RecorderResume,
    RecorderStop,
    RecorderCancel,
    DeviceEnumerate,
    DeviceSelect,
    RecoveryScan,
    RecoveryInspect,
    RecoveryOpen,
    RecoveryDiscard,
    EditorOpen,
    EditorApply,
    EditorSave,
    ExportStart,
    ExportCancel,
    UploadStart,
    UploadPause,
    UploadResume,
    UploadCancel,
    RecorderConfigure,
    CaptureTargetSelect,
    SettingsApply,
    PresetApply,
    Lifecycle,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecorderMode {
    Instant,
    Studio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTargetKind {
    Display,
    Window,
    Region,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleAction {
    RegisterHotkeys,
    ShowMainWindow,
    HideMainWindow,
    ShowOverlay,
    HideOverlay,
    ShowTargetPicker,
    HideTargetPicker,
    CloseWindow,
    ReopenWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAction {
    Check,
    Install,
    Relaunch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceClass {
    Display,
    Microphone,
    SystemAudio,
    Camera,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportProfile {
    DistributionMp4,
    EditableWebm,
    Archive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditorMutation {
    Trim {
        start_ms: u64,
        end_ms: u64,
    },
    DeleteRange {
        start_ms: u64,
        end_ms: u64,
    },
    Split {
        at_ms: u64,
    },
    Speed {
        start_ms: u64,
        end_ms: u64,
        rate_milli: u16,
    },
    AudioGain {
        start_ms: u64,
        end_ms: u64,
        gain_millibels: i32,
    },
}

impl EditorMutation {
    fn validate(&self) -> Result<(), IpcError> {
        match self {
            Self::Trim { start_ms, end_ms }
            | Self::DeleteRange { start_ms, end_ms }
            | Self::AudioGain {
                start_ms, end_ms, ..
            } => validate_range(*start_ms, *end_ms),
            Self::Speed {
                start_ms,
                end_ms,
                rate_milli,
            } => {
                validate_range(*start_ms, *end_ms)?;
                if !(250..=4_000).contains(rate_milli) {
                    return Err(IpcError::InvalidPayload);
                }
                Ok(())
            }
            Self::Split { at_ms } if *at_ms > 0 => Ok(()),
            Self::Split { .. } => Err(IpcError::InvalidPayload),
        }?;

        if let Self::AudioGain { gain_millibels, .. } = self
            && !(-9_600..=2_400).contains(gain_millibels)
        {
            return Err(IpcError::InvalidPayload);
        }
        Ok(())
    }
}

fn validate_range(start: u64, end: u64) -> Result<(), IpcError> {
    if start < end {
        Ok(())
    } else {
        Err(IpcError::InvalidPayload)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", content = "payload", rename_all = "snake_case")]
pub enum IpcCommand {
    WindowOpen {
        role: WindowRole,
    },
    RecorderPrepare,
    RecorderStart {
        intent_id: String,
    },
    RecorderPause {
        intent_id: String,
    },
    RecorderResume {
        intent_id: String,
    },
    RecorderStop {
        intent_id: String,
    },
    RecorderCancel {
        intent_id: String,
    },
    DeviceEnumerate {
        class: DeviceClass,
    },
    DeviceSelect {
        class: DeviceClass,
        device_token: String,
    },
    RecoveryScan,
    RecoveryInspect {
        project_path: String,
    },
    RecoveryOpen {
        project_path: String,
    },
    RecoveryDiscard {
        project_path: String,
    },
    EditorOpen {
        project_path: String,
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
        output_path: String,
        profile: ExportProfile,
    },
    ExportCancel {
        intent_id: String,
    },
    UploadStart {
        source_path: String,
        upload_intent: String,
    },
    UploadPause {
        intent_id: String,
    },
    UploadResume {
        intent_id: String,
    },
    UploadCancel {
        intent_id: String,
    },
    RecorderConfigure {
        mode: RecorderMode,
        countdown_seconds: u8,
        exclude_frame_windows: bool,
    },
    CaptureTargetSelect {
        kind: CaptureTargetKind,
        target_token: String,
    },
    SettingsApply {
        expected_revision: u64,
        mode: RecorderMode,
        frame_rate: u16,
        microphone_enabled: bool,
        system_audio_enabled: bool,
        camera_enabled: bool,
        reduced_motion: bool,
    },
    PresetApply {
        preset_token: String,
        expected_settings_revision: u64,
    },
    Lifecycle {
        action: LifecycleAction,
    },
    Update {
        action: UpdateAction,
        expected_revision: u64,
    },
}

impl fmt::Debug for IpcCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("IpcCommand")
            .field(&self.kind())
            .finish()
    }
}

impl IpcCommand {
    #[must_use]
    pub const fn kind(&self) -> CommandKind {
        match self {
            Self::WindowOpen { .. } => CommandKind::WindowOpen,
            Self::RecorderPrepare => CommandKind::RecorderPrepare,
            Self::RecorderStart { .. } => CommandKind::RecorderStart,
            Self::RecorderPause { .. } => CommandKind::RecorderPause,
            Self::RecorderResume { .. } => CommandKind::RecorderResume,
            Self::RecorderStop { .. } => CommandKind::RecorderStop,
            Self::RecorderCancel { .. } => CommandKind::RecorderCancel,
            Self::DeviceEnumerate { .. } => CommandKind::DeviceEnumerate,
            Self::DeviceSelect { .. } => CommandKind::DeviceSelect,
            Self::RecoveryScan => CommandKind::RecoveryScan,
            Self::RecoveryInspect { .. } => CommandKind::RecoveryInspect,
            Self::RecoveryOpen { .. } => CommandKind::RecoveryOpen,
            Self::RecoveryDiscard { .. } => CommandKind::RecoveryDiscard,
            Self::EditorOpen { .. } => CommandKind::EditorOpen,
            Self::EditorApply { .. } => CommandKind::EditorApply,
            Self::EditorSave { .. } => CommandKind::EditorSave,
            Self::ExportStart { .. } => CommandKind::ExportStart,
            Self::ExportCancel { .. } => CommandKind::ExportCancel,
            Self::UploadStart { .. } => CommandKind::UploadStart,
            Self::UploadPause { .. } => CommandKind::UploadPause,
            Self::UploadResume { .. } => CommandKind::UploadResume,
            Self::UploadCancel { .. } => CommandKind::UploadCancel,
            Self::RecorderConfigure { .. } => CommandKind::RecorderConfigure,
            Self::CaptureTargetSelect { .. } => CommandKind::CaptureTargetSelect,
            Self::SettingsApply { .. } => CommandKind::SettingsApply,
            Self::PresetApply { .. } => CommandKind::PresetApply,
            Self::Lifecycle { .. } => CommandKind::Lifecycle,
            Self::Update { .. } => CommandKind::Update,
        }
    }

    fn path_request(&self) -> Option<(&str, PathUse)> {
        match self {
            Self::RecoveryInspect { project_path } | Self::RecoveryOpen { project_path } => {
                Some((project_path, PathUse::ProjectRead))
            }
            Self::RecoveryDiscard { project_path } => Some((project_path, PathUse::ProjectDelete)),
            Self::EditorOpen { project_path } => Some((project_path, PathUse::ProjectRead)),
            Self::ExportStart { output_path, .. } => Some((output_path, PathUse::ExportWrite)),
            Self::UploadStart { source_path, .. } => Some((source_path, PathUse::MediaRead)),
            _ => None,
        }
    }

    fn validate_payload(&self) -> Result<(), IpcError> {
        match self {
            Self::RecorderStart { intent_id }
            | Self::RecorderPause { intent_id }
            | Self::RecorderResume { intent_id }
            | Self::RecorderStop { intent_id }
            | Self::RecorderCancel { intent_id }
            | Self::ExportCancel { intent_id }
            | Self::UploadPause { intent_id }
            | Self::UploadResume { intent_id }
            | Self::UploadCancel { intent_id } => validate_token(intent_id),
            Self::DeviceSelect { device_token, .. } => validate_token(device_token),
            Self::CaptureTargetSelect { target_token, .. } => validate_token(target_token),
            Self::RecoveryInspect { project_path }
            | Self::RecoveryOpen { project_path }
            | Self::RecoveryDiscard { project_path }
            | Self::EditorOpen { project_path } => validate_path_text(project_path),
            Self::EditorApply {
                base_revision,
                mutation,
            } => {
                if *base_revision == 0 {
                    return Err(IpcError::InvalidPayload);
                }
                mutation.validate()
            }
            Self::EditorSave { expected_revision } if *expected_revision == 0 => {
                Err(IpcError::InvalidPayload)
            }
            Self::ExportStart {
                project_revision,
                output_path,
                ..
            } => {
                if *project_revision == 0 {
                    return Err(IpcError::InvalidPayload);
                }
                validate_path_text(output_path)
            }
            Self::UploadStart {
                source_path,
                upload_intent,
            } => {
                validate_path_text(source_path)?;
                validate_token(upload_intent)
            }
            Self::RecorderConfigure {
                countdown_seconds, ..
            } if *countdown_seconds > 10 => Err(IpcError::InvalidPayload),
            Self::SettingsApply {
                expected_revision,
                frame_rate,
                ..
            } => {
                if *expected_revision == 0 || !matches!(frame_rate, 24 | 25 | 30 | 50 | 60) {
                    return Err(IpcError::InvalidPayload);
                }
                Ok(())
            }
            Self::PresetApply {
                preset_token,
                expected_settings_revision,
            } => {
                if *expected_settings_revision == 0 {
                    return Err(IpcError::InvalidPayload);
                }
                validate_token(preset_token)
            }
            Self::Update {
                expected_revision, ..
            } if *expected_revision == 0 => Err(IpcError::InvalidPayload),
            _ => Ok(()),
        }
    }
}

fn validate_token(value: &str) -> Result<(), IpcError> {
    if valid_opaque_id(value) {
        Ok(())
    } else {
        Err(IpcError::InvalidPayload)
    }
}

fn validate_path_text(value: &str) -> Result<(), IpcError> {
    if value.is_empty() || value.len() > MAX_PATH_BYTES || value.contains('\0') {
        Err(IpcError::InvalidPayload)
    } else {
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub window_id: WindowId,
    pub session_id: SessionId,
    pub sequence: u64,
    #[serde(flatten)]
    pub command: IpcCommand,
}

impl fmt::Debug for RequestEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RequestEnvelope")
            .field("protocol_version", &self.protocol_version)
            .field("request_id", &self.request_id)
            .field("window_id", &self.window_id)
            .field("session_id", &self.session_id)
            .field("sequence", &self.sequence)
            .field("command", &self.command.kind())
            .finish()
    }
}

const KNOWN_COMMANDS: &[&str] = &[
    "window_open",
    "recorder_prepare",
    "recorder_start",
    "recorder_pause",
    "recorder_resume",
    "recorder_stop",
    "recorder_cancel",
    "device_enumerate",
    "device_select",
    "recovery_scan",
    "recovery_inspect",
    "recovery_open",
    "recovery_discard",
    "editor_open",
    "editor_apply",
    "editor_save",
    "export_start",
    "export_cancel",
    "upload_start",
    "upload_pause",
    "upload_resume",
    "upload_cancel",
    "recorder_configure",
    "capture_target_select",
    "settings_apply",
    "preset_apply",
    "lifecycle",
    "update",
];

/// Decodes untrusted JSON without reflecting parser details or payload content
/// into public errors.
pub fn decode_request(json: &str) -> Result<RequestEnvelope, IpcError> {
    if json.len() > 64 * 1_024 {
        return Err(IpcError::EnvelopeTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| IpcError::MalformedEnvelope)?;
    let command = value
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or(IpcError::MalformedEnvelope)?;
    if !KNOWN_COMMANDS.contains(&command) {
        return Err(IpcError::UnknownCommand);
    }
    let request: RequestEnvelope =
        serde_json::from_value(value).map_err(|_| IpcError::MalformedEnvelope)?;
    request.command.validate_payload()?;
    Ok(request)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathUse {
    ProjectRead,
    ProjectDelete,
    MediaRead,
    ExportWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootAccess {
    pub read: bool,
    pub write: bool,
    pub delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScopedRoot {
    root: PathBuf,
    access: RootAccess,
}

#[derive(Clone, PartialEq, Eq, Default)]
pub struct PathPolicy {
    roots: Vec<ScopedRoot>,
}

impl fmt::Debug for PathPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PathPolicy")
            .field("root_count", &self.roots.len())
            .finish()
    }
}

impl PathPolicy {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn allow_root(
        mut self,
        root: impl Into<PathBuf>,
        access: RootAccess,
    ) -> Result<Self, IpcError> {
        let root = validate_absolute_path(root.into())?;
        if !access.read && !access.write && !access.delete {
            return Err(IpcError::InvalidPathScope);
        }
        if self.roots.iter().any(|existing| existing.root == root) {
            return Err(IpcError::DuplicatePathScope);
        }
        self.roots.push(ScopedRoot { root, access });
        Ok(self)
    }

    pub fn validate(&self, path: &str, usage: PathUse) -> Result<ValidatedPath, IpcError> {
        let path = validate_absolute_path(PathBuf::from(path))?;
        let root = self
            .roots
            .iter()
            .filter(|root| path.starts_with(&root.root))
            .max_by_key(|root| root.root.components().count())
            .ok_or(IpcError::PathOutOfScope)?;
        if path == root.root || !root_allows(root.access, usage) {
            return Err(IpcError::PathOutOfScope);
        }
        validate_extension(&path, usage)?;
        Ok(ValidatedPath {
            path,
            usage,
            requires_no_follow: true,
        })
    }
}

fn validate_absolute_path(path: PathBuf) -> Result<PathBuf, IpcError> {
    if !path.is_absolute() || path.as_os_str().len() > MAX_PATH_BYTES {
        return Err(IpcError::InvalidPath);
    }
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(IpcError::InvalidPath);
        }
    }
    Ok(path)
}

fn root_allows(access: RootAccess, usage: PathUse) -> bool {
    match usage {
        PathUse::ProjectRead | PathUse::MediaRead => access.read,
        PathUse::ExportWrite => access.write,
        PathUse::ProjectDelete => access.delete,
    }
}

fn validate_extension(path: &Path, usage: PathUse) -> Result<(), IpcError> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or(IpcError::UnsupportedPathType)?;
    let allowed: &[&str] = match usage {
        PathUse::ProjectRead | PathUse::ProjectDelete => &["cap", "frame", "json"],
        PathUse::MediaRead => &["mp4", "mov", "webm", "mkv", "wav", "m4a", "aac"],
        PathUse::ExportWrite => &["mp4", "mov", "webm", "mkv"],
    };
    if allowed.contains(&extension.as_str()) {
        Ok(())
    } else {
        Err(IpcError::UnsupportedPathType)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ValidatedPath {
    path: PathBuf,
    usage: PathUse,
    requires_no_follow: bool,
}

impl ValidatedPath {
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn usage(&self) -> PathUse {
        self.usage
    }

    /// Callers opening the path must use platform no-follow/reparse-point-safe
    /// semantics and re-check the resolved handle remains under the trusted root.
    #[must_use]
    pub const fn requires_no_follow(&self) -> bool {
        self.requires_no_follow
    }
}

impl fmt::Debug for ValidatedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedPath")
            .field("usage", &self.usage)
            .field("requires_no_follow", &self.requires_no_follow)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct WindowScope {
    pub window_id: WindowId,
    pub session_id: SessionId,
    pub role: WindowRole,
    pub paths: PathPolicy,
}

impl fmt::Debug for WindowScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WindowScope")
            .field("window_id", &self.window_id)
            .field("session_id", &self.session_id)
            .field("role", &self.role)
            .field("paths", &self.paths)
            .finish()
    }
}

#[derive(Debug, Clone)]
struct WindowSession {
    scope: WindowScope,
    last_sequence: u64,
}

#[derive(Debug, Clone)]
struct PendingRequest {
    window_id: WindowId,
    session_id: SessionId,
    sequence: u64,
    command: CommandKind,
}

#[derive(Clone, Default)]
pub struct ScopeRegistry {
    windows: HashMap<WindowId, WindowSession>,
    pending: HashMap<RequestId, PendingRequest>,
    completed: HashSet<RequestId>,
    completed_order: VecDeque<RequestId>,
}

const COMPLETED_REQUEST_WINDOW: usize = 4_096;

impl fmt::Debug for ScopeRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopeRegistry")
            .field("window_count", &self.windows.len())
            .field("pending_count", &self.pending.len())
            .field("completed_count", &self.completed.len())
            .finish()
    }
}

impl ScopeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, scope: WindowScope) -> Result<(), IpcError> {
        if self.windows.contains_key(&scope.window_id) {
            return Err(IpcError::WindowAlreadyRegistered);
        }
        self.windows.insert(
            scope.window_id.clone(),
            WindowSession {
                scope,
                last_sequence: 0,
            },
        );
        Ok(())
    }

    pub fn revoke(&mut self, window: &WindowId, session: &SessionId) -> Result<(), IpcError> {
        let current = self.windows.get(window).ok_or(IpcError::UnknownWindow)?;
        if &current.scope.session_id != session {
            return Err(IpcError::SessionMismatch);
        }
        self.windows.remove(window);
        self.pending
            .retain(|_, pending| &pending.window_id != window);
        Ok(())
    }

    pub fn accept(&mut self, request: RequestEnvelope) -> Result<AcceptedRequest, IpcError> {
        if request.protocol_version != IPC_PROTOCOL_VERSION {
            return Err(IpcError::UnsupportedProtocol);
        }
        request.command.validate_payload()?;
        let session = self
            .windows
            .get(&request.window_id)
            .ok_or(IpcError::UnknownWindow)?;
        if session.scope.session_id != request.session_id {
            return Err(IpcError::SessionMismatch);
        }
        if !command_allowed(session.scope.role, request.command.kind()) {
            return Err(IpcError::CommandOutOfScope);
        }
        let expected_sequence = session
            .last_sequence
            .checked_add(1)
            .ok_or(IpcError::SequenceOverflow)?;
        if request.sequence < expected_sequence {
            return Err(IpcError::Replay);
        }
        if request.sequence > expected_sequence {
            return Err(IpcError::SequenceGap);
        }
        if self.pending.contains_key(&request.request_id)
            || self.completed.contains(&request.request_id)
        {
            return Err(IpcError::DuplicateRequestId);
        }
        let validated_path = request
            .command
            .path_request()
            .map(|(path, usage)| session.scope.paths.validate(path, usage))
            .transpose()?;

        let command = request.command.kind();
        self.pending.insert(
            request.request_id.clone(),
            PendingRequest {
                window_id: request.window_id.clone(),
                session_id: request.session_id.clone(),
                sequence: request.sequence,
                command,
            },
        );
        self.windows
            .get_mut(&request.window_id)
            .ok_or(IpcError::UnknownWindow)?
            .last_sequence = request.sequence;

        Ok(AcceptedRequest {
            request,
            validated_path,
        })
    }

    pub fn accept_response(
        &mut self,
        response: ResponseEnvelope,
    ) -> Result<AcceptedResponse, IpcError> {
        if response.protocol_version != IPC_PROTOCOL_VERSION {
            return Err(IpcError::UnsupportedProtocol);
        }
        let pending = self
            .pending
            .get(&response.request_id)
            .ok_or(IpcError::StaleResponse)?;
        if pending.window_id != response.window_id
            || pending.session_id != response.session_id
            || pending.sequence != response.sequence
        {
            return Err(IpcError::ResponseScopeMismatch);
        }
        let command = pending.command;
        self.pending.remove(&response.request_id);
        self.completed.insert(response.request_id.clone());
        self.completed_order.push_back(response.request_id.clone());
        if self.completed_order.len() > COMPLETED_REQUEST_WINDOW
            && let Some(expired) = self.completed_order.pop_front()
        {
            self.completed.remove(&expired);
        }
        Ok(AcceptedResponse { response, command })
    }
}

#[derive(Clone)]
pub struct AcceptedRequest {
    pub request: RequestEnvelope,
    pub validated_path: Option<ValidatedPath>,
}

impl fmt::Debug for AcceptedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AcceptedRequest")
            .field("request", &self.request)
            .field("validated_path", &self.validated_path)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicErrorCode {
    InvalidRequest,
    Forbidden,
    Conflict,
    Busy,
    Unavailable,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandOutcome {
    Ok {
        revision: Option<u64>,
    },
    Error {
        code: PublicErrorCode,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub window_id: WindowId,
    pub session_id: SessionId,
    pub sequence: u64,
    pub outcome: CommandOutcome,
}

#[derive(Debug, Clone)]
pub struct AcceptedResponse {
    pub response: ResponseEnvelope,
    pub command: CommandKind,
}

fn command_allowed(role: WindowRole, command: CommandKind) -> bool {
    match role {
        WindowRole::Main => matches!(
            command,
            CommandKind::WindowOpen
                | CommandKind::DeviceEnumerate
                | CommandKind::RecoveryScan
                | CommandKind::Lifecycle
                | CommandKind::Update
        ),
        WindowRole::Recorder => matches!(
            command,
            CommandKind::RecorderPrepare
                | CommandKind::RecorderStart
                | CommandKind::RecorderPause
                | CommandKind::RecorderResume
                | CommandKind::RecorderStop
                | CommandKind::RecorderCancel
                | CommandKind::DeviceEnumerate
                | CommandKind::DeviceSelect
                | CommandKind::UploadStart
                | CommandKind::UploadPause
                | CommandKind::UploadResume
                | CommandKind::UploadCancel
                | CommandKind::RecorderConfigure
                | CommandKind::CaptureTargetSelect
        ),
        WindowRole::Recovery => matches!(
            command,
            CommandKind::RecoveryScan
                | CommandKind::RecoveryInspect
                | CommandKind::RecoveryOpen
                | CommandKind::RecoveryDiscard
        ),
        WindowRole::Editor => matches!(
            command,
            CommandKind::EditorOpen
                | CommandKind::EditorApply
                | CommandKind::EditorSave
                | CommandKind::ExportStart
                | CommandKind::ExportCancel
                | CommandKind::UploadStart
                | CommandKind::UploadPause
                | CommandKind::UploadResume
                | CommandKind::UploadCancel
        ),
        WindowRole::Export => {
            matches!(
                command,
                CommandKind::ExportStart | CommandKind::ExportCancel
            )
        }
        WindowRole::Settings => {
            matches!(
                command,
                CommandKind::SettingsApply | CommandKind::PresetApply
            )
        }
        WindowRole::Overlay => matches!(
            command,
            CommandKind::RecorderPause
                | CommandKind::RecorderResume
                | CommandKind::RecorderStop
                | CommandKind::RecorderCancel
                | CommandKind::Lifecycle
        ),
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    #[error("IPC identifier is invalid")]
    InvalidIdentifier,
    #[error("IPC envelope exceeds the size limit")]
    EnvelopeTooLarge,
    #[error("IPC envelope is malformed")]
    MalformedEnvelope,
    #[error("IPC command is unknown")]
    UnknownCommand,
    #[error("IPC payload is invalid")]
    InvalidPayload,
    #[error("IPC protocol version is unsupported")]
    UnsupportedProtocol,
    #[error("window is not registered")]
    UnknownWindow,
    #[error("window is already registered")]
    WindowAlreadyRegistered,
    #[error("window session does not match")]
    SessionMismatch,
    #[error("command is outside the window capability")]
    CommandOutOfScope,
    #[error("IPC request was replayed")]
    Replay,
    #[error("IPC request sequence has a gap")]
    SequenceGap,
    #[error("IPC request sequence overflowed")]
    SequenceOverflow,
    #[error("IPC request ID is already pending")]
    DuplicateRequestId,
    #[error("path scope is invalid")]
    InvalidPathScope,
    #[error("path scope is duplicated")]
    DuplicatePathScope,
    #[error("filesystem path is invalid")]
    InvalidPath,
    #[error("filesystem path is outside the approved scope")]
    PathOutOfScope,
    #[error("filesystem path type is unsupported")]
    UnsupportedPathType,
    #[error("IPC response is stale or already consumed")]
    StaleResponse,
    #[error("IPC response does not match its request scope")]
    ResponseScopeMismatch,
}

impl IpcError {
    #[must_use]
    pub const fn public_code(self) -> PublicErrorCode {
        match self {
            Self::CommandOutOfScope
            | Self::PathOutOfScope
            | Self::SessionMismatch
            | Self::UnknownWindow => PublicErrorCode::Forbidden,
            Self::Replay
            | Self::SequenceGap
            | Self::DuplicateRequestId
            | Self::StaleResponse
            | Self::ResponseScopeMismatch
            | Self::WindowAlreadyRegistered => PublicErrorCode::Conflict,
            Self::SequenceOverflow => PublicErrorCode::Internal,
            _ => PublicErrorCode::InvalidRequest,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(command: IpcCommand, sequence: u64, id: &str) -> RequestEnvelope {
        RequestEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: RequestId::new(id).expect("request ID"),
            window_id: WindowId::new("editor-main").expect("window ID"),
            session_id: SessionId::new("session-1").expect("session ID"),
            sequence,
            command,
        }
    }

    fn registry() -> ScopeRegistry {
        let paths = PathPolicy::empty()
            .allow_root(
                "/safe/projects",
                RootAccess {
                    read: true,
                    write: false,
                    delete: false,
                },
            )
            .expect("project scope")
            .allow_root(
                "/safe/exports",
                RootAccess {
                    read: true,
                    write: true,
                    delete: false,
                },
            )
            .expect("export scope");
        let mut registry = ScopeRegistry::new();
        registry
            .register(WindowScope {
                window_id: WindowId::new("editor-main").expect("window"),
                session_id: SessionId::new("session-1").expect("session"),
                role: WindowRole::Editor,
                paths,
            })
            .expect("register");
        registry
    }

    #[test]
    fn decode_rejects_unknown_and_malformed_commands_safely() {
        let unknown = r#"{"protocol_version":1,"request_id":"r1","window_id":"w1","session_id":"s1","sequence":1,"command":"run_shell","payload":{}}"#;
        assert_eq!(decode_request(unknown), Err(IpcError::UnknownCommand));
        assert_eq!(decode_request("{secret"), Err(IpcError::MalformedEnvelope));
        assert!(!IpcError::MalformedEnvelope.to_string().contains("secret"));
    }

    #[test]
    fn known_envelopes_round_trip_without_debugging_sensitive_payloads() {
        let original = request(
            IpcCommand::EditorOpen {
                project_path: "/safe/projects/private-project.frame".into(),
            },
            1,
            "request-private",
        );
        let json = serde_json::to_string(&original).expect("serialize request");
        let decoded = decode_request(&json).expect("decode request");

        assert_eq!(decoded, original);
        let rendered = format!("{decoded:?}");
        assert!(!rendered.contains("private-project.frame"));
        assert!(!rendered.contains("request-private"));
        assert!(!rendered.contains("session-1"));
    }

    #[test]
    fn window_allowlist_precedes_state_mutation() {
        let mut registry = registry();
        let forbidden = request(IpcCommand::RecorderPrepare, 1, "request-1");
        assert!(matches!(
            registry.accept(forbidden),
            Err(IpcError::CommandOutOfScope)
        ));
        let accepted = request(
            IpcCommand::EditorOpen {
                project_path: "/safe/projects/demo.frame".into(),
            },
            1,
            "request-2",
        );
        assert!(registry.accept(accepted).is_ok());
    }

    #[test]
    fn path_scope_rejects_traversal_wrong_extension_and_wrong_root() {
        let policy = PathPolicy::empty()
            .allow_root(
                "/safe/projects",
                RootAccess {
                    read: true,
                    write: false,
                    delete: false,
                },
            )
            .expect("scope");
        assert!(matches!(
            policy.validate("/safe/projects/../secret.frame", PathUse::ProjectRead),
            Err(IpcError::InvalidPath)
        ));
        assert!(matches!(
            policy.validate("/safe/projects/script.sh", PathUse::ProjectRead),
            Err(IpcError::UnsupportedPathType)
        ));
        assert!(matches!(
            policy.validate("/other/demo.frame", PathUse::ProjectRead),
            Err(IpcError::PathOutOfScope)
        ));
        let accepted = policy
            .validate("/safe/projects/demo.frame", PathUse::ProjectRead)
            .expect("path");
        assert!(accepted.requires_no_follow());
        assert!(!format!("{accepted:?}").contains("demo.frame"));
    }

    #[test]
    fn request_sequence_replay_and_gap_are_rejected() {
        let mut registry = registry();
        registry
            .accept(request(
                IpcCommand::EditorSave {
                    expected_revision: 1,
                },
                1,
                "request-1",
            ))
            .expect("first");
        assert!(matches!(
            registry.accept(request(
                IpcCommand::EditorSave {
                    expected_revision: 1,
                },
                1,
                "request-2"
            )),
            Err(IpcError::Replay)
        ));
        assert!(matches!(
            registry.accept(request(
                IpcCommand::EditorSave {
                    expected_revision: 1,
                },
                3,
                "request-3"
            )),
            Err(IpcError::SequenceGap)
        ));
    }

    #[test]
    fn response_scope_is_exact_and_duplicates_are_stale() {
        let mut registry = registry();
        let accepted = registry
            .accept(request(
                IpcCommand::EditorSave {
                    expected_revision: 1,
                },
                1,
                "request-1",
            ))
            .expect("request");
        let response = ResponseEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: accepted.request.request_id.clone(),
            window_id: accepted.request.window_id.clone(),
            session_id: accepted.request.session_id.clone(),
            sequence: accepted.request.sequence,
            outcome: CommandOutcome::Ok { revision: Some(2) },
        };
        registry
            .accept_response(response.clone())
            .expect("response");
        assert!(matches!(
            registry.accept_response(response),
            Err(IpcError::StaleResponse)
        ));
    }

    #[test]
    fn session_ids_are_never_rendered_in_debug_output() {
        let session = SessionId::new("private-session-token").expect("session");
        assert!(!format!("{session:?}").contains("private-session-token"));
    }
}
