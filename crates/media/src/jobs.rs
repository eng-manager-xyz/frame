use std::fmt;

use thiserror::Error;

pub const MEDIA_JOB_CATALOG_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaJobKind {
    OptimizedClip,
    Frame,
    Spritesheet,
    AudioExtract,
    Probe,
    RemuxRepair,
    Waveform,
    Composition,
    Normalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaExecutorKind {
    CloudflareMedia,
    NativeGstreamer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressCapability {
    Monotonic,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationCapability {
    InFlight,
    SuppressPublication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaJobSpec {
    pub kind: MediaJobKind,
    pub managed_supported: bool,
    pub native_supported: bool,
    pub preferred: MediaExecutorKind,
    pub progress: ProgressCapability,
    pub cancellation: CancellationCapability,
    pub timeout_ms: u64,
    pub fallback_to_native: bool,
}

const JOBS: &[MediaJobSpec] = &[
    MediaJobSpec {
        kind: MediaJobKind::OptimizedClip,
        managed_supported: true,
        native_supported: true,
        preferred: MediaExecutorKind::CloudflareMedia,
        progress: ProgressCapability::Indeterminate,
        cancellation: CancellationCapability::SuppressPublication,
        timeout_ms: 120_000,
        fallback_to_native: true,
    },
    MediaJobSpec {
        kind: MediaJobKind::Frame,
        managed_supported: true,
        native_supported: true,
        preferred: MediaExecutorKind::CloudflareMedia,
        progress: ProgressCapability::Indeterminate,
        cancellation: CancellationCapability::SuppressPublication,
        timeout_ms: 60_000,
        fallback_to_native: true,
    },
    MediaJobSpec {
        kind: MediaJobKind::Spritesheet,
        managed_supported: true,
        native_supported: true,
        preferred: MediaExecutorKind::CloudflareMedia,
        progress: ProgressCapability::Indeterminate,
        cancellation: CancellationCapability::SuppressPublication,
        timeout_ms: 120_000,
        fallback_to_native: true,
    },
    MediaJobSpec {
        kind: MediaJobKind::AudioExtract,
        managed_supported: true,
        native_supported: true,
        preferred: MediaExecutorKind::CloudflareMedia,
        progress: ProgressCapability::Indeterminate,
        cancellation: CancellationCapability::SuppressPublication,
        timeout_ms: 120_000,
        fallback_to_native: true,
    },
    MediaJobSpec {
        kind: MediaJobKind::Probe,
        managed_supported: false,
        native_supported: true,
        preferred: MediaExecutorKind::NativeGstreamer,
        progress: ProgressCapability::Indeterminate,
        cancellation: CancellationCapability::InFlight,
        timeout_ms: 30_000,
        fallback_to_native: false,
    },
    MediaJobSpec {
        kind: MediaJobKind::RemuxRepair,
        managed_supported: false,
        native_supported: true,
        preferred: MediaExecutorKind::NativeGstreamer,
        progress: ProgressCapability::Monotonic,
        cancellation: CancellationCapability::InFlight,
        timeout_ms: 900_000,
        fallback_to_native: false,
    },
    MediaJobSpec {
        kind: MediaJobKind::Waveform,
        managed_supported: false,
        native_supported: true,
        preferred: MediaExecutorKind::NativeGstreamer,
        progress: ProgressCapability::Monotonic,
        cancellation: CancellationCapability::InFlight,
        timeout_ms: 900_000,
        fallback_to_native: false,
    },
    MediaJobSpec {
        kind: MediaJobKind::Composition,
        managed_supported: false,
        native_supported: true,
        preferred: MediaExecutorKind::NativeGstreamer,
        progress: ProgressCapability::Monotonic,
        cancellation: CancellationCapability::InFlight,
        timeout_ms: 3_600_000,
        fallback_to_native: false,
    },
    MediaJobSpec {
        kind: MediaJobKind::Normalize,
        managed_supported: false,
        native_supported: true,
        preferred: MediaExecutorKind::NativeGstreamer,
        progress: ProgressCapability::Monotonic,
        cancellation: CancellationCapability::InFlight,
        timeout_ms: 900_000,
        fallback_to_native: false,
    },
];

#[derive(Debug, Clone, Copy)]
pub struct MediaJobCatalog {
    pub version: u16,
    pub jobs: &'static [MediaJobSpec],
}

#[must_use]
pub const fn media_job_catalog() -> MediaJobCatalog {
    MediaJobCatalog {
        version: MEDIA_JOB_CATALOG_VERSION,
        jobs: JOBS,
    }
}

impl MediaJobCatalog {
    #[must_use]
    pub fn get(self, kind: MediaJobKind) -> Option<&'static MediaJobSpec> {
        self.jobs.iter().find(|job| job.kind == kind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerFormat {
    Mp4,
    QuickTime,
    WebM,
    Matroska,
    Wave,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
    Vp8,
    Vp9,
    Av1,
    None,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodec {
    Aac,
    Opus,
    Vorbis,
    Pcm,
    None,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaInput {
    pub bytes: u64,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub container: ContainerFormat,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub encrypted: bool,
}

impl MediaInput {
    pub fn validate(self) -> Result<Self, RouteError> {
        if self.bytes == 0
            || self.duration_ms == 0
            || (self.width == 0) != (self.height == 0)
            || (self.video_codec != VideoCodec::None && self.width == 0)
        {
            return Err(RouteError::InvalidInput);
        }
        let _ = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .ok_or(RouteError::InvalidInput)?;
        if self.encrypted {
            return Err(RouteError::EncryptedInput);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedMediaLimits {
    /// Configuration revision/date from the remote contract test lane.
    pub revision: String,
    pub max_bytes: u64,
    pub max_duration_ms: u64,
    pub max_width: u32,
    pub max_height: u32,
}

impl ManagedMediaLimits {
    pub fn validate(self) -> Result<Self, RouteError> {
        if self.revision.trim().is_empty()
            || self.revision.len() > 64
            || !self
                .revision
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
            || self.max_bytes == 0
            || self.max_duration_ms == 0
            || self.max_width == 0
            || self.max_height == 0
        {
            return Err(RouteError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeMediaLimits {
    pub max_bytes: u64,
    pub max_duration_ms: u64,
    pub max_pixels: u64,
}

impl NativeMediaLimits {
    pub fn validate(self) -> Result<Self, RouteError> {
        if self.max_bytes == 0 || self.max_duration_ms == 0 || self.max_pixels == 0 {
            return Err(RouteError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    Bytes,
    Duration,
    Width,
    Height,
    Pixels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteReason {
    ManagedPreferred,
    ManagedDisabled,
    ManagedFormatUnsupported,
    ManagedLimitExceeded(LimitKind),
    NativeOnly,
    ManagedFailure(ExecutionFailureClass),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteDecision {
    pub catalog_version: u16,
    pub kind: MediaJobKind,
    pub executor: MediaExecutorKind,
    pub reason: RouteReason,
    pub fallback_from: Option<MediaExecutorKind>,
    pub fallback_available: bool,
    pub progress: ProgressCapability,
    pub cancellation: CancellationCapability,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct MediaRouter {
    managed_enabled: bool,
    managed_limits: ManagedMediaLimits,
    native_limits: NativeMediaLimits,
}

impl MediaRouter {
    pub fn new(
        managed_enabled: bool,
        managed_limits: ManagedMediaLimits,
        native_limits: NativeMediaLimits,
    ) -> Result<Self, RouteError> {
        Ok(Self {
            managed_enabled,
            managed_limits: managed_limits.validate()?,
            native_limits: native_limits.validate()?,
        })
    }

    pub fn route(
        &self,
        kind: MediaJobKind,
        input: MediaInput,
    ) -> Result<RouteDecision, RouteError> {
        let input = input.validate()?;
        let spec = media_job_catalog()
            .get(kind)
            .ok_or(RouteError::UnknownJob)?;

        if !spec.managed_supported || spec.preferred == MediaExecutorKind::NativeGstreamer {
            if !spec.native_supported {
                return Err(RouteError::NoExecutor);
            }
            self.ensure_native_bounds(input)?;
            return Ok(decision(
                spec,
                MediaExecutorKind::NativeGstreamer,
                RouteReason::NativeOnly,
            ));
        }
        if !self.managed_enabled {
            return self.native_fallback(spec, input, RouteReason::ManagedDisabled);
        }
        if !managed_format_supported(input) {
            return self.native_fallback(spec, input, RouteReason::ManagedFormatUnsupported);
        }
        if let Some(limit) = self.managed_limit_exceeded(input) {
            return self.native_fallback(spec, input, RouteReason::ManagedLimitExceeded(limit));
        }
        let mut routed = decision(
            spec,
            MediaExecutorKind::CloudflareMedia,
            RouteReason::ManagedPreferred,
        );
        routed.fallback_available = spec.fallback_to_native
            && spec.native_supported
            && self.ensure_native_bounds(input).is_ok();
        Ok(routed)
    }

    pub fn fallback_after_failure(
        &self,
        current: RouteDecision,
        failure: ExecutionFailureClass,
    ) -> Result<RouteDecision, RouteError> {
        if current.executor != MediaExecutorKind::CloudflareMedia {
            return Err(RouteError::FallbackUnavailable);
        }
        if matches!(
            failure,
            ExecutionFailureClass::InvalidInput
                | ExecutionFailureClass::SecurityViolation
                | ExecutionFailureClass::Cancelled
        ) {
            return Err(RouteError::FallbackForbidden(failure));
        }
        let spec = media_job_catalog()
            .get(current.kind)
            .ok_or(RouteError::UnknownJob)?;
        if !current.fallback_available || !spec.fallback_to_native || !spec.native_supported {
            return Err(RouteError::FallbackUnavailable);
        }
        let mut fallback = decision(
            spec,
            MediaExecutorKind::NativeGstreamer,
            RouteReason::ManagedFailure(failure),
        );
        fallback.fallback_from = Some(MediaExecutorKind::CloudflareMedia);
        Ok(fallback)
    }

    fn native_fallback(
        &self,
        spec: &MediaJobSpec,
        input: MediaInput,
        reason: RouteReason,
    ) -> Result<RouteDecision, RouteError> {
        if !spec.native_supported {
            return Err(RouteError::NoExecutor);
        }
        self.ensure_native_bounds(input)?;
        let mut routed = decision(spec, MediaExecutorKind::NativeGstreamer, reason);
        routed.fallback_from = Some(MediaExecutorKind::CloudflareMedia);
        Ok(routed)
    }

    fn ensure_native_bounds(&self, input: MediaInput) -> Result<(), RouteError> {
        if input.bytes > self.native_limits.max_bytes {
            return Err(RouteError::NativeLimitExceeded(LimitKind::Bytes));
        }
        if input.duration_ms > self.native_limits.max_duration_ms {
            return Err(RouteError::NativeLimitExceeded(LimitKind::Duration));
        }
        let pixels = u64::from(input.width) * u64::from(input.height);
        if pixels > self.native_limits.max_pixels {
            return Err(RouteError::NativeLimitExceeded(LimitKind::Pixels));
        }
        Ok(())
    }

    fn managed_limit_exceeded(&self, input: MediaInput) -> Option<LimitKind> {
        if input.bytes > self.managed_limits.max_bytes {
            Some(LimitKind::Bytes)
        } else if input.duration_ms > self.managed_limits.max_duration_ms {
            Some(LimitKind::Duration)
        } else if input.width > self.managed_limits.max_width {
            Some(LimitKind::Width)
        } else if input.height > self.managed_limits.max_height {
            Some(LimitKind::Height)
        } else {
            None
        }
    }
}

fn managed_format_supported(input: MediaInput) -> bool {
    input.container == ContainerFormat::Mp4
        && input.video_codec == VideoCodec::H264
        && matches!(input.audio_codec, AudioCodec::Aac | AudioCodec::None)
}

fn decision(
    spec: &MediaJobSpec,
    executor: MediaExecutorKind,
    reason: RouteReason,
) -> RouteDecision {
    RouteDecision {
        catalog_version: MEDIA_JOB_CATALOG_VERSION,
        kind: spec.kind,
        executor,
        reason,
        fallback_from: None,
        fallback_available: false,
        progress: spec.progress,
        cancellation: spec.cancellation,
        timeout_ms: spec.timeout_ms,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionFailureClass {
    Quota,
    Timeout,
    ProviderOutage,
    OutputIncompatible,
    BetaRegression,
    InvalidInput,
    SecurityViolation,
    Cancelled,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PublicationGuard {
    state: PublicationState,
}

impl fmt::Debug for PublicationGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match &self.state {
            PublicationState::Pending => "pending",
            PublicationState::Published(_) => "published",
            PublicationState::Cancelled => "cancelled",
        };
        formatter
            .debug_struct("PublicationGuard")
            .field("state", &state)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PublicationState {
    Pending,
    Published(String),
    Cancelled,
}

impl Default for PublicationGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl PublicationGuard {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: PublicationState::Pending,
        }
    }

    pub fn publish(&mut self, immutable_identity: impl Into<String>) -> Result<bool, RouteError> {
        let identity = immutable_identity.into();
        if identity.is_empty()
            || identity.len() > 256
            || identity.starts_with('/')
            || identity
                .split('/')
                .any(|segment| segment.is_empty() || segment == "..")
        {
            return Err(RouteError::InvalidPublicationIdentity);
        }
        match &self.state {
            PublicationState::Pending => {
                self.state = PublicationState::Published(identity);
                Ok(true)
            }
            PublicationState::Published(existing) if existing == &identity => Ok(false),
            PublicationState::Published(_) => Err(RouteError::PublicationConflict),
            PublicationState::Cancelled => Err(RouteError::PublicationCancelled),
        }
    }

    pub fn cancel(&mut self) -> bool {
        if matches!(self.state, PublicationState::Pending) {
            self.state = PublicationState::Cancelled;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    #[error("media input metadata is invalid")]
    InvalidInput,
    #[error("encrypted media input is unsupported")]
    EncryptedInput,
    #[error("media capability limits are invalid")]
    InvalidLimits,
    #[error("media job is absent from the catalog")]
    UnknownJob,
    #[error("no executor can safely handle the job")]
    NoExecutor,
    #[error("native media safety limit exceeded: {0:?}")]
    NativeLimitExceeded(LimitKind),
    #[error("fallback is unavailable")]
    FallbackUnavailable,
    #[error("fallback is forbidden for failure class {0:?}")]
    FallbackForbidden(ExecutionFailureClass),
    #[error("immutable publication identity is invalid")]
    InvalidPublicationIdentity,
    #[error("a different logical result is already published")]
    PublicationConflict,
    #[error("cancelled work cannot publish a result")]
    PublicationCancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router() -> MediaRouter {
        MediaRouter::new(
            true,
            ManagedMediaLimits {
                revision: "remote-contract-2026-07".into(),
                max_bytes: 1_000,
                max_duration_ms: 2_000,
                max_width: 1_920,
                max_height: 1_080,
            },
            NativeMediaLimits {
                max_bytes: 10_000,
                max_duration_ms: 20_000,
                max_pixels: 8_294_400,
            },
        )
        .expect("router")
    }

    fn input() -> MediaInput {
        MediaInput {
            bytes: 1_000,
            duration_ms: 2_000,
            width: 1_920,
            height: 1_080,
            container: ContainerFormat::Mp4,
            video_codec: VideoCodec::H264,
            audio_codec: AudioCodec::Aac,
            encrypted: false,
        }
    }

    #[test]
    fn exact_managed_limits_route_managed() {
        let routed = router().route(MediaJobKind::Frame, input()).expect("route");
        assert_eq!(routed.executor, MediaExecutorKind::CloudflareMedia);
        assert_eq!(routed.reason, RouteReason::ManagedPreferred);
    }

    #[test]
    fn just_over_managed_limit_routes_native() {
        let routed = router()
            .route(
                MediaJobKind::Frame,
                MediaInput {
                    bytes: 1_001,
                    ..input()
                },
            )
            .expect("route");
        assert_eq!(routed.executor, MediaExecutorKind::NativeGstreamer);
        assert_eq!(
            routed.reason,
            RouteReason::ManagedLimitExceeded(LimitKind::Bytes)
        );
    }

    #[test]
    fn unsupported_managed_format_routes_native() {
        let routed = router()
            .route(
                MediaJobKind::AudioExtract,
                MediaInput {
                    container: ContainerFormat::WebM,
                    video_codec: VideoCodec::Vp8,
                    audio_codec: AudioCodec::Opus,
                    ..input()
                },
            )
            .expect("route");
        assert_eq!(routed.reason, RouteReason::ManagedFormatUnsupported);
    }

    #[test]
    fn provider_outage_falls_back_but_security_failure_does_not() {
        let managed = router()
            .route(MediaJobKind::OptimizedClip, input())
            .expect("managed");
        let fallback = router()
            .fallback_after_failure(managed, ExecutionFailureClass::ProviderOutage)
            .expect("fallback");
        assert_eq!(fallback.executor, MediaExecutorKind::NativeGstreamer);
        assert_eq!(
            fallback.reason,
            RouteReason::ManagedFailure(ExecutionFailureClass::ProviderOutage)
        );
        assert!(matches!(
            router().fallback_after_failure(managed, ExecutionFailureClass::SecurityViolation),
            Err(RouteError::FallbackForbidden(_))
        ));
    }

    #[test]
    fn managed_work_can_run_when_native_fallback_is_over_limit() {
        let managed_only_for_input = MediaRouter::new(
            true,
            ManagedMediaLimits {
                revision: "remote-contract-2026-07".into(),
                max_bytes: 1_000,
                max_duration_ms: 2_000,
                max_width: 1_920,
                max_height: 1_080,
            },
            NativeMediaLimits {
                max_bytes: 500,
                max_duration_ms: 20_000,
                max_pixels: 8_294_400,
            },
        )
        .expect("router");
        let routed = managed_only_for_input
            .route(MediaJobKind::Frame, input())
            .expect("managed route");
        assert_eq!(routed.executor, MediaExecutorKind::CloudflareMedia);
        assert!(!routed.fallback_available);
        assert_eq!(
            managed_only_for_input
                .fallback_after_failure(routed, ExecutionFailureClass::ProviderOutage),
            Err(RouteError::FallbackUnavailable)
        );
    }

    #[test]
    fn publication_is_idempotent_and_cancel_safe() {
        let mut publication = PublicationGuard::new();
        assert!(publication.publish("outputs/revision-1").expect("publish"));
        assert!(!publication.publish("outputs/revision-1").expect("replay"));
        assert!(!format!("{publication:?}").contains("outputs/revision-1"));
        assert_eq!(
            publication.publish("outputs/revision-2"),
            Err(RouteError::PublicationConflict)
        );

        let mut cancelled = PublicationGuard::new();
        assert!(cancelled.cancel());
        assert_eq!(
            cancelled.publish("outputs/revision-1"),
            Err(RouteError::PublicationCancelled)
        );
    }
}
