use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use serde::{Deserialize, Deserializer, Serialize, de};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ByteSize, ChecksumSha256, ContentType, CorrelationId, CorsOriginV1, MAX_TIMESTAMP_MS,
    MAX_WIRE_INTEGER, TenantId, TimestampMillis, UserId,
};

pub const STORAGE_GOVERNANCE_SCHEMA_VERSION: u16 = 1;
pub const MAX_SIGNED_GRANT_LIFETIME_MS: i64 = 15 * 60 * 1_000;
pub const MAX_CACHE_INVALIDATION_SLO_MS: i64 = 60 * 1_000;
pub const DELETION_RESTORE_GRACE_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
pub const MAX_LIFECYCLE_OBJECTS: usize = 16_384;
pub const MAX_CUSTOM_DOMAIN_LENGTH: usize = 253;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SignedGrantId(Uuid);

impl SignedGrantId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn parse(value: &str) -> Result<Self, StorageGovernanceError> {
        let parsed = Uuid::parse_str(value).map_err(|_| StorageGovernanceError::InvalidGrant)?;
        if parsed.is_nil() || parsed.to_string() != value {
            return Err(StorageGovernanceError::InvalidGrant);
        }
        Ok(Self(parsed))
    }
}

impl Default for SignedGrantId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SignedGrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SignedGrantId([redacted])")
    }
}

impl fmt::Display for SignedGrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for SignedGrantId {
    type Error = StorageGovernanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<SignedGrantId> for String {
    fn from(value: SignedGrantId) -> Self {
        value.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SignedGrantKeyVersion(u32);

impl SignedGrantKeyVersion {
    pub fn new(value: u32) -> Result<Self, StorageGovernanceError> {
        if value == 0 || u64::from(value) > MAX_WIRE_INTEGER {
            return Err(StorageGovernanceError::InvalidGrant);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedGrantSecret(Vec<u8>);

impl SignedGrantSecret {
    pub fn parse(value: impl Into<Vec<u8>>) -> Result<Self, StorageGovernanceError> {
        let value = value.into();
        if !(32..=128).contains(&value.len()) {
            return Err(StorageGovernanceError::InvalidGrant);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_hmac(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn opaque_token(&self) -> String {
        let mut output = String::with_capacity(self.0.len() * 2);
        for byte in &self.0 {
            use fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
        }
        output
    }

    pub fn parse_opaque_token(value: &str) -> Result<Self, StorageGovernanceError> {
        if !value.len().is_multiple_of(2) || !(64..=256).contains(&value.len()) {
            return Err(StorageGovernanceError::InvalidGrant);
        }
        let mut bytes = Vec::with_capacity(value.len() / 2);
        for pair in value.as_bytes().chunks_exact(2) {
            let pair =
                std::str::from_utf8(pair).map_err(|_| StorageGovernanceError::InvalidGrant)?;
            bytes.push(
                u8::from_str_radix(pair, 16).map_err(|_| StorageGovernanceError::InvalidGrant)?,
            );
        }
        Self::parse(bytes)
    }
}

impl fmt::Debug for SignedGrantSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SignedGrantSecret([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedObjectRole {
    Source,
    RecordingSegment,
    Thumbnail,
    Preview,
    Spritesheet,
    Audio,
    Export,
    Caption,
    Avatar,
    Manifest,
    MultipartSession,
    BackupCopy,
}

impl GovernedObjectRole {
    pub const ALL: [Self; 12] = [
        Self::Source,
        Self::RecordingSegment,
        Self::Thumbnail,
        Self::Preview,
        Self::Spritesheet,
        Self::Audio,
        Self::Export,
        Self::Caption,
        Self::Avatar,
        Self::Manifest,
        Self::MultipartSession,
        Self::BackupCopy,
    ];

    #[must_use]
    pub const fn is_source(self) -> bool {
        matches!(self, Self::Source | Self::RecordingSegment)
    }

    #[must_use]
    pub const fn is_derivative(self) -> bool {
        matches!(
            self,
            Self::Thumbnail
                | Self::Preview
                | Self::Spritesheet
                | Self::Audio
                | Self::Export
                | Self::Caption
        )
    }

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::RecordingSegment => "recording_segment",
            Self::Thumbnail => "thumbnail",
            Self::Preview => "preview",
            Self::Spritesheet => "spritesheet",
            Self::Audio => "audio",
            Self::Export => "export",
            Self::Caption => "caption",
            Self::Avatar => "avatar",
            Self::Manifest => "manifest",
            Self::MultipartSession => "multipart_session",
            Self::BackupCopy => "backup_copy",
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GovernedObjectId(String);

impl GovernedObjectId {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageGovernanceError> {
        let value = value.into();
        if !(1..=512).contains(&value.len())
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.' | b'/')
            })
            || value.starts_with('/')
            || value.ends_with('/')
            || value
                .split('/')
                .any(|segment| matches!(segment, "" | "." | ".."))
        {
            return Err(StorageGovernanceError::InvalidObject);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GovernedObjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GovernedObjectId([redacted])")
    }
}

impl TryFrom<String> for GovernedObjectId {
    type Error = StorageGovernanceError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<GovernedObjectId> for String {
    fn from(value: GovernedObjectId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectVisibility {
    Private,
    Unlisted,
    Public,
}

impl ObjectVisibility {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Unlisted => "unlisted",
            Self::Public => "public",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GovernedObjectState {
    Active,
    Quarantined,
    Tombstoned,
    Erased,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MalwareDisposition {
    Pending,
    Clean,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GovernedObject {
    schema_version: u16,
    tenant_id: TenantId,
    object_id: GovernedObjectId,
    role: GovernedObjectRole,
    visibility: ObjectVisibility,
    state: GovernedObjectState,
    malware: MalwareDisposition,
    immutable_revision: u64,
    cache_generation: u64,
    checksum: ChecksumSha256,
    size: ByteSize,
    retention_until: Option<TimestampMillis>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GovernedObjectWire {
    schema_version: u16,
    tenant_id: TenantId,
    object_id: GovernedObjectId,
    role: GovernedObjectRole,
    visibility: ObjectVisibility,
    state: GovernedObjectState,
    malware: MalwareDisposition,
    immutable_revision: u64,
    cache_generation: u64,
    checksum: ChecksumSha256,
    size: ByteSize,
    retention_until: Option<TimestampMillis>,
}

impl<'de> Deserialize<'de> for GovernedObject {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = GovernedObjectWire::deserialize(deserializer)?;
        if wire.schema_version != STORAGE_GOVERNANCE_SCHEMA_VERSION {
            return Err(de::Error::custom(
                "unsupported storage governance schema version",
            ));
        }
        Self::new(
            wire.tenant_id,
            wire.object_id,
            wire.role,
            wire.visibility,
            wire.state,
            wire.malware,
            wire.immutable_revision,
            wire.cache_generation,
            wire.checksum,
            wire.size,
            wire.retention_until,
        )
        .map_err(de::Error::custom)
    }
}

impl GovernedObject {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        object_id: GovernedObjectId,
        role: GovernedObjectRole,
        visibility: ObjectVisibility,
        state: GovernedObjectState,
        malware: MalwareDisposition,
        immutable_revision: u64,
        cache_generation: u64,
        checksum: ChecksumSha256,
        size: ByteSize,
        retention_until: Option<TimestampMillis>,
    ) -> Result<Self, StorageGovernanceError> {
        let tenant_prefix = format!("tenants/{tenant_id}/");
        if immutable_revision == 0
            || immutable_revision > MAX_WIRE_INTEGER
            || cache_generation == 0
            || cache_generation > MAX_WIRE_INTEGER
            || size.get() == 0
            || !object_id.as_str().starts_with(&tenant_prefix)
            || matches!(state, GovernedObjectState::Active)
                && matches!(malware, MalwareDisposition::Rejected)
        {
            return Err(StorageGovernanceError::InvalidObject);
        }
        Ok(Self {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            tenant_id,
            object_id,
            role,
            visibility,
            state,
            malware,
            immutable_revision,
            cache_generation,
            checksum,
            size,
            retention_until,
        })
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub fn object_id(&self) -> &GovernedObjectId {
        &self.object_id
    }
    #[must_use]
    pub const fn role(&self) -> GovernedObjectRole {
        self.role
    }
    #[must_use]
    pub const fn visibility(&self) -> ObjectVisibility {
        self.visibility
    }
    #[must_use]
    pub const fn state(&self) -> GovernedObjectState {
        self.state
    }
    #[must_use]
    pub const fn malware(&self) -> MalwareDisposition {
        self.malware
    }
    #[must_use]
    pub const fn cache_generation(&self) -> u64 {
        self.cache_generation
    }
    #[must_use]
    pub const fn immutable_revision(&self) -> u64 {
        self.immutable_revision
    }
    #[must_use]
    pub const fn size(&self) -> ByteSize {
        self.size
    }
    #[must_use]
    pub const fn retention_until(&self) -> Option<TimestampMillis> {
        self.retention_until
    }
    #[must_use]
    pub fn checksum(&self) -> &ChecksumSha256 {
        &self.checksum
    }

    #[must_use]
    pub fn audit_digest(&self) -> ChecksumSha256 {
        digest_fields(&[
            self.tenant_id.to_string().as_bytes(),
            self.object_id.as_str().as_bytes(),
            self.role.stable_code().as_bytes(),
            self.immutable_revision.to_string().as_bytes(),
            self.cache_generation.to_string().as_bytes(),
            self.checksum.as_str().as_bytes(),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageOperation {
    Read,
    ReadRange,
    WriteImmutable,
    List,
    Copy,
    Sign,
    Delete,
    Restore,
    Export,
    PurgeCache,
    ManageCustomDomain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAccessSurface {
    SameOriginApplication,
    DirectOrigin,
    SignedRoute,
    CustomDomain,
    MediaTransformation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageMemberRole {
    Viewer,
    Editor,
    Admin,
    Owner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageServicePurpose {
    MediaProcessor,
    MalwareScanner,
    Backfill,
    Export,
    Deletion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageActor {
    Anonymous,
    Member {
        tenant_id: TenantId,
        user_id: UserId,
        role: StorageMemberRole,
    },
    Service {
        tenant_id: TenantId,
        purpose: StorageServicePurpose,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SignedObjectGrant {
    schema_version: u16,
    grant_id: SignedGrantId,
    key_version: SignedGrantKeyVersion,
    tenant_id: TenantId,
    object_id: GovernedObjectId,
    immutable_revision: u64,
    cache_generation: u64,
    object_checksum: ChecksumSha256,
    operation: StorageOperation,
    issued_at: TimestampMillis,
    expires_at: TimestampMillis,
    nonce_digest: ChecksumSha256,
    revoked_at: Option<TimestampMillis>,
}

impl fmt::Debug for SignedObjectGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SignedObjectGrant([redacted])")
    }
}

impl SignedObjectGrant {
    #[allow(clippy::too_many_arguments)]
    pub fn persisted(
        grant_id: SignedGrantId,
        key_version: SignedGrantKeyVersion,
        object: &GovernedObject,
        operation: StorageOperation,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
        nonce_digest: ChecksumSha256,
    ) -> Result<Self, StorageGovernanceError> {
        Self::persisted_with_revocation(
            grant_id,
            key_version,
            object,
            operation,
            issued_at,
            expires_at,
            nonce_digest,
            None,
        )
    }

    pub fn new(
        object: &GovernedObject,
        operation: StorageOperation,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
        nonce_digest: ChecksumSha256,
    ) -> Result<Self, StorageGovernanceError> {
        Self::persisted(
            SignedGrantId::new(),
            SignedGrantKeyVersion::new(1)?,
            object,
            operation,
            issued_at,
            expires_at,
            nonce_digest,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn persisted_with_revocation(
        grant_id: SignedGrantId,
        key_version: SignedGrantKeyVersion,
        object: &GovernedObject,
        operation: StorageOperation,
        issued_at: TimestampMillis,
        expires_at: TimestampMillis,
        nonce_digest: ChecksumSha256,
        revoked_at: Option<TimestampMillis>,
    ) -> Result<Self, StorageGovernanceError> {
        let lifetime = expires_at
            .get()
            .checked_sub(issued_at.get())
            .ok_or(StorageGovernanceError::InvalidGrant)?;
        if !matches!(
            operation,
            StorageOperation::Read | StorageOperation::ReadRange
        ) || !(1..=MAX_SIGNED_GRANT_LIFETIME_MS).contains(&lifetime)
            || object.state != GovernedObjectState::Active
            || revoked_at.is_some_and(|value| value < issued_at)
        {
            return Err(StorageGovernanceError::InvalidGrant);
        }
        Ok(Self {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            grant_id,
            key_version,
            tenant_id: object.tenant_id,
            object_id: object.object_id.clone(),
            immutable_revision: object.immutable_revision,
            cache_generation: object.cache_generation,
            object_checksum: object.checksum.clone(),
            operation,
            issued_at,
            expires_at,
            nonce_digest,
            revoked_at,
        })
    }

    pub fn revoke(&mut self, revoked_at: TimestampMillis) -> Result<(), StorageGovernanceError> {
        if revoked_at < self.issued_at {
            return Err(StorageGovernanceError::InvalidGrant);
        }
        match self.revoked_at {
            None => {
                self.revoked_at = Some(revoked_at);
                Ok(())
            }
            Some(existing) if existing == revoked_at => Ok(()),
            Some(_) => Err(StorageGovernanceError::InvalidGrant),
        }
    }

    #[must_use]
    pub const fn grant_id(&self) -> SignedGrantId {
        self.grant_id
    }

    #[must_use]
    pub const fn key_version(&self) -> SignedGrantKeyVersion {
        self.key_version
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub fn object_id(&self) -> &GovernedObjectId {
        &self.object_id
    }

    #[must_use]
    pub const fn operation(&self) -> StorageOperation {
        self.operation
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
    pub fn nonce_digest(&self) -> &ChecksumSha256 {
        &self.nonce_digest
    }

    #[must_use]
    pub const fn revoked_at(&self) -> Option<TimestampMillis> {
        self.revoked_at
    }

    pub fn authorizes_persisted_proof(
        &self,
        object: &GovernedObject,
        operation: StorageOperation,
        now: TimestampMillis,
        presented_nonce_digest: &ChecksumSha256,
    ) -> bool {
        self.tenant_id == object.tenant_id
            && self.object_id == object.object_id
            && self.immutable_revision == object.immutable_revision
            && self.cache_generation == object.cache_generation
            && self.object_checksum == object.checksum
            && self.operation == operation
            && constant_time_bytes_eq(
                self.nonce_digest.as_str().as_bytes(),
                presented_nonce_digest.as_str().as_bytes(),
            )
            && now.get() >= self.issued_at.get()
            && now.get() < self.expires_at.get()
            && self.revoked_at.is_none_or(|revoked_at| now < revoked_at)
            && ChecksumSha256::parse(self.nonce_digest.as_str()).is_ok()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedObjectGrantWire {
    schema_version: u16,
    grant_id: SignedGrantId,
    key_version: SignedGrantKeyVersion,
    tenant_id: TenantId,
    object_id: GovernedObjectId,
    immutable_revision: u64,
    cache_generation: u64,
    object_checksum: ChecksumSha256,
    operation: StorageOperation,
    issued_at: TimestampMillis,
    expires_at: TimestampMillis,
    nonce_digest: ChecksumSha256,
    revoked_at: Option<TimestampMillis>,
}

impl<'de> Deserialize<'de> for SignedObjectGrant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = SignedObjectGrantWire::deserialize(deserializer)?;
        let lifetime = wire
            .expires_at
            .get()
            .checked_sub(wire.issued_at.get())
            .ok_or_else(|| de::Error::custom("signed grant lifetime is invalid"))?;
        if wire.schema_version != STORAGE_GOVERNANCE_SCHEMA_VERSION
            || wire.immutable_revision == 0
            || wire.immutable_revision > MAX_WIRE_INTEGER
            || wire.cache_generation == 0
            || wire.cache_generation > MAX_WIRE_INTEGER
            || wire.key_version.get() == 0
            || !matches!(
                wire.operation,
                StorageOperation::Read | StorageOperation::ReadRange
            )
            || !(1..=MAX_SIGNED_GRANT_LIFETIME_MS).contains(&lifetime)
            || wire.revoked_at.is_some_and(|value| value < wire.issued_at)
        {
            return Err(de::Error::custom("signed object grant is invalid"));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            grant_id: wire.grant_id,
            key_version: wire.key_version,
            tenant_id: wire.tenant_id,
            object_id: wire.object_id,
            immutable_revision: wire.immutable_revision,
            cache_generation: wire.cache_generation,
            object_checksum: wire.object_checksum,
            operation: wire.operation,
            issued_at: wire.issued_at,
            expires_at: wire.expires_at,
            nonce_digest: wire.nonce_digest,
            revoked_at: wire.revoked_at,
        })
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CustomDomainName(String);

impl CustomDomainName {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageGovernanceError> {
        let value = value.into();
        let canonical = value.to_ascii_lowercase();
        let labels = canonical.split('.').collect::<Vec<_>>();
        if value != canonical
            || canonical.len() > MAX_CUSTOM_DOMAIN_LENGTH
            || labels.len() < 2
            || labels.iter().any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
        {
            return Err(StorageGovernanceError::InvalidCustomDomain);
        }
        Ok(Self(canonical))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CustomDomainName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CustomDomainName([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCustomDomain {
    tenant_id: TenantId,
    domain: CustomDomainName,
    verification_version: u64,
    active: bool,
}

impl VerifiedCustomDomain {
    pub fn new(
        tenant_id: TenantId,
        domain: CustomDomainName,
        verification_version: u64,
        active: bool,
    ) -> Result<Self, StorageGovernanceError> {
        if verification_version == 0 || verification_version > MAX_WIRE_INTEGER {
            return Err(StorageGovernanceError::InvalidCustomDomain);
        }
        Ok(Self {
            tenant_id,
            domain,
            verification_version,
            active,
        })
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub fn domain(&self) -> &CustomDomainName {
        &self.domain
    }

    #[must_use]
    pub const fn verification_version(&self) -> u64 {
        self.verification_version
    }

    #[must_use]
    pub const fn active(&self) -> bool {
        self.active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageDenialReason {
    AccessDenied,
    ObjectUnavailable,
    UnsafeMedia,
    GrantRequired,
    GrantInvalid,
    DomainInvalid,
    RetentionLocked,
}

impl StorageDenialReason {
    #[must_use]
    pub const fn public_code(self) -> &'static str {
        let _ = self;
        "storage_access_denied"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageAuthorizationDecision {
    Allow,
    Deny(StorageDenialReason),
}

pub struct StorageAccessRequest<'a> {
    pub actor: StorageActor,
    pub operation: StorageOperation,
    pub surface: StorageAccessSurface,
    pub object: &'a GovernedObject,
    pub now: TimestampMillis,
    pub grant: Option<&'a SignedObjectGrant>,
    pub grant_proof: Option<&'a ChecksumSha256>,
    pub request_domain: Option<&'a CustomDomainName>,
    pub custom_domain: Option<&'a VerifiedCustomDomain>,
}

pub struct StorageAuthorizationPolicy;

impl StorageAuthorizationPolicy {
    #[must_use]
    pub fn evaluate(request: StorageAccessRequest<'_>) -> StorageAuthorizationDecision {
        let object = request.object;
        if request.surface == StorageAccessSurface::DirectOrigin {
            return StorageAuthorizationDecision::Deny(StorageDenialReason::AccessDenied);
        }
        let actor_tenant = match request.actor {
            StorageActor::Anonymous => None,
            StorageActor::Member { tenant_id, .. } | StorageActor::Service { tenant_id, .. } => {
                Some(tenant_id)
            }
        };
        if actor_tenant.is_some_and(|tenant_id| tenant_id != object.tenant_id) {
            return StorageAuthorizationDecision::Deny(StorageDenialReason::AccessDenied);
        }
        let scanner_read = matches!(
            request.actor,
            StorageActor::Service {
                purpose: StorageServicePurpose::MalwareScanner,
                ..
            }
        ) && matches!(
            request.operation,
            StorageOperation::Read | StorageOperation::ReadRange
        );
        if object.state != GovernedObjectState::Active
            && !(scanner_read && object.state == GovernedObjectState::Quarantined)
        {
            return StorageAuthorizationDecision::Deny(StorageDenialReason::ObjectUnavailable);
        }
        if matches!(
            request.operation,
            StorageOperation::Read | StorageOperation::ReadRange
        ) && object.malware != MalwareDisposition::Clean
            && !scanner_read
        {
            return StorageAuthorizationDecision::Deny(StorageDenialReason::UnsafeMedia);
        }
        if request.surface == StorageAccessSurface::CustomDomain {
            let (Some(request_domain), Some(binding)) =
                (request.request_domain, request.custom_domain)
            else {
                return StorageAuthorizationDecision::Deny(StorageDenialReason::DomainInvalid);
            };
            if !binding.active
                || binding.verification_version == 0
                || binding.tenant_id != object.tenant_id
                || binding.domain != *request_domain
            {
                return StorageAuthorizationDecision::Deny(StorageDenialReason::DomainInvalid);
            }
        }

        if request.surface == StorageAccessSurface::SignedRoute {
            let (Some(grant), Some(grant_proof)) = (request.grant, request.grant_proof) else {
                return StorageAuthorizationDecision::Deny(StorageDenialReason::GrantRequired);
            };
            return if grant.authorizes_persisted_proof(
                object,
                request.operation,
                request.now,
                grant_proof,
            ) {
                StorageAuthorizationDecision::Allow
            } else {
                StorageAuthorizationDecision::Deny(StorageDenialReason::GrantInvalid)
            };
        }

        if let Some(grant) = request.grant {
            let Some(grant_proof) = request.grant_proof else {
                return StorageAuthorizationDecision::Deny(StorageDenialReason::GrantRequired);
            };
            if !matches!(
                request.operation,
                StorageOperation::Read | StorageOperation::ReadRange
            ) || !grant.authorizes_persisted_proof(
                object,
                request.operation,
                request.now,
                grant_proof,
            ) {
                return StorageAuthorizationDecision::Deny(StorageDenialReason::GrantInvalid);
            }
            if request.surface == StorageAccessSurface::CustomDomain {
                return StorageAuthorizationDecision::Allow;
            }
        }

        if request.surface == StorageAccessSurface::MediaTransformation {
            let purpose_allowed = matches!(
                request.actor,
                StorageActor::Service {
                    purpose: StorageServicePurpose::MediaProcessor,
                    ..
                }
            );
            let object_allowed = match request.operation {
                StorageOperation::Read | StorageOperation::ReadRange => object.role.is_source(),
                StorageOperation::WriteImmutable => {
                    object.role.is_derivative() && object.visibility == ObjectVisibility::Private
                }
                _ => false,
            };
            return if purpose_allowed && object_allowed {
                StorageAuthorizationDecision::Allow
            } else {
                StorageAuthorizationDecision::Deny(StorageDenialReason::AccessDenied)
            };
        }

        match request.actor {
            StorageActor::Anonymous => {
                if object.visibility == ObjectVisibility::Public
                    && request.surface == StorageAccessSurface::CustomDomain
                    && matches!(
                        request.operation,
                        StorageOperation::Read | StorageOperation::ReadRange
                    )
                {
                    StorageAuthorizationDecision::Allow
                } else {
                    StorageAuthorizationDecision::Deny(StorageDenialReason::GrantRequired)
                }
            }
            StorageActor::Member { role, .. } => {
                if member_allows(role, request.operation) {
                    StorageAuthorizationDecision::Allow
                } else {
                    StorageAuthorizationDecision::Deny(StorageDenialReason::AccessDenied)
                }
            }
            StorageActor::Service { purpose, .. } => {
                if service_allows(purpose, request.operation) {
                    StorageAuthorizationDecision::Allow
                } else {
                    StorageAuthorizationDecision::Deny(StorageDenialReason::AccessDenied)
                }
            }
        }
    }
}

const fn member_allows(role: StorageMemberRole, operation: StorageOperation) -> bool {
    match role {
        StorageMemberRole::Viewer => matches!(
            operation,
            StorageOperation::Read | StorageOperation::ReadRange
        ),
        StorageMemberRole::Editor => !matches!(
            operation,
            StorageOperation::Delete
                | StorageOperation::Restore
                | StorageOperation::Export
                | StorageOperation::PurgeCache
                | StorageOperation::ManageCustomDomain
        ),
        StorageMemberRole::Admin | StorageMemberRole::Owner => true,
    }
}

const fn service_allows(purpose: StorageServicePurpose, operation: StorageOperation) -> bool {
    match purpose {
        StorageServicePurpose::MediaProcessor => matches!(
            operation,
            StorageOperation::Read | StorageOperation::ReadRange | StorageOperation::WriteImmutable
        ),
        StorageServicePurpose::MalwareScanner => matches!(
            operation,
            StorageOperation::Read | StorageOperation::ReadRange
        ),
        StorageServicePurpose::Backfill => matches!(
            operation,
            StorageOperation::Read
                | StorageOperation::ReadRange
                | StorageOperation::WriteImmutable
                | StorageOperation::List
                | StorageOperation::Copy
        ),
        StorageServicePurpose::Export => matches!(
            operation,
            StorageOperation::Read | StorageOperation::ReadRange | StorageOperation::Export
        ),
        StorageServicePurpose::Deletion => matches!(
            operation,
            StorageOperation::Delete | StorageOperation::PurgeCache | StorageOperation::List
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedMediaState {
    Enabled,
    DisabledByIncident,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ManagedMediaInput {
    tenant_id: TenantId,
    object_id: GovernedObjectId,
    immutable_revision: u64,
    cache_generation: u64,
    checksum: ChecksumSha256,
}

impl fmt::Debug for ManagedMediaInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedMediaInput([redacted])")
    }
}

impl ManagedMediaInput {
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub fn deterministic_derivative_key(
        &self,
        normalized_profile_digest: &ChecksumSha256,
    ) -> ChecksumSha256 {
        digest_fields(&[
            self.tenant_id.to_string().as_bytes(),
            self.object_id.as_str().as_bytes(),
            self.immutable_revision.to_string().as_bytes(),
            self.cache_generation.to_string().as_bytes(),
            self.checksum.as_str().as_bytes(),
            normalized_profile_digest.as_str().as_bytes(),
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagedMediaSourcePolicy {
    state: ManagedMediaState,
    max_input_bytes: ByteSize,
    max_output_bytes: ByteSize,
}

impl ManagedMediaSourcePolicy {
    pub fn new(
        state: ManagedMediaState,
        max_input_bytes: ByteSize,
        max_output_bytes: ByteSize,
    ) -> Result<Self, StorageGovernanceError> {
        if max_input_bytes.get() == 0 || max_output_bytes.get() == 0 {
            return Err(StorageGovernanceError::InvalidMediaPolicy);
        }
        Ok(Self {
            state,
            max_input_bytes,
            max_output_bytes,
        })
    }

    pub fn authorize(
        self,
        actor_tenant_id: TenantId,
        object: &GovernedObject,
    ) -> Result<ManagedMediaInput, StorageGovernanceError> {
        if self.state != ManagedMediaState::Enabled {
            return Err(StorageGovernanceError::MediaKillSwitchActive);
        }
        if actor_tenant_id != object.tenant_id
            || object.state != GovernedObjectState::Active
            || object.malware != MalwareDisposition::Clean
            || !object.role.is_source()
            || object.size > self.max_input_bytes
        {
            return Err(StorageGovernanceError::MediaInputDenied);
        }
        Ok(ManagedMediaInput {
            tenant_id: object.tenant_id,
            object_id: object.object_id.clone(),
            immutable_revision: object.immutable_revision,
            cache_generation: object.cache_generation,
            checksum: object.checksum.clone(),
        })
    }

    pub fn authorize_output(
        self,
        input: &ManagedMediaInput,
        output: &GovernedObject,
        normalized_profile_digest: &ChecksumSha256,
    ) -> Result<ManagedMediaOutput, StorageGovernanceError> {
        if self.state != ManagedMediaState::Enabled {
            return Err(StorageGovernanceError::MediaKillSwitchActive);
        }
        let derivative_identity = input.deterministic_derivative_key(normalized_profile_digest);
        if output.tenant_id != input.tenant_id
            || !output.role.is_derivative()
            || output.visibility != ObjectVisibility::Private
            || output.state != GovernedObjectState::Active
            || output.malware != MalwareDisposition::Clean
            || output.size > self.max_output_bytes
            || output
                .object_id
                .as_str()
                .rsplit('/')
                .next()
                .is_none_or(|segment| segment != derivative_identity.as_str())
        {
            return Err(StorageGovernanceError::MediaOutputDenied);
        }
        Ok(ManagedMediaOutput {
            tenant_id: input.tenant_id,
            derivative_identity,
            output_resource_digest: output.audit_digest(),
            output_checksum: output.checksum.clone(),
            output_size: output.size,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ManagedMediaOutput {
    tenant_id: TenantId,
    derivative_identity: ChecksumSha256,
    output_resource_digest: ChecksumSha256,
    output_checksum: ChecksumSha256,
    output_size: ByteSize,
}

impl fmt::Debug for ManagedMediaOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedMediaOutput")
            .field("tenant_id", &self.tenant_id)
            .field("output_size", &self.output_size)
            .finish_non_exhaustive()
    }
}

impl ManagedMediaOutput {
    #[must_use]
    pub fn derivative_identity(&self) -> &ChecksumSha256 {
        &self.derivative_identity
    }

    #[must_use]
    pub fn output_resource_digest(&self) -> &ChecksumSha256 {
        &self.output_resource_digest
    }

    #[must_use]
    pub fn output_checksum(&self) -> &ChecksumSha256 {
        &self.output_checksum
    }

    #[must_use]
    pub const fn output_size(&self) -> ByteSize {
        self.output_size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageResponsePolicy {
    headers: BTreeMap<&'static str, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageHttpMethod {
    Get,
    Head,
    Options,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedRangeResponse {
    status: u16,
    content_length: u64,
    content_range_start: Option<u64>,
    content_range_end_inclusive: Option<u64>,
    complete_size: u64,
}

impl VerifiedRangeResponse {
    pub fn new(
        status: u16,
        content_length: u64,
        range: Option<(u64, u64)>,
        complete_size: u64,
    ) -> Result<Self, StorageGovernanceError> {
        if complete_size == 0 || content_length == 0 {
            return Err(StorageGovernanceError::InvalidRangeResponse);
        }
        let (start, end) = match (status, range) {
            (200, None) if content_length == complete_size => (None, None),
            (206, Some((start, end_exclusive)))
                if start < end_exclusive
                    && end_exclusive <= complete_size
                    && end_exclusive - start == content_length =>
            {
                (Some(start), Some(end_exclusive - 1))
            }
            _ => return Err(StorageGovernanceError::InvalidRangeResponse),
        };
        Ok(Self {
            status,
            content_length,
            content_range_start: start,
            content_range_end_inclusive: end,
            complete_size,
        })
    }

    #[must_use]
    pub const fn status(self) -> u16 {
        self.status
    }

    #[must_use]
    pub const fn content_length(self) -> u64 {
        self.content_length
    }

    #[must_use]
    pub fn content_range(self) -> Option<String> {
        self.content_range_start
            .zip(self.content_range_end_inclusive)
            .map(|(start, end)| format!("bytes {start}-{end}/{}", self.complete_size))
    }
}

impl StorageResponsePolicy {
    pub fn for_preflight(
        request_origin: &str,
        requested_method: StorageHttpMethod,
        requested_headers: &[&str],
        allowed_origins: &BTreeSet<String>,
    ) -> Result<Self, StorageGovernanceError> {
        if !matches!(
            requested_method,
            StorageHttpMethod::Get | StorageHttpMethod::Head
        ) || requested_headers.len() > 8
            || requested_headers.iter().any(|header| {
                !matches!(
                    header.to_ascii_lowercase().as_str(),
                    "range" | "if-none-match" | "if-range"
                )
            })
        {
            return Err(StorageGovernanceError::CorsDenied);
        }
        let mut policy = Self::for_object(
            "application/octet-stream",
            ObjectVisibility::Private,
            Some(request_origin),
            allowed_origins,
            true,
        )?;
        policy
            .headers
            .insert("access-control-max-age", "600".to_owned());
        Ok(policy)
    }

    pub fn for_object(
        content_type: &str,
        visibility: ObjectVisibility,
        request_origin: Option<&str>,
        allowed_origins: &BTreeSet<String>,
        download: bool,
    ) -> Result<Self, StorageGovernanceError> {
        let content_type = ContentType::parse(content_type)
            .map_err(|_| StorageGovernanceError::InvalidResponsePolicy)?;
        let mut headers = BTreeMap::from([
            ("accept-ranges", "bytes".to_owned()),
            (
                "content-security-policy",
                "sandbox; default-src 'none'".to_owned(),
            ),
            ("content-type", content_type.as_str().to_owned()),
            ("cross-origin-resource-policy", "cross-origin".to_owned()),
            ("x-content-type-options", "nosniff".to_owned()),
        ]);
        headers.insert(
            "cache-control",
            if visibility == ObjectVisibility::Public {
                "public, max-age=31536000, immutable".to_owned()
            } else {
                "private, no-store, max-age=0".to_owned()
            },
        );
        headers.insert(
            "content-disposition",
            if download || !is_safe_inline_content_type(content_type.as_str()) {
                "attachment; filename=media.bin"
            } else {
                "inline"
            }
            .to_owned(),
        );
        if let Some(origin) = request_origin {
            if !is_canonical_origin(origin) || !allowed_origins.contains(origin) {
                return Err(StorageGovernanceError::CorsDenied);
            }
            headers.insert("access-control-allow-origin", origin.to_owned());
            headers.insert(
                "access-control-allow-methods",
                "GET, HEAD, OPTIONS".to_owned(),
            );
            headers.insert(
                "access-control-allow-headers",
                "Range, If-None-Match, If-Range".to_owned(),
            );
            headers.insert(
                "access-control-expose-headers",
                "Accept-Ranges, Content-Length, Content-Range, ETag".to_owned(),
            );
            headers.insert("vary", "Origin".to_owned());
        }
        Ok(Self { headers })
    }

    #[must_use]
    pub fn headers(&self) -> &BTreeMap<&'static str, String> {
        &self.headers
    }

    pub fn with_verified_range(
        mut self,
        range: VerifiedRangeResponse,
    ) -> Result<Self, StorageGovernanceError> {
        self.headers
            .insert("content-length", range.content_length().to_string());
        if let Some(content_range) = range.content_range() {
            self.headers.insert("content-range", content_range);
        }
        Ok(self)
    }
}

fn is_safe_inline_content_type(value: &str) -> bool {
    value.starts_with("video/")
        || value.starts_with("audio/")
        || matches!(
            value,
            "image/jpeg"
                | "image/png"
                | "image/webp"
                | "image/gif"
                | "text/vtt"
                | "application/vnd.apple.mpegurl"
                | "application/dash+xml"
        )
}

fn is_canonical_origin(value: &str) -> bool {
    CorsOriginV1::parse(value).is_ok() || is_canonical_local_http_origin(value)
}

fn is_canonical_local_http_origin(value: &str) -> bool {
    let Some(authority) = value.strip_prefix("http://") else {
        return false;
    };
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) if !port.contains(':') => (host, Some(port)),
        Some(_) => return false,
        None => (authority, None),
    };
    matches!(host, "localhost" | "127.0.0.1")
        && port.is_none_or(|port| {
            port.parse::<u16>()
                .ok()
                .is_some_and(|parsed| parsed != 0 && parsed != 80 && parsed.to_string() == port)
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CacheInvalidationPlan {
    tenant_id: TenantId,
    object_id: GovernedObjectId,
    from_generation: u64,
    to_generation: u64,
    from_visibility: ObjectVisibility,
    to_visibility: ObjectVisibility,
    changed_at: TimestampMillis,
    deadline: TimestampMillis,
    purge_positive: bool,
    purge_negative: bool,
}

impl CacheInvalidationPlan {
    pub fn privacy_change(
        object: &GovernedObject,
        to_visibility: ObjectVisibility,
        to_generation: u64,
        changed_at: TimestampMillis,
        slo_ms: i64,
    ) -> Result<Self, StorageGovernanceError> {
        let expected_generation = object
            .cache_generation
            .checked_add(1)
            .filter(|value| *value <= MAX_WIRE_INTEGER)
            .ok_or(StorageGovernanceError::InvalidCacheTransition)?;
        if to_visibility == object.visibility
            || to_generation != expected_generation
            || !(1..=MAX_CACHE_INVALIDATION_SLO_MS).contains(&slo_ms)
        {
            return Err(StorageGovernanceError::InvalidCacheTransition);
        }
        let deadline_ms = changed_at
            .get()
            .checked_add(slo_ms)
            .filter(|value| *value <= MAX_TIMESTAMP_MS)
            .ok_or(StorageGovernanceError::InvalidCacheTransition)?;
        Ok(Self {
            tenant_id: object.tenant_id,
            object_id: object.object_id.clone(),
            from_generation: object.cache_generation,
            to_generation,
            from_visibility: object.visibility,
            to_visibility,
            changed_at,
            deadline: TimestampMillis::new(deadline_ms)
                .map_err(|_| StorageGovernanceError::InvalidCacheTransition)?,
            purge_positive: true,
            purge_negative: true,
        })
    }

    #[must_use]
    pub const fn deadline(&self) -> TimestampMillis {
        self.deadline
    }
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }
    #[must_use]
    pub fn object_id(&self) -> &GovernedObjectId {
        &self.object_id
    }
    #[must_use]
    pub const fn from_generation(&self) -> u64 {
        self.from_generation
    }
    #[must_use]
    pub const fn to_generation(&self) -> u64 {
        self.to_generation
    }
    #[must_use]
    pub const fn purges_positive_and_negative(&self) -> bool {
        self.purge_positive && self.purge_negative
    }
    #[must_use]
    pub fn cache_tag_digest(&self) -> ChecksumSha256 {
        digest_fields(&[
            self.tenant_id.to_string().as_bytes(),
            self.object_id.as_str().as_bytes(),
            self.from_generation.to_string().as_bytes(),
            self.to_generation.to_string().as_bytes(),
            self.from_visibility.stable_code().as_bytes(),
            self.to_visibility.stable_code().as_bytes(),
            self.changed_at.get().to_string().as_bytes(),
            self.deadline.get().to_string().as_bytes(),
        ])
    }

    pub fn verify_receipt(
        &self,
        receipt: &CachePurgeReceipt,
    ) -> Result<(), StorageGovernanceError> {
        if receipt.schema_version != STORAGE_GOVERNANCE_SCHEMA_VERSION
            || receipt.tenant_id != self.tenant_id
            || receipt.object_id != self.object_id
            || receipt.from_generation != self.from_generation
            || receipt.plan_digest != self.cache_tag_digest()
            || !receipt.positive_absent
            || !receipt.negative_absent
            || receipt.observed_at < self.changed_at
            || receipt.observed_at > self.deadline
        {
            return Err(StorageGovernanceError::CachePurgeUnverified);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachePurgeReceipt {
    schema_version: u16,
    tenant_id: TenantId,
    object_id: GovernedObjectId,
    from_generation: u64,
    plan_digest: ChecksumSha256,
    provider_receipt_digest: ChecksumSha256,
    observed_at: TimestampMillis,
    positive_absent: bool,
    negative_absent: bool,
}

impl CachePurgeReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn verified_absence(
        tenant_id: TenantId,
        object_id: GovernedObjectId,
        from_generation: u64,
        plan_digest: ChecksumSha256,
        provider_receipt_digest: ChecksumSha256,
        observed_at: TimestampMillis,
        positive_absent: bool,
        negative_absent: bool,
    ) -> Result<Self, StorageGovernanceError> {
        if from_generation == 0 || from_generation > MAX_WIRE_INTEGER {
            return Err(StorageGovernanceError::CachePurgeUnverified);
        }
        Ok(Self {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            tenant_id,
            object_id,
            from_generation,
            plan_digest,
            provider_receipt_digest,
            observed_at,
            positive_absent,
            negative_absent,
        })
    }

    #[must_use]
    pub fn provider_receipt_digest(&self) -> &ChecksumSha256 {
        &self.provider_receipt_digest
    }

    #[must_use]
    pub const fn observed_at(&self) -> TimestampMillis {
        self.observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleObject {
    pub tenant_id: TenantId,
    pub object_id: GovernedObjectId,
    pub role: GovernedObjectRole,
    pub checksum: ChecksumSha256,
    pub size: ByteSize,
    pub retention_until: Option<TimestampMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleInventory {
    tenant_id: TenantId,
    subject_digest: ChecksumSha256,
    objects: BTreeMap<GovernedObjectId, LifecycleObject>,
    roles: BTreeSet<GovernedObjectRole>,
    total_bytes: ByteSize,
    digest: ChecksumSha256,
}

impl LifecycleInventory {
    pub fn new(
        tenant_id: TenantId,
        subject_digest: ChecksumSha256,
        required_roles: &BTreeSet<GovernedObjectRole>,
        objects: Vec<LifecycleObject>,
    ) -> Result<Self, StorageGovernanceError> {
        let closed_roles = GovernedObjectRole::ALL.into_iter().collect::<BTreeSet<_>>();
        if objects.is_empty()
            || objects.len() > MAX_LIFECYCLE_OBJECTS
            || required_roles != &closed_roles
        {
            return Err(StorageGovernanceError::InvalidInventory);
        }
        let mut by_id = BTreeMap::new();
        let mut roles = BTreeSet::new();
        let mut total = ByteSize::new(0).map_err(|_| StorageGovernanceError::InvalidInventory)?;
        for object in objects {
            let tenant_prefix = format!("tenants/{tenant_id}/");
            if object.tenant_id != tenant_id
                || object.size.get() == 0
                || !object.object_id.as_str().starts_with(&tenant_prefix)
            {
                return Err(StorageGovernanceError::InvalidInventory);
            }
            roles.insert(object.role);
            total = total
                .checked_add(object.size)
                .map_err(|_| StorageGovernanceError::InvalidInventory)?;
            if by_id.insert(object.object_id.clone(), object).is_some() {
                return Err(StorageGovernanceError::InvalidInventory);
            }
        }
        if roles != closed_roles {
            return Err(StorageGovernanceError::IncompleteInventory);
        }
        let mut fields = vec![tenant_id.to_string(), subject_digest.as_str().to_owned()];
        for object in by_id.values() {
            fields.extend([
                object.object_id.as_str().to_owned(),
                object.role.stable_code().to_owned(),
                object.checksum.as_str().to_owned(),
                object.size.get().to_string(),
                object
                    .retention_until
                    .map_or_else(|| "none".to_owned(), |value| value.get().to_string()),
            ]);
        }
        let refs = fields.iter().map(String::as_bytes).collect::<Vec<_>>();
        let digest = digest_fields(&refs);
        Ok(Self {
            tenant_id,
            subject_digest,
            objects: by_id,
            roles,
            total_bytes: total,
            digest,
        })
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }
    #[must_use]
    pub fn subject_digest(&self) -> &ChecksumSha256 {
        &self.subject_digest
    }
    #[must_use]
    pub fn digest(&self) -> &ChecksumSha256 {
        &self.digest
    }
    #[must_use]
    pub fn object_ids(&self) -> impl ExactSizeIterator<Item = &GovernedObjectId> {
        self.objects.keys()
    }
    #[must_use]
    pub fn objects(&self) -> impl ExactSizeIterator<Item = &LifecycleObject> {
        self.objects.values()
    }
    #[must_use]
    pub const fn total_bytes(&self) -> ByteSize {
        self.total_bytes
    }
    #[must_use]
    pub fn roles(&self) -> &BTreeSet<GovernedObjectRole> {
        &self.roles
    }

    #[must_use]
    pub fn contains_governed_object(&self, object: &GovernedObject) -> bool {
        object.tenant_id == self.tenant_id
            && self.objects.get(&object.object_id).is_some_and(|entry| {
                entry.role == object.role
                    && entry.checksum == object.checksum
                    && entry.size == object.size
            })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageExportPlan {
    tenant_id: TenantId,
    inventory_digest: ChecksumSha256,
    targets: BTreeMap<GovernedObjectId, ChecksumSha256>,
    plan_digest: ChecksumSha256,
}

impl fmt::Debug for StorageExportPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageExportPlan")
            .field("tenant_id", &self.tenant_id)
            .field("target_count", &self.targets.len())
            .field("plan_digest", &self.plan_digest)
            .finish()
    }
}

impl StorageExportPlan {
    #[must_use]
    pub fn from_manifest(inventory: &LifecycleInventory) -> Self {
        let targets = inventory
            .objects
            .values()
            .filter(|object| object.role != GovernedObjectRole::MultipartSession)
            .map(|object| (object.object_id.clone(), object.checksum.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut values = vec![
            inventory.tenant_id.to_string(),
            inventory.digest.as_str().to_owned(),
            "manifest_only_export".to_owned(),
        ];
        for (object_id, checksum) in &targets {
            values.extend([object_id.as_str().to_owned(), checksum.as_str().to_owned()]);
        }
        let refs = values.iter().map(String::as_bytes).collect::<Vec<_>>();
        Self {
            tenant_id: inventory.tenant_id,
            inventory_digest: inventory.digest.clone(),
            targets,
            plan_digest: digest_fields(&refs),
        }
    }

    #[must_use]
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    #[must_use]
    pub fn plan_digest(&self) -> &ChecksumSha256 {
        &self.plan_digest
    }

    #[must_use]
    pub fn still_matches(&self, inventory: &LifecycleInventory) -> bool {
        self.tenant_id == inventory.tenant_id && self.inventory_digest == inventory.digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalHoldState {
    Active,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageLegalHold {
    tenant_id: TenantId,
    subject_digest: ChecksumSha256,
    state: LegalHoldState,
    created_at: TimestampMillis,
    released_at: Option<TimestampMillis>,
}

impl StorageLegalHold {
    #[must_use]
    pub fn active(
        tenant_id: TenantId,
        subject_digest: ChecksumSha256,
        created_at: TimestampMillis,
    ) -> Self {
        Self {
            tenant_id,
            subject_digest,
            state: LegalHoldState::Active,
            created_at,
            released_at: None,
        }
    }

    pub fn release(&mut self, released_at: TimestampMillis) -> Result<(), StorageGovernanceError> {
        if released_at.get() < self.created_at.get() {
            return Err(StorageGovernanceError::InvalidHold);
        }
        if self.state == LegalHoldState::Released {
            return if self.released_at == Some(released_at) {
                Ok(())
            } else {
                Err(StorageGovernanceError::InvalidHold)
            };
        }
        self.state = LegalHoldState::Released;
        self.released_at = Some(released_at);
        Ok(())
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, LegalHoldState::Active)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionStage {
    Planned,
    Tombstoned,
    OriginDeleted,
    CachePurged,
    BackupDeleted,
    Verified,
    Complete,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionGuardSnapshot {
    schema_version: u16,
    tenant_id: TenantId,
    subject_digest: ChecksumSha256,
    observed_at: TimestampMillis,
    authority_revision: u64,
    active_hold: bool,
    retention_until: Option<TimestampMillis>,
}

impl DeletionGuardSnapshot {
    pub fn new(
        tenant_id: TenantId,
        subject_digest: ChecksumSha256,
        observed_at: TimestampMillis,
        authority_revision: u64,
        active_hold: bool,
        retention_until: Option<TimestampMillis>,
    ) -> Result<Self, StorageGovernanceError> {
        if authority_revision == 0
            || authority_revision > MAX_WIRE_INTEGER
            || retention_until.is_some_and(|value| value < observed_at)
        {
            return Err(StorageGovernanceError::InvalidDeletionGuard);
        }
        Ok(Self {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            tenant_id,
            subject_digest,
            observed_at,
            authority_revision,
            active_hold,
            retention_until,
        })
    }

    fn permits(
        &self,
        tenant_id: TenantId,
        subject_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> Result<(), StorageGovernanceError> {
        if self.schema_version != STORAGE_GOVERNANCE_SCHEMA_VERSION
            || self.tenant_id != tenant_id
            || &self.subject_digest != subject_digest
            || self.observed_at > now
        {
            return Err(StorageGovernanceError::InvalidDeletionGuard);
        }
        if self.active_hold {
            return Err(StorageGovernanceError::LegalHoldActive);
        }
        if self.retention_until.is_some_and(|value| value > now) {
            return Err(StorageGovernanceError::RetentionActive);
        }
        Ok(())
    }

    fn matches_subject(
        &self,
        tenant_id: TenantId,
        subject_digest: &ChecksumSha256,
        now: TimestampMillis,
    ) -> bool {
        self.schema_version == STORAGE_GOVERNANCE_SCHEMA_VERSION
            && self.tenant_id == tenant_id
            && &self.subject_digest == subject_digest
            && self.observed_at <= now
    }

    #[must_use]
    pub const fn authority_revision(&self) -> u64 {
        self.authority_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionEvidenceReceipt {
    schema_version: u16,
    tenant_id: TenantId,
    inventory_digest: ChecksumSha256,
    stage: DeletionStage,
    target_digest: ChecksumSha256,
    provider_receipt_digest: ChecksumSha256,
    observed_at: TimestampMillis,
    positive_cache_absent: bool,
    negative_cache_absent: bool,
}

impl DeletionEvidenceReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn verified(
        tenant_id: TenantId,
        inventory_digest: ChecksumSha256,
        stage: DeletionStage,
        target_digest: ChecksumSha256,
        provider_receipt_digest: ChecksumSha256,
        observed_at: TimestampMillis,
        positive_cache_absent: bool,
        negative_cache_absent: bool,
    ) -> Result<Self, StorageGovernanceError> {
        if deletion_stage_key(stage).is_none()
            || (stage == DeletionStage::CachePurged
                && !(positive_cache_absent && negative_cache_absent))
            || (stage != DeletionStage::CachePurged
                && (positive_cache_absent || negative_cache_absent))
        {
            return Err(StorageGovernanceError::InvalidDeletionEvidence);
        }
        Ok(Self {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            tenant_id,
            inventory_digest,
            stage,
            target_digest,
            provider_receipt_digest,
            observed_at,
            positive_cache_absent,
            negative_cache_absent,
        })
    }

    #[must_use]
    pub const fn stage(&self) -> DeletionStage {
        self.stage
    }

    #[must_use]
    pub fn provider_receipt_digest(&self) -> &ChecksumSha256 {
        &self.provider_receipt_digest
    }

    #[must_use]
    pub fn target_digest(&self) -> &ChecksumSha256 {
        &self.target_digest
    }

    #[must_use]
    pub const fn observed_at(&self) -> TimestampMillis {
        self.observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionWorkflow {
    correlation_id: CorrelationId,
    tenant_id: TenantId,
    subject_digest: ChecksumSha256,
    inventory_digest: ChecksumSha256,
    object_digests: BTreeMap<GovernedObjectId, (GovernedObjectRole, ChecksumSha256)>,
    stage: DeletionStage,
    evidence: BTreeMap<DeletionStageKey, ChecksumSha256>,
    requested_at: TimestampMillis,
    restore_deadline: TimestampMillis,
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeletionStageKey {
    Tombstone,
    OriginDelete,
    CachePurge,
    BackupDelete,
    Verify,
}

const fn deletion_stage_key(stage: DeletionStage) -> Option<DeletionStageKey> {
    match stage {
        DeletionStage::Tombstoned => Some(DeletionStageKey::Tombstone),
        DeletionStage::OriginDeleted => Some(DeletionStageKey::OriginDelete),
        DeletionStage::CachePurged => Some(DeletionStageKey::CachePurge),
        DeletionStage::BackupDeleted => Some(DeletionStageKey::BackupDelete),
        DeletionStage::Verified => Some(DeletionStageKey::Verify),
        DeletionStage::Planned | DeletionStage::Complete | DeletionStage::Restored => None,
    }
}

impl DeletionWorkflow {
    pub fn plan(
        correlation_id: CorrelationId,
        inventory: &LifecycleInventory,
        guard: &DeletionGuardSnapshot,
        requested_at: TimestampMillis,
    ) -> Result<Self, StorageGovernanceError> {
        guard.permits(inventory.tenant_id, &inventory.subject_digest, requested_at)?;
        if inventory.objects.values().any(|object| {
            object
                .retention_until
                .is_some_and(|deadline| deadline.get() > requested_at.get())
        }) {
            return Err(StorageGovernanceError::RetentionActive);
        }
        let restore_deadline = TimestampMillis::new(
            requested_at
                .get()
                .checked_add(DELETION_RESTORE_GRACE_MS)
                .filter(|value| *value <= MAX_TIMESTAMP_MS)
                .ok_or(StorageGovernanceError::InvalidDeletionTransition)?,
        )
        .map_err(|_| StorageGovernanceError::InvalidDeletionTransition)?;
        let object_digests = inventory
            .objects
            .values()
            .map(|object| {
                (
                    object.object_id.clone(),
                    (
                        object.role,
                        digest_fields(&[
                            object.object_id.as_str().as_bytes(),
                            object.checksum.as_str().as_bytes(),
                            object.size.get().to_string().as_bytes(),
                        ]),
                    ),
                )
            })
            .collect();
        Ok(Self {
            correlation_id,
            tenant_id: inventory.tenant_id,
            subject_digest: inventory.subject_digest.clone(),
            inventory_digest: inventory.digest.clone(),
            object_digests,
            stage: DeletionStage::Planned,
            evidence: BTreeMap::new(),
            requested_at,
            restore_deadline,
            revision: 1,
        })
    }

    pub fn record(
        &mut self,
        inventory: &LifecycleInventory,
        receipt: &DeletionEvidenceReceipt,
        guard: &DeletionGuardSnapshot,
        now: TimestampMillis,
    ) -> Result<(), StorageGovernanceError> {
        if inventory.tenant_id != self.tenant_id || inventory.digest != self.inventory_digest {
            return Err(StorageGovernanceError::InventoryChanged);
        }
        guard.permits(self.tenant_id, &self.subject_digest, now)?;
        if inventory.objects.values().any(|object| {
            object
                .retention_until
                .is_some_and(|deadline| deadline > now)
        }) {
            return Err(StorageGovernanceError::RetentionActive);
        }
        let next = receipt.stage;
        if let Some(key) = deletion_stage_key(next) {
            let expected_digest = self.expected_target_digest(next)?;
            if receipt.schema_version != STORAGE_GOVERNANCE_SCHEMA_VERSION
                || receipt.tenant_id != self.tenant_id
                || receipt.inventory_digest != self.inventory_digest
                || receipt.target_digest != expected_digest
                || receipt.observed_at < self.requested_at
                || receipt.observed_at > now
            {
                return Err(StorageGovernanceError::InvalidDeletionEvidence);
            }
            if let Some(existing) = self.evidence.get(&key) {
                return if existing == &receipt.provider_receipt_digest {
                    Ok(())
                } else {
                    Err(StorageGovernanceError::EvidenceConflict)
                };
            }
        }
        let (expected, key) = match self.stage {
            DeletionStage::Planned => (DeletionStage::Tombstoned, DeletionStageKey::Tombstone),
            DeletionStage::Tombstoned => {
                (DeletionStage::OriginDeleted, DeletionStageKey::OriginDelete)
            }
            DeletionStage::OriginDeleted => {
                (DeletionStage::CachePurged, DeletionStageKey::CachePurge)
            }
            DeletionStage::CachePurged => {
                (DeletionStage::BackupDeleted, DeletionStageKey::BackupDelete)
            }
            DeletionStage::BackupDeleted => (DeletionStage::Verified, DeletionStageKey::Verify),
            DeletionStage::Verified => {
                return Err(StorageGovernanceError::InvalidDeletionTransition);
            }
            DeletionStage::Complete => {
                return if next == DeletionStage::Complete {
                    Ok(())
                } else {
                    Err(StorageGovernanceError::InvalidDeletionTransition)
                };
            }
            DeletionStage::Restored => {
                return Err(StorageGovernanceError::InvalidDeletionTransition);
            }
        };
        if next != expected {
            return Err(StorageGovernanceError::InvalidDeletionTransition);
        }
        self.evidence
            .insert(key, receipt.provider_receipt_digest.clone());
        self.stage = next;
        self.revision = self
            .revision
            .checked_add(1)
            .filter(|value| *value <= MAX_WIRE_INTEGER)
            .ok_or(StorageGovernanceError::InvalidDeletionTransition)?;
        Ok(())
    }

    pub fn preflight_transition(
        &self,
        inventory: &LifecycleInventory,
        guard: &DeletionGuardSnapshot,
        now: TimestampMillis,
    ) -> Result<(), StorageGovernanceError> {
        if inventory.tenant_id != self.tenant_id || inventory.digest != self.inventory_digest {
            return Err(StorageGovernanceError::InventoryChanged);
        }
        guard.permits(self.tenant_id, &self.subject_digest, now)?;
        if inventory.objects.values().any(|object| {
            object
                .retention_until
                .is_some_and(|deadline| deadline > now)
        }) {
            return Err(StorageGovernanceError::RetentionActive);
        }
        Ok(())
    }

    fn expected_target_digest(
        &self,
        stage: DeletionStage,
    ) -> Result<ChecksumSha256, StorageGovernanceError> {
        let stage_code = match stage {
            DeletionStage::Tombstoned => "tombstoned",
            DeletionStage::OriginDeleted => "origin_deleted",
            DeletionStage::CachePurged => "cache_positive_negative_purged",
            DeletionStage::BackupDeleted => "backup_deleted",
            DeletionStage::Verified => "verified_absent",
            _ => return Err(StorageGovernanceError::InvalidDeletionTransition),
        };
        let mut values = vec![
            self.inventory_digest.as_str().to_owned(),
            stage_code.to_owned(),
        ];
        for (object_id, (role, digest)) in &self.object_digests {
            let include = match stage {
                DeletionStage::OriginDeleted => *role != GovernedObjectRole::BackupCopy,
                DeletionStage::BackupDeleted => *role == GovernedObjectRole::BackupCopy,
                _ => true,
            };
            if include {
                values.extend([
                    object_id.as_str().to_owned(),
                    role.stable_code().to_owned(),
                    digest.as_str().to_owned(),
                ]);
            }
        }
        let refs = values.iter().map(String::as_bytes).collect::<Vec<_>>();
        Ok(digest_fields(&refs))
    }

    pub fn evidence_target_digest(
        &self,
        stage: DeletionStage,
    ) -> Result<ChecksumSha256, StorageGovernanceError> {
        self.expected_target_digest(stage)
    }

    pub fn complete(
        &mut self,
        inventory: &LifecycleInventory,
        guard: &DeletionGuardSnapshot,
        now: TimestampMillis,
    ) -> Result<(), StorageGovernanceError> {
        if inventory.tenant_id != self.tenant_id || inventory.digest != self.inventory_digest {
            return Err(StorageGovernanceError::InventoryChanged);
        }
        guard.permits(self.tenant_id, &self.subject_digest, now)?;
        if self.stage == DeletionStage::Complete {
            return Ok(());
        }
        if self.stage != DeletionStage::Verified || self.evidence.len() != 5 {
            return Err(StorageGovernanceError::DeletionIncomplete);
        }
        self.stage = DeletionStage::Complete;
        self.revision = self
            .revision
            .checked_add(1)
            .filter(|value| *value <= MAX_WIRE_INTEGER)
            .ok_or(StorageGovernanceError::InvalidDeletionTransition)?;
        Ok(())
    }

    pub fn restore(
        &mut self,
        inventory: &LifecycleInventory,
        guard: &DeletionGuardSnapshot,
        now: TimestampMillis,
    ) -> Result<(), StorageGovernanceError> {
        if inventory.tenant_id != self.tenant_id || inventory.digest != self.inventory_digest {
            return Err(StorageGovernanceError::InventoryChanged);
        }
        if !guard.matches_subject(self.tenant_id, &self.subject_digest, now) {
            return Err(StorageGovernanceError::InvalidDeletionGuard);
        }
        if now.get() > self.restore_deadline.get() {
            return Err(StorageGovernanceError::RestoreWindowExpired);
        }
        match self.stage {
            DeletionStage::Planned | DeletionStage::Tombstoned => {
                self.stage = DeletionStage::Restored;
                self.revision = self
                    .revision
                    .checked_add(1)
                    .filter(|value| *value <= MAX_WIRE_INTEGER)
                    .ok_or(StorageGovernanceError::InvalidDeletionTransition)?;
                Ok(())
            }
            DeletionStage::Restored => Ok(()),
            _ => Err(StorageGovernanceError::InvalidDeletionTransition),
        }
    }

    pub fn completion_proof(
        &self,
        completed_at: TimestampMillis,
    ) -> Result<ErasureProof, StorageGovernanceError> {
        if self.stage != DeletionStage::Complete
            || completed_at.get() < self.requested_at.get()
            || self.evidence.len() != 5
        {
            return Err(StorageGovernanceError::DeletionIncomplete);
        }
        let mut values = vec![
            self.correlation_id.to_string(),
            self.tenant_id.to_string(),
            self.subject_digest.as_str().to_owned(),
            self.inventory_digest.as_str().to_owned(),
        ];
        values.extend(
            self.evidence
                .values()
                .map(|digest| digest.as_str().to_owned()),
        );
        let refs = values.iter().map(String::as_bytes).collect::<Vec<_>>();
        Ok(ErasureProof {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            correlation_id: self.correlation_id,
            tenant_id: self.tenant_id,
            inventory_digest: self.inventory_digest.clone(),
            object_count: u32::try_from(self.object_digests.len())
                .map_err(|_| StorageGovernanceError::DeletionIncomplete)?,
            completed_at,
            evidence_root: digest_fields(&refs),
        })
    }

    #[must_use]
    pub const fn stage(&self) -> DeletionStage {
        self.stage
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    #[must_use]
    pub const fn requested_at(&self) -> TimestampMillis {
        self.requested_at
    }

    #[must_use]
    pub fn inventory_digest(&self) -> &ChecksumSha256 {
        &self.inventory_digest
    }

    #[must_use]
    pub fn subject_digest(&self) -> &ChecksumSha256 {
        &self.subject_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErasureProof {
    schema_version: u16,
    correlation_id: CorrelationId,
    tenant_id: TenantId,
    inventory_digest: ChecksumSha256,
    object_count: u32,
    completed_at: TimestampMillis,
    evidence_root: ChecksumSha256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuotaPolicy {
    max_bytes: ByteSize,
    max_objects: u64,
}

impl StorageQuotaPolicy {
    pub fn new(max_bytes: ByteSize, max_objects: u64) -> Result<Self, StorageGovernanceError> {
        if max_bytes.get() == 0 || max_objects == 0 || max_objects > MAX_WIRE_INTEGER {
            return Err(StorageGovernanceError::InvalidQuota);
        }
        Ok(Self {
            max_bytes,
            max_objects,
        })
    }

    pub fn reserve(
        self,
        used_bytes: ByteSize,
        used_objects: u64,
        requested_bytes: ByteSize,
    ) -> Result<(), StorageGovernanceError> {
        let next_bytes = used_bytes
            .checked_add(requested_bytes)
            .map_err(|_| StorageGovernanceError::QuotaExceeded)?;
        let next_objects = used_objects
            .checked_add(1)
            .ok_or(StorageGovernanceError::QuotaExceeded)?;
        if next_bytes > self.max_bytes || next_objects > self.max_objects {
            Err(StorageGovernanceError::QuotaExceeded)
        } else {
            Ok(())
        }
    }

    pub fn reserve_atomic(
        self,
        tenant_id: TenantId,
        reservation_id: CorrelationId,
        snapshot: StorageQuotaSnapshot,
        requested_bytes: ByteSize,
        created_at: TimestampMillis,
        expires_at: TimestampMillis,
    ) -> Result<StorageQuotaReservation, StorageGovernanceError> {
        if snapshot.tenant_id != tenant_id || expires_at <= created_at {
            return Err(StorageGovernanceError::InvalidQuota);
        }
        let committed_and_reserved = snapshot
            .used_bytes
            .checked_add(snapshot.reserved_bytes)
            .map_err(|_| StorageGovernanceError::QuotaExceeded)?;
        let next_bytes = committed_and_reserved
            .checked_add(requested_bytes)
            .map_err(|_| StorageGovernanceError::QuotaExceeded)?;
        let next_objects = snapshot
            .used_objects
            .checked_add(snapshot.reserved_objects)
            .and_then(|value| value.checked_add(1))
            .ok_or(StorageGovernanceError::QuotaExceeded)?;
        if next_bytes > self.max_bytes || next_objects > self.max_objects {
            return Err(StorageGovernanceError::QuotaExceeded);
        }
        Ok(StorageQuotaReservation {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            reservation_id,
            tenant_id,
            requested_bytes,
            expected_quota_revision: snapshot.revision,
            created_at,
            expires_at,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageQuotaSnapshot {
    tenant_id: TenantId,
    used_bytes: ByteSize,
    reserved_bytes: ByteSize,
    used_objects: u64,
    reserved_objects: u64,
    revision: u64,
}

impl StorageQuotaSnapshot {
    pub fn new(
        tenant_id: TenantId,
        used_bytes: ByteSize,
        reserved_bytes: ByteSize,
        used_objects: u64,
        reserved_objects: u64,
        revision: u64,
    ) -> Result<Self, StorageGovernanceError> {
        if used_objects > MAX_WIRE_INTEGER
            || reserved_objects > MAX_WIRE_INTEGER
            || revision == 0
            || revision > MAX_WIRE_INTEGER
        {
            return Err(StorageGovernanceError::InvalidQuota);
        }
        Ok(Self {
            tenant_id,
            used_bytes,
            reserved_bytes,
            used_objects,
            reserved_objects,
            revision,
        })
    }

    #[must_use]
    pub const fn revision(self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn tenant_id(self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn used_bytes(self) -> ByteSize {
        self.used_bytes
    }

    #[must_use]
    pub const fn reserved_bytes(self) -> ByteSize {
        self.reserved_bytes
    }

    #[must_use]
    pub const fn used_objects(self) -> u64 {
        self.used_objects
    }

    #[must_use]
    pub const fn reserved_objects(self) -> u64 {
        self.reserved_objects
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageQuotaReservation {
    schema_version: u16,
    reservation_id: CorrelationId,
    tenant_id: TenantId,
    requested_bytes: ByteSize,
    expected_quota_revision: u64,
    created_at: TimestampMillis,
    expires_at: TimestampMillis,
}

impl StorageQuotaReservation {
    #[must_use]
    pub const fn reservation_id(self) -> CorrelationId {
        self.reservation_id
    }
    #[must_use]
    pub const fn tenant_id(self) -> TenantId {
        self.tenant_id
    }
    #[must_use]
    pub const fn requested_bytes(self) -> ByteSize {
        self.requested_bytes
    }
    #[must_use]
    pub const fn expected_quota_revision(self) -> u64 {
        self.expected_quota_revision
    }
    #[must_use]
    pub const fn expires_at(self) -> TimestampMillis {
        self.expires_at
    }
    #[must_use]
    pub const fn created_at(self) -> TimestampMillis {
        self.created_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceAuditEvent {
    sequence: u64,
    occurred_at: TimestampMillis,
    action_code: &'static str,
    outcome_code: &'static str,
    resource_digest: ChecksumSha256,
    previous_digest: ChecksumSha256,
    digest: ChecksumSha256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceAuditChain {
    events: Vec<GovernanceAuditEvent>,
}

impl GovernanceAuditChain {
    #[must_use]
    pub const fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn append(
        &mut self,
        occurred_at: TimestampMillis,
        action_code: &'static str,
        outcome_code: &'static str,
        resource_digest: ChecksumSha256,
    ) -> Result<&GovernanceAuditEvent, StorageGovernanceError> {
        if !safe_code(action_code)
            || !safe_code(outcome_code)
            || self.events.len() >= MAX_LIFECYCLE_OBJECTS
        {
            return Err(StorageGovernanceError::InvalidAuditEvent);
        }
        if self
            .events
            .last()
            .is_some_and(|last| last.occurred_at > occurred_at)
        {
            return Err(StorageGovernanceError::InvalidAuditEvent);
        }
        let sequence = u64::try_from(self.events.len())
            .map_err(|_| StorageGovernanceError::InvalidAuditEvent)?
            + 1;
        let previous_digest = self
            .events
            .last()
            .map_or_else(zero_digest, |event| event.digest.clone());
        let digest = audit_event_digest(
            sequence,
            occurred_at,
            action_code,
            outcome_code,
            &resource_digest,
            &previous_digest,
        );
        self.events.push(GovernanceAuditEvent {
            sequence,
            occurred_at,
            action_code,
            outcome_code,
            resource_digest,
            previous_digest,
            digest,
        });
        self.events
            .last()
            .ok_or(StorageGovernanceError::InvalidAuditEvent)
    }

    #[must_use]
    pub fn verify(&self) -> bool {
        let mut previous = zero_digest();
        for (index, event) in self.events.iter().enumerate() {
            if event.sequence != u64::try_from(index).unwrap_or(u64::MAX) + 1
                || event.previous_digest != previous
                || event.digest
                    != audit_event_digest(
                        event.sequence,
                        event.occurred_at,
                        event.action_code,
                        event.outcome_code,
                        &event.resource_digest,
                        &event.previous_digest,
                    )
            {
                return false;
            }
            previous = event.digest.clone();
        }
        true
    }
}

impl Default for GovernanceAuditChain {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableGovernanceAuditRecord {
    schema_version: u16,
    sequence: u64,
    tenant_id: TenantId,
    correlation_id: CorrelationId,
    principal_digest: ChecksumSha256,
    occurred_at: TimestampMillis,
    action_code: String,
    outcome_code: String,
    resource_digest: ChecksumSha256,
    previous_digest: ChecksumSha256,
    digest: ChecksumSha256,
}

impl DurableGovernanceAuditRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn chained(
        sequence: u64,
        tenant_id: TenantId,
        correlation_id: CorrelationId,
        principal_digest: ChecksumSha256,
        occurred_at: TimestampMillis,
        action_code: impl Into<String>,
        outcome_code: impl Into<String>,
        resource_digest: ChecksumSha256,
        previous_digest: ChecksumSha256,
    ) -> Result<Self, StorageGovernanceError> {
        let action_code = action_code.into();
        let outcome_code = outcome_code.into();
        if sequence == 0
            || sequence > MAX_WIRE_INTEGER
            || !safe_code(&action_code)
            || !safe_code(&outcome_code)
        {
            return Err(StorageGovernanceError::InvalidAuditEvent);
        }
        let digest = durable_audit_digest(
            sequence,
            tenant_id,
            correlation_id,
            &principal_digest,
            occurred_at,
            &action_code,
            &outcome_code,
            &resource_digest,
            &previous_digest,
        );
        Ok(Self {
            schema_version: STORAGE_GOVERNANCE_SCHEMA_VERSION,
            sequence,
            tenant_id,
            correlation_id,
            principal_digest,
            occurred_at,
            action_code,
            outcome_code,
            resource_digest,
            previous_digest,
            digest,
        })
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    #[must_use]
    pub const fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }
    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
    #[must_use]
    pub const fn occurred_at(&self) -> TimestampMillis {
        self.occurred_at
    }
    #[must_use]
    pub fn previous_digest(&self) -> &ChecksumSha256 {
        &self.previous_digest
    }
    #[must_use]
    pub fn digest(&self) -> &ChecksumSha256 {
        &self.digest
    }
    #[must_use]
    pub fn verify(&self) -> bool {
        self.schema_version == STORAGE_GOVERNANCE_SCHEMA_VERSION
            && self.digest
                == durable_audit_digest(
                    self.sequence,
                    self.tenant_id,
                    self.correlation_id,
                    &self.principal_digest,
                    self.occurred_at,
                    &self.action_code,
                    &self.outcome_code,
                    &self.resource_digest,
                    &self.previous_digest,
                )
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum StorageGovernanceError {
    #[error("governed object is invalid")]
    InvalidObject,
    #[error("signed object grant is invalid")]
    InvalidGrant,
    #[error("custom domain is invalid")]
    InvalidCustomDomain,
    #[error("response security policy is invalid")]
    InvalidResponsePolicy,
    #[error("provider range response is inconsistent with the authorized range")]
    InvalidRangeResponse,
    #[error("request origin is not allowed")]
    CorsDenied,
    #[error("cache transition is invalid")]
    InvalidCacheTransition,
    #[error("cache purge and positive/negative absence were not verified within the SLO")]
    CachePurgeUnverified,
    #[error("managed media policy is invalid")]
    InvalidMediaPolicy,
    #[error("managed media is disabled by the incident kill switch")]
    MediaKillSwitchActive,
    #[error("managed media input is denied")]
    MediaInputDenied,
    #[error("managed media output is denied")]
    MediaOutputDenied,
    #[error("lifecycle inventory is invalid")]
    InvalidInventory,
    #[error("lifecycle inventory omits a required role")]
    IncompleteInventory,
    #[error("legal hold is invalid")]
    InvalidHold,
    #[error("deletion guard snapshot is invalid")]
    InvalidDeletionGuard,
    #[error("an active legal hold prevents deletion")]
    LegalHoldActive,
    #[error("retention policy prevents deletion")]
    RetentionActive,
    #[error("lifecycle inventory changed during deletion")]
    InventoryChanged,
    #[error("deletion transition is invalid")]
    InvalidDeletionTransition,
    #[error("deletion evidence conflicts with the recorded evidence")]
    EvidenceConflict,
    #[error("deletion evidence is invalid or unverified")]
    InvalidDeletionEvidence,
    #[error("deletion has not completed")]
    DeletionIncomplete,
    #[error("the deletion restore window expired")]
    RestoreWindowExpired,
    #[error("storage quota policy is invalid")]
    InvalidQuota,
    #[error("storage quota is exceeded")]
    QuotaExceeded,
    #[error("audit event is invalid")]
    InvalidAuditEvent,
}

fn digest_fields(fields: &[&[u8]]) -> ChecksumSha256 {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update(u64::try_from(field.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(field);
    }
    ChecksumSha256::digest_bytes(&hasher.finalize())
}

fn zero_digest() -> ChecksumSha256 {
    ChecksumSha256::parse("0".repeat(64)).expect("constant SHA-256 digest is valid")
}

fn safe_code(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn constant_time_bytes_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let maximum = left.len().max(right.len());
    for index in 0..maximum {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

fn audit_event_digest(
    sequence: u64,
    occurred_at: TimestampMillis,
    action_code: &str,
    outcome_code: &str,
    resource_digest: &ChecksumSha256,
    previous_digest: &ChecksumSha256,
) -> ChecksumSha256 {
    digest_fields(&[
        sequence.to_string().as_bytes(),
        occurred_at.get().to_string().as_bytes(),
        action_code.as_bytes(),
        outcome_code.as_bytes(),
        resource_digest.as_str().as_bytes(),
        previous_digest.as_str().as_bytes(),
    ])
}

#[allow(clippy::too_many_arguments)]
fn durable_audit_digest(
    sequence: u64,
    tenant_id: TenantId,
    correlation_id: CorrelationId,
    principal_digest: &ChecksumSha256,
    occurred_at: TimestampMillis,
    action_code: &str,
    outcome_code: &str,
    resource_digest: &ChecksumSha256,
    previous_digest: &ChecksumSha256,
) -> ChecksumSha256 {
    digest_fields(&[
        b"frame.storage.governance.audit.v1",
        sequence.to_string().as_bytes(),
        tenant_id.to_string().as_bytes(),
        correlation_id.to_string().as_bytes(),
        principal_digest.as_str().as_bytes(),
        occurred_at.get().to_string().as_bytes(),
        action_code.as_bytes(),
        outcome_code.as_bytes(),
        resource_digest.as_str().as_bytes(),
        previous_digest.as_str().as_bytes(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("timestamp")
    }
    fn checksum(value: u8) -> ChecksumSha256 {
        ChecksumSha256::parse(format!("{value:064x}")).expect("checksum")
    }
    fn object(tenant_id: TenantId, visibility: ObjectVisibility) -> GovernedObject {
        GovernedObject::new(
            tenant_id,
            GovernedObjectId::parse(format!("tenants/{tenant_id}/videos/video-1/source-r1"))
                .expect("id"),
            GovernedObjectRole::Source,
            visibility,
            GovernedObjectState::Active,
            MalwareDisposition::Clean,
            1,
            1,
            checksum(1),
            ByteSize::new(10).expect("size"),
            None,
        )
        .expect("object")
    }

    #[test]
    fn every_surface_fails_closed_across_tenants() {
        let tenant = TenantId::new();
        let other = TenantId::new();
        let object = object(tenant, ObjectVisibility::Private);
        for surface in [
            StorageAccessSurface::SameOriginApplication,
            StorageAccessSurface::DirectOrigin,
            StorageAccessSurface::SignedRoute,
            StorageAccessSurface::CustomDomain,
            StorageAccessSurface::MediaTransformation,
        ] {
            let decision = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
                actor: StorageActor::Member {
                    tenant_id: other,
                    user_id: UserId::new(),
                    role: StorageMemberRole::Owner,
                },
                operation: StorageOperation::Read,
                surface,
                object: &object,
                now: timestamp(2),
                grant: None,
                grant_proof: None,
                request_domain: None,
                custom_domain: None,
            });
            assert!(matches!(decision, StorageAuthorizationDecision::Deny(_)));
        }
    }

    #[test]
    fn governed_object_wire_format_revalidates_schema_and_derived_invariants() {
        let object = object(TenantId::new(), ObjectVisibility::Private);
        let mut wire = serde_json::to_value(&object).expect("serialize");
        wire["schema_version"] = serde_json::json!(2);
        assert!(serde_json::from_value::<GovernedObject>(wire).is_err());

        let mut wire = serde_json::to_value(&object).expect("serialize");
        wire["immutable_revision"] = serde_json::json!(0);
        assert!(serde_json::from_value::<GovernedObject>(wire).is_err());

        let mut wire = serde_json::to_value(&object).expect("serialize");
        wire["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<GovernedObject>(wire).is_err());
    }

    #[test]
    fn signed_grants_bind_object_generation_operation_and_expiry() {
        let tenant = TenantId::new();
        let object = object(tenant, ObjectVisibility::Unlisted);
        let proof = checksum(2);
        let grant = SignedObjectGrant::new(
            &object,
            StorageOperation::Read,
            timestamp(1),
            timestamp(100),
            proof.clone(),
        )
        .expect("grant");
        let allowed = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: StorageActor::Anonymous,
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::SignedRoute,
            object: &object,
            now: timestamp(99),
            grant: Some(&grant),
            grant_proof: Some(&proof),
            request_domain: None,
            custom_domain: None,
        });
        assert_eq!(allowed, StorageAuthorizationDecision::Allow);
        let wrong_proof = checksum(3);
        let forged = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: StorageActor::Anonymous,
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::SignedRoute,
            object: &object,
            now: timestamp(50),
            grant: Some(&grant),
            grant_proof: Some(&wrong_proof),
            request_domain: None,
            custom_domain: None,
        });
        assert_eq!(
            forged,
            StorageAuthorizationDecision::Deny(StorageDenialReason::GrantInvalid)
        );
        let expired = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            now: timestamp(100),
            ..StorageAccessRequest {
                actor: StorageActor::Anonymous,
                operation: StorageOperation::Read,
                surface: StorageAccessSurface::SignedRoute,
                object: &object,
                now: timestamp(100),
                grant: Some(&grant),
                grant_proof: Some(&proof),
                request_domain: None,
                custom_domain: None,
            }
        });
        assert_eq!(
            expired,
            StorageAuthorizationDecision::Deny(StorageDenialReason::GrantInvalid)
        );
        let changed_revision = GovernedObject::new(
            tenant,
            object.object_id().clone(),
            GovernedObjectRole::Source,
            ObjectVisibility::Unlisted,
            GovernedObjectState::Active,
            MalwareDisposition::Clean,
            2,
            1,
            checksum(1),
            ByteSize::new(10).expect("size"),
            None,
        )
        .expect("changed revision");
        let stale = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: StorageActor::Anonymous,
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::SignedRoute,
            object: &changed_revision,
            now: timestamp(50),
            grant: Some(&grant),
            grant_proof: Some(&proof),
            request_domain: None,
            custom_domain: None,
        });
        assert_eq!(
            stale,
            StorageAuthorizationDecision::Deny(StorageDenialReason::GrantInvalid)
        );
        let mut wire = serde_json::to_value(&grant).expect("serialize grant");
        wire["expires_at"] = serde_json::json!(MAX_TIMESTAMP_MS);
        assert!(serde_json::from_value::<SignedObjectGrant>(wire).is_err());
    }

    #[test]
    fn unlisted_custom_domain_requires_an_exact_grant_and_verified_ownership() {
        let tenant = TenantId::new();
        let object = object(tenant, ObjectVisibility::Unlisted);
        let proof = checksum(2);
        let grant = SignedObjectGrant::new(
            &object,
            StorageOperation::Read,
            timestamp(1),
            timestamp(100),
            proof.clone(),
        )
        .expect("grant");
        let binding = VerifiedCustomDomain::new(
            tenant,
            CustomDomainName::parse("media.example.com").expect("domain"),
            1,
            true,
        )
        .expect("binding");
        let denied = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: StorageActor::Anonymous,
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::CustomDomain,
            object: &object,
            now: timestamp(50),
            grant: None,
            grant_proof: None,
            request_domain: Some(binding.domain()),
            custom_domain: Some(&binding),
        });
        assert_eq!(
            denied,
            StorageAuthorizationDecision::Deny(StorageDenialReason::GrantRequired)
        );
        let allowed = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            grant: Some(&grant),
            grant_proof: Some(&proof),
            ..StorageAccessRequest {
                actor: StorageActor::Anonymous,
                operation: StorageOperation::Read,
                surface: StorageAccessSurface::CustomDomain,
                object: &object,
                now: timestamp(50),
                grant: None,
                grant_proof: None,
                request_domain: Some(binding.domain()),
                custom_domain: Some(&binding),
            }
        });
        assert_eq!(allowed, StorageAuthorizationDecision::Allow);
        let other_host = CustomDomainName::parse("other.example.com").expect("domain");
        let wrong_host = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: StorageActor::Anonymous,
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::CustomDomain,
            object: &object,
            now: timestamp(50),
            grant: Some(&grant),
            grant_proof: Some(&proof),
            request_domain: Some(&other_host),
            custom_domain: Some(&binding),
        });
        assert_eq!(
            wrong_host,
            StorageAuthorizationDecision::Deny(StorageDenialReason::DomainInvalid)
        );
    }

    #[test]
    fn cache_plan_purges_positive_and_negative_entries_with_bounded_slo() {
        let object = object(TenantId::new(), ObjectVisibility::Public);
        let plan = CacheInvalidationPlan::privacy_change(
            &object,
            ObjectVisibility::Private,
            2,
            timestamp(10),
            1_000,
        )
        .expect("plan");
        assert!(plan.purges_positive_and_negative());
        assert_eq!(plan.deadline(), timestamp(1_010));
        assert!(
            CacheInvalidationPlan::privacy_change(
                &object,
                ObjectVisibility::Private,
                2,
                timestamp(10),
                MAX_CACHE_INVALIDATION_SLO_MS + 1
            )
            .is_err()
        );
    }

    fn lifecycle_object(
        tenant_id: TenantId,
        id: &str,
        role: GovernedObjectRole,
        value: u8,
    ) -> LifecycleObject {
        LifecycleObject {
            tenant_id,
            object_id: GovernedObjectId::parse(format!("tenants/{tenant_id}/{id}")).expect("id"),
            role,
            checksum: checksum(value),
            size: ByteSize::new(10).expect("size"),
            retention_until: None,
        }
    }

    fn complete_inventory(
        tenant_id: TenantId,
        subject_digest: ChecksumSha256,
    ) -> LifecycleInventory {
        let required = GovernedObjectRole::ALL.into_iter().collect::<BTreeSet<_>>();
        let objects = GovernedObjectRole::ALL
            .into_iter()
            .enumerate()
            .map(|(index, role)| {
                lifecycle_object(
                    tenant_id,
                    &format!("object-{index}"),
                    role,
                    u8::try_from(index + 1).expect("small index"),
                )
            })
            .collect();
        LifecycleInventory::new(tenant_id, subject_digest, &required, objects).expect("inventory")
    }

    fn deletion_guard(
        tenant_id: TenantId,
        subject_digest: ChecksumSha256,
        observed_at: TimestampMillis,
        active_hold: bool,
    ) -> DeletionGuardSnapshot {
        DeletionGuardSnapshot::new(tenant_id, subject_digest, observed_at, 1, active_hold, None)
            .expect("guard")
    }

    fn deletion_receipt(
        workflow: &DeletionWorkflow,
        inventory: &LifecycleInventory,
        stage: DeletionStage,
        receipt_value: u8,
        observed_at: TimestampMillis,
    ) -> DeletionEvidenceReceipt {
        let cache_stage = stage == DeletionStage::CachePurged;
        DeletionEvidenceReceipt::verified(
            inventory.tenant_id(),
            inventory.digest().clone(),
            stage,
            workflow
                .evidence_target_digest(stage)
                .expect("evidence target"),
            checksum(receipt_value),
            observed_at,
            cache_stage,
            cache_stage,
        )
        .expect("provider receipt")
    }

    #[test]
    fn global_lifecycle_matrix_covers_every_declared_role_and_exports_from_manifest_only() {
        let tenant = TenantId::new();
        let required = GovernedObjectRole::ALL.into_iter().collect::<BTreeSet<_>>();
        let objects = GovernedObjectRole::ALL
            .into_iter()
            .enumerate()
            .map(|(index, role)| {
                lifecycle_object(
                    tenant,
                    &format!("object-{index}"),
                    role,
                    u8::try_from(index + 1).expect("small index"),
                )
            })
            .collect();
        let inventory =
            LifecycleInventory::new(tenant, checksum(42), &required, objects).expect("inventory");
        assert_eq!(inventory.roles(), &required);
        let export = StorageExportPlan::from_manifest(&inventory);
        assert_eq!(export.target_count(), GovernedObjectRole::ALL.len() - 1);
        assert!(export.still_matches(&inventory));
    }

    #[test]
    fn inventory_requires_declared_roles_and_deletion_is_manifest_bound_and_idempotent() {
        let tenant = TenantId::new();
        let inventory = complete_inventory(tenant, checksum(9));
        let active_hold = deletion_guard(tenant, checksum(9), timestamp(1), true);
        assert_eq!(
            DeletionWorkflow::plan(CorrelationId::new(), &inventory, &active_hold, timestamp(2)),
            Err(StorageGovernanceError::LegalHoldActive)
        );

        let guard = deletion_guard(tenant, checksum(9), timestamp(2), false);
        let mut workflow =
            DeletionWorkflow::plan(CorrelationId::new(), &inventory, &guard, timestamp(2))
                .expect("workflow");
        for (index, stage) in [
            DeletionStage::Tombstoned,
            DeletionStage::OriginDeleted,
            DeletionStage::CachePurged,
            DeletionStage::BackupDeleted,
            DeletionStage::Verified,
        ]
        .into_iter()
        .enumerate()
        {
            let evidence = deletion_receipt(
                &workflow,
                &inventory,
                stage,
                u8::try_from(index + 20).expect("small index"),
                timestamp(2),
            );
            workflow
                .record(&inventory, &evidence, &guard, timestamp(2))
                .expect("transition");
            workflow
                .record(&inventory, &evidence, &guard, timestamp(2))
                .expect("idempotent stage retry");
        }
        workflow
            .complete(&inventory, &guard, timestamp(2))
            .expect("complete");
        workflow
            .complete(&inventory, &guard, timestamp(2))
            .expect("terminal retry");
        let proof = workflow.completion_proof(timestamp(3)).expect("proof");
        let serialized = serde_json::to_string(&proof).expect("serialize");
        assert!(!serialized.contains("source"));
        assert!(!serialized.contains("backup"));
    }

    #[test]
    fn retention_blocks_deletion_and_tombstones_restore_only_inside_the_grace_window() {
        let tenant = TenantId::new();
        let required = GovernedObjectRole::ALL.into_iter().collect::<BTreeSet<_>>();
        let mut retained_objects = GovernedObjectRole::ALL
            .into_iter()
            .enumerate()
            .map(|(index, role)| {
                lifecycle_object(
                    tenant,
                    &format!("retained-{index}"),
                    role,
                    u8::try_from(index + 1).expect("small index"),
                )
            })
            .collect::<Vec<_>>();
        retained_objects[0].retention_until = Some(timestamp(100));
        let retained_inventory =
            LifecycleInventory::new(tenant, checksum(9), &required, retained_objects)
                .expect("inventory");
        let guard = deletion_guard(tenant, checksum(9), timestamp(1), false);
        assert_eq!(
            DeletionWorkflow::plan(
                CorrelationId::new(),
                &retained_inventory,
                &guard,
                timestamp(99)
            ),
            Err(StorageGovernanceError::RetentionActive)
        );

        let inventory = complete_inventory(tenant, checksum(10));
        let guard = deletion_guard(tenant, checksum(10), timestamp(1), false);
        let mut workflow =
            DeletionWorkflow::plan(CorrelationId::new(), &inventory, &guard, timestamp(1))
                .expect("workflow");
        let evidence = deletion_receipt(
            &workflow,
            &inventory,
            DeletionStage::Tombstoned,
            20,
            timestamp(1),
        );
        workflow
            .record(&inventory, &evidence, &guard, timestamp(1))
            .expect("tombstone");
        workflow
            .restore(&inventory, &guard, timestamp(1 + DELETION_RESTORE_GRACE_MS))
            .expect("restore at boundary");
        assert_eq!(workflow.stage(), DeletionStage::Restored);
    }

    #[test]
    fn response_policy_is_exact_origin_and_hardens_untrusted_media() {
        let origins = BTreeSet::from(["https://app.example".to_owned()]);
        let policy = StorageResponsePolicy::for_object(
            "video/mp4",
            ObjectVisibility::Private,
            Some("https://app.example"),
            &origins,
            false,
        )
        .expect("policy");
        assert_eq!(policy.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            policy.headers()["content-security-policy"],
            "sandbox; default-src 'none'"
        );
        assert!(
            !policy
                .headers()
                .contains_key("access-control-allow-credentials")
        );
        assert!(
            StorageResponsePolicy::for_object(
                "video/mp4",
                ObjectVisibility::Private,
                Some("https://evil.example"),
                &origins,
                false
            )
            .is_err()
        );
        let unsafe_inline = StorageResponsePolicy::for_object(
            "text/html",
            ObjectVisibility::Private,
            None,
            &origins,
            false,
        )
        .expect("policy");
        assert_eq!(
            unsafe_inline.headers()["content-disposition"],
            "attachment; filename=media.bin"
        );
    }

    #[test]
    fn only_the_scanner_can_read_pending_quarantined_media() {
        let tenant = TenantId::new();
        let quarantined = GovernedObject::new(
            tenant,
            GovernedObjectId::parse(format!("tenants/{tenant}/quarantine/object")).expect("id"),
            GovernedObjectRole::Source,
            ObjectVisibility::Private,
            GovernedObjectState::Quarantined,
            MalwareDisposition::Pending,
            1,
            1,
            checksum(1),
            ByteSize::new(10).expect("size"),
            None,
        )
        .expect("object");
        let scanner = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: StorageActor::Service {
                tenant_id: tenant,
                purpose: StorageServicePurpose::MalwareScanner,
            },
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::SameOriginApplication,
            object: &quarantined,
            now: timestamp(1),
            grant: None,
            grant_proof: None,
            request_domain: None,
            custom_domain: None,
        });
        assert_eq!(scanner, StorageAuthorizationDecision::Allow);
        let owner = StorageAuthorizationPolicy::evaluate(StorageAccessRequest {
            actor: StorageActor::Member {
                tenant_id: tenant,
                user_id: UserId::new(),
                role: StorageMemberRole::Owner,
            },
            operation: StorageOperation::Read,
            surface: StorageAccessSurface::SameOriginApplication,
            object: &quarantined,
            now: timestamp(1),
            grant: None,
            grant_proof: None,
            request_domain: None,
            custom_domain: None,
        });
        assert!(matches!(owner, StorageAuthorizationDecision::Deny(_)));
    }

    #[test]
    fn quota_and_audit_chain_are_bounded_and_tamper_evident() {
        let quota = StorageQuotaPolicy::new(ByteSize::new(100).expect("size"), 2).expect("quota");
        quota
            .reserve(
                ByteSize::new(50).expect("size"),
                1,
                ByteSize::new(50).expect("size"),
            )
            .expect("within quota");
        assert_eq!(
            quota.reserve(
                ByteSize::new(50).expect("size"),
                2,
                ByteSize::new(1).expect("size")
            ),
            Err(StorageGovernanceError::QuotaExceeded)
        );
        let mut audit = GovernanceAuditChain::new();
        audit
            .append(timestamp(1), "object_read", "allowed", checksum(1))
            .expect("event");
        audit
            .append(timestamp(2), "object_delete", "denied_hold", checksum(2))
            .expect("event");
        assert!(audit.verify());
    }

    #[test]
    fn managed_media_accepts_only_clean_tenant_scoped_sources_and_has_a_kill_switch() {
        let tenant = TenantId::new();
        let object = object(tenant, ObjectVisibility::Private);
        let enabled = ManagedMediaSourcePolicy::new(
            ManagedMediaState::Enabled,
            ByteSize::new(100).expect("size"),
            ByteSize::new(100).expect("size"),
        )
        .expect("policy");
        let input = enabled.authorize(tenant, &object).expect("input");
        let key = input.deterministic_derivative_key(&checksum(8));
        assert_eq!(key.as_str().len(), 64);
        let output = GovernedObject::new(
            tenant,
            GovernedObjectId::parse(format!("tenants/{tenant}/videos/video-1/{}", key.as_str()))
                .expect("id"),
            GovernedObjectRole::Preview,
            ObjectVisibility::Private,
            GovernedObjectState::Active,
            MalwareDisposition::Clean,
            1,
            1,
            checksum(9),
            ByteSize::new(20).expect("size"),
            None,
        )
        .expect("output");
        let authorized_output = enabled
            .authorize_output(&input, &output, &checksum(8))
            .expect("output");
        assert_eq!(authorized_output.derivative_identity(), &key);
        assert_eq!(
            enabled.authorize(TenantId::new(), &object),
            Err(StorageGovernanceError::MediaInputDenied)
        );
        let disabled = ManagedMediaSourcePolicy::new(
            ManagedMediaState::DisabledByIncident,
            ByteSize::new(100).expect("size"),
            ByteSize::new(100).expect("size"),
        )
        .expect("policy");
        assert_eq!(
            disabled.authorize(tenant, &object),
            Err(StorageGovernanceError::MediaKillSwitchActive)
        );
    }
}
