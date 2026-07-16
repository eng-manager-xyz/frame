use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    sync::{Mutex, RwLock},
};

use async_trait::async_trait;
use frame_domain::{
    ByteSize, ChecksumSha256, ContentType, CorrelationId, CorsOriginV1, IdempotencyKey,
    MultipartGrantId, MultipartGrantRecordV1, MultipartOperationV1, MultipartPartNumberV1,
    MultipartUploadId, MultipartUploadSpecV1, ScopedObjectKey, TenantId, TimestampMillis,
    TrustedMediaProbeV1,
};

use crate::{
    ObjectByteRange, ObjectCachePolicy, ProviderEntityTag, ProviderObjectVersion, StorageFailure,
    StorageFailureKind, StorageRequestContext,
};

pub const MULTIPART_PROVIDER_CONTRACT_VERSION: u16 = 1;
pub const MULTIPART_JOURNAL_CONTRACT_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MultipartProviderOperationV1 {
    Create,
    Lookup,
    ListParts,
    PutPart,
    Complete,
    Abort,
    Head,
    Get,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultipartProviderCapabilitiesV1 {
    contract_version: u16,
    operations: [bool; 8],
    min_part_size: ByteSize,
    max_part_size: ByteSize,
    max_part_count: u16,
    max_total_size: ByteSize,
    max_range_size: ByteSize,
    checksum_sha256: bool,
}

impl MultipartProviderCapabilitiesV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn full(
        min_part_size: ByteSize,
        max_part_size: ByteSize,
        max_part_count: u16,
        max_total_size: ByteSize,
        max_range_size: ByteSize,
        checksum_sha256: bool,
    ) -> Result<Self, StorageFailure> {
        if min_part_size.get() == 0
            || min_part_size > max_part_size
            || !(1..=10_000).contains(&max_part_count)
            || max_total_size < min_part_size
            || max_range_size.get() == 0
        {
            return Err(invalid());
        }
        Ok(Self {
            contract_version: MULTIPART_PROVIDER_CONTRACT_VERSION,
            operations: [true; 8],
            min_part_size,
            max_part_size,
            max_part_count,
            max_total_size,
            max_range_size,
            checksum_sha256,
        })
    }

    #[must_use]
    pub const fn contract_version(self) -> u16 {
        self.contract_version
    }

    #[must_use]
    pub fn supports(self, operation: MultipartProviderOperationV1) -> bool {
        self.operations[provider_operation_index(operation)]
    }

    #[must_use]
    pub fn without(mut self, operation: MultipartProviderOperationV1) -> Self {
        self.operations[provider_operation_index(operation)] = false;
        self
    }

    pub fn require(self, operation: MultipartProviderOperationV1) -> Result<(), StorageFailure> {
        if self.supports(operation) {
            Ok(())
        } else {
            Err(StorageFailure::unsupported())
        }
    }

    #[must_use]
    pub const fn min_part_size(self) -> ByteSize {
        self.min_part_size
    }

    #[must_use]
    pub const fn max_part_size(self) -> ByteSize {
        self.max_part_size
    }

    #[must_use]
    pub const fn max_part_count(self) -> u16 {
        self.max_part_count
    }

    #[must_use]
    pub const fn max_total_size(self) -> ByteSize {
        self.max_total_size
    }

    #[must_use]
    pub const fn max_range_size(self) -> ByteSize {
        self.max_range_size
    }

    #[must_use]
    pub const fn checksum_sha256(self) -> bool {
        self.checksum_sha256
    }
}

const fn provider_operation_index(operation: MultipartProviderOperationV1) -> usize {
    match operation {
        MultipartProviderOperationV1::Create => 0,
        MultipartProviderOperationV1::Lookup => 1,
        MultipartProviderOperationV1::ListParts => 2,
        MultipartProviderOperationV1::PutPart => 3,
        MultipartProviderOperationV1::Complete => 4,
        MultipartProviderOperationV1::Abort => 5,
        MultipartProviderOperationV1::Head => 6,
        MultipartProviderOperationV1::Get => 7,
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderMultipartHandleV1(String);

impl ProviderMultipartHandleV1 {
    pub fn parse(value: impl Into<String>) -> Result<Self, StorageFailure> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 256
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(invalid());
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn expose_for_provider(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ProviderMultipartHandleV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderMultipartHandleV1([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCreateMultipartRequestV1 {
    upload_id: MultipartUploadId,
    spec: MultipartUploadSpecV1,
    expires_at: TimestampMillis,
    correlation_id: CorrelationId,
}

impl ProviderCreateMultipartRequestV1 {
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        spec: MultipartUploadSpecV1,
        expires_at: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            spec,
            expires_at,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn spec(&self) -> &MultipartUploadSpecV1 {
        &self.spec
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        self.spec.key()
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLookupMultipartRequestV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    correlation_id: CorrelationId,
}

impl ProviderLookupMultipartRequestV1 {
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            key,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMultipartSessionV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    handle: ProviderMultipartHandleV1,
    expires_at: TimestampMillis,
    correlation_id: CorrelationId,
}

impl ProviderMultipartSessionV1 {
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        handle: ProviderMultipartHandleV1,
        expires_at: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            key,
            handle,
            expires_at,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn handle(&self) -> &ProviderMultipartHandleV1 {
        &self.handle
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderUploadReferenceV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    handle: ProviderMultipartHandleV1,
    correlation_id: CorrelationId,
}

impl ProviderUploadReferenceV1 {
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        handle: ProviderMultipartHandleV1,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            key,
            handle,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn handle(&self) -> &ProviderMultipartHandleV1 {
        &self.handle
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPartReceiptV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    part_number: MultipartPartNumberV1,
    size: ByteSize,
    checksum_sha256: ChecksumSha256,
    etag: ProviderEntityTag,
    correlation_id: CorrelationId,
}

impl ProviderPartReceiptV1 {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        part_number: MultipartPartNumberV1,
        size: ByteSize,
        checksum_sha256: ChecksumSha256,
        etag: ProviderEntityTag,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            key,
            part_number,
            size,
            checksum_sha256,
            etag,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn part_number(&self) -> MultipartPartNumberV1 {
        self.part_number
    }

    #[must_use]
    pub const fn size(&self) -> ByteSize {
        self.size
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn etag(&self) -> &ProviderEntityTag {
        &self.etag
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderPutPartRequestV1 {
    reference: ProviderUploadReferenceV1,
    part_number: MultipartPartNumberV1,
    checksum_sha256: ChecksumSha256,
    bytes: Vec<u8>,
}

impl fmt::Debug for ProviderPutPartRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderPutPartRequestV1")
            .field("reference", &self.reference)
            .field("part_number", &self.part_number)
            .field("checksum_sha256", &self.checksum_sha256)
            .field("byte_length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl ProviderPutPartRequestV1 {
    #[must_use]
    pub fn new(
        reference: ProviderUploadReferenceV1,
        part_number: MultipartPartNumberV1,
        checksum_sha256: ChecksumSha256,
        bytes: Vec<u8>,
    ) -> Self {
        Self {
            reference,
            part_number,
            checksum_sha256,
            bytes,
        }
    }

    #[must_use]
    pub const fn reference(&self) -> &ProviderUploadReferenceV1 {
        &self.reference
    }

    #[must_use]
    pub const fn part_number(&self) -> MultipartPartNumberV1 {
        self.part_number
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPartsListV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    parts: Vec<ProviderPartReceiptV1>,
    correlation_id: CorrelationId,
}

impl ProviderPartsListV1 {
    pub fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        parts: Vec<ProviderPartReceiptV1>,
        correlation_id: CorrelationId,
    ) -> Result<Self, StorageFailure> {
        let ordered = parts
            .windows(2)
            .all(|pair| pair[0].part_number().get() < pair[1].part_number().get());
        let bound = parts.iter().all(|part| {
            part.upload_id() == upload_id
                && part.key() == &key
                && part.correlation_id() == correlation_id
        });
        if !ordered || !bound {
            return Err(invalid());
        }
        Ok(Self {
            upload_id,
            key,
            parts,
            correlation_id,
        })
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub fn parts(&self) -> &[ProviderPartReceiptV1] {
        &self.parts
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCompleteMultipartRequestV1 {
    reference: ProviderUploadReferenceV1,
    parts: Vec<ProviderPartReceiptV1>,
    expected_size: ByteSize,
    expected_checksum_sha256: ChecksumSha256,
    expected_content_type: ContentType,
}

impl ProviderCompleteMultipartRequestV1 {
    pub fn new(
        reference: ProviderUploadReferenceV1,
        parts: Vec<ProviderPartReceiptV1>,
        expected_size: ByteSize,
        expected_checksum_sha256: ChecksumSha256,
        expected_content_type: ContentType,
    ) -> Result<Self, StorageFailure> {
        let ordered = parts
            .windows(2)
            .all(|pair| pair[0].part_number().get() < pair[1].part_number().get());
        let bound = parts.iter().all(|part| {
            part.upload_id() == reference.upload_id()
                && part.key() == reference.key()
                && part.correlation_id() == reference.correlation_id()
        });
        if parts.is_empty() || !ordered || !bound {
            return Err(invalid());
        }
        Ok(Self {
            reference,
            parts,
            expected_size,
            expected_checksum_sha256,
            expected_content_type,
        })
    }

    #[must_use]
    pub const fn reference(&self) -> &ProviderUploadReferenceV1 {
        &self.reference
    }

    #[must_use]
    pub fn parts(&self) -> &[ProviderPartReceiptV1] {
        &self.parts
    }

    #[must_use]
    pub const fn expected_size(&self) -> ByteSize {
        self.expected_size
    }

    #[must_use]
    pub const fn expected_checksum_sha256(&self) -> &ChecksumSha256 {
        &self.expected_checksum_sha256
    }

    #[must_use]
    pub const fn expected_content_type(&self) -> &ContentType {
        &self.expected_content_type
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCompletedObjectV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    size: ByteSize,
    checksum_sha256: ChecksumSha256,
    content_type: ContentType,
    provider_version: ProviderObjectVersion,
    provider_etag: ProviderEntityTag,
    last_modified: TimestampMillis,
    media_probe: TrustedMediaProbeV1,
    correlation_id: CorrelationId,
}

impl ProviderCompletedObjectV1 {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        size: ByteSize,
        checksum_sha256: ChecksumSha256,
        content_type: ContentType,
        provider_version: ProviderObjectVersion,
        provider_etag: ProviderEntityTag,
        last_modified: TimestampMillis,
        media_probe: TrustedMediaProbeV1,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            key,
            size,
            checksum_sha256,
            content_type,
            provider_version,
            provider_etag,
            last_modified,
            media_probe,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
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
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
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
    pub const fn last_modified(&self) -> TimestampMillis {
        self.last_modified
    }

    #[must_use]
    pub const fn media_probe(&self) -> &TrustedMediaProbeV1 {
        &self.media_probe
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAbortDispositionV1 {
    Aborted,
    AlreadyAborted,
    AlreadyCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAbortReceiptV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    disposition: ProviderAbortDispositionV1,
    correlation_id: CorrelationId,
}

impl ProviderAbortReceiptV1 {
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        disposition: ProviderAbortDispositionV1,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            key,
            disposition,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn disposition(&self) -> ProviderAbortDispositionV1 {
        self.disposition
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadValidatorV1 {
    None,
    IfMatch(ProviderEntityTag),
    IfNoneMatch(ProviderEntityTag),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDownloadRequestV1 {
    key: ScopedObjectKey,
    range: Option<ObjectByteRange>,
    validator: DownloadValidatorV1,
    correlation_id: CorrelationId,
}

impl ProviderDownloadRequestV1 {
    #[must_use]
    pub const fn new(
        key: ScopedObjectKey,
        range: Option<ObjectByteRange>,
        validator: DownloadValidatorV1,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            key,
            range,
            validator,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
    }

    #[must_use]
    pub const fn range(&self) -> Option<ObjectByteRange> {
        self.range
    }

    #[must_use]
    pub const fn validator(&self) -> &DownloadValidatorV1 {
        &self.validator
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDownloadMetadataV1 {
    key: ScopedObjectKey,
    size: ByteSize,
    checksum_sha256: ChecksumSha256,
    content_type: ContentType,
    provider_version: ProviderObjectVersion,
    provider_etag: ProviderEntityTag,
    last_modified: TimestampMillis,
    correlation_id: CorrelationId,
}

impl ProviderDownloadMetadataV1 {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        key: ScopedObjectKey,
        size: ByteSize,
        checksum_sha256: ChecksumSha256,
        content_type: ContentType,
        provider_version: ProviderObjectVersion,
        provider_etag: ProviderEntityTag,
        last_modified: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            key,
            size,
            checksum_sha256,
            content_type,
            provider_version,
            provider_etag,
            last_modified,
            correlation_id,
        }
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
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
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
    pub const fn last_modified(&self) -> TimestampMillis {
        self.last_modified
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[async_trait]
pub trait ProviderDownloadBodyV1: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, StorageFailure>;
}

pub enum ProviderDownloadResponseV1 {
    NotModified(ProviderDownloadMetadataV1),
    Head(ProviderDownloadMetadataV1),
    Body {
        metadata: ProviderDownloadMetadataV1,
        range: ObjectByteRange,
        body: Box<dyn ProviderDownloadBodyV1>,
    },
}

impl ProviderDownloadResponseV1 {
    #[must_use]
    pub const fn metadata(&self) -> &ProviderDownloadMetadataV1 {
        match self {
            Self::NotModified(metadata) | Self::Head(metadata) | Self::Body { metadata, .. } => {
                metadata
            }
        }
    }

    #[must_use]
    pub const fn range(&self) -> Option<ObjectByteRange> {
        match self {
            Self::Body { range, .. } => Some(*range),
            Self::NotModified(_) | Self::Head(_) => None,
        }
    }
}

impl fmt::Debug for ProviderDownloadResponseV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotModified(metadata) => formatter
                .debug_tuple("NotModified")
                .field(metadata)
                .finish(),
            Self::Head(metadata) => formatter.debug_tuple("Head").field(metadata).finish(),
            Self::Body {
                metadata,
                range,
                body: _,
            } => formatter
                .debug_struct("Body")
                .field("metadata", metadata)
                .field("range", range)
                .finish_non_exhaustive(),
        }
    }
}

#[async_trait]
pub trait MultipartObjectStoreV1: Send + Sync {
    fn capabilities(&self) -> MultipartProviderCapabilitiesV1;

    async fn create_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCreateMultipartRequestV1,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure>;

    async fn lookup_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderLookupMultipartRequestV1,
    ) -> Result<Option<ProviderMultipartSessionV1>, StorageFailure>;

    async fn list_parts(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderPartsListV1, StorageFailure>;

    async fn put_part(
        &self,
        context: StorageRequestContext,
        request: ProviderPutPartRequestV1,
    ) -> Result<ProviderPartReceiptV1, StorageFailure>;

    async fn complete_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCompleteMultipartRequestV1,
    ) -> Result<ProviderCompletedObjectV1, StorageFailure>;

    async fn abort_multipart(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure>;

    async fn head_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure>;

    async fn get_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultipartJournalPhaseV1 {
    Creating,
    Uploading,
    ProviderCompleted,
    Finalized,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartUploadSnapshotV1 {
    upload_id: MultipartUploadId,
    spec: MultipartUploadSpecV1,
    provider_session: Option<ProviderMultipartSessionV1>,
    parts: Vec<ProviderPartReceiptV1>,
    phase: MultipartJournalPhaseV1,
    completed: Option<ProviderCompletedObjectV1>,
    expires_at: TimestampMillis,
    correlation_id: CorrelationId,
}

impl MultipartUploadSnapshotV1 {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        spec: MultipartUploadSpecV1,
        provider_session: Option<ProviderMultipartSessionV1>,
        parts: Vec<ProviderPartReceiptV1>,
        phase: MultipartJournalPhaseV1,
        completed: Option<ProviderCompletedObjectV1>,
        expires_at: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            spec,
            provider_session,
            parts,
            phase,
            completed,
            expires_at,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn spec(&self) -> &MultipartUploadSpecV1 {
        &self.spec
    }

    #[must_use]
    pub const fn provider_session(&self) -> Option<&ProviderMultipartSessionV1> {
        self.provider_session.as_ref()
    }

    #[must_use]
    pub fn parts(&self) -> &[ProviderPartReceiptV1] {
        &self.parts
    }

    #[must_use]
    pub const fn phase(&self) -> MultipartJournalPhaseV1 {
        self.phase
    }

    #[must_use]
    pub const fn completed(&self) -> Option<&ProviderCompletedObjectV1> {
        self.completed.as_ref()
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFinalizeRecordV1 {
    upload_id: MultipartUploadId,
    key: ScopedObjectKey,
    provider_version: ProviderObjectVersion,
    provider_etag: ProviderEntityTag,
    size: ByteSize,
    checksum_sha256: ChecksumSha256,
    content_type: ContentType,
    provider_last_modified: TimestampMillis,
    media_probe: TrustedMediaProbeV1,
    finalized_at: TimestampMillis,
    correlation_id: CorrelationId,
}

impl SourceFinalizeRecordV1 {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        upload_id: MultipartUploadId,
        key: ScopedObjectKey,
        provider_version: ProviderObjectVersion,
        provider_etag: ProviderEntityTag,
        size: ByteSize,
        checksum_sha256: ChecksumSha256,
        content_type: ContentType,
        provider_last_modified: TimestampMillis,
        media_probe: TrustedMediaProbeV1,
        finalized_at: TimestampMillis,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            upload_id,
            key,
            provider_version,
            provider_etag,
            size,
            checksum_sha256,
            content_type,
            provider_last_modified,
            media_probe,
            finalized_at,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn upload_id(&self) -> MultipartUploadId {
        self.upload_id
    }

    #[must_use]
    pub const fn key(&self) -> &ScopedObjectKey {
        &self.key
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
    pub const fn size(&self) -> ByteSize {
        self.size
    }

    #[must_use]
    pub const fn checksum_sha256(&self) -> &ChecksumSha256 {
        &self.checksum_sha256
    }

    #[must_use]
    pub const fn content_type(&self) -> &ContentType {
        &self.content_type
    }

    #[must_use]
    pub const fn provider_last_modified(&self) -> TimestampMillis {
        self.provider_last_modified
    }

    #[must_use]
    pub const fn media_probe(&self) -> &TrustedMediaProbeV1 {
        &self.media_probe
    }

    #[must_use]
    pub const fn finalized_at(&self) -> TimestampMillis {
        self.finalized_at
    }

    #[must_use]
    pub const fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalCreateOutcomeV1 {
    Claimed(MultipartUploadSnapshotV1),
    Resume(MultipartUploadSnapshotV1),
    Replay(MultipartUploadSnapshotV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalMutationOutcomeV1<T> {
    Applied(T),
    Replay(T),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MultipartSystemReplayOperationV1 {
    Complete,
    Finalize,
    Abort,
}

/// A structural replay namespace. Client commands can supply only the `Client` value through
/// their idempotency-key DTO; `System` values are constructed by reconciliation code and are
/// deliberately not serializable.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum MultipartReplayKeyV1 {
    Client(IdempotencyKey),
    System {
        operation: MultipartSystemReplayOperationV1,
        upload_id: MultipartUploadId,
    },
}

impl MultipartReplayKeyV1 {
    #[must_use]
    pub const fn client(key: IdempotencyKey) -> Self {
        Self::Client(key)
    }

    #[must_use]
    pub const fn reconciliation(
        operation: MultipartSystemReplayOperationV1,
        upload_id: MultipartUploadId,
    ) -> Self {
        Self::System {
            operation,
            upload_id,
        }
    }
}

impl fmt::Debug for MultipartReplayKeyV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(_) => formatter.write_str("MultipartReplayKeyV1::Client([redacted])"),
            Self::System {
                operation,
                upload_id,
            } => formatter
                .debug_struct("MultipartReplayKeyV1::System")
                .field("operation", operation)
                .field("upload_id", upload_id)
                .finish(),
        }
    }
}

#[async_trait]
pub trait MultipartJournalV1: Send + Sync {
    async fn register_grant(
        &self,
        context: StorageRequestContext,
        record: MultipartGrantRecordV1,
    ) -> Result<JournalMutationOutcomeV1<MultipartGrantRecordV1>, StorageFailure>;

    async fn get_grant(
        &self,
        context: StorageRequestContext,
        id: MultipartGrantId,
    ) -> Result<Option<MultipartGrantRecordV1>, StorageFailure>;

    async fn revoke_grant(
        &self,
        context: StorageRequestContext,
        id: MultipartGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<(), StorageFailure>;

    async fn claim_create(
        &self,
        context: StorageRequestContext,
        grant_id: MultipartGrantId,
        now: TimestampMillis,
        idempotency_key: IdempotencyKey,
        fingerprint: ChecksumSha256,
        draft: MultipartUploadSnapshotV1,
    ) -> Result<JournalCreateOutcomeV1, StorageFailure>;

    async fn activate_upload(
        &self,
        context: StorageRequestContext,
        session: ProviderMultipartSessionV1,
    ) -> Result<MultipartUploadSnapshotV1, StorageFailure>;

    async fn get_upload(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
    ) -> Result<Option<MultipartUploadSnapshotV1>, StorageFailure>;

    async fn get_finalize(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure>;

    async fn get_finalize_by_key(
        &self,
        context: StorageRequestContext,
        key: ScopedObjectKey,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure>;

    async fn record_part(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        receipt: ProviderPartReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderPartReceiptV1>, StorageFailure>;

    async fn record_provider_complete(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        completed: ProviderCompletedObjectV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderCompletedObjectV1>, StorageFailure>;

    async fn finalize(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        record: SourceFinalizeRecordV1,
    ) -> Result<JournalMutationOutcomeV1<SourceFinalizeRecordV1>, StorageFailure>;

    async fn abort(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        receipt: ProviderAbortReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderAbortReceiptV1>, StorageFailure>;

    async fn reconciliation_candidates(
        &self,
        context: StorageRequestContext,
        limit: u16,
    ) -> Result<Vec<MultipartUploadSnapshotV1>, StorageFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadPolicyV1 {
    allowed_origins: Vec<CorsOriginV1>,
    cache_policy: ObjectCachePolicy,
}

impl DownloadPolicyV1 {
    pub fn new(
        mut allowed_origins: Vec<CorsOriginV1>,
        cache_policy: ObjectCachePolicy,
    ) -> Result<Self, StorageFailure> {
        allowed_origins.sort();
        allowed_origins.dedup();
        if allowed_origins.len() > 32 {
            return Err(invalid());
        }
        Ok(Self {
            allowed_origins,
            cache_policy,
        })
    }

    #[must_use]
    pub fn allows(&self, origin: &CorsOriginV1) -> bool {
        self.allowed_origins.binary_search(origin).is_ok()
    }

    #[must_use]
    pub fn allowed_origins(&self) -> &[CorsOriginV1] {
        &self.allowed_origins
    }

    #[must_use]
    pub const fn cache_policy(&self) -> ObjectCachePolicy {
        self.cache_policy
    }
}

fn invalid() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::InvalidRequest)
}

fn not_found() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::NotFound)
}

fn conflict() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::PreconditionFailed)
}

fn integrity() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::Integrity)
}

fn unavailable() -> StorageFailure {
    StorageFailure::new(StorageFailureKind::Unavailable)
}

#[derive(Clone)]
enum FakeProviderPhase {
    Uploading,
    Completed {
        completed: Box<ProviderCompletedObjectV1>,
    },
    Aborted,
}

#[derive(Clone)]
struct FakeProviderPart {
    receipt: ProviderPartReceiptV1,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct FakeProviderUpload {
    spec: MultipartUploadSpecV1,
    expires_at: TimestampMillis,
    handle: ProviderMultipartHandleV1,
    parts: BTreeMap<u16, FakeProviderPart>,
    phase: FakeProviderPhase,
}

struct FakeDownloadBodyV1 {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl ProviderDownloadBodyV1 for FakeDownloadBodyV1 {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, StorageFailure> {
        Ok(self.chunks.pop_front())
    }
}

/// A deterministic, provider-free multipart adapter with real SHA-256 checks.
pub struct DeterministicMultipartObjectStore {
    capabilities: MultipartProviderCapabilitiesV1,
    media_probe: TrustedMediaProbeV1,
    uploads: Mutex<HashMap<MultipartUploadId, FakeProviderUpload>>,
    objects: RwLock<HashMap<String, (ProviderCompletedObjectV1, Vec<u8>)>>,
    failures: Mutex<BTreeMap<MultipartProviderOperationV1, VecDeque<StorageFailure>>>,
}

impl DeterministicMultipartObjectStore {
    #[must_use]
    pub fn new(
        capabilities: MultipartProviderCapabilitiesV1,
        media_probe: TrustedMediaProbeV1,
    ) -> Self {
        Self {
            capabilities,
            media_probe,
            uploads: Mutex::new(HashMap::new()),
            objects: RwLock::new(HashMap::new()),
            failures: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn inject_failure(
        &self,
        operation: MultipartProviderOperationV1,
        failure: StorageFailure,
    ) -> Result<(), StorageFailure> {
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

    pub fn active_upload_count(&self) -> Result<usize, StorageFailure> {
        Ok(self
            .uploads
            .lock()
            .map_err(|_| unavailable())?
            .values()
            .filter(|upload| matches!(upload.phase, FakeProviderPhase::Uploading))
            .count())
    }

    fn guard(&self, operation: MultipartProviderOperationV1) -> Result<(), StorageFailure> {
        self.capabilities.require(operation)?;
        if let Some(failure) = self
            .failures
            .lock()
            .map_err(|_| unavailable())?
            .get_mut(&operation)
            .and_then(VecDeque::pop_front)
        {
            return Err(failure);
        }
        Ok(())
    }

    fn authorize(
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<(), StorageFailure> {
        if context.tenant_id() == key.tenant_id() {
            Ok(())
        } else {
            Err(not_found())
        }
    }

    fn validate_reference(
        context: StorageRequestContext,
        reference: &ProviderUploadReferenceV1,
        upload: &FakeProviderUpload,
    ) -> Result<(), StorageFailure> {
        Self::authorize(context, reference.key())?;
        if reference.key() != upload.spec.key()
            || reference.handle() != &upload.handle
            || reference.correlation_id() != context.correlation_id()
        {
            return Err(not_found());
        }
        Ok(())
    }

    fn rebind_part(
        part: &ProviderPartReceiptV1,
        correlation_id: CorrelationId,
    ) -> ProviderPartReceiptV1 {
        ProviderPartReceiptV1::new(
            part.upload_id(),
            part.key().clone(),
            part.part_number(),
            part.size(),
            part.checksum_sha256().clone(),
            part.etag().clone(),
            correlation_id,
        )
    }

    fn rebind_completed(
        completed: &ProviderCompletedObjectV1,
        correlation_id: CorrelationId,
    ) -> ProviderCompletedObjectV1 {
        ProviderCompletedObjectV1::new(
            completed.upload_id(),
            completed.key().clone(),
            completed.size(),
            completed.checksum_sha256().clone(),
            completed.content_type().clone(),
            completed.provider_version().clone(),
            completed.provider_etag().clone(),
            completed.last_modified(),
            completed.media_probe().clone(),
            correlation_id,
        )
    }

    fn download_metadata(
        completed: &ProviderCompletedObjectV1,
        correlation_id: CorrelationId,
    ) -> ProviderDownloadMetadataV1 {
        ProviderDownloadMetadataV1::new(
            completed.key().clone(),
            completed.size(),
            completed.checksum_sha256().clone(),
            completed.content_type().clone(),
            completed.provider_version().clone(),
            completed.provider_etag().clone(),
            completed.last_modified(),
            correlation_id,
        )
    }

    fn condition(
        validator: &DownloadValidatorV1,
        metadata: ProviderDownloadMetadataV1,
    ) -> Result<Option<ProviderDownloadResponseV1>, StorageFailure> {
        match validator {
            DownloadValidatorV1::None => Ok(None),
            DownloadValidatorV1::IfMatch(expected) if expected != metadata.provider_etag() => {
                Err(StorageFailure::new(StorageFailureKind::PreconditionFailed))
            }
            DownloadValidatorV1::IfNoneMatch(expected) if expected == metadata.provider_etag() => {
                Ok(Some(ProviderDownloadResponseV1::NotModified(metadata)))
            }
            DownloadValidatorV1::IfMatch(_) | DownloadValidatorV1::IfNoneMatch(_) => Ok(None),
        }
    }

    fn find_object(
        &self,
        context: StorageRequestContext,
        key: &ScopedObjectKey,
    ) -> Result<(ProviderCompletedObjectV1, Vec<u8>), StorageFailure> {
        Self::authorize(context, key)?;
        self.objects
            .read()
            .map_err(|_| unavailable())?
            .get(key.as_str())
            .cloned()
            .ok_or_else(not_found)
    }
}

impl fmt::Debug for DeterministicMultipartObjectStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeterministicMultipartObjectStore")
            .field("capabilities", &self.capabilities)
            .field("object_count", &self.object_count().unwrap_or_default())
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl MultipartObjectStoreV1 for DeterministicMultipartObjectStore {
    fn capabilities(&self) -> MultipartProviderCapabilitiesV1 {
        self.capabilities
    }

    async fn create_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCreateMultipartRequestV1,
    ) -> Result<ProviderMultipartSessionV1, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.correlation_id() != context.correlation_id() {
            return Err(not_found());
        }
        let capabilities = self.capabilities;
        let spec = request.spec();
        if !capabilities.checksum_sha256() {
            return Err(StorageFailure::unsupported());
        }
        if spec.part_size() < capabilities.min_part_size()
            || spec.part_size() > capabilities.max_part_size()
            || spec.part_count() > capabilities.max_part_count()
            || spec.total_size() > capabilities.max_total_size()
        {
            return Err(StorageFailure::new(StorageFailureKind::QuotaExceeded));
        }
        self.guard(MultipartProviderOperationV1::Create)?;
        let mut uploads = self.uploads.lock().map_err(|_| unavailable())?;
        if let Some(existing) = uploads.get(&request.upload_id()) {
            if existing.spec != *request.spec() || existing.expires_at != request.expires_at() {
                return Err(conflict());
            }
            if matches!(existing.phase, FakeProviderPhase::Aborted) {
                return Err(conflict());
            }
            return Ok(ProviderMultipartSessionV1::new(
                request.upload_id(),
                request.key().clone(),
                existing.handle.clone(),
                existing.expires_at,
                context.correlation_id(),
            ));
        }
        let handle = ProviderMultipartHandleV1::parse(format!(
            "fake-provider-upload-{}",
            request.upload_id()
        ))?;
        uploads.insert(
            request.upload_id(),
            FakeProviderUpload {
                spec: request.spec().clone(),
                expires_at: request.expires_at(),
                handle: handle.clone(),
                parts: BTreeMap::new(),
                phase: FakeProviderPhase::Uploading,
            },
        );
        Ok(ProviderMultipartSessionV1::new(
            request.upload_id(),
            request.key().clone(),
            handle,
            request.expires_at(),
            context.correlation_id(),
        ))
    }

    async fn lookup_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderLookupMultipartRequestV1,
    ) -> Result<Option<ProviderMultipartSessionV1>, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.correlation_id() != context.correlation_id() {
            return Err(not_found());
        }
        self.guard(MultipartProviderOperationV1::Lookup)?;
        let uploads = self.uploads.lock().map_err(|_| unavailable())?;
        let Some(upload) = uploads.get(&request.upload_id()) else {
            return Ok(None);
        };
        if upload.spec.key() != request.key() {
            return Err(not_found());
        }
        if !matches!(upload.phase, FakeProviderPhase::Uploading) {
            return Ok(None);
        }
        Ok(Some(ProviderMultipartSessionV1::new(
            request.upload_id(),
            request.key().clone(),
            upload.handle.clone(),
            upload.expires_at,
            context.correlation_id(),
        )))
    }

    async fn list_parts(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderPartsListV1, StorageFailure> {
        Self::authorize(context, reference.key())?;
        let uploads = self.uploads.lock().map_err(|_| unavailable())?;
        let upload = uploads.get(&reference.upload_id()).ok_or_else(not_found)?;
        Self::validate_reference(context, &reference, upload)?;
        if matches!(upload.phase, FakeProviderPhase::Aborted) {
            return Err(not_found());
        }
        self.guard(MultipartProviderOperationV1::ListParts)?;
        let parts = upload
            .parts
            .values()
            .map(|part| Self::rebind_part(&part.receipt, context.correlation_id()))
            .collect();
        ProviderPartsListV1::new(
            reference.upload_id(),
            reference.key().clone(),
            parts,
            context.correlation_id(),
        )
    }

    async fn put_part(
        &self,
        context: StorageRequestContext,
        request: ProviderPutPartRequestV1,
    ) -> Result<ProviderPartReceiptV1, StorageFailure> {
        Self::authorize(context, request.reference().key())?;
        let mut uploads = self.uploads.lock().map_err(|_| unavailable())?;
        let upload = uploads
            .get_mut(&request.reference().upload_id())
            .ok_or_else(not_found)?;
        Self::validate_reference(context, request.reference(), upload)?;
        if !matches!(upload.phase, FakeProviderPhase::Uploading) {
            return Err(conflict());
        }
        self.guard(MultipartProviderOperationV1::PutPart)?;
        if !self.capabilities.checksum_sha256() {
            return Err(StorageFailure::unsupported());
        }
        if request.checksum_sha256() != &ChecksumSha256::digest_bytes(request.bytes()) {
            return Err(integrity());
        }
        let size = ByteSize::new(u64::try_from(request.bytes().len()).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
        upload
            .spec
            .validate_part(request.part_number(), size)
            .map_err(|_| invalid())?;
        if let Some(existing) = upload.parts.get(&request.part_number().get()) {
            if existing.bytes != request.bytes()
                || existing.receipt.checksum_sha256() != request.checksum_sha256()
            {
                return Err(conflict());
            }
            return Ok(Self::rebind_part(
                &existing.receipt,
                context.correlation_id(),
            ));
        }
        let etag = ProviderEntityTag::parse(format!(
            "part-{:05}-{}",
            request.part_number().get(),
            request.checksum_sha256().as_str()
        ))?;
        let receipt = ProviderPartReceiptV1::new(
            request.reference().upload_id(),
            request.reference().key().clone(),
            request.part_number(),
            size,
            request.checksum_sha256().clone(),
            etag,
            context.correlation_id(),
        );
        upload.parts.insert(
            request.part_number().get(),
            FakeProviderPart {
                receipt: receipt.clone(),
                bytes: request.bytes().to_vec(),
            },
        );
        Ok(receipt)
    }

    async fn complete_multipart(
        &self,
        context: StorageRequestContext,
        request: ProviderCompleteMultipartRequestV1,
    ) -> Result<ProviderCompletedObjectV1, StorageFailure> {
        Self::authorize(context, request.reference().key())?;
        let mut uploads = self.uploads.lock().map_err(|_| unavailable())?;
        let upload = uploads
            .get_mut(&request.reference().upload_id())
            .ok_or_else(not_found)?;
        Self::validate_reference(context, request.reference(), upload)?;
        if let FakeProviderPhase::Completed { completed, .. } = &upload.phase {
            return Ok(Self::rebind_completed(completed, context.correlation_id()));
        }
        if matches!(upload.phase, FakeProviderPhase::Aborted) {
            return Err(conflict());
        }
        if !self.capabilities.checksum_sha256() {
            return Err(StorageFailure::unsupported());
        }
        self.guard(MultipartProviderOperationV1::Complete)?;
        if request.parts().len() != usize::from(upload.spec.part_count())
            || request.expected_size() != upload.spec.total_size()
            || request.expected_checksum_sha256() != upload.spec.checksum_sha256()
            || request.expected_content_type() != upload.spec.content_type()
        {
            return Err(integrity());
        }
        let mut bytes = Vec::new();
        for expected_number in 1..=upload.spec.part_count() {
            let part = upload.parts.get(&expected_number).ok_or_else(conflict)?;
            let supplied = &request.parts()[usize::from(expected_number - 1)];
            if supplied.part_number().get() != expected_number
                || supplied.size() != part.receipt.size()
                || supplied.checksum_sha256() != part.receipt.checksum_sha256()
                || supplied.etag() != part.receipt.etag()
            {
                return Err(integrity());
            }
            bytes.extend_from_slice(&part.bytes);
        }
        if ByteSize::new(u64::try_from(bytes.len()).map_err(|_| invalid())?)
            .map_err(|_| invalid())?
            != upload.spec.total_size()
            || ChecksumSha256::digest_bytes(&bytes) != *upload.spec.checksum_sha256()
        {
            return Err(integrity());
        }
        let provider_version = ProviderObjectVersion::parse(format!(
            "multipart-v1-{}",
            request.reference().upload_id()
        ))?;
        let provider_etag =
            ProviderEntityTag::parse(format!("sha256-{}", upload.spec.checksum_sha256().as_str()))?;
        let completed = ProviderCompletedObjectV1::new(
            request.reference().upload_id(),
            upload.spec.key().clone(),
            upload.spec.total_size(),
            upload.spec.checksum_sha256().clone(),
            upload.spec.content_type().clone(),
            provider_version,
            provider_etag,
            TimestampMillis::new(10).map_err(|_| unavailable())?,
            self.media_probe.clone(),
            context.correlation_id(),
        );
        if self
            .objects
            .read()
            .map_err(|_| unavailable())?
            .contains_key(upload.spec.key().as_str())
        {
            return Err(conflict());
        }
        self.objects.write().map_err(|_| unavailable())?.insert(
            upload.spec.key().as_str().to_owned(),
            (completed.clone(), bytes.clone()),
        );
        upload.phase = FakeProviderPhase::Completed {
            completed: Box::new(completed.clone()),
        };
        Ok(completed)
    }

    async fn abort_multipart(
        &self,
        context: StorageRequestContext,
        reference: ProviderUploadReferenceV1,
    ) -> Result<ProviderAbortReceiptV1, StorageFailure> {
        Self::authorize(context, reference.key())?;
        let mut uploads = self.uploads.lock().map_err(|_| unavailable())?;
        let upload = uploads
            .get_mut(&reference.upload_id())
            .ok_or_else(not_found)?;
        Self::validate_reference(context, &reference, upload)?;
        self.guard(MultipartProviderOperationV1::Abort)?;
        let disposition = match upload.phase {
            FakeProviderPhase::Uploading => {
                upload.parts.clear();
                upload.phase = FakeProviderPhase::Aborted;
                ProviderAbortDispositionV1::Aborted
            }
            FakeProviderPhase::Aborted => ProviderAbortDispositionV1::AlreadyAborted,
            FakeProviderPhase::Completed { .. } => ProviderAbortDispositionV1::AlreadyCompleted,
        };
        Ok(ProviderAbortReceiptV1::new(
            reference.upload_id(),
            reference.key().clone(),
            disposition,
            context.correlation_id(),
        ))
    }

    async fn head_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.range().is_some() || request.correlation_id() != context.correlation_id() {
            return Err(invalid());
        }
        let (completed, _) = self.find_object(context, request.key())?;
        self.guard(MultipartProviderOperationV1::Head)?;
        let metadata = Self::download_metadata(&completed, context.correlation_id());
        if let Some(condition) = Self::condition(request.validator(), metadata.clone())? {
            return Ok(condition);
        }
        Ok(ProviderDownloadResponseV1::Head(metadata))
    }

    async fn get_private(
        &self,
        context: StorageRequestContext,
        request: ProviderDownloadRequestV1,
    ) -> Result<ProviderDownloadResponseV1, StorageFailure> {
        Self::authorize(context, request.key())?;
        if request.correlation_id() != context.correlation_id() {
            return Err(invalid());
        }
        let (completed, bytes) = self.find_object(context, request.key())?;
        self.guard(MultipartProviderOperationV1::Get)?;
        let metadata = Self::download_metadata(&completed, context.correlation_id());
        if let Some(condition) = Self::condition(request.validator(), metadata.clone())? {
            return Ok(condition);
        }
        let full_end = completed.size().get();
        let range = request
            .range()
            .unwrap_or(ObjectByteRange::new(0, full_end)?);
        let range_size = ByteSize::new(
            range
                .end_exclusive()
                .checked_sub(range.start())
                .ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
        if range.end_exclusive() > full_end
            || (request.range().is_some() && range_size > self.capabilities.max_range_size())
        {
            return Err(invalid());
        }
        let start = usize::try_from(range.start()).map_err(|_| invalid())?;
        let end = usize::try_from(range.end_exclusive()).map_err(|_| invalid())?;
        let chunk_size = usize::try_from(self.capabilities.max_range_size().get())
            .unwrap_or(usize::MAX)
            .clamp(1, 8);
        let chunks = bytes[start..end]
            .chunks(chunk_size)
            .map(<[u8]>::to_vec)
            .collect();
        Ok(ProviderDownloadResponseV1::Body {
            metadata,
            range,
            body: Box::new(FakeDownloadBodyV1 { chunks }),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MultipartJournalOperationV1 {
    RegisterGrant,
    GetGrant,
    RevokeGrant,
    ClaimCreate,
    Activate,
    GetUpload,
    RecordPart,
    RecordComplete,
    Finalize,
    Abort,
    Reconcile,
}

fn same_part_identity(left: &ProviderPartReceiptV1, right: &ProviderPartReceiptV1) -> bool {
    left.upload_id() == right.upload_id()
        && left.key() == right.key()
        && left.part_number() == right.part_number()
        && left.size() == right.size()
        && left.checksum_sha256() == right.checksum_sha256()
        && left.etag() == right.etag()
}

fn rebind_part_receipt(
    receipt: &ProviderPartReceiptV1,
    correlation_id: CorrelationId,
) -> ProviderPartReceiptV1 {
    ProviderPartReceiptV1::new(
        receipt.upload_id(),
        receipt.key().clone(),
        receipt.part_number(),
        receipt.size(),
        receipt.checksum_sha256().clone(),
        receipt.etag().clone(),
        correlation_id,
    )
}

fn same_completed_identity(
    left: &ProviderCompletedObjectV1,
    right: &ProviderCompletedObjectV1,
) -> bool {
    left.upload_id() == right.upload_id()
        && left.key() == right.key()
        && left.size() == right.size()
        && left.checksum_sha256() == right.checksum_sha256()
        && left.content_type() == right.content_type()
        && left.provider_version() == right.provider_version()
        && left.provider_etag() == right.provider_etag()
        && left.last_modified() == right.last_modified()
        && left.media_probe() == right.media_probe()
}

fn rebind_completed_object(
    completed: &ProviderCompletedObjectV1,
    correlation_id: CorrelationId,
) -> ProviderCompletedObjectV1 {
    ProviderCompletedObjectV1::new(
        completed.upload_id(),
        completed.key().clone(),
        completed.size(),
        completed.checksum_sha256().clone(),
        completed.content_type().clone(),
        completed.provider_version().clone(),
        completed.provider_etag().clone(),
        completed.last_modified(),
        completed.media_probe().clone(),
        correlation_id,
    )
}

fn same_finalize_identity(left: &SourceFinalizeRecordV1, right: &SourceFinalizeRecordV1) -> bool {
    left.upload_id() == right.upload_id()
        && left.key() == right.key()
        && left.provider_version() == right.provider_version()
        && left.provider_etag() == right.provider_etag()
        && left.size() == right.size()
        && left.checksum_sha256() == right.checksum_sha256()
        && left.content_type() == right.content_type()
        && left.provider_last_modified() == right.provider_last_modified()
        && left.media_probe() == right.media_probe()
}

fn rebind_finalize_record(
    record: &SourceFinalizeRecordV1,
    correlation_id: CorrelationId,
) -> SourceFinalizeRecordV1 {
    SourceFinalizeRecordV1::new(
        record.upload_id(),
        record.key().clone(),
        record.provider_version().clone(),
        record.provider_etag().clone(),
        record.size(),
        record.checksum_sha256().clone(),
        record.content_type().clone(),
        record.provider_last_modified(),
        record.media_probe().clone(),
        record.finalized_at(),
        correlation_id,
    )
}

fn same_abort_identity(left: &ProviderAbortReceiptV1, right: &ProviderAbortReceiptV1) -> bool {
    let same_terminal_disposition = left.disposition() == right.disposition()
        || matches!(
            (left.disposition(), right.disposition()),
            (
                ProviderAbortDispositionV1::Aborted,
                ProviderAbortDispositionV1::AlreadyAborted
            ) | (
                ProviderAbortDispositionV1::AlreadyAborted,
                ProviderAbortDispositionV1::Aborted
            )
        );
    left.upload_id() == right.upload_id() && left.key() == right.key() && same_terminal_disposition
}

fn rebind_abort_receipt(
    receipt: &ProviderAbortReceiptV1,
    correlation_id: CorrelationId,
) -> ProviderAbortReceiptV1 {
    ProviderAbortReceiptV1::new(
        receipt.upload_id(),
        receipt.key().clone(),
        receipt.disposition(),
        correlation_id,
    )
}

#[derive(Clone)]
enum JournalReplayV1 {
    Create {
        fingerprint: ChecksumSha256,
        upload_id: MultipartUploadId,
    },
    Part {
        fingerprint: ChecksumSha256,
        receipt: ProviderPartReceiptV1,
    },
    Complete {
        fingerprint: ChecksumSha256,
        completed: ProviderCompletedObjectV1,
    },
    Finalize {
        fingerprint: ChecksumSha256,
        record: SourceFinalizeRecordV1,
    },
    Abort {
        fingerprint: ChecksumSha256,
        receipt: ProviderAbortReceiptV1,
    },
}

#[derive(Clone)]
struct JournalUploadV1 {
    snapshot: MultipartUploadSnapshotV1,
    finalized: Option<SourceFinalizeRecordV1>,
}

#[derive(Clone)]
struct JournalCreateGrantClaimV1 {
    tenant_id: TenantId,
    idempotency_key: IdempotencyKey,
    fingerprint: ChecksumSha256,
    upload_id: MultipartUploadId,
}

#[derive(Default)]
struct JournalStateV1 {
    grants: HashMap<MultipartGrantId, MultipartGrantRecordV1>,
    create_grants: HashMap<MultipartGrantId, JournalCreateGrantClaimV1>,
    uploads: HashMap<MultipartUploadId, JournalUploadV1>,
    replays: HashMap<(TenantId, MultipartReplayKeyV1), JournalReplayV1>,
}

/// Linearizable in-memory journal used to prove restart, replay, and race semantics.
pub struct DeterministicMultipartJournal {
    state: Mutex<JournalStateV1>,
    failures: Mutex<BTreeMap<MultipartJournalOperationV1, VecDeque<StorageFailure>>>,
}

impl Default for DeterministicMultipartJournal {
    fn default() -> Self {
        Self {
            state: Mutex::new(JournalStateV1::default()),
            failures: Mutex::new(BTreeMap::new()),
        }
    }
}

impl DeterministicMultipartJournal {
    pub fn inject_failure(
        &self,
        operation: MultipartJournalOperationV1,
        failure: StorageFailure,
    ) -> Result<(), StorageFailure> {
        self.failures
            .lock()
            .map_err(|_| unavailable())?
            .entry(operation)
            .or_default()
            .push_back(failure);
        Ok(())
    }

    fn guard(&self, operation: MultipartJournalOperationV1) -> Result<(), StorageFailure> {
        if let Some(failure) = self
            .failures
            .lock()
            .map_err(|_| unavailable())?
            .get_mut(&operation)
            .and_then(VecDeque::pop_front)
        {
            return Err(failure);
        }
        Ok(())
    }

    fn replay_key(
        context: StorageRequestContext,
        key: MultipartReplayKeyV1,
    ) -> (TenantId, MultipartReplayKeyV1) {
        (context.tenant_id(), key)
    }

    fn authorize_snapshot(
        context: StorageRequestContext,
        snapshot: &MultipartUploadSnapshotV1,
    ) -> Result<(), StorageFailure> {
        if snapshot.spec().key().tenant_id() == context.tenant_id() {
            Ok(())
        } else {
            Err(not_found())
        }
    }

    fn snapshot_with(
        snapshot: &MultipartUploadSnapshotV1,
        provider_session: Option<ProviderMultipartSessionV1>,
        parts: Vec<ProviderPartReceiptV1>,
        phase: MultipartJournalPhaseV1,
        completed: Option<ProviderCompletedObjectV1>,
    ) -> MultipartUploadSnapshotV1 {
        MultipartUploadSnapshotV1::new(
            snapshot.upload_id(),
            snapshot.spec().clone(),
            provider_session,
            parts,
            phase,
            completed,
            snapshot.expires_at(),
            snapshot.correlation_id(),
        )
    }
}

impl fmt::Debug for DeterministicMultipartJournal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (grant_count, upload_count) = self
            .state
            .lock()
            .map_or((0, 0), |state| (state.grants.len(), state.uploads.len()));
        formatter
            .debug_struct("DeterministicMultipartJournal")
            .field("grant_count", &grant_count)
            .field("upload_count", &upload_count)
            .finish()
    }
}

#[async_trait]
impl MultipartJournalV1 for DeterministicMultipartJournal {
    async fn register_grant(
        &self,
        context: StorageRequestContext,
        record: MultipartGrantRecordV1,
    ) -> Result<JournalMutationOutcomeV1<MultipartGrantRecordV1>, StorageFailure> {
        if record.scope().tenant_id() != context.tenant_id()
            || record.scope().key().tenant_id() != context.tenant_id()
        {
            return Err(not_found());
        }
        self.guard(MultipartJournalOperationV1::RegisterGrant)?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if let Some(previous) = state.grants.get(&record.id()) {
            return if previous == &record {
                Ok(JournalMutationOutcomeV1::Replay(previous.clone()))
            } else {
                Err(conflict())
            };
        }
        state.grants.insert(record.id(), record.clone());
        Ok(JournalMutationOutcomeV1::Applied(record))
    }

    async fn get_grant(
        &self,
        context: StorageRequestContext,
        id: MultipartGrantId,
    ) -> Result<Option<MultipartGrantRecordV1>, StorageFailure> {
        let record = self
            .state
            .lock()
            .map_err(|_| unavailable())?
            .grants
            .get(&id)
            .filter(|grant| grant.scope().tenant_id() == context.tenant_id())
            .cloned();
        if record.is_none() {
            return Ok(None);
        }
        self.guard(MultipartJournalOperationV1::GetGrant)?;
        Ok(record)
    }

    async fn revoke_grant(
        &self,
        context: StorageRequestContext,
        id: MultipartGrantId,
        revoked_at: TimestampMillis,
    ) -> Result<(), StorageFailure> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let Some(grant) = state.grants.get_mut(&id) else {
            return Ok(());
        };
        if grant.scope().tenant_id() != context.tenant_id() {
            return Ok(());
        }
        self.guard(MultipartJournalOperationV1::RevokeGrant)?;
        grant.revoke(revoked_at).map_err(|_| invalid())
    }

    async fn claim_create(
        &self,
        context: StorageRequestContext,
        grant_id: MultipartGrantId,
        now: TimestampMillis,
        idempotency_key: IdempotencyKey,
        fingerprint: ChecksumSha256,
        draft: MultipartUploadSnapshotV1,
    ) -> Result<JournalCreateOutcomeV1, StorageFailure> {
        Self::authorize_snapshot(context, &draft)?;
        if draft.phase() != MultipartJournalPhaseV1::Creating
            || draft.provider_session().is_some()
            || !draft.parts().is_empty()
            || draft.completed().is_some()
            || draft.correlation_id() != context.correlation_id()
        {
            return Err(invalid());
        }
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let grant = state.grants.get(&grant_id).ok_or_else(not_found)?;
        if !grant.active_at(now)
            || grant.scope().tenant_id() != context.tenant_id()
            || grant.scope().key() != draft.spec().key()
            || grant.scope().upload_id().is_some()
            || grant.scope().operation() != MultipartOperationV1::Create
        {
            return Err(not_found());
        }
        self.guard(MultipartJournalOperationV1::ClaimCreate)?;
        if let Some(previous) = state.create_grants.get(&grant_id) {
            if previous.tenant_id != context.tenant_id()
                || previous.idempotency_key != idempotency_key
                || previous.fingerprint != fingerprint
            {
                return Err(conflict());
            }
            let snapshot = state
                .uploads
                .get(&previous.upload_id)
                .ok_or_else(unavailable)?
                .snapshot
                .clone();
            return if snapshot.phase() == MultipartJournalPhaseV1::Creating {
                Ok(JournalCreateOutcomeV1::Resume(snapshot))
            } else {
                Ok(JournalCreateOutcomeV1::Replay(snapshot))
            };
        }
        let replay_key = Self::replay_key(
            context,
            MultipartReplayKeyV1::client(idempotency_key.clone()),
        );
        if let Some(previous) = state.replays.get(&replay_key) {
            let JournalReplayV1::Create {
                fingerprint: previous_fingerprint,
                upload_id,
            } = previous
            else {
                return Err(conflict());
            };
            if previous_fingerprint != &fingerprint {
                return Err(conflict());
            }
            let upload_id = *upload_id;
            let snapshot = state
                .uploads
                .get(&upload_id)
                .ok_or_else(unavailable)?
                .snapshot
                .clone();
            state.create_grants.insert(
                grant_id,
                JournalCreateGrantClaimV1 {
                    tenant_id: context.tenant_id(),
                    idempotency_key,
                    fingerprint,
                    upload_id,
                },
            );
            return if snapshot.phase() == MultipartJournalPhaseV1::Creating {
                Ok(JournalCreateOutcomeV1::Resume(snapshot))
            } else {
                Ok(JournalCreateOutcomeV1::Replay(snapshot))
            };
        }
        if state.uploads.contains_key(&draft.upload_id()) {
            return Err(conflict());
        }
        state.create_grants.insert(
            grant_id,
            JournalCreateGrantClaimV1 {
                tenant_id: context.tenant_id(),
                idempotency_key,
                fingerprint: fingerprint.clone(),
                upload_id: draft.upload_id(),
            },
        );
        state.replays.insert(
            replay_key,
            JournalReplayV1::Create {
                fingerprint,
                upload_id: draft.upload_id(),
            },
        );
        state.uploads.insert(
            draft.upload_id(),
            JournalUploadV1 {
                snapshot: draft.clone(),
                finalized: None,
            },
        );
        Ok(JournalCreateOutcomeV1::Claimed(draft))
    }

    async fn activate_upload(
        &self,
        context: StorageRequestContext,
        session: ProviderMultipartSessionV1,
    ) -> Result<MultipartUploadSnapshotV1, StorageFailure> {
        if session.key().tenant_id() != context.tenant_id()
            || session.correlation_id() != context.correlation_id()
        {
            return Err(not_found());
        }
        self.guard(MultipartJournalOperationV1::Activate)?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let upload = state
            .uploads
            .get_mut(&session.upload_id())
            .ok_or_else(not_found)?;
        Self::authorize_snapshot(context, &upload.snapshot)?;
        if session.key() != upload.snapshot.spec().key()
            || session.expires_at() != upload.snapshot.expires_at()
        {
            return Err(integrity());
        }
        if let Some(previous) = upload.snapshot.provider_session() {
            if previous.handle() != session.handle() {
                return Err(integrity());
            }
            return Ok(upload.snapshot.clone());
        }
        if upload.snapshot.phase() != MultipartJournalPhaseV1::Creating {
            return Err(conflict());
        }
        upload.snapshot = Self::snapshot_with(
            &upload.snapshot,
            Some(session),
            Vec::new(),
            MultipartJournalPhaseV1::Uploading,
            None,
        );
        Ok(upload.snapshot.clone())
    }

    async fn get_upload(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
    ) -> Result<Option<MultipartUploadSnapshotV1>, StorageFailure> {
        let snapshot = self
            .state
            .lock()
            .map_err(|_| unavailable())?
            .uploads
            .get(&upload_id)
            .filter(|upload| upload.snapshot.spec().key().tenant_id() == context.tenant_id())
            .map(|upload| upload.snapshot.clone());
        if snapshot.is_none() {
            return Ok(None);
        }
        self.guard(MultipartJournalOperationV1::GetUpload)?;
        Ok(snapshot)
    }

    async fn get_finalize(
        &self,
        context: StorageRequestContext,
        upload_id: MultipartUploadId,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure> {
        let record = self
            .state
            .lock()
            .map_err(|_| unavailable())?
            .uploads
            .get(&upload_id)
            .filter(|upload| upload.snapshot.spec().key().tenant_id() == context.tenant_id())
            .and_then(|upload| upload.finalized.clone());
        if record.is_none() {
            return Ok(None);
        }
        self.guard(MultipartJournalOperationV1::GetUpload)?;
        Ok(record)
    }

    async fn get_finalize_by_key(
        &self,
        context: StorageRequestContext,
        key: ScopedObjectKey,
    ) -> Result<Option<SourceFinalizeRecordV1>, StorageFailure> {
        if key.tenant_id() != context.tenant_id() {
            return Ok(None);
        }
        let records = self
            .state
            .lock()
            .map_err(|_| unavailable())?
            .uploads
            .values()
            .filter_map(|upload| upload.finalized.as_ref())
            .filter(|record| record.key() == &key)
            .cloned()
            .collect::<Vec<_>>();
        let record = match records.as_slice() {
            [] => None,
            [record] => Some(record.clone()),
            _ => return Err(integrity()),
        };
        if record.is_none() {
            return Ok(None);
        }
        self.guard(MultipartJournalOperationV1::GetUpload)?;
        Ok(record)
    }

    async fn record_part(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        receipt: ProviderPartReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderPartReceiptV1>, StorageFailure> {
        if receipt.key().tenant_id() != context.tenant_id()
            || receipt.correlation_id() != context.correlation_id()
        {
            return Err(not_found());
        }
        self.guard(MultipartJournalOperationV1::RecordPart)?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let replay_key = Self::replay_key(context, replay_key);
        if let Some(previous) = state.replays.get(&replay_key) {
            let JournalReplayV1::Part {
                fingerprint: previous_fingerprint,
                receipt: previous_receipt,
            } = previous
            else {
                return Err(conflict());
            };
            return if previous_fingerprint == &fingerprint
                && same_part_identity(previous_receipt, &receipt)
            {
                Ok(JournalMutationOutcomeV1::Replay(rebind_part_receipt(
                    previous_receipt,
                    context.correlation_id(),
                )))
            } else {
                Err(conflict())
            };
        }
        let upload = state
            .uploads
            .get_mut(&receipt.upload_id())
            .ok_or_else(not_found)?;
        Self::authorize_snapshot(context, &upload.snapshot)?;
        if upload.snapshot.phase() != MultipartJournalPhaseV1::Uploading
            || receipt.key() != upload.snapshot.spec().key()
            || upload
                .snapshot
                .spec()
                .validate_part(receipt.part_number(), receipt.size())
                .is_err()
        {
            return Err(conflict());
        }
        if let Some(previous) = upload
            .snapshot
            .parts()
            .iter()
            .find(|part| part.part_number() == receipt.part_number())
        {
            return if same_part_identity(previous, &receipt) {
                let previous = previous.clone();
                let rebound = rebind_part_receipt(&previous, context.correlation_id());
                state.replays.insert(
                    replay_key,
                    JournalReplayV1::Part {
                        fingerprint,
                        receipt: previous,
                    },
                );
                Ok(JournalMutationOutcomeV1::Replay(rebound))
            } else {
                Err(conflict())
            };
        }
        let mut parts = upload.snapshot.parts().to_vec();
        parts.push(receipt.clone());
        parts.sort_by_key(ProviderPartReceiptV1::part_number);
        upload.snapshot = Self::snapshot_with(
            &upload.snapshot,
            upload.snapshot.provider_session().cloned(),
            parts,
            MultipartJournalPhaseV1::Uploading,
            None,
        );
        state.replays.insert(
            replay_key,
            JournalReplayV1::Part {
                fingerprint,
                receipt: receipt.clone(),
            },
        );
        Ok(JournalMutationOutcomeV1::Applied(receipt))
    }

    async fn record_provider_complete(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        completed: ProviderCompletedObjectV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderCompletedObjectV1>, StorageFailure> {
        if completed.key().tenant_id() != context.tenant_id()
            || completed.correlation_id() != context.correlation_id()
        {
            return Err(not_found());
        }
        self.guard(MultipartJournalOperationV1::RecordComplete)?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let replay_key = Self::replay_key(context, replay_key);
        if let Some(previous) = state.replays.get(&replay_key) {
            let JournalReplayV1::Complete {
                fingerprint: previous_fingerprint,
                completed: previous_completed,
            } = previous
            else {
                return Err(conflict());
            };
            return if previous_fingerprint == &fingerprint
                && same_completed_identity(previous_completed, &completed)
            {
                Ok(JournalMutationOutcomeV1::Replay(rebind_completed_object(
                    previous_completed,
                    context.correlation_id(),
                )))
            } else {
                Err(conflict())
            };
        }
        let upload = state
            .uploads
            .get_mut(&completed.upload_id())
            .ok_or_else(not_found)?;
        Self::authorize_snapshot(context, &upload.snapshot)?;
        let spec = upload.snapshot.spec();
        if completed.key() != spec.key()
            || completed.size() != spec.total_size()
            || completed.checksum_sha256() != spec.checksum_sha256()
            || completed.content_type() != spec.content_type()
        {
            return Err(integrity());
        }
        if let Some(previous) = upload.snapshot.completed() {
            return if same_completed_identity(previous, &completed) {
                let previous = previous.clone();
                let rebound = rebind_completed_object(&previous, context.correlation_id());
                state.replays.insert(
                    replay_key,
                    JournalReplayV1::Complete {
                        fingerprint,
                        completed: previous,
                    },
                );
                Ok(JournalMutationOutcomeV1::Replay(rebound))
            } else {
                Err(integrity())
            };
        }
        if upload.snapshot.phase() != MultipartJournalPhaseV1::Uploading {
            return Err(conflict());
        }
        upload.snapshot = Self::snapshot_with(
            &upload.snapshot,
            upload.snapshot.provider_session().cloned(),
            upload.snapshot.parts().to_vec(),
            MultipartJournalPhaseV1::ProviderCompleted,
            Some(completed.clone()),
        );
        state.replays.insert(
            replay_key,
            JournalReplayV1::Complete {
                fingerprint,
                completed: completed.clone(),
            },
        );
        Ok(JournalMutationOutcomeV1::Applied(completed))
    }

    async fn finalize(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        record: SourceFinalizeRecordV1,
    ) -> Result<JournalMutationOutcomeV1<SourceFinalizeRecordV1>, StorageFailure> {
        if record.key().tenant_id() != context.tenant_id()
            || record.correlation_id() != context.correlation_id()
        {
            return Err(not_found());
        }
        self.guard(MultipartJournalOperationV1::Finalize)?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let replay_key = Self::replay_key(context, replay_key);
        if let Some(previous) = state.replays.get(&replay_key) {
            let JournalReplayV1::Finalize {
                fingerprint: previous_fingerprint,
                record: previous_record,
            } = previous
            else {
                return Err(conflict());
            };
            if previous_fingerprint != &fingerprint
                || !same_finalize_identity(previous_record, &record)
            {
                return Err(conflict());
            }
            if record.finalized_at() < previous_record.provider_last_modified() {
                return Err(integrity());
            }
            return Ok(JournalMutationOutcomeV1::Replay(rebind_finalize_record(
                previous_record,
                context.correlation_id(),
            )));
        }
        let upload = state
            .uploads
            .get_mut(&record.upload_id())
            .ok_or_else(not_found)?;
        Self::authorize_snapshot(context, &upload.snapshot)?;
        let completed = upload.snapshot.completed().ok_or_else(conflict)?;
        if record.key() != completed.key()
            || record.provider_version() != completed.provider_version()
            || record.provider_etag() != completed.provider_etag()
            || record.size() != completed.size()
            || record.checksum_sha256() != completed.checksum_sha256()
            || record.content_type() != completed.content_type()
            || record.provider_last_modified() != completed.last_modified()
            || record.media_probe() != completed.media_probe()
            || record.finalized_at() < completed.last_modified()
        {
            return Err(integrity());
        }
        if let Some(previous) = &upload.finalized {
            return if same_finalize_identity(previous, &record) {
                let previous = previous.clone();
                let rebound = rebind_finalize_record(&previous, context.correlation_id());
                state.replays.insert(
                    replay_key,
                    JournalReplayV1::Finalize {
                        fingerprint,
                        record: previous,
                    },
                );
                Ok(JournalMutationOutcomeV1::Replay(rebound))
            } else {
                Err(conflict())
            };
        }
        if upload.snapshot.phase() != MultipartJournalPhaseV1::ProviderCompleted {
            return Err(conflict());
        }
        upload.finalized = Some(record.clone());
        upload.snapshot = Self::snapshot_with(
            &upload.snapshot,
            upload.snapshot.provider_session().cloned(),
            upload.snapshot.parts().to_vec(),
            MultipartJournalPhaseV1::Finalized,
            upload.snapshot.completed().cloned(),
        );
        state.replays.insert(
            replay_key,
            JournalReplayV1::Finalize {
                fingerprint,
                record: record.clone(),
            },
        );
        Ok(JournalMutationOutcomeV1::Applied(record))
    }

    async fn abort(
        &self,
        context: StorageRequestContext,
        replay_key: MultipartReplayKeyV1,
        fingerprint: ChecksumSha256,
        receipt: ProviderAbortReceiptV1,
    ) -> Result<JournalMutationOutcomeV1<ProviderAbortReceiptV1>, StorageFailure> {
        if receipt.key().tenant_id() != context.tenant_id()
            || receipt.correlation_id() != context.correlation_id()
        {
            return Err(not_found());
        }
        self.guard(MultipartJournalOperationV1::Abort)?;
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        let replay_key = Self::replay_key(context, replay_key);
        if let Some(previous) = state.replays.get(&replay_key) {
            let JournalReplayV1::Abort {
                fingerprint: previous_fingerprint,
                receipt: previous_receipt,
            } = previous
            else {
                return Err(conflict());
            };
            return if previous_fingerprint == &fingerprint
                && same_abort_identity(previous_receipt, &receipt)
            {
                Ok(JournalMutationOutcomeV1::Replay(rebind_abort_receipt(
                    previous_receipt,
                    context.correlation_id(),
                )))
            } else {
                Err(conflict())
            };
        }
        let upload = state
            .uploads
            .get_mut(&receipt.upload_id())
            .ok_or_else(not_found)?;
        Self::authorize_snapshot(context, &upload.snapshot)?;
        if receipt.key() != upload.snapshot.spec().key() {
            return Err(integrity());
        }
        if matches!(
            receipt.disposition(),
            ProviderAbortDispositionV1::Aborted | ProviderAbortDispositionV1::AlreadyAborted
        ) {
            if matches!(
                upload.snapshot.phase(),
                MultipartJournalPhaseV1::ProviderCompleted | MultipartJournalPhaseV1::Finalized
            ) {
                return Err(integrity());
            }
            upload.snapshot = Self::snapshot_with(
                &upload.snapshot,
                upload.snapshot.provider_session().cloned(),
                upload.snapshot.parts().to_vec(),
                MultipartJournalPhaseV1::Aborted,
                None,
            );
        }
        state.replays.insert(
            replay_key,
            JournalReplayV1::Abort {
                fingerprint,
                receipt: receipt.clone(),
            },
        );
        Ok(JournalMutationOutcomeV1::Applied(receipt))
    }

    async fn reconciliation_candidates(
        &self,
        context: StorageRequestContext,
        limit: u16,
    ) -> Result<Vec<MultipartUploadSnapshotV1>, StorageFailure> {
        if !(1..=100).contains(&limit) {
            return Err(invalid());
        }
        self.guard(MultipartJournalOperationV1::Reconcile)?;
        let mut candidates = self
            .state
            .lock()
            .map_err(|_| unavailable())?
            .uploads
            .values()
            .filter(|upload| {
                upload.snapshot.spec().key().tenant_id() == context.tenant_id()
                    && matches!(
                        upload.snapshot.phase(),
                        MultipartJournalPhaseV1::Creating
                            | MultipartJournalPhaseV1::Uploading
                            | MultipartJournalPhaseV1::ProviderCompleted
                    )
            })
            .map(|upload| upload.snapshot.clone())
            .collect::<Vec<_>>();
        candidates.sort_by_key(|candidate| candidate.upload_id().to_string());
        candidates.truncate(usize::from(limit));
        Ok(candidates)
    }
}
