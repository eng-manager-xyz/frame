//! Provider-neutral microphone, system-audio, and camera capture contracts.
//!
//! Native objects remain behind [`NativeAvBridge`]. This module owns identity,
//! negotiation, timing, bounded ingress, session races, and privacy-safe UI and
//! diagnostic output; it deliberately contains no platform or GStreamer type.

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    time::Duration,
};

use thiserror::Error;

use crate::{AudioFormat, CameraFormat, FrameTimestamp, PermissionState, SourceClass};

pub const AV_CAPTURE_CONTRACT_VERSION: u16 = 1;
pub const AV_SETTINGS_VERSION: u16 = 2;
pub const AV_DIAGNOSTIC_VERSION: u16 = 1;
pub const MAX_AV_DEVICES: usize = 128;
pub const MAX_FORMATS_PER_DEVICE: usize = 64;
pub const MAX_AV_QUEUE_BUFFERS: u16 = 512;
pub const MAX_AV_QUEUE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_AV_QUEUE_AGE_NS: u64 = 5_000_000_000;
pub const DEFAULT_UI_EVENT_INTERVAL_NS: u64 = 100_000_000;
pub const START_SYNC_BUDGET_NS: u64 = 80_000_000;
pub const LONG_SYNC_BUDGET_NS: u64 = 50_000_000;
pub const MAX_CALIBRATION_SAMPLES: usize = 31;
pub const MAX_GAIN_MILLI: u16 = 4_000;
pub const MAX_GAIN_RAMP_FRAMES: u32 = 480_000;
pub const MAX_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_PERSISTED_AV_SETTINGS_BYTES: usize = 4_096;

macro_rules! opaque_id {
    ($name:ident, $error:ident, $debug:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);

        impl $name {
            pub fn from_opaque(bytes: [u8; 16]) -> Result<Self, AvCaptureError> {
                if bytes.iter().all(|byte| *byte == 0) {
                    return Err(AvCaptureError::$error);
                }
                Ok(Self(bytes))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str($debug)
            }
        }
    };
}

opaque_id!(AvDeviceId, InvalidDeviceId, "AvDeviceId(<redacted>)");
opaque_id!(
    AvAdapterInstanceId,
    InvalidAdapterInstanceId,
    "AvAdapterInstanceId(<redacted>)"
);
opaque_id!(AvSessionId, InvalidSessionId, "AvSessionId(<redacted>)");

impl AvDeviceId {
    /// Stable, label-free persisted representation of the provider-owned ID.
    #[must_use]
    pub fn to_persisted_hex(self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut encoded = String::with_capacity(32);
        for byte in self.0 {
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        encoded
    }

    pub fn from_persisted_hex(encoded: &str) -> Result<Self, AvCaptureError> {
        if encoded.len() != 32 || !encoded.is_ascii() {
            return Err(AvCaptureError::MalformedPersistedSettings);
        }
        let mut bytes = [0_u8; 16];
        for (index, destination) in bytes.iter_mut().enumerate() {
            let offset = index * 2;
            *destination = u8::from_str_radix(&encoded[offset..offset + 2], 16)
                .map_err(|_| AvCaptureError::MalformedPersistedSettings)?;
        }
        Self::from_opaque(bytes).map_err(|_| AvCaptureError::MalformedPersistedSettings)
    }
}

impl AvSessionId {
    /// The caller must supply 128 bits from a cryptographically secure RNG.
    pub fn from_csprng(bytes: [u8; 16]) -> Result<Self, AvCaptureError> {
        Self::from_opaque(bytes)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AvDeviceGeneration(u64);

impl AvDeviceGeneration {
    pub fn new(value: u64) -> Result<Self, AvCaptureError> {
        if value == 0 {
            return Err(AvCaptureError::InvalidDeviceGeneration);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for AvDeviceGeneration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AvDeviceGeneration(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AvSourceClass {
    Microphone,
    SystemAudio,
    Camera,
}

impl AvSourceClass {
    #[must_use]
    pub const fn generic_source_class(self) -> SourceClass {
        match self {
            Self::Microphone => SourceClass::Microphone,
            Self::SystemAudio => SourceClass::SystemAudio,
            Self::Camera => SourceClass::Camera,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NativeRouteClass {
    BuiltIn,
    Wired,
    WirelessWideband,
    WirelessTelephony,
    Virtual,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NativeTimestampKind {
    HostMonotonic,
    DeviceMonotonic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AvFormat {
    Audio(AudioFormat),
    Camera(CameraFormat),
}

impl AvFormat {
    pub fn validate_for(self, class: AvSourceClass) -> Result<Self, AvCaptureError> {
        match (class, self) {
            (AvSourceClass::Microphone | AvSourceClass::SystemAudio, Self::Audio(format)) => {
                format.validate()?;
            }
            (AvSourceClass::Camera, Self::Camera(format)) => {
                format.validate()?;
                let pixels = u64::from(format.width)
                    .checked_mul(u64::from(format.height))
                    .ok_or(AvCaptureError::FormatTooLarge)?;
                if pixels > 132_710_400 || format.frame_rate_numerator > 1_000_000 {
                    return Err(AvCaptureError::FormatTooLarge);
                }
            }
            _ => return Err(AvCaptureError::FormatClassMismatch),
        }
        Ok(self)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AvDeviceDescriptor {
    id: AvDeviceId,
    generation: AvDeviceGeneration,
    class: AvSourceClass,
    default: bool,
    permission: PermissionState,
    route: NativeRouteClass,
    timestamp_kind: NativeTimestampKind,
    formats: Vec<AvFormat>,
}

impl AvDeviceDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AvDeviceId,
        generation: AvDeviceGeneration,
        class: AvSourceClass,
        default: bool,
        permission: PermissionState,
        route: NativeRouteClass,
        timestamp_kind: NativeTimestampKind,
        formats: Vec<AvFormat>,
    ) -> Result<Self, AvCaptureError> {
        if formats.is_empty() || formats.len() > MAX_FORMATS_PER_DEVICE {
            return Err(AvCaptureError::InvalidFormatCount);
        }
        let mut unique = BTreeSet::new();
        for format in &formats {
            format.validate_for(class)?;
            if !unique.insert(*format) {
                return Err(AvCaptureError::DuplicateFormat);
            }
        }
        Ok(Self {
            id,
            generation,
            class,
            default,
            permission,
            route,
            timestamp_kind,
            formats,
        })
    }

    #[must_use]
    pub const fn id(&self) -> AvDeviceId {
        self.id
    }

    #[must_use]
    pub const fn generation(&self) -> AvDeviceGeneration {
        self.generation
    }

    #[must_use]
    pub const fn class(&self) -> AvSourceClass {
        self.class
    }

    #[must_use]
    pub const fn is_default(&self) -> bool {
        self.default
    }

    #[must_use]
    pub const fn permission(&self) -> PermissionState {
        self.permission
    }

    #[must_use]
    pub const fn route(&self) -> NativeRouteClass {
        self.route
    }

    #[must_use]
    pub const fn timestamp_kind(&self) -> NativeTimestampKind {
        self.timestamp_kind
    }

    #[must_use]
    pub fn formats(&self) -> &[AvFormat] {
        &self.formats
    }

    #[must_use]
    pub fn supports(&self, format: AvFormat) -> bool {
        self.formats.contains(&format)
    }
}

impl fmt::Debug for AvDeviceDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvDeviceDescriptor")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("class", &self.class)
            .field("default", &self.default)
            .field("permission", &self.permission)
            .field("route", &self.route)
            .field("timestamp_kind", &self.timestamp_kind)
            .field("formats", &self.formats)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvDeviceCatalog {
    adapter: AvAdapterInstanceId,
    revision: u64,
    devices: Vec<AvDeviceDescriptor>,
}

impl AvDeviceCatalog {
    pub fn new(
        adapter: AvAdapterInstanceId,
        revision: u64,
        devices: Vec<AvDeviceDescriptor>,
    ) -> Result<Self, AvCaptureError> {
        if revision == 0 || devices.len() > MAX_AV_DEVICES {
            return Err(AvCaptureError::InvalidCatalog);
        }
        let mut identities = BTreeSet::new();
        let mut defaults = BTreeSet::new();
        for device in &devices {
            if !identities.insert((device.class(), device.id())) {
                return Err(AvCaptureError::DuplicateDevice);
            }
            if device.is_default() && !defaults.insert(device.class()) {
                return Err(AvCaptureError::MultipleDefaults);
            }
        }
        Ok(Self {
            adapter,
            revision,
            devices,
        })
    }

    #[must_use]
    pub const fn adapter(&self) -> AvAdapterInstanceId {
        self.adapter
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn devices(&self) -> &[AvDeviceDescriptor] {
        &self.devices
    }

    fn exact(&self, class: AvSourceClass, id: AvDeviceId) -> Option<&AvDeviceDescriptor> {
        self.devices
            .iter()
            .find(|device| device.class() == class && device.id() == id)
    }

    fn default_for(&self, class: AvSourceClass) -> Option<&AvDeviceDescriptor> {
        self.devices
            .iter()
            .find(|device| device.class() == class && device.is_default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAvBridgeCapabilities {
    pub contract_version: u16,
    pub adapter: AvAdapterInstanceId,
    pub permission_prompt: bool,
    pub hotplug_events: bool,
    pub default_change_events: bool,
    pub sleep_wake_events: bool,
    pub bounded_nonblocking_ingress: bool,
    pub explicit_timestamps: bool,
    pub discontinuity_signaling: bool,
    pub latency_reporting: bool,
}

impl NativeAvBridgeCapabilities {
    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.contract_version != AV_CAPTURE_CONTRACT_VERSION {
            return Err(AvCaptureError::ContractVersionMismatch);
        }
        if !self.hotplug_events
            || !self.default_change_events
            || !self.sleep_wake_events
            || !self.bounded_nonblocking_ingress
            || !self.explicit_timestamps
            || !self.discontinuity_signaling
            || !self.latency_reporting
        {
            return Err(AvCaptureError::MissingBridgeCapability);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSelectionV2 {
    Disabled,
    Pinned {
        id: AvDeviceId,
        format: AvFormat,
    },
    FollowDefault {
        format: AvFormat,
        allow_default_changes: bool,
        confirmed_id: Option<AvDeviceId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvCaptureSettingsV2 {
    pub version: u16,
    pub microphone: DeviceSelectionV2,
    pub system_audio: DeviceSelectionV2,
    pub camera: DeviceSelectionV2,
}

impl AvCaptureSettingsV2 {
    #[must_use]
    pub const fn screen_only() -> Self {
        Self {
            version: AV_SETTINGS_VERSION,
            microphone: DeviceSelectionV2::Disabled,
            system_audio: DeviceSelectionV2::Disabled,
            camera: DeviceSelectionV2::Disabled,
        }
    }

    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.version != AV_SETTINGS_VERSION {
            return Err(AvCaptureError::SettingsVersionMismatch);
        }
        validate_selection(AvSourceClass::Microphone, self.microphone)?;
        validate_selection(AvSourceClass::SystemAudio, self.system_audio)?;
        validate_selection(AvSourceClass::Camera, self.camera)?;
        Ok(self)
    }

    #[must_use]
    pub const fn selection(self, class: AvSourceClass) -> DeviceSelectionV2 {
        match class {
            AvSourceClass::Microphone => self.microphone,
            AvSourceClass::SystemAudio => self.system_audio,
            AvSourceClass::Camera => self.camera,
        }
    }
}

pub struct AvSettingsCodec;

impl AvSettingsCodec {
    pub fn encode(settings: AvCaptureSettingsV2) -> Result<Vec<u8>, AvCaptureError> {
        let settings = settings.validate()?;
        let encoded = format!(
            "frame-av-settings\nversion={}\nmicrophone={}\nsystem_audio={}\ncamera={}",
            AV_SETTINGS_VERSION,
            encode_selection(settings.microphone),
            encode_selection(settings.system_audio),
            encode_selection(settings.camera),
        );
        if encoded.len() > MAX_PERSISTED_AV_SETTINGS_BYTES {
            return Err(AvCaptureError::PersistedSettingsTooLarge);
        }
        Ok(encoded.into_bytes())
    }

    pub fn decode(bytes: &[u8]) -> Result<AvCaptureSettingsV2, AvCaptureError> {
        if bytes.len() > MAX_PERSISTED_AV_SETTINGS_BYTES {
            return Err(AvCaptureError::PersistedSettingsTooLarge);
        }
        let text =
            std::str::from_utf8(bytes).map_err(|_| AvCaptureError::MalformedPersistedSettings)?;
        let lines: Vec<_> = text.lines().collect();
        if lines.len() != 5 || lines[0] != "frame-av-settings" {
            return Err(AvCaptureError::MalformedPersistedSettings);
        }
        let version = lines[1]
            .strip_prefix("version=")
            .ok_or(AvCaptureError::MalformedPersistedSettings)?
            .parse::<u16>()
            .map_err(|_| AvCaptureError::MalformedPersistedSettings)?;
        if version != AV_SETTINGS_VERSION {
            return Err(AvCaptureError::SettingsVersionMismatch);
        }
        let microphone = decode_selection_line(lines[2], "microphone=", AvSourceClass::Microphone)?;
        let system_audio =
            decode_selection_line(lines[3], "system_audio=", AvSourceClass::SystemAudio)?;
        let camera = decode_selection_line(lines[4], "camera=", AvSourceClass::Camera)?;
        AvCaptureSettingsV2 {
            version,
            microphone,
            system_audio,
            camera,
        }
        .validate()
    }
}

fn encode_selection(selection: DeviceSelectionV2) -> String {
    match selection {
        DeviceSelectionV2::Disabled => "d".to_owned(),
        DeviceSelectionV2::Pinned { id, format } => {
            format!("p;{};{}", id.to_persisted_hex(), encode_format(format))
        }
        DeviceSelectionV2::FollowDefault {
            format,
            allow_default_changes,
            confirmed_id,
        } => format!(
            "f;{};{};{}",
            u8::from(allow_default_changes),
            confirmed_id.map_or_else(|| "-".to_owned(), AvDeviceId::to_persisted_hex),
            encode_format(format)
        ),
    }
}

fn encode_format(format: AvFormat) -> String {
    match format {
        AvFormat::Audio(format) => format!(
            "a:{}:{}:{}",
            format.sample_rate,
            format.channels,
            match format.sample_format {
                crate::AudioSampleFormat::Signed16 => "s16",
                crate::AudioSampleFormat::Signed32 => "s32",
                crate::AudioSampleFormat::Float32 => "f32",
            }
        ),
        AvFormat::Camera(format) => format!(
            "c:{}:{}:{}:{}:{}",
            format.width,
            format.height,
            format.frame_rate_numerator,
            format.frame_rate_denominator,
            match format.pixel_format {
                crate::PixelFormat::Bgra8 => "bgra8",
                crate::PixelFormat::Rgba8 => "rgba8",
                crate::PixelFormat::Nv12 => "nv12",
                crate::PixelFormat::I420 => "i420",
            }
        ),
    }
}

fn decode_selection_line(
    line: &str,
    prefix: &str,
    class: AvSourceClass,
) -> Result<DeviceSelectionV2, AvCaptureError> {
    let encoded = line
        .strip_prefix(prefix)
        .ok_or(AvCaptureError::MalformedPersistedSettings)?;
    let parts: Vec<_> = encoded.split(';').collect();
    let selection = match parts.as_slice() {
        ["d"] => DeviceSelectionV2::Disabled,
        ["p", id, format] => DeviceSelectionV2::Pinned {
            id: AvDeviceId::from_persisted_hex(id)?,
            format: decode_format(format)?,
        },
        ["f", allow, confirmed, format] => DeviceSelectionV2::FollowDefault {
            allow_default_changes: match *allow {
                "0" => false,
                "1" => true,
                _ => return Err(AvCaptureError::MalformedPersistedSettings),
            },
            confirmed_id: if *confirmed == "-" {
                None
            } else {
                Some(AvDeviceId::from_persisted_hex(confirmed)?)
            },
            format: decode_format(format)?,
        },
        _ => return Err(AvCaptureError::MalformedPersistedSettings),
    };
    validate_selection(class, selection)?;
    Ok(selection)
}

fn decode_format(encoded: &str) -> Result<AvFormat, AvCaptureError> {
    let parts: Vec<_> = encoded.split(':').collect();
    match parts.as_slice() {
        ["a", sample_rate, channels, sample_format] => Ok(AvFormat::Audio(AudioFormat {
            sample_rate: parse_setting_number(sample_rate)?,
            channels: parse_setting_number(channels)?,
            sample_format: match *sample_format {
                "s16" => crate::AudioSampleFormat::Signed16,
                "s32" => crate::AudioSampleFormat::Signed32,
                "f32" => crate::AudioSampleFormat::Float32,
                _ => return Err(AvCaptureError::MalformedPersistedSettings),
            },
        })),
        ["c", width, height, numerator, denominator, pixel_format] => {
            Ok(AvFormat::Camera(CameraFormat {
                width: parse_setting_number(width)?,
                height: parse_setting_number(height)?,
                frame_rate_numerator: parse_setting_number(numerator)?,
                frame_rate_denominator: parse_setting_number(denominator)?,
                pixel_format: match *pixel_format {
                    "bgra8" => crate::PixelFormat::Bgra8,
                    "rgba8" => crate::PixelFormat::Rgba8,
                    "nv12" => crate::PixelFormat::Nv12,
                    "i420" => crate::PixelFormat::I420,
                    _ => return Err(AvCaptureError::MalformedPersistedSettings),
                },
            }))
        }
        _ => Err(AvCaptureError::MalformedPersistedSettings),
    }
}

fn parse_setting_number<T: std::str::FromStr>(encoded: &str) -> Result<T, AvCaptureError> {
    encoded
        .parse()
        .map_err(|_| AvCaptureError::MalformedPersistedSettings)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("A/V settings storage is unavailable")]
pub struct AvSettingsStorageError;

pub trait AvSettingsStorage {
    /// Implementations must not read or return more than `max_bytes`.
    fn load(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, AvSettingsStorageError>;
    fn store(&mut self, encoded: &[u8]) -> Result<(), AvSettingsStorageError>;
}

pub fn load_persisted_av_settings<S: AvSettingsStorage>(
    storage: &mut S,
) -> Result<Option<AvCaptureSettingsV2>, AvCaptureError> {
    storage
        .load(MAX_PERSISTED_AV_SETTINGS_BYTES)?
        .as_deref()
        .map(AvSettingsCodec::decode)
        .transpose()
}

pub fn store_persisted_av_settings<S: AvSettingsStorage>(
    storage: &mut S,
    settings: AvCaptureSettingsV2,
) -> Result<(), AvCaptureError> {
    let encoded = AvSettingsCodec::encode(settings)?;
    storage.store(&encoded)?;
    Ok(())
}

fn validate_selection(
    class: AvSourceClass,
    selection: DeviceSelectionV2,
) -> Result<(), AvCaptureError> {
    match selection {
        DeviceSelectionV2::Disabled => Ok(()),
        DeviceSelectionV2::Pinned { format, .. }
        | DeviceSelectionV2::FollowDefault { format, .. } => format.validate_for(class).map(|_| ()),
    }
}

/// Legacy settings intentionally exclude labels. Migrating a legacy default
/// never authorizes a later default-device switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeviceSelectionV1 {
    pub id: Option<AvDeviceId>,
    pub format: Option<AvFormat>,
    pub followed_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyAvCaptureSettingsV1 {
    pub microphone: LegacyDeviceSelectionV1,
    pub system_audio: LegacyDeviceSelectionV1,
    pub camera: LegacyDeviceSelectionV1,
}

impl LegacyAvCaptureSettingsV1 {
    pub fn migrate(self) -> Result<AvCaptureSettingsV2, AvCaptureError> {
        Ok(AvCaptureSettingsV2 {
            version: AV_SETTINGS_VERSION,
            microphone: migrate_selection(AvSourceClass::Microphone, self.microphone)?,
            system_audio: migrate_selection(AvSourceClass::SystemAudio, self.system_audio)?,
            camera: migrate_selection(AvSourceClass::Camera, self.camera)?,
        })
    }
}

fn migrate_selection(
    class: AvSourceClass,
    legacy: LegacyDeviceSelectionV1,
) -> Result<DeviceSelectionV2, AvCaptureError> {
    match (legacy.id, legacy.format) {
        (None, None) => Ok(DeviceSelectionV2::Disabled),
        (Some(id), Some(format)) => {
            format.validate_for(class)?;
            if legacy.followed_default {
                Ok(DeviceSelectionV2::FollowDefault {
                    format,
                    allow_default_changes: false,
                    confirmed_id: Some(id),
                })
            } else {
                Ok(DeviceSelectionV2::Pinned { id, format })
            }
        }
        _ => Err(AvCaptureError::IncompleteLegacySelection),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionResolution {
    Disabled,
    Ready {
        id: AvDeviceId,
        generation: AvDeviceGeneration,
        format: AvFormat,
    },
    PermissionRequired {
        id: AvDeviceId,
    },
    Unavailable,
    ConfirmationRequired {
        candidate: AvDeviceId,
    },
}

pub fn resolve_selection(
    catalog: &AvDeviceCatalog,
    class: AvSourceClass,
    selection: DeviceSelectionV2,
) -> Result<SelectionResolution, AvCaptureError> {
    validate_selection(class, selection)?;
    let chosen = match selection {
        DeviceSelectionV2::Disabled => return Ok(SelectionResolution::Disabled),
        DeviceSelectionV2::Pinned { id, format } => catalog
            .exact(class, id)
            .map(|device| (device, format, true)),
        DeviceSelectionV2::FollowDefault {
            format,
            allow_default_changes,
            confirmed_id,
        } => catalog.default_for(class).map(|device| {
            let authorized = allow_default_changes || confirmed_id == Some(device.id());
            (device, format, authorized)
        }),
    };
    let Some((device, format, authorized)) = chosen else {
        return Ok(SelectionResolution::Unavailable);
    };
    if !authorized {
        return Ok(SelectionResolution::ConfirmationRequired {
            candidate: device.id(),
        });
    }
    if !device.supports(format) {
        return Err(AvCaptureError::UnsupportedExactFormat);
    }
    match device.permission() {
        PermissionState::Granted => Ok(SelectionResolution::Ready {
            id: device.id(),
            generation: device.generation(),
            format,
        }),
        PermissionState::Unknown | PermissionState::PromptRequired => {
            Ok(SelectionResolution::PermissionRequired { id: device.id() })
        }
        PermissionState::Denied | PermissionState::Restricted | PermissionState::Revoked => {
            Ok(SelectionResolution::Unavailable)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GstElementFamily {
    AppSrc,
    Queue,
    AudioConvert,
    AudioResample,
    AudioMixer,
    Volume,
    Level,
    VideoConvert,
    VideoScale,
    CapsFilter,
    Tee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvBackpressurePolicy {
    DropNewest,
    DropOldest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppSrcTimestampMode {
    ExplicitMasterCorrected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvAppSrcBridgeSpec {
    pub is_live: bool,
    pub do_timestamp: bool,
    pub block: bool,
    pub time_format_nanoseconds: bool,
    pub timestamp_mode: AppSrcTimestampMode,
    pub retain_native_lease_until_downstream_release: bool,
}

impl AvAppSrcBridgeSpec {
    fn audited() -> Self {
        Self {
            is_live: true,
            do_timestamp: false,
            block: false,
            time_format_nanoseconds: true,
            timestamp_mode: AppSrcTimestampMode::ExplicitMasterCorrected,
            retain_native_lease_until_downstream_release: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvQueueSpec {
    pub max_buffers: u16,
    pub max_bytes: u64,
    pub max_age_ns: u64,
    pub backpressure: AvBackpressurePolicy,
    pub producer_blocks: bool,
}

impl AvQueueSpec {
    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.max_buffers == 0
            || self.max_buffers > MAX_AV_QUEUE_BUFFERS
            || self.max_bytes == 0
            || self.max_bytes > MAX_AV_QUEUE_BYTES
            || self.max_age_ns == 0
            || self.max_age_ns > MAX_AV_QUEUE_AGE_NS
            || self.producer_blocks
        {
            return Err(AvCaptureError::InvalidQueueSpec);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioCapsSpec {
    pub format: AudioFormat,
    pub interleaved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraCapsSpec {
    pub format: CameraFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactCapsSpec {
    Audio(AudioCapsSpec),
    Camera(CameraCapsSpec),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvSourceGraphSpec {
    pub class: AvSourceClass,
    pub device: AvDeviceId,
    pub generation: AvDeviceGeneration,
    pub input_caps: ExactCapsSpec,
    pub output_caps: ExactCapsSpec,
    pub appsrc: AvAppSrcBridgeSpec,
    pub queue: AvQueueSpec,
    pub elements: Vec<GstElementFamily>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AudioMixerPadId {
    Microphone,
    SystemAudio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioMixerRequestPadSpec {
    pub class: AvSourceClass,
    pub pad: AudioMixerPadId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedAudioMixerTopology {
    pub element: GstElementFamily,
    pub request_pads: Vec<AudioMixerRequestPadSpec>,
    pub output_caps: AudioCapsSpec,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraTeeTopology {
    pub element: GstElementFamily,
    pub record_branch: Vec<GstElementFamily>,
    pub preview_branch: Option<Vec<GstElementFamily>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvPipelineGraphSpec {
    pub sources: Vec<AvSourceGraphSpec>,
    pub audio_mix_caps: AudioCapsSpec,
    pub shared_audio_mixer: Option<SharedAudioMixerTopology>,
    pub camera_tee: Option<CameraTeeTopology>,
    pub camera_preview_enabled: bool,
}

impl AvPipelineGraphSpec {
    pub fn negotiate(
        catalog: &AvDeviceCatalog,
        settings: AvCaptureSettingsV2,
        camera_preview_enabled: bool,
    ) -> Result<Self, AvCaptureError> {
        settings.validate()?;
        let mut sources = Vec::new();
        for class in [
            AvSourceClass::Microphone,
            AvSourceClass::SystemAudio,
            AvSourceClass::Camera,
        ] {
            match resolve_selection(catalog, class, settings.selection(class))? {
                SelectionResolution::Disabled | SelectionResolution::Unavailable => {}
                SelectionResolution::PermissionRequired { .. } => {
                    return Err(AvCaptureError::PermissionNotGranted);
                }
                SelectionResolution::ConfirmationRequired { .. } => {
                    return Err(AvCaptureError::DefaultChangeNeedsConfirmation);
                }
                SelectionResolution::Ready {
                    id,
                    generation,
                    format,
                } => sources.push(source_graph(class, id, generation, format)?),
            }
        }
        let preview_available = sources
            .iter()
            .any(|source| source.class == AvSourceClass::Camera);
        let audio_mix_caps = AudioCapsSpec {
            format: AudioFormat {
                sample_rate: 48_000,
                channels: 2,
                sample_format: crate::AudioSampleFormat::Float32,
            },
            interleaved: true,
        };
        let request_pads: Vec<_> = sources
            .iter()
            .filter_map(|source| match source.class {
                AvSourceClass::Microphone => Some(AudioMixerRequestPadSpec {
                    class: source.class,
                    pad: AudioMixerPadId::Microphone,
                }),
                AvSourceClass::SystemAudio => Some(AudioMixerRequestPadSpec {
                    class: source.class,
                    pad: AudioMixerPadId::SystemAudio,
                }),
                AvSourceClass::Camera => None,
            })
            .collect();
        let shared_audio_mixer = (!request_pads.is_empty()).then_some(SharedAudioMixerTopology {
            element: GstElementFamily::AudioMixer,
            request_pads,
            output_caps: audio_mix_caps,
        });
        let camera_tee = preview_available.then_some(CameraTeeTopology {
            element: GstElementFamily::Tee,
            record_branch: vec![GstElementFamily::Queue],
            preview_branch: camera_preview_enabled.then_some(vec![
                GstElementFamily::Queue,
                GstElementFamily::VideoConvert,
            ]),
        });
        Ok(Self {
            sources,
            audio_mix_caps,
            shared_audio_mixer,
            camera_tee,
            camera_preview_enabled: camera_preview_enabled && preview_available,
        })
    }

    #[must_use]
    pub fn source(&self, class: AvSourceClass) -> Option<&AvSourceGraphSpec> {
        self.sources.iter().find(|source| source.class == class)
    }
}

fn source_graph(
    class: AvSourceClass,
    device: AvDeviceId,
    generation: AvDeviceGeneration,
    format: AvFormat,
) -> Result<AvSourceGraphSpec, AvCaptureError> {
    format.validate_for(class)?;
    let queue = AvQueueSpec {
        max_buffers: if class == AvSourceClass::Camera {
            8
        } else {
            128
        },
        max_bytes: if class == AvSourceClass::Camera {
            128 * 1024 * 1024
        } else {
            8 * 1024 * 1024
        },
        max_age_ns: if class == AvSourceClass::Camera {
            500_000_000
        } else {
            2_000_000_000
        },
        backpressure: AvBackpressurePolicy::DropOldest,
        producer_blocks: false,
    }
    .validate()?;
    let (input_caps, output_caps, elements) = match format {
        AvFormat::Audio(input) => (
            ExactCapsSpec::Audio(AudioCapsSpec {
                format: input,
                interleaved: true,
            }),
            ExactCapsSpec::Audio(AudioCapsSpec {
                format: AudioFormat {
                    sample_rate: 48_000,
                    channels: 2,
                    sample_format: crate::AudioSampleFormat::Float32,
                },
                interleaved: true,
            }),
            vec![
                GstElementFamily::AppSrc,
                GstElementFamily::Queue,
                GstElementFamily::AudioConvert,
                GstElementFamily::AudioResample,
                GstElementFamily::CapsFilter,
                GstElementFamily::Volume,
                GstElementFamily::Level,
            ],
        ),
        AvFormat::Camera(input) => (
            ExactCapsSpec::Camera(CameraCapsSpec { format: input }),
            ExactCapsSpec::Camera(CameraCapsSpec { format: input }),
            vec![
                GstElementFamily::AppSrc,
                GstElementFamily::Queue,
                GstElementFamily::VideoConvert,
                GstElementFamily::VideoScale,
                GstElementFamily::CapsFilter,
            ],
        ),
    };
    Ok(AvSourceGraphSpec {
        class,
        device,
        generation,
        input_caps,
        output_caps,
        appsrc: AvAppSrcBridgeSpec::audited(),
        queue,
        elements,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MonotonicTimeNs(u64);

impl MonotonicTimeNs {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLatency {
    pub reported_ns: u64,
    pub confidence: LatencyConfidence,
}

impl SourceLatency {
    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.reported_ns > 5_000_000_000 {
            return Err(AvCaptureError::InvalidLatency);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LatencyConfidence {
    Unknown,
    Reported,
    Measured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalibrationSample {
    pub master_arrival: MonotonicTimeNs,
    pub source_pts_ns: u64,
    pub latency: SourceLatency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupCalibration {
    pub offset_ns: i64,
    pub spread_ns: u64,
    pub sample_count: u8,
    pub confidence: CalibrationConfidence,
}

impl StartupCalibration {
    pub fn measure(samples: &[CalibrationSample]) -> Result<Self, AvCaptureError> {
        if samples.len() < 3 || samples.len() > MAX_CALIBRATION_SAMPLES {
            return Err(AvCaptureError::InvalidCalibrationCount);
        }
        let mut prior_master = None;
        let mut prior_source = None;
        let mut offsets = Vec::with_capacity(samples.len());
        let mut aggregate_confidence = LatencyConfidence::Measured;
        for sample in samples {
            sample.latency.validate()?;
            if prior_master.is_some_and(|prior| sample.master_arrival.get() <= prior) {
                return Err(AvCaptureError::NonMonotonicMasterClock);
            }
            if prior_source.is_some_and(|prior| sample.source_pts_ns <= prior) {
                return Err(AvCaptureError::SourceTimestampRollback);
            }
            prior_master = Some(sample.master_arrival.get());
            prior_source = Some(sample.source_pts_ns);
            aggregate_confidence = aggregate_confidence.min(sample.latency.confidence);
            let capture_master =
                i128::from(sample.master_arrival.get()) - i128::from(sample.latency.reported_ns);
            let offset = capture_master - i128::from(sample.source_pts_ns);
            offsets.push(i64::try_from(offset).map_err(|_| AvCaptureError::TimestampRange)?);
        }
        offsets.sort_unstable();
        let median = offsets[offsets.len() / 2];
        let minimum = offsets[0];
        let maximum = offsets[offsets.len() - 1];
        let spread_ns = maximum.abs_diff(minimum);
        let confidence = match (samples.len(), spread_ns, aggregate_confidence) {
            (count, 0..=2_000_000, LatencyConfidence::Measured) if count >= 7 => {
                CalibrationConfidence::High
            }
            (count, 0..=10_000_000, LatencyConfidence::Reported | LatencyConfidence::Measured)
                if count >= 5 =>
            {
                CalibrationConfidence::Medium
            }
            _ => CalibrationConfidence::Low,
        };
        Ok(Self {
            offset_ns: median,
            spread_ns,
            sample_count: u8::try_from(samples.len())
                .map_err(|_| AvCaptureError::InvalidCalibrationCount)?,
            confidence,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvSyncPolicy {
    pub start_budget_ns: u64,
    pub long_budget_ns: u64,
    pub discontinuity_ns: u64,
    pub max_abs_drift_ppm: i32,
    pub max_correction_ns_per_second: u64,
}

impl Default for AvSyncPolicy {
    fn default() -> Self {
        Self {
            start_budget_ns: START_SYNC_BUDGET_NS,
            long_budget_ns: LONG_SYNC_BUDGET_NS,
            discontinuity_ns: 250_000_000,
            max_abs_drift_ppm: 5_000,
            max_correction_ns_per_second: 5_000_000,
        }
    }
}

impl AvSyncPolicy {
    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.start_budget_ns == 0
            || self.start_budget_ns > START_SYNC_BUDGET_NS
            || self.long_budget_ns == 0
            || self.long_budget_ns > LONG_SYNC_BUDGET_NS
            || self.discontinuity_ns <= self.start_budget_ns
            || self.max_abs_drift_ppm <= 0
            || self.max_abs_drift_ppm > 100_000
            || self.max_correction_ns_per_second == 0
            || self.max_correction_ns_per_second
                < u64::try_from(self.max_abs_drift_ppm)
                    .map_err(|_| AvCaptureError::InvalidSyncPolicyV2)?
                    .saturating_mul(1_000)
        {
            return Err(AvCaptureError::InvalidSyncPolicyV2);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectedTimestamp {
    pub frame: FrameTimestamp,
    pub observed_offset_ns: i64,
    pub estimated_drift_ppm: i32,
    pub calibration_confidence: CalibrationConfidence,
}

#[derive(Debug, Clone)]
pub struct SourceTimebase {
    policy: AvSyncPolicy,
    calibration: StartupCalibration,
    anchor_source_ns: u64,
    anchor_master_ns: u64,
    last_source_ns: u64,
    last_master_ns: u64,
    last_output_end_ns: u64,
    estimated_drift_ppm: f64,
    applied_correction_ns: i64,
    latency_confidence: LatencyConfidence,
    paused: bool,
    awaiting_discontinuity: bool,
}

impl SourceTimebase {
    pub fn new(
        policy: AvSyncPolicy,
        calibration: StartupCalibration,
        first: CalibrationSample,
    ) -> Result<Self, AvCaptureError> {
        policy.validate()?;
        first.latency.validate()?;
        let anchor_master = first
            .master_arrival
            .get()
            .checked_sub(first.latency.reported_ns)
            .ok_or(AvCaptureError::TimestampRange)?;
        let predicted = add_signed(first.source_pts_ns, calibration.offset_ns)?;
        if predicted.abs_diff(anchor_master) > policy.start_budget_ns {
            return Err(AvCaptureError::StartupOffsetBudgetExceeded);
        }
        Ok(Self {
            policy,
            calibration,
            anchor_source_ns: first.source_pts_ns,
            anchor_master_ns: anchor_master,
            last_source_ns: first.source_pts_ns,
            last_master_ns: anchor_master,
            last_output_end_ns: anchor_master,
            estimated_drift_ppm: 0.0,
            applied_correction_ns: 0,
            latency_confidence: first.latency.confidence,
            paused: false,
            awaiting_discontinuity: false,
        })
    }

    #[must_use]
    pub const fn calibration(&self) -> StartupCalibration {
        self.calibration
    }

    pub fn pause(&mut self, now: MonotonicTimeNs) -> Result<(), AvCaptureError> {
        if now.get() < self.last_master_ns {
            return Err(AvCaptureError::NonMonotonicMasterClock);
        }
        self.paused = true;
        self.awaiting_discontinuity = true;
        Ok(())
    }

    pub fn resume(&mut self, now: MonotonicTimeNs) -> Result<(), AvCaptureError> {
        if !self.paused || now.get() < self.last_master_ns {
            return Err(AvCaptureError::InvalidTimebaseTransition);
        }
        self.paused = false;
        self.awaiting_discontinuity = true;
        self.last_master_ns = now.get();
        Ok(())
    }

    pub fn observe(
        &mut self,
        source_pts_ns: u64,
        duration_ns: u64,
        master_arrival: MonotonicTimeNs,
        latency: SourceLatency,
        discontinuity: bool,
    ) -> Result<CorrectedTimestamp, AvCaptureError> {
        latency.validate()?;
        if self.paused {
            return Err(AvCaptureError::TimebasePaused);
        }
        if latency.confidence != self.latency_confidence
            && !discontinuity
            && !self.awaiting_discontinuity
        {
            return Err(AvCaptureError::ClockDiscontinuityRequired);
        }
        let capture_master = master_arrival
            .get()
            .checked_sub(latency.reported_ns)
            .ok_or(AvCaptureError::TimestampRange)?;
        if capture_master < self.last_master_ns {
            return Err(AvCaptureError::NonMonotonicMasterClock);
        }
        let source_rolled_back = source_pts_ns < self.last_source_ns;
        if source_rolled_back && !discontinuity && !self.awaiting_discontinuity {
            return Err(AvCaptureError::SourceTimestampRollback);
        }
        if discontinuity || self.awaiting_discontinuity || source_rolled_back {
            self.anchor_source_ns = source_pts_ns;
            self.anchor_master_ns = self.last_output_end_ns;
            self.last_source_ns = source_pts_ns;
            self.last_master_ns = capture_master;
            self.estimated_drift_ppm = 0.0;
            self.applied_correction_ns = 0;
            self.latency_confidence = latency.confidence;
            self.awaiting_discontinuity = false;
            let frame = FrameTimestamp {
                pts_ns: self.last_output_end_ns,
                duration_ns,
                discontinuity: true,
            };
            validate_frame_timestamp(frame)?;
            self.last_output_end_ns = frame.end_ns();
            return Ok(CorrectedTimestamp {
                frame,
                observed_offset_ns: signed_difference(capture_master, frame.pts_ns)?,
                estimated_drift_ppm: 0,
                calibration_confidence: self.calibration.confidence,
            });
        }
        if source_pts_ns == self.last_source_ns {
            return Err(AvCaptureError::SourceTimestampRollback);
        }
        let source_step = source_pts_ns - self.last_source_ns;
        let master_step = capture_master
            .checked_sub(self.last_master_ns)
            .ok_or(AvCaptureError::TimestampRange)?;
        if source_step.abs_diff(master_step) >= self.policy.discontinuity_ns {
            return Err(AvCaptureError::ClockDiscontinuityRequired);
        }
        let source_elapsed = source_pts_ns - self.anchor_source_ns;
        let master_elapsed = capture_master
            .checked_sub(self.anchor_master_ns)
            .ok_or(AvCaptureError::TimestampRange)?;
        if master_elapsed == 0 {
            return Err(AvCaptureError::NonMonotonicMasterClock);
        }
        let observed_ppm =
            ((source_elapsed as f64 - master_elapsed as f64) / master_elapsed as f64) * 1_000_000.0;
        let calibration_uncertainty_ppm =
            (self.calibration.spread_ns as f64 / master_elapsed as f64) * 1_000_000.0;
        if !observed_ppm.is_finite()
            || observed_ppm.abs()
                > f64::from(self.policy.max_abs_drift_ppm) + calibration_uncertainty_ppm
        {
            return Err(AvCaptureError::ClockDiscontinuityRequired);
        }
        self.estimated_drift_ppm = if master_elapsed < 1_000_000_000 {
            observed_ppm
        } else {
            (self.estimated_drift_ppm * 0.75) + (observed_ppm * 0.25)
        };
        let desired_correction =
            (source_elapsed as f64 * self.estimated_drift_ppm / 1_000_000.0).round();
        let maximum_correction_step = u128::from(self.policy.max_correction_ns_per_second)
            .saturating_mul(u128::from(master_step))
            / 1_000_000_000;
        let maximum_correction_step = maximum_correction_step.min(i64::MAX as u128) as i64;
        let correction_step = (desired_correction as i64)
            .saturating_sub(self.applied_correction_ns)
            .clamp(-maximum_correction_step, maximum_correction_step);
        self.applied_correction_ns = self.applied_correction_ns.saturating_add(correction_step);
        let uncorrected = self
            .anchor_master_ns
            .checked_add(source_elapsed)
            .ok_or(AvCaptureError::TimestampRange)?;
        let candidate = add_signed(uncorrected, -self.applied_correction_ns)?;
        let pts_ns = candidate.max(self.last_output_end_ns);
        let frame = FrameTimestamp {
            pts_ns,
            duration_ns,
            discontinuity: false,
        };
        validate_frame_timestamp(frame)?;
        let observed_offset_ns = signed_difference(capture_master, pts_ns)?;
        if observed_offset_ns.unsigned_abs() > self.policy.long_budget_ns {
            return Err(AvCaptureError::SynchronizationBudgetExceeded);
        }
        self.last_source_ns = source_pts_ns;
        self.last_master_ns = capture_master;
        self.last_output_end_ns = frame.end_ns();
        Ok(CorrectedTimestamp {
            frame,
            observed_offset_ns,
            estimated_drift_ppm: self.estimated_drift_ppm.round() as i32,
            calibration_confidence: self.calibration.confidence,
        })
    }
}

fn validate_frame_timestamp(frame: FrameTimestamp) -> Result<(), AvCaptureError> {
    if frame.duration_ns == 0 || frame.pts_ns.checked_add(frame.duration_ns).is_none() {
        return Err(AvCaptureError::InvalidFrameTimestamp);
    }
    Ok(())
}

fn add_signed(value: u64, delta: i64) -> Result<u64, AvCaptureError> {
    if delta >= 0 {
        value
            .checked_add(delta.unsigned_abs())
            .ok_or(AvCaptureError::TimestampRange)
    } else {
        value
            .checked_sub(delta.unsigned_abs())
            .ok_or(AvCaptureError::TimestampRange)
    }
}

fn signed_difference(left: u64, right: u64) -> Result<i64, AvCaptureError> {
    i64::try_from(i128::from(left) - i128::from(right)).map_err(|_| AvCaptureError::TimestampRange)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClippingPolicy {
    HardLimit,
    SoftLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSourceMixSettings {
    pub gain_milli: u16,
    pub muted: bool,
    pub ramp_frames: u32,
}

impl AudioSourceMixSettings {
    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.gain_milli > MAX_GAIN_MILLI || self.ramp_frames > MAX_GAIN_RAMP_FRAMES {
            return Err(AvCaptureError::InvalidMixSettings);
        }
        Ok(self)
    }

    fn effective_gain(self) -> f32 {
        if self.muted {
            0.0
        } else {
            f32::from(self.gain_milli) / 1_000.0
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MeterBucket {
    Silence,
    VeryLow,
    Low,
    Medium,
    High,
    Clipping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioMeterSummary {
    pub class: AvSourceClass,
    pub rms: MeterBucket,
    pub peak: MeterBucket,
    pub clipped: bool,
}

#[derive(Clone, PartialEq)]
pub struct MixedAudioBlock {
    pub timestamp: FrameTimestamp,
    pub channels: u8,
    samples: Vec<f32>,
    pub meters: Vec<AudioMeterSummary>,
    pub output_clipped: bool,
}

impl fmt::Debug for MixedAudioBlock {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MixedAudioBlock")
            .field("timestamp", &"<redacted>")
            .field("channels", &self.channels)
            .field("samples", &"<local-media-redacted>")
            .field("meters", &self.meters)
            .field("output_clipped", &self.output_clipped)
            .finish()
    }
}

impl MixedAudioBlock {
    #[must_use]
    pub fn samples_for_local_pipeline(&self) -> &[f32] {
        &self.samples
    }
}

#[derive(Debug, Clone, Copy)]
struct GainRamp {
    current: f32,
    target: f32,
    remaining: u32,
}

impl GainRamp {
    fn new() -> Self {
        Self {
            current: 1.0,
            target: 1.0,
            remaining: 0,
        }
    }

    fn update(&mut self, settings: AudioSourceMixSettings) {
        self.target = settings.effective_gain();
        self.remaining = settings.ramp_frames;
        if self.remaining == 0 {
            self.current = self.target;
        }
    }

    fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            let step = (self.target - self.current) / self.remaining as f32;
            self.current += step;
            self.remaining -= 1;
        }
        self.current
    }

    fn advance_silence(&mut self, frames: u32) {
        if self.remaining == 0 || frames == 0 {
            return;
        }
        let advanced = frames.min(self.remaining);
        if advanced == self.remaining {
            self.current = self.target;
            self.remaining = 0;
            return;
        }
        let fraction = advanced as f32 / self.remaining as f32;
        self.current += (self.target - self.current) * fraction;
        self.remaining -= advanced;
    }
}

#[derive(Debug, Clone)]
pub struct AudioMixEngine {
    channels: u8,
    sample_rate: u32,
    clipping: ClippingPolicy,
    gains: BTreeMap<AvSourceClass, GainRamp>,
    timeline_origin_ns: Option<u64>,
    sample_position: u128,
    next_discontinuity: bool,
}

impl AudioMixEngine {
    pub fn new(
        sample_rate: u32,
        channels: u8,
        clipping: ClippingPolicy,
    ) -> Result<Self, AvCaptureError> {
        AudioFormat {
            sample_rate,
            channels,
            sample_format: crate::AudioSampleFormat::Float32,
        }
        .validate()?;
        let mut gains = BTreeMap::new();
        gains.insert(AvSourceClass::Microphone, GainRamp::new());
        gains.insert(AvSourceClass::SystemAudio, GainRamp::new());
        Ok(Self {
            channels,
            sample_rate,
            clipping,
            gains,
            timeline_origin_ns: None,
            sample_position: 0,
            next_discontinuity: false,
        })
    }

    pub fn set_source(
        &mut self,
        class: AvSourceClass,
        settings: AudioSourceMixSettings,
    ) -> Result<(), AvCaptureError> {
        if class == AvSourceClass::Camera {
            return Err(AvCaptureError::FormatClassMismatch);
        }
        settings.validate()?;
        self.gains
            .get_mut(&class)
            .ok_or(AvCaptureError::UnknownAudioSource)?
            .update(settings);
        Ok(())
    }

    pub fn mix(
        &mut self,
        pts_ns: u64,
        frames: u32,
        microphone: Option<&[f32]>,
        system_audio: Option<&[f32]>,
    ) -> Result<MixedAudioBlock, AvCaptureError> {
        if frames == 0 {
            return Err(AvCaptureError::EmptyAudioBlock);
        }
        let sample_count = usize::try_from(frames)
            .ok()
            .and_then(|value| value.checked_mul(usize::from(self.channels)))
            .ok_or(AvCaptureError::AudioBlockTooLarge)?;
        if sample_count > 8_388_608 {
            return Err(AvCaptureError::AudioBlockTooLarge);
        }
        for input in [microphone, system_audio].into_iter().flatten() {
            if input.len() != sample_count || input.iter().any(|sample| !sample.is_finite()) {
                return Err(AvCaptureError::InvalidAudioBlock);
            }
        }
        if self.timeline_origin_ns.is_none() {
            self.timeline_origin_ns = Some(pts_ns);
        }
        let expected_pts_ns = self.timeline_ns(self.sample_position)?;
        if pts_ns != expected_pts_ns {
            return Err(AvCaptureError::AudioTimelineDiscontinuity);
        }
        let end_position = self
            .sample_position
            .checked_add(u128::from(frames))
            .ok_or(AvCaptureError::TimestampRange)?;
        let end_pts_ns = self.timeline_ns(end_position)?;
        let duration_ns = end_pts_ns
            .checked_sub(expected_pts_ns)
            .ok_or(AvCaptureError::TimestampRange)?;
        let timestamp = FrameTimestamp {
            pts_ns,
            duration_ns,
            discontinuity: self.next_discontinuity,
        };
        validate_frame_timestamp(timestamp)?;
        let mut mixed = vec![0.0_f32; sample_count];
        let mut meters = Vec::new();
        for (class, input) in [
            (AvSourceClass::Microphone, microphone),
            (AvSourceClass::SystemAudio, system_audio),
        ] {
            let gain = self
                .gains
                .get_mut(&class)
                .ok_or(AvCaptureError::UnknownAudioSource)?;
            if input.is_none() {
                gain.advance_silence(frames);
                meters.push(AudioMeterSummary {
                    class,
                    rms: MeterBucket::Silence,
                    peak: MeterBucket::Silence,
                    clipped: false,
                });
                continue;
            }
            let mut sum_squares = 0.0_f64;
            let mut peak = 0.0_f32;
            for frame in
                0..usize::try_from(frames).map_err(|_| AvCaptureError::AudioBlockTooLarge)?
            {
                let frame_gain = gain.next();
                for channel in 0..usize::from(self.channels) {
                    let index = frame * usize::from(self.channels) + channel;
                    let input_sample = input.map_or(0.0, |samples| samples[index]);
                    let adjusted = input_sample * frame_gain;
                    mixed[index] += adjusted;
                    sum_squares += f64::from(adjusted) * f64::from(adjusted);
                    peak = peak.max(adjusted.abs());
                }
            }
            let rms = (sum_squares / sample_count as f64).sqrt() as f32;
            meters.push(AudioMeterSummary {
                class,
                rms: meter_bucket(rms),
                peak: meter_bucket(peak),
                clipped: peak > 1.0,
            });
        }
        let output_clipped = mixed.iter().any(|sample| sample.abs() > 1.0);
        for sample in &mut mixed {
            *sample = match self.clipping {
                ClippingPolicy::HardLimit => sample.clamp(-1.0, 1.0),
                ClippingPolicy::SoftLimit => sample.tanh(),
            };
        }
        self.sample_position = end_position;
        self.next_discontinuity = false;
        Ok(MixedAudioBlock {
            timestamp,
            channels: self.channels,
            samples: mixed,
            meters,
            output_clipped,
        })
    }

    pub fn mark_discontinuity(&mut self, next_pts_ns: u64) -> Result<(), AvCaptureError> {
        if self.timeline_origin_ns.is_some()
            && next_pts_ns < self.timeline_ns(self.sample_position)?
        {
            return Err(AvCaptureError::AudioTimelineDiscontinuity);
        }
        self.timeline_origin_ns = Some(next_pts_ns);
        self.sample_position = 0;
        self.next_discontinuity = true;
        Ok(())
    }

    /// Advances a declared silent span without allocating an all-zero media
    /// block. It uses the same rational sample accumulator and gain ramps as
    /// [`Self::mix`].
    pub fn advance_silence_timeline(
        &mut self,
        pts_ns: u64,
        frames: u32,
    ) -> Result<FrameTimestamp, AvCaptureError> {
        if frames == 0 {
            return Err(AvCaptureError::EmptyAudioBlock);
        }
        if self.timeline_origin_ns.is_none() {
            self.timeline_origin_ns = Some(pts_ns);
        }
        let expected_pts_ns = self.timeline_ns(self.sample_position)?;
        if pts_ns != expected_pts_ns {
            return Err(AvCaptureError::AudioTimelineDiscontinuity);
        }
        let end_position = self
            .sample_position
            .checked_add(u128::from(frames))
            .ok_or(AvCaptureError::TimestampRange)?;
        let end_pts_ns = self.timeline_ns(end_position)?;
        let timestamp = FrameTimestamp {
            pts_ns,
            duration_ns: end_pts_ns
                .checked_sub(pts_ns)
                .ok_or(AvCaptureError::TimestampRange)?,
            discontinuity: self.next_discontinuity,
        };
        validate_frame_timestamp(timestamp)?;
        for gain in self.gains.values_mut() {
            gain.advance_silence(frames);
        }
        self.sample_position = end_position;
        self.next_discontinuity = false;
        Ok(timestamp)
    }

    fn timeline_ns(&self, sample_position: u128) -> Result<u64, AvCaptureError> {
        let origin = self
            .timeline_origin_ns
            .ok_or(AvCaptureError::AudioTimelineDiscontinuity)?;
        let offset_ns = sample_position
            .checked_mul(1_000_000_000)
            .ok_or(AvCaptureError::TimestampRange)?
            / u128::from(self.sample_rate);
        let offset_ns = u64::try_from(offset_ns).map_err(|_| AvCaptureError::TimestampRange)?;
        origin
            .checked_add(offset_ns)
            .ok_or(AvCaptureError::TimestampRange)
    }
}

fn meter_bucket(value: f32) -> MeterBucket {
    match value {
        value if value <= 0.000_01 => MeterBucket::Silence,
        value if value < 0.01 => MeterBucket::VeryLow,
        value if value < 0.1 => MeterBucket::Low,
        value if value < 0.5 => MeterBucket::Medium,
        value if value <= 1.0 => MeterBucket::High,
        _ => MeterBucket::Clipping,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TimingBucket {
    UnderFiveMs,
    FiveToTwentyMs,
    TwentyToFiftyMs,
    FiftyToEightyMs,
    OverBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AvCapabilityBucket {
    AudioStandard,
    AudioHighRate,
    AudioMultichannel,
    CameraSd,
    CameraHd,
    CameraUhd,
    CameraHighFrameRate,
}

impl AvCapabilityBucket {
    #[must_use]
    pub fn from_format(format: AvFormat) -> Self {
        match format {
            AvFormat::Audio(format) if format.channels > 2 => Self::AudioMultichannel,
            AvFormat::Audio(format) if format.sample_rate > 96_000 => Self::AudioHighRate,
            AvFormat::Audio(_) => Self::AudioStandard,
            AvFormat::Camera(format)
                if u64::from(format.frame_rate_numerator)
                    > u64::from(format.frame_rate_denominator).saturating_mul(60) =>
            {
                Self::CameraHighFrameRate
            }
            AvFormat::Camera(format)
                if u64::from(format.width).saturating_mul(u64::from(format.height))
                    >= 3_840 * 2_160 =>
            {
                Self::CameraUhd
            }
            AvFormat::Camera(format) if format.width >= 1_280 && format.height >= 720 => {
                Self::CameraHd
            }
            AvFormat::Camera(_) => Self::CameraSd,
        }
    }
}

impl TimingBucket {
    #[must_use]
    pub fn from_abs_ns(value: u64) -> Self {
        match value {
            0..5_000_000 => Self::UnderFiveMs,
            5_000_000..20_000_000 => Self::FiveToTwentyMs,
            20_000_000..50_000_000 => Self::TwentyToFiftyMs,
            50_000_000..=80_000_000 => Self::FiftyToEightyMs,
            _ => Self::OverBudget,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AvStableCode {
    PermissionPrompt,
    PermissionDenied,
    PermissionRevoked,
    DeviceUnavailable,
    DeviceGenerationChanged,
    DefaultConfirmationRequired,
    CapabilityChanged,
    FormatRenegotiationRequired,
    IngressOverload,
    ClockDiscontinuity,
    Sleep,
    Wake,
    OptionalSourceDisabled,
    SourceRecovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvDiagnostic {
    pub version: u16,
    pub class: Option<AvSourceClass>,
    pub route: Option<NativeRouteClass>,
    pub capability: Option<AvCapabilityBucket>,
    pub timing: Option<TimingBucket>,
    pub code: AvStableCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum UiEventKey {
    Source(AvSourceClass),
    Meter(AvSourceClass),
    Preview,
    Timing(AvSourceClass),
    Lifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvUiEvent {
    SourceStatus {
        class: AvSourceClass,
        code: AvStableCode,
    },
    Meter(AudioMeterSummary),
    CameraPreview {
        enabled: bool,
    },
    Timing {
        class: AvSourceClass,
        offset: TimingBucket,
        confidence: CalibrationConfidence,
    },
    Paused,
    Resumed,
    Stopped,
}

impl AvUiEvent {
    fn key(self) -> UiEventKey {
        match self {
            Self::SourceStatus { class, .. } => UiEventKey::Source(class),
            Self::Meter(summary) => UiEventKey::Meter(summary.class),
            Self::CameraPreview { .. } => UiEventKey::Preview,
            Self::Timing { class, .. } => UiEventKey::Timing(class),
            Self::Paused | Self::Resumed | Self::Stopped => UiEventKey::Lifecycle,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AvUiEventCoalescer {
    interval_ns: u64,
    last_emit_ns: Option<u64>,
    pending: BTreeMap<UiEventKey, AvUiEvent>,
}

impl AvUiEventCoalescer {
    pub fn new(interval_ns: u64) -> Result<Self, AvCaptureError> {
        if !(10_000_000..=1_000_000_000).contains(&interval_ns) {
            return Err(AvCaptureError::InvalidUiThrottle);
        }
        Ok(Self {
            interval_ns,
            last_emit_ns: None,
            pending: BTreeMap::new(),
        })
    }

    pub fn push(&mut self, now: MonotonicTimeNs, event: AvUiEvent) -> Result<(), AvCaptureError> {
        if self.last_emit_ns.is_some_and(|last| now.get() < last) {
            return Err(AvCaptureError::NonMonotonicMasterClock);
        }
        self.pending.insert(event.key(), event);
        Ok(())
    }

    pub fn drain_ready(&mut self, now: MonotonicTimeNs) -> Result<Vec<AvUiEvent>, AvCaptureError> {
        if self.last_emit_ns.is_some_and(|last| now.get() < last) {
            return Err(AvCaptureError::NonMonotonicMasterClock);
        }
        let ready = self
            .last_emit_ns
            .is_none_or(|last| now.get().saturating_sub(last) >= self.interval_ns);
        if !ready {
            return Ok(Vec::new());
        }
        self.last_emit_ns = Some(now.get());
        Ok(std::mem::take(&mut self.pending).into_values().collect())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AvOwnerBinding {
    adapter: AvAdapterInstanceId,
    session: AvSessionId,
}

impl AvOwnerBinding {
    #[must_use]
    pub const fn adapter(self) -> AvAdapterInstanceId {
        self.adapter
    }
}

impl fmt::Debug for AvOwnerBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AvOwnerBinding(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AvStreamEpoch(u64);

impl AvStreamEpoch {
    fn first() -> Self {
        Self(1)
    }

    fn next(self) -> Result<Self, AvCaptureError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(AvCaptureError::StreamEpochExhausted)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AvSourceStamp {
    owner: AvOwnerBinding,
    class: AvSourceClass,
    device: AvDeviceId,
    generation: AvDeviceGeneration,
    stream_epoch: AvStreamEpoch,
}

impl AvSourceStamp {
    #[must_use]
    pub const fn class(self) -> AvSourceClass {
        self.class
    }

    #[must_use]
    pub const fn generation(self) -> AvDeviceGeneration {
        self.generation
    }

    #[must_use]
    pub const fn stream_epoch(self) -> AvStreamEpoch {
        self.stream_epoch
    }
}

impl fmt::Debug for AvSourceStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvSourceStamp")
            .field("owner", &self.owner)
            .field("class", &self.class)
            .field("device", &self.device)
            .field("generation", &self.generation)
            .field("stream_epoch", &self.stream_epoch)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AvControlEventStamp {
    owner: AvOwnerBinding,
    revision: u64,
    sequence: u64,
}

impl AvControlEventStamp {
    pub fn new(
        owner: AvOwnerBinding,
        revision: u64,
        sequence: u64,
    ) -> Result<Self, AvCaptureError> {
        if revision == 0 || sequence == 0 {
            return Err(AvCaptureError::InvalidControlEventStamp);
        }
        Ok(Self {
            owner,
            revision,
            sequence,
        })
    }

    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }
}

impl fmt::Debug for AvControlEventStamp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AvControlEventStamp(<redacted>)")
    }
}

pub trait AvBufferLease: Send {
    /// Called exactly once by `NativeAvBuffer::new`; later accounting uses the
    /// immutable snapshot retained by the buffer.
    fn retained_bytes(&self) -> u64;
    /// Moves the provider-neutral body out exactly once. The remaining lease
    /// stays alive until the local appsrc/downstream owner releases it.
    fn take_payload(&mut self) -> Option<AvPayloadBody>;
    fn release(self: Box<Self>);
}

pub enum AvPayloadBody {
    Bytes(Vec<u8>),
    Opaque(Box<dyn Any + Send>),
}

impl fmt::Debug for AvPayloadBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(bytes) => formatter
                .debug_tuple("Bytes")
                .field(&format_args!("<{} bytes redacted>", bytes.len()))
                .finish(),
            Self::Opaque(_) => formatter.write_str("Opaque(<redacted>)"),
        }
    }
}

pub struct AvAppSrcPayload {
    body: AvPayloadBody,
    retained_bytes: u64,
    lease: Option<Box<dyn AvBufferLease>>,
}

impl AvAppSrcPayload {
    #[must_use]
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.body {
            AvPayloadBody::Bytes(bytes) => Some(bytes),
            AvPayloadBody::Opaque(_) => None,
        }
    }

    #[must_use]
    pub fn opaque<T: Any + Send>(&self) -> Option<&T> {
        match &self.body {
            AvPayloadBody::Opaque(value) => value.downcast_ref(),
            AvPayloadBody::Bytes(_) => None,
        }
    }

    pub fn release(mut self) {
        if let Some(lease) = self.lease.take() {
            lease.release();
        }
    }
}

impl fmt::Debug for AvAppSrcPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvAppSrcPayload")
            .field("body", &self.body)
            .field("retained_bytes", &self.retained_bytes)
            .finish()
    }
}

impl Drop for AvAppSrcPayload {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            lease.release();
        }
    }
}

pub struct AvAppSrcInput {
    stamp: AvSourceStamp,
    timestamp: FrameTimestamp,
    format: AvFormat,
    payload: AvAppSrcPayload,
}

impl AvAppSrcInput {
    #[must_use]
    pub const fn stamp(&self) -> AvSourceStamp {
        self.stamp
    }

    #[must_use]
    pub const fn timestamp(&self) -> FrameTimestamp {
        self.timestamp
    }

    #[must_use]
    pub const fn format(&self) -> AvFormat {
        self.format
    }

    #[must_use]
    pub const fn payload(&self) -> &AvAppSrcPayload {
        &self.payload
    }

    pub fn release(self) {
        self.payload.release();
    }
}

impl fmt::Debug for AvAppSrcInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvAppSrcInput")
            .field("stamp", &self.stamp)
            .field("timestamp", &"<redacted>")
            .field("format", &self.format)
            .field("payload", &self.payload)
            .finish()
    }
}

pub trait AvLocalAppSrcAdapter {
    /// Ownership transfers to the adapter. It must retain the input until the
    /// downstream buffer is released; dropping it releases the native lease.
    fn push(&mut self, input: AvAppSrcInput) -> Result<(), Box<AvAppSrcInput>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAvBufferTiming {
    pub sequence: u64,
    pub source_pts_ns: u64,
    pub duration_ns: u64,
    pub arrival: MonotonicTimeNs,
    pub latency: SourceLatency,
    pub discontinuity: bool,
}

impl NativeAvBufferTiming {
    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.sequence == 0 || self.duration_ns == 0 {
            return Err(AvCaptureError::InvalidNativeBufferTiming);
        }
        self.source_pts_ns
            .checked_add(self.duration_ns)
            .ok_or(AvCaptureError::TimestampRange)?;
        self.latency.validate()?;
        Ok(self)
    }
}

pub struct NativeAvBuffer {
    stamp: AvSourceStamp,
    timing: NativeAvBufferTiming,
    corrected_timestamp: Option<FrameTimestamp>,
    format: AvFormat,
    retained_bytes: u64,
    lease: Option<Box<dyn AvBufferLease>>,
}

impl NativeAvBuffer {
    pub fn new(
        stamp: AvSourceStamp,
        timing: NativeAvBufferTiming,
        format: AvFormat,
        lease: Box<dyn AvBufferLease>,
    ) -> Result<Self, AvCaptureError> {
        if let Err(error) = timing.validate() {
            lease.release();
            return Err(error);
        }
        if let Err(error) = format.validate_for(stamp.class()) {
            lease.release();
            return Err(error);
        }
        let retained = lease.retained_bytes();
        if retained == 0 || retained > MAX_AV_QUEUE_BYTES {
            lease.release();
            return Err(AvCaptureError::InvalidBufferLease);
        }
        Ok(Self {
            stamp,
            timing,
            corrected_timestamp: None,
            format,
            retained_bytes: retained,
            lease: Some(lease),
        })
    }

    #[must_use]
    pub const fn stamp(&self) -> AvSourceStamp {
        self.stamp
    }

    #[must_use]
    pub const fn timing(&self) -> NativeAvBufferTiming {
        self.timing
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.timing.sequence
    }

    #[must_use]
    pub const fn timestamp(&self) -> Option<FrameTimestamp> {
        self.corrected_timestamp
    }

    #[must_use]
    pub const fn arrival(&self) -> MonotonicTimeNs {
        self.timing.arrival
    }

    #[must_use]
    pub const fn format(&self) -> AvFormat {
        self.format
    }

    #[must_use]
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    fn apply_corrected_timestamp(
        &mut self,
        timestamp: FrameTimestamp,
    ) -> Result<(), AvCaptureError> {
        validate_frame_timestamp(timestamp)?;
        if self.corrected_timestamp.is_some() {
            return Err(AvCaptureError::BufferAlreadyCorrected);
        }
        self.corrected_timestamp = Some(timestamp);
        Ok(())
    }

    pub fn into_appsrc_input(mut self) -> Result<AvAppSrcInput, AvCaptureError> {
        let timestamp = self
            .corrected_timestamp
            .ok_or(AvCaptureError::UncorrectedBuffer)?;
        let mut lease = self
            .lease
            .take()
            .ok_or(AvCaptureError::PayloadAlreadyTaken)?;
        let Some(body) = lease.take_payload() else {
            lease.release();
            return Err(AvCaptureError::PayloadAlreadyTaken);
        };
        if matches!(&body, AvPayloadBody::Bytes(bytes) if u64::try_from(bytes.len()).ok() != Some(self.retained_bytes))
        {
            lease.release();
            return Err(AvCaptureError::PayloadSizeMismatch);
        }
        Ok(AvAppSrcInput {
            stamp: self.stamp,
            timestamp,
            format: self.format,
            payload: AvAppSrcPayload {
                body,
                retained_bytes: self.retained_bytes,
                lease: Some(lease),
            },
        })
    }

    pub fn release(mut self) {
        if let Some(lease) = self.lease.take() {
            lease.release();
        }
    }
}

impl fmt::Debug for NativeAvBuffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeAvBuffer")
            .field("stamp", &self.stamp)
            .field("timing", &"<redacted>")
            .field("corrected_timestamp", &"<redacted>")
            .field("format", &self.format)
            .field("retained_bytes", &self.retained_bytes())
            .finish()
    }
}

impl Drop for NativeAvBuffer {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            lease.release();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvQueuePush {
    Accepted,
    DroppedNewest,
    DroppedOldest { count: u16, bytes: u64 },
}

#[derive(Debug)]
pub struct AvIngressQueue {
    stamp: AvSourceStamp,
    format: AvFormat,
    spec: AvQueueSpec,
    buffers: VecDeque<NativeAvBuffer>,
    retained_bytes: u64,
    last_observed_ns: Option<u64>,
    last_sequence: u64,
    last_corrected_end_ns: Option<u64>,
}

impl AvIngressQueue {
    pub fn new(
        stamp: AvSourceStamp,
        format: AvFormat,
        spec: AvQueueSpec,
    ) -> Result<Self, AvCaptureError> {
        format.validate_for(stamp.class())?;
        Ok(Self {
            stamp,
            format,
            spec: spec.validate()?,
            buffers: VecDeque::new(),
            retained_bytes: 0,
            last_observed_ns: None,
            last_sequence: 0,
            last_corrected_end_ns: None,
        })
    }

    pub fn push(&mut self, buffer: NativeAvBuffer) -> Result<AvQueuePush, AvCaptureError> {
        if buffer.stamp() != self.stamp {
            return Err(AvCaptureError::StaleSourceStamp);
        }
        if buffer.format() != self.format {
            return Err(AvCaptureError::FormatRenegotiationRequired);
        }
        let expected_sequence = self
            .last_sequence
            .checked_add(1)
            .ok_or(AvCaptureError::BufferSequenceExhausted)?;
        if buffer.sequence() != expected_sequence {
            return Err(AvCaptureError::OutOfOrderBuffer);
        }
        let timestamp = buffer
            .timestamp()
            .ok_or(AvCaptureError::UncorrectedBuffer)?;
        if self
            .last_corrected_end_ns
            .is_some_and(|last| timestamp.pts_ns < last)
        {
            return Err(AvCaptureError::CorrectedTimestampRollback);
        }
        if self
            .last_observed_ns
            .is_some_and(|last| buffer.arrival().get() < last)
        {
            return Err(AvCaptureError::NonMonotonicMasterClock);
        }
        self.last_observed_ns = Some(buffer.arrival().get());
        self.last_sequence = buffer.sequence();
        self.last_corrected_end_ns = Some(timestamp.end_ns());
        self.expire(buffer.arrival().get());
        let bytes = buffer.retained_bytes();
        let would_overflow = self.buffers.len() >= usize::from(self.spec.max_buffers)
            || self.retained_bytes.saturating_add(bytes) > self.spec.max_bytes;
        if would_overflow && self.spec.backpressure == AvBackpressurePolicy::DropNewest {
            buffer.release();
            return Ok(AvQueuePush::DroppedNewest);
        }
        let mut dropped_count = 0_u16;
        let mut dropped_bytes = 0_u64;
        while self.buffers.len() >= usize::from(self.spec.max_buffers)
            || self.retained_bytes.saturating_add(bytes) > self.spec.max_bytes
        {
            let Some(oldest) = self.buffers.pop_front() else {
                buffer.release();
                return Ok(AvQueuePush::DroppedNewest);
            };
            let removed = oldest.retained_bytes();
            self.retained_bytes = self.retained_bytes.saturating_sub(removed);
            dropped_count = dropped_count.saturating_add(1);
            dropped_bytes = dropped_bytes.saturating_add(removed);
            oldest.release();
        }
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);
        self.buffers.push_back(buffer);
        if dropped_count == 0 {
            Ok(AvQueuePush::Accepted)
        } else {
            Ok(AvQueuePush::DroppedOldest {
                count: dropped_count,
                bytes: dropped_bytes,
            })
        }
    }

    pub fn pop(&mut self, now: MonotonicTimeNs) -> Result<Option<NativeAvBuffer>, AvCaptureError> {
        if self.last_observed_ns.is_some_and(|last| now.get() < last) {
            return Err(AvCaptureError::NonMonotonicMasterClock);
        }
        self.last_observed_ns = Some(now.get());
        self.expire(now.get());
        let Some(buffer) = self.buffers.pop_front() else {
            return Ok(None);
        };
        self.retained_bytes = self.retained_bytes.saturating_sub(buffer.retained_bytes());
        Ok(Some(buffer))
    }

    pub fn drain(&mut self) -> (u16, u64) {
        let count = u16::try_from(self.buffers.len()).unwrap_or(u16::MAX);
        let bytes = self.retained_bytes;
        self.buffers.clear();
        self.retained_bytes = 0;
        (count, bytes)
    }

    fn expire(&mut self, now_ns: u64) {
        while self.buffers.front().is_some_and(|buffer| {
            now_ns.saturating_sub(buffer.arrival().get()) > self.spec.max_age_ns
        }) {
            if let Some(buffer) = self.buffers.pop_front() {
                self.retained_bytes = self.retained_bytes.saturating_sub(buffer.retained_bytes());
                buffer.release();
            }
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }

    #[must_use]
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    fn expected_sequence(&self) -> Result<u64, AvCaptureError> {
        self.last_sequence
            .checked_add(1)
            .ok_or(AvCaptureError::BufferSequenceExhausted)
    }
}

pub struct AvSessionClaimTicket {
    binding: AvOwnerBinding,
}

/// Non-cloneable proof that the one bound adapter owner has not already been
/// used to construct a session state machine.
pub struct AvSessionOwnerTicket {
    binding: AvOwnerBinding,
}

impl fmt::Debug for AvSessionOwnerTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvSessionOwnerTicket")
            .field("binding", &self.binding)
            .finish()
    }
}

impl AvSessionClaimTicket {
    #[must_use]
    pub const fn proposed_binding(&self) -> AvOwnerBinding {
        self.binding
    }

    #[must_use]
    pub const fn accept(self) -> AvOwnerBinding {
        self.binding
    }
}

impl fmt::Debug for AvSessionClaimTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvSessionClaimTicket")
            .field("binding", &self.binding)
            .finish()
    }
}

pub struct AvSourceCallTicket<'a> {
    binding: &'a AvOwnerBinding,
}

impl AvSourceCallTicket<'_> {
    #[must_use]
    pub const fn binding(&self) -> AvOwnerBinding {
        *self.binding
    }
}

impl fmt::Debug for AvSourceCallTicket<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvSourceCallTicket")
            .field("binding", self.binding)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AvOperationId(u64);

impl fmt::Debug for AvOperationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AvOperationId(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AvTerminalId(u64);

impl fmt::Debug for AvTerminalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AvTerminalId(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvTerminalResult {
    pub id: AvTerminalId,
    pub kind: AvOperationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvOperationPolicy {
    pub native_timeout: Duration,
}

impl Default for AvOperationPolicy {
    fn default() -> Self {
        Self {
            native_timeout: Duration::from_secs(10),
        }
    }
}

impl AvOperationPolicy {
    pub fn validate(self) -> Result<Self, AvCaptureError> {
        if self.native_timeout.is_zero() || self.native_timeout > MAX_OPERATION_TIMEOUT {
            return Err(AvCaptureError::InvalidOperationPolicy);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvOperationKind {
    RequestPermission(AvSourceClass),
    Start,
    Reconfigure,
    Pause,
    Resume,
    Stop,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvNativeRequest {
    RequestPermission(AvSourceClass),
    Start(AvPipelineGraphSpec),
    Reconfigure(AvPipelineGraphSpec),
    Pause,
    Resume,
    Stop,
    Cancel,
}

impl AvNativeRequest {
    #[must_use]
    pub const fn kind(&self) -> AvOperationKind {
        match self {
            Self::RequestPermission(class) => AvOperationKind::RequestPermission(*class),
            Self::Start(_) => AvOperationKind::Start,
            Self::Reconfigure(_) => AvOperationKind::Reconfigure,
            Self::Pause => AvOperationKind::Pause,
            Self::Resume => AvOperationKind::Resume,
            Self::Stop => AvOperationKind::Stop,
            Self::Cancel => AvOperationKind::Cancel,
        }
    }

    fn requires_live_snapshot(&self) -> bool {
        matches!(self, Self::Start(_) | Self::Reconfigure(_) | Self::Resume)
    }
}

pub struct AvOperationTicket {
    owner: AvOwnerBinding,
    operation: AvOperationId,
    kind: AvOperationKind,
    stamps: Vec<AvSourceStamp>,
    predecessor_stamps: Vec<AvSourceStamp>,
    terminal_id: Option<AvTerminalId>,
    native_timeout: Duration,
}

impl AvOperationTicket {
    #[must_use]
    pub const fn owner(&self) -> AvOwnerBinding {
        self.owner
    }

    #[must_use]
    pub const fn operation(&self) -> AvOperationId {
        self.operation
    }

    #[must_use]
    pub const fn kind(&self) -> AvOperationKind {
        self.kind
    }

    #[must_use]
    pub fn stamps(&self) -> &[AvSourceStamp] {
        &self.stamps
    }

    /// Streams that the adapter must quiesce before acknowledging the new
    /// operation. This includes ambiguously successful prior operations.
    #[must_use]
    pub fn predecessor_stamps(&self) -> &[AvSourceStamp] {
        &self.predecessor_stamps
    }

    #[must_use]
    pub const fn terminal_id(&self) -> Option<AvTerminalId> {
        self.terminal_id
    }

    #[must_use]
    pub const fn native_timeout(&self) -> Duration {
        self.native_timeout
    }

    #[must_use]
    pub fn acknowledge(self, teardown_complete: bool) -> NativeAvAcknowledgement {
        NativeAvAcknowledgement {
            owner: self.owner,
            operation: self.operation,
            kind: self.kind,
            stamps: self.stamps,
            predecessor_stamps: self.predecessor_stamps,
            terminal_id: self.terminal_id,
            teardown_complete,
        }
    }
}

impl fmt::Debug for AvOperationTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvOperationTicket")
            .field("owner", &self.owner)
            .field("operation", &self.operation)
            .field("kind", &self.kind)
            .field("stamps", &self.stamps)
            .field("predecessor_stamps", &self.predecessor_stamps)
            .field("terminal_id", &self.terminal_id)
            .field("native_timeout", &self.native_timeout)
            .finish()
    }
}

#[derive(Debug)]
pub struct NativeAvAcknowledgement {
    owner: AvOwnerBinding,
    operation: AvOperationId,
    kind: AvOperationKind,
    stamps: Vec<AvSourceStamp>,
    predecessor_stamps: Vec<AvSourceStamp>,
    terminal_id: Option<AvTerminalId>,
    teardown_complete: bool,
}

pub struct AvTerminalReconcileTicket {
    owner: AvOwnerBinding,
    terminal_id: AvTerminalId,
    kind: AvOperationKind,
    stamps: Vec<AvSourceStamp>,
    predecessor_stamps: Vec<AvSourceStamp>,
    native_timeout: Duration,
}

impl AvTerminalReconcileTicket {
    #[must_use]
    pub const fn owner(&self) -> AvOwnerBinding {
        self.owner
    }

    #[must_use]
    pub const fn terminal_id(&self) -> AvTerminalId {
        self.terminal_id
    }

    #[must_use]
    pub const fn kind(&self) -> AvOperationKind {
        self.kind
    }

    #[must_use]
    pub fn stamps(&self) -> &[AvSourceStamp] {
        &self.stamps
    }

    #[must_use]
    pub fn predecessor_stamps(&self) -> &[AvSourceStamp] {
        &self.predecessor_stamps
    }

    #[must_use]
    pub const fn native_timeout(&self) -> Duration {
        self.native_timeout
    }
}

impl fmt::Debug for AvTerminalReconcileTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AvTerminalReconcileTicket")
            .field("owner", &self.owner)
            .field("terminal_id", &self.terminal_id)
            .field("kind", &self.kind)
            .field("stamps", &self.stamps)
            .field("predecessor_stamps", &self.predecessor_stamps)
            .field("native_timeout", &self.native_timeout)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvTerminalPostcondition {
    NotApplied,
    Applied { terminal_id: AvTerminalId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeAvFailureCode {
    PermissionDenied,
    PermissionRestricted,
    SourceUnavailable,
    CapabilityChanged,
    FormatChanged,
    Busy,
    Timeout,
    Cancelled,
    BackendFault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("native A/V bridge failed with privacy-safe code {code:?}")]
pub struct NativeAvFailure {
    pub code: NativeAvFailureCode,
    pub retryable: bool,
}

#[derive(Debug)]
pub enum NativeAvEvent {
    Buffer(NativeAvBuffer),
    PermissionChanged {
        stamp: AvControlEventStamp,
        class: AvSourceClass,
        state: PermissionState,
    },
    CatalogChanged {
        stamp: AvControlEventStamp,
        catalog: AvDeviceCatalog,
        reason: CatalogChangeReason,
    },
    Overload {
        stamp: AvSourceStamp,
        dropped_buffers: u32,
    },
    SourceFailed {
        stamp: AvSourceStamp,
        code: NativeAvFailureCode,
    },
    Sleep,
    Wake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogChangeReason {
    Hotplug,
    DefaultChanged,
    WirelessProfileChanged,
    CapabilityChanged,
}

pub trait NativeAvBridge {
    /// Pure identity read; this method may not access a platform API.
    fn adapter_instance(&self) -> AvAdapterInstanceId;

    /// Claims this adapter object for exactly one session.
    fn bind(&mut self, ticket: AvSessionClaimTicket) -> Result<AvOwnerBinding, NativeAvFailure>;

    fn capabilities(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<NativeAvBridgeCapabilities, NativeAvFailure>;

    fn enumerate(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<AvDeviceCatalog, NativeAvFailure>;

    /// Reports whether the stable terminal request was already applied. The
    /// adapter must not perform another native release while reconciling.
    fn reconcile_terminal(
        &mut self,
        ticket: AvTerminalReconcileTicket,
    ) -> Result<AvTerminalPostcondition, NativeAvFailure>;

    /// The ticket is non-cloneable and can acknowledge only its own request.
    fn execute(
        &mut self,
        ticket: AvOperationTicket,
        request: &AvNativeRequest,
    ) -> Result<NativeAvAcknowledgement, NativeAvFailure>;

    fn poll(
        &mut self,
        ticket: AvSourceCallTicket<'_>,
    ) -> Result<Option<NativeAvEvent>, NativeAvFailure>;
}

pub struct BoundNativeAvBridge<B> {
    bridge: B,
    binding: AvOwnerBinding,
    session_owner_available: bool,
}

impl<B: NativeAvBridge> BoundNativeAvBridge<B> {
    pub fn new(mut bridge: B, session: AvSessionId) -> Result<Self, AvCaptureError> {
        let binding = AvOwnerBinding {
            adapter: bridge.adapter_instance(),
            session,
        };
        let accepted = bridge
            .bind(AvSessionClaimTicket { binding })
            .map_err(AvCaptureError::Native)?;
        if accepted != binding {
            return Err(AvCaptureError::OwnerMismatch);
        }
        Ok(Self {
            bridge,
            binding,
            session_owner_available: true,
        })
    }

    #[must_use]
    pub const fn binding(&self) -> AvOwnerBinding {
        self.binding
    }

    pub fn claim_session(&mut self) -> Result<AvSessionOwnerTicket, AvCaptureError> {
        if !self.session_owner_available {
            return Err(AvCaptureError::SessionAlreadyClaimed);
        }
        self.session_owner_available = false;
        Ok(AvSessionOwnerTicket {
            binding: self.binding,
        })
    }

    pub fn capabilities(&mut self) -> Result<NativeAvBridgeCapabilities, AvCaptureError> {
        let capabilities = self
            .bridge
            .capabilities(AvSourceCallTicket {
                binding: &self.binding,
            })
            .map_err(AvCaptureError::Native)?
            .validate()?;
        if capabilities.adapter != self.binding.adapter {
            return Err(AvCaptureError::OwnerMismatch);
        }
        Ok(capabilities)
    }

    pub fn enumerate(&mut self) -> Result<AvDeviceCatalog, AvCaptureError> {
        let catalog = self
            .bridge
            .enumerate(AvSourceCallTicket {
                binding: &self.binding,
            })
            .map_err(AvCaptureError::Native)?;
        if catalog.adapter() != self.binding.adapter {
            return Err(AvCaptureError::OwnerMismatch);
        }
        Ok(catalog)
    }

    pub fn poll_owned(&mut self) -> Result<Option<OwnedNativeAvEvent>, AvCaptureError> {
        self.bridge
            .poll(AvSourceCallTicket {
                binding: &self.binding,
            })
            .map(|event| {
                event.map(|event| OwnedNativeAvEvent {
                    owner: self.binding,
                    event,
                })
            })
            .map_err(AvCaptureError::Native)
    }

    fn execute(
        &mut self,
        ticket: AvOperationTicket,
        request: &AvNativeRequest,
    ) -> Result<NativeAvAcknowledgement, NativeAvFailure> {
        self.bridge.execute(ticket, request)
    }

    fn reconcile_terminal(
        &mut self,
        ticket: AvTerminalReconcileTicket,
    ) -> Result<AvTerminalPostcondition, NativeAvFailure> {
        self.bridge.reconcile_terminal(ticket)
    }
}

impl<B> fmt::Debug for BoundNativeAvBridge<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundNativeAvBridge")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

pub struct OwnedNativeAvEvent {
    owner: AvOwnerBinding,
    event: NativeAvEvent,
}

impl fmt::Debug for OwnedNativeAvEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedNativeAvEvent")
            .field("owner", &self.owner)
            .field("event", &self.event)
            .finish()
    }
}

pub struct OwnedAvAcknowledgement {
    acknowledgement: NativeAvAcknowledgement,
}

impl fmt::Debug for OwnedAvAcknowledgement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedAvAcknowledgement")
            .field("acknowledgement", &self.acknowledgement)
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OwnedAvOperationFailure {
    owner: AvOwnerBinding,
    operation: AvOperationId,
    kind: AvOperationKind,
    failure: NativeAvFailure,
}

#[derive(Debug)]
pub enum AvActionExecution {
    Acknowledged(OwnedAvAcknowledgement),
    Failed(OwnedAvOperationFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvSessionState {
    Idle,
    Starting,
    Recording,
    Pausing,
    Paused,
    Resuming,
    Reconfiguring,
    Suspended,
    Stopping,
    TeardownRequired,
    Stopped,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone)]
struct PendingAvOperation {
    id: AvOperationId,
    kind: AvOperationKind,
    request: AvNativeRequest,
    expected_capabilities: Option<NativeAvBridgeCapabilities>,
    expected_catalog: Option<AvDeviceCatalog>,
    stamps: Vec<AvSourceStamp>,
    predecessor_stamps: Vec<AvSourceStamp>,
    terminal_id: Option<AvTerminalId>,
    proposed_settings: Option<AvCaptureSettingsV2>,
    proposed_preview: Option<bool>,
    return_state: AvSessionState,
    dispatched: bool,
}

#[derive(Debug)]
struct ActiveAvSource {
    stamp: AvSourceStamp,
    format: AvFormat,
    queue: AvIngressQueue,
    timebase: Option<SourceTimebase>,
}

#[derive(Debug, Clone, Copy)]
struct TerminalRequestState {
    id: AvTerminalId,
    kind: AvOperationKind,
}

#[derive(Debug)]
pub struct AvSessionAction {
    owner: AvOwnerBinding,
    operation: AvOperationId,
    kind: AvOperationKind,
}

impl AvSessionAction {
    pub fn execute_source<B: NativeAvBridge>(
        self,
        session: &mut AvCaptureSession,
        source: &mut BoundNativeAvBridge<B>,
    ) -> Result<AvActionExecution, AvCaptureError> {
        if self.owner != session.owner || self.owner != source.binding() {
            return Err(AvCaptureError::OwnerMismatch);
        }
        let pending = session
            .pending
            .as_ref()
            .ok_or(AvCaptureError::StaleOperation)?;
        if pending.id != self.operation || pending.kind != self.kind || pending.dispatched {
            return Err(AvCaptureError::StaleOperation);
        }
        if pending.request.requires_live_snapshot() {
            let live_capabilities = match source.capabilities() {
                Ok(capabilities) => capabilities,
                Err(error) => {
                    return session.bind_failure(
                        self.operation,
                        self.kind,
                        operation_failure_from_capture_error(error),
                    );
                }
            };
            let live_catalog = match source.enumerate() {
                Ok(catalog) => catalog,
                Err(error) => {
                    return session.bind_failure(
                        self.operation,
                        self.kind,
                        operation_failure_from_capture_error(error),
                    );
                }
            };
            if pending.expected_capabilities != Some(live_capabilities)
                || pending.expected_catalog.as_ref() != Some(&live_catalog)
            {
                return session.bind_failure(
                    self.operation,
                    self.kind,
                    NativeAvFailure {
                        code: NativeAvFailureCode::CapabilityChanged,
                        retryable: true,
                    },
                );
            }
        }
        let (request, stamps, predecessor_stamps, terminal_id) = {
            let pending = session
                .pending
                .as_mut()
                .ok_or(AvCaptureError::StaleOperation)?;
            pending.dispatched = true;
            (
                pending.request.clone(),
                pending.stamps.clone(),
                pending.predecessor_stamps.clone(),
                pending.terminal_id,
            )
        };
        if let Some(terminal_id) = terminal_id {
            let reconcile = AvTerminalReconcileTicket {
                owner: self.owner,
                terminal_id,
                kind: self.kind,
                stamps: stamps.clone(),
                predecessor_stamps: predecessor_stamps.clone(),
                native_timeout: session.operation_policy.native_timeout,
            };
            match source.reconcile_terminal(reconcile) {
                Ok(AvTerminalPostcondition::Applied {
                    terminal_id: applied,
                }) if applied == terminal_id => {
                    return Ok(AvActionExecution::Acknowledged(OwnedAvAcknowledgement {
                        acknowledgement: NativeAvAcknowledgement {
                            owner: self.owner,
                            operation: self.operation,
                            kind: self.kind,
                            stamps,
                            predecessor_stamps,
                            terminal_id: Some(terminal_id),
                            teardown_complete: true,
                        },
                    }));
                }
                Ok(AvTerminalPostcondition::Applied { .. }) => {
                    return Err(AvCaptureError::InvalidTerminalPostcondition);
                }
                Ok(AvTerminalPostcondition::NotApplied) => {}
                Err(failure) => {
                    return session.bind_failure(self.operation, self.kind, failure);
                }
            }
        }
        let ticket = AvOperationTicket {
            owner: self.owner,
            operation: self.operation,
            kind: self.kind,
            stamps,
            predecessor_stamps,
            terminal_id,
            native_timeout: session.operation_policy.native_timeout,
        };
        match source.execute(ticket, &request) {
            Ok(acknowledgement) => {
                if acknowledgement.owner != self.owner
                    || acknowledgement.operation != self.operation
                    || acknowledgement.kind != self.kind
                    || acknowledgement.terminal_id != terminal_id
                {
                    return Err(AvCaptureError::InvalidNativeAcknowledgement);
                }
                Ok(AvActionExecution::Acknowledged(OwnedAvAcknowledgement {
                    acknowledgement,
                }))
            }
            Err(failure) => session.bind_failure(self.operation, self.kind, failure),
        }
    }
}

fn operation_failure_from_capture_error(error: AvCaptureError) -> NativeAvFailure {
    match error {
        AvCaptureError::Native(failure) => failure,
        _ => NativeAvFailure {
            code: NativeAvFailureCode::CapabilityChanged,
            retryable: false,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvEventOutcome {
    pub queue: Option<AvQueuePush>,
    pub diagnostics: Vec<AvDiagnostic>,
    pub disabled_sources: Vec<AvSourceClass>,
    pub native_reconfigure_required: bool,
}

impl AvEventOutcome {
    fn empty() -> Self {
        Self {
            queue: None,
            diagnostics: Vec::new(),
            disabled_sources: Vec::new(),
            native_reconfigure_required: false,
        }
    }
}

#[derive(Debug)]
pub struct AvCaptureSession {
    owner: AvOwnerBinding,
    state: AvSessionState,
    next_operation: u64,
    next_terminal: u64,
    operation_policy: AvOperationPolicy,
    pending: Option<PendingAvOperation>,
    active: BTreeMap<AvSourceClass, ActiveAvSource>,
    last_issued_epochs: BTreeMap<AvSourceClass, AvStreamEpoch>,
    unconfirmed_stamps: Vec<AvSourceStamp>,
    terminal_request: Option<TerminalRequestState>,
    terminal_result: Option<AvTerminalResult>,
    last_control_revision: u64,
    last_control_sequence: u64,
    settings: AvCaptureSettingsV2,
    graph: Option<AvPipelineGraphSpec>,
    catalog: Option<AvDeviceCatalog>,
    camera_preview_enabled: bool,
}

impl AvCaptureSession {
    #[must_use]
    pub fn new(ticket: AvSessionOwnerTicket) -> Self {
        Self::with_validated_policy(ticket, AvOperationPolicy::default())
    }

    pub fn new_with_policy(
        ticket: AvSessionOwnerTicket,
        policy: AvOperationPolicy,
    ) -> Result<Self, AvCaptureError> {
        Ok(Self::with_validated_policy(ticket, policy.validate()?))
    }

    fn with_validated_policy(
        ticket: AvSessionOwnerTicket,
        operation_policy: AvOperationPolicy,
    ) -> Self {
        let owner = ticket.binding;
        Self {
            owner,
            state: AvSessionState::Idle,
            next_operation: 1,
            next_terminal: 1,
            operation_policy,
            pending: None,
            active: BTreeMap::new(),
            last_issued_epochs: BTreeMap::new(),
            unconfirmed_stamps: Vec::new(),
            terminal_request: None,
            terminal_result: None,
            last_control_revision: 0,
            last_control_sequence: 0,
            settings: AvCaptureSettingsV2::screen_only(),
            graph: None,
            catalog: None,
            camera_preview_enabled: false,
        }
    }

    #[must_use]
    pub const fn owner(&self) -> AvOwnerBinding {
        self.owner
    }

    #[must_use]
    pub const fn state(&self) -> AvSessionState {
        self.state
    }

    #[must_use]
    pub const fn camera_preview_enabled(&self) -> bool {
        self.camera_preview_enabled
    }

    #[must_use]
    pub const fn terminal_result(&self) -> Option<AvTerminalResult> {
        self.terminal_result
    }

    #[must_use]
    pub fn source_stamp(&self, class: AvSourceClass) -> Option<AvSourceStamp> {
        self.active.get(&class).map(|active| active.stamp)
    }

    pub fn calibrate_source(
        &mut self,
        stamp: AvSourceStamp,
        policy: AvSyncPolicy,
        samples: &[CalibrationSample],
    ) -> Result<StartupCalibration, AvCaptureError> {
        if !matches!(
            self.state,
            AvSessionState::Recording | AvSessionState::Paused | AvSessionState::Suspended
        ) {
            return Err(AvCaptureError::InvalidSessionTransition);
        }
        let active = self
            .active
            .get_mut(&stamp.class())
            .ok_or(AvCaptureError::StaleSourceStamp)?;
        if active.stamp != stamp {
            return Err(AvCaptureError::StaleSourceStamp);
        }
        let calibration = StartupCalibration::measure(samples)?;
        let anchor = *samples
            .last()
            .ok_or(AvCaptureError::InvalidCalibrationCount)?;
        active.timebase = Some(SourceTimebase::new(policy, calibration, anchor)?);
        Ok(calibration)
    }

    pub fn request_permission(
        &mut self,
        class: AvSourceClass,
    ) -> Result<AvSessionAction, AvCaptureError> {
        if self.state != AvSessionState::Idle || self.pending.is_some() {
            return Err(AvCaptureError::InvalidSessionTransition);
        }
        self.begin_operation(
            AvNativeRequest::RequestPermission(class),
            None,
            None,
            Vec::new(),
            AvSessionState::Idle,
            AvSessionState::Idle,
        )
    }

    pub fn request_start(
        &mut self,
        capabilities: NativeAvBridgeCapabilities,
        catalog: AvDeviceCatalog,
        settings: AvCaptureSettingsV2,
        camera_preview_enabled: bool,
    ) -> Result<AvSessionAction, AvCaptureError> {
        if self.state != AvSessionState::Idle || self.pending.is_some() {
            return Err(AvCaptureError::InvalidSessionTransition);
        }
        let capabilities = capabilities.validate()?;
        if capabilities.adapter != self.owner.adapter || catalog.adapter() != self.owner.adapter {
            return Err(AvCaptureError::OwnerMismatch);
        }
        let settings = settings.validate()?;
        let graph = AvPipelineGraphSpec::negotiate(&catalog, settings, camera_preview_enabled)?;
        let negotiated_preview = graph.camera_preview_enabled;
        let stamps = self.stamps_for_graph(&graph)?;
        let action = self.begin_operation(
            AvNativeRequest::Start(graph),
            Some(capabilities),
            Some(catalog),
            stamps,
            AvSessionState::Starting,
            AvSessionState::Idle,
        )?;
        if let Some(pending) = self.pending.as_mut() {
            pending.proposed_settings = Some(settings);
            pending.proposed_preview = Some(negotiated_preview);
        }
        Ok(action)
    }

    pub fn request_reconfigure(
        &mut self,
        capabilities: NativeAvBridgeCapabilities,
        catalog: AvDeviceCatalog,
        settings: AvCaptureSettingsV2,
        camera_preview_enabled: bool,
    ) -> Result<AvSessionAction, AvCaptureError> {
        if !matches!(
            self.state,
            AvSessionState::Recording | AvSessionState::Paused
        ) || self.pending.is_some()
        {
            return Err(AvCaptureError::InvalidSessionTransition);
        }
        let return_state = self.state;
        let capabilities = capabilities.validate()?;
        if capabilities.adapter != self.owner.adapter || catalog.adapter() != self.owner.adapter {
            return Err(AvCaptureError::OwnerMismatch);
        }
        let settings = settings.validate()?;
        let graph = AvPipelineGraphSpec::negotiate(&catalog, settings, camera_preview_enabled)?;
        let negotiated_preview = graph.camera_preview_enabled;
        let stamps = self.stamps_for_graph(&graph)?;
        let action = self.begin_operation(
            AvNativeRequest::Reconfigure(graph),
            Some(capabilities),
            Some(catalog),
            stamps,
            AvSessionState::Reconfiguring,
            return_state,
        )?;
        if let Some(pending) = self.pending.as_mut() {
            pending.proposed_settings = Some(settings);
            pending.proposed_preview = Some(negotiated_preview);
        }
        Ok(action)
    }

    pub fn request_pause(&mut self) -> Result<AvSessionAction, AvCaptureError> {
        if self.state != AvSessionState::Recording || self.pending.is_some() {
            return Err(AvCaptureError::InvalidSessionTransition);
        }
        self.begin_operation(
            AvNativeRequest::Pause,
            None,
            None,
            self.current_stamps(),
            AvSessionState::Pausing,
            AvSessionState::Recording,
        )
    }

    pub fn request_resume(
        &mut self,
        capabilities: NativeAvBridgeCapabilities,
        catalog: AvDeviceCatalog,
    ) -> Result<AvSessionAction, AvCaptureError> {
        if !matches!(
            self.state,
            AvSessionState::Paused | AvSessionState::Suspended
        ) || self.pending.is_some()
        {
            return Err(AvCaptureError::InvalidSessionTransition);
        }
        let capabilities = capabilities.validate()?;
        if capabilities.adapter != self.owner.adapter || catalog.adapter() != self.owner.adapter {
            return Err(AvCaptureError::OwnerMismatch);
        }
        self.validate_active_catalog(&catalog)?;
        let stamps = self.bump_active_stamps()?;
        self.begin_operation(
            AvNativeRequest::Resume,
            Some(capabilities),
            Some(catalog),
            stamps,
            AvSessionState::Resuming,
            self.state,
        )
    }

    pub fn request_stop(&mut self) -> Result<Option<AvSessionAction>, AvCaptureError> {
        if matches!(
            self.state,
            AvSessionState::Stopped | AvSessionState::Cancelled
        ) {
            return Ok(None);
        }
        if self.state == AvSessionState::Stopping {
            return Ok(None);
        }
        if let Some(terminal) = self.terminal_request {
            if terminal.kind != AvOperationKind::Stop || self.pending.is_some() {
                return Err(AvCaptureError::InvalidSessionTransition);
            }
            return self.retry_teardown().map(Some);
        }
        let stamps = self.teardown_stamps();
        self.pending = None;
        self.terminal_request = Some(self.allocate_terminal(AvOperationKind::Stop)?);
        self.begin_operation(
            AvNativeRequest::Stop,
            None,
            None,
            stamps,
            AvSessionState::Stopping,
            AvSessionState::TeardownRequired,
        )
        .map(Some)
    }

    pub fn cancel(&mut self) -> Result<Option<AvSessionAction>, AvCaptureError> {
        if matches!(
            self.state,
            AvSessionState::Stopped | AvSessionState::Cancelled
        ) {
            return Ok(None);
        }
        if self.state == AvSessionState::Stopping {
            return Ok(None);
        }
        if let Some(terminal) = self.terminal_request {
            if terminal.kind != AvOperationKind::Cancel || self.pending.is_some() {
                return Err(AvCaptureError::InvalidSessionTransition);
            }
            return self.retry_teardown().map(Some);
        }
        let stamps = self.teardown_stamps();
        self.pending = None;
        self.terminal_request = Some(self.allocate_terminal(AvOperationKind::Cancel)?);
        self.begin_operation(
            AvNativeRequest::Cancel,
            None,
            None,
            stamps,
            AvSessionState::Stopping,
            AvSessionState::TeardownRequired,
        )
        .map(Some)
    }

    /// Reissues a session-scoped teardown after the caller lost an
    /// acknowledgement or its local deadline elapsed. No start path is
    /// reachable from this method.
    pub fn retry_teardown(&mut self) -> Result<AvSessionAction, AvCaptureError> {
        if !matches!(
            self.state,
            AvSessionState::Stopping | AvSessionState::TeardownRequired
        ) {
            return Err(AvCaptureError::InvalidSessionTransition);
        }
        let request = match self
            .terminal_request
            .ok_or(AvCaptureError::InvalidSessionTransition)?
            .kind
        {
            AvOperationKind::Cancel => AvNativeRequest::Cancel,
            AvOperationKind::Stop => AvNativeRequest::Stop,
            _ => return Err(AvCaptureError::InvalidSessionTransition),
        };
        let stamps = self.teardown_stamps();
        self.pending = None;
        self.begin_operation(
            request,
            None,
            None,
            stamps,
            AvSessionState::Stopping,
            AvSessionState::TeardownRequired,
        )
    }

    fn allocate_terminal(
        &mut self,
        kind: AvOperationKind,
    ) -> Result<TerminalRequestState, AvCaptureError> {
        let id = AvTerminalId(self.next_terminal);
        self.next_terminal = self
            .next_terminal
            .checked_add(1)
            .ok_or(AvCaptureError::TerminalIdExhausted)?;
        Ok(TerminalRequestState { id, kind })
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_operation(
        &mut self,
        request: AvNativeRequest,
        expected_capabilities: Option<NativeAvBridgeCapabilities>,
        expected_catalog: Option<AvDeviceCatalog>,
        stamps: Vec<AvSourceStamp>,
        transition: AvSessionState,
        return_state: AvSessionState,
    ) -> Result<AvSessionAction, AvCaptureError> {
        let predecessor_stamps = match &request {
            AvNativeRequest::Start(_) => self.unconfirmed_stamps.clone(),
            AvNativeRequest::Reconfigure(_) | AvNativeRequest::Resume => {
                self.active_and_unconfirmed_stamps()
            }
            _ => Vec::new(),
        };
        let operation = AvOperationId(self.next_operation);
        self.next_operation = self
            .next_operation
            .checked_add(1)
            .ok_or(AvCaptureError::OperationIdExhausted)?;
        let kind = request.kind();
        let terminal_id = match kind {
            AvOperationKind::Stop | AvOperationKind::Cancel => Some(
                self.terminal_request
                    .ok_or(AvCaptureError::InvalidSessionTransition)?
                    .id,
            ),
            _ => None,
        };
        self.pending = Some(PendingAvOperation {
            id: operation,
            kind,
            request,
            expected_capabilities,
            expected_catalog,
            stamps,
            predecessor_stamps,
            terminal_id,
            proposed_settings: None,
            proposed_preview: None,
            return_state,
            dispatched: false,
        });
        self.state = transition;
        Ok(AvSessionAction {
            owner: self.owner,
            operation,
            kind,
        })
    }

    fn stamps_for_graph(
        &mut self,
        graph: &AvPipelineGraphSpec,
    ) -> Result<Vec<AvSourceStamp>, AvCaptureError> {
        let mut stamps = Vec::with_capacity(graph.sources.len());
        for source in &graph.sources {
            stamps.push(AvSourceStamp {
                owner: self.owner,
                class: source.class,
                device: source.device,
                generation: source.generation,
                stream_epoch: self.issue_epoch(source.class)?,
            });
        }
        Ok(stamps)
    }

    fn bump_active_stamps(&mut self) -> Result<Vec<AvSourceStamp>, AvCaptureError> {
        let active: Vec<_> = self.active.values().map(|source| source.stamp).collect();
        let mut stamps = Vec::with_capacity(active.len());
        for stamp in active {
            stamps.push(AvSourceStamp {
                stream_epoch: self.issue_epoch(stamp.class())?,
                ..stamp
            });
        }
        Ok(stamps)
    }

    fn issue_epoch(&mut self, class: AvSourceClass) -> Result<AvStreamEpoch, AvCaptureError> {
        let epoch = self
            .last_issued_epochs
            .get(&class)
            .copied()
            .map_or(Ok(AvStreamEpoch::first()), AvStreamEpoch::next)?;
        self.last_issued_epochs.insert(class, epoch);
        Ok(epoch)
    }

    fn current_stamps(&self) -> Vec<AvSourceStamp> {
        self.active.values().map(|active| active.stamp).collect()
    }

    fn validate_active_catalog(&self, catalog: &AvDeviceCatalog) -> Result<(), AvCaptureError> {
        if self.active.values().any(|active| {
            catalog
                .exact(active.stamp.class(), active.stamp.device)
                .is_none_or(|device| {
                    device.generation() != active.stamp.generation()
                        || device.permission() != PermissionState::Granted
                        || !device.supports(active.format)
                })
        }) {
            return Err(AvCaptureError::ResumeRequiresReconfiguration);
        }
        Ok(())
    }

    fn teardown_stamps(&self) -> Vec<AvSourceStamp> {
        let mut stamps = self.active_and_unconfirmed_stamps();
        if let Some(pending) = &self.pending {
            for stamp in &pending.stamps {
                if !stamps.contains(stamp) {
                    stamps.push(*stamp);
                }
            }
        }
        stamps
    }

    fn active_and_unconfirmed_stamps(&self) -> Vec<AvSourceStamp> {
        let mut stamps = self.current_stamps();
        for stamp in &self.unconfirmed_stamps {
            if !stamps.contains(stamp) {
                stamps.push(*stamp);
            }
        }
        stamps
    }

    fn remember_unconfirmed(&mut self, stamps: impl IntoIterator<Item = AvSourceStamp>) {
        for stamp in stamps {
            if !self.unconfirmed_stamps.contains(&stamp) {
                self.unconfirmed_stamps.push(stamp);
            }
        }
    }

    fn bind_failure(
        &mut self,
        operation: AvOperationId,
        kind: AvOperationKind,
        failure: NativeAvFailure,
    ) -> Result<AvActionExecution, AvCaptureError> {
        let pending = self
            .pending
            .as_ref()
            .ok_or(AvCaptureError::StaleOperation)?;
        if pending.id != operation || pending.kind != kind {
            return Err(AvCaptureError::StaleOperation);
        }
        Ok(AvActionExecution::Failed(OwnedAvOperationFailure {
            owner: self.owner,
            operation,
            kind,
            failure,
        }))
    }

    pub fn complete(&mut self, owned: OwnedAvAcknowledgement) -> Result<(), AvCaptureError> {
        let acknowledgement = owned.acknowledgement;
        if acknowledgement.owner != self.owner {
            return Err(AvCaptureError::OwnerMismatch);
        }
        let pending = self.pending.take().ok_or(AvCaptureError::StaleOperation)?;
        if acknowledgement.operation != pending.id
            || acknowledgement.kind != pending.kind
            || acknowledgement.stamps != pending.stamps
            || acknowledgement.predecessor_stamps != pending.predecessor_stamps
            || acknowledgement.terminal_id != pending.terminal_id
            || !pending.dispatched
        {
            self.pending = Some(pending);
            return Err(AvCaptureError::InvalidNativeAcknowledgement);
        }
        if let Some(catalog) = pending.expected_catalog.clone() {
            self.catalog = Some(catalog);
        }
        if let Some(settings) = pending.proposed_settings {
            self.settings = settings;
        }
        if let Some(preview) = pending.proposed_preview {
            self.camera_preview_enabled = preview;
        }
        match pending.request {
            AvNativeRequest::RequestPermission(_) => self.state = AvSessionState::Idle,
            AvNativeRequest::Start(graph) | AvNativeRequest::Reconfigure(graph) => {
                self.install_graph(graph, acknowledgement.stamps)?;
                self.unconfirmed_stamps.clear();
                if pending.kind == AvOperationKind::Start {
                    self.state = AvSessionState::Recording;
                } else {
                    self.state = pending.return_state;
                }
            }
            AvNativeRequest::Pause => {
                self.drain_queues();
                self.state = AvSessionState::Paused;
            }
            AvNativeRequest::Resume => {
                self.replace_active_stamps(acknowledgement.stamps)?;
                self.unconfirmed_stamps.clear();
                self.state = AvSessionState::Recording;
            }
            AvNativeRequest::Stop | AvNativeRequest::Cancel => {
                if !acknowledgement.teardown_complete {
                    self.remember_unconfirmed(
                        acknowledgement
                            .stamps
                            .into_iter()
                            .chain(acknowledgement.predecessor_stamps),
                    );
                    self.state = AvSessionState::TeardownRequired;
                    return Err(AvCaptureError::TeardownNotConfirmed);
                }
                self.drain_queues();
                self.active.clear();
                self.unconfirmed_stamps.clear();
                self.graph = None;
                let terminal_id = acknowledgement
                    .terminal_id
                    .ok_or(AvCaptureError::InvalidNativeAcknowledgement)?;
                let terminal = self
                    .terminal_request
                    .ok_or(AvCaptureError::InvalidNativeAcknowledgement)?;
                if terminal.id != terminal_id || terminal.kind != pending.kind {
                    return Err(AvCaptureError::InvalidNativeAcknowledgement);
                }
                self.terminal_result = Some(AvTerminalResult {
                    id: terminal_id,
                    kind: pending.kind,
                });
                self.state = if pending.kind == AvOperationKind::Cancel {
                    AvSessionState::Cancelled
                } else {
                    AvSessionState::Stopped
                };
            }
        }
        Ok(())
    }

    pub fn complete_failure(
        &mut self,
        owned: OwnedAvOperationFailure,
    ) -> Result<NativeAvFailure, AvCaptureError> {
        if owned.owner != self.owner {
            return Err(AvCaptureError::OwnerMismatch);
        }
        let pending = self.pending.take().ok_or(AvCaptureError::StaleOperation)?;
        if pending.id != owned.operation || pending.kind != owned.kind {
            self.pending = Some(pending);
            return Err(AvCaptureError::StaleOperation);
        }
        if pending.dispatched {
            self.remember_unconfirmed(
                pending
                    .stamps
                    .iter()
                    .chain(&pending.predecessor_stamps)
                    .copied(),
            );
        }
        self.state = match pending.kind {
            AvOperationKind::Stop | AvOperationKind::Cancel => AvSessionState::TeardownRequired,
            AvOperationKind::Start | AvOperationKind::RequestPermission(_) => AvSessionState::Idle,
            _ => pending.return_state,
        };
        Ok(owned.failure)
    }

    fn install_graph(
        &mut self,
        graph: AvPipelineGraphSpec,
        stamps: Vec<AvSourceStamp>,
    ) -> Result<(), AvCaptureError> {
        if graph.sources.len() != stamps.len() {
            return Err(AvCaptureError::InvalidNativeAcknowledgement);
        }
        self.drain_queues();
        self.active.clear();
        for (source, stamp) in graph.sources.iter().zip(stamps) {
            if stamp.owner != self.owner
                || stamp.class != source.class
                || stamp.device != source.device
                || stamp.generation != source.generation
            {
                return Err(AvCaptureError::InvalidNativeAcknowledgement);
            }
            let format = match source.input_caps {
                ExactCapsSpec::Audio(caps) => AvFormat::Audio(caps.format),
                ExactCapsSpec::Camera(caps) => AvFormat::Camera(caps.format),
            };
            let queue = AvIngressQueue::new(stamp, format, source.queue)?;
            self.active.insert(
                source.class,
                ActiveAvSource {
                    stamp,
                    format,
                    queue,
                    timebase: None,
                },
            );
        }
        self.graph = Some(graph);
        Ok(())
    }

    fn replace_active_stamps(&mut self, stamps: Vec<AvSourceStamp>) -> Result<(), AvCaptureError> {
        if stamps.len() != self.active.len() {
            return Err(AvCaptureError::InvalidNativeAcknowledgement);
        }
        for stamp in stamps {
            let active = self
                .active
                .get_mut(&stamp.class)
                .ok_or(AvCaptureError::InvalidNativeAcknowledgement)?;
            if stamp.owner != self.owner
                || stamp.device != active.stamp.device
                || stamp.generation != active.stamp.generation
                || stamp.stream_epoch.get() <= active.stamp.stream_epoch.get()
            {
                return Err(AvCaptureError::InvalidNativeAcknowledgement);
            }
            active.queue.drain();
            active.queue = AvIngressQueue::new(stamp, active.format, active.queue.spec)?;
            active.stamp = stamp;
            active.timebase = None;
        }
        Ok(())
    }

    fn drain_queues(&mut self) {
        for active in self.active.values_mut() {
            active.queue.drain();
        }
    }

    fn accept_control_stamp(&mut self, stamp: AvControlEventStamp) -> Result<(), AvCaptureError> {
        if stamp.owner != self.owner
            || stamp.sequence <= self.last_control_sequence
            || stamp.revision < self.last_control_revision
        {
            return Err(AvCaptureError::StaleControlEvent);
        }
        self.last_control_revision = stamp.revision;
        self.last_control_sequence = stamp.sequence;
        Ok(())
    }

    fn invalidate_pending_for_control(&mut self, class: Option<AvSourceClass>) -> bool {
        let affected = self.pending.as_ref().is_some_and(|pending| {
            matches!(
                pending.kind,
                AvOperationKind::Start | AvOperationKind::Reconfigure | AvOperationKind::Resume
            ) && class.is_none_or(|class| {
                pending
                    .stamps
                    .iter()
                    .chain(&pending.predecessor_stamps)
                    .any(|stamp| stamp.class() == class)
            })
        });
        if !affected {
            return false;
        }
        let Some(pending) = self.pending.take() else {
            return false;
        };
        let ambiguous = pending.dispatched
            && (!pending.stamps.is_empty() || !pending.predecessor_stamps.is_empty());
        if ambiguous {
            self.remember_unconfirmed(pending.stamps.into_iter().chain(pending.predecessor_stamps));
            self.state = AvSessionState::TeardownRequired;
        } else {
            self.state = match pending.kind {
                AvOperationKind::Start => AvSessionState::Idle,
                _ => pending.return_state,
            };
        }
        true
    }

    pub fn apply_owned_event(
        &mut self,
        owned: OwnedNativeAvEvent,
    ) -> Result<AvEventOutcome, AvCaptureError> {
        if owned.owner != self.owner {
            return Err(AvCaptureError::OwnerMismatch);
        }
        let mut outcome = AvEventOutcome::empty();
        match &owned.event {
            NativeAvEvent::PermissionChanged { stamp, class, .. } => {
                self.accept_control_stamp(*stamp)?;
                if self.invalidate_pending_for_control(Some(*class)) {
                    outcome.native_reconfigure_required = true;
                }
            }
            NativeAvEvent::CatalogChanged { stamp, catalog, .. } => {
                if catalog.adapter() != self.owner.adapter || stamp.revision() != catalog.revision()
                {
                    return Err(AvCaptureError::InvalidControlEventStamp);
                }
                self.accept_control_stamp(*stamp)?;
                if self.invalidate_pending_for_control(None) {
                    outcome.native_reconfigure_required = true;
                }
            }
            _ => {}
        }
        match owned.event {
            NativeAvEvent::Buffer(buffer) => {
                if self.state != AvSessionState::Recording {
                    return Err(AvCaptureError::SessionNotAcceptingBuffers);
                }
                let mut buffer = buffer;
                let class = buffer.stamp().class();
                let active = self
                    .active
                    .get_mut(&class)
                    .ok_or(AvCaptureError::StaleSourceStamp)?;
                if buffer.stamp() != active.stamp {
                    return Err(AvCaptureError::StaleSourceStamp);
                }
                if buffer.format() != active.format {
                    return Err(AvCaptureError::FormatRenegotiationRequired);
                }
                if buffer.sequence() != active.queue.expected_sequence()? {
                    return Err(AvCaptureError::OutOfOrderBuffer);
                }
                let timing = buffer.timing();
                let corrected = active
                    .timebase
                    .as_mut()
                    .ok_or(AvCaptureError::CalibrationRequired)?
                    .observe(
                        timing.source_pts_ns,
                        timing.duration_ns,
                        timing.arrival,
                        timing.latency,
                        timing.discontinuity,
                    )?;
                buffer.apply_corrected_timestamp(corrected.frame)?;
                let capability = AvCapabilityBucket::from_format(active.format);
                let pushed = active.queue.push(buffer)?;
                if pushed != AvQueuePush::Accepted {
                    outcome.diagnostics.push(AvDiagnostic {
                        version: AV_DIAGNOSTIC_VERSION,
                        class: Some(class),
                        route: None,
                        capability: Some(capability),
                        timing: None,
                        code: AvStableCode::IngressOverload,
                    });
                }
                outcome.queue = Some(pushed);
            }
            NativeAvEvent::PermissionChanged {
                stamp: _,
                class,
                state,
            } => {
                if matches!(
                    state,
                    PermissionState::Denied
                        | PermissionState::Restricted
                        | PermissionState::Revoked
                ) {
                    let capability = self
                        .active
                        .get(&class)
                        .map(|active| AvCapabilityBucket::from_format(active.format));
                    self.disable_source(class);
                    outcome.disabled_sources.push(class);
                    outcome.native_reconfigure_required = true;
                    outcome.diagnostics.push(AvDiagnostic {
                        version: AV_DIAGNOSTIC_VERSION,
                        class: Some(class),
                        route: None,
                        capability,
                        timing: None,
                        code: if state == PermissionState::Revoked {
                            AvStableCode::PermissionRevoked
                        } else {
                            AvStableCode::PermissionDenied
                        },
                    });
                }
            }
            NativeAvEvent::CatalogChanged {
                stamp: _,
                catalog,
                reason,
            } => {
                if catalog.adapter() != self.owner.adapter {
                    return Err(AvCaptureError::OwnerMismatch);
                }
                for class in [
                    AvSourceClass::Microphone,
                    AvSourceClass::SystemAudio,
                    AvSourceClass::Camera,
                ] {
                    let Some(active) = self.active.get(&class) else {
                        continue;
                    };
                    let capability = AvCapabilityBucket::from_format(active.format);
                    let live = catalog.exact(class, active.stamp.device);
                    let invalid = live.is_none_or(|device| {
                        device.generation() != active.stamp.generation
                            || !device.supports(active.format)
                            || device.permission() != PermissionState::Granted
                    });
                    let changed_followed_default = reason == CatalogChangeReason::DefaultChanged
                        && matches!(
                            self.settings.selection(class),
                            DeviceSelectionV2::FollowDefault { .. }
                        )
                        && catalog
                            .default_for(class)
                            .is_some_and(|device| device.id() != active.stamp.device);
                    let default_was_authorized = matches!(
                        self.settings.selection(class),
                        DeviceSelectionV2::FollowDefault {
                            allow_default_changes: true,
                            ..
                        }
                    );
                    if invalid || changed_followed_default {
                        let code = if changed_followed_default && !default_was_authorized {
                            AvStableCode::DefaultConfirmationRequired
                        } else if changed_followed_default {
                            AvStableCode::CapabilityChanged
                        } else if matches!(
                            reason,
                            CatalogChangeReason::CapabilityChanged
                                | CatalogChangeReason::WirelessProfileChanged
                        ) {
                            AvStableCode::FormatRenegotiationRequired
                        } else {
                            AvStableCode::DeviceGenerationChanged
                        };
                        self.disable_source(class);
                        outcome.disabled_sources.push(class);
                        outcome.native_reconfigure_required = true;
                        outcome.diagnostics.push(AvDiagnostic {
                            version: AV_DIAGNOSTIC_VERSION,
                            class: Some(class),
                            route: live.map(AvDeviceDescriptor::route),
                            capability: Some(capability),
                            timing: None,
                            code,
                        });
                    }
                }
                self.catalog = Some(catalog);
            }
            NativeAvEvent::Overload {
                stamp,
                dropped_buffers: _,
            } => {
                self.verify_active_stamp(stamp)?;
                let capability = self
                    .active
                    .get(&stamp.class())
                    .map(|active| AvCapabilityBucket::from_format(active.format));
                outcome.diagnostics.push(AvDiagnostic {
                    version: AV_DIAGNOSTIC_VERSION,
                    class: Some(stamp.class()),
                    route: None,
                    capability,
                    timing: None,
                    code: AvStableCode::IngressOverload,
                });
            }
            NativeAvEvent::SourceFailed { stamp, code } => {
                self.verify_active_stamp(stamp)?;
                let capability = self
                    .active
                    .get(&stamp.class())
                    .map(|active| AvCapabilityBucket::from_format(active.format));
                self.disable_source(stamp.class());
                outcome.disabled_sources.push(stamp.class());
                outcome.native_reconfigure_required = true;
                outcome.diagnostics.push(AvDiagnostic {
                    version: AV_DIAGNOSTIC_VERSION,
                    class: Some(stamp.class()),
                    route: None,
                    capability,
                    timing: None,
                    code: match code {
                        NativeAvFailureCode::PermissionDenied
                        | NativeAvFailureCode::PermissionRestricted => {
                            AvStableCode::PermissionDenied
                        }
                        NativeAvFailureCode::CapabilityChanged
                        | NativeAvFailureCode::FormatChanged => {
                            AvStableCode::FormatRenegotiationRequired
                        }
                        _ => AvStableCode::OptionalSourceDisabled,
                    },
                });
            }
            NativeAvEvent::Sleep => {
                if !matches!(
                    self.state,
                    AvSessionState::Stopping
                        | AvSessionState::Stopped
                        | AvSessionState::Cancelled
                        | AvSessionState::TeardownRequired
                ) {
                    let prior_state = self.state;
                    let pending = self.pending.take();
                    let ambiguous_stream = pending.as_ref().is_some_and(|operation| {
                        operation.dispatched
                            && (!operation.stamps.is_empty()
                                || !operation.predecessor_stamps.is_empty())
                    });
                    if let Some(operation) = pending
                        && ambiguous_stream
                    {
                        self.remember_unconfirmed(
                            operation
                                .stamps
                                .into_iter()
                                .chain(operation.predecessor_stamps),
                        );
                    }
                    self.drain_queues();
                    self.state = if ambiguous_stream {
                        AvSessionState::TeardownRequired
                    } else if self.active.is_empty()
                        && matches!(prior_state, AvSessionState::Starting | AvSessionState::Idle)
                    {
                        AvSessionState::Idle
                    } else {
                        AvSessionState::Suspended
                    };
                }
                outcome.diagnostics.push(AvDiagnostic {
                    version: AV_DIAGNOSTIC_VERSION,
                    class: None,
                    route: None,
                    capability: None,
                    timing: None,
                    code: AvStableCode::Sleep,
                });
            }
            NativeAvEvent::Wake => {
                outcome.diagnostics.push(AvDiagnostic {
                    version: AV_DIAGNOSTIC_VERSION,
                    class: None,
                    route: None,
                    capability: None,
                    timing: None,
                    code: AvStableCode::Wake,
                });
            }
        }
        Ok(outcome)
    }

    fn verify_active_stamp(&self, stamp: AvSourceStamp) -> Result<(), AvCaptureError> {
        if stamp.owner != self.owner
            || self
                .active
                .get(&stamp.class())
                .is_none_or(|active| active.stamp != stamp)
        {
            return Err(AvCaptureError::StaleSourceStamp);
        }
        Ok(())
    }

    fn disable_source(&mut self, class: AvSourceClass) {
        if let Some(mut active) = self.active.remove(&class) {
            self.remember_unconfirmed([active.stamp]);
            active.queue.drain();
        }
        if class == AvSourceClass::Camera {
            self.camera_preview_enabled = false;
        }
    }

    pub fn pop_buffer(
        &mut self,
        class: AvSourceClass,
        now: MonotonicTimeNs,
    ) -> Result<Option<NativeAvBuffer>, AvCaptureError> {
        if !matches!(
            self.state,
            AvSessionState::Recording | AvSessionState::Paused
        ) {
            return Err(AvCaptureError::SessionNotAcceptingBuffers);
        }
        self.active
            .get_mut(&class)
            .map_or(Ok(None), |active| active.queue.pop(now))
    }

    pub fn poll_source<B: NativeAvBridge>(
        &mut self,
        source: &mut BoundNativeAvBridge<B>,
    ) -> Result<Option<AvEventOutcome>, AvCaptureError> {
        if source.binding() != self.owner {
            return Err(AvCaptureError::OwnerMismatch);
        }
        source
            .poll_owned()?
            .map(|event| self.apply_owned_event(event))
            .transpose()
    }
}

#[derive(Debug, Error)]
pub enum AvCaptureError {
    #[error("A/V device ID is invalid")]
    InvalidDeviceId,
    #[error("A/V adapter instance ID is invalid")]
    InvalidAdapterInstanceId,
    #[error("A/V session ID is invalid")]
    InvalidSessionId,
    #[error("A/V device generation is invalid")]
    InvalidDeviceGeneration,
    #[error("A/V format count is outside the bounded contract")]
    InvalidFormatCount,
    #[error("A/V format appears more than once")]
    DuplicateFormat,
    #[error("A/V format does not match its source class")]
    FormatClassMismatch,
    #[error("A/V format exceeds the hard safety ceiling")]
    FormatTooLarge,
    #[error("A/V device catalog is invalid")]
    InvalidCatalog,
    #[error("A/V device appears more than once")]
    DuplicateDevice,
    #[error("A/V catalog declares multiple default devices for one class")]
    MultipleDefaults,
    #[error("A/V bridge contract version does not match")]
    ContractVersionMismatch,
    #[error("A/V bridge omits a mandatory capability")]
    MissingBridgeCapability,
    #[error("A/V settings version does not match")]
    SettingsVersionMismatch,
    #[error("persisted A/V settings exceed their hard size bound")]
    PersistedSettingsTooLarge,
    #[error("persisted A/V settings are malformed")]
    MalformedPersistedSettings,
    #[error("legacy A/V selection is incomplete")]
    IncompleteLegacySelection,
    #[error("selected A/V device does not support the exact persisted format")]
    UnsupportedExactFormat,
    #[error("selected A/V source requires permission")]
    PermissionNotGranted,
    #[error("a changed default A/V device requires explicit confirmation")]
    DefaultChangeNeedsConfirmation,
    #[error("A/V queue specification is invalid")]
    InvalidQueueSpec,
    #[error("A/V source latency is invalid")]
    InvalidLatency,
    #[error("startup calibration sample count is invalid")]
    InvalidCalibrationCount,
    #[error("master monotonic clock moved backwards or repeated")]
    NonMonotonicMasterClock,
    #[error("A/V timestamp exceeds the supported range")]
    TimestampRange,
    #[error("A/V synchronization policy is invalid")]
    InvalidSyncPolicyV2,
    #[error("measured startup offset exceeds the synchronization charter")]
    StartupOffsetBudgetExceeded,
    #[error("source timestamp rolled back without a declared discontinuity")]
    SourceTimestampRollback,
    #[error("A/V clock jump requires an explicit discontinuity")]
    ClockDiscontinuityRequired,
    #[error("corrected A/V offset exceeds the enforced long-run synchronization budget")]
    SynchronizationBudgetExceeded,
    #[error("A/V timebase transition is invalid")]
    InvalidTimebaseTransition,
    #[error("A/V timebase is paused")]
    TimebasePaused,
    #[error("corrected frame timestamp is invalid")]
    InvalidFrameTimestamp,
    #[error("audio mix settings are invalid")]
    InvalidMixSettings,
    #[error("audio source is not mixable")]
    UnknownAudioSource,
    #[error("audio block is empty")]
    EmptyAudioBlock,
    #[error("audio block exceeds its hard safety limit")]
    AudioBlockTooLarge,
    #[error("audio block shape or samples are invalid")]
    InvalidAudioBlock,
    #[error("audio block would break declared timeline continuity")]
    AudioTimelineDiscontinuity,
    #[error("UI event throttle interval is invalid")]
    InvalidUiThrottle,
    #[error("native buffer lease is invalid")]
    InvalidBufferLease,
    #[error("native buffer timing is invalid")]
    InvalidNativeBufferTiming,
    #[error("native buffer has not passed the session-owned timebase")]
    UncorrectedBuffer,
    #[error("native buffer was corrected more than once")]
    BufferAlreadyCorrected,
    #[error("native buffer payload was already transferred")]
    PayloadAlreadyTaken,
    #[error("byte payload length does not match its immutable retained-size snapshot")]
    PayloadSizeMismatch,
    #[error("native buffer arrived out of sequence")]
    OutOfOrderBuffer,
    #[error("native buffer sequence is exhausted")]
    BufferSequenceExhausted,
    #[error("corrected buffer timestamp moved backwards")]
    CorrectedTimestampRollback,
    #[error("the active A/V source has not been calibrated for this stream epoch")]
    CalibrationRequired,
    #[error("A/V control event stamp is invalid")]
    InvalidControlEventStamp,
    #[error("A/V control event is stale, replayed, or out of order")]
    StaleControlEvent,
    #[error("A/V source stamp is stale or belongs to another session")]
    StaleSourceStamp,
    #[error("A/V source changed format without negotiated reconfiguration")]
    FormatRenegotiationRequired,
    #[error("A/V source owner does not match")]
    OwnerMismatch,
    #[error("the bound A/V adapter already constructed its one session")]
    SessionAlreadyClaimed,
    #[error("A/V operation is stale, replayed, or superseded")]
    StaleOperation,
    #[error("native A/V acknowledgement is invalid")]
    InvalidNativeAcknowledgement,
    #[error("A/V session transition is invalid")]
    InvalidSessionTransition,
    #[error("A/V resume requires a fresh exact source reconfiguration")]
    ResumeRequiresReconfiguration,
    #[error("A/V operation sequence is exhausted")]
    OperationIdExhausted,
    #[error("A/V terminal identity sequence is exhausted")]
    TerminalIdExhausted,
    #[error("A/V operation timeout policy is invalid")]
    InvalidOperationPolicy,
    #[error("native terminal postcondition does not match the stable terminal request")]
    InvalidTerminalPostcondition,
    #[error("A/V stream epoch is exhausted")]
    StreamEpochExhausted,
    #[error("native teardown was not confirmed")]
    TeardownNotConfirmed,
    #[error("A/V session is not accepting buffers")]
    SessionNotAcceptingBuffers,
    #[error(transparent)]
    Capture(#[from] crate::CaptureError),
    #[error(transparent)]
    SettingsStorage(#[from] AvSettingsStorageError),
    #[error(transparent)]
    Native(#[from] NativeAvFailure),
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    struct TestLease {
        released: Arc<AtomicUsize>,
        payload: Option<AvPayloadBody>,
    }

    impl AvBufferLease for TestLease {
        fn retained_bytes(&self) -> u64 {
            32
        }

        fn take_payload(&mut self) -> Option<AvPayloadBody> {
            self.payload.take()
        }

        fn release(self: Box<Self>) {
            self.released.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn corrected_buffer(
        stamp: AvSourceStamp,
        sequence: u64,
        released: &Arc<AtomicUsize>,
    ) -> NativeAvBuffer {
        let mut buffer = NativeAvBuffer::new(
            stamp,
            NativeAvBufferTiming {
                sequence,
                source_pts_ns: sequence * 10,
                duration_ns: 10,
                arrival: MonotonicTimeNs::new(sequence * 10),
                latency: SourceLatency {
                    reported_ns: 0,
                    confidence: LatencyConfidence::Measured,
                },
                discontinuity: false,
            },
            AvFormat::Audio(AudioFormat {
                sample_rate: 48_000,
                channels: 1,
                sample_format: crate::AudioSampleFormat::Float32,
            }),
            Box::new(TestLease {
                released: Arc::clone(released),
                payload: Some(AvPayloadBody::Bytes(vec![0; 32])),
            }),
        )
        .expect("test buffer");
        buffer
            .apply_corrected_timestamp(FrameTimestamp {
                pts_ns: (sequence - 1) * 10,
                duration_ns: 10,
                discontinuity: false,
            })
            .expect("test correction");
        buffer
    }

    #[test]
    fn drop_newest_releases_the_rejected_lease_and_advances_sequence() {
        let owner = AvOwnerBinding {
            adapter: AvAdapterInstanceId::from_opaque([1; 16]).expect("adapter"),
            session: AvSessionId::from_opaque([2; 16]).expect("session"),
        };
        let stamp = AvSourceStamp {
            owner,
            class: AvSourceClass::Microphone,
            device: AvDeviceId::from_opaque([3; 16]).expect("device"),
            generation: AvDeviceGeneration::new(1).expect("generation"),
            stream_epoch: AvStreamEpoch::first(),
        };
        let mut queue = AvIngressQueue::new(
            stamp,
            AvFormat::Audio(AudioFormat {
                sample_rate: 48_000,
                channels: 1,
                sample_format: crate::AudioSampleFormat::Float32,
            }),
            AvQueueSpec {
                max_buffers: 1,
                max_bytes: 64,
                max_age_ns: 1_000,
                backpressure: AvBackpressurePolicy::DropNewest,
                producer_blocks: false,
            },
        )
        .expect("queue");
        let released = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            queue
                .push(corrected_buffer(stamp, 1, &released))
                .expect("first push"),
            AvQueuePush::Accepted
        );
        assert_eq!(
            queue
                .push(corrected_buffer(stamp, 2, &released))
                .expect("second push"),
            AvQueuePush::DroppedNewest
        );
        assert_eq!(released.load(Ordering::SeqCst), 1);
        assert_eq!(queue.drain(), (1, 32));
        assert_eq!(released.load(Ordering::SeqCst), 2);
    }
}
