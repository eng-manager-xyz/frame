use std::{
    collections::{BTreeMap, VecDeque},
    error::Error,
    fmt,
    sync::{
        Mutex, RwLock,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use frame_domain::{
    ByteSize, ChecksumSha256, ContentType, CorrelationId, DurationMillis, MAX_WIRE_INTEGER,
    ObjectRole, ScopedObjectKey, TenantId, TimestampMillis, VideoId,
};

pub const OBJECT_STORE_CONTRACT_VERSION: u16 = 1;
pub const UPLOAD_BROKER_CONTRACT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectStoreOperation {
    Head,
    Get,
    Range,
    Put,
    Copy,
    Delete,
    List,
    ConditionalCreate,
    ConditionalSourceVersion,
    ConditionalDeleteVersion,
    Sha256Integrity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectStoreCapabilitiesV1 {
    contract_version: u16,
    operations: [bool; 11],
    max_object_size: ByteSize,
    max_range_size: ByteSize,
    max_list_page_size: u16,
}

impl ObjectStoreCapabilitiesV1 {
    pub fn full(
        max_object_size: ByteSize,
        max_range_size: ByteSize,
        max_list_page_size: u16,
    ) -> Result<Self, StorageFailure> {
        if max_object_size.get() == 0
            || max_range_size.get() == 0
            || ByteSize::new(max_object_size.get()).is_err()
            || ByteSize::new(max_range_size.get()).is_err()
            || !(1..=100).contains(&max_list_page_size)
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            contract_version: OBJECT_STORE_CONTRACT_VERSION,
            operations: [true; 11],
            max_object_size,
            max_range_size,
            max_list_page_size,
        })
    }

    #[must_use]
    pub const fn contract_version(self) -> u16 {
        self.contract_version
    }

    #[must_use]
    pub const fn max_object_size(self) -> ByteSize {
        self.max_object_size
    }

    #[must_use]
    pub const fn max_range_size(self) -> ByteSize {
        self.max_range_size
    }

    #[must_use]
    pub const fn max_list_page_size(self) -> u16 {
        self.max_list_page_size
    }

    #[must_use]
    pub fn supports(self, operation: ObjectStoreOperation) -> bool {
        self.operations[operation_index(operation)]
    }

    #[must_use]
    pub fn without(mut self, operation: ObjectStoreOperation) -> Self {
        self.operations[operation_index(operation)] = false;
        self
    }

    pub fn require(self, operation: ObjectStoreOperation) -> Result<(), StorageFailure> {
        if self.supports(operation) {
            Ok(())
        } else {
            Err(StorageFailure::unsupported())
        }
    }
}

const fn operation_index(operation: ObjectStoreOperation) -> usize {
    match operation {
        ObjectStoreOperation::Head => 0,
        ObjectStoreOperation::Get => 1,
        ObjectStoreOperation::Range => 2,
        ObjectStoreOperation::Put => 3,
        ObjectStoreOperation::Copy => 4,
        ObjectStoreOperation::Delete => 5,
        ObjectStoreOperation::List => 6,
        ObjectStoreOperation::ConditionalCreate => 7,
        ObjectStoreOperation::ConditionalSourceVersion => 8,
        ObjectStoreOperation::ConditionalDeleteVersion => 9,
        ObjectStoreOperation::Sha256Integrity => 10,
    }
}

const fn is_fault_injectable_operation(operation: ObjectStoreOperation) -> bool {
    matches!(
        operation,
        ObjectStoreOperation::Head
            | ObjectStoreOperation::Get
            | ObjectStoreOperation::Range
            | ObjectStoreOperation::Put
            | ObjectStoreOperation::Copy
            | ObjectStoreOperation::Delete
            | ObjectStoreOperation::List
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFailureKind {
    NotFound,
    PreconditionFailed,
    Throttled,
    Unauthorized,
    QuotaExceeded,
    Timeout,
    Integrity,
    Unavailable,
    UnsupportedCapability,
    InvalidRequest,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageFailure {
    kind: StorageFailureKind,
    retry_after: Option<DurationMillis>,
}

impl StorageFailure {
    #[must_use]
    pub const fn new(kind: StorageFailureKind) -> Self {
        Self {
            kind,
            retry_after: None,
        }
    }

    #[must_use]
    pub const fn throttled(retry_after: DurationMillis) -> Self {
        Self {
            kind: StorageFailureKind::Throttled,
            retry_after: Some(retry_after),
        }
    }

    #[must_use]
    pub const fn unsupported() -> Self {
        Self::new(StorageFailureKind::UnsupportedCapability)
    }

    #[must_use]
    pub const fn kind(&self) -> StorageFailureKind {
        self.kind
    }

    #[must_use]
    pub const fn retry_after(&self) -> Option<DurationMillis> {
        self.retry_after
    }

    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self.kind,
            StorageFailureKind::Throttled
                | StorageFailureKind::Timeout
                | StorageFailureKind::Unavailable
        )
    }

    #[must_use]
    pub const fn safe_message(&self) -> &'static str {
        match self.kind {
            StorageFailureKind::NotFound => "The object was not found.",
            StorageFailureKind::PreconditionFailed => "The object precondition was not satisfied.",
            StorageFailureKind::Throttled => "The storage request was rate limited.",
            StorageFailureKind::Unauthorized => "The storage request was not authorized.",
            StorageFailureKind::QuotaExceeded => "The storage quota was exceeded.",
            StorageFailureKind::Timeout => "The storage request timed out.",
            StorageFailureKind::Integrity => "The object failed integrity validation.",
            StorageFailureKind::Unavailable => "The storage service is temporarily unavailable.",
            StorageFailureKind::UnsupportedCapability => "The storage capability is not supported.",
            StorageFailureKind::InvalidRequest => "The storage request is invalid.",
        }
    }
}

impl fmt::Debug for StorageFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageFailure")
            .field("kind", &self.kind)
            .field("retry_after", &self.retry_after)
            .finish()
    }
}

impl fmt::Display for StorageFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.safe_message())
    }
}

impl Error for StorageFailure {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageRequestContext {
    tenant_id: TenantId,
    correlation_id: CorrelationId,
}

impl StorageRequestContext {
    #[must_use]
    pub const fn new(tenant_id: TenantId, correlation_id: CorrelationId) -> Self {
        Self {
            tenant_id,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn tenant_id(self) -> TenantId {
        self.tenant_id
    }

    #[must_use]
    pub const fn correlation_id(self) -> CorrelationId {
        self.correlation_id
    }
}

macro_rules! provider_token {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, StorageFailure> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > 256
                    || !value.bytes().all(|byte| byte.is_ascii_graphic())
                {
                    return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn expose_for_provider_comparison(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}

provider_token!(ProviderObjectVersion);
provider_token!(ProviderEntityTag);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectCachePolicy {
    NoStore,
    PrivateImmutable,
    PublicImmutable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMetadataV1 {
    key: ScopedObjectKey,
    size: ByteSize,
    content_type: ContentType,
    checksum_sha256: ChecksumSha256,
    provider_version: ProviderObjectVersion,
    provider_etag: ProviderEntityTag,
    cache_policy: ObjectCachePolicy,
    last_modified: TimestampMillis,
    correlation_id: CorrelationId,
}

impl ObjectMetadataV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: ScopedObjectKey,
        size: ByteSize,
        content_type: ContentType,
        checksum_sha256: ChecksumSha256,
        provider_version: ProviderObjectVersion,
        provider_etag: ProviderEntityTag,
        cache_policy: ObjectCachePolicy,
        last_modified: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> Result<Self, StorageFailure> {
        if size.get() == 0
            || ByteSize::new(size.get()).is_err()
            || ContentType::parse(content_type.as_str()).is_err()
            || ChecksumSha256::parse(checksum_sha256.as_str()).is_err()
            || TimestampMillis::new(last_modified.get()).is_err()
            || CorrelationId::parse(&correlation_id.to_string()).is_err()
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            key,
            size,
            content_type,
            checksum_sha256,
            provider_version,
            provider_etag,
            cache_policy,
            last_modified,
            correlation_id,
        })
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn size(&self) -> ByteSize {
        self.size
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn provider_version(&self) -> &ProviderObjectVersion {
        &self.provider_version
    }

    #[must_use]
    pub const fn provider_etag(&self) -> &ProviderEntityTag {
        &self.provider_etag
    }

    #[must_use]
    pub const fn cache_policy(&self) -> ObjectCachePolicy {
        self.cache_policy
    }

    #[must_use]
    pub const fn last_modified(&self) -> TimestampMillis {
        self.last_modified
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PutObjectRequestV1 {
    key: ScopedObjectKey,
    bytes: Vec<u8>,
    content_type: ContentType,
    checksum_sha256: ChecksumSha256,
    cache_policy: ObjectCachePolicy,
}

impl fmt::Debug for PutObjectRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PutObjectRequestV1")
            .field("key", &self.key)
            .field("byte_count", &self.bytes.len())
            .field("content_type", &self.content_type)
            .field("checksum_sha256", &self.checksum_sha256)
            .field("cache_policy", &self.cache_policy)
            .finish()
    }
}

impl PutObjectRequestV1 {
    pub fn immutable(
        key: ScopedObjectKey,
        bytes: Vec<u8>,
        content_type: ContentType,
        checksum_sha256: ChecksumSha256,
        cache_policy: ObjectCachePolicy,
    ) -> Result<Self, StorageFailure> {
        if bytes.is_empty()
            || ContentType::parse(content_type.as_str()).is_err()
            || ChecksumSha256::parse(checksum_sha256.as_str()).is_err()
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            key,
            bytes,
            content_type,
            checksum_sha256,
            cache_policy,
        })
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn cache_policy(&self) -> ObjectCachePolicy {
        self.cache_policy
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectWriteReceiptV1 {
    metadata: ObjectMetadataV1,
}

impl ObjectWriteReceiptV1 {
    #[must_use]
    pub const fn new(metadata: ObjectMetadataV1) -> Self {
        Self { metadata }
    }

    #[must_use]
    pub const fn metadata(&self) -> &ObjectMetadataV1 {
        &self.metadata
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ObjectBodyV1 {
    metadata: ObjectMetadataV1,
    bytes: Vec<u8>,
}

impl fmt::Debug for ObjectBodyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectBodyV1")
            .field("metadata", &self.metadata)
            .field("byte_count", &self.bytes.len())
            .finish()
    }
}

impl ObjectBodyV1 {
    pub fn new(metadata: ObjectMetadataV1, bytes: Vec<u8>) -> Result<Self, StorageFailure> {
        let size = ByteSize::new(
            u64::try_from(bytes.len())
                .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?,
        )
        .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?;
        if size != metadata.size || ChecksumSha256::digest_bytes(&bytes) != metadata.checksum_sha256
        {
            return Err(StorageFailure::new(StorageFailureKind::Integrity));
        }
        Ok(Self { metadata, bytes })
    }

    #[must_use]
    pub const fn metadata(&self) -> &ObjectMetadataV1 {
        &self.metadata
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectByteRange {
    start: u64,
    end_exclusive: u64,
}

impl ObjectByteRange {
    pub fn new(start: u64, end_exclusive: u64) -> Result<Self, StorageFailure> {
        if start >= end_exclusive || end_exclusive > MAX_WIRE_INTEGER {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            start,
            end_exclusive,
        })
    }

    #[must_use]
    pub const fn start(self) -> u64 {
        self.start
    }

    #[must_use]
    pub const fn end_exclusive(self) -> u64 {
        self.end_exclusive
    }

    #[must_use]
    pub const fn length(self) -> u64 {
        self.end_exclusive - self.start
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ObjectRangeBodyV1 {
    metadata: ObjectMetadataV1,
    bytes: Vec<u8>,
    range: ObjectByteRange,
}

impl fmt::Debug for ObjectRangeBodyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectRangeBodyV1")
            .field("metadata", &self.metadata)
            .field("byte_count", &self.bytes.len())
            .field("range", &self.range)
            .finish()
    }
}

impl ObjectRangeBodyV1 {
    pub fn new(
        metadata: ObjectMetadataV1,
        bytes: Vec<u8>,
        range: ObjectByteRange,
    ) -> Result<Self, StorageFailure> {
        let byte_count = u64::try_from(bytes.len())
            .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?;
        if byte_count != range.length() || range.end_exclusive > metadata.size.get() {
            return Err(StorageFailure::new(StorageFailureKind::Integrity));
        }
        Ok(Self {
            metadata,
            bytes,
            range,
        })
    }

    #[must_use]
    pub const fn metadata(&self) -> &ObjectMetadataV1 {
        &self.metadata
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn range(&self) -> ObjectByteRange {
        self.range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyObjectRequestV1 {
    source: ScopedObjectKey,
    destination: ScopedObjectKey,
    expected_source_version: Option<ProviderObjectVersion>,
}

impl CopyObjectRequestV1 {
    pub fn immutable(
        source: ScopedObjectKey,
        destination: ScopedObjectKey,
    ) -> Result<Self, StorageFailure> {
        if source == destination
            || source.tenant_id() != destination.tenant_id()
            || source.video_id() != destination.video_id()
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            source,
            destination,
            expected_source_version: None,
        })
    }

    #[must_use]
    pub fn if_source_version(mut self, expected: ProviderObjectVersion) -> Self {
        self.expected_source_version = Some(expected);
        self
    }

    #[must_use]
    pub const fn source(&self) -> &ScopedObjectKey {
        &self.source
    }

    #[must_use]
    pub const fn destination(&self) -> &ScopedObjectKey {
        &self.destination
    }

    #[must_use]
    pub const fn expected_source_version(&self) -> Option<&ProviderObjectVersion> {
        self.expected_source_version.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteObjectRequestV1 {
    key: ScopedObjectKey,
    expected_version: Option<ProviderObjectVersion>,
}

impl DeleteObjectRequestV1 {
    #[must_use]
    pub const fn idempotent(key: ScopedObjectKey) -> Self {
        Self {
            key,
            expected_version: None,
        }
    }

    #[must_use]
    pub fn if_version(key: ScopedObjectKey, expected_version: ProviderObjectVersion) -> Self {
        Self {
            key,
            expected_version: Some(expected_version),
        }
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn expected_version(&self) -> Option<&ProviderObjectVersion> {
        self.expected_version.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteObjectDisposition {
    Deleted,
    AlreadyAbsent,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StorageListCursor(String);

impl StorageListCursor {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageFailure> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 1_024
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.')
            })
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_adapter(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for StorageListCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StorageListCursor([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListObjectsRequestV1 {
    tenant_id: TenantId,
    video_id: VideoId,
    role: Option<ObjectRole>,
    cursor: Option<StorageListCursor>,
    limit: u16,
}

impl ListObjectsRequestV1 {
    pub fn new(
        tenant_id: TenantId,
        video_id: VideoId,
        role: Option<ObjectRole>,
        cursor: Option<StorageListCursor>,
        limit: u16,
    ) -> Result<Self, StorageFailure> {
        if !(1..=100).contains(&limit) {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            tenant_id,
            video_id,
            role,
            cursor,
            limit,
        })
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
    pub const fn role(&self) -> Option<ObjectRole> {
        self.role
    }

    #[must_use]
    pub const fn cursor(&self) -> Option<&StorageListCursor> {
        self.cursor.as_ref()
    }

    #[must_use]
    pub const fn limit(&self) -> u16 {
        self.limit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListObjectsPageV1 {
    pub items: Vec<ObjectMetadataV1>,
    pub next_cursor: Option<StorageListCursor>,
}

#[async_trait]
pub trait ObjectStoreV1: Send + Sync {
    fn capabilities(&self) -> ObjectStoreCapabilitiesV1;

    async fn put(
        &self,
        context: StorageRequestContext,
        request: PutObjectRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure>;
    async fn head(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<ObjectMetadataV1, StorageFailure>;
    async fn get(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<ObjectBodyV1, StorageFailure>;
    async fn get_range(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
        range: ObjectByteRange,
    ) -> Result<ObjectRangeBodyV1, StorageFailure>;
    async fn copy(
        &self,
        context: StorageRequestContext,
        request: CopyObjectRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure>;
    async fn delete(
        &self,
        context: StorageRequestContext,
        request: DeleteObjectRequestV1,
    ) -> Result<DeleteObjectDisposition, StorageFailure>;
    async fn list(
        &self,
        context: StorageRequestContext,
        request: ListObjectsRequestV1,
    ) -> Result<ListObjectsPageV1, StorageFailure>;
}

#[derive(Debug, Clone)]
struct StoredObjectV1 {
    metadata: ObjectMetadataV1,
    bytes: Vec<u8>,
}

pub struct DeterministicObjectStore {
    capabilities: ObjectStoreCapabilitiesV1,
    objects: RwLock<BTreeMap<String, StoredObjectV1>>,
    failures: Mutex<BTreeMap<ObjectStoreOperation, VecDeque<StorageFailure>>>,
    version: AtomicU64,
    clock: AtomicI64,
}

impl DeterministicObjectStore {
    #[must_use]
    pub fn new(capabilities: ObjectStoreCapabilitiesV1) -> Self {
        Self {
            capabilities,
            objects: RwLock::new(BTreeMap::new()),
            failures: Mutex::new(BTreeMap::new()),
            version: AtomicU64::new(0),
            clock: AtomicI64::new(0),
        }
    }

    pub fn inject_failure(
        &self,
        operation: ObjectStoreOperation,
        failure: StorageFailure,
    ) -> Result<(), StorageFailure> {
        if !is_fault_injectable_operation(operation) {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        self.failures
            .lock()
            .map_err(|_| unavailable())?
            .entry(operation)
            .or_default()
            .push_back(failure);
        Ok(())
    }

    pub fn object_count(&self) -> Result<usize, StorageFailure> {
        Ok(self.objects.read().map_err(|_| unavailable())?.len())
    }

    fn guard(&self, operation: ObjectStoreOperation) -> Result<(), StorageFailure> {
        self.capabilities.require(operation)?;
        let mut failures = self.failures.lock().map_err(|_| unavailable())?;
        if let Some(failure) = failures.get_mut(&operation).and_then(VecDeque::pop_front) {
            return Err(failure);
        }
        Ok(())
    }

    fn authorize(
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<(), StorageFailure> {
        if key.tenant_id() == context.tenant_id {
            Ok(())
        } else {
            // Scope violations are deliberately indistinguishable from missing objects.
            Err(StorageFailure::new(StorageFailureKind::NotFound))
        }
    }

    fn next_tokens(
        &self,
        checksum: &ChecksumSha256,
    ) -> Result<(ProviderObjectVersion, ProviderEntityTag, TimestampMillis), StorageFailure> {
        let value = self.version.fetch_add(1, Ordering::Relaxed) + 1;
        let time = self.clock.fetch_add(1, Ordering::Relaxed) + 1;
        Ok((
            ProviderObjectVersion::parse(format!("fake-v{value:016}"))?,
            ProviderEntityTag::parse(format!("sha256:{}.v{value}", checksum.as_str()))?,
            TimestampMillis::new(time).map_err(|_| unavailable())?,
        ))
    }

    fn write(
        &self,
        context: StorageRequestContext,
        key: ScopedObjectKey,
        bytes: Vec<u8>,
        content_type: ContentType,
        checksum_sha256: ChecksumSha256,
        cache_policy: ObjectCachePolicy,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
        Self::authorize(context, &key)?;
        self.capabilities
            .require(ObjectStoreOperation::ConditionalCreate)?;
        let size = ByteSize::new(
            u64::try_from(bytes.len())
                .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?,
        )
        .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?;
        if size.get() == 0 || size > self.capabilities.max_object_size {
            return Err(StorageFailure::new(StorageFailureKind::QuotaExceeded));
        }
        if ChecksumSha256::digest_bytes(&bytes) != checksum_sha256 {
            return Err(StorageFailure::new(StorageFailureKind::Integrity));
        }
        let mut objects = self.objects.write().map_err(|_| unavailable())?;
        if objects.contains_key(key.as_str()) {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        let (provider_version, provider_etag, last_modified) =
            self.next_tokens(&checksum_sha256)?;
        let metadata = ObjectMetadataV1::new(
            key.clone(),
            size,
            content_type,
            checksum_sha256,
            provider_version,
            provider_etag,
            cache_policy,
            last_modified,
            context.correlation_id,
        )?;
        objects.insert(
            key.as_str().to_owned(),
            StoredObjectV1 {
                metadata: metadata.clone(),
                bytes,
            },
        );
        Ok(ObjectWriteReceiptV1::new(metadata))
    }
}

impl fmt::Debug for DeterministicObjectStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeterministicObjectStore")
            .field("capabilities", &self.capabilities)
            .field("object_count", &self.object_count().unwrap_or_default())
            .finish()
    }
}

#[async_trait]
impl ObjectStoreV1 for DeterministicObjectStore {
    fn capabilities(&self) -> ObjectStoreCapabilitiesV1 {
        self.capabilities
    }

    async fn put(
        &self,
        context: StorageRequestContext,
        request: PutObjectRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
        Self::authorize(context, &request.key)?;
        self.guard(ObjectStoreOperation::Put)?;
        self.capabilities
            .require(ObjectStoreOperation::Sha256Integrity)?;
        self.write(
            context,
            request.key,
            request.bytes,
            request.content_type,
            request.checksum_sha256,
            request.cache_policy,
        )
    }

    async fn head(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<ObjectMetadataV1, StorageFailure> {
        Self::authorize(context, key)?;
        self.guard(ObjectStoreOperation::Head)?;
        self.objects
            .read()
            .map_err(|_| unavailable())?
            .get(key.as_str())
            .map(|object| object.metadata.clone())
            .ok_or_else(|| StorageFailure::new(StorageFailureKind::NotFound))
    }

    async fn get(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<ObjectBodyV1, StorageFailure> {
        Self::authorize(context, key)?;
        self.guard(ObjectStoreOperation::Get)?;
        let object = self
            .objects
            .read()
            .map_err(|_| unavailable())?
            .get(key.as_str())
            .cloned()
            .ok_or_else(|| StorageFailure::new(StorageFailureKind::NotFound))?;
        ObjectBodyV1::new(object.metadata, object.bytes)
    }

    async fn get_range(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
        range: ObjectByteRange,
    ) -> Result<ObjectRangeBodyV1, StorageFailure> {
        Self::authorize(context, key)?;
        self.guard(ObjectStoreOperation::Range)?;
        if range.length() > self.capabilities.max_range_size.get() {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        let object = self
            .objects
            .read()
            .map_err(|_| unavailable())?
            .get(key.as_str())
            .cloned()
            .ok_or_else(|| StorageFailure::new(StorageFailureKind::NotFound))?;
        let object_length = u64::try_from(object.bytes.len()).map_err(|_| unavailable())?;
        if range.start >= object_length {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        let end_exclusive = range.end_exclusive.min(object_length);
        let start = usize::try_from(range.start)
            .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?;
        let end = usize::try_from(end_exclusive)
            .map_err(|_| StorageFailure::new(StorageFailureKind::InvalidRequest))?;
        ObjectRangeBodyV1::new(
            object.metadata,
            object.bytes[start..end].to_vec(),
            ObjectByteRange {
                start: range.start,
                end_exclusive,
            },
        )
    }

    async fn copy(
        &self,
        context: StorageRequestContext,
        request: CopyObjectRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
        Self::authorize(context, &request.source)?;
        Self::authorize(context, &request.destination)?;
        self.guard(ObjectStoreOperation::Copy)?;
        if request.expected_source_version.is_some() {
            self.capabilities
                .require(ObjectStoreOperation::ConditionalSourceVersion)?;
        }
        let source = self
            .objects
            .read()
            .map_err(|_| unavailable())?
            .get(request.source.as_str())
            .cloned()
            .ok_or_else(|| {
                StorageFailure::new(if request.expected_source_version.is_some() {
                    StorageFailureKind::PreconditionFailed
                } else {
                    StorageFailureKind::NotFound
                })
            })?;
        if request
            .expected_source_version
            .as_ref()
            .is_some_and(|expected| source.metadata.provider_version != *expected)
        {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        self.write(
            context,
            request.destination,
            source.bytes,
            source.metadata.content_type,
            source.metadata.checksum_sha256,
            source.metadata.cache_policy,
        )
    }

    async fn delete(
        &self,
        context: StorageRequestContext,
        request: DeleteObjectRequestV1,
    ) -> Result<DeleteObjectDisposition, StorageFailure> {
        Self::authorize(context, &request.key)?;
        self.guard(ObjectStoreOperation::Delete)?;
        if request.expected_version.is_some() {
            self.capabilities
                .require(ObjectStoreOperation::ConditionalDeleteVersion)?;
        }
        let mut objects = self.objects.write().map_err(|_| unavailable())?;
        let Some(existing) = objects.get(request.key.as_str()) else {
            if request.expected_version.is_some() {
                return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
            }
            return Ok(DeleteObjectDisposition::AlreadyAbsent);
        };
        if request
            .expected_version
            .as_ref()
            .is_some_and(|expected| existing.metadata.provider_version != *expected)
        {
            return Err(StorageFailure::new(StorageFailureKind::PreconditionFailed));
        }
        objects.remove(request.key.as_str());
        Ok(DeleteObjectDisposition::Deleted)
    }

    async fn list(
        &self,
        context: StorageRequestContext,
        request: ListObjectsRequestV1,
    ) -> Result<ListObjectsPageV1, StorageFailure> {
        if request.tenant_id != context.tenant_id {
            return Err(StorageFailure::new(StorageFailureKind::NotFound));
        }
        self.guard(ObjectStoreOperation::List)?;
        if request.limit > self.capabilities.max_list_page_size {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        let cursor = request
            .cursor
            .as_ref()
            .map(StorageListCursor::expose_for_adapter);
        let objects = self.objects.read().map_err(|_| unavailable())?;
        let mut matches = objects
            .iter()
            .filter(|(key, object)| {
                cursor.is_none_or(|cursor| key.as_str() > cursor)
                    && object
                        .metadata
                        .key
                        .belongs_to(request.tenant_id, request.video_id)
                    && request
                        .role
                        .is_none_or(|role| object.metadata.key.role() == role)
            })
            .map(|(_, object)| object.metadata.clone())
            .take(usize::from(request.limit) + 1)
            .collect::<Vec<_>>();
        let has_more = matches.len() > usize::from(request.limit);
        if has_more {
            matches.pop();
        }
        let next_cursor = if has_more {
            matches
                .last()
                .map(|metadata| StorageListCursor(metadata.key.as_str().to_owned()))
        } else {
            None
        };
        Ok(ListObjectsPageV1 {
            items: matches,
            next_cursor,
        })
    }
}

fn unavailable() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::Unavailable)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadMode {
    BrokeredSinglePut,
    DirectSinglePut,
    Multipart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadBrokerCapabilitiesV1 {
    contract_version: u16,
    brokered_single_put: bool,
    direct_single_put: bool,
    multipart: bool,
    sha256_required: bool,
    max_object_size: ByteSize,
}

impl UploadBrokerCapabilitiesV1 {
    pub fn new(
        brokered_single_put: bool,
        direct_single_put: bool,
        multipart: bool,
        sha256_required: bool,
        max_object_size: ByteSize,
    ) -> Result<Self, StorageFailure> {
        if (!brokered_single_put && !direct_single_put && !multipart)
            || max_object_size.get() == 0
            || ByteSize::new(max_object_size.get()).is_err()
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            contract_version: UPLOAD_BROKER_CONTRACT_VERSION,
            brokered_single_put,
            direct_single_put,
            multipart,
            sha256_required,
            max_object_size,
        })
    }

    #[must_use]
    pub const fn contract_version(self) -> u16 {
        self.contract_version
    }

    #[must_use]
    pub const fn max_object_size(self) -> ByteSize {
        self.max_object_size
    }

    #[must_use]
    pub const fn sha256_required(self) -> bool {
        self.sha256_required
    }

    #[must_use]
    pub const fn supports(self, mode: UploadMode) -> bool {
        match mode {
            UploadMode::BrokeredSinglePut => self.brokered_single_put,
            UploadMode::DirectSinglePut => self.direct_single_put,
            UploadMode::Multipart => self.multipart,
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BrokerUploadId(String);

impl BrokerUploadId {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageFailure> {
        let value = value.into();
        if !(8..=128).contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_adapter(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BrokerUploadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrokerUploadId([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SameOriginUploadPath(String);

impl SameOriginUploadPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageFailure> {
        let value = value.into();
        let path_segments_valid = value.strip_prefix('/').is_some_and(|path| {
            !path.is_empty()
                && path
                    .split('/')
                    .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
        });
        if value.len() > 256
            || !value.starts_with('/')
            || value.starts_with("//")
            || !path_segments_valid
            || value.contains('?')
            || value.contains('#')
            || value
                .split('/')
                .any(|segment| matches!(segment, "." | ".."))
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_'))
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SameOriginUploadPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SameOriginUploadPath([redacted])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DirectUploadAuthorization {
    https_url: String,
    headers: BTreeMap<String, String>,
}

impl DirectUploadAuthorization {
    pub fn new(
        https_url: impl Into<String>,
        headers: BTreeMap<String, String>,
    ) -> Result<Self, StorageFailure> {
        let https_url = https_url.into();
        let authority = https_url
            .strip_prefix("https://")
            .and_then(|remainder| remainder.split(['/', '?']).next());
        let valid_url = (12..=2_048).contains(&https_url.len())
            && https_url.is_ascii()
            && authority.is_some_and(valid_https_authority)
            && !https_url.contains('#')
            && !https_url.contains('\\')
            && !https_url
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control());
        let valid_headers = headers.len() <= 16
            && headers.iter().all(|(name, value)| {
                (1..=64).contains(&name.len())
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    && value.len() <= 4_096
                    && !value.bytes().any(|byte| byte.is_ascii_control())
            })
            && headers.keys().enumerate().all(|(index, name)| {
                headers
                    .keys()
                    .skip(index + 1)
                    .all(|other| !name.eq_ignore_ascii_case(other))
            });
        if !valid_url || !valid_headers {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self { https_url, headers })
    }

    #[must_use]
    pub fn expose_url_for_delivery(&self) -> &str {
        &self.https_url
    }

    #[must_use]
    pub const fn expose_headers_for_delivery(&self) -> &BTreeMap<String, String> {
        &self.headers
    }
}

fn valid_https_authority(authority: &str) -> bool {
    if authority.is_empty() || authority.contains('@') || authority.contains('\\') {
        return false;
    }
    if let Some(ipv6) = authority.strip_prefix('[') {
        let Some((address, suffix)) = ipv6.split_once(']') else {
            return false;
        };
        return !address.is_empty()
            && address
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b':' | b'.'))
            && valid_optional_port(suffix);
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, Some(port)),
        Some(_) => return false,
        None => (authority, None),
    };
    let host = host.strip_suffix('.').unwrap_or(host);
    let valid_host = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .as_bytes()
                    .first()
                    .zip(label.as_bytes().last())
                    .is_some_and(|(first, last)| {
                        first.is_ascii_alphanumeric() && last.is_ascii_alphanumeric()
                    })
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    valid_host && port.is_none_or(valid_port)
}

fn valid_optional_port(suffix: &str) -> bool {
    suffix.is_empty() || suffix.strip_prefix(':').is_some_and(valid_port)
}

fn valid_port(value: &str) -> bool {
    value
        .parse::<u16>()
        .is_ok_and(|port| port != 0 && value.bytes().all(|byte| byte.is_ascii_digit()))
}

impl fmt::Debug for DirectUploadAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectUploadAuthorization")
            .field("https_url", &"[redacted]")
            .field("header_count", &self.headers.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadDelivery {
    Brokered {
        path: SameOriginUploadPath,
    },
    Direct {
        authorization: DirectUploadAuthorization,
    },
    MultipartBrokered {
        path: SameOriginUploadPath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginUploadRequestV1 {
    key: ScopedObjectKey,
    mode: UploadMode,
    expected_size: ByteSize,
    content_type: ContentType,
    checksum_sha256: ChecksumSha256,
    cache_policy: ObjectCachePolicy,
    expires_at: TimestampMillis,
}

impl BeginUploadRequestV1 {
    pub fn new(
        key: ScopedObjectKey,
        mode: UploadMode,
        expected_size: ByteSize,
        content_type: ContentType,
        checksum_sha256: ChecksumSha256,
        cache_policy: ObjectCachePolicy,
        expires_at: TimestampMillis,
    ) -> Result<Self, StorageFailure> {
        if expected_size.get() == 0
            || ByteSize::new(expected_size.get()).is_err()
            || ContentType::parse(content_type.as_str()).is_err()
            || ChecksumSha256::parse(checksum_sha256.as_str()).is_err()
            || TimestampMillis::new(expires_at.get()).is_err()
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            key,
            mode,
            expected_size,
            content_type,
            checksum_sha256,
            cache_policy,
            expires_at,
        })
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn mode(&self) -> UploadMode {
        self.mode
    }

    #[must_use]
    pub const fn expected_size(&self) -> ByteSize {
        self.expected_size
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn cache_policy(&self) -> ObjectCachePolicy {
        self.cache_policy
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPlanV1 {
    id: BrokerUploadId,
    key: ScopedObjectKey,
    mode: UploadMode,
    delivery: UploadDelivery,
    expected_size: ByteSize,
    content_type: ContentType,
    checksum_sha256: ChecksumSha256,
    cache_policy: ObjectCachePolicy,
    expires_at: TimestampMillis,
    correlation_id: CorrelationId,
}

impl UploadPlanV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: BrokerUploadId,
        key: ScopedObjectKey,
        mode: UploadMode,
        delivery: UploadDelivery,
        expected_size: ByteSize,
        content_type: ContentType,
        checksum_sha256: ChecksumSha256,
        cache_policy: ObjectCachePolicy,
        expires_at: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> Result<Self, StorageFailure> {
        if expected_size.get() == 0
            || ByteSize::new(expected_size.get()).is_err()
            || ContentType::parse(content_type.as_str()).is_err()
            || ChecksumSha256::parse(checksum_sha256.as_str()).is_err()
            || TimestampMillis::new(expires_at.get()).is_err()
            || CorrelationId::parse(&correlation_id.to_string()).is_err()
            || !delivery_matches_mode(&delivery, mode)
        {
            return Err(StorageFailure::new(StorageFailureKind::InvalidRequest));
        }
        Ok(Self {
            id,
            key,
            mode,
            delivery,
            expected_size,
            content_type,
            checksum_sha256,
            cache_policy,
            expires_at,
            correlation_id,
        })
    }

    #[must_use]
    pub const fn id(&self) -> &BrokerUploadId {
        &self.id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn mode(&self) -> UploadMode {
        self.mode
    }

    #[must_use]
    pub const fn delivery(&self) -> &UploadDelivery {
        &self.delivery
    }

    #[must_use]
    pub const fn expected_size(&self) -> ByteSize {
        self.expected_size
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn cache_policy(&self) -> ObjectCachePolicy {
        self.cache_policy
    }

    #[must_use]
    pub const fn expires_at(&self) -> TimestampMillis {
        self.expires_at
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

const fn delivery_matches_mode(delivery: &UploadDelivery, mode: UploadMode) -> bool {
    matches!(
        (delivery, mode),
        (
            UploadDelivery::Brokered { .. },
            UploadMode::BrokeredSinglePut
        ) | (UploadDelivery::Direct { .. }, UploadMode::DirectSinglePut)
            | (
                UploadDelivery::MultipartBrokered { .. },
                UploadMode::Multipart
            )
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteUploadRequestV1 {
    id: BrokerUploadId,
    receipt: ObjectWriteReceiptV1,
}

impl CompleteUploadRequestV1 {
    #[must_use]
    pub const fn new(id: BrokerUploadId, receipt: ObjectWriteReceiptV1) -> Self {
        Self { id, receipt }
    }

    #[must_use]
    pub const fn id(&self) -> &BrokerUploadId {
        &self.id
    }

    #[must_use]
    pub const fn receipt(&self) -> &ObjectWriteReceiptV1 {
        &self.receipt
    }
}

#[async_trait]
pub trait UploadBrokerV1: Send + Sync {
    fn capabilities(&self) -> UploadBrokerCapabilitiesV1;
    async fn begin(
        &self,
        context: StorageRequestContext,
        request: BeginUploadRequestV1,
    ) -> Result<UploadPlanV1, StorageFailure>;
    async fn complete(
        &self,
        context: StorageRequestContext,
        request: CompleteUploadRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure>;
    async fn abort(
        &self,
        context: StorageRequestContext,
        id: &BrokerUploadId,
    ) -> Result<(), StorageFailure>;
}

#[derive(Debug, Clone)]
struct PendingUpload {
    tenant_id: TenantId,
    key: ScopedObjectKey,
    expected_size: ByteSize,
    content_type: ContentType,
    checksum_sha256: ChecksumSha256,
    cache_policy: ObjectCachePolicy,
    correlation_id: CorrelationId,
    completed: Option<ObjectWriteReceiptV1>,
}

pub struct DeterministicUploadBroker {
    capabilities: UploadBrokerCapabilitiesV1,
    sequence: AtomicU64,
    begin_calls: AtomicU64,
    uploads: Mutex<BTreeMap<BrokerUploadId, PendingUpload>>,
}

impl DeterministicUploadBroker {
    #[must_use]
    pub fn new(capabilities: UploadBrokerCapabilitiesV1) -> Self {
        Self {
            capabilities,
            sequence: AtomicU64::new(0),
            begin_calls: AtomicU64::new(0),
            uploads: Mutex::new(BTreeMap::new()),
        }
    }

    #[must_use]
    pub fn begin_call_count(&self) -> u64 {
        self.begin_calls.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for DeterministicUploadBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeterministicUploadBroker")
            .field("capabilities", &self.capabilities)
            .field("begin_calls", &self.begin_call_count())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl UploadBrokerV1 for DeterministicUploadBroker {
    fn capabilities(&self) -> UploadBrokerCapabilitiesV1 {
        self.capabilities
    }

    async fn begin(
        &self,
        context: StorageRequestContext,
        request: BeginUploadRequestV1,
    ) -> Result<UploadPlanV1, StorageFailure> {
        if request.key.tenant_id() != context.tenant_id {
            return Err(StorageFailure::new(StorageFailureKind::NotFound));
        }
        self.begin_calls.fetch_add(1, Ordering::Relaxed);
        if !self.capabilities.supports(request.mode) {
            return Err(StorageFailure::unsupported());
        }
        if !self.capabilities.sha256_required() {
            return Err(StorageFailure::unsupported());
        }
        if request.expected_size > self.capabilities.max_object_size {
            return Err(StorageFailure::new(StorageFailureKind::QuotaExceeded));
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let id = BrokerUploadId::parse(format!("fake-upload-{sequence:016}"))?;
        let path = SameOriginUploadPath::parse(format!("/api/storage/uploads/{sequence}"))?;
        let delivery = match request.mode {
            UploadMode::BrokeredSinglePut => UploadDelivery::Brokered { path },
            UploadMode::Multipart => UploadDelivery::MultipartBrokered { path },
            UploadMode::DirectSinglePut => {
                let mut headers = BTreeMap::new();
                headers.insert("x-frame-upload".to_owned(), format!("fake-{sequence}"));
                UploadDelivery::Direct {
                    authorization: DirectUploadAuthorization::new(
                        format!("https://upload.invalid/{sequence}"),
                        headers,
                    )?,
                }
            }
        };
        let plan = UploadPlanV1::new(
            id.clone(),
            request.key.clone(),
            request.mode,
            delivery,
            request.expected_size,
            request.content_type.clone(),
            request.checksum_sha256.clone(),
            request.cache_policy,
            request.expires_at,
            context.correlation_id,
        )?;
        let pending = PendingUpload {
            tenant_id: context.tenant_id,
            key: request.key.clone(),
            expected_size: request.expected_size,
            content_type: request.content_type,
            checksum_sha256: request.checksum_sha256,
            cache_policy: request.cache_policy,
            correlation_id: context.correlation_id,
            completed: None,
        };
        self.uploads
            .lock()
            .map_err(|_| unavailable())?
            .insert(id.clone(), pending);
        Ok(plan)
    }

    async fn complete(
        &self,
        context: StorageRequestContext,
        request: CompleteUploadRequestV1,
    ) -> Result<ObjectWriteReceiptV1, StorageFailure> {
        let mut uploads = self.uploads.lock().map_err(|_| unavailable())?;
        let pending = uploads
            .get_mut(&request.id)
            .ok_or_else(|| StorageFailure::new(StorageFailureKind::NotFound))?;
        if pending.tenant_id != context.tenant_id {
            return Err(StorageFailure::new(StorageFailureKind::NotFound));
        }
        if let Some(completed) = &pending.completed {
            return if completed == &request.receipt {
                Ok(completed.clone())
            } else {
                Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
            };
        }
        let metadata = request.receipt.metadata();
        if metadata.key != pending.key
            || metadata.size != pending.expected_size
            || metadata.content_type != pending.content_type
            || metadata.checksum_sha256 != pending.checksum_sha256
            || metadata.cache_policy != pending.cache_policy
            || metadata.correlation_id != pending.correlation_id
            || context.correlation_id != pending.correlation_id
        {
            return Err(StorageFailure::new(StorageFailureKind::Integrity));
        }
        pending.completed = Some(request.receipt.clone());
        Ok(request.receipt)
    }

    async fn abort(
        &self,
        context: StorageRequestContext,
        id: &BrokerUploadId,
    ) -> Result<(), StorageFailure> {
        let mut uploads = self.uploads.lock().map_err(|_| unavailable())?;
        let Some(pending) = uploads.get(id) else {
            return Ok(());
        };
        if pending.tenant_id != context.tenant_id || pending.completed.is_some() {
            return Ok(());
        }
        uploads.remove(id);
        Ok(())
    }
}
