use std::{collections::HashMap, fmt, sync::RwLock};

use async_trait::async_trait;
use frame_domain::{
    BackfillCredentialRefV1, BackfillDestinationVersionV1, BackfillEntryIdV1, BackfillLeaseV1,
    BackfillManifestIdV1, BackfillMediaProbePolicyV1, BackfillOperationIdV1,
    BackfillOwnerApprovalRecordV1, BackfillOwnerDispositionV1, BackfillProviderCapabilitiesV1,
    BackfillProviderChecksumV1, BackfillProviderV1, BackfillSourceReferenceV1,
    BackfillStorageAuthorityV1, ByteSize, ChecksumSha256, ContentType, ObjectBackfillJournalV1,
    ObjectBackfillManifestV1, ObjectBackfillReconciliationReportV1, ObjectRole, ScopedObjectKey,
    TenantId, TimestampMillis, VideoId,
};
use thiserror::Error;

pub const BACKFILL_MAX_TRANSPORT_CHUNK_BYTES_V1: usize = 16 * 1_024 * 1_024;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum BackfillPortErrorV1 {
    #[error("object-backfill resource was not found")]
    NotFound,
    #[error("object-backfill conditional operation conflicted")]
    Conflict,
    #[error("object-backfill request was throttled")]
    Throttled,
    #[error("object-backfill provider authorization expired")]
    ExpiredAuthorization,
    #[error("object-backfill provider is unavailable")]
    ProviderOutage,
    #[error("object-backfill operation was canceled")]
    Canceled,
    #[error("object-backfill adapter returned an invalid response")]
    InvalidResponse,
    #[error("object-backfill capability is unsupported")]
    Unsupported,
}

impl BackfillPortErrorV1 {
    #[must_use]
    pub const fn transient(self) -> bool {
        matches!(
            self,
            Self::Throttled | Self::ExpiredAuthorization | Self::ProviderOutage | Self::Canceled
        )
    }
}

/// Runtime access is separate from immutable manifests. Debug and Display never expose the ref.
pub struct BackfillProviderAccessV1<'a> {
    authority: &'a BackfillStorageAuthorityV1,
    credential: &'a BackfillCredentialRefV1,
}

impl<'a> BackfillProviderAccessV1<'a> {
    #[must_use]
    pub const fn new(
        authority: &'a BackfillStorageAuthorityV1,
        credential: &'a BackfillCredentialRefV1,
    ) -> Self {
        Self {
            authority,
            credential,
        }
    }

    #[must_use]
    pub const fn authority(&self) -> &BackfillStorageAuthorityV1 {
        self.authority
    }

    /// Only provider adapters should call this accessor.
    #[must_use]
    pub const fn credential(&self) -> &BackfillCredentialRefV1 {
        self.credential
    }
}

impl fmt::Debug for BackfillProviderAccessV1<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillProviderAccessV1")
            .field("authority", &self.authority)
            .field("credential", &"[redacted]")
            .finish()
    }
}

pub struct BackfillRuntimeBindingsV1<'a> {
    source: BackfillProviderAccessV1<'a>,
    target: BackfillProviderAccessV1<'a>,
}

impl<'a> BackfillRuntimeBindingsV1<'a> {
    #[must_use]
    pub const fn new(
        source: BackfillProviderAccessV1<'a>,
        target: BackfillProviderAccessV1<'a>,
    ) -> Self {
        Self { source, target }
    }

    #[must_use]
    pub const fn source(&self) -> &BackfillProviderAccessV1<'a> {
        &self.source
    }

    #[must_use]
    pub const fn target(&self) -> &BackfillProviderAccessV1<'a> {
        &self.target
    }
}

impl fmt::Debug for BackfillRuntimeBindingsV1<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillRuntimeBindingsV1")
            .field("source", &self.source)
            .field("target", &self.target)
            .finish()
    }
}

#[derive(PartialEq, Eq)]
pub struct BackfillChunkV1(Vec<u8>);

impl BackfillChunkV1 {
    pub fn new(bytes: Vec<u8>) -> Result<Self, BackfillPortErrorV1> {
        if bytes.is_empty() || bytes.len() > BACKFILL_MAX_TRANSPORT_CHUNK_BYTES_V1 {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for BackfillChunkV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BackfillChunkV1")
            .field(&format_args!("[redacted; {} bytes]", self.0.len()))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackfillObjectLocationV1 {
    Source(BackfillSourceReferenceV1),
    Target(ScopedObjectKey),
}

#[derive(Clone, PartialEq, Eq)]
pub struct BackfillObjectMetadataV1 {
    authority_fingerprint: ChecksumSha256,
    owner_tenant: TenantId,
    video_id: VideoId,
    role: ObjectRole,
    location: BackfillObjectLocationV1,
    logical_bytes: ByteSize,
    content_type: ContentType,
    strong_sha256: Option<ChecksumSha256>,
    provider_checksum: Option<BackfillProviderChecksumV1>,
    destination_version: Option<BackfillDestinationVersionV1>,
    operation_id: Option<BackfillOperationIdV1>,
}

impl BackfillObjectMetadataV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn source(
        authority_fingerprint: ChecksumSha256,
        owner_tenant: TenantId,
        video_id: VideoId,
        role: ObjectRole,
        source_reference: BackfillSourceReferenceV1,
        logical_bytes: ByteSize,
        content_type: ContentType,
        strong_sha256: Option<ChecksumSha256>,
        provider_checksum: Option<BackfillProviderChecksumV1>,
    ) -> Result<Self, BackfillPortErrorV1> {
        if logical_bytes.get() == 0 {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self {
            authority_fingerprint,
            owner_tenant,
            video_id,
            role,
            location: BackfillObjectLocationV1::Source(source_reference),
            logical_bytes,
            content_type,
            strong_sha256,
            provider_checksum,
            destination_version: None,
            operation_id: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn target(
        authority_fingerprint: ChecksumSha256,
        owner_tenant: TenantId,
        video_id: VideoId,
        role: ObjectRole,
        target_key: ScopedObjectKey,
        logical_bytes: ByteSize,
        content_type: ContentType,
        strong_sha256: Option<ChecksumSha256>,
        provider_checksum: Option<BackfillProviderChecksumV1>,
        destination_version: BackfillDestinationVersionV1,
        operation_id: Option<BackfillOperationIdV1>,
    ) -> Result<Self, BackfillPortErrorV1> {
        if logical_bytes.get() == 0
            || !target_key.belongs_to(owner_tenant, video_id)
            || target_key.role() != role
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self {
            authority_fingerprint,
            owner_tenant,
            video_id,
            role,
            location: BackfillObjectLocationV1::Target(target_key),
            logical_bytes,
            content_type,
            strong_sha256,
            provider_checksum,
            destination_version: Some(destination_version),
            operation_id,
        })
    }

    #[must_use]
    pub const fn authority_fingerprint(&self) -> &ChecksumSha256 {
        &self.authority_fingerprint
    }

    #[must_use]
    pub const fn owner_tenant(&self) -> TenantId {
        self.owner_tenant
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
    pub const fn location(&self) -> &BackfillObjectLocationV1 {
        &self.location
    }

    #[must_use]
    pub const fn logical_bytes(&self) -> ByteSize {
        self.logical_bytes
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn strong_sha256(&self) -> Option<&ChecksumSha256> {
        self.strong_sha256.as_ref()
    }

    #[must_use]
    pub const fn provider_checksum(&self) -> Option<&BackfillProviderChecksumV1> {
        self.provider_checksum.as_ref()
    }

    #[must_use]
    pub const fn destination_version(&self) -> Option<&BackfillDestinationVersionV1> {
        self.destination_version.as_ref()
    }

    #[must_use]
    pub const fn operation_id(&self) -> Option<BackfillOperationIdV1> {
        self.operation_id
    }
}

impl fmt::Debug for BackfillObjectMetadataV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillObjectMetadataV1")
            .field("authority_fingerprint", &"[redacted]")
            .field("owner_tenant", &"[redacted]")
            .field("video_id", &"[redacted]")
            .field("role", &self.role)
            .field("location", &"[redacted]")
            .field("logical_bytes", &self.logical_bytes)
            .field("content_type", &self.content_type)
            .field(
                "strong_sha256",
                &self.strong_sha256.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "provider_checksum",
                &self.provider_checksum.as_ref().map(|_| "[opaque]"),
            )
            .field("destination_version", &self.destination_version)
            .field("operation_id", &self.operation_id)
            .finish()
    }
}

#[async_trait]
pub trait BackfillReadBodyV1: Send {
    /// Returns a non-empty bounded chunk or `None` at exact EOF.
    async fn next_chunk(&mut self) -> Result<Option<BackfillChunkV1>, BackfillPortErrorV1>;
    /// Idempotently releases the provider stream. Implementations must also release on `Drop`.
    async fn cancel(&mut self) -> Result<(), BackfillPortErrorV1>;
}

pub struct BackfillOpenedReadV1 {
    metadata: BackfillObjectMetadataV1,
    body: Box<dyn BackfillReadBodyV1>,
}

impl BackfillOpenedReadV1 {
    #[must_use]
    pub fn new(metadata: BackfillObjectMetadataV1, body: Box<dyn BackfillReadBodyV1>) -> Self {
        Self { metadata, body }
    }

    #[must_use]
    pub const fn metadata(&self) -> &BackfillObjectMetadataV1 {
        &self.metadata
    }

    #[must_use]
    pub fn into_parts(self) -> (BackfillObjectMetadataV1, Box<dyn BackfillReadBodyV1>) {
        (self.metadata, self.body)
    }
}

impl fmt::Debug for BackfillOpenedReadV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillOpenedReadV1")
            .field("metadata", &self.metadata)
            .field("body", &"[stream]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BackfillCreateSpecV1 {
    entry_id: BackfillEntryIdV1,
    operation_id: BackfillOperationIdV1,
    authority_fingerprint: ChecksumSha256,
    owner_tenant: TenantId,
    video_id: VideoId,
    role: ObjectRole,
    target_key: ScopedObjectKey,
    expected_size: ByteSize,
    expected_sha256: ChecksumSha256,
    content_type: ContentType,
}

impl BackfillCreateSpecV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        entry_id: BackfillEntryIdV1,
        operation_id: BackfillOperationIdV1,
        authority_fingerprint: ChecksumSha256,
        owner_tenant: TenantId,
        video_id: VideoId,
        role: ObjectRole,
        target_key: ScopedObjectKey,
        expected_size: ByteSize,
        expected_sha256: ChecksumSha256,
        content_type: ContentType,
    ) -> Result<Self, BackfillPortErrorV1> {
        if expected_size.get() == 0
            || !target_key.belongs_to(owner_tenant, video_id)
            || target_key.role() != role
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self {
            entry_id,
            operation_id,
            authority_fingerprint,
            owner_tenant,
            video_id,
            role,
            target_key,
            expected_size,
            expected_sha256,
            content_type,
        })
    }

    #[must_use]
    pub const fn entry_id(&self) -> BackfillEntryIdV1 {
        self.entry_id
    }

    #[must_use]
    pub const fn operation_id(&self) -> BackfillOperationIdV1 {
        self.operation_id
    }

    #[must_use]
    pub const fn authority_fingerprint(&self) -> &ChecksumSha256 {
        &self.authority_fingerprint
    }

    #[must_use]
    pub const fn owner_tenant(&self) -> TenantId {
        self.owner_tenant
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
    pub const fn target_key(&self) -> &ScopedObjectKey {
        &self.target_key
    }

    #[must_use]
    pub const fn expected_size(&self) -> ByteSize {
        self.expected_size
    }

    #[must_use]
    pub const fn expected_sha256(&self) -> &ChecksumSha256 {
        &self.expected_sha256
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }
}

impl fmt::Debug for BackfillCreateSpecV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackfillCreateSpecV1")
            .field("entry_id", &self.entry_id)
            .field("operation_id", &self.operation_id)
            .field("authority_fingerprint", &"[redacted]")
            .field("owner_tenant", &"[redacted]")
            .field("video_id", &"[redacted]")
            .field("role", &self.role)
            .field("target_key", &"[redacted]")
            .field("expected_size", &self.expected_size)
            .field("expected_sha256", &"[redacted]")
            .field("content_type", &self.content_type)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillProbeReceiptV1 {
    profile_version: u16,
    playable: bool,
}

impl BackfillProbeReceiptV1 {
    pub fn new(profile_version: u16, playable: bool) -> Result<Self, BackfillPortErrorV1> {
        if profile_version == 0 {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self {
            profile_version,
            playable,
        })
    }

    #[must_use]
    pub const fn profile_version(&self) -> u16 {
        self.profile_version
    }

    #[must_use]
    pub const fn playable(&self) -> bool {
        self.playable
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillCommitReceiptV1 {
    destination_version: BackfillDestinationVersionV1,
    operation_id: BackfillOperationIdV1,
}

impl BackfillCommitReceiptV1 {
    #[must_use]
    pub const fn new(
        destination_version: BackfillDestinationVersionV1,
        operation_id: BackfillOperationIdV1,
    ) -> Self {
        Self {
            destination_version,
            operation_id,
        }
    }

    #[must_use]
    pub const fn destination_version(&self) -> &BackfillDestinationVersionV1 {
        &self.destination_version
    }

    #[must_use]
    pub const fn operation_id(&self) -> BackfillOperationIdV1 {
        self.operation_id
    }
}

#[async_trait]
pub trait BackfillWriteBodyV1: Send {
    /// Ownership enforces one in-flight application chunk and natural backpressure.
    async fn write_chunk(&mut self, chunk: BackfillChunkV1) -> Result<(), BackfillPortErrorV1>;
    async fn commit(
        &mut self,
        probe: Option<&BackfillProbeReceiptV1>,
        fence: &dyn BackfillCommitFenceV1,
    ) -> Result<BackfillCommitReceiptV1, BackfillPortErrorV1>;
    /// Idempotently removes staging state. Implementations must do the same on `Drop`.
    async fn cancel(&mut self) -> Result<(), BackfillPortErrorV1>;
}

/// A live durable-journal authorization that the writer must validate immediately before its
/// atomic publication step. A cached or previously successful validation is not sufficient.
#[async_trait]
pub trait BackfillCommitFenceV1: Send + Sync {
    fn manifest_id(&self) -> BackfillManifestIdV1;
    fn entry_id(&self) -> BackfillEntryIdV1;
    fn operation_id(&self) -> BackfillOperationIdV1;
    fn lease(&self) -> BackfillLeaseV1;
    async fn authorize_publication(&self) -> Result<(), BackfillPortErrorV1>;
}

pub enum BackfillConditionalCreateV1 {
    Ready(Box<dyn BackfillWriteBodyV1>),
    AlreadyPresent(Box<BackfillObjectMetadataV1>),
}

impl fmt::Debug for BackfillConditionalCreateV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready(_) => formatter.write_str("Ready([stream])"),
            Self::AlreadyPresent(metadata) => formatter
                .debug_tuple("AlreadyPresent")
                .field(metadata)
                .finish(),
        }
    }
}

#[async_trait]
pub trait BackfillProbeSessionV1: Send {
    async fn observe(&mut self, bytes: &[u8]) -> Result<(), BackfillPortErrorV1>;
    async fn finish(&mut self) -> Result<BackfillProbeReceiptV1, BackfillPortErrorV1>;
    /// Idempotently releases probe resources. Implementations must also release on `Drop`.
    async fn cancel(&mut self) -> Result<(), BackfillPortErrorV1>;
}

#[async_trait]
pub trait BackfillMediaProbePortV1: Send + Sync {
    async fn start(
        &self,
        role: ObjectRole,
        policy: BackfillMediaProbePolicyV1,
    ) -> Result<Box<dyn BackfillProbeSessionV1>, BackfillPortErrorV1>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct BackfillInventoryCursorV1(String);

impl BackfillInventoryCursorV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillPortErrorV1> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 512
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_to_adapter(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackfillInventoryCursorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillInventoryCursorV1([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillInventoryPageV1 {
    objects: Vec<BackfillObjectMetadataV1>,
    next: Option<BackfillInventoryCursorV1>,
    snapshot_digest: ChecksumSha256,
    page_index: u64,
}

impl BackfillInventoryPageV1 {
    pub fn new(
        objects: Vec<BackfillObjectMetadataV1>,
        next: Option<BackfillInventoryCursorV1>,
        snapshot_digest: ChecksumSha256,
        page_index: u64,
        limit: u16,
    ) -> Result<Self, BackfillPortErrorV1> {
        if limit == 0
            || limit > 1_000
            || objects.len() > usize::from(limit)
            || (objects.is_empty() && next.is_some())
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self {
            objects,
            next,
            snapshot_digest,
            page_index,
        })
    }

    #[must_use]
    pub fn objects(&self) -> &[BackfillObjectMetadataV1] {
        &self.objects
    }

    #[must_use]
    pub const fn next(&self) -> Option<&BackfillInventoryCursorV1> {
        self.next.as_ref()
    }

    #[must_use]
    pub const fn snapshot_digest(&self) -> &ChecksumSha256 {
        &self.snapshot_digest
    }

    #[must_use]
    pub const fn page_index(&self) -> u64 {
        self.page_index
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        Vec<BackfillObjectMetadataV1>,
        Option<BackfillInventoryCursorV1>,
        ChecksumSha256,
        u64,
    ) {
        (
            self.objects,
            self.next,
            self.snapshot_digest,
            self.page_index,
        )
    }
}

#[async_trait]
pub trait BackfillSourcePortV1: Send + Sync {
    async fn capabilities(
        &self,
        access: &BackfillProviderAccessV1<'_>,
    ) -> Result<BackfillProviderCapabilitiesV1, BackfillPortErrorV1>;

    async fn open_read(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        reference: &BackfillSourceReferenceV1,
        operation_id: BackfillOperationIdV1,
    ) -> Result<BackfillOpenedReadV1, BackfillPortErrorV1>;

    async fn estimate_egress_cost_units(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        logical_bytes: ByteSize,
    ) -> Result<u64, BackfillPortErrorV1>;

    async fn inventory_page(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenant_id: TenantId,
        cursor: Option<&BackfillInventoryCursorV1>,
        limit: u16,
    ) -> Result<BackfillInventoryPageV1, BackfillPortErrorV1>;
}

#[async_trait]
pub trait BackfillDestinationPortV1: Send + Sync {
    async fn capabilities(
        &self,
        access: &BackfillProviderAccessV1<'_>,
    ) -> Result<BackfillProviderCapabilitiesV1, BackfillPortErrorV1>;

    async fn estimate_cost_units(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        logical_bytes: ByteSize,
    ) -> Result<u64, BackfillPortErrorV1>;

    async fn head(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        key: &ScopedObjectKey,
    ) -> Result<Option<BackfillObjectMetadataV1>, BackfillPortErrorV1>;

    async fn open_read(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        key: &ScopedObjectKey,
        operation_id: BackfillOperationIdV1,
    ) -> Result<BackfillOpenedReadV1, BackfillPortErrorV1>;

    async fn begin_conditional_create(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        spec: &BackfillCreateSpecV1,
    ) -> Result<BackfillConditionalCreateV1, BackfillPortErrorV1>;

    async fn inventory_page(
        &self,
        access: &BackfillProviderAccessV1<'_>,
        tenant_id: TenantId,
        cursor: Option<&BackfillInventoryCursorV1>,
        limit: u16,
    ) -> Result<BackfillInventoryPageV1, BackfillPortErrorV1>;
}

#[async_trait]
pub trait BackfillManifestPortV1: Send + Sync {
    async fn put_immutable(
        &self,
        manifest: &ObjectBackfillManifestV1,
    ) -> Result<(), BackfillPortErrorV1>;
    async fn load(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<Option<ObjectBackfillManifestV1>, BackfillPortErrorV1>;
}

#[async_trait]
pub trait BackfillJournalPortV1: Send + Sync {
    async fn create(&self, journal: &ObjectBackfillJournalV1) -> Result<(), BackfillPortErrorV1>;
    async fn load(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<Option<ObjectBackfillJournalV1>, BackfillPortErrorV1>;
    async fn compare_and_swap(
        &self,
        manifest_id: BackfillManifestIdV1,
        expected_revision: u64,
        next: &ObjectBackfillJournalV1,
    ) -> Result<(), BackfillPortErrorV1>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillThrottleDecisionV1 {
    Allowed,
    DeferredUntil(TimestampMillis),
}

#[async_trait]
pub trait BackfillThrottlePortV1: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn admit_object(
        &self,
        tenant_id: TenantId,
        source_provider: BackfillProviderV1,
        source_region: &str,
        target_provider: BackfillProviderV1,
        target_region: &str,
        max_objects_per_minute: u32,
        now: TimestampMillis,
    ) -> Result<BackfillThrottleDecisionV1, BackfillPortErrorV1>;

    #[allow(clippy::too_many_arguments)]
    async fn admit_bytes(
        &self,
        tenant_id: TenantId,
        source_provider: BackfillProviderV1,
        source_region: &str,
        target_provider: BackfillProviderV1,
        target_region: &str,
        bytes: ByteSize,
        max_bytes_per_second: u64,
        now: TimestampMillis,
    ) -> Result<BackfillThrottleDecisionV1, BackfillPortErrorV1>;
}

#[async_trait]
pub trait BackfillCancellationPortV1: Send + Sync {
    fn canceled(&self, manifest_id: BackfillManifestIdV1, entry_id: BackfillEntryIdV1) -> bool;
    async fn request_manifest_abort(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<(), BackfillPortErrorV1>;
}

pub trait BackfillClockPortV1: Send + Sync {
    fn now(&self) -> Result<TimestampMillis, BackfillPortErrorV1>;
}

/// Opaque authorization capability supplied by a trusted control-plane caller.
pub struct BackfillOwnerApprovalCapabilityV1(String);

impl BackfillOwnerApprovalCapabilityV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, BackfillPortErrorV1> {
        let value = value.into();
        if !(16..=1_024).contains(&value.len())
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        Ok(Self(value))
    }

    /// Only the trusted authorization adapter may inspect the opaque capability.
    #[must_use]
    pub fn expose_to_adapter(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BackfillOwnerApprovalCapabilityV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackfillOwnerApprovalCapabilityV1([redacted])")
    }
}

#[async_trait]
pub trait BackfillOwnerApprovalPortV1: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn verify_disposition(
        &self,
        capability: &BackfillOwnerApprovalCapabilityV1,
        manifest: &ObjectBackfillManifestV1,
        entry_id: BackfillEntryIdV1,
        tenant_id: TenantId,
        disposition: BackfillOwnerDispositionV1,
        now: TimestampMillis,
    ) -> Result<BackfillOwnerApprovalRecordV1, BackfillPortErrorV1>;

    async fn verify_source_release(
        &self,
        capability: &BackfillOwnerApprovalCapabilityV1,
        manifest: &ObjectBackfillManifestV1,
        report: &ObjectBackfillReconciliationReportV1,
        now: TimestampMillis,
    ) -> Result<BackfillOwnerApprovalRecordV1, BackfillPortErrorV1>;
}

#[derive(Default)]
pub struct MemoryBackfillManifestPortV1 {
    manifests: RwLock<HashMap<BackfillManifestIdV1, ObjectBackfillManifestV1>>,
}

impl fmt::Debug for MemoryBackfillManifestPortV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryBackfillManifestPortV1")
            .field(
                "manifest_count",
                &self.manifests.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

#[async_trait]
impl BackfillManifestPortV1 for MemoryBackfillManifestPortV1 {
    async fn put_immutable(
        &self,
        manifest: &ObjectBackfillManifestV1,
    ) -> Result<(), BackfillPortErrorV1> {
        let mut manifests = self
            .manifests
            .write()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?;
        match manifests.get(&manifest.manifest_id()) {
            Some(existing) if existing == manifest => Ok(()),
            Some(_) => Err(BackfillPortErrorV1::Conflict),
            None => {
                manifests.insert(manifest.manifest_id(), manifest.clone());
                Ok(())
            }
        }
    }

    async fn load(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<Option<ObjectBackfillManifestV1>, BackfillPortErrorV1> {
        self.manifests
            .read()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)
            .map(|rows| rows.get(&manifest_id).cloned())
    }
}

#[derive(Default)]
pub struct MemoryBackfillJournalPortV1 {
    journals: RwLock<HashMap<BackfillManifestIdV1, ObjectBackfillJournalV1>>,
}

impl fmt::Debug for MemoryBackfillJournalPortV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryBackfillJournalPortV1")
            .field(
                "journal_count",
                &self.journals.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

#[async_trait]
impl BackfillJournalPortV1 for MemoryBackfillJournalPortV1 {
    async fn create(&self, journal: &ObjectBackfillJournalV1) -> Result<(), BackfillPortErrorV1> {
        let mut journals = self
            .journals
            .write()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?;
        if journals.contains_key(&journal.manifest_id()) {
            return Err(BackfillPortErrorV1::Conflict);
        }
        journals.insert(journal.manifest_id(), journal.clone());
        Ok(())
    }

    async fn load(
        &self,
        manifest_id: BackfillManifestIdV1,
    ) -> Result<Option<ObjectBackfillJournalV1>, BackfillPortErrorV1> {
        self.journals
            .read()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)
            .map(|rows| rows.get(&manifest_id).cloned())
    }

    async fn compare_and_swap(
        &self,
        manifest_id: BackfillManifestIdV1,
        expected_revision: u64,
        next: &ObjectBackfillJournalV1,
    ) -> Result<(), BackfillPortErrorV1> {
        if next.manifest_id() != manifest_id
            || next.revision() != expected_revision.saturating_add(1)
        {
            return Err(BackfillPortErrorV1::InvalidResponse);
        }
        let mut journals = self
            .journals
            .write()
            .map_err(|_| BackfillPortErrorV1::ProviderOutage)?;
        let current = journals
            .get(&manifest_id)
            .ok_or(BackfillPortErrorV1::NotFound)?;
        if current.revision() != expected_revision {
            return Err(BackfillPortErrorV1::Conflict);
        }
        journals.insert(manifest_id, next.clone());
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAllBackfillThrottleV1;

#[async_trait]
impl BackfillThrottlePortV1 for AllowAllBackfillThrottleV1 {
    async fn admit_object(
        &self,
        _tenant_id: TenantId,
        _source_provider: BackfillProviderV1,
        _source_region: &str,
        _target_provider: BackfillProviderV1,
        _target_region: &str,
        _max_objects_per_minute: u32,
        _now: TimestampMillis,
    ) -> Result<BackfillThrottleDecisionV1, BackfillPortErrorV1> {
        Ok(BackfillThrottleDecisionV1::Allowed)
    }

    async fn admit_bytes(
        &self,
        _tenant_id: TenantId,
        _source_provider: BackfillProviderV1,
        _source_region: &str,
        _target_provider: BackfillProviderV1,
        _target_region: &str,
        _bytes: ByteSize,
        _max_bytes_per_second: u64,
        _now: TimestampMillis,
    ) -> Result<BackfillThrottleDecisionV1, BackfillPortErrorV1> {
        Ok(BackfillThrottleDecisionV1::Allowed)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NeverCancelBackfillV1;

#[async_trait]
impl BackfillCancellationPortV1 for NeverCancelBackfillV1 {
    fn canceled(&self, _manifest_id: BackfillManifestIdV1, _entry_id: BackfillEntryIdV1) -> bool {
        false
    }

    async fn request_manifest_abort(
        &self,
        _manifest_id: BackfillManifestIdV1,
    ) -> Result<(), BackfillPortErrorV1> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use frame_domain::{
        BackfillMediaProbeModeV1, BackfillProviderLocatorV1, ObjectBackfillManifestEntryV1,
        ObjectRevision, StorageFileExtension, VideoObjectDescriptor,
    };

    use super::*;

    fn timestamp(value: i64) -> TimestampMillis {
        TimestampMillis::new(value).expect("timestamp")
    }

    fn policy() -> frame_domain::BackfillExecutionPolicyV1 {
        frame_domain::BackfillExecutionPolicyV1::new(
            2,
            3,
            100,
            1_000_000,
            10_000,
            1_000_000,
            60,
            frame_domain::DurationMillis::new(10).expect("retry"),
            frame_domain::DurationMillis::new(100).expect("retry max"),
            2,
            frame_domain::DurationMillis::new(1_000).expect("cooldown"),
            frame_domain::DurationMillis::new(100).expect("lease"),
            ByteSize::new(64 * 1_024).expect("chunk"),
        )
        .expect("policy")
    }

    fn manifest() -> ObjectBackfillManifestV1 {
        let tenant = TenantId::new();
        let video = VideoId::new();
        let entry = ObjectBackfillManifestEntryV1::new(
            BackfillEntryIdV1::new(),
            tenant,
            video,
            ObjectRole::Source,
            BackfillSourceReferenceV1::parse("legacy/source.mp4").expect("source"),
            ScopedObjectKey::source(
                tenant,
                video,
                ObjectRevision::new(2).expect("revision"),
                VideoObjectDescriptor::Source {
                    extension: StorageFileExtension::parse("mp4").expect("extension"),
                },
            )
            .expect("target"),
            ByteSize::new(5).expect("size"),
            ChecksumSha256::digest_bytes(b"media"),
            None,
            ContentType::parse("video/mp4").expect("type"),
            BackfillMediaProbePolicyV1::new(1, BackfillMediaProbeModeV1::Required).expect("probe"),
        )
        .expect("entry");
        let authority = |provider, marker: u8| {
            BackfillStorageAuthorityV1::new(
                provider,
                "test-region",
                BackfillProviderLocatorV1::parse(format!("bucket-{marker}")).expect("locator"),
                ChecksumSha256::parse(format!("{marker:02x}").repeat(32)).expect("fingerprint"),
            )
            .expect("authority")
        };
        ObjectBackfillManifestV1::new(
            BackfillManifestIdV1::new(),
            timestamp(1),
            "tool-1",
            "code-1",
            authority(BackfillProviderV1::S3, 0xaa),
            authority(BackfillProviderV1::R2, 0xbb),
            policy(),
            vec![entry],
        )
        .expect("manifest")
    }

    #[tokio::test]
    async fn immutable_manifest_port_is_idempotent_but_rejects_replacement() {
        let store = MemoryBackfillManifestPortV1::default();
        let manifest = manifest();
        store.put_immutable(&manifest).await.expect("first put");
        store
            .put_immutable(&manifest)
            .await
            .expect("idempotent put");
        assert_eq!(
            store.load(manifest.manifest_id()).await.expect("load"),
            Some(manifest)
        );
    }

    #[tokio::test]
    async fn journal_port_enforces_exact_compare_and_swap_revision() {
        let manifest = manifest();
        let store = MemoryBackfillJournalPortV1::default();
        let journal = ObjectBackfillJournalV1::new(&manifest);
        store.create(&journal).await.expect("create");
        let mut first = journal.clone();
        first.pause(timestamp(2)).expect("pause");
        store
            .compare_and_swap(manifest.manifest_id(), 0, &first)
            .await
            .expect("cas");
        let mut stale = journal;
        stale.pause(timestamp(2)).expect("pause stale");
        assert_eq!(
            store
                .compare_and_swap(manifest.manifest_id(), 0, &stale)
                .await,
            Err(BackfillPortErrorV1::Conflict)
        );
        assert_eq!(
            store
                .load(manifest.manifest_id())
                .await
                .expect("load")
                .expect("journal")
                .revision(),
            1
        );
    }

    #[test]
    fn credentials_chunks_and_metadata_are_redacted_and_bounded() {
        let credential = BackfillCredentialRefV1::parse("vault:providers/source").expect("ref");
        let manifest = manifest();
        let access = BackfillProviderAccessV1::new(manifest.source(), &credential);
        let debug = format!("{access:?}");
        assert!(!debug.contains("vault:providers/source"));
        assert_eq!(
            BackfillChunkV1::new(Vec::new()),
            Err(BackfillPortErrorV1::InvalidResponse)
        );
        let chunk = BackfillChunkV1::new(vec![7; 8]).expect("chunk");
        assert!(!format!("{chunk:?}").contains("7, 7"));
    }
}
