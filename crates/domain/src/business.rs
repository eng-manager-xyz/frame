//! Provider-neutral contracts for collaboration and business metadata.
//!
//! The types in this module deliberately contain no provider URLs, JavaScript
//! values, plaintext credentials, or payment-provider objects.  They are the
//! authority and persistence boundary shared by D1 and native adapters.

use std::{collections::BTreeMap, fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    ByteSize, ChecksumSha256, CommentId, ContentType, FolderId, IdempotencyKey, MAX_WIRE_INTEGER,
    MediaJobId, ObjectKey, OrganizationId, OrganizationRevision, OrganizationRole, SecretDigest,
    TenantId, TimestampMillis, UploadState, UserId, VideoId,
};

pub const BUSINESS_SCHEMA_VERSION: u16 = 1;
pub const MAX_VIDEO_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_EDIT_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_EVENT_DOCUMENT_BYTES: usize = 64 * 1024;
pub const MAX_DEVELOPER_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_DOCUMENT_DEPTH: usize = 16;
pub const MAX_DOCUMENT_NODES: usize = 16_384;
pub const MAX_COMMENT_BYTES: usize = 10_000;
pub const MAX_REDACTED_FAILURE_BYTES: usize = 64;
pub const MAX_LEDGER_AMOUNT: i64 = 9_007_199_254_740_991;
pub const CAP_NANOID_LENGTH: usize = 15;
pub const CAP_NANOID_ALPHABET: &str = "0123456789abcdefghjkmnpqrstvwxyz";

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LegacyCapNanoId(String);

impl LegacyCapNanoId {
    pub fn parse(value: impl Into<String>) -> Result<Self, BusinessContractError> {
        let value = value.into();
        if value.len() != CAP_NANOID_LENGTH
            || !value
                .bytes()
                .all(|byte| CAP_NANOID_ALPHABET.as_bytes().contains(&byte))
        {
            return Err(BusinessContractError::InvalidIdentifier("Cap NanoID"));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// RFC 9562 UUIDv8 derived without a table name, so the same source value
    /// maps identically in primary- and foreign-key columns across all tables.
    #[must_use]
    pub fn mapped_uuid(&self) -> Uuid {
        let mut digest = Sha256::new();
        digest.update(b"frame-cap-nanoid-to-uuid-v1\0");
        digest.update(self.0.as_bytes());
        let digest = digest.finalize();
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes)
    }
}

impl fmt::Debug for LegacyCapNanoId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("LegacyCapNanoId")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for LegacyCapNanoId {
    type Error = BusinessContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<LegacyCapNanoId> for String {
    fn from(value: LegacyCapNanoId) -> Self {
        value.0
    }
}

macro_rules! business_uuid {
    ($name:ident, $kind:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn parse(value: &str) -> Result<Self, BusinessContractError> {
                let parsed = Uuid::parse_str(value)
                    .ok()
                    .filter(|candidate| !candidate.is_nil())
                    .ok_or(BusinessContractError::InvalidIdentifier($kind))?;
                Ok(Self(parsed))
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
            type Err = BusinessContractError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = BusinessContractError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(&value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.to_string()
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(de::Error::custom)
            }
        }
    };
}

business_uuid!(BusinessOperationId, "business operation");
business_uuid!(VideoEditId, "video edit");
business_uuid!(VideoShareId, "video share");
business_uuid!(NotificationId, "notification");
business_uuid!(OutboxEventId, "outbox event");
business_uuid!(StorageIntegrationId, "storage integration");
business_uuid!(StorageObjectId, "storage object");
business_uuid!(ImportId, "import");
business_uuid!(BusinessUploadId, "business upload");
business_uuid!(DeveloperAppId, "developer app");
business_uuid!(DeveloperVideoId, "developer video");
business_uuid!(DeveloperApiKeyId, "developer API key");
business_uuid!(CreditAccountId, "credit account");
business_uuid!(CreditTransactionId, "credit transaction");
business_uuid!(UsageLedgerId, "usage ledger entry");
business_uuid!(LegalHoldId, "legal hold");

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum BusinessContractError {
    #[error("invalid {0} identifier")]
    InvalidIdentifier(&'static str),
    #[error("tenant and organization scopes do not match")]
    ScopeMismatch,
    #[error("document is malformed, non-canonical, or outside its size policy")]
    InvalidDocument,
    #[error("document schema version is not writable")]
    ReadOnlyDocument,
    #[error("revision or event sequence is outside the supported range")]
    InvalidSequence,
    #[error("an event conflicts with a previously accepted event")]
    ConflictingReplay,
    #[error("the lifecycle transition is invalid")]
    InvalidTransition,
    #[error("the record contains invalid or sensitive material")]
    InvalidRecord,
    #[error("the operation would violate an accounting invariant")]
    AccountingInvariant,
    #[error("the requested retention action is blocked")]
    RetentionLocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BusinessScope {
    pub tenant_id: TenantId,
    pub organization_id: OrganizationId,
}

impl BusinessScope {
    pub fn new(
        tenant_id: TenantId,
        organization_id: OrganizationId,
    ) -> Result<Self, BusinessContractError> {
        if tenant_id.as_uuid() != organization_id.as_uuid() {
            return Err(BusinessContractError::ScopeMismatch);
        }
        Ok(Self {
            tenant_id,
            organization_id,
        })
    }

    pub fn from_organization(
        organization_id: OrganizationId,
    ) -> Result<Self, BusinessContractError> {
        let tenant_id = TenantId::parse(&organization_id.to_string())
            .map_err(|_| BusinessContractError::ScopeMismatch)?;
        Self::new(tenant_id, organization_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BusinessRevision(u64);

impl BusinessRevision {
    pub const INITIAL: Self = Self(0);

    pub fn new(value: u64) -> Result<Self, BusinessContractError> {
        if value > MAX_WIRE_INTEGER {
            return Err(BusinessContractError::InvalidSequence);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, BusinessContractError> {
        self.0
            .checked_add(1)
            .ok_or(BusinessContractError::InvalidSequence)
            .and_then(Self::new)
    }
}

impl TryFrom<u64> for BusinessRevision {
    type Error = BusinessContractError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<BusinessRevision> for u64 {
    fn from(value: BusinessRevision) -> Self {
        value.get()
    }
}

impl Serialize for BusinessRevision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.get())
    }
}

impl<'de> Deserialize<'de> for BusinessRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BusinessAuthorityFence {
    pub identity_revision: OrganizationRevision,
    pub session_version: OrganizationRevision,
    pub organization_revision: OrganizationRevision,
    pub organization_authority_version: OrganizationRevision,
    pub membership_revision: OrganizationRevision,
    pub membership_authority_version: OrganizationRevision,
    pub resource_revision: BusinessRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    VideoMetadata,
    VideoEdit,
    NotificationPayload,
    OutboxPayload,
    StorageCapabilities,
    DeveloperMetadata,
}

impl DocumentKind {
    #[must_use]
    pub const fn maximum_bytes(self) -> usize {
        match self {
            Self::VideoMetadata => MAX_VIDEO_METADATA_BYTES,
            Self::VideoEdit => MAX_EDIT_DOCUMENT_BYTES,
            Self::NotificationPayload | Self::OutboxPayload => MAX_EVENT_DOCUMENT_BYTES,
            Self::StorageCapabilities | Self::DeveloperMetadata => MAX_DEVELOPER_METADATA_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentCompatibility {
    ReadWrite,
    ReadOnlyPreserve,
}

/// A bounded canonical JSON document with explicit forward-version behavior.
///
/// Current version documents are writable. Unknown positive versions are
/// retained byte-for-byte only when they are already canonical; callers must
/// not rewrite them until a compatible codec is deployed.
#[derive(Clone, PartialEq, Eq)]
pub struct VersionedBusinessDocument {
    kind: DocumentKind,
    schema_version: u16,
    compatibility: DocumentCompatibility,
    canonical_json: String,
    checksum: ChecksumSha256,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionedBusinessDocumentWire {
    kind: DocumentKind,
    canonical_json: String,
}

impl Serialize for VersionedBusinessDocument {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        VersionedBusinessDocumentWire {
            kind: self.kind,
            canonical_json: self.canonical_json.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for VersionedBusinessDocument {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = VersionedBusinessDocumentWire::deserialize(deserializer)?;
        Self::parse(wire.kind, &wire.canonical_json).map_err(de::Error::custom)
    }
}

impl VersionedBusinessDocument {
    pub fn parse(kind: DocumentKind, value: &str) -> Result<Self, BusinessContractError> {
        if value.is_empty() || value.len() > kind.maximum_bytes() {
            return Err(BusinessContractError::InvalidDocument);
        }
        let parsed: Value =
            serde_json::from_str(value).map_err(|_| BusinessContractError::InvalidDocument)?;
        let mut nodes = 0_usize;
        validate_document_value(&parsed, 0, &mut nodes)?;
        let schema_version = parsed
            .as_object()
            .and_then(|object| object.get("schema_version"))
            .and_then(Value::as_u64)
            .and_then(|version| u16::try_from(version).ok())
            .filter(|version| *version > 0)
            .ok_or(BusinessContractError::InvalidDocument)?;
        let canonical_json = canonical_json(&parsed)?;
        if canonical_json != value {
            return Err(BusinessContractError::InvalidDocument);
        }
        let compatibility = if schema_version == BUSINESS_SCHEMA_VERSION {
            DocumentCompatibility::ReadWrite
        } else {
            DocumentCompatibility::ReadOnlyPreserve
        };
        let checksum = ChecksumSha256::digest_bytes(canonical_json.as_bytes());
        Ok(Self {
            kind,
            schema_version,
            compatibility,
            canonical_json,
            checksum,
        })
    }

    pub fn from_value(kind: DocumentKind, value: Value) -> Result<Self, BusinessContractError> {
        let encoded = canonical_json(&value)?;
        Self::parse(kind, &encoded)
    }

    #[must_use]
    pub const fn kind(&self) -> DocumentKind {
        self.kind
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn compatibility(&self) -> DocumentCompatibility {
        self.compatibility
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    #[must_use]
    pub fn checksum(&self) -> &ChecksumSha256 {
        &self.checksum
    }

    pub fn require_writable(&self) -> Result<(), BusinessContractError> {
        if self.compatibility == DocumentCompatibility::ReadWrite {
            Ok(())
        } else {
            Err(BusinessContractError::ReadOnlyDocument)
        }
    }
}

impl fmt::Debug for VersionedBusinessDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionedBusinessDocument")
            .field("kind", &self.kind)
            .field("schema_version", &self.schema_version)
            .field("compatibility", &self.compatibility)
            .field("checksum", &self.checksum)
            .finish_non_exhaustive()
    }
}

fn validate_document_value(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), BusinessContractError> {
    if depth > MAX_DOCUMENT_DEPTH || *nodes >= MAX_DOCUMENT_NODES {
        return Err(BusinessContractError::InvalidDocument);
    }
    *nodes += 1;
    match value {
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
        Value::Number(number) => {
            let valid = number
                .as_u64()
                .is_some_and(|candidate| candidate <= MAX_WIRE_INTEGER)
                || number
                    .as_i64()
                    .is_some_and(|candidate| candidate.unsigned_abs() <= MAX_WIRE_INTEGER);
            if valid {
                Ok(())
            } else {
                Err(BusinessContractError::InvalidDocument)
            }
        }
        Value::Array(values) => {
            if values.len() > 4_096 {
                return Err(BusinessContractError::InvalidDocument);
            }
            for child in values {
                validate_document_value(child, depth + 1, nodes)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            if values.len() > 1_024
                || values.keys().any(|key| {
                    key.is_empty() || key.len() > 128 || key.chars().any(char::is_control)
                })
            {
                return Err(BusinessContractError::InvalidDocument);
            }
            for child in values.values() {
                validate_document_value(child, depth + 1, nodes)?;
            }
            Ok(())
        }
    }
}

fn canonical_json(value: &Value) -> Result<String, BusinessContractError> {
    let canonical = canonical_value(value);
    serde_json::to_string(&canonical).map_err(|_| BusinessContractError::InvalidDocument)
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let sorted: BTreeMap<_, _> = values
                .iter()
                .map(|(key, value)| (key.clone(), canonical_value(value)))
                .collect();
            let mut canonical = Map::new();
            for (key, value) in sorted {
                canonical.insert(key, value);
            }
            Value::Object(canonical)
        }
        primitive => primitive.clone(),
    }
}

/// Collision-resistant semantic digest using length-framed components.
#[must_use]
pub fn business_semantic_fingerprint<'a>(
    components: impl IntoIterator<Item = &'a [u8]>,
) -> ChecksumSha256 {
    let mut digest = Sha256::new();
    digest.update(b"frame-business-semantic-v1\0");
    for component in components {
        digest.update(
            u64::try_from(component.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        digest.update(component);
    }
    ChecksumSha256::digest_bytes(&digest.finalize())
}

/// Deterministic checksum for a typed command payload.
///
/// Repository adapters recompute this value from domain records instead of
/// trusting a checksum supplied by an application or transport layer.
pub fn business_payload_checksum<T: Serialize>(
    value: &T,
) -> Result<ChecksumSha256, BusinessContractError> {
    serde_json::to_vec(value)
        .map(|encoded| ChecksumSha256::digest_bytes(&encoded))
        .map_err(|_| BusinessContractError::InvalidRecord)
}

/// Global database uniqueness is safely reused for tenant-local logical keys
/// by storing this domain-separated digest instead of the caller's raw key.
#[must_use]
pub fn tenant_scoped_idempotency_digest(
    scope: BusinessScope,
    purpose: &str,
    key: &IdempotencyKey,
) -> ChecksumSha256 {
    let tenant = scope.tenant_id.to_string();
    business_semantic_fingerprint([
        b"frame-business-tenant-key-v1".as_slice(),
        tenant.as_bytes(),
        purpose.as_bytes(),
        key.expose().as_bytes(),
    ])
}

/// Canonical sequence-zero fingerprint shared by every ordered lifecycle.
#[must_use]
pub fn business_initial_event_fingerprint() -> ChecksumSha256 {
    ChecksumSha256::digest_bytes(b"frame-business-ordered-lifecycle-initial-v1")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoPrivacy {
    Private,
    Organization,
    Unlisted,
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusinessAction {
    ReadVideo,
    ManageVideo,
    ManageEdit,
    ManageShare,
    ReadComment,
    CreateComment,
    DeleteComment,
    ReadNotification,
    ManageNotification,
    ManageUpload,
    ManageStorage,
    ManageImport,
    ManageDeveloper,
    ManageLedger,
    ManageLegalHold,
    ExportData,
    DeleteData,
}

impl BusinessAction {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::ReadVideo => "video_read",
            Self::ManageVideo => "video_manage",
            Self::ManageEdit => "edit_manage",
            Self::ManageShare => "share_manage",
            Self::ReadComment => "comment_read",
            Self::CreateComment => "comment_create",
            Self::DeleteComment => "comment_delete",
            Self::ReadNotification => "notification_read",
            Self::ManageNotification => "notification_manage",
            Self::ManageUpload => "upload_manage",
            Self::ManageStorage => "storage_manage",
            Self::ManageImport => "import_manage",
            Self::ManageDeveloper => "developer_manage",
            Self::ManageLedger => "ledger_manage",
            Self::ManageLegalHold => "legal_hold_manage",
            Self::ExportData => "data_export",
            Self::DeleteData => "data_delete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusinessActor {
    Authenticated {
        tenant_id: TenantId,
        user_id: UserId,
        role: OrganizationRole,
    },
    Anonymous {
        actor_digest_present: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusinessPolicyContext {
    pub scope: BusinessScope,
    pub actor: BusinessActor,
    pub privacy: VideoPrivacy,
    pub resource_deleted: bool,
    pub comments_enabled: bool,
    pub owns_resource: bool,
    pub owns_comment: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusinessAuthorizationDecision {
    Allow,
    /// Missing, cross-tenant, private, and unauthorized resources collapse to
    /// one public result so callers cannot probe existence.
    AccessDenied,
    Deleted,
}

pub struct BusinessAuthorizationPolicy;

impl BusinessAuthorizationPolicy {
    #[must_use]
    pub fn evaluate(
        context: BusinessPolicyContext,
        action: BusinessAction,
    ) -> BusinessAuthorizationDecision {
        if context.resource_deleted {
            return BusinessAuthorizationDecision::Deleted;
        }
        match context.actor {
            BusinessActor::Anonymous {
                actor_digest_present,
            } => match action {
                BusinessAction::ReadVideo | BusinessAction::ReadComment
                    if matches!(
                        context.privacy,
                        VideoPrivacy::Public | VideoPrivacy::Unlisted
                    ) =>
                {
                    BusinessAuthorizationDecision::Allow
                }
                BusinessAction::CreateComment
                    if actor_digest_present
                        && context.comments_enabled
                        && matches!(
                            context.privacy,
                            VideoPrivacy::Public | VideoPrivacy::Unlisted
                        ) =>
                {
                    BusinessAuthorizationDecision::Allow
                }
                _ => BusinessAuthorizationDecision::AccessDenied,
            },
            BusinessActor::Authenticated {
                tenant_id, role, ..
            } => {
                if tenant_id != context.scope.tenant_id {
                    return BusinessAuthorizationDecision::AccessDenied;
                }
                let may_read = !matches!(context.privacy, VideoPrivacy::Private)
                    || matches!(role, OrganizationRole::Owner | OrganizationRole::Admin)
                    || context.owns_resource;
                let allowed = match action {
                    BusinessAction::ReadVideo | BusinessAction::ReadComment => may_read,
                    BusinessAction::CreateComment => context.comments_enabled && may_read,
                    BusinessAction::ManageVideo
                    | BusinessAction::ManageEdit
                    | BusinessAction::ManageShare => {
                        matches!(role, OrganizationRole::Owner | OrganizationRole::Admin)
                            || context.owns_resource
                    }
                    BusinessAction::DeleteComment => {
                        matches!(role, OrganizationRole::Owner | OrganizationRole::Admin)
                            || context.owns_comment
                    }
                    BusinessAction::ReadNotification => true,
                    BusinessAction::ManageNotification => {
                        matches!(role, OrganizationRole::Owner | OrganizationRole::Admin)
                    }
                    BusinessAction::ManageUpload
                    | BusinessAction::ManageStorage
                    | BusinessAction::ManageImport
                    | BusinessAction::ManageDeveloper => {
                        matches!(role, OrganizationRole::Owner | OrganizationRole::Admin)
                    }
                    BusinessAction::ManageLedger
                    | BusinessAction::ManageLegalHold
                    | BusinessAction::ExportData
                    | BusinessAction::DeleteData => role == OrganizationRole::Owner,
                };
                if allowed {
                    BusinessAuthorizationDecision::Allow
                } else {
                    BusinessAuthorizationDecision::AccessDenied
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessVideoRecord {
    pub id: VideoId,
    pub scope: BusinessScope,
    pub owner_id: UserId,
    pub privacy: VideoPrivacy,
    pub metadata: VersionedBusinessDocument,
    pub comments_enabled: bool,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub deleted_at: Option<TimestampMillis>,
    pub revision: BusinessRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoEditRecord {
    pub id: VideoEditId,
    pub scope: BusinessScope,
    pub video_id: VideoId,
    pub document: VersionedBusinessDocument,
    pub created_by_user_id: UserId,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub revision: BusinessRevision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareMode {
    Organization,
    Space,
    PublicLink,
}

impl ShareMode {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Space => "space",
            Self::PublicLink => "public_link",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoShareRecord {
    pub id: VideoShareId,
    pub scope: BusinessScope,
    pub video_id: VideoId,
    pub folder_id: Option<FolderId>,
    pub shared_by_user_id: UserId,
    pub mode: ShareMode,
    pub shared_at: TimestampMillis,
    pub revoked_at: Option<TimestampMillis>,
    pub revision: BusinessRevision,
}

impl VideoShareRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if (self.mode == ShareMode::Space) != self.folder_id.is_some()
            || self
                .revoked_at
                .is_some_and(|revoked| revoked < self.shared_at)
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentKind {
    Text,
    Emoji,
}

impl CommentKind {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Emoji => "emoji",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommentBody(String);

impl CommentBody {
    pub fn parse(value: impl Into<String>) -> Result<Self, BusinessContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_COMMENT_BYTES
            || value.trim() != value
            || value
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t' | '\r'))
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CommentBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CommentBody([redacted])")
    }
}

impl TryFrom<String> for CommentBody {
    type Error = BusinessContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<CommentBody> for String {
    fn from(value: CommentBody) -> Self {
        value.0
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentAuthor {
    User(UserId),
    Anonymous(SecretDigest),
}

impl fmt::Debug for CommentAuthor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(user_id) => formatter.debug_tuple("User").field(user_id).finish(),
            Self::Anonymous(_) => formatter.write_str("Anonymous([redacted])"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessCommentRecord {
    pub id: CommentId,
    pub scope: BusinessScope,
    pub video_id: VideoId,
    pub parent_comment_id: Option<CommentId>,
    pub author: CommentAuthor,
    pub kind: CommentKind,
    pub body: CommentBody,
    /// Optional position on the video timeline in integer microseconds.
    pub timeline_micros: Option<u64>,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub deleted_at: Option<TimestampMillis>,
    pub revision: BusinessRevision,
}

impl BusinessCommentRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.parent_comment_id == Some(self.id)
            || self
                .timeline_micros
                .is_some_and(|value| value > MAX_WIRE_INTEGER)
            || (self.kind == CommentKind::Emoji
                && (self.body.as_str().chars().count() > 16
                    || self.body.as_str().contains(char::is_whitespace)))
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct StableEventCode(String);

impl StableEventCode {
    pub fn parse(value: impl Into<String>) -> Result<Self, BusinessContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'.' | b':')
            })
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for StableEventCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StableEventCode")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for StableEventCode {
    type Error = BusinessContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<StableEventCode> for String {
    fn from(value: StableEventCode) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    View,
    Comment,
    Reply,
    Reaction,
    AnonymousView,
}

impl NotificationKind {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Comment => "comment",
            Self::Reply => "reply",
            Self::Reaction => "reaction",
            Self::AnonymousView => "anon_view",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    Leased,
    Delivered,
    DeadLetter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportState {
    Queued,
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderedEventResult {
    Applied,
    Duplicate,
    StaleIgnored,
    DeferredGap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedDeliveryLifecycle {
    pub state: DeliveryState,
    pub last_sequence: BusinessRevision,
    pub last_fingerprint: ChecksumSha256,
}

impl OrderedDeliveryLifecycle {
    pub fn apply(
        &mut self,
        sequence: BusinessRevision,
        fingerprint: ChecksumSha256,
        next: DeliveryState,
    ) -> Result<OrderedEventResult, BusinessContractError> {
        ordered_event_preflight(
            self.last_sequence,
            &self.last_fingerprint,
            sequence,
            &fingerprint,
        )?;
        if sequence < self.last_sequence {
            return Ok(OrderedEventResult::StaleIgnored);
        }
        if sequence == self.last_sequence {
            return Ok(OrderedEventResult::Duplicate);
        }
        if sequence.get() != self.last_sequence.get() + 1 {
            return Ok(OrderedEventResult::DeferredGap);
        }
        let transition = matches!(
            (self.state, next),
            (DeliveryState::Pending, DeliveryState::Leased)
                | (DeliveryState::Leased, DeliveryState::Pending)
                | (DeliveryState::Leased, DeliveryState::Delivered)
                | (DeliveryState::Leased, DeliveryState::DeadLetter)
        );
        if !transition {
            return Err(BusinessContractError::InvalidTransition);
        }
        self.state = next;
        self.last_sequence = sequence;
        self.last_fingerprint = fingerprint;
        Ok(OrderedEventResult::Applied)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedImportLifecycle {
    pub state: ImportState,
    pub last_sequence: BusinessRevision,
    pub last_fingerprint: ChecksumSha256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedUploadLifecycle {
    pub state: UploadState,
    pub last_sequence: BusinessRevision,
    pub last_fingerprint: ChecksumSha256,
}

impl OrderedUploadLifecycle {
    pub fn apply(
        &mut self,
        sequence: BusinessRevision,
        fingerprint: ChecksumSha256,
        next: UploadState,
    ) -> Result<OrderedEventResult, BusinessContractError> {
        ordered_event_preflight(
            self.last_sequence,
            &self.last_fingerprint,
            sequence,
            &fingerprint,
        )?;
        if sequence < self.last_sequence {
            return Ok(OrderedEventResult::StaleIgnored);
        }
        if sequence == self.last_sequence {
            return Ok(OrderedEventResult::Duplicate);
        }
        if sequence.get() != self.last_sequence.get() + 1 {
            return Ok(OrderedEventResult::DeferredGap);
        }
        let transition = matches!(
            (self.state, next),
            (
                UploadState::Initiated,
                UploadState::Uploading | UploadState::Failed | UploadState::Aborted
            ) | (
                UploadState::Uploading,
                UploadState::Finalizing | UploadState::Failed | UploadState::Aborted
            ) | (
                UploadState::Finalizing,
                UploadState::Complete | UploadState::Failed | UploadState::Aborted
            ) | (
                UploadState::Failed,
                UploadState::Uploading | UploadState::Aborted
            )
        );
        if !transition {
            return Err(BusinessContractError::InvalidTransition);
        }
        self.state = next;
        self.last_sequence = sequence;
        self.last_fingerprint = fingerprint;
        Ok(OrderedEventResult::Applied)
    }
}

impl OrderedImportLifecycle {
    pub fn apply(
        &mut self,
        sequence: BusinessRevision,
        fingerprint: ChecksumSha256,
        next: ImportState,
    ) -> Result<OrderedEventResult, BusinessContractError> {
        ordered_event_preflight(
            self.last_sequence,
            &self.last_fingerprint,
            sequence,
            &fingerprint,
        )?;
        if sequence < self.last_sequence {
            return Ok(OrderedEventResult::StaleIgnored);
        }
        if sequence == self.last_sequence {
            return Ok(OrderedEventResult::Duplicate);
        }
        if sequence.get() != self.last_sequence.get() + 1 {
            return Ok(OrderedEventResult::DeferredGap);
        }
        let transition = matches!(
            (self.state, next),
            (
                ImportState::Queued,
                ImportState::Running | ImportState::Cancelled
            ) | (
                ImportState::Running,
                ImportState::Complete | ImportState::Failed | ImportState::Cancelled
            ) | (
                ImportState::Failed,
                ImportState::Running | ImportState::Cancelled
            )
        );
        if !transition {
            return Err(BusinessContractError::InvalidTransition);
        }
        self.state = next;
        self.last_sequence = sequence;
        self.last_fingerprint = fingerprint;
        Ok(OrderedEventResult::Applied)
    }
}

fn ordered_event_preflight(
    current_sequence: BusinessRevision,
    current_fingerprint: &ChecksumSha256,
    sequence: BusinessRevision,
    fingerprint: &ChecksumSha256,
) -> Result<(), BusinessContractError> {
    if sequence == current_sequence && fingerprint != current_fingerprint {
        Err(BusinessContractError::ConflictingReplay)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub id: NotificationId,
    pub scope: BusinessScope,
    pub recipient_user_id: UserId,
    pub kind: NotificationKind,
    pub deduplication_key: IdempotencyKey,
    pub payload: VersionedBusinessDocument,
    pub created_at: TimestampMillis,
    pub read_at: Option<TimestampMillis>,
}

impl NotificationRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.payload.kind() != DocumentKind::NotificationPayload
            || self.read_at.is_some_and(|read| read < self.created_at)
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        self.payload.require_writable()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxRecord {
    pub id: OutboxEventId,
    pub scope: BusinessScope,
    pub aggregate_kind: StableEventCode,
    pub aggregate_id: String,
    pub event_type: StableEventCode,
    pub deduplication_key: IdempotencyKey,
    pub payload: VersionedBusinessDocument,
    pub lifecycle: OrderedDeliveryLifecycle,
    pub available_at: TimestampMillis,
    pub created_at: TimestampMillis,
}

impl OutboxRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.aggregate_id.is_empty()
            || self.aggregate_id.len() > 255
            || self.aggregate_id.contains("://")
            || self.aggregate_id.chars().any(char::is_control)
            || self.payload.kind() != DocumentKind::OutboxPayload
            || self.available_at < self.created_at
            || self.lifecycle.state != DeliveryState::Pending
            || self.lifecycle.last_sequence != BusinessRevision::INITIAL
            || self.lifecycle.last_fingerprint != business_initial_event_fingerprint()
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        self.payload.require_writable()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoUploadRecord {
    pub id: BusinessUploadId,
    pub scope: BusinessScope,
    pub video_id: VideoId,
    pub expected_bytes: ByteSize,
    pub received_bytes: ByteSize,
    pub idempotency_key: IdempotencyKey,
    pub source_object_key: ObjectKey,
    pub source_version: BusinessRevision,
    pub content_type: ContentType,
    pub checksum: Option<ChecksumSha256>,
    pub lifecycle: OrderedUploadLifecycle,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub revision: BusinessRevision,
}

impl VideoUploadRecord {
    pub fn validate_initial(&self) -> Result<(), BusinessContractError> {
        if self.lifecycle.state != UploadState::Initiated
            || self.lifecycle.last_sequence != BusinessRevision::INITIAL
            || self.lifecycle.last_fingerprint != business_initial_event_fingerprint()
            || self.received_bytes.get() != 0
            || self.checksum.is_some()
            || self.source_version == BusinessRevision::INITIAL
            || self.updated_at < self.created_at
            || self.revision != BusinessRevision::INITIAL
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageProviderKind {
    R2,
    S3Compatible,
    Minio,
    GoogleDrive,
}

impl StorageProviderKind {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::R2 => "r2",
            Self::S3Compatible => "s3_compatible",
            Self::Minio => "minio",
            Self::GoogleDrive => "google_drive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageIntegrationState {
    Pending,
    Active,
    Disabled,
    Revoked,
}

impl StorageIntegrationState {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EncryptedProviderConfig(String);

impl EncryptedProviderConfig {
    pub fn parse(ciphertext: impl Into<String>) -> Result<Self, BusinessContractError> {
        let ciphertext = ciphertext.into();
        if !(32..=65_536).contains(&ciphertext.len())
            || ciphertext.chars().any(char::is_control)
            || ciphertext.contains("://")
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(Self(ciphertext))
    }

    #[must_use]
    pub fn expose_to_provider_adapter(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EncryptedProviderConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EncryptedProviderConfig([redacted])")
    }
}

impl TryFrom<String> for EncryptedProviderConfig {
    type Error = BusinessContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<EncryptedProviderConfig> for String {
    fn from(value: EncryptedProviderConfig) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageIntegrationRecord {
    pub id: StorageIntegrationId,
    pub scope: BusinessScope,
    pub owner_user_id: Option<UserId>,
    pub provider: StorageProviderKind,
    pub state: StorageIntegrationState,
    /// Canonical versioned capability metadata; never credentials.
    pub capabilities: VersionedBusinessDocument,
    pub encrypted_config: EncryptedProviderConfig,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub revision: BusinessRevision,
    pub authority_version: BusinessRevision,
}

impl StorageIntegrationRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.capabilities.kind() != DocumentKind::StorageCapabilities
            || self.updated_at < self.created_at
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        self.capabilities.require_writable()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessObjectRole {
    Source,
    Segment,
    Thumbnail,
    Preview,
    Spritesheet,
    Audio,
    Export,
    Manifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageObjectState {
    Pending,
    Available,
    Quarantined,
    Deleting,
    Deleted,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageObjectManifest {
    pub id: StorageObjectId,
    pub scope: BusinessScope,
    pub integration_id: StorageIntegrationId,
    pub video_id: Option<VideoId>,
    pub object_key: ObjectKey,
    pub role: BusinessObjectRole,
    pub object_version: BusinessRevision,
    pub state: StorageObjectState,
    pub bytes: ByteSize,
    pub content_type: ContentType,
    pub checksum: ChecksumSha256,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
}

impl StorageObjectManifest {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.object_version == BusinessRevision::INITIAL
            || self.updated_at < self.created_at
            || self.object_key.as_str().contains("://")
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeExecutor {
    CloudflareMedia,
    NativeGstreamer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivativeState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RedactedFailureClass(String);

impl RedactedFailureClass {
    pub fn parse(value: impl Into<String>) -> Result<Self, BusinessContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_REDACTED_FAILURE_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn stable_code(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RedactedFailureClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RedactedFailureClass")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for RedactedFailureClass {
    type Error = BusinessContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RedactedFailureClass> for String {
    fn from(value: RedactedFailureClass) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivativeJobManifest {
    pub job_id: MediaJobId,
    pub scope: BusinessScope,
    pub executor: DerivativeExecutor,
    pub source_object_id: StorageObjectId,
    pub source_version: BusinessRevision,
    pub transform_profile: String,
    pub profile_version: BusinessRevision,
    pub output_role: BusinessObjectRole,
    pub output_object_id: Option<StorageObjectId>,
    pub output_key: ObjectKey,
    pub output_checksum: Option<ChecksumSha256>,
    pub output_content_type: ContentType,
    pub state: DerivativeState,
    pub usage_units: u64,
    pub cost_microcredits: u64,
    pub failure_class: Option<RedactedFailureClass>,
    pub revision: BusinessRevision,
}

impl DerivativeJobManifest {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        let succeeded_fields = self.output_object_id.is_some() && self.output_checksum.is_some();
        if self.source_version == BusinessRevision::INITIAL
            || self.profile_version == BusinessRevision::INITIAL
            || self.transform_profile.is_empty()
            || self.transform_profile.len() > 128
            || self.transform_profile.contains("://")
            || self.usage_units > MAX_WIRE_INTEGER
            || self.cost_microcredits > MAX_WIRE_INTEGER
            || (self.state == DerivativeState::Succeeded && !succeeded_fields)
            || (self.state != DerivativeState::Succeeded && succeeded_fields)
            || (self.state == DerivativeState::Failed) != self.failure_class.is_some()
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedVideoRecord {
    pub id: ImportId,
    pub scope: BusinessScope,
    pub video_id: Option<VideoId>,
    pub source: ImportProvider,
    pub external_id_digest: SecretDigest,
    pub idempotency_key: IdempotencyKey,
    pub lifecycle: OrderedImportLifecycle,
    pub error_class: Option<RedactedFailureClass>,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
}

impl ImportedVideoRecord {
    pub fn validate_initial(&self) -> Result<(), BusinessContractError> {
        if self.updated_at < self.created_at
            || self.lifecycle.state != ImportState::Queued
            || self.lifecycle.last_sequence != BusinessRevision::INITIAL
            || self.lifecycle.last_fingerprint != business_initial_event_fingerprint()
            || self.error_class.is_some()
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportProvider {
    Loom,
    File,
    GoogleDrive,
    Other,
}

impl ImportProvider {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Loom => "loom",
            Self::File => "file",
            Self::GoogleDrive => "google_drive",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperEnvironment {
    Test,
    Live,
}

impl DeveloperEnvironment {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Live => "live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperAppState {
    Active,
    Suspended,
    Deleted,
}

impl DeveloperAppState {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeveloperName(String);

impl DeveloperName {
    pub fn parse(value: impl Into<String>) -> Result<Self, BusinessContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 160
            || value.trim() != value
            || value.chars().any(char::is_control)
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DeveloperName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("DeveloperName")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for DeveloperName {
    type Error = BusinessContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<DeveloperName> for String {
    fn from(value: DeveloperName) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeveloperAppRecord {
    pub id: DeveloperAppId,
    pub scope: BusinessScope,
    pub owner_user_id: UserId,
    pub name: DeveloperName,
    pub environment: DeveloperEnvironment,
    pub state: DeveloperAppState,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub deleted_at: Option<TimestampMillis>,
    pub revision: BusinessRevision,
    pub authority_version: BusinessRevision,
}

impl DeveloperAppRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.updated_at < self.created_at
            || (self.state == DeveloperAppState::Deleted) != self.deleted_at.is_some()
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NormalizedDomain(String);

impl NormalizedDomain {
    pub fn parse(value: impl Into<String>) -> Result<Self, BusinessContractError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 253
            && value == value.to_ascii_lowercase()
            && !value.starts_with('.')
            && !value.ends_with('.')
            && value.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            });
        if !valid {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NormalizedDomain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("NormalizedDomain")
            .field(&self.0)
            .finish()
    }
}

impl TryFrom<String> for NormalizedDomain {
    type Error = BusinessContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<NormalizedDomain> for String {
    fn from(value: NormalizedDomain) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeveloperDomainRecord {
    pub scope: BusinessScope,
    pub app_id: DeveloperAppId,
    pub domain: NormalizedDomain,
    pub created_at: TimestampMillis,
    pub verified_at: Option<TimestampMillis>,
    pub revision: BusinessRevision,
}

impl DeveloperDomainRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self
            .verified_at
            .is_some_and(|verified| verified < self.created_at)
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeveloperVideoRecord {
    pub id: DeveloperVideoId,
    pub scope: BusinessScope,
    pub app_id: DeveloperAppId,
    pub video_id: Option<VideoId>,
    pub external_user_digest: SecretDigest,
    pub metadata: Option<VersionedBusinessDocument>,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub deleted_at: Option<TimestampMillis>,
    pub revision: BusinessRevision,
}

impl DeveloperVideoRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.updated_at < self.created_at
            || self
                .deleted_at
                .is_some_and(|deleted| deleted < self.created_at)
            || self.metadata.as_ref().is_some_and(|metadata| {
                metadata.kind() != DocumentKind::DeveloperMetadata
                    || metadata.require_writable().is_err()
            })
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeveloperKeyKind {
    Publishable,
    Secret,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeveloperApiKeyRecord {
    pub id: DeveloperApiKeyId,
    pub scope: BusinessScope,
    pub app_id: DeveloperAppId,
    pub kind: DeveloperKeyKind,
    pub key_digest: SecretDigest,
    pub display_prefix: String,
    pub created_at: TimestampMillis,
    pub last_used_at: Option<TimestampMillis>,
    pub revoked_at: Option<TimestampMillis>,
}

impl fmt::Debug for DeveloperApiKeyRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeveloperApiKeyRecord")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .field("app_id", &self.app_id)
            .field("kind", &self.kind)
            .field("display_prefix", &self.display_prefix)
            .field("created_at", &self.created_at)
            .field("last_used_at", &self.last_used_at)
            .field("revoked_at", &self.revoked_at)
            .finish_non_exhaustive()
    }
}

impl DeveloperApiKeyRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if !(4..=12).contains(&self.display_prefix.len())
            || !self
                .display_prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreditTransactionKind {
    Purchase,
    Usage,
    Refund,
    Adjustment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditTransactionRecord {
    pub id: CreditTransactionId,
    pub scope: BusinessScope,
    pub account_id: CreditAccountId,
    pub sequence: BusinessRevision,
    pub kind: CreditTransactionKind,
    pub amount_microcredits: i64,
    pub balance_after_microcredits: u64,
    pub reference_kind: String,
    pub reference_digest: ChecksumSha256,
    pub idempotency_key: IdempotencyKey,
    pub occurred_at: TimestampMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditAccountState {
    pub balance_microcredits: u64,
    pub last_sequence: BusinessRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditAccountRecord {
    pub id: CreditAccountId,
    pub scope: BusinessScope,
    pub app_id: DeveloperAppId,
    pub state: CreditAccountState,
    pub auto_top_up_enabled: bool,
    pub auto_top_up_threshold_microcredits: Option<u64>,
    pub created_at: TimestampMillis,
    pub updated_at: TimestampMillis,
    pub revision: BusinessRevision,
}

impl CreditAccountRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.updated_at < self.created_at
            || self
                .auto_top_up_threshold_microcredits
                .is_some_and(|value| value > MAX_WIRE_INTEGER)
        {
            return Err(BusinessContractError::AccountingInvariant);
        }
        Ok(())
    }
}

impl CreditAccountState {
    pub fn apply(
        &mut self,
        transaction: &CreditTransactionRecord,
    ) -> Result<(), BusinessContractError> {
        if transaction.sequence != self.last_sequence.next()?
            || transaction.amount_microcredits.unsigned_abs() > MAX_LEDGER_AMOUNT as u64
            || transaction.balance_after_microcredits > MAX_WIRE_INTEGER
            || transaction.reference_kind.is_empty()
            || transaction.reference_kind.len() > 64
        {
            return Err(BusinessContractError::AccountingInvariant);
        }
        let expected =
            i128::from(self.balance_microcredits) + i128::from(transaction.amount_microcredits);
        if expected < 0
            || u64::try_from(expected).ok() != Some(transaction.balance_after_microcredits)
        {
            return Err(BusinessContractError::AccountingInvariant);
        }
        self.balance_microcredits = transaction.balance_after_microcredits;
        self.last_sequence = transaction.sequence;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageKind {
    StorageByteDay,
    UploadByte,
    DownloadByte,
    TransformUnit,
    ComputeMillisecond,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageLedgerRecord {
    pub id: UsageLedgerId,
    pub scope: BusinessScope,
    pub app_id: Option<DeveloperAppId>,
    pub video_id: Option<VideoId>,
    pub media_job_id: Option<MediaJobId>,
    pub kind: UsageKind,
    pub quantity: u64,
    pub microcredits_charged: u64,
    pub idempotency_key: IdempotencyKey,
    pub occurred_at: TimestampMillis,
    pub recorded_at: TimestampMillis,
}

impl UsageLedgerRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.quantity > MAX_WIRE_INTEGER
            || self.microcredits_charged > MAX_WIRE_INTEGER
            || self.recorded_at < self.occurred_at
        {
            return Err(BusinessContractError::AccountingInvariant);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyStorageSnapshot {
    pub scope: BusinessScope,
    pub app_id: DeveloperAppId,
    /// UTC date in strict `YYYY-MM-DD` form.
    pub snapshot_day: String,
    pub total_bytes: ByteSize,
    pub microcredits_charged: u64,
    pub source_checksum: ChecksumSha256,
    pub processed_at: Option<TimestampMillis>,
    pub created_at: TimestampMillis,
}

impl DailyStorageSnapshot {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        let bytes = self.snapshot_day.as_bytes();
        let shaped = bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
        if !shaped {
            return Err(BusinessContractError::AccountingInvariant);
        }
        let year = self.snapshot_day[0..4]
            .parse::<u16>()
            .map_err(|_| BusinessContractError::AccountingInvariant)?;
        let month = self.snapshot_day[5..7]
            .parse::<u8>()
            .map_err(|_| BusinessContractError::AccountingInvariant)?;
        let day = self.snapshot_day[8..10]
            .parse::<u8>()
            .map_err(|_| BusinessContractError::AccountingInvariant)?;
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let maximum_day = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
        if year == 0
            || day == 0
            || day > maximum_day
            || self.microcredits_charged > MAX_WIRE_INTEGER
        {
            return Err(BusinessContractError::AccountingInvariant);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusinessDataClass {
    VideoMetadata,
    VideoEdit,
    Share,
    Comment,
    Notification,
    Outbox,
    StorageIntegration,
    StorageObject,
    DerivativeJob,
    Upload,
    Import,
    DeveloperApp,
    DeveloperDomain,
    DeveloperApiKey,
    DeveloperVideo,
    CreditAccount,
    CreditTransaction,
    UsageLedger,
    DailyStorageSnapshot,
    MessengerLegacy,
}

impl BusinessDataClass {
    pub const ALL: [Self; 20] = [
        Self::VideoMetadata,
        Self::VideoEdit,
        Self::Share,
        Self::Comment,
        Self::Notification,
        Self::Outbox,
        Self::StorageIntegration,
        Self::StorageObject,
        Self::DerivativeJob,
        Self::Upload,
        Self::Import,
        Self::DeveloperApp,
        Self::DeveloperDomain,
        Self::DeveloperApiKey,
        Self::DeveloperVideo,
        Self::CreditAccount,
        Self::CreditTransaction,
        Self::UsageLedger,
        Self::DailyStorageSnapshot,
        Self::MessengerLegacy,
    ];

    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::VideoMetadata => "video_metadata",
            Self::VideoEdit => "video_edit",
            Self::Share => "share",
            Self::Comment => "comment",
            Self::Notification => "notification",
            Self::Outbox => "outbox",
            Self::StorageIntegration => "storage_integration",
            Self::StorageObject => "storage_object",
            Self::DerivativeJob => "derivative_job",
            Self::Upload => "upload",
            Self::Import => "import",
            Self::DeveloperApp => "developer_app",
            Self::DeveloperDomain => "developer_domain",
            Self::DeveloperApiKey => "developer_api_key",
            Self::DeveloperVideo => "developer_video",
            Self::CreditAccount => "credit_account",
            Self::CreditTransaction => "credit_transaction",
            Self::UsageLedger => "usage_ledger",
            Self::DailyStorageSnapshot => "daily_storage_snapshot",
            Self::MessengerLegacy => "messenger_legacy",
        }
    }
}

#[must_use]
pub fn deletion_compensation_reference(
    class: BusinessDataClass,
    subject_id: &str,
) -> ChecksumSha256 {
    business_semantic_fingerprint([
        b"frame-business-deletion-compensation-v1".as_slice(),
        class.stable_code().as_bytes(),
        subject_id.as_bytes(),
    ])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionMode {
    TombstoneThenPurge,
    CryptographicEraseThenPurge,
    AppendCompensatingEntry,
    RetainAuditOnly,
    ExcludedQuarantine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DataHandlingRule {
    pub exportable: bool,
    pub deletion: DeletionMode,
    pub default_retention_days: u16,
    pub legal_hold_supported: bool,
}

#[must_use]
pub const fn data_handling_rule(class: BusinessDataClass) -> DataHandlingRule {
    match class {
        BusinessDataClass::CreditTransaction | BusinessDataClass::UsageLedger => DataHandlingRule {
            exportable: true,
            deletion: DeletionMode::AppendCompensatingEntry,
            default_retention_days: 2_555,
            legal_hold_supported: true,
        },
        BusinessDataClass::DeveloperApiKey | BusinessDataClass::StorageIntegration => {
            DataHandlingRule {
                exportable: false,
                deletion: DeletionMode::CryptographicEraseThenPurge,
                default_retention_days: 30,
                legal_hold_supported: false,
            }
        }
        BusinessDataClass::Outbox | BusinessDataClass::Notification => DataHandlingRule {
            exportable: true,
            deletion: DeletionMode::TombstoneThenPurge,
            default_retention_days: 90,
            legal_hold_supported: false,
        },
        BusinessDataClass::MessengerLegacy => DataHandlingRule {
            exportable: false,
            deletion: DeletionMode::ExcludedQuarantine,
            default_retention_days: 30,
            legal_hold_supported: false,
        },
        BusinessDataClass::CreditAccount | BusinessDataClass::DailyStorageSnapshot => {
            DataHandlingRule {
                exportable: true,
                deletion: DeletionMode::RetainAuditOnly,
                default_retention_days: 2_555,
                legal_hold_supported: true,
            }
        }
        _ => DataHandlingRule {
            exportable: true,
            deletion: DeletionMode::TombstoneThenPurge,
            default_retention_days: 30,
            legal_hold_supported: true,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessengerDisposition {
    /// No API or repository capability is exposed. Legacy rows are quarantined
    /// for bounded deletion and new writes fail at the database boundary.
    ExcludedFailClosed,
}

pub const MESSENGER_DISPOSITION: MessengerDisposition = MessengerDisposition::ExcludedFailClosed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalHold {
    pub id: LegalHoldId,
    pub scope: BusinessScope,
    pub data_class: BusinessDataClass,
    pub placed_at: TimestampMillis,
    pub released_at: Option<TimestampMillis>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusinessLegalHoldRecord {
    pub id: LegalHoldId,
    pub scope: BusinessScope,
    pub data_class: BusinessDataClass,
    pub subject_id: String,
    pub reason_code: StableEventCode,
    pub placed_by_user_id: UserId,
    pub placed_at: TimestampMillis,
    pub released_at: Option<TimestampMillis>,
}

impl BusinessLegalHoldRecord {
    pub fn validate(&self) -> Result<(), BusinessContractError> {
        if self.subject_id.is_empty()
            || self.subject_id.len() > 255
            || self.subject_id.chars().any(char::is_control)
            || self
                .released_at
                .is_some_and(|released| released < self.placed_at)
        {
            return Err(BusinessContractError::InvalidRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionDecision {
    Export,
    Delete(DeletionMode),
    Denied,
}

#[must_use]
pub fn retention_decision(
    class: BusinessDataClass,
    export: bool,
    active_legal_hold: bool,
) -> RetentionDecision {
    let rule = data_handling_rule(class);
    if export {
        if rule.exportable {
            RetentionDecision::Export
        } else {
            RetentionDecision::Denied
        }
    } else if active_legal_hold && rule.legal_hold_supported {
        RetentionDecision::Denied
    } else {
        RetentionDecision::Delete(rule.deletion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scope() -> BusinessScope {
        let organization_id = OrganizationId::new();
        BusinessScope::from_organization(organization_id).expect("scope")
    }

    fn revision(value: u64) -> BusinessRevision {
        BusinessRevision::new(value).expect("revision")
    }

    fn fingerprint(value: &str) -> ChecksumSha256 {
        business_semantic_fingerprint([value.as_bytes()])
    }

    #[test]
    fn document_codec_is_canonical_bounded_and_forward_preserving() {
        let document = VersionedBusinessDocument::from_value(
            DocumentKind::VideoEdit,
            json!({"timeline":[{"end_us":20,"start_us":10}],"schema_version":1}),
        )
        .expect("document");
        assert_eq!(
            document.canonical_json(),
            r#"{"schema_version":1,"timeline":[{"end_us":20,"start_us":10}]}"#
        );
        assert_eq!(document.compatibility(), DocumentCompatibility::ReadWrite);
        assert!(
            VersionedBusinessDocument::parse(DocumentKind::VideoEdit, r#"{ "schema_version":1}"#)
                .is_err()
        );
        assert!(
            VersionedBusinessDocument::from_value(
                DocumentKind::VideoEdit,
                json!({"schema_version": 1, "float": 1.25})
            )
            .is_err()
        );

        let future = VersionedBusinessDocument::parse(
            DocumentKind::VideoMetadata,
            r#"{"schema_version":2,"unknown":"preserve"}"#,
        )
        .expect("canonical future version");
        assert_eq!(
            future.compatibility(),
            DocumentCompatibility::ReadOnlyPreserve
        );
        assert_eq!(
            future.require_writable(),
            Err(BusinessContractError::ReadOnlyDocument)
        );
    }

    #[test]
    fn serialized_contracts_revalidate_instead_of_trusting_derived_fields() {
        let forged_document = r#"{"kind":"video_edit","canonical_json":"{ \"schema_version\":1}"}"#;
        assert!(serde_json::from_str::<VersionedBusinessDocument>(forged_document).is_err());
        assert!(
            serde_json::from_str::<BusinessOperationId>(
                r#""00000000-0000-0000-0000-000000000000""#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<BusinessRevision>("9007199254740992").is_err(),
            "wire revisions above JavaScript's exact integer range must fail closed"
        );
    }

    #[test]
    fn semantic_fingerprint_is_length_framed() {
        assert_ne!(
            business_semantic_fingerprint([b"ab".as_slice(), b"c".as_slice()]),
            business_semantic_fingerprint([b"a".as_slice(), b"bc".as_slice()])
        );
    }

    #[test]
    fn cap_nanoid_mapping_has_cross_runtime_known_answers() {
        for (source, expected) in [
            ("000000000000000", "229a6452-9523-896c-89cb-19ad6c78f2c6"),
            ("0123456789abcde", "2a6a8a87-d5ca-8c83-8666-2e92c2a69404"),
            ("zzzzzzzzzzzzzzz", "e2a9076f-abad-8f8b-8f84-a7054c9612c8"),
            ("a1b2c3d4e5f6g7h", "b72c7d87-9728-883a-b630-40a15f1d4662"),
        ] {
            let source = LegacyCapNanoId::parse(source).expect("Cap NanoID");
            assert_eq!(source.mapped_uuid().to_string(), expected);
            assert_eq!(source.mapped_uuid().to_string(), expected);
        }
        for invalid in ["short", "iiiiiiiiiiiiiii", "AAAAAAAAAAAAAAA"] {
            assert!(LegacyCapNanoId::parse(invalid).is_err());
        }
    }

    #[test]
    fn tenant_scoped_keys_and_initial_events_are_canonical() {
        let first = scope();
        let second = scope();
        let key = IdempotencyKey::parse("same-logical-key").expect("key");
        assert_ne!(
            tenant_scoped_idempotency_digest(first, "outbox", &key),
            tenant_scoped_idempotency_digest(second, "outbox", &key)
        );
        assert_ne!(
            tenant_scoped_idempotency_digest(first, "outbox", &key),
            tenant_scoped_idempotency_digest(first, "usage", &key)
        );
        assert_eq!(
            business_initial_event_fingerprint().as_str(),
            "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9"
        );
    }

    #[test]
    fn policy_denies_cross_tenant_and_private_anonymous_access() {
        let context = BusinessPolicyContext {
            scope: scope(),
            actor: BusinessActor::Anonymous {
                actor_digest_present: true,
            },
            privacy: VideoPrivacy::Private,
            resource_deleted: false,
            comments_enabled: true,
            owns_resource: false,
            owns_comment: false,
        };
        assert_eq!(
            BusinessAuthorizationPolicy::evaluate(context, BusinessAction::ReadVideo),
            BusinessAuthorizationDecision::AccessDenied
        );
        let public = BusinessPolicyContext {
            privacy: VideoPrivacy::Unlisted,
            ..context
        };
        assert_eq!(
            BusinessAuthorizationPolicy::evaluate(public, BusinessAction::CreateComment),
            BusinessAuthorizationDecision::Allow
        );

        let other_tenant = TenantId::new();
        let authenticated = BusinessPolicyContext {
            actor: BusinessActor::Authenticated {
                tenant_id: other_tenant,
                user_id: UserId::new(),
                role: OrganizationRole::Owner,
            },
            ..public
        };
        assert_eq!(
            BusinessAuthorizationPolicy::evaluate(authenticated, BusinessAction::ManageVideo),
            BusinessAuthorizationDecision::AccessDenied
        );
    }

    #[test]
    fn ordered_events_are_duplicate_and_gap_safe() {
        let first = fingerprint("first");
        let mut delivery = OrderedDeliveryLifecycle {
            state: DeliveryState::Pending,
            last_sequence: revision(0),
            last_fingerprint: fingerprint("seed"),
        };
        assert_eq!(
            delivery.apply(revision(2), fingerprint("gap"), DeliveryState::Leased),
            Ok(OrderedEventResult::DeferredGap)
        );
        assert_eq!(
            delivery.apply(revision(1), first.clone(), DeliveryState::Leased),
            Ok(OrderedEventResult::Applied)
        );
        assert_eq!(
            delivery.apply(revision(1), first, DeliveryState::Leased),
            Ok(OrderedEventResult::Duplicate)
        );
        assert_eq!(
            delivery.apply(revision(1), fingerprint("tamper"), DeliveryState::Leased),
            Err(BusinessContractError::ConflictingReplay)
        );
    }

    #[test]
    fn upload_events_use_the_same_gap_safe_ordering_contract() {
        let initial = business_initial_event_fingerprint();
        let first = fingerprint("uploading");
        let mut upload = OrderedUploadLifecycle {
            state: UploadState::Initiated,
            last_sequence: revision(0),
            last_fingerprint: initial,
        };
        assert_eq!(
            upload.apply(
                revision(2),
                fingerprint("finalizing"),
                UploadState::Finalizing,
            ),
            Ok(OrderedEventResult::DeferredGap)
        );
        assert_eq!(
            upload.apply(revision(1), first.clone(), UploadState::Uploading),
            Ok(OrderedEventResult::Applied)
        );
        assert_eq!(
            upload.apply(revision(1), first, UploadState::Uploading),
            Ok(OrderedEventResult::Duplicate)
        );
        assert_eq!(
            upload.apply(
                revision(2),
                fingerprint("finalizing"),
                UploadState::Finalizing,
            ),
            Ok(OrderedEventResult::Applied)
        );
        assert_eq!(upload.state, UploadState::Finalizing);
    }

    #[test]
    fn deletion_compensation_references_bind_class_and_subject() {
        let first = deletion_compensation_reference(BusinessDataClass::CreditTransaction, "one");
        assert_eq!(
            first,
            deletion_compensation_reference(BusinessDataClass::CreditTransaction, "one")
        );
        assert_ne!(
            first,
            deletion_compensation_reference(BusinessDataClass::CreditTransaction, "two")
        );
        assert_ne!(
            first,
            deletion_compensation_reference(BusinessDataClass::UsageLedger, "one")
        );
    }

    #[test]
    fn credit_balance_requires_exact_sequence_and_balance() {
        let mut account = CreditAccountState {
            balance_microcredits: 100,
            last_sequence: revision(0),
        };
        let transaction = CreditTransactionRecord {
            id: CreditTransactionId::new(),
            scope: scope(),
            account_id: CreditAccountId::new(),
            sequence: revision(1),
            kind: CreditTransactionKind::Usage,
            amount_microcredits: -25,
            balance_after_microcredits: 75,
            reference_kind: "media_job".into(),
            reference_digest: fingerprint("job"),
            idempotency_key: IdempotencyKey::parse("ledger:one").expect("key"),
            occurred_at: TimestampMillis::new(1).expect("time"),
        };
        account.apply(&transaction).expect("apply");
        assert_eq!(account.balance_microcredits, 75);
        assert!(account.apply(&transaction).is_err());

        let mut invalid = transaction;
        invalid.sequence = revision(2);
        invalid.balance_after_microcredits = 49;
        assert_eq!(
            account.apply(&invalid),
            Err(BusinessContractError::AccountingInvariant)
        );
    }

    #[test]
    fn every_data_class_has_explicit_handling() {
        assert_eq!(BusinessDataClass::ALL.len(), 20);
        for class in BusinessDataClass::ALL {
            let rule = data_handling_rule(class);
            assert!(rule.default_retention_days > 0);
        }
        assert_eq!(
            retention_decision(BusinessDataClass::StorageObject, false, true),
            RetentionDecision::Denied
        );
        assert_eq!(
            retention_decision(BusinessDataClass::DeveloperApiKey, true, false),
            RetentionDecision::Denied
        );
        assert_eq!(
            MESSENGER_DISPOSITION,
            MessengerDisposition::ExcludedFailClosed
        );
    }

    #[test]
    fn strict_calendar_and_collaboration_shapes_reject_ambiguous_rows() {
        let invalid_snapshot = DailyStorageSnapshot {
            scope: scope(),
            app_id: DeveloperAppId::new(),
            snapshot_day: "2025-02-29".into(),
            total_bytes: ByteSize::new(1).expect("bytes"),
            microcredits_charged: 1,
            source_checksum: fingerprint("snapshot"),
            processed_at: None,
            created_at: TimestampMillis::new(1).expect("time"),
        };
        assert_eq!(
            invalid_snapshot.validate(),
            Err(BusinessContractError::AccountingInvariant)
        );

        let comment = BusinessCommentRecord {
            id: CommentId::new(),
            scope: scope(),
            video_id: VideoId::new(),
            parent_comment_id: None,
            author: CommentAuthor::User(UserId::new()),
            kind: CommentKind::Emoji,
            body: CommentBody::parse("emoji with spaces").expect("bounded body"),
            timeline_micros: None,
            created_at: TimestampMillis::new(1).expect("time"),
            updated_at: TimestampMillis::new(1).expect("time"),
            deleted_at: None,
            revision: revision(1),
        };
        assert_eq!(
            comment.validate(),
            Err(BusinessContractError::InvalidRecord)
        );
    }
}
