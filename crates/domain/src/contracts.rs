use std::{collections::BTreeMap, fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{ObjectKey, VideoId};

pub const MAX_WIRE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_TIMESTAMP_MS: i64 = 253_402_300_799_999;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("invalid {0} identifier")]
    InvalidIdentifier(&'static str),
    #[error("timestamp is outside the supported range")]
    InvalidTimestamp,
    #[error("duration must be positive and within the supported range")]
    InvalidDuration,
    #[error("size is outside the JSON/D1-safe integer range")]
    InvalidSize,
    #[error("page size must be between 1 and 100")]
    InvalidPageSize,
    #[error("page cursor is invalid")]
    InvalidPageCursor,
    #[error("idempotency key is invalid")]
    InvalidIdempotencyKey,
    #[error("SHA-256 checksum must contain exactly 64 hexadecimal characters")]
    InvalidChecksum,
    #[error("content type is invalid")]
    InvalidContentType,
    #[error("object version must be non-zero")]
    InvalidObjectVersion,
    #[error("object file name is invalid")]
    InvalidObjectFileName,
    #[error("worker identifier is invalid")]
    InvalidWorkerId,
    #[error("progress must be between 0 and 10000 basis points")]
    InvalidProgress,
    #[error("checkpoint token is invalid")]
    InvalidCheckpoint,
    #[error("table name is invalid")]
    InvalidTableName,
    #[error("retention days must be between 0 and 36500")]
    InvalidRetention,
    #[error("object key does not belong to the declared tenant")]
    TenantObjectMismatch,
    #[error("media source and output keys must be different")]
    SourceEqualsOutput,
    #[error("media profile version must be non-zero")]
    InvalidProfileVersion,
    #[error("unknown executors cannot be dispatched")]
    UnknownExecutor,
}

macro_rules! typed_uuid {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn parse(value: &str) -> Result<Self, ContractError> {
                let value =
                    Uuid::parse_str(value).map_err(|_| ContractError::InvalidIdentifier($kind))?;
                if value.is_nil() {
                    return Err(ContractError::InvalidIdentifier($kind));
                }
                Ok(Self(value))
            }

            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = ContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

typed_uuid!(TenantId, "tenant");
typed_uuid!(UserId, "user");
typed_uuid!(OrganizationId, "organization");
typed_uuid!(SpaceId, "space");
typed_uuid!(FolderId, "folder");
typed_uuid!(UploadId, "upload");
typed_uuid!(MediaJobId, "media job");
typed_uuid!(SessionId, "session");
typed_uuid!(ApiKeyId, "API key");
typed_uuid!(CommentId, "comment");
typed_uuid!(MultipartUploadId, "multipart upload");
typed_uuid!(EtlRunId, "ETL run");
typed_uuid!(CorrelationId, "correlation");

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TimestampMillis(i64);

impl TimestampMillis {
    pub fn new(value: i64) -> Result<Self, ContractError> {
        if !(0..=MAX_TIMESTAMP_MS).contains(&value) {
            return Err(ContractError::InvalidTimestamp);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    pub fn checked_add(self, duration: DurationMillis) -> Result<Self, ContractError> {
        let value = self
            .0
            .checked_add(i64::try_from(duration.0).map_err(|_| ContractError::InvalidTimestamp)?)
            .ok_or(ContractError::InvalidTimestamp)?;
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DurationMillis(u64);

impl DurationMillis {
    pub fn new(value: u64) -> Result<Self, ContractError> {
        if value == 0 || value > MAX_WIRE_INTEGER {
            return Err(ContractError::InvalidDuration);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ByteSize(u64);

impl ByteSize {
    pub fn new(value: u64) -> Result<Self, ContractError> {
        if value > MAX_WIRE_INTEGER {
            return Err(ContractError::InvalidSize);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, ContractError> {
        Self::new(
            self.0
                .checked_add(other.0)
                .ok_or(ContractError::InvalidSize)?,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageSize(u16);

impl PageSize {
    pub fn new(value: u16) -> Result<Self, ContractError> {
        if !(1..=100).contains(&value) {
            return Err(ContractError::InvalidPageSize);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageCursor(String);

impl PageCursor {
    pub fn parse(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 512
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ContractError::InvalidPageCursor);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PageCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PageCursor([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    pub cursor: Option<PageCursor>,
    pub limit: PageSize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<PageCursor>,
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if !(8..=128).contains(&value.len())
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(ContractError::InvalidIdempotencyKey);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for IdempotencyKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("IdempotencyKey([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretDigest(String);

impl SecretDigest {
    pub fn parse_sha256(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ContractError::InvalidChecksum);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn expose_for_verification(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretDigest([redacted])")
    }
}

impl fmt::Display for SecretDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicErrorCode {
    Validation,
    NotFound,
    Conflict,
    Unauthenticated,
    Forbidden,
    PreconditionFailed,
    RateLimited,
    Unavailable,
    UnsupportedVersion,
    Internal,
}

impl PublicErrorCode {
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Unavailable)
    }

    #[must_use]
    pub const fn safe_message(self) -> &'static str {
        match self {
            Self::Validation => "The request is invalid.",
            Self::NotFound => "The resource was not found.",
            Self::Conflict => "The request conflicts with current state.",
            Self::Unauthenticated => "Authentication is required.",
            Self::Forbidden => "The operation is not permitted.",
            Self::PreconditionFailed => "A request precondition was not satisfied.",
            Self::RateLimited => "The request was rate limited.",
            Self::Unavailable => "The service is temporarily unavailable.",
            Self::UnsupportedVersion => "The requested contract version is unsupported.",
            Self::Internal => "An internal error occurred.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorEnvelope {
    pub schema_version: u16,
    pub code: PublicErrorCode,
    pub message: String,
    pub retryable: bool,
    pub correlation_id: CorrelationId,
}

impl ApiErrorEnvelope {
    #[must_use]
    pub fn new(code: PublicErrorCode, correlation_id: CorrelationId) -> Self {
        Self {
            schema_version: 1,
            code,
            message: code.safe_message().to_owned(),
            retryable: code.retryable(),
            correlation_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectVersion(u32);

impl ObjectVersion {
    pub fn new(value: u32) -> Result<Self, ContractError> {
        if value == 0 {
            return Err(ContractError::InvalidObjectVersion);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectRole {
    Source,
    RecordingSegment,
    Thumbnail,
    Screenshot,
    Preview,
    Spritesheet,
    Audio,
    Caption,
    Export,
    Manifest,
}

impl ObjectRole {
    #[must_use]
    pub const fn path_segment(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::RecordingSegment => "segment",
            Self::Thumbnail => "thumbnail",
            Self::Screenshot => "screenshot",
            Self::Preview => "preview",
            Self::Spritesheet => "spritesheet",
            Self::Audio => "audio",
            Self::Caption => "caption",
            Self::Export => "export",
            Self::Manifest => "manifest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RetentionDays(u16);

impl RetentionDays {
    pub fn new(value: u16) -> Result<Self, ContractError> {
        if value > 36_500 {
            return Err(ContractError::InvalidRetention);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    fn as_millis(self) -> u64 {
        u64::from(self.0) * 24 * 60 * 60 * 1_000
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRetentionPolicy {
    pub source: Option<RetentionDays>,
    pub derivatives: Option<RetentionDays>,
    pub deleted_grace: RetentionDays,
}

impl ObjectRetentionPolicy {
    #[must_use]
    pub const fn retention_for(self, role: ObjectRole) -> Option<RetentionDays> {
        match role {
            ObjectRole::Source | ObjectRole::RecordingSegment => self.source,
            ObjectRole::Thumbnail
            | ObjectRole::Screenshot
            | ObjectRole::Preview
            | ObjectRole::Spritesheet
            | ObjectRole::Audio
            | ObjectRole::Caption
            | ObjectRole::Export
            | ObjectRole::Manifest => self.derivatives,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectDeletionDecision {
    NotDeleted,
    BlockedByLegalHold,
    RetainIndefinitely,
    RetainUntil(TimestampMillis),
    Eligible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectRetentionContext {
    pub role: ObjectRole,
    pub created_at: TimestampMillis,
    pub deleted_at: Option<TimestampMillis>,
    pub legal_hold_active: bool,
}

impl ObjectRetentionContext {
    pub fn deletion_decision(
        self,
        policy: ObjectRetentionPolicy,
        now: TimestampMillis,
    ) -> Result<ObjectDeletionDecision, ContractError> {
        let Some(deleted_at) = self.deleted_at else {
            return Ok(ObjectDeletionDecision::NotDeleted);
        };
        if self.legal_hold_active {
            return Ok(ObjectDeletionDecision::BlockedByLegalHold);
        }
        let Some(retention) = policy.retention_for(self.role) else {
            return Ok(ObjectDeletionDecision::RetainIndefinitely);
        };
        let add_days = |timestamp: TimestampMillis,
                        days: RetentionDays|
         -> Result<TimestampMillis, ContractError> {
            if days.get() == 0 {
                Ok(timestamp)
            } else {
                timestamp.checked_add(DurationMillis::new(days.as_millis())?)
            }
        };
        let retention_end = add_days(self.created_at, retention)?;
        let grace_end = add_days(deleted_at, policy.deleted_grace)?;
        let eligible_at = retention_end.max(grace_end);
        if now >= eligible_at {
            Ok(ObjectDeletionDecision::Eligible)
        } else {
            Ok(ObjectDeletionDecision::RetainUntil(eligible_at))
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChecksumSha256(String);

impl ChecksumSha256 {
    pub fn parse(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ContractError::InvalidChecksum);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ChecksumSha256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ChecksumSha256")
            .field(&self.0)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentType(String);

impl ContentType {
    pub fn parse(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        let mut parts = value.split('/');
        if value.len() > 127
            || parts.next().is_none_or(str::is_empty)
            || parts.next().is_none_or(str::is_empty)
            || parts.next().is_some()
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'+' | b'-' | b'.')
            })
        {
            return Err(ContractError::InvalidContentType);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ObjectKey {
    pub fn for_video(
        tenant_id: TenantId,
        video_id: VideoId,
        role: ObjectRole,
        version: ObjectVersion,
        file_name: &str,
    ) -> Result<Self, ContractError> {
        if file_name.is_empty()
            || file_name.len() > 255
            || file_name == "."
            || file_name == ".."
            || !file_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ContractError::InvalidObjectFileName);
        }
        Self::parse(format!(
            "tenants/{tenant_id}/videos/{video_id}/{}/v{}/{file_name}",
            role.path_segment(),
            version.get()
        ))
        .map_err(|_| ContractError::InvalidObjectFileName)
    }

    #[must_use]
    pub fn belongs_to_tenant(&self, tenant_id: TenantId) -> bool {
        let prefix = format!("tenants/{tenant_id}/");
        self.as_str().starts_with(&prefix)
    }
}

impl VideoId {
    pub fn parse_strict(value: &str) -> Result<Self, ContractError> {
        let parsed =
            Uuid::parse_str(value).map_err(|_| ContractError::InvalidIdentifier("video"))?;
        if parsed.is_nil() {
            return Err(ContractError::InvalidIdentifier("video"));
        }
        Ok(Self(parsed))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MediaProfileVersion(u16);

impl MediaProfileVersion {
    pub fn new(value: u16) -> Result<Self, ContractError> {
        if value == 0 {
            return Err(ContractError::InvalidProfileVersion);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaExecutorKind {
    CloudflareMedia,
    NativeGstreamer,
    Unknown,
}

impl MediaExecutorKind {
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "cloudflare_media" => Self::CloudflareMedia,
            "native_gstreamer" => Self::NativeGstreamer,
            _ => Self::Unknown,
        }
    }

    #[must_use]
    pub const fn dispatchable(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDecision {
    Supported,
    LimitExceeded,
    UnsupportedFormat,
    ProviderUnavailable,
    PolicyNativeOnly,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaJobSpec {
    pub schema_version: u16,
    pub tenant_id: TenantId,
    pub video_id: VideoId,
    pub source: ObjectKey,
    pub source_version: ObjectVersion,
    pub source_checksum: Option<ChecksumSha256>,
    pub output: ObjectKey,
    pub profile_version: MediaProfileVersion,
    pub selected_executor: MediaExecutorKind,
    pub capability_decision: CapabilityDecision,
    pub idempotency_key: IdempotencyKey,
}

impl MediaJobSpec {
    pub fn validate_for_dispatch(&self) -> Result<(), ContractError> {
        if !self.source.belongs_to_tenant(self.tenant_id)
            || !self.output.belongs_to_tenant(self.tenant_id)
        {
            return Err(ContractError::TenantObjectMismatch);
        }
        if self.source == self.output {
            return Err(ContractError::SourceEqualsOutput);
        }
        if !self.selected_executor.dispatchable()
            || self.capability_decision == CapabilityDecision::Unknown
        {
            return Err(ContractError::UnknownExecutor);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadState {
    Initiated,
    Uploading,
    Finalizing,
    Complete,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadContract {
    pub id: UploadId,
    pub tenant_id: TenantId,
    pub video_id: VideoId,
    pub state: UploadState,
    pub expected_size: ByteSize,
    pub received_size: ByteSize,
    pub revision: u64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("the requested lifecycle transition is invalid")]
    InvalidTransition,
    #[error("progress cannot move backwards or exceed the declared size")]
    InvalidProgress,
    #[error("the declared upload size has not been received")]
    SizeMismatch,
    #[error("a non-expired lease is already active")]
    LeaseActive,
    #[error("the lease is expired")]
    LeaseExpired,
    #[error("the lease token does not match")]
    LeaseMismatch,
    #[error("the job is terminal")]
    Terminal,
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("the executor cannot report progress")]
    ProgressUnsupported,
    #[error("the lifecycle epoch is exhausted")]
    EpochExhausted,
}

impl UploadContract {
    #[must_use]
    pub fn new(tenant_id: TenantId, video_id: VideoId, expected_size: ByteSize) -> Self {
        Self {
            id: UploadId::new(),
            tenant_id,
            video_id,
            state: UploadState::Initiated,
            expected_size,
            received_size: ByteSize(0),
            revision: 0,
        }
    }

    pub fn begin(&mut self) -> Result<(), LifecycleError> {
        match self.state {
            UploadState::Initiated => {
                self.state = UploadState::Uploading;
                self.revision += 1;
                Ok(())
            }
            UploadState::Uploading => Ok(()),
            _ => Err(LifecycleError::InvalidTransition),
        }
    }

    pub fn record_chunk(&mut self, offset: ByteSize, size: ByteSize) -> Result<(), LifecycleError> {
        if self.state != UploadState::Uploading || offset != self.received_size {
            return Err(LifecycleError::InvalidProgress);
        }
        let received = self
            .received_size
            .checked_add(size)
            .map_err(|_| LifecycleError::InvalidProgress)?;
        if received > self.expected_size {
            return Err(LifecycleError::InvalidProgress);
        }
        self.received_size = received;
        self.revision += 1;
        Ok(())
    }

    pub fn begin_finalizing(&mut self) -> Result<(), LifecycleError> {
        if self.state != UploadState::Uploading {
            return Err(LifecycleError::InvalidTransition);
        }
        if self.received_size != self.expected_size {
            return Err(LifecycleError::SizeMismatch);
        }
        self.state = UploadState::Finalizing;
        self.revision += 1;
        Ok(())
    }

    pub fn complete(&mut self) -> Result<(), LifecycleError> {
        if self.state != UploadState::Finalizing {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = UploadState::Complete;
        self.revision += 1;
        Ok(())
    }

    pub fn fail(&mut self) -> Result<(), LifecycleError> {
        match self.state {
            UploadState::Initiated | UploadState::Uploading | UploadState::Finalizing => {
                self.state = UploadState::Failed;
                self.revision += 1;
                Ok(())
            }
            UploadState::Failed => Ok(()),
            UploadState::Complete | UploadState::Aborted => Err(LifecycleError::Terminal),
        }
    }

    pub fn abort(&mut self) -> Result<(), LifecycleError> {
        match self.state {
            UploadState::Complete => Err(LifecycleError::Terminal),
            UploadState::Aborted => Ok(()),
            _ => {
                self.state = UploadState::Aborted;
                self.revision += 1;
                Ok(())
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LeaseToken(Uuid);

impl LeaseToken {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for LeaseToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LeaseToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LeaseToken([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkerId(String);

impl WorkerId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(ContractError::InvalidWorkerId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobLease {
    pub token: LeaseToken,
    pub worker_id: WorkerId,
    pub expires_at: TimestampMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaJobState {
    Queued,
    Leased,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    DeadLetter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressCapability {
    None,
    Milestones,
    Continuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationCapability {
    BeforeStart,
    Cooperative,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobProgress(u16);

impl JobProgress {
    pub fn new(value: u16) -> Result<Self, ContractError> {
        if value > 10_000 {
            return Err(ContractError::InvalidProgress);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn basis_points(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    Cancelled,
    PendingExecutorCompletion,
    AlreadyCancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionOutcome {
    Succeeded,
    DiscardedAfterCancellation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaJobContract {
    pub id: MediaJobId,
    pub tenant_id: TenantId,
    pub video_id: VideoId,
    pub state: MediaJobState,
    pub attempt: u32,
    pub lease: Option<JobLease>,
    pub progress: Option<JobProgress>,
    pub progress_capability: ProgressCapability,
    pub cancellation_capability: CancellationCapability,
    pub cancel_requested: bool,
    pub revision: u64,
}

impl MediaJobContract {
    #[must_use]
    pub fn new(
        tenant_id: TenantId,
        video_id: VideoId,
        progress_capability: ProgressCapability,
        cancellation_capability: CancellationCapability,
    ) -> Self {
        Self {
            id: MediaJobId::new(),
            tenant_id,
            video_id,
            state: MediaJobState::Queued,
            attempt: 0,
            lease: None,
            progress: None,
            progress_capability,
            cancellation_capability,
            cancel_requested: false,
            revision: 0,
        }
    }

    pub fn claim(
        &mut self,
        worker_id: WorkerId,
        now: TimestampMillis,
        lease_for: DurationMillis,
    ) -> Result<LeaseToken, LifecycleError> {
        if self.state.is_terminal() {
            return Err(LifecycleError::Terminal);
        }
        if matches!(self.state, MediaJobState::Leased | MediaJobState::Running)
            && self
                .lease
                .as_ref()
                .is_some_and(|lease| lease.expires_at > now)
        {
            return Err(LifecycleError::LeaseActive);
        }
        if self.state != MediaJobState::Queued
            && !matches!(self.state, MediaJobState::Leased | MediaJobState::Running)
        {
            return Err(LifecycleError::InvalidTransition);
        }
        let token = LeaseToken::new();
        self.lease = Some(JobLease {
            token,
            worker_id,
            expires_at: now
                .checked_add(lease_for)
                .map_err(|_| LifecycleError::InvalidTransition)?,
        });
        self.state = MediaJobState::Leased;
        self.attempt = self.attempt.saturating_add(1);
        self.progress = None;
        self.revision += 1;
        Ok(token)
    }

    pub fn heartbeat(
        &mut self,
        token: LeaseToken,
        now: TimestampMillis,
        lease_for: DurationMillis,
    ) -> Result<(), LifecycleError> {
        self.verify_lease(token, now)?;
        let lease = self.lease.as_mut().ok_or(LifecycleError::LeaseMismatch)?;
        lease.expires_at = now
            .checked_add(lease_for)
            .map_err(|_| LifecycleError::InvalidTransition)?;
        self.revision += 1;
        Ok(())
    }

    pub fn start(&mut self, token: LeaseToken, now: TimestampMillis) -> Result<(), LifecycleError> {
        self.verify_lease(token, now)?;
        if self.state != MediaJobState::Leased {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = MediaJobState::Running;
        self.revision += 1;
        Ok(())
    }

    pub fn report_progress(
        &mut self,
        token: LeaseToken,
        now: TimestampMillis,
        progress: JobProgress,
    ) -> Result<(), LifecycleError> {
        self.verify_lease(token, now)?;
        if self.state != MediaJobState::Running {
            return Err(LifecycleError::InvalidTransition);
        }
        if self.progress_capability == ProgressCapability::None {
            return Err(LifecycleError::ProgressUnsupported);
        }
        if self.progress.is_some_and(|current| progress < current) {
            return Err(LifecycleError::InvalidProgress);
        }
        self.progress = Some(progress);
        self.revision += 1;
        Ok(())
    }

    pub fn request_cancel(&mut self) -> Result<CancelOutcome, LifecycleError> {
        match self.state {
            MediaJobState::Cancelled => Ok(CancelOutcome::AlreadyCancelled),
            MediaJobState::Succeeded | MediaJobState::Failed | MediaJobState::DeadLetter => {
                Err(LifecycleError::Terminal)
            }
            MediaJobState::Running
                if self.cancellation_capability == CancellationCapability::None =>
            {
                self.cancel_requested = true;
                self.revision += 1;
                Ok(CancelOutcome::PendingExecutorCompletion)
            }
            _ => {
                self.cancel_requested = true;
                self.state = MediaJobState::Cancelled;
                self.lease = None;
                self.revision += 1;
                Ok(CancelOutcome::Cancelled)
            }
        }
    }

    pub fn complete_success(
        &mut self,
        token: LeaseToken,
        now: TimestampMillis,
    ) -> Result<CompletionOutcome, LifecycleError> {
        self.verify_lease(token, now)?;
        if self.state != MediaJobState::Running {
            return Err(LifecycleError::InvalidTransition);
        }
        self.lease = None;
        self.revision += 1;
        if self.cancel_requested {
            self.state = MediaJobState::Cancelled;
            Ok(CompletionOutcome::DiscardedAfterCancellation)
        } else {
            self.state = MediaJobState::Succeeded;
            self.progress = Some(JobProgress(10_000));
            Ok(CompletionOutcome::Succeeded)
        }
    }

    pub fn complete_failure(
        &mut self,
        token: LeaseToken,
        now: TimestampMillis,
        retryable: bool,
        max_attempts: u32,
    ) -> Result<MediaJobState, LifecycleError> {
        self.verify_lease(token, now)?;
        if !matches!(self.state, MediaJobState::Leased | MediaJobState::Running) {
            return Err(LifecycleError::InvalidTransition);
        }
        self.lease = None;
        self.revision += 1;
        self.state = if self.cancel_requested {
            MediaJobState::Cancelled
        } else if retryable && self.attempt < max_attempts {
            MediaJobState::Queued
        } else if retryable {
            MediaJobState::DeadLetter
        } else {
            MediaJobState::Failed
        };
        Ok(self.state)
    }

    fn verify_lease(&self, token: LeaseToken, now: TimestampMillis) -> Result<(), LifecycleError> {
        let lease = self.lease.as_ref().ok_or(LifecycleError::LeaseMismatch)?;
        if lease.token != token {
            return Err(LifecycleError::LeaseMismatch);
        }
        if lease.expires_at <= now {
            return Err(LifecycleError::LeaseExpired);
        }
        Ok(())
    }
}

impl MediaJobState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::DeadLetter
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationDecision {
    Authenticated,
    Expired,
    Revoked,
    SessionVersionMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: SessionId,
    pub user_id: UserId,
    pub token_digest: SecretDigest,
    pub issued_at: TimestampMillis,
    pub expires_at: TimestampMillis,
    pub session_version: u64,
    pub state: SessionState,
}

impl SessionRecord {
    pub fn new(
        user_id: UserId,
        token_digest: SecretDigest,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
        session_version: u64,
    ) -> Result<Self, LifecycleError> {
        if expires_at <= issued_at {
            return Err(LifecycleError::InvalidTransition);
        }
        Ok(Self {
            id: SessionId::new(),
            user_id,
            token_digest,
            issued_at,
            expires_at,
            session_version,
            state: SessionState::Active,
        })
    }

    #[must_use]
    pub fn authenticate(
        &self,
        now: TimestampMillis,
        current_session_version: u64,
    ) -> AuthenticationDecision {
        if self.state == SessionState::Revoked {
            AuthenticationDecision::Revoked
        } else if self.expires_at <= now {
            AuthenticationDecision::Expired
        } else if self.session_version != current_session_version {
            AuthenticationDecision::SessionVersionMismatch
        } else {
            AuthenticationDecision::Authenticated
        }
    }

    pub fn revoke(&mut self) {
        self.state = SessionState::Revoked;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyScope {
    VideosRead,
    VideosWrite,
    UploadsWrite,
    OrganizationsRead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyRecord {
    pub id: ApiKeyId,
    pub owner_id: UserId,
    pub key_digest: SecretDigest,
    pub scopes: Vec<ApiKeyScope>,
    pub expires_at: Option<TimestampMillis>,
    pub revoked_at: Option<TimestampMillis>,
}

impl ApiKeyRecord {
    #[must_use]
    pub fn allows(&self, scope: ApiKeyScope, now: TimestampMillis) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|expires| expires > now)
            && self.scopes.contains(&scope)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationRole {
    Owner,
    Admin,
    Member,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipState {
    Active,
    Suspended,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceRole {
    Manager,
    Contributor,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    OrganizationRead,
    OrganizationUpdate,
    TransferOwnership,
    DeleteOrganization,
    ManageMembers,
    ManageBilling,
    ManageStorage,
    SpaceManage,
    VideoRead,
    VideoCreate,
    VideoUpdate,
    VideoDelete,
    CommentCreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenialReason {
    CrossTenant,
    NoMembership,
    InactiveMembership,
    InsufficientRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationDecision {
    Allow,
    Deny(DenialReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub actor_id: UserId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationMembership {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub role: OrganizationRole,
    pub state: MembershipState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourcePolicyContext {
    pub tenant_id: TenantId,
    pub owner_id: Option<UserId>,
    pub space_role: Option<SpaceRole>,
}

pub struct AuthorizationPolicy;

impl AuthorizationPolicy {
    #[must_use]
    pub fn evaluate(
        actor: TenantContext,
        membership: Option<OrganizationMembership>,
        resource: ResourcePolicyContext,
        action: PolicyAction,
    ) -> AuthorizationDecision {
        if actor.tenant_id != resource.tenant_id {
            return AuthorizationDecision::Deny(DenialReason::CrossTenant);
        }
        let Some(membership) = membership else {
            return AuthorizationDecision::Deny(DenialReason::NoMembership);
        };
        if membership.tenant_id != actor.tenant_id || membership.user_id != actor.actor_id {
            return AuthorizationDecision::Deny(DenialReason::CrossTenant);
        }
        if membership.state != MembershipState::Active {
            return AuthorizationDecision::Deny(DenialReason::InactiveMembership);
        }

        let allowed = match membership.role {
            OrganizationRole::Owner => true,
            OrganizationRole::Admin => !matches!(
                action,
                PolicyAction::TransferOwnership | PolicyAction::DeleteOrganization
            ),
            OrganizationRole::Member => match action {
                PolicyAction::OrganizationRead
                | PolicyAction::VideoRead
                | PolicyAction::VideoCreate
                | PolicyAction::CommentCreate => true,
                PolicyAction::VideoUpdate | PolicyAction::VideoDelete => {
                    resource.owner_id == Some(actor.actor_id)
                        || matches!(resource.space_role, Some(SpaceRole::Manager))
                }
                PolicyAction::SpaceManage => {
                    matches!(resource.space_role, Some(SpaceRole::Manager))
                }
                _ => false,
            },
            OrganizationRole::Viewer => matches!(
                action,
                PolicyAction::OrganizationRead | PolicyAction::VideoRead
            ),
        };
        if allowed {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny(DenialReason::InsufficientRole)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EtlRunState {
    Planned,
    Running,
    Reconciling,
    Matched,
    Mismatched,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtlTableManifest {
    pub table: String,
    pub expected_rows: u64,
    pub expected_checksum: ChecksumSha256,
}

impl EtlTableManifest {
    pub fn new(
        table: impl Into<String>,
        expected_rows: u64,
        expected_checksum: ChecksumSha256,
    ) -> Result<Self, ContractError> {
        let table = table.into();
        if table.is_empty()
            || table.len() > 64
            || !table
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ContractError::InvalidTableName);
        }
        if expected_rows > MAX_WIRE_INTEGER {
            return Err(ContractError::InvalidSize);
        }
        Ok(Self {
            table,
            expected_rows,
            expected_checksum,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtlManifest {
    pub run_id: EtlRunId,
    pub source_revision: String,
    pub state: EtlRunState,
    pub tables: BTreeMap<String, EtlTableManifest>,
}

impl EtlManifest {
    pub fn new(source_revision: impl Into<String>) -> Result<Self, ContractError> {
        let source_revision = source_revision.into();
        if source_revision.is_empty()
            || source_revision.len() > 128
            || !source_revision
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ContractError::InvalidCheckpoint);
        }
        Ok(Self {
            run_id: EtlRunId::new(),
            source_revision,
            state: EtlRunState::Planned,
            tables: BTreeMap::new(),
        })
    }

    pub fn add_table(&mut self, table: EtlTableManifest) -> Result<(), LifecycleError> {
        if self.state != EtlRunState::Planned || self.tables.contains_key(&table.table) {
            return Err(LifecycleError::InvalidTransition);
        }
        self.tables.insert(table.table.clone(), table);
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), LifecycleError> {
        if self.state != EtlRunState::Planned || self.tables.is_empty() {
            return Err(LifecycleError::InvalidTransition);
        }
        self.state = EtlRunState::Running;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckpointToken(String);

impl CheckpointToken {
    pub fn parse(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 512
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(ContractError::InvalidCheckpoint);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_resume(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CheckpointToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CheckpointToken([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EtlCheckpoint {
    pub run_id: EtlRunId,
    pub table: String,
    pub token: CheckpointToken,
    pub processed_rows: u64,
}

impl EtlCheckpoint {
    pub fn new(
        run_id: EtlRunId,
        table: impl Into<String>,
        token: CheckpointToken,
        processed_rows: u64,
    ) -> Result<Self, ContractError> {
        let table = table.into();
        if table.is_empty()
            || table.len() > 64
            || !table
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ContractError::InvalidTableName);
        }
        if processed_rows > MAX_WIRE_INTEGER {
            return Err(ContractError::InvalidSize);
        }
        Ok(Self {
            run_id,
            table,
            token,
            processed_rows,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationSummary {
    pub expected_rows: u64,
    pub actual_rows: u64,
    pub expected_bytes: u64,
    pub actual_bytes: u64,
    pub semantic_mismatches: u64,
}

impl ReconciliationSummary {
    pub fn new(
        expected_rows: u64,
        actual_rows: u64,
        expected_bytes: u64,
        actual_bytes: u64,
        semantic_mismatches: u64,
    ) -> Result<Self, ContractError> {
        if [
            expected_rows,
            actual_rows,
            expected_bytes,
            actual_bytes,
            semantic_mismatches,
        ]
        .into_iter()
        .any(|value| value > MAX_WIRE_INTEGER)
        {
            return Err(ContractError::InvalidSize);
        }
        Ok(Self {
            expected_rows,
            actual_rows,
            expected_bytes,
            actual_bytes,
            semantic_mismatches,
        })
    }

    #[must_use]
    pub const fn is_clean(self) -> bool {
        self.expected_rows == self.actual_rows
            && self.expected_bytes == self.actual_bytes
            && self.semantic_mismatches == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataAuthority {
    Legacy,
    D1,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CutoverDomain(String);

impl CutoverDomain {
    pub fn parse(value: &str) -> Result<Self, ContractError> {
        if value.is_empty()
            || value.len() > 64
            || !value.as_bytes()[0].is_ascii_lowercase()
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            })
        {
            return Err(ContractError::InvalidIdentifier("cutover domain"));
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CutoverDomain {
    type Error = ContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<CutoverDomain> for String {
    fn from(value: CutoverDomain) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CutoverScope {
    pub tenant_id: TenantId,
    pub domain: CutoverDomain,
}

impl CutoverScope {
    #[must_use]
    pub const fn new(tenant_id: TenantId, domain: CutoverDomain) -> Self {
        Self { tenant_id, domain }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutoverPhase {
    LegacyAuthoritative,
    ShadowRead,
    DualWrite,
    D1Authoritative,
    RolledBack,
    Finalized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CutoverEvidence {
    pub shadow_observation_ready: bool,
    pub reconciliation_clean: bool,
    pub rollback_rehearsed: bool,
    pub observation_window_complete: bool,
    pub reconciliation_digest_present: bool,
    pub legacy_fenced: bool,
    pub d1_fenced: bool,
    pub legacy_caught_up: bool,
    pub pending_events: u64,
    pub dead_letter_events: u64,
    pub shadow_mismatches: u64,
}

impl CutoverEvidence {
    #[must_use]
    const fn counts_are_safe(self) -> bool {
        self.pending_events <= MAX_WIRE_INTEGER
            && self.dead_letter_events <= MAX_WIRE_INTEGER
            && self.shadow_mismatches <= MAX_WIRE_INTEGER
    }

    #[must_use]
    const fn shadow_is_clean(self) -> bool {
        self.observation_window_complete && self.shadow_mismatches == 0
    }

    #[must_use]
    const fn replay_is_drained(self) -> bool {
        self.pending_events == 0 && self.dead_letter_events == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CutoverState {
    pub scope: CutoverScope,
    pub phase: CutoverPhase,
    pub writer: DataAuthority,
    pub mirror_enabled: bool,
    pub replay_paused: bool,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityFence {
    pub scope: CutoverScope,
    pub writer: DataAuthority,
    pub epoch: u64,
}

impl CutoverState {
    #[must_use]
    pub const fn new(scope: CutoverScope) -> Self {
        Self {
            scope,
            phase: CutoverPhase::LegacyAuthoritative,
            writer: DataAuthority::Legacy,
            mirror_enabled: false,
            replay_paused: false,
            epoch: 0,
        }
    }

    pub fn transition(
        &mut self,
        next: CutoverPhase,
        evidence: CutoverEvidence,
    ) -> Result<(), LifecycleError> {
        if !evidence.counts_are_safe() {
            return Err(LifecycleError::InvalidTransition);
        }
        let writer = match (self.phase, next) {
            (CutoverPhase::LegacyAuthoritative, CutoverPhase::ShadowRead)
                if evidence.shadow_observation_ready =>
            {
                DataAuthority::Legacy
            }
            (CutoverPhase::ShadowRead | CutoverPhase::RolledBack, CutoverPhase::DualWrite)
                if evidence.reconciliation_clean && evidence.shadow_is_clean() =>
            {
                DataAuthority::Legacy
            }
            (CutoverPhase::DualWrite, CutoverPhase::D1Authoritative)
                if evidence.reconciliation_clean
                    && evidence.rollback_rehearsed
                    && evidence.reconciliation_digest_present
                    && evidence.legacy_fenced
                    && evidence.shadow_is_clean()
                    && evidence.replay_is_drained() =>
            {
                DataAuthority::D1
            }
            (CutoverPhase::D1Authoritative, CutoverPhase::Finalized)
                if evidence.reconciliation_clean
                    && evidence.reconciliation_digest_present
                    && evidence.shadow_is_clean()
                    && evidence.replay_is_drained() =>
            {
                DataAuthority::D1
            }
            (CutoverPhase::D1Authoritative, CutoverPhase::RolledBack)
                if evidence.d1_fenced
                    && evidence.legacy_caught_up
                    && evidence.rollback_rehearsed
                    && evidence.reconciliation_clean
                    && evidence.reconciliation_digest_present
                    && evidence.replay_is_drained() =>
            {
                DataAuthority::Legacy
            }
            _ => return Err(LifecycleError::InvalidTransition),
        };
        let epoch = self
            .epoch
            .checked_add(1)
            .filter(|epoch| *epoch <= MAX_WIRE_INTEGER)
            .ok_or(LifecycleError::EpochExhausted)?;
        self.phase = next;
        self.writer = writer;
        self.mirror_enabled = matches!(
            next,
            CutoverPhase::DualWrite | CutoverPhase::D1Authoritative | CutoverPhase::RolledBack
        );
        self.replay_paused = false;
        self.epoch = epoch;
        Ok(())
    }

    #[must_use]
    pub const fn invariants_hold(&self) -> bool {
        let writer_matches_phase = match self.phase {
            CutoverPhase::LegacyAuthoritative
            | CutoverPhase::ShadowRead
            | CutoverPhase::DualWrite
            | CutoverPhase::RolledBack => matches!(self.writer, DataAuthority::Legacy),
            CutoverPhase::D1Authoritative | CutoverPhase::Finalized => {
                matches!(self.writer, DataAuthority::D1)
            }
        };
        writer_matches_phase
            && self.mirror_enabled
                == matches!(
                    self.phase,
                    CutoverPhase::DualWrite
                        | CutoverPhase::D1Authoritative
                        | CutoverPhase::RolledBack
                )
            && self.epoch <= MAX_WIRE_INTEGER
    }

    pub fn set_replay_paused(&mut self, paused: bool) -> Result<(), LifecycleError> {
        if self.replay_paused == paused
            || !matches!(
                self.phase,
                CutoverPhase::ShadowRead
                    | CutoverPhase::DualWrite
                    | CutoverPhase::D1Authoritative
                    | CutoverPhase::RolledBack
            )
        {
            return Err(LifecycleError::InvalidTransition);
        }
        self.epoch = self
            .epoch
            .checked_add(1)
            .filter(|epoch| *epoch <= MAX_WIRE_INTEGER)
            .ok_or(LifecycleError::EpochExhausted)?;
        self.replay_paused = paused;
        Ok(())
    }

    pub fn authorize_writer(
        &self,
        writer: DataAuthority,
        expected_epoch: u64,
    ) -> Result<AuthorityFence, LifecycleError> {
        if self.writer != writer || self.epoch != expected_epoch {
            return Err(LifecycleError::InvalidTransition);
        }
        Ok(AuthorityFence {
            scope: self.scope.clone(),
            writer,
            epoch: expected_epoch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("valid timestamp")
    }

    fn duration(value: u64) -> DurationMillis {
        DurationMillis::new(value).expect("valid duration")
    }

    #[test]
    fn primitives_reject_ambiguous_or_unsafe_values() {
        assert!(TenantId::parse("00000000-0000-0000-0000-000000000000").is_err());
        assert!(TimestampMillis::new(-1).is_err());
        assert!(ByteSize::new(MAX_WIRE_INTEGER + 1).is_err());
        assert!(PageSize::new(0).is_err());
        assert!(PageSize::new(101).is_err());
        assert!(PageCursor::parse("contains whitespace").is_err());
        assert!(IdempotencyKey::parse("short").is_err());
        assert!(ContentType::parse("video/mp4").is_ok());
        assert!(ContentType::parse("video/mp4; secret=x").is_err());
    }

    #[test]
    fn secret_bearing_values_are_redacted_in_debug_and_display() {
        let digest = SecretDigest::parse_sha256("a".repeat(64)).expect("digest");
        assert_eq!(format!("{digest:?}"), "SecretDigest([redacted])");
        assert_eq!(digest.to_string(), "[redacted]");
        let key = IdempotencyKey::parse("customer-command-123").expect("key");
        assert_eq!(format!("{key:?}"), "IdempotencyKey([redacted])");
    }

    #[test]
    fn error_envelope_uses_only_stable_safe_messages() {
        let envelope = ApiErrorEnvelope::new(PublicErrorCode::Unavailable, CorrelationId::new());
        assert_eq!(envelope.schema_version, 1);
        assert_eq!(envelope.message, "The service is temporarily unavailable.");
        assert!(envelope.retryable);
    }

    #[test]
    fn deterministic_object_keys_are_tenant_version_and_role_scoped() {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let key = ObjectKey::for_video(
            tenant,
            video,
            ObjectRole::Thumbnail,
            ObjectVersion::new(2).expect("version"),
            "poster.jpg",
        )
        .expect("key");
        assert_eq!(
            key.as_str(),
            format!("tenants/{tenant}/videos/{video}/thumbnail/v2/poster.jpg")
        );
        assert!(key.belongs_to_tenant(tenant));
        assert!(!key.belongs_to_tenant(TenantId::new()));
        assert!(
            ObjectKey::for_video(
                tenant,
                video,
                ObjectRole::Source,
                ObjectVersion::new(1).expect("version"),
                "../source.mp4"
            )
            .is_err()
        );
    }

    #[test]
    fn media_job_specs_reject_cross_tenant_and_unknown_executor_dispatch() {
        assert!(VideoId::parse_strict("00000000-0000-0000-0000-000000000000").is_err());
        let tenant = TenantId::new();
        let other_tenant = TenantId::new();
        let video = VideoId::new();
        let version = ObjectVersion::new(1).expect("version");
        let mut spec = MediaJobSpec {
            schema_version: 1,
            tenant_id: tenant,
            video_id: video,
            source: ObjectKey::for_video(tenant, video, ObjectRole::Source, version, "source.mp4")
                .expect("source"),
            source_version: version,
            source_checksum: None,
            output: ObjectKey::for_video(
                tenant,
                video,
                ObjectRole::Thumbnail,
                version,
                "poster.jpg",
            )
            .expect("output"),
            profile_version: MediaProfileVersion::new(1).expect("profile"),
            selected_executor: MediaExecutorKind::CloudflareMedia,
            capability_decision: CapabilityDecision::Supported,
            idempotency_key: IdempotencyKey::parse("media-job-command-1").expect("key"),
        };
        spec.validate_for_dispatch().expect("dispatchable");
        spec.selected_executor = MediaExecutorKind::from_wire("future_provider");
        assert_eq!(
            spec.validate_for_dispatch(),
            Err(ContractError::UnknownExecutor)
        );
        spec.selected_executor = MediaExecutorKind::NativeGstreamer;
        spec.output = ObjectKey::for_video(
            other_tenant,
            video,
            ObjectRole::Export,
            version,
            "export.mp4",
        )
        .expect("output");
        assert_eq!(
            spec.validate_for_dispatch(),
            Err(ContractError::TenantObjectMismatch)
        );
    }

    #[test]
    fn deletion_respects_retention_grace_and_legal_holds() {
        let day_ms = 24 * 60 * 60 * 1_000;
        let policy = ObjectRetentionPolicy {
            source: Some(RetentionDays::new(30).expect("retention")),
            derivatives: Some(RetentionDays::new(7).expect("retention")),
            deleted_grace: RetentionDays::new(2).expect("grace"),
        };
        let context = ObjectRetentionContext {
            role: ObjectRole::Thumbnail,
            created_at: timestamp(0),
            deleted_at: Some(timestamp(6 * day_ms)),
            legal_hold_active: false,
        };
        assert_eq!(
            context
                .deletion_decision(policy, timestamp(7 * day_ms))
                .expect("decision"),
            ObjectDeletionDecision::RetainUntil(timestamp(8 * day_ms))
        );
        assert_eq!(
            context
                .deletion_decision(policy, timestamp(8 * day_ms))
                .expect("decision"),
            ObjectDeletionDecision::Eligible
        );
        assert_eq!(
            ObjectRetentionContext {
                legal_hold_active: true,
                ..context
            }
            .deletion_decision(policy, timestamp(100 * day_ms))
            .expect("decision"),
            ObjectDeletionDecision::BlockedByLegalHold
        );
    }

    #[test]
    fn upload_progress_is_monotonic_and_exact_before_completion() {
        let mut upload = UploadContract::new(
            TenantId::new(),
            VideoId::new(),
            ByteSize::new(10).expect("size"),
        );
        upload.begin().expect("begin");
        upload
            .record_chunk(
                ByteSize::new(0).expect("offset"),
                ByteSize::new(4).expect("size"),
            )
            .expect("chunk");
        assert_eq!(upload.begin_finalizing(), Err(LifecycleError::SizeMismatch));
        assert_eq!(
            upload.record_chunk(
                ByteSize::new(0).expect("offset"),
                ByteSize::new(6).expect("size")
            ),
            Err(LifecycleError::InvalidProgress)
        );
        upload
            .record_chunk(
                ByteSize::new(4).expect("offset"),
                ByteSize::new(6).expect("size"),
            )
            .expect("chunk");
        upload.begin_finalizing().expect("finalize");
        upload.complete().expect("complete");
        assert_eq!(upload.abort(), Err(LifecycleError::Terminal));
    }

    #[test]
    fn job_leases_reclaim_expired_work_and_reject_stale_callbacks() {
        let mut job = MediaJobContract::new(
            TenantId::new(),
            VideoId::new(),
            ProgressCapability::Continuous,
            CancellationCapability::Cooperative,
        );
        let first = job
            .claim(
                WorkerId::parse("worker-a").expect("worker"),
                timestamp(100),
                duration(20),
            )
            .expect("claim");
        assert_eq!(
            job.claim(
                WorkerId::parse("worker-b").expect("worker"),
                timestamp(110),
                duration(20)
            ),
            Err(LifecycleError::LeaseActive)
        );
        let second = job
            .claim(
                WorkerId::parse("worker-b").expect("worker"),
                timestamp(121),
                duration(20),
            )
            .expect("reclaim");
        assert_ne!(first, second);
        assert_eq!(
            job.start(first, timestamp(122)),
            Err(LifecycleError::LeaseMismatch)
        );
        job.start(second, timestamp(122)).expect("start");
        job.report_progress(
            second,
            timestamp(123),
            JobProgress::new(5000).expect("progress"),
        )
        .expect("progress");
        assert_eq!(
            job.report_progress(
                second,
                timestamp(124),
                JobProgress::new(4000).expect("progress")
            ),
            Err(LifecycleError::InvalidProgress)
        );
        assert_eq!(
            job.complete_success(second, timestamp(125)),
            Ok(CompletionOutcome::Succeeded)
        );
        assert_eq!(job.state, MediaJobState::Succeeded);
    }

    #[test]
    fn uncancellable_executor_results_are_discarded_after_cancel() {
        let mut job = MediaJobContract::new(
            TenantId::new(),
            VideoId::new(),
            ProgressCapability::None,
            CancellationCapability::None,
        );
        let token = job
            .claim(
                WorkerId::parse("managed-media").expect("worker"),
                timestamp(1),
                duration(100),
            )
            .expect("claim");
        job.start(token, timestamp(2)).expect("start");
        assert_eq!(
            job.request_cancel(),
            Ok(CancelOutcome::PendingExecutorCompletion)
        );
        assert_eq!(
            job.complete_success(token, timestamp(3)),
            Ok(CompletionOutcome::DiscardedAfterCancellation)
        );
        assert_eq!(job.state, MediaJobState::Cancelled);
    }

    #[test]
    fn retry_exhaustion_dead_letters_the_job() {
        let mut job = MediaJobContract::new(
            TenantId::new(),
            VideoId::new(),
            ProgressCapability::Milestones,
            CancellationCapability::Cooperative,
        );
        for attempt in 1..=3 {
            let now = i64::from(attempt * 10);
            let token = job
                .claim(
                    WorkerId::parse("worker").expect("worker"),
                    timestamp(now),
                    duration(5),
                )
                .expect("claim");
            let state = job
                .complete_failure(token, timestamp(now + 1), true, 3)
                .expect("failure");
            if attempt < 3 {
                assert_eq!(state, MediaJobState::Queued);
            }
        }
        assert_eq!(job.state, MediaJobState::DeadLetter);
    }

    #[test]
    fn session_decisions_are_ordered_and_tokens_never_exposed() {
        let mut session = SessionRecord::new(
            UserId::new(),
            SecretDigest::parse_sha256("b".repeat(64)).expect("digest"),
            timestamp(10),
            timestamp(20),
            4,
        )
        .expect("session");
        assert_eq!(
            session.authenticate(timestamp(15), 4),
            AuthenticationDecision::Authenticated
        );
        assert_eq!(
            session.authenticate(timestamp(20), 4),
            AuthenticationDecision::Expired
        );
        assert_eq!(
            session.authenticate(timestamp(15), 5),
            AuthenticationDecision::SessionVersionMismatch
        );
        session.revoke();
        assert_eq!(
            session.authenticate(timestamp(15), 4),
            AuthenticationDecision::Revoked
        );
        assert!(!format!("{session:?}").contains(&"b".repeat(64)));
    }

    #[test]
    fn authorization_is_tenant_safe_and_role_driven() {
        let tenant = TenantId::new();
        let user = UserId::new();
        let actor = TenantContext {
            tenant_id: tenant,
            actor_id: user,
        };
        let member = OrganizationMembership {
            tenant_id: tenant,
            user_id: user,
            role: OrganizationRole::Member,
            state: MembershipState::Active,
        };
        let own = ResourcePolicyContext {
            tenant_id: tenant,
            owner_id: Some(user),
            space_role: None,
        };
        assert_eq!(
            AuthorizationPolicy::evaluate(actor, Some(member), own, PolicyAction::VideoDelete),
            AuthorizationDecision::Allow
        );
        assert_eq!(
            AuthorizationPolicy::evaluate(actor, Some(member), own, PolicyAction::ManageBilling),
            AuthorizationDecision::Deny(DenialReason::InsufficientRole)
        );
        assert_eq!(
            AuthorizationPolicy::evaluate(
                actor,
                Some(member),
                ResourcePolicyContext {
                    tenant_id: TenantId::new(),
                    ..own
                },
                PolicyAction::VideoRead
            ),
            AuthorizationDecision::Deny(DenialReason::CrossTenant)
        );
    }

    #[test]
    fn authorization_role_matrix_covers_every_action() {
        let actions = [
            PolicyAction::OrganizationRead,
            PolicyAction::OrganizationUpdate,
            PolicyAction::TransferOwnership,
            PolicyAction::DeleteOrganization,
            PolicyAction::ManageMembers,
            PolicyAction::ManageBilling,
            PolicyAction::ManageStorage,
            PolicyAction::SpaceManage,
            PolicyAction::VideoRead,
            PolicyAction::VideoCreate,
            PolicyAction::VideoUpdate,
            PolicyAction::VideoDelete,
            PolicyAction::CommentCreate,
        ];
        let tenant = TenantId::new();
        let user = UserId::new();
        let actor = TenantContext {
            tenant_id: tenant,
            actor_id: user,
        };
        let resource = ResourcePolicyContext {
            tenant_id: tenant,
            owner_id: Some(UserId::new()),
            space_role: None,
        };
        for action in actions {
            let decision = |role| {
                AuthorizationPolicy::evaluate(
                    actor,
                    Some(OrganizationMembership {
                        tenant_id: tenant,
                        user_id: user,
                        role,
                        state: MembershipState::Active,
                    }),
                    resource,
                    action,
                )
            };
            assert_eq!(
                decision(OrganizationRole::Owner),
                AuthorizationDecision::Allow
            );
            assert_eq!(
                decision(OrganizationRole::Admin),
                if matches!(
                    action,
                    PolicyAction::TransferOwnership | PolicyAction::DeleteOrganization
                ) {
                    AuthorizationDecision::Deny(DenialReason::InsufficientRole)
                } else {
                    AuthorizationDecision::Allow
                }
            );
            assert_eq!(
                decision(OrganizationRole::Viewer),
                if matches!(
                    action,
                    PolicyAction::OrganizationRead | PolicyAction::VideoRead
                ) {
                    AuthorizationDecision::Allow
                } else {
                    AuthorizationDecision::Deny(DenialReason::InsufficientRole)
                }
            );
        }
    }

    #[test]
    fn cutover_requires_evidence_and_remains_rollbackable_until_finalized() {
        let scope = CutoverScope::new(
            TenantId::parse("00000000-0000-0000-0000-000000000017").expect("tenant"),
            CutoverDomain::parse("metadata").expect("domain"),
        );
        let mut state = CutoverState::new(scope);
        state
            .transition(
                CutoverPhase::ShadowRead,
                CutoverEvidence {
                    shadow_observation_ready: true,
                    ..CutoverEvidence::default()
                },
            )
            .expect("shadow");
        assert_eq!(
            state.transition(CutoverPhase::DualWrite, CutoverEvidence::default()),
            Err(LifecycleError::InvalidTransition)
        );
        state
            .transition(
                CutoverPhase::DualWrite,
                CutoverEvidence {
                    reconciliation_clean: true,
                    observation_window_complete: true,
                    ..CutoverEvidence::default()
                },
            )
            .expect("dual write");
        assert_eq!(state.writer, DataAuthority::Legacy);
        assert!(state.mirror_enabled);
        state
            .transition(
                CutoverPhase::D1Authoritative,
                CutoverEvidence {
                    reconciliation_clean: true,
                    rollback_rehearsed: true,
                    observation_window_complete: true,
                    reconciliation_digest_present: true,
                    legacy_fenced: true,
                    ..CutoverEvidence::default()
                },
            )
            .expect("D1");
        let stale_epoch = state.epoch - 1;
        assert_eq!(
            state.authorize_writer(DataAuthority::Legacy, stale_epoch),
            Err(LifecycleError::InvalidTransition)
        );
        assert!(
            state
                .authorize_writer(DataAuthority::D1, state.epoch)
                .is_ok()
        );
        state
            .transition(
                CutoverPhase::RolledBack,
                CutoverEvidence {
                    rollback_rehearsed: true,
                    d1_fenced: true,
                    legacy_caught_up: true,
                    reconciliation_clean: true,
                    reconciliation_digest_present: true,
                    ..CutoverEvidence::default()
                },
            )
            .expect("rollback");
        assert_eq!(state.writer, DataAuthority::Legacy);
        assert!(state.invariants_hold());
    }

    #[test]
    fn reconciliation_requires_counts_bytes_and_semantics_to_match() {
        let clean = ReconciliationSummary::new(2, 2, 10, 10, 0).expect("summary");
        assert!(clean.is_clean());
        assert!(
            !ReconciliationSummary {
                semantic_mismatches: 1,
                ..clean
            }
            .is_clean()
        );
    }
}
