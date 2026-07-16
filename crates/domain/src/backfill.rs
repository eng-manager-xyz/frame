use std::{
    collections::{HashMap, HashSet},
    fmt,
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ByteSize, ChecksumSha256, ContentType, DurationMillis, ObjectRevision, ObjectRole,
    ScopedObjectKey, TenantId, TimestampMillis, VideoId,
};

pub const OBJECT_BACKFILL_PROTOCOL_VERSION_V1: u16 = 1;
pub const OBJECT_BACKFILL_MANIFEST_SCHEMA_VERSION_V1: u16 = 1;
pub const OBJECT_BACKFILL_JOURNAL_SCHEMA_VERSION_V1: u16 = 1;
const ROLE_COUNT: usize = 10;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum BackfillContractErrorV1 {
    #[error("object-backfill protocol version is unsupported")]
    UnsupportedVersion,
    #[error("object-backfill manifest is invalid")]
    InvalidManifest,
    #[error("object-backfill policy is invalid")]
    InvalidPolicy,
    #[error("object-backfill journal is invalid")]
    InvalidJournal,
    #[error("object-backfill state transition is invalid")]
    InvalidTransition,
    #[error("object-backfill lease is stale")]
    StaleLease,
    #[error("object-backfill owner disposition is invalid")]
    InvalidDisposition,
    #[error("object-backfill owner approval is invalid")]
    InvalidApproval,
    #[error("object-backfill source-retention release is not authorized")]
    RetentionBlocked,
    #[error("object-backfill clock moved backwards")]
    ClockRollback,
    #[error("object-backfill reconciliation report is invalid")]
    InvalidReport,
    #[error("object-backfill provider value is invalid")]
    InvalidProviderValue,
    #[error("object-backfill credential reference is invalid")]
    InvalidCredentialReference,
}

macro_rules! backfill_uuid {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn parse(value: &str) -> Result<Self, BackfillContractErrorV1> {
                let value = Uuid::parse_str(value)
                    .map_err(|_| BackfillContractErrorV1::InvalidProviderValue)?;
                if value.is_nil() {
                    return Err(BackfillContractErrorV1::InvalidProviderValue);
                }
                Ok(Self(value))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.debug_tuple($kind).field(&self.0).finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = BackfillContractErrorV1;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

backfill_uuid!(BackfillManifestIdV1, "BackfillManifestIdV1");
backfill_uuid!(BackfillEntryIdV1, "BackfillEntryIdV1");
backfill_uuid!(BackfillOperationIdV1, "BackfillOperationIdV1");
backfill_uuid!(BackfillWorkerIdV1, "BackfillWorkerIdV1");
backfill_uuid!(BackfillOwnerApprovalIdV1, "BackfillOwnerApprovalIdV1");

/// Durable evidence returned by the trusted owner-authorization boundary.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillOwnerApprovalRecordV1 {
    approval_id: BackfillOwnerApprovalIdV1,
    subject_fingerprint: ChecksumSha256,
    scope_fingerprint: ChecksumSha256,
    issued_at: TimestampMillis,
    expires_at: TimestampMillis,
    verified_at: TimestampMillis,
}

impl BackfillOwnerApprovalRecordV1 {
    pub fn new(
        approval_id: BackfillOwnerApprovalIdV1,
        subject_fingerprint: ChecksumSha256,
        scope_fingerprint: ChecksumSha256,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
        verified_at: TimestampMillis,
    ) -> Result<Self, BackfillContractErrorV1> {
        if issued_at > verified_at || expires_at <= verified_at {
            return Err(BackfillContractErrorV1::InvalidApproval);
        }
        Ok(Self {
            approval_id,
            subject_fingerprint,
            scope_fingerprint,
            issued_at,
            expires_at,
            verified_at,
        })
    }

    #[must_use]
    pub const fn approval_id(&self) -> BackfillOwnerApprovalIdV1 {
        self.approval_id
    }

    #[must_use]
    pub const fn subject_fingerprint(&self) -> &ChecksumSha256 {
        &self.subject_fingerprint
    }

    #[must_use]
    pub const fn scope_fingerprint(&self) -> &ChecksumSha256 {
        &self.scope_fingerprint
    }

    #[must_use]
    pub const fn issued_at(&self) -> TimestampMillis {
        self.issued_at
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }

    #[must_use]
    pub const fn verified_at(&self) -> TimestampMillis {
        self.verified_at
    }

    pub fn validate_record(
        &self,
        expected_scope: &ChecksumSha256,
    ) -> Result<(), BackfillContractErrorV1> {
        if &self.scope_fingerprint != expected_scope
            || self.issued_at > self.verified_at
            || self.expires_at <= self.verified_at
        {
            return Err(BackfillContractErrorV1::InvalidApproval);
        }
        Ok(())
    }

    pub fn validate_at(
        &self,
        expected_scope: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<(), BackfillContractErrorV1> {
        self.validate_record(expected_scope)?;
        if self.verified_at > now || self.expires_at <= now {
            return Err(BackfillContractErrorV1::InvalidApproval);
        }
        Ok(())
    }
}

impl fmt::Debug for BackfillOwnerApprovalRecordV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillOwnerApprovalRecordV1")
            .field("approval_id", &self.approval_id)
            .field("subject_fingerprint", &"[redacted]")
            .field("scope_fingerprint", &"[redacted]")
            .field("issued_at", &self.issued_at)
            .field("expires_at", &self.expires_at)
            .field("verified_at", &self.verified_at)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillProviderV1 {
    S3,
    R2,
    Minio,
    GoogleDrive,
    CustomS3Compatible,
}

impl BackfillProviderV1 {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::R2 => "r2",
            Self::Minio => "minio",
            Self::GoogleDrive => "google_drive",
            Self::CustomS3Compatible => "custom_s3_compatible",
        }
    }
}

fn valid_provider_token(value: &str, maximum: usize, allow_slash: bool) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
        && !value
            .split('/')
            .any(|segment| matches!(segment, "." | ".."))
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'-' | b'_' | b'.')
                || (allow_slash && byte == b'/')
        })
}

/// A stable, credential-free provider namespace such as a bucket, container, or Drive ID.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BackfillProviderLocatorV1(String);

impl BackfillProviderLocatorV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillContractErrorV1> {
        let value = value.into();
        if !valid_provider_token(&value, 256, false) {
            return Err(BackfillContractErrorV1::InvalidProviderValue);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackfillProviderLocatorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillProviderLocatorV1([redacted])")
    }
}

impl TryFrom<String> for BackfillProviderLocatorV1 {
    type Error = BackfillContractErrorV1;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<BackfillProviderLocatorV1> for String {
    fn from(value: BackfillProviderLocatorV1) -> Self {
        value.0
    }
}

/// A credential-free source object reference. Signed URLs and query strings cannot parse.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BackfillSourceReferenceV1(String);

impl BackfillSourceReferenceV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillContractErrorV1> {
        let value = value.into();
        if !valid_provider_token(&value, 1_024, true) {
            return Err(BackfillContractErrorV1::InvalidProviderValue);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackfillSourceReferenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillSourceReferenceV1([redacted])")
    }
}

impl TryFrom<String> for BackfillSourceReferenceV1 {
    type Error = BackfillContractErrorV1;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<BackfillSourceReferenceV1> for String {
    fn from(value: BackfillSourceReferenceV1) -> Self {
        value.0
    }
}

/// An opaque secret-store reference. It is deliberately neither `Serialize` nor `Clone`.
pub struct BackfillCredentialRefV1(String);

impl BackfillCredentialRefV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillContractErrorV1> {
        let value = value.into();
        if !(8..=256).contains(&value.len())
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b':')
            })
            || value.contains("//")
        {
            return Err(BackfillContractErrorV1::InvalidCredentialReference);
        }
        Ok(Self(value))
    }

    /// Exposes the opaque reference only at the credential-provider adapter boundary.
    #[must_use]
    pub fn expose_to_adapter(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackfillCredentialRefV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillCredentialRefV1([redacted])")
    }
}

impl fmt::Display for BackfillCredentialRefV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillStorageAuthorityV1 {
    provider: BackfillProviderV1,
    region: String,
    locator: BackfillProviderLocatorV1,
    authority_fingerprint: ChecksumSha256,
}

impl BackfillStorageAuthorityV1 {
    pub fn new(
        provider: BackfillProviderV1,
        region: impl Into<String>,
        locator: BackfillProviderLocatorV1,
        authority_fingerprint: ChecksumSha256,
    ) -> Result<Self, BackfillContractErrorV1> {
        let region = region.into();
        if !valid_provider_token(&region, 64, false) {
            return Err(BackfillContractErrorV1::InvalidProviderValue);
        }
        Ok(Self {
            provider,
            region,
            locator,
            authority_fingerprint,
        })
    }

    #[must_use]
    pub const fn provider(&self) -> BackfillProviderV1 {
        self.provider
    }

    #[must_use]
    pub fn region(&self) -> &str {
        &self.region
    }

    #[must_use]
    pub const fn locator(&self) -> &BackfillProviderLocatorV1 {
        &self.locator
    }

    #[must_use]
    pub const fn authority_fingerprint(&self) -> &ChecksumSha256 {
        &self.authority_fingerprint
    }

    pub fn validate(&self) -> Result<(), BackfillContractErrorV1> {
        if !valid_provider_token(&self.region, 64, false) {
            return Err(BackfillContractErrorV1::InvalidProviderValue);
        }
        Ok(())
    }
}

impl fmt::Debug for BackfillStorageAuthorityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillStorageAuthorityV1")
            .field("provider", &self.provider)
            .field("region", &self.region)
            .field("locator", &self.locator)
            .field("authority_fingerprint", &"[redacted]")
            .finish()
    }
}

/// Opaque provider checksums are retained for inventory diagnostics only. They are never SHA-256.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BackfillProviderChecksumV1(String);

impl BackfillProviderChecksumV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillContractErrorV1> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 256
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(BackfillContractErrorV1::InvalidProviderValue);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackfillProviderChecksumV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillProviderChecksumV1([opaque])")
    }
}

impl TryFrom<String> for BackfillProviderChecksumV1 {
    type Error = BackfillContractErrorV1;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<BackfillProviderChecksumV1> for String {
    fn from(value: BackfillProviderChecksumV1) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillMediaProbeModeV1 {
    Required,
    NotRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillMediaProbePolicyV1 {
    profile_version: u16,
    mode: BackfillMediaProbeModeV1,
}

impl BackfillMediaProbePolicyV1 {
    pub fn new(
        profile_version: u16,
        mode: BackfillMediaProbeModeV1,
    ) -> Result<Self, BackfillContractErrorV1> {
        if profile_version == 0 {
            return Err(BackfillContractErrorV1::InvalidManifest);
        }
        Ok(Self {
            profile_version,
            mode,
        })
    }

    #[must_use]
    pub const fn profile_version(self) -> u16 {
        self.profile_version
    }

    #[must_use]
    pub const fn mode(self) -> BackfillMediaProbeModeV1 {
        self.mode
    }

    #[must_use]
    pub const fn required(self) -> bool {
        matches!(self.mode, BackfillMediaProbeModeV1::Required)
    }

    pub fn validate(self) -> Result<(), BackfillContractErrorV1> {
        if self.profile_version == 0 {
            return Err(BackfillContractErrorV1::InvalidManifest);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBackfillManifestEntryV1 {
    entry_id: BackfillEntryIdV1,
    tenant_id: TenantId,
    video_id: VideoId,
    role: ObjectRole,
    source_reference: BackfillSourceReferenceV1,
    target_key: ScopedObjectKey,
    target_version: ObjectRevision,
    expected_size: ByteSize,
    strong_sha256: ChecksumSha256,
    source_provider_checksum: Option<BackfillProviderChecksumV1>,
    content_type: ContentType,
    media_probe: BackfillMediaProbePolicyV1,
}

impl ObjectBackfillManifestEntryV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entry_id: BackfillEntryIdV1,
        tenant_id: TenantId,
        video_id: VideoId,
        role: ObjectRole,
        source_reference: BackfillSourceReferenceV1,
        target_key: ScopedObjectKey,
        expected_size: ByteSize,
        strong_sha256: ChecksumSha256,
        source_provider_checksum: Option<BackfillProviderChecksumV1>,
        content_type: ContentType,
        media_probe: BackfillMediaProbePolicyV1,
    ) -> Result<Self, BackfillContractErrorV1> {
        if expected_size.get() == 0
            || !target_key.belongs_to(tenant_id, video_id)
            || target_key.role() != role
        {
            return Err(BackfillContractErrorV1::InvalidManifest);
        }
        let target_version = target_key.source_revision();
        Ok(Self {
            entry_id,
            tenant_id,
            video_id,
            role,
            source_reference,
            target_key,
            target_version,
            expected_size,
            strong_sha256,
            source_provider_checksum,
            content_type,
            media_probe,
        })
    }

    #[must_use]
    pub const fn entry_id(&self) -> BackfillEntryIdV1 {
        self.entry_id
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn video_id(&self) -> VideoId {
        self.video_id
    }

    #[must_use]
    pub const fn role(&self) -> ObjectRole {
        self.role
    }

    #[must_use]
    pub const fn source_reference(&self) -> &BackfillSourceReferenceV1 {
        &self.source_reference
    }

    #[must_use]
    pub const fn target_key(&self) -> &ScopedObjectKey {
        &self.target_key
    }

    #[must_use]
    pub const fn target_version(&self) -> ObjectRevision {
        self.target_version
    }

    #[must_use]
    pub const fn expected_size(&self) -> ByteSize {
        self.expected_size
    }

    #[must_use]
    pub const fn strong_sha256(&self) -> &ChecksumSha256 {
        &self.strong_sha256
    }

    #[must_use]
    pub const fn source_provider_checksum(&self) -> Option<&BackfillProviderChecksumV1> {
        self.source_provider_checksum.as_ref()
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn media_probe(&self) -> BackfillMediaProbePolicyV1 {
        self.media_probe
    }
}

impl fmt::Debug for ObjectBackfillManifestEntryV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectBackfillManifestEntryV1")
            .field("entry_id", &self.entry_id)
            .field("role", &self.role)
            .field("target_version", &self.target_version)
            .field("expected_size", &self.expected_size)
            .field("content_type", &self.content_type)
            .field("media_probe", &self.media_probe)
            .field("tenant_id", &"[redacted]")
            .field("video_id", &"[redacted]")
            .field("source_reference", &"[redacted]")
            .field("target_key", &"[redacted]")
            .field("strong_sha256", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBackfillManifestV1 {
    protocol_version: u16,
    schema_version: u16,
    manifest_id: BackfillManifestIdV1,
    created_at: TimestampMillis,
    tool_version: String,
    code_version: String,
    source: BackfillStorageAuthorityV1,
    target: BackfillStorageAuthorityV1,
    execution_policy: BackfillExecutionPolicyV1,
    entries: Vec<ObjectBackfillManifestEntryV1>,
    digest: ChecksumSha256,
}

impl ObjectBackfillManifestV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manifest_id: BackfillManifestIdV1,
        created_at: TimestampMillis,
        tool_version: impl Into<String>,
        code_version: impl Into<String>,
        source: BackfillStorageAuthorityV1,
        target: BackfillStorageAuthorityV1,
        execution_policy: BackfillExecutionPolicyV1,
        entries: Vec<ObjectBackfillManifestEntryV1>,
    ) -> Result<Self, BackfillContractErrorV1> {
        let tool_version = tool_version.into();
        let code_version = code_version.into();
        if !valid_version(&tool_version)
            || !valid_version(&code_version)
            || entries.is_empty()
            || source == target
        {
            return Err(BackfillContractErrorV1::InvalidManifest);
        }
        let mut entry_ids = HashSet::with_capacity(entries.len());
        let mut target_keys = HashSet::with_capacity(entries.len());
        if entries.iter().any(|entry| {
            !entry_ids.insert(entry.entry_id)
                || !target_keys.insert(entry.target_key.as_str().to_owned())
        }) {
            return Err(BackfillContractErrorV1::InvalidManifest);
        }
        let digest = manifest_digest(
            manifest_id,
            created_at,
            &tool_version,
            &code_version,
            &source,
            &target,
            execution_policy,
            &entries,
        );
        Ok(Self {
            protocol_version: OBJECT_BACKFILL_PROTOCOL_VERSION_V1,
            schema_version: OBJECT_BACKFILL_MANIFEST_SCHEMA_VERSION_V1,
            manifest_id,
            created_at,
            tool_version,
            code_version,
            source,
            target,
            execution_policy,
            entries,
            digest,
        })
    }

    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn manifest_id(&self) -> BackfillManifestIdV1 {
        self.manifest_id
    }

    #[must_use]
    pub const fn created_at(&self) -> TimestampMillis {
        self.created_at
    }

    #[must_use]
    pub fn tool_version(&self) -> &str {
        &self.tool_version
    }

    #[must_use]
    pub fn code_version(&self) -> &str {
        &self.code_version
    }

    #[must_use]
    pub const fn source(&self) -> &BackfillStorageAuthorityV1 {
        &self.source
    }

    #[must_use]
    pub const fn target(&self) -> &BackfillStorageAuthorityV1 {
        &self.target
    }

    #[must_use]
    pub const fn execution_policy(&self) -> BackfillExecutionPolicyV1 {
        self.execution_policy
    }

    #[must_use]
    pub fn entries(&self) -> &[ObjectBackfillManifestEntryV1] {
        &self.entries
    }

    #[must_use]
    pub fn entry(&self, entry_id: BackfillEntryIdV1) -> Option<&ObjectBackfillManifestEntryV1> {
        self.entries.iter().find(|entry| entry.entry_id == entry_id)
    }

    #[must_use]
    pub const fn digest(&self) -> &ChecksumSha256 {
        &self.digest
    }

    pub fn validate_integrity(&self) -> Result<(), BackfillContractErrorV1> {
        let mut entry_ids = HashSet::with_capacity(self.entries.len());
        let mut target_keys = HashSet::with_capacity(self.entries.len());
        let entries_valid = self.entries.iter().all(|entry| {
            entry.expected_size.get() > 0
                && entry.target_key.belongs_to(entry.tenant_id, entry.video_id)
                && entry.target_key.role() == entry.role
                && entry.target_version == entry.target_key.source_revision()
                && entry.media_probe.validate().is_ok()
                && entry_ids.insert(entry.entry_id)
                && target_keys.insert(entry.target_key.as_str().to_owned())
        });
        let expected_digest = manifest_digest(
            self.manifest_id,
            self.created_at,
            &self.tool_version,
            &self.code_version,
            &self.source,
            &self.target,
            self.execution_policy,
            &self.entries,
        );
        if self.protocol_version != OBJECT_BACKFILL_PROTOCOL_VERSION_V1
            || self.schema_version != OBJECT_BACKFILL_MANIFEST_SCHEMA_VERSION_V1
            || !valid_version(&self.tool_version)
            || !valid_version(&self.code_version)
            || self.entries.is_empty()
            || self.source == self.target
            || self.source.validate().is_err()
            || self.target.validate().is_err()
            || self.execution_policy.validate().is_err()
            || !entries_valid
            || self.digest != expected_digest
        {
            return Err(BackfillContractErrorV1::InvalidManifest);
        }
        Ok(())
    }
}

impl fmt::Debug for ObjectBackfillManifestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectBackfillManifestV1")
            .field("protocol_version", &self.protocol_version)
            .field("schema_version", &self.schema_version)
            .field("manifest_id", &self.manifest_id)
            .field("created_at", &self.created_at)
            .field("tool_version", &self.tool_version)
            .field("code_version", &self.code_version)
            .field("source", &self.source)
            .field("target", &self.target)
            .field("execution_policy", &self.execution_policy)
            .field("entry_count", &self.entries.len())
            .field("digest", &"[redacted]")
            .finish()
    }
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
}

fn frame(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    output.extend_from_slice(value);
}

fn frame_authority(output: &mut Vec<u8>, authority: &BackfillStorageAuthorityV1) {
    frame(output, authority.provider.tag().as_bytes());
    frame(output, authority.region.as_bytes());
    frame(output, authority.locator.as_str().as_bytes());
    frame(output, authority.authority_fingerprint.as_str().as_bytes());
}

#[allow(clippy::too_many_arguments)]
fn manifest_digest(
    manifest_id: BackfillManifestIdV1,
    created_at: TimestampMillis,
    tool_version: &str,
    code_version: &str,
    source: &BackfillStorageAuthorityV1,
    target: &BackfillStorageAuthorityV1,
    execution_policy: BackfillExecutionPolicyV1,
    entries: &[ObjectBackfillManifestEntryV1],
) -> ChecksumSha256 {
    let mut bytes = Vec::with_capacity(entries.len().saturating_mul(512));
    frame(
        &mut bytes,
        &OBJECT_BACKFILL_PROTOCOL_VERSION_V1.to_be_bytes(),
    );
    frame(
        &mut bytes,
        &OBJECT_BACKFILL_MANIFEST_SCHEMA_VERSION_V1.to_be_bytes(),
    );
    frame(&mut bytes, manifest_id.to_string().as_bytes());
    frame(&mut bytes, &created_at.get().to_be_bytes());
    frame(&mut bytes, tool_version.as_bytes());
    frame(&mut bytes, code_version.as_bytes());
    frame_authority(&mut bytes, source);
    frame_authority(&mut bytes, target);
    frame(&mut bytes, &execution_policy.max_concurrency.to_be_bytes());
    frame(&mut bytes, &execution_policy.max_attempts.to_be_bytes());
    frame(
        &mut bytes,
        &execution_policy.max_entries_per_run.to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy.max_logical_bytes_per_run.to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy.max_cost_units_per_run.to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy
            .max_bandwidth_bytes_per_second
            .to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy.max_objects_per_minute.to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy.retry_base_delay.get().to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy.retry_max_delay.get().to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy.circuit_failure_threshold.to_be_bytes(),
    );
    frame(
        &mut bytes,
        &execution_policy.circuit_cooldown.get().to_be_bytes(),
    );
    frame(&mut bytes, &execution_policy.lease_ttl.get().to_be_bytes());
    frame(
        &mut bytes,
        &execution_policy.max_chunk_bytes.get().to_be_bytes(),
    );
    for entry in entries {
        frame(&mut bytes, entry.entry_id.to_string().as_bytes());
        frame(&mut bytes, entry.tenant_id.to_string().as_bytes());
        frame(&mut bytes, entry.video_id.to_string().as_bytes());
        frame(&mut bytes, entry.role.path_segment().as_bytes());
        frame(&mut bytes, entry.source_reference.as_str().as_bytes());
        frame(&mut bytes, entry.target_key.as_str().as_bytes());
        frame(&mut bytes, &entry.target_version.get().to_be_bytes());
        frame(&mut bytes, &entry.expected_size.get().to_be_bytes());
        frame(&mut bytes, entry.strong_sha256.as_str().as_bytes());
        frame(
            &mut bytes,
            entry
                .source_provider_checksum
                .as_ref()
                .map_or(&[][..], |checksum| checksum.as_str().as_bytes()),
        );
        frame(&mut bytes, entry.content_type.as_str().as_bytes());
        frame(&mut bytes, &entry.media_probe.profile_version.to_be_bytes());
        frame(&mut bytes, &[u8::from(entry.media_probe.required())]);
    }
    ChecksumSha256::digest_bytes(&bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillProviderCapabilitiesV1 {
    pub streaming_read: bool,
    pub streaming_conditional_create: bool,
    pub exact_head_after_write: bool,
    pub immutable_versions: bool,
    pub independent_inventory: bool,
    pub snapshot_inventory: bool,
    pub cancelable_staging_write: bool,
    pub live_commit_fencing: bool,
}

impl BackfillProviderCapabilitiesV1 {
    #[must_use]
    pub const fn source_satisfies_backfill(self) -> bool {
        self.streaming_read && self.independent_inventory && self.snapshot_inventory
    }

    #[must_use]
    pub const fn target_satisfies_backfill(self) -> bool {
        self.streaming_read
            && self.streaming_conditional_create
            && self.exact_head_after_write
            && self.immutable_versions
            && self.independent_inventory
            && self.snapshot_inventory
            && self.cancelable_staging_write
            && self.live_commit_fencing
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillExecutionPolicyV1 {
    max_concurrency: u16,
    max_attempts: u16,
    max_entries_per_run: u64,
    max_logical_bytes_per_run: u64,
    max_cost_units_per_run: u64,
    max_bandwidth_bytes_per_second: u64,
    max_objects_per_minute: u32,
    retry_base_delay: DurationMillis,
    retry_max_delay: DurationMillis,
    circuit_failure_threshold: u16,
    circuit_cooldown: DurationMillis,
    lease_ttl: DurationMillis,
    max_chunk_bytes: ByteSize,
}

impl BackfillExecutionPolicyV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_concurrency: u16,
        max_attempts: u16,
        max_entries_per_run: u64,
        max_logical_bytes_per_run: u64,
        max_cost_units_per_run: u64,
        max_bandwidth_bytes_per_second: u64,
        max_objects_per_minute: u32,
        retry_base_delay: DurationMillis,
        retry_max_delay: DurationMillis,
        circuit_failure_threshold: u16,
        circuit_cooldown: DurationMillis,
        lease_ttl: DurationMillis,
        max_chunk_bytes: ByteSize,
    ) -> Result<Self, BackfillContractErrorV1> {
        let policy = Self {
            max_concurrency,
            max_attempts,
            max_entries_per_run,
            max_logical_bytes_per_run,
            max_cost_units_per_run,
            max_bandwidth_bytes_per_second,
            max_objects_per_minute,
            retry_base_delay,
            retry_max_delay,
            circuit_failure_threshold,
            circuit_cooldown,
            lease_ttl,
            max_chunk_bytes,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(self) -> Result<(), BackfillContractErrorV1> {
        if self.max_concurrency == 0
            || self.max_attempts == 0
            || self.max_attempts == u16::MAX
            || self.max_entries_per_run == 0
            || self.max_entries_per_run > MAX_SAFE_INTEGER
            || self.max_logical_bytes_per_run == 0
            || self.max_logical_bytes_per_run > MAX_SAFE_INTEGER
            || self.max_cost_units_per_run == 0
            || self.max_cost_units_per_run > MAX_SAFE_INTEGER
            || self.max_bandwidth_bytes_per_second == 0
            || self.max_bandwidth_bytes_per_second > MAX_SAFE_INTEGER
            || self.max_objects_per_minute == 0
            || self.retry_base_delay > self.retry_max_delay
            || self.circuit_failure_threshold == 0
            || self.max_chunk_bytes.get() == 0
            || self.max_chunk_bytes.get() > self.max_logical_bytes_per_run
        {
            return Err(BackfillContractErrorV1::InvalidPolicy);
        }
        Ok(())
    }

    #[must_use]
    pub const fn max_concurrency(self) -> u16 {
        self.max_concurrency
    }

    #[must_use]
    pub const fn max_attempts(self) -> u16 {
        self.max_attempts
    }

    #[must_use]
    pub const fn max_entries_per_run(self) -> u64 {
        self.max_entries_per_run
    }

    #[must_use]
    pub const fn max_logical_bytes_per_run(self) -> u64 {
        self.max_logical_bytes_per_run
    }

    #[must_use]
    pub const fn max_cost_units_per_run(self) -> u64 {
        self.max_cost_units_per_run
    }

    #[must_use]
    pub const fn max_bandwidth_bytes_per_second(self) -> u64 {
        self.max_bandwidth_bytes_per_second
    }

    #[must_use]
    pub const fn max_objects_per_minute(self) -> u32 {
        self.max_objects_per_minute
    }

    #[must_use]
    pub const fn retry_base_delay(self) -> DurationMillis {
        self.retry_base_delay
    }

    #[must_use]
    pub const fn retry_max_delay(self) -> DurationMillis {
        self.retry_max_delay
    }

    #[must_use]
    pub const fn circuit_failure_threshold(self) -> u16 {
        self.circuit_failure_threshold
    }

    #[must_use]
    pub const fn circuit_cooldown(self) -> DurationMillis {
        self.circuit_cooldown
    }

    #[must_use]
    pub const fn lease_ttl(self) -> DurationMillis {
        self.lease_ttl
    }

    #[must_use]
    pub const fn max_chunk_bytes(self) -> ByteSize {
        self.max_chunk_bytes
    }

    #[must_use]
    pub fn backoff_for_attempt(self, attempt: u16) -> DurationMillis {
        let shift = u32::from(attempt.saturating_sub(1).min(62));
        let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
        let delay = self
            .retry_base_delay
            .get()
            .saturating_mul(multiplier)
            .min(self.retry_max_delay.get());
        DurationMillis::new(delay).expect("validated retry delays are positive and wire-safe")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillRunStateV1 {
    Running,
    Paused,
    Aborting,
    Aborted,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillFailureClassV1 {
    MissingSource,
    MissingTarget,
    TruncatedSource,
    ExtraSourceBytes,
    SourceChecksumMismatch,
    TargetChecksumMismatch,
    SourceOwnershipMismatch,
    TargetOwnershipMismatch,
    SourceMetadataMismatch,
    TargetMetadataMismatch,
    MediaUnplayable,
    EmptyChunk,
    OversizedChunk,
    ProviderThrottled,
    ProviderExpiredAuthorization,
    ProviderOutage,
    CircuitOpen,
    TransferCanceled,
    DestinationConflict,
    CheckpointDivergence,
    CapabilityMissing,
    BudgetExceeded,
}

impl BackfillFailureClassV1 {
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            Self::ProviderThrottled
                | Self::ProviderExpiredAuthorization
                | Self::ProviderOutage
                | Self::CircuitOpen
                | Self::TransferCanceled
        )
    }

    const fn tag(self) -> &'static str {
        match self {
            Self::MissingSource => "missing_source",
            Self::MissingTarget => "missing_target",
            Self::TruncatedSource => "truncated_source",
            Self::ExtraSourceBytes => "extra_source_bytes",
            Self::SourceChecksumMismatch => "source_checksum_mismatch",
            Self::TargetChecksumMismatch => "target_checksum_mismatch",
            Self::SourceOwnershipMismatch => "source_ownership_mismatch",
            Self::TargetOwnershipMismatch => "target_ownership_mismatch",
            Self::SourceMetadataMismatch => "source_metadata_mismatch",
            Self::TargetMetadataMismatch => "target_metadata_mismatch",
            Self::MediaUnplayable => "media_unplayable",
            Self::EmptyChunk => "empty_chunk",
            Self::OversizedChunk => "oversized_chunk",
            Self::ProviderThrottled => "provider_throttled",
            Self::ProviderExpiredAuthorization => "provider_expired_authorization",
            Self::ProviderOutage => "provider_outage",
            Self::CircuitOpen => "circuit_open",
            Self::TransferCanceled => "transfer_canceled",
            Self::DestinationConflict => "destination_conflict",
            Self::CheckpointDivergence => "checkpoint_divergence",
            Self::CapabilityMissing => "capability_missing",
            Self::BudgetExceeded => "budget_exceeded",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillOwnerDispositionV1 {
    PendingOwnerApproval,
    RetryApproved,
    ReferenceApproved,
    ExcludeApproved,
}

impl BackfillOwnerDispositionV1 {
    const fn tag(self) -> &'static str {
        match self {
            Self::PendingOwnerApproval => "pending_owner_approval",
            Self::RetryApproved => "retry_approved",
            Self::ReferenceApproved => "reference_approved",
            Self::ExcludeApproved => "exclude_approved",
        }
    }
}

#[must_use]
pub fn backfill_disposition_approval_scope_v1(
    manifest_digest: &ChecksumSha256,
    entry_id: BackfillEntryIdV1,
    disposition: BackfillOwnerDispositionV1,
) -> ChecksumSha256 {
    let mut bytes = Vec::new();
    frame(&mut bytes, b"object-backfill-owner-disposition-v1");
    frame(&mut bytes, manifest_digest.as_str().as_bytes());
    frame(&mut bytes, entry_id.to_string().as_bytes());
    frame(&mut bytes, disposition.tag().as_bytes());
    ChecksumSha256::digest_bytes(&bytes)
}

#[must_use]
pub fn backfill_source_release_approval_scope_v1(
    manifest_digest: &ChecksumSha256,
    report_digest: &ChecksumSha256,
) -> ChecksumSha256 {
    let mut bytes = Vec::new();
    frame(&mut bytes, b"object-backfill-source-release-v1");
    frame(&mut bytes, manifest_digest.as_str().as_bytes());
    frame(&mut bytes, report_digest.as_str().as_bytes());
    ChecksumSha256::digest_bytes(&bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillLeaseV1 {
    worker_id: BackfillWorkerIdV1,
    fencing_token: u64,
    expires_at: TimestampMillis,
}

impl BackfillLeaseV1 {
    #[must_use]
    pub const fn worker_id(self) -> BackfillWorkerIdV1 {
        self.worker_id
    }

    #[must_use]
    pub const fn fencing_token(self) -> u64 {
        self.fencing_token
    }

    #[must_use]
    pub const fn expires_at(self) -> TimestampMillis {
        self.expires_at
    }

    #[must_use]
    pub const fn expired_at(self, now: TimestampMillis) -> bool {
        self.expires_at.get() <= now.get()
    }

    #[must_use]
    pub fn same_fence(self, other: Self) -> bool {
        self.worker_id == other.worker_id && self.fencing_token == other.fencing_token
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BackfillDestinationVersionV1(String);

impl BackfillDestinationVersionV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillContractErrorV1> {
        let value = value.into();
        if !valid_provider_token(&value, 256, false) {
            return Err(BackfillContractErrorV1::InvalidProviderValue);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackfillDestinationVersionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillDestinationVersionV1([redacted])")
    }
}

impl TryFrom<String> for BackfillDestinationVersionV1 {
    type Error = BackfillContractErrorV1;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<BackfillDestinationVersionV1> for String {
    fn from(value: BackfillDestinationVersionV1) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillWriteResultV1 {
    Created,
    ReusedExact,
    Referenced,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillOperationReceiptV1 {
    operation_id: BackfillOperationIdV1,
    entry_id: BackfillEntryIdV1,
    result: BackfillWriteResultV1,
    destination_version: BackfillDestinationVersionV1,
    logical_bytes: ByteSize,
    strong_sha256: ChecksumSha256,
    media_probe_profile_version: Option<u16>,
    committed_at: TimestampMillis,
}

impl BackfillOperationReceiptV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation_id: BackfillOperationIdV1,
        entry_id: BackfillEntryIdV1,
        result: BackfillWriteResultV1,
        destination_version: BackfillDestinationVersionV1,
        logical_bytes: ByteSize,
        strong_sha256: ChecksumSha256,
        media_probe_profile_version: Option<u16>,
        committed_at: TimestampMillis,
    ) -> Result<Self, BackfillContractErrorV1> {
        if logical_bytes.get() == 0 || media_probe_profile_version == Some(0) {
            return Err(BackfillContractErrorV1::InvalidJournal);
        }
        Ok(Self {
            operation_id,
            entry_id,
            result,
            destination_version,
            logical_bytes,
            strong_sha256,
            media_probe_profile_version,
            committed_at,
        })
    }

    #[must_use]
    pub const fn operation_id(&self) -> BackfillOperationIdV1 {
        self.operation_id
    }

    #[must_use]
    pub const fn entry_id(&self) -> BackfillEntryIdV1 {
        self.entry_id
    }

    #[must_use]
    pub const fn result(&self) -> BackfillWriteResultV1 {
        self.result
    }

    #[must_use]
    pub const fn destination_version(&self) -> &BackfillDestinationVersionV1 {
        &self.destination_version
    }

    #[must_use]
    pub const fn logical_bytes(&self) -> ByteSize {
        self.logical_bytes
    }

    #[must_use]
    pub const fn strong_sha256(&self) -> &ChecksumSha256 {
        &self.strong_sha256
    }

    #[must_use]
    pub const fn media_probe_profile_version(&self) -> Option<u16> {
        self.media_probe_profile_version
    }

    #[must_use]
    pub const fn committed_at(&self) -> TimestampMillis {
        self.committed_at
    }
}

impl fmt::Debug for BackfillOperationReceiptV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillOperationReceiptV1")
            .field("operation_id", &self.operation_id)
            .field("entry_id", &self.entry_id)
            .field("result", &self.result)
            .field("destination_version", &self.destination_version)
            .field("logical_bytes", &self.logical_bytes)
            .field(
                "media_probe_profile_version",
                &self.media_probe_profile_version,
            )
            .field("committed_at", &self.committed_at)
            .field("strong_sha256", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackfillEntryStatusV1 {
    Pending,
    Leased {
        lease: BackfillLeaseV1,
        operation_id: BackfillOperationIdV1,
    },
    RetryScheduled {
        failure: BackfillFailureClassV1,
        not_before: TimestampMillis,
    },
    Succeeded {
        receipt: BackfillOperationReceiptV1,
    },
    Quarantined {
        failure: BackfillFailureClassV1,
        disposition: BackfillOwnerDispositionV1,
        approval: Option<BackfillOwnerApprovalRecordV1>,
    },
}

impl fmt::Debug for BackfillEntryStatusV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => formatter.write_str("Pending"),
            Self::Leased {
                lease,
                operation_id,
            } => formatter
                .debug_struct("Leased")
                .field("lease", lease)
                .field("operation_id", operation_id)
                .finish(),
            Self::RetryScheduled {
                failure,
                not_before,
            } => formatter
                .debug_struct("RetryScheduled")
                .field("failure", failure)
                .field("not_before", not_before)
                .finish(),
            Self::Succeeded { receipt } => formatter
                .debug_struct("Succeeded")
                .field("receipt", receipt)
                .finish(),
            Self::Quarantined {
                failure,
                disposition,
                approval,
            } => formatter
                .debug_struct("Quarantined")
                .field("failure", failure)
                .field("disposition", disposition)
                .field("approval", approval)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillJournalEntryV1 {
    entry_id: BackfillEntryIdV1,
    attempts: u16,
    retry_approval: Option<BackfillOwnerApprovalRecordV1>,
    retry_approval_consumed: bool,
    last_failure: Option<BackfillFailureClassV1>,
    status: BackfillEntryStatusV1,
}

impl BackfillJournalEntryV1 {
    #[must_use]
    pub const fn entry_id(&self) -> BackfillEntryIdV1 {
        self.entry_id
    }

    #[must_use]
    pub const fn attempts(&self) -> u16 {
        self.attempts
    }

    #[must_use]
    pub const fn retry_approval(&self) -> Option<&BackfillOwnerApprovalRecordV1> {
        self.retry_approval.as_ref()
    }

    #[must_use]
    pub const fn retry_approval_consumed(&self) -> bool {
        self.retry_approval_consumed
    }

    #[must_use]
    pub const fn last_failure(&self) -> Option<BackfillFailureClassV1> {
        self.last_failure
    }

    #[must_use]
    pub const fn status(&self) -> &BackfillEntryStatusV1 {
        &self.status
    }

    #[must_use]
    pub fn eligible_at(&self, now: TimestampMillis) -> bool {
        match &self.status {
            BackfillEntryStatusV1::Pending => true,
            BackfillEntryStatusV1::RetryScheduled { not_before, .. } => *not_before <= now,
            BackfillEntryStatusV1::Leased { lease, .. } => lease.expired_at(now),
            BackfillEntryStatusV1::Succeeded { .. } | BackfillEntryStatusV1::Quarantined { .. } => {
                false
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillUsageV1 {
    admitted_entries: u64,
    admitted_logical_bytes: u64,
    admitted_cost_units: u64,
}

impl BackfillUsageV1 {
    #[must_use]
    pub const fn admitted_entries(self) -> u64 {
        self.admitted_entries
    }

    #[must_use]
    pub const fn admitted_logical_bytes(self) -> u64 {
        self.admitted_logical_bytes
    }

    #[must_use]
    pub const fn admitted_cost_units(self) -> u64 {
        self.admitted_cost_units
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillCircuitStateV1 {
    consecutive_provider_failures: u16,
    open_until: Option<TimestampMillis>,
    half_open_fencing_token: Option<u64>,
}

impl BackfillCircuitStateV1 {
    #[must_use]
    pub const fn consecutive_provider_failures(self) -> u16 {
        self.consecutive_provider_failures
    }

    #[must_use]
    pub const fn open_until(self) -> Option<TimestampMillis> {
        self.open_until
    }

    #[must_use]
    pub const fn half_open_fencing_token(self) -> Option<u64> {
        self.half_open_fencing_token
    }

    #[must_use]
    pub fn is_open_at(self, now: TimestampMillis) -> bool {
        self.open_until.is_some_and(|until| until > now)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackfillSourceRetentionStateV1 {
    Retained,
    ReleaseApproved {
        approval: BackfillOwnerApprovalRecordV1,
        reconciliation_digest: ChecksumSha256,
        approved_at: TimestampMillis,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBackfillJournalV1 {
    protocol_version: u16,
    schema_version: u16,
    manifest_id: BackfillManifestIdV1,
    manifest_digest: ChecksumSha256,
    revision: u64,
    next_fencing_token: u64,
    last_transition_at: TimestampMillis,
    state: BackfillRunStateV1,
    entries: Vec<BackfillJournalEntryV1>,
    usage: BackfillUsageV1,
    circuit: BackfillCircuitStateV1,
    source_retention: BackfillSourceRetentionStateV1,
}

impl ObjectBackfillJournalV1 {
    #[must_use]
    pub fn new(manifest: &ObjectBackfillManifestV1) -> Self {
        Self {
            protocol_version: OBJECT_BACKFILL_PROTOCOL_VERSION_V1,
            schema_version: OBJECT_BACKFILL_JOURNAL_SCHEMA_VERSION_V1,
            manifest_id: manifest.manifest_id,
            manifest_digest: manifest.digest.clone(),
            revision: 0,
            next_fencing_token: 1,
            last_transition_at: manifest.created_at,
            state: BackfillRunStateV1::Running,
            entries: manifest
                .entries
                .iter()
                .map(|entry| BackfillJournalEntryV1 {
                    entry_id: entry.entry_id,
                    attempts: 0,
                    retry_approval: None,
                    retry_approval_consumed: false,
                    last_failure: None,
                    status: BackfillEntryStatusV1::Pending,
                })
                .collect(),
            usage: BackfillUsageV1::default(),
            circuit: BackfillCircuitStateV1::default(),
            source_retention: BackfillSourceRetentionStateV1::Retained,
        }
    }

    pub fn validate_for_manifest(
        &self,
        manifest: &ObjectBackfillManifestV1,
    ) -> Result<(), BackfillContractErrorV1> {
        self.validate_for_manifest_at(manifest, self.last_transition_at)
    }

    pub fn validate_for_manifest_at(
        &self,
        manifest: &ObjectBackfillManifestV1,
        now: TimestampMillis,
    ) -> Result<(), BackfillContractErrorV1> {
        manifest.validate_integrity()?;
        if now < manifest.created_at || now < self.last_transition_at {
            return Err(BackfillContractErrorV1::ClockRollback);
        }
        let entries_valid =
            self.entries
                .iter()
                .zip(&manifest.entries)
                .all(|(journal, manifest_entry)| {
                    let maximum_attempts = manifest.execution_policy.max_attempts.saturating_add(1);
                    if journal.entry_id != manifest_entry.entry_id
                        || journal.attempts > maximum_attempts
                        || journal.retry_approval_consumed && journal.retry_approval.is_none()
                        || (journal.attempts > manifest.execution_policy.max_attempts
                            && !journal.retry_approval_consumed)
                    {
                        return false;
                    }
                    if let Some(approval) = &journal.retry_approval {
                        let scope = backfill_disposition_approval_scope_v1(
                            &self.manifest_digest,
                            journal.entry_id,
                            BackfillOwnerDispositionV1::RetryApproved,
                        );
                        if approval.validate_record(&scope).is_err()
                            || approval.verified_at < manifest.created_at
                            || approval.verified_at > self.last_transition_at
                        {
                            return false;
                        }
                    }
                    match &journal.status {
                        BackfillEntryStatusV1::Pending => true,
                        BackfillEntryStatusV1::Leased { lease, .. } => {
                            journal.attempts > 0
                                && lease.fencing_token > 0
                                && lease.fencing_token < self.next_fencing_token
                        }
                        BackfillEntryStatusV1::RetryScheduled {
                            failure,
                            not_before,
                        } => {
                            journal.attempts > 0
                                && failure.retryable()
                                && *not_before >= manifest.created_at
                        }
                        BackfillEntryStatusV1::Succeeded { receipt } => {
                            journal.attempts > 0
                                && receipt.entry_id == journal.entry_id
                                && receipt.logical_bytes == manifest_entry.expected_size
                                && receipt.strong_sha256 == manifest_entry.strong_sha256
                                && receipt.committed_at >= manifest.created_at
                                && receipt.committed_at <= self.last_transition_at
                                && receipt.media_probe_profile_version
                                    == manifest_entry
                                        .media_probe
                                        .required()
                                        .then_some(manifest_entry.media_probe.profile_version)
                        }
                        BackfillEntryStatusV1::Quarantined {
                            disposition,
                            approval,
                            ..
                        } => {
                            journal.attempts > 0
                                && match disposition {
                                    BackfillOwnerDispositionV1::PendingOwnerApproval => {
                                        approval.is_none()
                                    }
                                    BackfillOwnerDispositionV1::RetryApproved => false,
                                    BackfillOwnerDispositionV1::ReferenceApproved
                                    | BackfillOwnerDispositionV1::ExcludeApproved => {
                                        approval.as_ref().is_some_and(|approval| {
                                            let scope = backfill_disposition_approval_scope_v1(
                                                &self.manifest_digest,
                                                journal.entry_id,
                                                *disposition,
                                            );
                                            approval.validate_record(&scope).is_ok()
                                                && approval.verified_at >= manifest.created_at
                                                && approval.verified_at <= self.last_transition_at
                                        })
                                    }
                                }
                        }
                    }
                });
        let terminal = self.entries.iter().all(|entry| match &entry.status {
            BackfillEntryStatusV1::Succeeded { .. } => true,
            BackfillEntryStatusV1::Quarantined { disposition, .. } => matches!(
                disposition,
                BackfillOwnerDispositionV1::ReferenceApproved
                    | BackfillOwnerDispositionV1::ExcludeApproved
            ),
            _ => false,
        });
        let active_leases = self
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    BackfillEntryStatusV1::Leased { lease, .. } if !lease.expired_at(now)
                )
            })
            .count();
        let half_open_valid = self.circuit.half_open_fencing_token.is_none_or(|token| {
            self.entries.iter().any(|entry| {
                matches!(
                    entry.status,
                    BackfillEntryStatusV1::Leased { lease, .. } if lease.fencing_token == token
                )
            })
        });
        let retention_valid = match &self.source_retention {
            BackfillSourceRetentionStateV1::Retained => true,
            BackfillSourceRetentionStateV1::ReleaseApproved {
                approval,
                reconciliation_digest,
                approved_at,
            } => {
                let scope = backfill_source_release_approval_scope_v1(
                    &self.manifest_digest,
                    reconciliation_digest,
                );
                self.state == BackfillRunStateV1::Completed
                    && *approved_at >= manifest.created_at
                    && *approved_at <= self.last_transition_at
                    && approval.verified_at == *approved_at
                    && approval.validate_record(&scope).is_ok()
            }
        };
        if self.protocol_version != OBJECT_BACKFILL_PROTOCOL_VERSION_V1
            || self.schema_version != OBJECT_BACKFILL_JOURNAL_SCHEMA_VERSION_V1
            || self.manifest_id != manifest.manifest_id
            || self.manifest_digest != manifest.digest
            || self.entries.len() != manifest.entries.len()
            || !entries_valid
            || self.next_fencing_token == 0
            || self.last_transition_at < manifest.created_at
            || self.last_transition_at > now
            || self.revision > MAX_SAFE_INTEGER
            || self.usage.admitted_entries > MAX_SAFE_INTEGER
            || self.usage.admitted_logical_bytes > MAX_SAFE_INTEGER
            || self.usage.admitted_cost_units > MAX_SAFE_INTEGER
            || self.usage.admitted_entries > manifest.execution_policy.max_entries_per_run
            || self.usage.admitted_logical_bytes
                > manifest.execution_policy.max_logical_bytes_per_run
            || self.usage.admitted_cost_units > manifest.execution_policy.max_cost_units_per_run
            || active_leases > usize::from(manifest.execution_policy.max_concurrency)
            || !half_open_valid
            || (self.state == BackfillRunStateV1::Completed && !terminal)
            || !retention_valid
        {
            return Err(BackfillContractErrorV1::InvalidJournal);
        }
        Ok(())
    }

    #[must_use]
    pub const fn manifest_id(&self) -> BackfillManifestIdV1 {
        self.manifest_id
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ChecksumSha256 {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn last_transition_at(&self) -> TimestampMillis {
        self.last_transition_at
    }

    #[must_use]
    pub const fn state(&self) -> BackfillRunStateV1 {
        self.state
    }

    #[must_use]
    pub fn entries(&self) -> &[BackfillJournalEntryV1] {
        &self.entries
    }

    #[must_use]
    pub fn entry(&self, entry_id: BackfillEntryIdV1) -> Option<&BackfillJournalEntryV1> {
        self.entries.iter().find(|entry| entry.entry_id == entry_id)
    }

    #[must_use]
    pub const fn usage(&self) -> BackfillUsageV1 {
        self.usage
    }

    #[must_use]
    pub const fn circuit(&self) -> BackfillCircuitStateV1 {
        self.circuit
    }

    #[must_use]
    pub const fn source_retention(&self) -> &BackfillSourceRetentionStateV1 {
        &self.source_retention
    }

    #[must_use]
    pub fn next_eligible_entry(&self, now: TimestampMillis) -> Option<BackfillEntryIdV1> {
        (self.state == BackfillRunStateV1::Running && !self.circuit.is_open_at(now))
            .then(|| {
                self.entries
                    .iter()
                    .find(|entry| entry.eligible_at(now))
                    .map(|entry| entry.entry_id)
            })
            .flatten()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim(
        &mut self,
        entry_id: BackfillEntryIdV1,
        worker_id: BackfillWorkerIdV1,
        operation_id: BackfillOperationIdV1,
        now: TimestampMillis,
        policy: BackfillExecutionPolicyV1,
        logical_bytes: ByteSize,
        cost_units: u64,
    ) -> Result<BackfillLeaseV1, BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        let active_leases = self
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    BackfillEntryStatusV1::Leased { lease, .. } if !lease.expired_at(now)
                )
            })
            .count();
        let entering_half_open = self.circuit.open_until.is_some_and(|until| until <= now);
        if self.state != BackfillRunStateV1::Running
            || self.circuit.is_open_at(now)
            || self.circuit.half_open_fencing_token.is_some()
            || active_leases >= usize::from(policy.max_concurrency)
            || cost_units == 0
            || self.usage.admitted_entries >= policy.max_entries_per_run
            || self
                .usage
                .admitted_logical_bytes
                .saturating_add(logical_bytes.get())
                > policy.max_logical_bytes_per_run
            || self.usage.admitted_cost_units.saturating_add(cost_units)
                > policy.max_cost_units_per_run
        {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(BackfillContractErrorV1::InvalidTransition)?;
        let claimable_status = matches!(
            entry.status,
            BackfillEntryStatusV1::Pending | BackfillEntryStatusV1::RetryScheduled { .. }
        );
        let normal_attempt = entry.attempts < policy.max_attempts;
        let approved_extra_attempt =
            !normal_attempt && entry.retry_approval.is_some() && !entry.retry_approval_consumed;
        if !claimable_status
            || !entry.eligible_at(now)
            || (!normal_attempt && !approved_extra_attempt)
        {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        let lease = BackfillLeaseV1 {
            worker_id,
            fencing_token: self.next_fencing_token,
            expires_at: now
                .checked_add(policy.lease_ttl)
                .map_err(|_| BackfillContractErrorV1::InvalidTransition)?,
        };
        self.next_fencing_token = self
            .next_fencing_token
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE_INTEGER)
            .ok_or(BackfillContractErrorV1::InvalidTransition)?;
        if approved_extra_attempt {
            entry.retry_approval_consumed = true;
        }
        entry.attempts = entry.attempts.saturating_add(1);
        entry.status = BackfillEntryStatusV1::Leased {
            lease,
            operation_id,
        };
        self.usage.admitted_entries = self.usage.admitted_entries.saturating_add(1);
        self.usage.admitted_logical_bytes = self
            .usage
            .admitted_logical_bytes
            .saturating_add(logical_bytes.get());
        self.usage.admitted_cost_units = self.usage.admitted_cost_units.saturating_add(cost_units);
        if entering_half_open {
            self.circuit.half_open_fencing_token = Some(lease.fencing_token);
        }
        self.finish_transition(now)?;
        Ok(lease)
    }

    pub fn renew_lease(
        &mut self,
        entry_id: BackfillEntryIdV1,
        lease: BackfillLeaseV1,
        now: TimestampMillis,
        lease_ttl: DurationMillis,
    ) -> Result<BackfillLeaseV1, BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        if self.state != BackfillRunStateV1::Running {
            return Err(BackfillContractErrorV1::StaleLease);
        }
        let entry = self.leased_entry_mut(entry_id, lease, now)?;
        let BackfillEntryStatusV1::Leased { lease: current, .. } = &mut entry.status else {
            return Err(BackfillContractErrorV1::StaleLease);
        };
        current.expires_at = now
            .checked_add(lease_ttl)
            .map_err(|_| BackfillContractErrorV1::InvalidTransition)?;
        let renewed = *current;
        self.finish_transition(now)?;
        Ok(renewed)
    }

    #[must_use]
    pub fn publication_allowed(
        &self,
        entry_id: BackfillEntryIdV1,
        operation_id: BackfillOperationIdV1,
        lease: BackfillLeaseV1,
        now: TimestampMillis,
    ) -> bool {
        if self.ensure_monotonic(now).is_err() || self.state != BackfillRunStateV1::Running {
            return false;
        }
        self.entry(entry_id).is_some_and(|entry| {
            matches!(
                entry.status,
                BackfillEntryStatusV1::Leased {
                    lease: current,
                    operation_id: current_operation,
                } if current.same_fence(lease)
                    && current_operation == operation_id
                    && !current.expired_at(now)
            )
        })
    }

    pub fn complete(
        &mut self,
        entry_id: BackfillEntryIdV1,
        lease: BackfillLeaseV1,
        receipt: BackfillOperationReceiptV1,
        now: TimestampMillis,
    ) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        if matches!(
            self.state,
            BackfillRunStateV1::Aborting | BackfillRunStateV1::Aborted
        ) {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        let entry = self.leased_entry_mut(entry_id, lease, now)?;
        let BackfillEntryStatusV1::Leased { operation_id, .. } = entry.status else {
            return Err(BackfillContractErrorV1::StaleLease);
        };
        if receipt.entry_id != entry_id || receipt.operation_id != operation_id {
            return Err(BackfillContractErrorV1::InvalidJournal);
        }
        entry.last_failure = None;
        entry.status = BackfillEntryStatusV1::Succeeded { receipt };
        self.circuit = BackfillCircuitStateV1::default();
        self.finish_transition(now)?;
        self.refresh_completion();
        Ok(())
    }

    pub fn recover_committed(
        &mut self,
        entry_id: BackfillEntryIdV1,
        operation_id: BackfillOperationIdV1,
        receipt: BackfillOperationReceiptV1,
        now: TimestampMillis,
    ) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        if self.state != BackfillRunStateV1::Running {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(BackfillContractErrorV1::InvalidTransition)?;
        let BackfillEntryStatusV1::Leased {
            lease,
            operation_id: current_operation,
        } = entry.status
        else {
            return Err(BackfillContractErrorV1::InvalidTransition);
        };
        if !lease.expired_at(now)
            || current_operation != operation_id
            || receipt.entry_id != entry_id
            || receipt.operation_id != operation_id
        {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        entry.last_failure = None;
        entry.status = BackfillEntryStatusV1::Succeeded { receipt };
        self.circuit = BackfillCircuitStateV1::default();
        self.finish_transition(now)?;
        self.refresh_completion();
        Ok(())
    }

    pub fn normalize_expired_lease(
        &mut self,
        entry_id: BackfillEntryIdV1,
        now: TimestampMillis,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(BackfillContractErrorV1::InvalidTransition)?;
        let BackfillEntryStatusV1::Leased { lease, .. } = entry.status else {
            return Err(BackfillContractErrorV1::InvalidTransition);
        };
        if !lease.expired_at(now) {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        entry.last_failure = Some(BackfillFailureClassV1::TransferCanceled);
        let retry_available = entry.attempts < policy.max_attempts
            || (entry.retry_approval.is_some() && !entry.retry_approval_consumed);
        entry.status = if retry_available {
            BackfillEntryStatusV1::RetryScheduled {
                failure: BackfillFailureClassV1::TransferCanceled,
                not_before: now,
            }
        } else {
            BackfillEntryStatusV1::Quarantined {
                failure: BackfillFailureClassV1::TransferCanceled,
                disposition: BackfillOwnerDispositionV1::PendingOwnerApproval,
                approval: None,
            }
        };
        if self.circuit.half_open_fencing_token == Some(lease.fencing_token) {
            self.circuit = BackfillCircuitStateV1::default();
        }
        self.finish_transition(now)
    }

    pub fn fail(
        &mut self,
        entry_id: BackfillEntryIdV1,
        lease: BackfillLeaseV1,
        failure: BackfillFailureClassV1,
        now: TimestampMillis,
        policy: BackfillExecutionPolicyV1,
    ) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        let attempt = {
            let entry = self.leased_entry_mut(entry_id, lease, now)?;
            entry.attempts
        };
        if failure.retryable() && attempt < policy.max_attempts {
            let delay = policy.backoff_for_attempt(attempt);
            let not_before = now
                .checked_add(delay)
                .map_err(|_| BackfillContractErrorV1::InvalidTransition)?;
            let entry = self
                .entries
                .iter_mut()
                .find(|entry| entry.entry_id == entry_id)
                .ok_or(BackfillContractErrorV1::InvalidTransition)?;
            entry.last_failure = Some(failure);
            entry.status = BackfillEntryStatusV1::RetryScheduled {
                failure,
                not_before,
            };
        } else {
            let entry = self
                .entries
                .iter_mut()
                .find(|entry| entry.entry_id == entry_id)
                .ok_or(BackfillContractErrorV1::InvalidTransition)?;
            entry.last_failure = Some(failure);
            entry.status = BackfillEntryStatusV1::Quarantined {
                failure,
                disposition: BackfillOwnerDispositionV1::PendingOwnerApproval,
                approval: None,
            };
        }
        let provider_failure = matches!(
            failure,
            BackfillFailureClassV1::ProviderThrottled
                | BackfillFailureClassV1::ProviderExpiredAuthorization
                | BackfillFailureClassV1::ProviderOutage
        );
        if provider_failure {
            self.circuit.consecutive_provider_failures =
                self.circuit.consecutive_provider_failures.saturating_add(1);
            if self.circuit.half_open_fencing_token == Some(lease.fencing_token)
                || self.circuit.consecutive_provider_failures >= policy.circuit_failure_threshold
            {
                self.circuit.open_until = Some(
                    now.checked_add(policy.circuit_cooldown)
                        .map_err(|_| BackfillContractErrorV1::InvalidTransition)?,
                );
                self.circuit.half_open_fencing_token = None;
            }
        } else {
            self.circuit = BackfillCircuitStateV1::default();
        }
        self.finish_transition(now)?;
        Ok(())
    }

    pub fn pause(&mut self, now: TimestampMillis) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        if self.state != BackfillRunStateV1::Running {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        self.state = BackfillRunStateV1::Paused;
        self.finish_transition(now)
    }

    pub fn resume(&mut self, now: TimestampMillis) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        if self.state != BackfillRunStateV1::Paused {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        self.state = BackfillRunStateV1::Running;
        self.finish_transition(now)
    }

    pub fn abort(&mut self, now: TimestampMillis) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        if !matches!(
            self.state,
            BackfillRunStateV1::Running | BackfillRunStateV1::Paused
        ) {
            return Err(BackfillContractErrorV1::InvalidTransition);
        }
        self.state = BackfillRunStateV1::Aborting;
        for entry in &mut self.entries {
            if matches!(entry.status, BackfillEntryStatusV1::Leased { .. }) {
                entry.status = BackfillEntryStatusV1::Quarantined {
                    failure: BackfillFailureClassV1::TransferCanceled,
                    disposition: BackfillOwnerDispositionV1::PendingOwnerApproval,
                    approval: None,
                };
                entry.last_failure = Some(BackfillFailureClassV1::TransferCanceled);
            }
        }
        self.circuit.half_open_fencing_token = None;
        self.state = BackfillRunStateV1::Aborted;
        self.finish_transition(now)
    }

    pub fn approve_disposition(
        &mut self,
        entry_id: BackfillEntryIdV1,
        disposition: BackfillOwnerDispositionV1,
        approval: BackfillOwnerApprovalRecordV1,
        now: TimestampMillis,
    ) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        if disposition == BackfillOwnerDispositionV1::PendingOwnerApproval {
            return Err(BackfillContractErrorV1::InvalidDisposition);
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(BackfillContractErrorV1::InvalidDisposition)?;
        let BackfillEntryStatusV1::Quarantined {
            failure,
            disposition: BackfillOwnerDispositionV1::PendingOwnerApproval,
            approval: None,
        } = entry.status
        else {
            return Err(BackfillContractErrorV1::InvalidDisposition);
        };
        let scope =
            backfill_disposition_approval_scope_v1(&self.manifest_digest, entry_id, disposition);
        approval.validate_at(&scope, now)?;
        if approval.verified_at != now {
            return Err(BackfillContractErrorV1::InvalidApproval);
        }
        entry.status = if disposition == BackfillOwnerDispositionV1::RetryApproved {
            if entry.retry_approval.is_some() {
                return Err(BackfillContractErrorV1::InvalidDisposition);
            }
            entry.retry_approval = Some(approval);
            BackfillEntryStatusV1::Pending
        } else {
            BackfillEntryStatusV1::Quarantined {
                failure,
                disposition,
                approval: Some(approval),
            }
        };
        self.finish_transition(now)?;
        self.refresh_completion();
        Ok(())
    }

    pub fn approve_source_release(
        &mut self,
        manifest: &ObjectBackfillManifestV1,
        report: &ObjectBackfillReconciliationReportV1,
        approval: BackfillOwnerApprovalRecordV1,
        now: TimestampMillis,
        maximum_report_age: DurationMillis,
    ) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        report.validate_integrity(manifest, now, maximum_report_age)?;
        let scope = backfill_source_release_approval_scope_v1(
            &self.manifest_digest,
            report.report_digest(),
        );
        approval.validate_at(&scope, now)?;
        if approval.verified_at != now {
            return Err(BackfillContractErrorV1::InvalidApproval);
        }
        let dispositions = self.reconciliation_dispositions();
        if self.state != BackfillRunStateV1::Completed
            || !report.clean
            || report.manifest_digest != self.manifest_digest
            || report.dispositions != dispositions
            || !report.discrepancies.is_empty()
            || !matches!(
                self.source_retention,
                BackfillSourceRetentionStateV1::Retained
            )
        {
            return Err(BackfillContractErrorV1::RetentionBlocked);
        }
        self.source_retention = BackfillSourceRetentionStateV1::ReleaseApproved {
            approval,
            reconciliation_digest: report.report_digest.clone(),
            approved_at: now,
        };
        self.finish_transition(now)
    }

    fn reconciliation_dispositions(&self) -> Vec<BackfillReconciliationDispositionV1> {
        let mut dispositions = self
            .entries
            .iter()
            .filter_map(|entry| match &entry.status {
                BackfillEntryStatusV1::Quarantined {
                    failure,
                    disposition:
                        disposition @ (BackfillOwnerDispositionV1::ReferenceApproved
                        | BackfillOwnerDispositionV1::ExcludeApproved),
                    approval: Some(approval),
                } => Some(BackfillReconciliationDispositionV1 {
                    entry_id: entry.entry_id,
                    failure: *failure,
                    disposition: *disposition,
                    approval: approval.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        dispositions.sort_by(|left, right| {
            left.entry_id
                .to_string()
                .cmp(&right.entry_id.to_string())
                .then_with(|| left.disposition.tag().cmp(right.disposition.tag()))
        });
        dispositions
    }

    fn leased_entry_mut(
        &mut self,
        entry_id: BackfillEntryIdV1,
        lease: BackfillLeaseV1,
        now: TimestampMillis,
    ) -> Result<&mut BackfillJournalEntryV1, BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.entry_id == entry_id)
            .ok_or(BackfillContractErrorV1::StaleLease)?;
        let BackfillEntryStatusV1::Leased { lease: current, .. } = entry.status else {
            return Err(BackfillContractErrorV1::StaleLease);
        };
        if !current.same_fence(lease) || current.expired_at(now) {
            return Err(BackfillContractErrorV1::StaleLease);
        }
        Ok(entry)
    }

    fn refresh_completion(&mut self) {
        let terminal = self.entries.iter().all(|entry| match &entry.status {
            BackfillEntryStatusV1::Succeeded { .. } => true,
            BackfillEntryStatusV1::Quarantined { disposition, .. } => matches!(
                disposition,
                BackfillOwnerDispositionV1::ReferenceApproved
                    | BackfillOwnerDispositionV1::ExcludeApproved
            ),
            _ => false,
        });
        if terminal && !matches!(self.state, BackfillRunStateV1::Aborted) {
            self.state = BackfillRunStateV1::Completed;
        }
    }

    fn ensure_monotonic(&self, now: TimestampMillis) -> Result<(), BackfillContractErrorV1> {
        if now < self.last_transition_at {
            return Err(BackfillContractErrorV1::ClockRollback);
        }
        Ok(())
    }

    fn finish_transition(&mut self, now: TimestampMillis) -> Result<(), BackfillContractErrorV1> {
        self.ensure_monotonic(now)?;
        self.last_transition_at = now;
        self.bump_revision()
    }

    fn bump_revision(&mut self) -> Result<(), BackfillContractErrorV1> {
        self.revision = self
            .revision
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE_INTEGER)
            .ok_or(BackfillContractErrorV1::InvalidTransition)?;
        Ok(())
    }
}

impl fmt::Debug for ObjectBackfillJournalV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectBackfillJournalV1")
            .field("protocol_version", &self.protocol_version)
            .field("schema_version", &self.schema_version)
            .field("manifest_id", &self.manifest_id)
            .field("manifest_digest", &"[redacted]")
            .field("revision", &self.revision)
            .field("last_transition_at", &self.last_transition_at)
            .field("state", &self.state)
            .field("entries", &self.entries)
            .field("usage", &self.usage)
            .field("circuit", &self.circuit)
            .field("source_retention", &self.source_retention)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillInventorySideV1 {
    Source,
    Target,
}

impl BackfillInventorySideV1 {
    const fn tag(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Target => "target",
        }
    }
}

/// Stable, redacted identity for one provider object/version in reconciliation evidence.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BackfillObjectFingerprintV1(ChecksumSha256);

impl BackfillObjectFingerprintV1 {
    #[must_use]
    pub fn derive(
        authority_fingerprint: &ChecksumSha256,
        side: BackfillInventorySideV1,
        location: &str,
        version: Option<&str>,
    ) -> Self {
        let mut bytes = Vec::new();
        frame(&mut bytes, b"object-backfill-object-fingerprint-v1");
        frame(&mut bytes, authority_fingerprint.as_str().as_bytes());
        frame(&mut bytes, side.tag().as_bytes());
        frame(&mut bytes, location.as_bytes());
        frame(&mut bytes, version.unwrap_or("").as_bytes());
        Self(ChecksumSha256::digest_bytes(&bytes))
    }

    #[must_use]
    pub const fn as_checksum(&self) -> &ChecksumSha256 {
        &self.0
    }
}

impl fmt::Debug for BackfillObjectFingerprintV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillObjectFingerprintV1([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillDiscrepancyKindV1 {
    MissingSource,
    MissingTarget,
    DuplicateSource,
    DuplicateTarget,
    OrphanSource,
    OrphanTarget,
    OwnershipMismatch,
    CorruptSource,
    CorruptTarget,
    UnplayableSource,
    UnplayableTarget,
    MetadataDivergence,
    CheckpointDivergence,
    ProviderUnavailable,
}

impl BackfillDiscrepancyKindV1 {
    const fn tag(self) -> &'static str {
        match self {
            Self::MissingSource => "missing_source",
            Self::MissingTarget => "missing_target",
            Self::DuplicateSource => "duplicate_source",
            Self::DuplicateTarget => "duplicate_target",
            Self::OrphanSource => "orphan_source",
            Self::OrphanTarget => "orphan_target",
            Self::OwnershipMismatch => "ownership_mismatch",
            Self::CorruptSource => "corrupt_source",
            Self::CorruptTarget => "corrupt_target",
            Self::UnplayableSource => "unplayable_source",
            Self::UnplayableTarget => "unplayable_target",
            Self::MetadataDivergence => "metadata_divergence",
            Self::CheckpointDivergence => "checkpoint_divergence",
            Self::ProviderUnavailable => "provider_unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillDiscrepancyV1 {
    entry_id: Option<BackfillEntryIdV1>,
    side: BackfillInventorySideV1,
    kind: BackfillDiscrepancyKindV1,
    object_fingerprint: BackfillObjectFingerprintV1,
}

impl BackfillDiscrepancyV1 {
    #[must_use]
    pub const fn new(
        entry_id: Option<BackfillEntryIdV1>,
        side: BackfillInventorySideV1,
        kind: BackfillDiscrepancyKindV1,
        object_fingerprint: BackfillObjectFingerprintV1,
    ) -> Self {
        Self {
            entry_id,
            side,
            kind,
            object_fingerprint,
        }
    }

    #[must_use]
    pub const fn entry_id(&self) -> Option<BackfillEntryIdV1> {
        self.entry_id
    }

    #[must_use]
    pub const fn side(&self) -> BackfillInventorySideV1 {
        self.side
    }

    #[must_use]
    pub const fn kind(&self) -> BackfillDiscrepancyKindV1 {
        self.kind
    }

    #[must_use]
    pub const fn object_fingerprint(&self) -> &BackfillObjectFingerprintV1 {
        &self.object_fingerprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillInventoryTotalsV1 {
    object_count: u64,
    logical_bytes: u64,
    role_counts: [u64; ROLE_COUNT],
    strong_checksums_verified: u64,
    media_probes_verified: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillReconciliationDispositionV1 {
    entry_id: BackfillEntryIdV1,
    failure: BackfillFailureClassV1,
    disposition: BackfillOwnerDispositionV1,
    approval: BackfillOwnerApprovalRecordV1,
}

impl BackfillReconciliationDispositionV1 {
    pub fn new(
        entry_id: BackfillEntryIdV1,
        failure: BackfillFailureClassV1,
        disposition: BackfillOwnerDispositionV1,
        approval: BackfillOwnerApprovalRecordV1,
    ) -> Result<Self, BackfillContractErrorV1> {
        if !matches!(
            disposition,
            BackfillOwnerDispositionV1::ReferenceApproved
                | BackfillOwnerDispositionV1::ExcludeApproved
        ) {
            return Err(BackfillContractErrorV1::InvalidDisposition);
        }
        Ok(Self {
            entry_id,
            failure,
            disposition,
            approval,
        })
    }

    #[must_use]
    pub const fn entry_id(&self) -> BackfillEntryIdV1 {
        self.entry_id
    }

    #[must_use]
    pub const fn failure(&self) -> BackfillFailureClassV1 {
        self.failure
    }

    #[must_use]
    pub const fn disposition(&self) -> BackfillOwnerDispositionV1 {
        self.disposition
    }

    #[must_use]
    pub const fn approval(&self) -> &BackfillOwnerApprovalRecordV1 {
        &self.approval
    }
}

impl fmt::Debug for BackfillReconciliationDispositionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillReconciliationDispositionV1")
            .field("entry_id", &self.entry_id)
            .field("failure", &self.failure)
            .field("disposition", &self.disposition)
            .field("approval", &self.approval)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillDispositionTotalsV1 {
    referenced_objects: u64,
    referenced_logical_bytes: u64,
    referenced_role_counts: [u64; ROLE_COUNT],
    excluded_objects: u64,
    excluded_logical_bytes: u64,
    excluded_role_counts: [u64; ROLE_COUNT],
}

impl BackfillDispositionTotalsV1 {
    #[must_use]
    pub const fn referenced_objects(self) -> u64 {
        self.referenced_objects
    }

    #[must_use]
    pub const fn referenced_logical_bytes(self) -> u64 {
        self.referenced_logical_bytes
    }

    #[must_use]
    pub const fn referenced_role_count(self, role: ObjectRole) -> u64 {
        self.referenced_role_counts[role_index(role)]
    }

    #[must_use]
    pub const fn excluded_objects(self) -> u64 {
        self.excluded_objects
    }

    #[must_use]
    pub const fn excluded_logical_bytes(self) -> u64 {
        self.excluded_logical_bytes
    }

    #[must_use]
    pub const fn excluded_role_count(self, role: ObjectRole) -> u64 {
        self.excluded_role_counts[role_index(role)]
    }

    fn add(
        &mut self,
        entry: &ObjectBackfillManifestEntryV1,
        disposition: BackfillOwnerDispositionV1,
    ) {
        match disposition {
            BackfillOwnerDispositionV1::ReferenceApproved => {
                self.referenced_objects = self.referenced_objects.saturating_add(1);
                self.referenced_logical_bytes = self
                    .referenced_logical_bytes
                    .saturating_add(entry.expected_size.get());
                self.referenced_role_counts[role_index(entry.role)] =
                    self.referenced_role_counts[role_index(entry.role)].saturating_add(1);
            }
            BackfillOwnerDispositionV1::ExcludeApproved => {
                self.excluded_objects = self.excluded_objects.saturating_add(1);
                self.excluded_logical_bytes = self
                    .excluded_logical_bytes
                    .saturating_add(entry.expected_size.get());
                self.excluded_role_counts[role_index(entry.role)] =
                    self.excluded_role_counts[role_index(entry.role)].saturating_add(1);
            }
            BackfillOwnerDispositionV1::PendingOwnerApproval
            | BackfillOwnerDispositionV1::RetryApproved => {}
        }
    }

    fn valid(self) -> bool {
        let referenced_roles = self
            .referenced_role_counts
            .iter()
            .try_fold(0_u64, |sum, count| sum.checked_add(*count));
        let excluded_roles = self
            .excluded_role_counts
            .iter()
            .try_fold(0_u64, |sum, count| sum.checked_add(*count));
        self.referenced_objects <= MAX_SAFE_INTEGER
            && self.referenced_logical_bytes <= MAX_SAFE_INTEGER
            && self.excluded_objects <= MAX_SAFE_INTEGER
            && self.excluded_logical_bytes <= MAX_SAFE_INTEGER
            && referenced_roles == Some(self.referenced_objects)
            && excluded_roles == Some(self.excluded_objects)
    }
}

impl BackfillInventoryTotalsV1 {
    #[must_use]
    pub const fn new(
        object_count: u64,
        logical_bytes: u64,
        role_counts: [u64; ROLE_COUNT],
        strong_checksums_verified: u64,
        media_probes_verified: u64,
    ) -> Self {
        Self {
            object_count,
            logical_bytes,
            role_counts,
            strong_checksums_verified,
            media_probes_verified,
        }
    }

    #[must_use]
    pub const fn object_count(self) -> u64 {
        self.object_count
    }

    #[must_use]
    pub const fn logical_bytes(self) -> u64 {
        self.logical_bytes
    }

    #[must_use]
    pub const fn role_count(self, role: ObjectRole) -> u64 {
        self.role_counts[role_index(role)]
    }

    #[must_use]
    pub const fn strong_checksums_verified(self) -> u64 {
        self.strong_checksums_verified
    }

    #[must_use]
    pub const fn media_probes_verified(self) -> u64 {
        self.media_probes_verified
    }
}

const fn role_index(role: ObjectRole) -> usize {
    match role {
        ObjectRole::Source => 0,
        ObjectRole::RecordingSegment => 1,
        ObjectRole::Thumbnail => 2,
        ObjectRole::Screenshot => 3,
        ObjectRole::Preview => 4,
        ObjectRole::Spritesheet => 5,
        ObjectRole::Audio => 6,
        ObjectRole::Caption => 7,
        ObjectRole::Export => 8,
        ObjectRole::Manifest => 9,
    }
}

#[cfg(test)]
fn expected_reconciliation_totals(
    manifest: &ObjectBackfillManifestV1,
) -> BackfillInventoryTotalsV1 {
    let mut object_count = 0_u64;
    let mut logical_bytes = 0_u64;
    let mut role_counts = [0_u64; ROLE_COUNT];
    let mut media_probes_verified = 0_u64;
    for entry in &manifest.entries {
        object_count = object_count.saturating_add(1);
        logical_bytes = logical_bytes.saturating_add(entry.expected_size.get());
        role_counts[role_index(entry.role)] = role_counts[role_index(entry.role)].saturating_add(1);
        if entry.media_probe.required() {
            media_probes_verified = media_probes_verified.saturating_add(1);
        }
    }
    BackfillInventoryTotalsV1::new(
        object_count,
        logical_bytes,
        role_counts,
        0,
        media_probes_verified,
    )
}

fn add_expected_reconciliation_entry(
    totals: &mut BackfillInventoryTotalsV1,
    entry: &ObjectBackfillManifestEntryV1,
) {
    totals.object_count = totals.object_count.saturating_add(1);
    totals.logical_bytes = totals
        .logical_bytes
        .saturating_add(entry.expected_size.get());
    totals.role_counts[role_index(entry.role)] =
        totals.role_counts[role_index(entry.role)].saturating_add(1);
    if entry.media_probe.required() {
        totals.media_probes_verified = totals.media_probes_verified.saturating_add(1);
    }
}

fn reconciliation_expectations(
    manifest: &ObjectBackfillManifestV1,
    dispositions: &[BackfillReconciliationDispositionV1],
    generated_at: TimestampMillis,
) -> Result<
    (
        BackfillInventoryTotalsV1,
        BackfillInventoryTotalsV1,
        BackfillDispositionTotalsV1,
    ),
    BackfillContractErrorV1,
> {
    let mut by_entry = HashMap::with_capacity(dispositions.len());
    for disposition in dispositions {
        let entry = manifest
            .entry(disposition.entry_id)
            .ok_or(BackfillContractErrorV1::InvalidReport)?;
        if by_entry
            .insert(disposition.entry_id, disposition.disposition)
            .is_some()
            || !matches!(
                disposition.disposition,
                BackfillOwnerDispositionV1::ReferenceApproved
                    | BackfillOwnerDispositionV1::ExcludeApproved
            )
        {
            return Err(BackfillContractErrorV1::InvalidReport);
        }
        let scope = backfill_disposition_approval_scope_v1(
            manifest.digest(),
            disposition.entry_id,
            disposition.disposition,
        );
        disposition
            .approval
            .validate_record(&scope)
            .map_err(|_| BackfillContractErrorV1::InvalidReport)?;
        if disposition.approval.verified_at < manifest.created_at
            || disposition.approval.verified_at > generated_at
            || entry.entry_id != disposition.entry_id
        {
            return Err(BackfillContractErrorV1::InvalidReport);
        }
    }

    let mut expected_source = BackfillInventoryTotalsV1::default();
    let mut expected_target = BackfillInventoryTotalsV1::default();
    let mut disposition_totals = BackfillDispositionTotalsV1::default();
    for entry in manifest.entries() {
        match by_entry.get(&entry.entry_id()).copied() {
            Some(BackfillOwnerDispositionV1::ReferenceApproved) => {
                add_expected_reconciliation_entry(&mut expected_source, entry);
                disposition_totals.add(entry, BackfillOwnerDispositionV1::ReferenceApproved);
            }
            Some(BackfillOwnerDispositionV1::ExcludeApproved) => {
                disposition_totals.add(entry, BackfillOwnerDispositionV1::ExcludeApproved);
            }
            Some(
                BackfillOwnerDispositionV1::PendingOwnerApproval
                | BackfillOwnerDispositionV1::RetryApproved,
            ) => return Err(BackfillContractErrorV1::InvalidReport),
            None => {
                add_expected_reconciliation_entry(&mut expected_source, entry);
                add_expected_reconciliation_entry(&mut expected_target, entry);
            }
        }
    }
    if !totals_are_bounded(expected_source)
        || !totals_are_bounded(expected_target)
        || !disposition_totals.valid()
    {
        return Err(BackfillContractErrorV1::InvalidReport);
    }
    Ok((expected_source, expected_target, disposition_totals))
}

fn totals_are_bounded(totals: BackfillInventoryTotalsV1) -> bool {
    totals.object_count <= MAX_SAFE_INTEGER
        && totals.logical_bytes <= MAX_SAFE_INTEGER
        && totals
            .role_counts
            .iter()
            .all(|count| *count <= totals.object_count)
        && totals.strong_checksums_verified <= totals.object_count
        && totals.media_probes_verified <= totals.object_count
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectBackfillReconciliationReportV1 {
    protocol_version: u16,
    manifest_digest: ChecksumSha256,
    generated_at: TimestampMillis,
    expected_source: BackfillInventoryTotalsV1,
    expected_target: BackfillInventoryTotalsV1,
    source: BackfillInventoryTotalsV1,
    target: BackfillInventoryTotalsV1,
    dispositions: Vec<BackfillReconciliationDispositionV1>,
    disposition_totals: BackfillDispositionTotalsV1,
    discrepancies: Vec<BackfillDiscrepancyV1>,
    clean: bool,
    report_digest: ChecksumSha256,
}

impl ObjectBackfillReconciliationReportV1 {
    #[must_use]
    pub fn new(
        manifest_digest: ChecksumSha256,
        generated_at: TimestampMillis,
        expected_source: BackfillInventoryTotalsV1,
        expected_target: BackfillInventoryTotalsV1,
        source: BackfillInventoryTotalsV1,
        target: BackfillInventoryTotalsV1,
        discrepancies: Vec<BackfillDiscrepancyV1>,
    ) -> Self {
        Self::from_parts(
            manifest_digest,
            generated_at,
            expected_source,
            expected_target,
            source,
            target,
            vec![],
            BackfillDispositionTotalsV1::default(),
            discrepancies,
        )
    }

    pub fn new_with_dispositions(
        manifest: &ObjectBackfillManifestV1,
        generated_at: TimestampMillis,
        source: BackfillInventoryTotalsV1,
        target: BackfillInventoryTotalsV1,
        dispositions: Vec<BackfillReconciliationDispositionV1>,
        discrepancies: Vec<BackfillDiscrepancyV1>,
    ) -> Result<Self, BackfillContractErrorV1> {
        let (expected_source, expected_target, disposition_totals) =
            reconciliation_expectations(manifest, &dispositions, generated_at)?;
        Ok(Self::from_parts(
            manifest.digest().clone(),
            generated_at,
            expected_source,
            expected_target,
            source,
            target,
            dispositions,
            disposition_totals,
            discrepancies,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn from_parts(
        manifest_digest: ChecksumSha256,
        generated_at: TimestampMillis,
        expected_source: BackfillInventoryTotalsV1,
        expected_target: BackfillInventoryTotalsV1,
        source: BackfillInventoryTotalsV1,
        target: BackfillInventoryTotalsV1,
        mut dispositions: Vec<BackfillReconciliationDispositionV1>,
        disposition_totals: BackfillDispositionTotalsV1,
        mut discrepancies: Vec<BackfillDiscrepancyV1>,
    ) -> Self {
        dispositions.sort_by(|left, right| {
            left.entry_id
                .to_string()
                .cmp(&right.entry_id.to_string())
                .then_with(|| left.disposition.tag().cmp(right.disposition.tag()))
        });
        discrepancies.sort_by(|left, right| {
            left.entry_id
                .map(|id| id.to_string())
                .cmp(&right.entry_id.map(|id| id.to_string()))
                .then_with(|| left.side.tag().cmp(right.side.tag()))
                .then_with(|| left.kind.tag().cmp(right.kind.tag()))
                .then_with(|| {
                    left.object_fingerprint
                        .as_checksum()
                        .as_str()
                        .cmp(right.object_fingerprint.as_checksum().as_str())
                })
        });
        discrepancies.dedup();
        let clean = discrepancies.is_empty()
            && source.object_count == expected_source.object_count
            && target.object_count == expected_target.object_count
            && source.logical_bytes == expected_source.logical_bytes
            && target.logical_bytes == expected_target.logical_bytes
            && source.role_counts == expected_source.role_counts
            && target.role_counts == expected_target.role_counts
            && source.strong_checksums_verified == expected_source.object_count
            && target.strong_checksums_verified == expected_target.object_count
            && source.media_probes_verified == expected_source.media_probes_verified
            && target.media_probes_verified == expected_target.media_probes_verified;
        let report_digest = reconciliation_digest(
            &manifest_digest,
            generated_at,
            expected_source,
            expected_target,
            source,
            target,
            &dispositions,
            disposition_totals,
            &discrepancies,
            clean,
        );
        Self {
            protocol_version: OBJECT_BACKFILL_PROTOCOL_VERSION_V1,
            manifest_digest,
            generated_at,
            expected_source,
            expected_target,
            source,
            target,
            dispositions,
            disposition_totals,
            discrepancies,
            clean,
            report_digest,
        }
    }

    #[must_use]
    pub const fn clean(&self) -> bool {
        self.clean
    }

    #[must_use]
    pub const fn generated_at(&self) -> TimestampMillis {
        self.generated_at
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ChecksumSha256 {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn expected_source(&self) -> BackfillInventoryTotalsV1 {
        self.expected_source
    }

    #[must_use]
    pub const fn expected_target(&self) -> BackfillInventoryTotalsV1 {
        self.expected_target
    }

    #[must_use]
    pub const fn source(&self) -> BackfillInventoryTotalsV1 {
        self.source
    }

    #[must_use]
    pub const fn target(&self) -> BackfillInventoryTotalsV1 {
        self.target
    }

    #[must_use]
    pub fn dispositions(&self) -> &[BackfillReconciliationDispositionV1] {
        &self.dispositions
    }

    #[must_use]
    pub const fn disposition_totals(&self) -> BackfillDispositionTotalsV1 {
        self.disposition_totals
    }

    #[must_use]
    pub fn discrepancies(&self) -> &[BackfillDiscrepancyV1] {
        &self.discrepancies
    }

    #[must_use]
    pub const fn report_digest(&self) -> &ChecksumSha256 {
        &self.report_digest
    }

    pub fn validate_integrity(
        &self,
        manifest: &ObjectBackfillManifestV1,
        now: TimestampMillis,
        maximum_age: DurationMillis,
    ) -> Result<(), BackfillContractErrorV1> {
        manifest.validate_integrity()?;
        let (expected_source, expected_target, disposition_totals) =
            reconciliation_expectations(manifest, &self.dispositions, self.generated_at)?;
        let mut normalized_dispositions = self.dispositions.clone();
        normalized_dispositions.sort_by(|left, right| {
            left.entry_id
                .to_string()
                .cmp(&right.entry_id.to_string())
                .then_with(|| left.disposition.tag().cmp(right.disposition.tag()))
        });
        let mut normalized = self.discrepancies.clone();
        normalized.sort_by(|left, right| {
            left.entry_id
                .map(|id| id.to_string())
                .cmp(&right.entry_id.map(|id| id.to_string()))
                .then_with(|| left.side.tag().cmp(right.side.tag()))
                .then_with(|| left.kind.tag().cmp(right.kind.tag()))
                .then_with(|| {
                    left.object_fingerprint
                        .as_checksum()
                        .as_str()
                        .cmp(right.object_fingerprint.as_checksum().as_str())
                })
        });
        normalized.dedup();
        let expires_at = self
            .generated_at
            .checked_add(maximum_age)
            .map_err(|_| BackfillContractErrorV1::InvalidReport)?;
        let expected_clean = self.discrepancies.is_empty()
            && self.source.object_count == expected_source.object_count
            && self.target.object_count == expected_target.object_count
            && self.source.logical_bytes == expected_source.logical_bytes
            && self.target.logical_bytes == expected_target.logical_bytes
            && self.source.role_counts == expected_source.role_counts
            && self.target.role_counts == expected_target.role_counts
            && self.source.strong_checksums_verified == expected_source.object_count
            && self.target.strong_checksums_verified == expected_target.object_count
            && self.source.media_probes_verified == expected_source.media_probes_verified
            && self.target.media_probes_verified == expected_target.media_probes_verified;
        let digest = reconciliation_digest(
            &self.manifest_digest,
            self.generated_at,
            self.expected_source,
            self.expected_target,
            self.source,
            self.target,
            &self.dispositions,
            self.disposition_totals,
            &self.discrepancies,
            self.clean,
        );
        if self.protocol_version != OBJECT_BACKFILL_PROTOCOL_VERSION_V1
            || self.manifest_digest != *manifest.digest()
            || self.generated_at < manifest.created_at()
            || self.generated_at > now
            || expires_at < now
            || self.expected_source != expected_source
            || self.expected_target != expected_target
            || self.disposition_totals != disposition_totals
            || normalized_dispositions != self.dispositions
            || !totals_are_bounded(self.source)
            || !totals_are_bounded(self.target)
            || !self.disposition_totals.valid()
            || normalized != self.discrepancies
            || self.clean != expected_clean
            || self.report_digest != digest
        {
            return Err(BackfillContractErrorV1::InvalidReport);
        }
        Ok(())
    }

    #[must_use]
    pub fn dry_run_repair_plan(&self) -> BackfillRepairPlanV1 {
        BackfillRepairPlanV1 {
            report_digest: self.report_digest.clone(),
            dry_run: true,
            actions: self
                .discrepancies
                .iter()
                .map(|discrepancy| match discrepancy.kind {
                    BackfillDiscrepancyKindV1::MissingTarget => {
                        BackfillRepairActionV1::CopyMissingTarget(
                            discrepancy.entry_id,
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                    BackfillDiscrepancyKindV1::MissingSource
                    | BackfillDiscrepancyKindV1::CorruptSource
                    | BackfillDiscrepancyKindV1::UnplayableSource => {
                        BackfillRepairActionV1::QuarantineSource(
                            discrepancy.entry_id,
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                    BackfillDiscrepancyKindV1::DuplicateSource
                    | BackfillDiscrepancyKindV1::DuplicateTarget => {
                        BackfillRepairActionV1::ReviewDuplicate(
                            discrepancy.entry_id,
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                    BackfillDiscrepancyKindV1::OrphanSource => {
                        BackfillRepairActionV1::ReviewOrphanSource(
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                    BackfillDiscrepancyKindV1::OrphanTarget => {
                        BackfillRepairActionV1::ReviewOrphanTarget(
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                    BackfillDiscrepancyKindV1::OwnershipMismatch => {
                        BackfillRepairActionV1::ReviewOwnership(
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                    BackfillDiscrepancyKindV1::CorruptTarget
                    | BackfillDiscrepancyKindV1::UnplayableTarget
                    | BackfillDiscrepancyKindV1::MetadataDivergence
                    | BackfillDiscrepancyKindV1::CheckpointDivergence => {
                        BackfillRepairActionV1::InvestigateConflict(
                            discrepancy.entry_id,
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                    BackfillDiscrepancyKindV1::ProviderUnavailable => {
                        BackfillRepairActionV1::RetryInventory(
                            discrepancy.object_fingerprint.clone(),
                        )
                    }
                })
                .collect(),
        }
    }
}

impl fmt::Debug for ObjectBackfillReconciliationReportV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectBackfillReconciliationReportV1")
            .field("protocol_version", &self.protocol_version)
            .field("manifest_digest", &"[redacted]")
            .field("generated_at", &self.generated_at)
            .field("expected_source", &self.expected_source)
            .field("expected_target", &self.expected_target)
            .field("source", &self.source)
            .field("target", &self.target)
            .field("dispositions", &self.dispositions)
            .field("disposition_totals", &self.disposition_totals)
            .field("discrepancies", &self.discrepancies)
            .field("clean", &self.clean)
            .field("report_digest", &"[redacted]")
            .finish()
    }
}

#[allow(clippy::too_many_arguments)]
fn reconciliation_digest(
    manifest_digest: &ChecksumSha256,
    generated_at: TimestampMillis,
    expected_source: BackfillInventoryTotalsV1,
    expected_target: BackfillInventoryTotalsV1,
    source: BackfillInventoryTotalsV1,
    target: BackfillInventoryTotalsV1,
    dispositions: &[BackfillReconciliationDispositionV1],
    disposition_totals: BackfillDispositionTotalsV1,
    discrepancies: &[BackfillDiscrepancyV1],
    clean: bool,
) -> ChecksumSha256 {
    let mut bytes = Vec::new();
    frame(&mut bytes, manifest_digest.as_str().as_bytes());
    frame(&mut bytes, &generated_at.get().to_be_bytes());
    for totals in [expected_source, expected_target, source, target] {
        frame(&mut bytes, &totals.object_count.to_be_bytes());
        frame(&mut bytes, &totals.logical_bytes.to_be_bytes());
        for count in totals.role_counts {
            frame(&mut bytes, &count.to_be_bytes());
        }
        frame(&mut bytes, &totals.strong_checksums_verified.to_be_bytes());
        frame(&mut bytes, &totals.media_probes_verified.to_be_bytes());
    }
    for disposition in dispositions {
        frame(&mut bytes, disposition.entry_id.to_string().as_bytes());
        frame(&mut bytes, disposition.failure.tag().as_bytes());
        frame(&mut bytes, disposition.disposition.tag().as_bytes());
        frame(
            &mut bytes,
            disposition.approval.approval_id.to_string().as_bytes(),
        );
        frame(
            &mut bytes,
            disposition.approval.subject_fingerprint.as_str().as_bytes(),
        );
        frame(
            &mut bytes,
            disposition.approval.scope_fingerprint.as_str().as_bytes(),
        );
        frame(
            &mut bytes,
            &disposition.approval.issued_at.get().to_be_bytes(),
        );
        frame(
            &mut bytes,
            &disposition.approval.expires_at.get().to_be_bytes(),
        );
        frame(
            &mut bytes,
            &disposition.approval.verified_at.get().to_be_bytes(),
        );
    }
    for count in [
        disposition_totals.referenced_objects,
        disposition_totals.referenced_logical_bytes,
        disposition_totals.excluded_objects,
        disposition_totals.excluded_logical_bytes,
    ] {
        frame(&mut bytes, &count.to_be_bytes());
    }
    for count in disposition_totals.referenced_role_counts {
        frame(&mut bytes, &count.to_be_bytes());
    }
    for count in disposition_totals.excluded_role_counts {
        frame(&mut bytes, &count.to_be_bytes());
    }
    for discrepancy in discrepancies {
        frame(
            &mut bytes,
            discrepancy
                .entry_id
                .map_or_else(Vec::new, |id| id.to_string().into_bytes())
                .as_slice(),
        );
        frame(&mut bytes, discrepancy.side.tag().as_bytes());
        frame(&mut bytes, discrepancy.kind.tag().as_bytes());
        frame(
            &mut bytes,
            discrepancy
                .object_fingerprint
                .as_checksum()
                .as_str()
                .as_bytes(),
        );
    }
    frame(&mut bytes, &[u8::from(clean)]);
    ChecksumSha256::digest_bytes(&bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackfillRepairActionV1 {
    CopyMissingTarget(Option<BackfillEntryIdV1>, BackfillObjectFingerprintV1),
    QuarantineSource(Option<BackfillEntryIdV1>, BackfillObjectFingerprintV1),
    InvestigateConflict(Option<BackfillEntryIdV1>, BackfillObjectFingerprintV1),
    ReviewDuplicate(Option<BackfillEntryIdV1>, BackfillObjectFingerprintV1),
    ReviewOrphanSource(BackfillObjectFingerprintV1),
    ReviewOrphanTarget(BackfillObjectFingerprintV1),
    ReviewOwnership(BackfillObjectFingerprintV1),
    RetryInventory(BackfillObjectFingerprintV1),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackfillRepairPlanV1 {
    report_digest: ChecksumSha256,
    dry_run: bool,
    actions: Vec<BackfillRepairActionV1>,
}

impl BackfillRepairPlanV1 {
    #[must_use]
    pub const fn report_digest(&self) -> &ChecksumSha256 {
        &self.report_digest
    }

    #[must_use]
    pub const fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    #[must_use]
    pub fn actions(&self) -> &[BackfillRepairActionV1] {
        &self.actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectRevision, StorageFileExtension, VideoObjectDescriptor};

    fn timestamp(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("timestamp")
    }

    fn authority(provider: BackfillProviderV1, marker: u8) -> BackfillStorageAuthorityV1 {
        BackfillStorageAuthorityV1::new(
            provider,
            "us-test-1",
            BackfillProviderLocatorV1::parse(format!("bucket-{marker}")).expect("locator"),
            ChecksumSha256::parse(format!("{marker:02x}").repeat(32)).expect("fingerprint"),
        )
        .expect("authority")
    }

    fn entry(bytes: &[u8]) -> ObjectBackfillManifestEntryV1 {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let target = ScopedObjectKey::source(
            tenant,
            video,
            ObjectRevision::new(2).expect("revision"),
            VideoObjectDescriptor::Source {
                extension: StorageFileExtension::parse("mp4").expect("extension"),
            },
        )
        .expect("key");
        ObjectBackfillManifestEntryV1::new(
            BackfillEntryIdV1::new(),
            tenant,
            video,
            ObjectRole::Source,
            BackfillSourceReferenceV1::parse("legacy/source.mp4").expect("source"),
            target,
            ByteSize::new(u64::try_from(bytes.len()).expect("length")).expect("size"),
            ChecksumSha256::digest_bytes(bytes),
            Some(BackfillProviderChecksumV1::parse("multipart-etag-2").expect("opaque")),
            ContentType::parse("video/mp4").expect("type"),
            BackfillMediaProbePolicyV1::new(1, BackfillMediaProbeModeV1::Required).expect("probe"),
        )
        .expect("entry")
    }

    fn manifest() -> ObjectBackfillManifestV1 {
        ObjectBackfillManifestV1::new(
            BackfillManifestIdV1::new(),
            timestamp(1),
            "backfill-tool-1.0.0",
            "git-deadbeef",
            authority(BackfillProviderV1::S3, 0xaa),
            authority(BackfillProviderV1::R2, 0xbb),
            policy(),
            vec![entry(b"valid-media")],
        )
        .expect("manifest")
    }

    fn multi_entry_manifest(
        count: usize,
        execution_policy: BackfillExecutionPolicyV1,
    ) -> ObjectBackfillManifestV1 {
        let entries = (0..count)
            .map(|index| {
                let bytes = format!("valid-media-{index}");
                let mut entry = entry(bytes.as_bytes());
                entry.source_reference =
                    BackfillSourceReferenceV1::parse(format!("legacy/source-{index}.mp4"))
                        .expect("unique source reference");
                entry
            })
            .collect();
        ObjectBackfillManifestV1::new(
            BackfillManifestIdV1::new(),
            timestamp(1),
            "backfill-tool-1.0.0",
            "git-deadbeef",
            authority(BackfillProviderV1::S3, 0xcc),
            authority(BackfillProviderV1::R2, 0xdd),
            execution_policy,
            entries,
        )
        .expect("multi-entry manifest")
    }

    fn policy() -> BackfillExecutionPolicyV1 {
        BackfillExecutionPolicyV1::new(
            2,
            3,
            100,
            1_000_000,
            10_000,
            1_000_000,
            60,
            DurationMillis::new(10).expect("delay"),
            DurationMillis::new(100).expect("delay"),
            2,
            DurationMillis::new(1_000).expect("cooldown"),
            DurationMillis::new(100).expect("lease"),
            ByteSize::new(64 * 1_024).expect("chunk"),
        )
        .expect("policy")
    }

    fn disposition_approval(
        manifest: &ObjectBackfillManifestV1,
        entry_id: BackfillEntryIdV1,
        disposition: BackfillOwnerDispositionV1,
        now: TimestampMillis,
    ) -> BackfillOwnerApprovalRecordV1 {
        BackfillOwnerApprovalRecordV1::new(
            BackfillOwnerApprovalIdV1::new(),
            ChecksumSha256::digest_bytes(b"authenticated-owner"),
            backfill_disposition_approval_scope_v1(manifest.digest(), entry_id, disposition),
            manifest.created_at(),
            now.checked_add(DurationMillis::new(100).expect("approval life"))
                .expect("approval expiry"),
            now,
        )
        .expect("approval")
    }

    fn release_approval(
        manifest: &ObjectBackfillManifestV1,
        report: &ObjectBackfillReconciliationReportV1,
        now: TimestampMillis,
    ) -> BackfillOwnerApprovalRecordV1 {
        BackfillOwnerApprovalRecordV1::new(
            BackfillOwnerApprovalIdV1::new(),
            ChecksumSha256::digest_bytes(b"authenticated-owner"),
            backfill_source_release_approval_scope_v1(manifest.digest(), report.report_digest()),
            manifest.created_at(),
            now.checked_add(DurationMillis::new(100).expect("approval life"))
                .expect("approval expiry"),
            now,
        )
        .expect("approval")
    }

    #[test]
    fn manifest_is_immutable_versioned_deterministic_and_credential_free() {
        let manifest = manifest();
        let encoded = serde_json::to_string(&manifest).expect("serialize manifest");
        assert!(encoded.contains("source_reference"));
        assert!(encoded.contains("authority_fingerprint"));
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("secret"));
        assert_eq!(manifest.protocol_version(), 1);
        assert_eq!(manifest.schema_version(), 1);
        assert_eq!(manifest.entries()[0].target_version().get(), 2);

        let decoded: ObjectBackfillManifestV1 =
            serde_json::from_str(&encoded).expect("deserialize manifest");
        assert_eq!(decoded.digest(), manifest.digest());
        assert_eq!(decoded, manifest);
        let debug = format!("{manifest:?}");
        assert!(!debug.contains("legacy/source.mp4"));
        assert!(!debug.contains(manifest.entries()[0].strong_sha256().as_str()));

        let credential = BackfillCredentialRefV1::parse("vault:storage/source").expect("ref");
        assert_eq!(
            format!("{credential:?}"),
            "BackfillCredentialRefV1([redacted])"
        );
        assert_eq!(credential.to_string(), "[redacted]");

        let mut unknown: serde_json::Value =
            serde_json::from_str(&encoded).expect("manifest value");
        unknown.as_object_mut().expect("manifest object").insert(
            "credential".into(),
            serde_json::Value::String("secret".into()),
        );
        assert!(serde_json::from_value::<ObjectBackfillManifestV1>(unknown).is_err());

        let mut tampered = manifest.clone();
        tampered.digest = ChecksumSha256::digest_bytes(b"tampered");
        assert_eq!(
            tampered.validate_integrity(),
            Err(BackfillContractErrorV1::InvalidManifest)
        );
    }

    #[test]
    fn signed_urls_and_duplicate_targets_are_rejected() {
        assert!(BackfillSourceReferenceV1::parse("https://host/object?token=secret").is_err());
        let first = entry(b"media");
        let mut second = first.clone();
        second.entry_id = BackfillEntryIdV1::new();
        assert_eq!(
            ObjectBackfillManifestV1::new(
                BackfillManifestIdV1::new(),
                timestamp(1),
                "tool",
                "code",
                authority(BackfillProviderV1::S3, 1),
                authority(BackfillProviderV1::R2, 2),
                policy(),
                vec![first, second],
            ),
            Err(BackfillContractErrorV1::InvalidManifest)
        );
    }

    #[test]
    fn journal_cas_payload_fences_expired_and_stale_workers() {
        let manifest = manifest();
        let entry = manifest.entries()[0].entry_id();
        let mut journal = ObjectBackfillJournalV1::new(&manifest);
        let first = journal
            .claim(
                entry,
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(10),
                policy(),
                manifest.entries()[0].expected_size(),
                1,
            )
            .expect("claim");
        let second_operation = BackfillOperationIdV1::new();
        journal
            .normalize_expired_lease(entry, timestamp(111), policy())
            .expect("normalize expired lease");
        let second = journal
            .claim(
                entry,
                BackfillWorkerIdV1::new(),
                second_operation,
                timestamp(111),
                policy(),
                manifest.entries()[0].expected_size(),
                1,
            )
            .expect("reclaim");
        assert!(second.fencing_token() > first.fencing_token());
        let receipt = BackfillOperationReceiptV1::new(
            second_operation,
            entry,
            BackfillWriteResultV1::Created,
            BackfillDestinationVersionV1::parse("version-1").expect("version"),
            manifest.entries()[0].expected_size(),
            manifest.entries()[0].strong_sha256().clone(),
            Some(1),
            timestamp(112),
        )
        .expect("receipt");
        assert_eq!(
            journal.complete(entry, first, receipt.clone(), timestamp(112)),
            Err(BackfillContractErrorV1::StaleLease)
        );
        journal
            .complete(entry, second, receipt, timestamp(112))
            .expect("current worker completes");
        assert_eq!(journal.state(), BackfillRunStateV1::Completed);
    }

    #[test]
    fn concurrency_is_global_across_entries_and_tenants_but_excludes_expired_leases() {
        let mut execution_policy = policy();
        execution_policy.max_concurrency = 1;
        let manifest = multi_entry_manifest(2, execution_policy);
        assert_ne!(
            manifest.entries()[0].tenant_id(),
            manifest.entries()[1].tenant_id()
        );
        let mut journal = ObjectBackfillJournalV1::new(&manifest);
        journal
            .claim(
                manifest.entries()[0].entry_id(),
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(10),
                execution_policy,
                manifest.entries()[0].expected_size(),
                1,
            )
            .expect("first tenant claims the only live slot");
        assert_eq!(
            journal.claim(
                manifest.entries()[1].entry_id(),
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(10),
                execution_policy,
                manifest.entries()[1].expected_size(),
                1,
            ),
            Err(BackfillContractErrorV1::InvalidTransition)
        );
        journal
            .claim(
                manifest.entries()[1].entry_id(),
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(111),
                execution_policy,
                manifest.entries()[1].expected_size(),
                1,
            )
            .expect("expired lease does not consume a live slot");
        journal
            .validate_for_manifest_at(&manifest, timestamp(111))
            .expect("one live lease remains valid");
    }

    #[test]
    fn circuit_counts_only_consecutive_provider_failures_and_allows_one_half_open_probe() {
        let mut execution_policy = policy();
        execution_policy.max_concurrency = 2;
        execution_policy.max_attempts = 4;
        let manifest = multi_entry_manifest(4, execution_policy);
        let ids = manifest
            .entries()
            .iter()
            .map(ObjectBackfillManifestEntryV1::entry_id)
            .collect::<Vec<_>>();
        let mut journal = ObjectBackfillJournalV1::new(&manifest);

        let provider_one = journal
            .claim(
                ids[0],
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(10),
                execution_policy,
                manifest.entries()[0].expected_size(),
                1,
            )
            .expect("first provider attempt");
        journal
            .fail(
                ids[0],
                provider_one,
                BackfillFailureClassV1::ProviderOutage,
                timestamp(11),
                execution_policy,
            )
            .expect("first provider failure");
        assert_eq!(journal.circuit().consecutive_provider_failures(), 1);

        let non_provider = journal
            .claim(
                ids[1],
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(12),
                execution_policy,
                manifest.entries()[1].expected_size(),
                1,
            )
            .expect("non-provider attempt");
        journal
            .fail(
                ids[1],
                non_provider,
                BackfillFailureClassV1::SourceChecksumMismatch,
                timestamp(13),
                execution_policy,
            )
            .expect("non-provider failure resets sequence");
        assert_eq!(journal.circuit(), BackfillCircuitStateV1::default());

        let provider_two = journal
            .claim(
                ids[0],
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(21),
                execution_policy,
                manifest.entries()[0].expected_size(),
                1,
            )
            .expect("provider sequence restarts");
        journal
            .fail(
                ids[0],
                provider_two,
                BackfillFailureClassV1::ProviderOutage,
                timestamp(22),
                execution_policy,
            )
            .expect("new first provider failure");
        let opens = journal
            .claim(
                ids[2],
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(23),
                execution_policy,
                manifest.entries()[2].expected_size(),
                1,
            )
            .expect("second consecutive provider attempt");
        journal
            .fail(
                ids[2],
                opens,
                BackfillFailureClassV1::ProviderOutage,
                timestamp(24),
                execution_policy,
            )
            .expect("threshold opens circuit");
        assert_eq!(journal.circuit().open_until(), Some(timestamp(1_024)));

        let half_open = journal
            .claim(
                ids[3],
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(1_024),
                execution_policy,
                manifest.entries()[3].expected_size(),
                1,
            )
            .expect("one half-open probe");
        assert_eq!(
            journal.circuit().half_open_fencing_token(),
            Some(half_open.fencing_token())
        );
        assert_eq!(
            journal.claim(
                ids[0],
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(1_024),
                execution_policy,
                manifest.entries()[0].expected_size(),
                1,
            ),
            Err(BackfillContractErrorV1::InvalidTransition)
        );
        journal
            .fail(
                ids[3],
                half_open,
                BackfillFailureClassV1::SourceChecksumMismatch,
                timestamp(1_025),
                execution_policy,
            )
            .expect("non-provider half-open result closes circuit");
        assert_eq!(journal.circuit(), BackfillCircuitStateV1::default());
    }

    #[test]
    fn retry_approval_authorizes_exactly_one_bounded_extra_attempt() {
        let mut execution_policy = policy();
        execution_policy.max_attempts = 1;
        let manifest = multi_entry_manifest(1, execution_policy);
        let entry = &manifest.entries()[0];
        let mut journal = ObjectBackfillJournalV1::new(&manifest);
        let first = journal
            .claim(
                entry.entry_id(),
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(10),
                execution_policy,
                entry.expected_size(),
                1,
            )
            .expect("normal attempt");
        journal
            .fail(
                entry.entry_id(),
                first,
                BackfillFailureClassV1::MissingSource,
                timestamp(11),
                execution_policy,
            )
            .expect("normal attempts exhausted");
        journal
            .approve_disposition(
                entry.entry_id(),
                BackfillOwnerDispositionV1::RetryApproved,
                disposition_approval(
                    &manifest,
                    entry.entry_id(),
                    BackfillOwnerDispositionV1::RetryApproved,
                    timestamp(12),
                ),
                timestamp(12),
            )
            .expect("approve one extra attempt");
        let extra = journal
            .claim(
                entry.entry_id(),
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(13),
                execution_policy,
                entry.expected_size(),
                1,
            )
            .expect("approved extra attempt");
        journal
            .fail(
                entry.entry_id(),
                extra,
                BackfillFailureClassV1::MissingSource,
                timestamp(14),
                execution_policy,
            )
            .expect("extra attempt exhausts approval");
        assert_eq!(journal.entries()[0].attempts(), 2);
        assert!(journal.entries()[0].retry_approval_consumed());
        assert_eq!(
            journal.approve_disposition(
                entry.entry_id(),
                BackfillOwnerDispositionV1::RetryApproved,
                disposition_approval(
                    &manifest,
                    entry.entry_id(),
                    BackfillOwnerDispositionV1::RetryApproved,
                    timestamp(15),
                ),
                timestamp(15),
            ),
            Err(BackfillContractErrorV1::InvalidDisposition)
        );
    }

    #[test]
    fn pause_resume_abort_and_owner_dispositions_are_explicit() {
        let manifest = manifest();
        let entry_id = manifest.entries()[0].entry_id();
        let mut journal = ObjectBackfillJournalV1::new(&manifest);
        journal.pause(timestamp(1)).expect("pause");
        assert!(journal.next_eligible_entry(timestamp(1)).is_none());
        journal.resume(timestamp(2)).expect("resume");
        let lease = journal
            .claim(
                entry_id,
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(3),
                policy(),
                manifest.entries()[0].expected_size(),
                1,
            )
            .expect("claim");
        journal
            .fail(
                entry_id,
                lease,
                BackfillFailureClassV1::SourceChecksumMismatch,
                timestamp(4),
                policy(),
            )
            .expect("quarantine");
        journal
            .approve_disposition(
                entry_id,
                BackfillOwnerDispositionV1::ExcludeApproved,
                disposition_approval(
                    &manifest,
                    entry_id,
                    BackfillOwnerDispositionV1::ExcludeApproved,
                    timestamp(5),
                ),
                timestamp(5),
            )
            .expect("approve exclusion");
        assert_eq!(journal.state(), BackfillRunStateV1::Completed);
        assert_eq!(
            journal.approve_disposition(
                entry_id,
                BackfillOwnerDispositionV1::RetryApproved,
                disposition_approval(
                    &manifest,
                    entry_id,
                    BackfillOwnerDispositionV1::RetryApproved,
                    timestamp(6),
                ),
                timestamp(6),
            ),
            Err(BackfillContractErrorV1::InvalidDisposition)
        );
        assert_eq!(
            journal.abort(timestamp(6)),
            Err(BackfillContractErrorV1::InvalidTransition)
        );
    }

    #[test]
    fn source_release_needs_clean_exact_reconciliation_and_owner_approval() {
        let manifest = manifest();
        let entry_id = manifest.entries()[0].entry_id();
        let mut journal = ObjectBackfillJournalV1::new(&manifest);
        let operation_id = BackfillOperationIdV1::new();
        let lease = journal
            .claim(
                entry_id,
                BackfillWorkerIdV1::new(),
                operation_id,
                timestamp(1),
                policy(),
                manifest.entries()[0].expected_size(),
                1,
            )
            .expect("claim");
        journal
            .complete(
                entry_id,
                lease,
                BackfillOperationReceiptV1::new(
                    operation_id,
                    entry_id,
                    BackfillWriteResultV1::Created,
                    BackfillDestinationVersionV1::parse("v1").expect("version"),
                    manifest.entries()[0].expected_size(),
                    manifest.entries()[0].strong_sha256().clone(),
                    Some(1),
                    timestamp(2),
                )
                .expect("receipt"),
                timestamp(2),
            )
            .expect("complete");
        let role_counts = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let expected = BackfillInventoryTotalsV1::new(
            1,
            manifest.entries()[0].expected_size().get(),
            role_counts,
            0,
            1,
        );
        let observed = BackfillInventoryTotalsV1::new(
            1,
            manifest.entries()[0].expected_size().get(),
            role_counts,
            1,
            1,
        );
        let target_fingerprint = BackfillObjectFingerprintV1::derive(
            manifest.target().authority_fingerprint(),
            BackfillInventorySideV1::Target,
            manifest.entries()[0].target_key().as_str(),
            Some("v1"),
        );
        let dirty = ObjectBackfillReconciliationReportV1::new(
            manifest.digest().clone(),
            timestamp(3),
            expected,
            expected,
            observed,
            observed,
            vec![BackfillDiscrepancyV1::new(
                Some(entry_id),
                BackfillInventorySideV1::Target,
                BackfillDiscrepancyKindV1::MetadataDivergence,
                target_fingerprint,
            )],
        );
        let dirty_approval = release_approval(&manifest, &dirty, timestamp(4));
        assert_eq!(
            journal.approve_source_release(
                &manifest,
                &dirty,
                dirty_approval,
                timestamp(4),
                DurationMillis::new(100).expect("maximum report age"),
            ),
            Err(BackfillContractErrorV1::RetentionBlocked)
        );
        let clean = ObjectBackfillReconciliationReportV1::new(
            manifest.digest().clone(),
            timestamp(3),
            expected,
            expected,
            observed,
            observed,
            vec![],
        );
        let clean_approval = release_approval(&manifest, &clean, timestamp(4));
        journal
            .approve_source_release(
                &manifest,
                &clean,
                clean_approval,
                timestamp(4),
                DurationMillis::new(100).expect("maximum report age"),
            )
            .expect("approve release");
        assert!(matches!(
            journal.source_retention(),
            BackfillSourceRetentionStateV1::ReleaseApproved { .. }
        ));
        let duplicate_approval = release_approval(&manifest, &clean, timestamp(5));
        assert_eq!(
            journal.approve_source_release(
                &manifest,
                &clean,
                duplicate_approval,
                timestamp(5),
                DurationMillis::new(100).expect("maximum report age"),
            ),
            Err(BackfillContractErrorV1::RetentionBlocked)
        );
    }

    #[test]
    fn reconciliation_reports_reject_forgery_and_staleness() {
        let manifest = manifest();
        let expected = expected_reconciliation_totals(&manifest);
        let mut observed = expected;
        observed.strong_checksums_verified = observed.object_count;
        let clean = ObjectBackfillReconciliationReportV1::new(
            manifest.digest().clone(),
            timestamp(3),
            expected,
            expected,
            observed,
            observed,
            vec![],
        );
        clean
            .validate_integrity(
                &manifest,
                timestamp(4),
                DurationMillis::new(100).expect("maximum report age"),
            )
            .expect("untampered fresh report");

        let mut forged = serde_json::to_value(&clean).expect("serialize report");
        forged
            .as_object_mut()
            .expect("report object")
            .insert("clean".into(), serde_json::Value::Bool(false));
        let forged: ObjectBackfillReconciliationReportV1 =
            serde_json::from_value(forged).expect("deserialize structurally valid forgery");
        assert_eq!(
            forged.validate_integrity(
                &manifest,
                timestamp(4),
                DurationMillis::new(100).expect("maximum report age"),
            ),
            Err(BackfillContractErrorV1::InvalidReport)
        );
        assert_eq!(
            clean.validate_integrity(
                &manifest,
                timestamp(104),
                DurationMillis::new(100).expect("maximum report age"),
            ),
            Err(BackfillContractErrorV1::InvalidReport)
        );
    }

    #[test]
    fn terminal_dispositions_are_verified_bound_and_accounted_deterministically() {
        let manifest = multi_entry_manifest(2, policy());
        let reference_entry = &manifest.entries()[0];
        let excluded_entry = &manifest.entries()[1];
        let mut journal = ObjectBackfillJournalV1::new(&manifest);

        let reference_lease = journal
            .claim(
                reference_entry.entry_id(),
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(2),
                policy(),
                reference_entry.expected_size(),
                1,
            )
            .expect("claim reference candidate");
        journal
            .fail(
                reference_entry.entry_id(),
                reference_lease,
                BackfillFailureClassV1::SourceMetadataMismatch,
                timestamp(3),
                policy(),
            )
            .expect("quarantine reference candidate");
        let reference_approval = disposition_approval(
            &manifest,
            reference_entry.entry_id(),
            BackfillOwnerDispositionV1::ReferenceApproved,
            timestamp(4),
        );
        journal
            .approve_disposition(
                reference_entry.entry_id(),
                BackfillOwnerDispositionV1::ReferenceApproved,
                reference_approval.clone(),
                timestamp(4),
            )
            .expect("approve reference");

        let excluded_lease = journal
            .claim(
                excluded_entry.entry_id(),
                BackfillWorkerIdV1::new(),
                BackfillOperationIdV1::new(),
                timestamp(5),
                policy(),
                excluded_entry.expected_size(),
                1,
            )
            .expect("claim exclusion candidate");
        journal
            .fail(
                excluded_entry.entry_id(),
                excluded_lease,
                BackfillFailureClassV1::MissingSource,
                timestamp(6),
                policy(),
            )
            .expect("quarantine exclusion candidate");
        let excluded_approval = disposition_approval(
            &manifest,
            excluded_entry.entry_id(),
            BackfillOwnerDispositionV1::ExcludeApproved,
            timestamp(7),
        );
        journal
            .approve_disposition(
                excluded_entry.entry_id(),
                BackfillOwnerDispositionV1::ExcludeApproved,
                excluded_approval.clone(),
                timestamp(7),
            )
            .expect("approve exclusion");

        let reference = BackfillReconciliationDispositionV1::new(
            reference_entry.entry_id(),
            BackfillFailureClassV1::SourceMetadataMismatch,
            BackfillOwnerDispositionV1::ReferenceApproved,
            reference_approval,
        )
        .expect("reference evidence");
        let exclusion = BackfillReconciliationDispositionV1::new(
            excluded_entry.entry_id(),
            BackfillFailureClassV1::MissingSource,
            BackfillOwnerDispositionV1::ExcludeApproved,
            excluded_approval,
        )
        .expect("exclusion evidence");
        let source = BackfillInventoryTotalsV1::new(
            1,
            reference_entry.expected_size().get(),
            [1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            1,
            1,
        );
        let target = BackfillInventoryTotalsV1::default();
        let report = ObjectBackfillReconciliationReportV1::new_with_dispositions(
            &manifest,
            timestamp(8),
            source,
            target,
            vec![exclusion.clone(), reference.clone()],
            vec![],
        )
        .expect("clean disposition-aware report");
        assert!(report.clean());
        assert_eq!(report.expected_source().object_count(), 1);
        assert_eq!(report.expected_target().object_count(), 0);
        assert_eq!(report.disposition_totals().referenced_objects(), 1);
        assert_eq!(report.disposition_totals().excluded_objects(), 1);
        assert_eq!(
            report.disposition_totals().referenced_logical_bytes(),
            reference_entry.expected_size().get()
        );
        assert_eq!(
            report.disposition_totals().excluded_logical_bytes(),
            excluded_entry.expected_size().get()
        );
        assert_eq!(
            report
                .disposition_totals()
                .referenced_role_count(ObjectRole::Source),
            1
        );
        assert_eq!(
            report
                .disposition_totals()
                .excluded_role_count(ObjectRole::Source),
            1
        );
        report
            .validate_integrity(
                &manifest,
                timestamp(9),
                DurationMillis::new(100).expect("maximum report age"),
            )
            .expect("valid report");

        let reordered = ObjectBackfillReconciliationReportV1::new_with_dispositions(
            &manifest,
            timestamp(8),
            source,
            target,
            vec![reference.clone(), exclusion.clone()],
            vec![],
        )
        .expect("reordered report");
        assert_eq!(report, reordered);

        let mut forged = reference.clone();
        forged.approval.scope_fingerprint = ChecksumSha256::digest_bytes(b"forged-scope");
        assert_eq!(
            ObjectBackfillReconciliationReportV1::new_with_dispositions(
                &manifest,
                timestamp(8),
                source,
                target,
                vec![forged, exclusion.clone()],
                vec![],
            ),
            Err(BackfillContractErrorV1::InvalidReport)
        );
        let mut stale = reference.clone();
        stale.approval.verified_at = stale.approval.expires_at;
        assert_eq!(
            ObjectBackfillReconciliationReportV1::new_with_dispositions(
                &manifest,
                timestamp(8),
                source,
                target,
                vec![stale, exclusion.clone()],
                vec![],
            ),
            Err(BackfillContractErrorV1::InvalidReport)
        );

        let wrong_failure = BackfillReconciliationDispositionV1::new(
            reference.entry_id(),
            BackfillFailureClassV1::MissingSource,
            reference.disposition(),
            reference.approval().clone(),
        )
        .expect("structurally valid but false evidence");
        let mismatched = ObjectBackfillReconciliationReportV1::new_with_dispositions(
            &manifest,
            timestamp(8),
            source,
            target,
            vec![wrong_failure, exclusion],
            vec![],
        )
        .expect("self-consistent but journal-mismatched report");
        let mismatch_approval = release_approval(&manifest, &mismatched, timestamp(9));
        assert_eq!(
            journal.approve_source_release(
                &manifest,
                &mismatched,
                mismatch_approval,
                timestamp(9),
                DurationMillis::new(100).expect("maximum report age"),
            ),
            Err(BackfillContractErrorV1::RetentionBlocked)
        );
        let approval = release_approval(&manifest, &report, timestamp(9));
        journal
            .approve_source_release(
                &manifest,
                &report,
                approval,
                timestamp(9),
                DurationMillis::new(100).expect("maximum report age"),
            )
            .expect("release exact disposition-aware report");
    }

    #[test]
    fn discrepancy_fingerprints_are_distinct_actionable_and_redacted() {
        let manifest = manifest();
        let expected = expected_reconciliation_totals(&manifest);
        let mut observed = expected;
        observed.strong_checksums_verified = observed.object_count;
        let first = BackfillObjectFingerprintV1::derive(
            manifest.target().authority_fingerprint(),
            BackfillInventorySideV1::Target,
            "private/tenant-a/orphan-one.mp4",
            Some("private-version-one"),
        );
        let second = BackfillObjectFingerprintV1::derive(
            manifest.target().authority_fingerprint(),
            BackfillInventorySideV1::Target,
            "private/tenant-a/orphan-two.mp4",
            Some("private-version-two"),
        );
        assert_ne!(first, second);
        let report = ObjectBackfillReconciliationReportV1::new(
            manifest.digest().clone(),
            timestamp(3),
            expected,
            expected,
            observed,
            observed,
            vec![
                BackfillDiscrepancyV1::new(
                    None,
                    BackfillInventorySideV1::Target,
                    BackfillDiscrepancyKindV1::OrphanTarget,
                    first.clone(),
                ),
                BackfillDiscrepancyV1::new(
                    None,
                    BackfillInventorySideV1::Target,
                    BackfillDiscrepancyKindV1::OrphanTarget,
                    second.clone(),
                ),
            ],
        );
        report
            .validate_integrity(
                &manifest,
                timestamp(4),
                DurationMillis::new(100).expect("maximum report age"),
            )
            .expect("fingerprinted report");
        let plan = report.dry_run_repair_plan();
        assert_eq!(plan.actions().len(), 2);
        assert!(
            plan.actions()
                .contains(&BackfillRepairActionV1::ReviewOrphanTarget(first.clone()))
        );
        assert!(
            plan.actions()
                .contains(&BackfillRepairActionV1::ReviewOrphanTarget(second.clone()))
        );
        let debug = format!("{report:?} {first:?} {second:?}");
        assert!(!debug.contains("private/tenant-a"));
        assert!(!debug.contains("private-version"));
    }

    #[test]
    fn multipart_etag_is_never_accepted_as_strong_checksum() {
        let opaque = BackfillProviderChecksumV1::parse("abc-42").expect("etag");
        assert_eq!(opaque.as_str(), "abc-42");
        assert!(ChecksumSha256::parse(opaque.as_str()).is_err());
        assert_eq!(
            format!("{opaque:?}"),
            "BackfillProviderChecksumV1([opaque])"
        );
    }
}
