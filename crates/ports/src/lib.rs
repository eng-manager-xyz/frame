use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{
        RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use frame_domain::{
    ByteSize, ChecksumSha256, ContentType, CutoverState, EtlCheckpoint, EtlManifest, EtlRunId,
    MultipartUploadId, ObjectKey, Page, PageCursor, PageRequest, ReconciliationSummary, SessionId,
    SessionRecord, TenantId, TimestampMillis, Video, VideoId,
};
use thiserror::Error;

mod identity;
mod multipart;
mod storage;

pub use identity::*;
pub use multipart::*;
pub use storage::*;

#[derive(Error, PartialEq, Eq)]
pub enum PortError {
    #[error("resource not found")]
    NotFound,
    #[error("resource already exists")]
    Conflict,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("unsupported capability: {0}")]
    Unsupported(String),
    #[error("adapter failure")]
    Adapter(String),
}

impl fmt::Debug for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("NotFound"),
            Self::Conflict => formatter.write_str("Conflict"),
            Self::InvalidRequest(message) => formatter
                .debug_tuple("InvalidRequest")
                .field(message)
                .finish(),
            Self::Unsupported(capability) => formatter
                .debug_tuple("Unsupported")
                .field(capability)
                .finish(),
            Self::Adapter(_) => formatter.write_str("Adapter([redacted])"),
        }
    }
}

impl PortError {
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Adapter(_))
    }
}

#[async_trait]
pub trait VideoRepository: Send + Sync {
    async fn insert(&self, video: Video) -> Result<(), PortError>;
    async fn get(&self, id: VideoId) -> Result<Option<Video>, PortError>;
    async fn save(&self, video: Video) -> Result<(), PortError>;
}

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn put(&self, key: &ObjectKey, bytes: Vec<u8>) -> Result<(), PortError>;
    async fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>, PortError>;
    async fn delete(&self, key: &ObjectKey) -> Result<(), PortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DerivativeKind {
    OptimizedVideo,
    Frame,
    Spritesheet,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaExecutor {
    CloudflareMedia,
    NativeGstreamer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTransformRequest {
    pub source: ObjectKey,
    pub output: ObjectKey,
    pub kind: DerivativeKind,
    pub profile_version: u16,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTransformResult {
    pub output: ObjectKey,
    pub executor: MediaExecutor,
    pub profile_version: u16,
    pub content_type: String,
}

#[async_trait]
pub trait MediaTransformer: Send + Sync {
    fn executor(&self) -> MediaExecutor;
    fn supports(&self, kind: DerivativeKind) -> bool;

    async fn transform(
        &self,
        request: &MediaTransformRequest,
    ) -> Result<MediaTransformResult, PortError>;
}

#[derive(Default)]
pub struct MemoryVideoRepository {
    videos: RwLock<HashMap<VideoId, Video>>,
}

impl fmt::Debug for MemoryVideoRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryVideoRepository")
            .field(
                "video_count",
                &self.videos.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

#[async_trait]
impl VideoRepository for MemoryVideoRepository {
    async fn insert(&self, video: Video) -> Result<(), PortError> {
        let mut videos = self.videos.write().map_err(lock_error)?;
        if videos.contains_key(&video.id) {
            return Err(PortError::Conflict);
        }
        videos.insert(video.id, video);
        Ok(())
    }

    async fn get(&self, id: VideoId) -> Result<Option<Video>, PortError> {
        let videos = self.videos.read().map_err(lock_error)?;
        Ok(videos.get(&id).cloned())
    }

    async fn save(&self, video: Video) -> Result<(), PortError> {
        let mut videos = self.videos.write().map_err(lock_error)?;
        if !videos.contains_key(&video.id) {
            return Err(PortError::NotFound);
        }
        videos.insert(video.id, video);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct StoredObject {
    bytes: Vec<u8>,
    metadata: ObjectMetadata,
}

#[derive(Debug, Clone)]
struct StoredPart {
    bytes: Vec<u8>,
    etag: EntityTag,
    checksum: Option<ChecksumSha256>,
}

#[derive(Debug, Clone)]
struct PendingMultipart {
    key: ObjectKey,
    options: PutOptions,
    parts: BTreeMap<PartNumber, StoredPart>,
}

#[derive(Default)]
pub struct MemoryObjectStore {
    objects: RwLock<HashMap<String, StoredObject>>,
    multiparts: RwLock<HashMap<MultipartUploadId, PendingMultipart>>,
    logical_clock: AtomicU64,
}

impl fmt::Debug for MemoryObjectStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryObjectStore")
            .field(
                "object_count",
                &self.objects.read().map_or(0, |rows| rows.len()),
            )
            .field(
                "multipart_count",
                &self.multiparts.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

pub struct MemoryMediaTransformer {
    executor: MediaExecutor,
    supported: Vec<DerivativeKind>,
    results: RwLock<HashMap<String, (MediaTransformRequest, MediaTransformResult)>>,
}

impl fmt::Debug for MemoryMediaTransformer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryMediaTransformer")
            .field("executor", &self.executor)
            .field("supported", &self.supported)
            .field(
                "result_count",
                &self.results.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

impl MemoryMediaTransformer {
    #[must_use]
    pub fn new(
        executor: MediaExecutor,
        supported: impl IntoIterator<Item = DerivativeKind>,
    ) -> Self {
        Self {
            executor,
            supported: supported.into_iter().collect(),
            results: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl MediaTransformer for MemoryMediaTransformer {
    fn executor(&self) -> MediaExecutor {
        self.executor
    }

    fn supports(&self, kind: DerivativeKind) -> bool {
        self.supported.contains(&kind)
    }

    async fn transform(
        &self,
        request: &MediaTransformRequest,
    ) -> Result<MediaTransformResult, PortError> {
        if request.profile_version == 0 {
            return Err(PortError::InvalidRequest(
                "media transform profile version must be non-zero".into(),
            ));
        }
        if request.content_type.trim().is_empty() {
            return Err(PortError::InvalidRequest(
                "media transform content type must be non-empty".into(),
            ));
        }
        if !self.supports(request.kind) {
            return Err(PortError::Unsupported(format!("{:?}", request.kind)));
        }

        let mut results = self.results.write().map_err(lock_error)?;
        if let Some((previous_request, result)) = results.get(request.output.as_str()) {
            return if previous_request == request {
                Ok(result.clone())
            } else {
                Err(PortError::Conflict)
            };
        }

        let result = MediaTransformResult {
            output: request.output.clone(),
            executor: self.executor,
            profile_version: request.profile_version,
            content_type: request.content_type.clone(),
        };
        results.insert(
            request.output.as_str().to_owned(),
            (request.clone(), result.clone()),
        );
        Ok(result)
    }
}

#[async_trait]
impl ObjectStore for MemoryObjectStore {
    async fn put(&self, key: &ObjectKey, bytes: Vec<u8>) -> Result<(), PortError> {
        self.put_with_options(key, bytes, PutOptions::binary())
            .await
            .map(|_| ())
    }

    async fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>, PortError> {
        Ok(self
            .objects
            .read()
            .map_err(lock_error)?
            .get(key.as_str())
            .map(|object| object.bytes.clone()))
    }

    async fn delete(&self, key: &ObjectKey) -> Result<(), PortError> {
        self.objects
            .write()
            .map_err(lock_error)?
            .remove(key.as_str());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedVideo {
    pub tenant_id: TenantId,
    pub video: Video,
    pub revision: u64,
}

#[async_trait]
pub trait TenantVideoRepository: Send + Sync {
    async fn insert_scoped(&self, video: VersionedVideo) -> Result<(), PortError>;
    async fn get_scoped(
        &self,
        tenant_id: TenantId,
        video_id: VideoId,
    ) -> Result<Option<VersionedVideo>, PortError>;
    async fn list_scoped(
        &self,
        tenant_id: TenantId,
        request: &PageRequest,
    ) -> Result<Page<VersionedVideo>, PortError>;
    async fn compare_and_save(
        &self,
        expected_revision: u64,
        video: VersionedVideo,
    ) -> Result<VersionedVideo, PortError>;
}

#[derive(Default)]
pub struct MemoryTenantVideoRepository {
    videos: RwLock<HashMap<(TenantId, VideoId), VersionedVideo>>,
}

impl fmt::Debug for MemoryTenantVideoRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryTenantVideoRepository")
            .field(
                "video_count",
                &self.videos.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

#[async_trait]
impl TenantVideoRepository for MemoryTenantVideoRepository {
    async fn insert_scoped(&self, video: VersionedVideo) -> Result<(), PortError> {
        if video.revision != 0 {
            return Err(PortError::InvalidRequest(
                "new aggregates must start at revision zero".into(),
            ));
        }
        let key = (video.tenant_id, video.video.id);
        let mut videos = self.videos.write().map_err(lock_error)?;
        if videos.contains_key(&key) {
            return Err(PortError::Conflict);
        }
        videos.insert(key, video);
        Ok(())
    }

    async fn get_scoped(
        &self,
        tenant_id: TenantId,
        video_id: VideoId,
    ) -> Result<Option<VersionedVideo>, PortError> {
        Ok(self
            .videos
            .read()
            .map_err(lock_error)?
            .get(&(tenant_id, video_id))
            .cloned())
    }

    async fn list_scoped(
        &self,
        tenant_id: TenantId,
        request: &PageRequest,
    ) -> Result<Page<VersionedVideo>, PortError> {
        let videos = self.videos.read().map_err(lock_error)?;
        let mut rows = videos
            .values()
            .filter(|video| video.tenant_id == tenant_id)
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by_key(|video| video.video.id.to_string());

        let start = request.cursor.as_ref().map_or(0, |cursor| {
            rows.iter()
                .position(|video| video.video.id.to_string().as_str() > cursor.expose())
                .unwrap_or(rows.len())
        });
        let limit = usize::from(request.limit.get());
        let end = start.saturating_add(limit).min(rows.len());
        let items = rows[start..end].to_vec();
        let next_cursor = if end < rows.len() {
            items
                .last()
                .map(|video| PageCursor::parse(video.video.id.to_string()))
                .transpose()
                .map_err(|error| PortError::Adapter(error.to_string()))?
        } else {
            None
        };
        Ok(Page { items, next_cursor })
    }

    async fn compare_and_save(
        &self,
        expected_revision: u64,
        mut video: VersionedVideo,
    ) -> Result<VersionedVideo, PortError> {
        let key = (video.tenant_id, video.video.id);
        let mut videos = self.videos.write().map_err(lock_error)?;
        let current = videos.get(&key).ok_or(PortError::NotFound)?;
        if current.revision != expected_revision || video.revision != expected_revision {
            return Err(PortError::Conflict);
        }
        video.revision = expected_revision
            .checked_add(1)
            .ok_or_else(|| PortError::InvalidRequest("revision is exhausted".into()))?;
        videos.insert(key, video.clone());
        Ok(video)
    }
}

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn insert_session(&self, session: SessionRecord) -> Result<(), PortError>;
    async fn get_session(&self, id: SessionId) -> Result<Option<SessionRecord>, PortError>;
    async fn save_session(&self, session: SessionRecord) -> Result<(), PortError>;
}

#[derive(Default)]
pub struct MemorySessionRepository {
    sessions: RwLock<HashMap<SessionId, SessionRecord>>,
}

impl fmt::Debug for MemorySessionRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemorySessionRepository")
            .field(
                "session_count",
                &self.sessions.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

#[async_trait]
impl SessionRepository for MemorySessionRepository {
    async fn insert_session(&self, session: SessionRecord) -> Result<(), PortError> {
        let mut sessions = self.sessions.write().map_err(lock_error)?;
        if sessions.contains_key(&session.id) {
            return Err(PortError::Conflict);
        }
        sessions.insert(session.id, session);
        Ok(())
    }

    async fn get_session(&self, id: SessionId) -> Result<Option<SessionRecord>, PortError> {
        Ok(self.sessions.read().map_err(lock_error)?.get(&id).cloned())
    }

    async fn save_session(&self, session: SessionRecord) -> Result<(), PortError> {
        let mut sessions = self.sessions.write().map_err(lock_error)?;
        if !sessions.contains_key(&session.id) {
            return Err(PortError::NotFound);
        }
        sessions.insert(session.id, session);
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct EntityTag(String);

impl EntityTag {
    pub fn parse(value: impl Into<String>) -> Result<Self, PortError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
            })
        {
            return Err(PortError::InvalidRequest("entity tag is invalid".into()));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EntityTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("EntityTag").field(&self.0).finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub key: ObjectKey,
    pub size: ByteSize,
    pub content_type: ContentType,
    pub checksum_sha256: Option<ChecksumSha256>,
    pub etag: EntityTag,
    pub last_modified: TimestampMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteCondition {
    Any,
    IfAbsent,
    IfMatch(EntityTag),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutOptions {
    pub content_type: ContentType,
    pub checksum_sha256: Option<ChecksumSha256>,
    pub condition: WriteCondition,
}

impl PutOptions {
    #[must_use]
    pub fn binary() -> Self {
        Self {
            content_type: ContentType::parse("application/octet-stream")
                .expect("the built-in binary content type is valid"),
            checksum_sha256: None,
            condition: WriteCondition::Any,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end_exclusive: u64,
}

impl ByteRange {
    pub fn new(start: u64, end_exclusive: u64) -> Result<Self, PortError> {
        if start >= end_exclusive || end_exclusive > frame_domain::MAX_WIRE_INTEGER {
            return Err(PortError::InvalidRequest("byte range is invalid".into()));
        }
        Ok(Self {
            start,
            end_exclusive,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRead {
    pub metadata: ObjectMetadata,
    pub bytes: Vec<u8>,
    pub range: ByteRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageCapabilities {
    pub metadata: bool,
    pub ranges: bool,
    pub conditional_writes: bool,
    pub multipart: bool,
    pub min_multipart_part_size: ByteSize,
    pub max_multipart_parts: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartNumber(u16);

impl PartNumber {
    pub fn new(value: u16) -> Result<Self, PortError> {
        if !(1..=10_000).contains(&value) {
            return Err(PortError::InvalidRequest(
                "multipart part number must be between 1 and 10000".into(),
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartUpload {
    pub id: MultipartUploadId,
    pub key: ObjectKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedPart {
    pub part_number: PartNumber,
    pub size: ByteSize,
    pub etag: EntityTag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedPart {
    pub part_number: PartNumber,
    pub etag: EntityTag,
}

#[async_trait]
pub trait AdvancedObjectStore: ObjectStore {
    fn capabilities(&self) -> StorageCapabilities;

    async fn head(&self, key: &ObjectKey) -> Result<Option<ObjectMetadata>, PortError>;
    async fn put_with_options(
        &self,
        key: &ObjectKey,
        bytes: Vec<u8>,
        options: PutOptions,
    ) -> Result<ObjectMetadata, PortError>;
    async fn get_range(
        &self,
        key: &ObjectKey,
        range: ByteRange,
    ) -> Result<Option<ObjectRead>, PortError>;
    async fn delete_if_match(
        &self,
        key: &ObjectKey,
        expected: Option<&EntityTag>,
    ) -> Result<bool, PortError>;
    async fn begin_multipart(
        &self,
        key: &ObjectKey,
        options: PutOptions,
    ) -> Result<MultipartUpload, PortError>;
    async fn upload_part(
        &self,
        upload_id: MultipartUploadId,
        part_number: PartNumber,
        bytes: Vec<u8>,
        checksum: Option<ChecksumSha256>,
    ) -> Result<UploadedPart, PortError>;
    async fn complete_multipart(
        &self,
        upload_id: MultipartUploadId,
        parts: &[CompletedPart],
    ) -> Result<ObjectMetadata, PortError>;
    async fn abort_multipart(&self, upload_id: MultipartUploadId) -> Result<(), PortError>;
}

impl MemoryObjectStore {
    fn next_timestamp(&self) -> Result<TimestampMillis, PortError> {
        let value = self.logical_clock.fetch_add(1, Ordering::Relaxed) + 1;
        let value = i64::try_from(value)
            .map_err(|_| PortError::Adapter("logical clock overflow".into()))?;
        TimestampMillis::new(value).map_err(|error| PortError::Adapter(error.to_string()))
    }

    fn make_etag(bytes: &[u8]) -> Result<EntityTag, PortError> {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        EntityTag::parse(format!("m-{hash:016x}-{}", bytes.len()))
    }

    fn condition_allows(existing: Option<&StoredObject>, condition: &WriteCondition) -> bool {
        match condition {
            WriteCondition::Any => true,
            WriteCondition::IfAbsent => existing.is_none(),
            WriteCondition::IfMatch(expected) => {
                existing.is_some_and(|object| object.metadata.etag == *expected)
            }
        }
    }
}

#[async_trait]
impl AdvancedObjectStore for MemoryObjectStore {
    fn capabilities(&self) -> StorageCapabilities {
        StorageCapabilities {
            metadata: true,
            ranges: true,
            conditional_writes: true,
            multipart: true,
            min_multipart_part_size: ByteSize::new(1).expect("constant size is valid"),
            max_multipart_parts: 10_000,
        }
    }

    async fn head(&self, key: &ObjectKey) -> Result<Option<ObjectMetadata>, PortError> {
        Ok(self
            .objects
            .read()
            .map_err(lock_error)?
            .get(key.as_str())
            .map(|object| object.metadata.clone()))
    }

    async fn put_with_options(
        &self,
        key: &ObjectKey,
        bytes: Vec<u8>,
        options: PutOptions,
    ) -> Result<ObjectMetadata, PortError> {
        let size = ByteSize::new(
            u64::try_from(bytes.len())
                .map_err(|_| PortError::InvalidRequest("object is too large".into()))?,
        )
        .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
        let etag = Self::make_etag(&bytes)?;
        let mut objects = self.objects.write().map_err(lock_error)?;
        if !Self::condition_allows(objects.get(key.as_str()), &options.condition) {
            return Err(PortError::Conflict);
        }
        let metadata = ObjectMetadata {
            key: key.clone(),
            size,
            content_type: options.content_type,
            checksum_sha256: options.checksum_sha256,
            etag,
            last_modified: self.next_timestamp()?,
        };
        objects.insert(
            key.as_str().to_owned(),
            StoredObject {
                bytes,
                metadata: metadata.clone(),
            },
        );
        Ok(metadata)
    }

    async fn get_range(
        &self,
        key: &ObjectKey,
        range: ByteRange,
    ) -> Result<Option<ObjectRead>, PortError> {
        let objects = self.objects.read().map_err(lock_error)?;
        let Some(object) = objects.get(key.as_str()) else {
            return Ok(None);
        };
        let length = u64::try_from(object.bytes.len())
            .map_err(|_| PortError::Adapter("object length overflow".into()))?;
        if range.start >= length {
            return Err(PortError::InvalidRequest(
                "byte range starts beyond the object".into(),
            ));
        }
        let end_exclusive = range.end_exclusive.min(length);
        let start = usize::try_from(range.start)
            .map_err(|_| PortError::InvalidRequest("byte range is invalid".into()))?;
        let end = usize::try_from(end_exclusive)
            .map_err(|_| PortError::InvalidRequest("byte range is invalid".into()))?;
        Ok(Some(ObjectRead {
            metadata: object.metadata.clone(),
            bytes: object.bytes[start..end].to_vec(),
            range: ByteRange {
                start: range.start,
                end_exclusive,
            },
        }))
    }

    async fn delete_if_match(
        &self,
        key: &ObjectKey,
        expected: Option<&EntityTag>,
    ) -> Result<bool, PortError> {
        let mut objects = self.objects.write().map_err(lock_error)?;
        let Some(object) = objects.get(key.as_str()) else {
            return Ok(false);
        };
        if expected.is_some_and(|etag| object.metadata.etag != *etag) {
            return Err(PortError::Conflict);
        }
        objects.remove(key.as_str());
        Ok(true)
    }

    async fn begin_multipart(
        &self,
        key: &ObjectKey,
        options: PutOptions,
    ) -> Result<MultipartUpload, PortError> {
        {
            let objects = self.objects.read().map_err(lock_error)?;
            if !Self::condition_allows(objects.get(key.as_str()), &options.condition) {
                return Err(PortError::Conflict);
            }
        }
        let upload = MultipartUpload {
            id: MultipartUploadId::new(),
            key: key.clone(),
        };
        self.multiparts.write().map_err(lock_error)?.insert(
            upload.id,
            PendingMultipart {
                key: key.clone(),
                options,
                parts: BTreeMap::new(),
            },
        );
        Ok(upload)
    }

    async fn upload_part(
        &self,
        upload_id: MultipartUploadId,
        part_number: PartNumber,
        bytes: Vec<u8>,
        checksum: Option<ChecksumSha256>,
    ) -> Result<UploadedPart, PortError> {
        if bytes.is_empty() {
            return Err(PortError::InvalidRequest(
                "multipart parts cannot be empty".into(),
            ));
        }
        let size = ByteSize::new(
            u64::try_from(bytes.len())
                .map_err(|_| PortError::InvalidRequest("part is too large".into()))?,
        )
        .map_err(|error| PortError::InvalidRequest(error.to_string()))?;
        let etag = Self::make_etag(&bytes)?;
        let mut uploads = self.multiparts.write().map_err(lock_error)?;
        let upload = uploads.get_mut(&upload_id).ok_or(PortError::NotFound)?;
        upload.parts.insert(
            part_number,
            StoredPart {
                bytes,
                etag: etag.clone(),
                checksum,
            },
        );
        Ok(UploadedPart {
            part_number,
            size,
            etag,
        })
    }

    async fn complete_multipart(
        &self,
        upload_id: MultipartUploadId,
        parts: &[CompletedPart],
    ) -> Result<ObjectMetadata, PortError> {
        if parts.is_empty() {
            return Err(PortError::InvalidRequest(
                "multipart completion requires at least one part".into(),
            ));
        }
        let pending = self
            .multiparts
            .read()
            .map_err(lock_error)?
            .get(&upload_id)
            .cloned()
            .ok_or(PortError::NotFound)?;
        let mut previous = None;
        let mut bytes = Vec::new();
        for completed in parts {
            if previous.is_some_and(|part| completed.part_number <= part) {
                return Err(PortError::InvalidRequest(
                    "multipart completion parts must be strictly ordered".into(),
                ));
            }
            let stored = pending
                .parts
                .get(&completed.part_number)
                .ok_or(PortError::NotFound)?;
            if stored.etag != completed.etag {
                return Err(PortError::Conflict);
            }
            let _checksum_was_recorded = stored.checksum.as_ref();
            bytes.extend_from_slice(&stored.bytes);
            previous = Some(completed.part_number);
        }
        let metadata = self
            .put_with_options(&pending.key, bytes, pending.options)
            .await?;
        self.multiparts
            .write()
            .map_err(lock_error)?
            .remove(&upload_id);
        Ok(metadata)
    }

    async fn abort_multipart(&self, upload_id: MultipartUploadId) -> Result<(), PortError> {
        self.multiparts
            .write()
            .map_err(lock_error)?
            .remove(&upload_id);
        Ok(())
    }
}

#[async_trait]
pub trait MigrationStateRepository: Send + Sync {
    async fn insert_manifest(&self, manifest: EtlManifest) -> Result<(), PortError>;
    async fn save_checkpoint(&self, checkpoint: EtlCheckpoint) -> Result<(), PortError>;
    async fn get_checkpoint(
        &self,
        run_id: EtlRunId,
        table: &str,
    ) -> Result<Option<EtlCheckpoint>, PortError>;
    async fn record_reconciliation(
        &self,
        run_id: EtlRunId,
        table: &str,
        summary: ReconciliationSummary,
    ) -> Result<(), PortError>;
    async fn cutover_state(&self) -> Result<CutoverState, PortError>;
    async fn compare_and_set_cutover(
        &self,
        expected_epoch: u64,
        next: CutoverState,
    ) -> Result<(), PortError>;
}

#[derive(Default)]
pub struct MemoryMigrationStateRepository {
    manifests: RwLock<HashMap<EtlRunId, EtlManifest>>,
    checkpoints: RwLock<HashMap<(EtlRunId, String), EtlCheckpoint>>,
    reconciliations: RwLock<HashMap<(EtlRunId, String), ReconciliationSummary>>,
    cutover: RwLock<CutoverState>,
}

impl fmt::Debug for MemoryMigrationStateRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryMigrationStateRepository")
            .field(
                "manifest_count",
                &self.manifests.read().map_or(0, |rows| rows.len()),
            )
            .field(
                "checkpoint_count",
                &self.checkpoints.read().map_or(0, |rows| rows.len()),
            )
            .finish()
    }
}

#[async_trait]
impl MigrationStateRepository for MemoryMigrationStateRepository {
    async fn insert_manifest(&self, manifest: EtlManifest) -> Result<(), PortError> {
        let mut manifests = self.manifests.write().map_err(lock_error)?;
        if manifests.contains_key(&manifest.run_id) {
            return Err(PortError::Conflict);
        }
        manifests.insert(manifest.run_id, manifest);
        Ok(())
    }

    async fn save_checkpoint(&self, checkpoint: EtlCheckpoint) -> Result<(), PortError> {
        let key = (checkpoint.run_id, checkpoint.table.clone());
        let mut checkpoints = self.checkpoints.write().map_err(lock_error)?;
        if checkpoints
            .get(&key)
            .is_some_and(|current| checkpoint.processed_rows < current.processed_rows)
        {
            return Err(PortError::Conflict);
        }
        checkpoints.insert(key, checkpoint);
        Ok(())
    }

    async fn get_checkpoint(
        &self,
        run_id: EtlRunId,
        table: &str,
    ) -> Result<Option<EtlCheckpoint>, PortError> {
        Ok(self
            .checkpoints
            .read()
            .map_err(lock_error)?
            .get(&(run_id, table.to_owned()))
            .cloned())
    }

    async fn record_reconciliation(
        &self,
        run_id: EtlRunId,
        table: &str,
        summary: ReconciliationSummary,
    ) -> Result<(), PortError> {
        if !self
            .manifests
            .read()
            .map_err(lock_error)?
            .contains_key(&run_id)
        {
            return Err(PortError::NotFound);
        }
        self.reconciliations
            .write()
            .map_err(lock_error)?
            .insert((run_id, table.to_owned()), summary);
        Ok(())
    }

    async fn cutover_state(&self) -> Result<CutoverState, PortError> {
        Ok(*self.cutover.read().map_err(lock_error)?)
    }

    async fn compare_and_set_cutover(
        &self,
        expected_epoch: u64,
        next: CutoverState,
    ) -> Result<(), PortError> {
        if next.epoch != expected_epoch.saturating_add(1) {
            return Err(PortError::InvalidRequest(
                "cutover epoch must advance exactly once".into(),
            ));
        }
        let mut state = self.cutover.write().map_err(lock_error)?;
        if state.epoch != expected_epoch {
            return Err(PortError::Conflict);
        }
        *state = next;
        Ok(())
    }
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> PortError {
    PortError::Adapter(format!("in-memory adapter lock poisoned: {error}"))
}

#[cfg(test)]
mod tests {
    use frame_domain::{
        CheckpointToken, CutoverEvidence, CutoverPhase, EtlTableManifest, SecretDigest,
        SessionState, UserId, VideoState,
    };

    use super::*;

    #[tokio::test]
    async fn memory_adapters_obey_contracts() {
        let videos = MemoryVideoRepository::default();
        let video = Video {
            id: VideoId::new(),
            owner_id: "user-1".into(),
            title: "Contract test".into(),
            state: VideoState::Pending,
            object_key: None,
            created_at_ms: 0,
        };
        videos.insert(video.clone()).await.expect("insert");
        assert_eq!(videos.get(video.id).await.expect("get"), Some(video));

        let objects = MemoryObjectStore::default();
        let key = ObjectKey::parse("videos/v1/source.webm").expect("valid key");
        objects.put(&key, b"frame".to_vec()).await.expect("put");
        assert_eq!(
            objects.get(&key).await.expect("get"),
            Some(b"frame".to_vec())
        );
        objects.delete(&key).await.expect("delete");
        assert_eq!(objects.get(&key).await.expect("get"), None);

        let transformer = MemoryMediaTransformer::new(
            MediaExecutor::CloudflareMedia,
            [DerivativeKind::Frame, DerivativeKind::Audio],
        );
        let request = MediaTransformRequest {
            source: ObjectKey::parse("videos/v1/source.mp4").expect("valid source key"),
            output: ObjectKey::parse("videos/v1/derivatives/thumbnail-v1.jpg")
                .expect("valid output key"),
            kind: DerivativeKind::Frame,
            profile_version: 1,
            content_type: "image/jpeg".into(),
        };
        let first = transformer.transform(&request).await.expect("transform");
        let replay = transformer.transform(&request).await.expect("replay");
        assert_eq!(first, replay);
        assert_eq!(first.executor, MediaExecutor::CloudflareMedia);
        assert!(!transformer.supports(DerivativeKind::OptimizedVideo));

        let conflicting_request = MediaTransformRequest {
            profile_version: 2,
            ..request
        };
        assert!(matches!(
            transformer.transform(&conflicting_request).await,
            Err(PortError::Conflict)
        ));
    }

    fn video(owner: &str) -> Video {
        Video {
            id: VideoId::new(),
            owner_id: owner.into(),
            title: "Scoped video".into(),
            state: VideoState::Pending,
            object_key: None,
            created_at_ms: 0,
        }
    }

    #[tokio::test]
    async fn tenant_repository_hides_cross_tenant_rows_and_checks_revisions() {
        let repository = MemoryTenantVideoRepository::default();
        let tenant_a = TenantId::new();
        let tenant_b = TenantId::new();
        let first = VersionedVideo {
            tenant_id: tenant_a,
            video: video("user-a"),
            revision: 0,
        };
        let id = first.video.id;
        repository
            .insert_scoped(first.clone())
            .await
            .expect("insert");
        assert_eq!(
            repository
                .get_scoped(tenant_b, id)
                .await
                .expect("cross-tenant lookup"),
            None
        );
        assert_eq!(
            repository
                .compare_and_save(
                    1,
                    VersionedVideo {
                        revision: 1,
                        ..first.clone()
                    }
                )
                .await,
            Err(PortError::Conflict)
        );
        let saved = repository
            .compare_and_save(0, first)
            .await
            .expect("compare and save");
        assert_eq!(saved.revision, 1);
    }

    #[tokio::test]
    async fn tenant_repository_paginates_deterministically() {
        let repository = MemoryTenantVideoRepository::default();
        let tenant = TenantId::new();
        for owner in ["one", "two", "three"] {
            repository
                .insert_scoped(VersionedVideo {
                    tenant_id: tenant,
                    video: video(owner),
                    revision: 0,
                })
                .await
                .expect("insert");
        }
        let first = repository
            .list_scoped(
                tenant,
                &PageRequest {
                    cursor: None,
                    limit: frame_domain::PageSize::new(2).expect("page size"),
                },
            )
            .await
            .expect("first page");
        assert_eq!(first.items.len(), 2);
        let second = repository
            .list_scoped(
                tenant,
                &PageRequest {
                    cursor: first.next_cursor,
                    limit: frame_domain::PageSize::new(2).expect("page size"),
                },
            )
            .await
            .expect("second page");
        assert_eq!(second.items.len(), 1);
        assert!(second.next_cursor.is_none());
    }

    #[tokio::test]
    async fn advanced_store_enforces_conditions_metadata_and_ranges() {
        let store = MemoryObjectStore::default();
        let key = ObjectKey::parse("tenants/t/videos/v/source/v1/source.mp4").expect("key");
        let checksum = ChecksumSha256::parse("c".repeat(64)).expect("checksum");
        let metadata = store
            .put_with_options(
                &key,
                b"0123456789".to_vec(),
                PutOptions {
                    content_type: ContentType::parse("video/mp4").expect("content type"),
                    checksum_sha256: Some(checksum.clone()),
                    condition: WriteCondition::IfAbsent,
                },
            )
            .await
            .expect("put");
        assert_eq!(metadata.size, ByteSize::new(10).expect("size"));
        assert_eq!(metadata.checksum_sha256, Some(checksum));
        assert_eq!(
            store.head(&key).await.expect("head"),
            Some(metadata.clone())
        );
        assert_eq!(
            store
                .put_with_options(
                    &key,
                    b"different".to_vec(),
                    PutOptions {
                        condition: WriteCondition::IfAbsent,
                        ..PutOptions::binary()
                    }
                )
                .await,
            Err(PortError::Conflict)
        );
        let read = store
            .get_range(&key, ByteRange::new(2, 6).expect("range"))
            .await
            .expect("range read")
            .expect("object");
        assert_eq!(read.bytes, b"2345");
        assert_eq!(read.range, ByteRange::new(2, 6).expect("range"));
        assert_eq!(
            store
                .delete_if_match(&key, Some(&EntityTag::parse("wrong-etag").expect("etag")))
                .await,
            Err(PortError::Conflict)
        );
        assert!(
            store
                .delete_if_match(&key, Some(&metadata.etag))
                .await
                .expect("conditional delete")
        );
        assert!(store.head(&key).await.expect("head").is_none());
    }

    #[tokio::test]
    async fn multipart_store_validates_order_etags_and_abort() {
        let store = MemoryObjectStore::default();
        assert!(store.capabilities().multipart);
        let key = ObjectKey::parse("tenants/t/videos/v/source/v1/source.webm").expect("key");
        let upload = store
            .begin_multipart(&key, PutOptions::binary())
            .await
            .expect("begin");
        let part_one = store
            .upload_part(
                upload.id,
                PartNumber::new(1).expect("part"),
                b"hello ".to_vec(),
                None,
            )
            .await
            .expect("part one");
        let part_two = store
            .upload_part(
                upload.id,
                PartNumber::new(2).expect("part"),
                b"world".to_vec(),
                None,
            )
            .await
            .expect("part two");
        assert_eq!(
            store
                .complete_multipart(
                    upload.id,
                    &[
                        CompletedPart {
                            part_number: part_two.part_number,
                            etag: part_two.etag.clone(),
                        },
                        CompletedPart {
                            part_number: part_one.part_number,
                            etag: part_one.etag.clone(),
                        },
                    ]
                )
                .await,
            Err(PortError::InvalidRequest(
                "multipart completion parts must be strictly ordered".into()
            ))
        );
        let metadata = store
            .complete_multipart(
                upload.id,
                &[
                    CompletedPart {
                        part_number: part_one.part_number,
                        etag: part_one.etag,
                    },
                    CompletedPart {
                        part_number: part_two.part_number,
                        etag: part_two.etag,
                    },
                ],
            )
            .await
            .expect("complete");
        assert_eq!(metadata.size, ByteSize::new(11).expect("size"));
        assert_eq!(
            store.get(&key).await.expect("get"),
            Some(b"hello world".to_vec())
        );

        let abandoned = store
            .begin_multipart(
                &ObjectKey::parse("tenants/t/videos/v/source/v2/source.webm").expect("key"),
                PutOptions::binary(),
            )
            .await
            .expect("begin");
        store.abort_multipart(abandoned.id).await.expect("abort");
        assert_eq!(
            store
                .upload_part(
                    abandoned.id,
                    PartNumber::new(1).expect("part"),
                    b"late".to_vec(),
                    None
                )
                .await,
            Err(PortError::NotFound)
        );
    }

    #[tokio::test]
    async fn session_repository_preserves_revocation() {
        let repository = MemorySessionRepository::default();
        let mut session = SessionRecord::new(
            UserId::new(),
            SecretDigest::parse_sha256("d".repeat(64)).expect("digest"),
            TimestampMillis::new(1).expect("time"),
            TimestampMillis::new(10).expect("time"),
            0,
        )
        .expect("session");
        repository
            .insert_session(session.clone())
            .await
            .expect("insert");
        session.revoke();
        repository
            .save_session(session.clone())
            .await
            .expect("save");
        assert_eq!(
            repository
                .get_session(session.id)
                .await
                .expect("get")
                .expect("session")
                .state,
            SessionState::Revoked
        );
    }

    #[tokio::test]
    async fn migration_repository_rejects_regressing_checkpoints_and_stale_cutover() {
        let repository = MemoryMigrationStateRepository::default();
        let checksum = ChecksumSha256::parse("e".repeat(64)).expect("checksum");
        let mut manifest = EtlManifest::new("mysql-binlog-100").expect("manifest");
        manifest
            .add_table(EtlTableManifest::new("users", 2, checksum).expect("table"))
            .expect("add table");
        manifest.start().expect("start");
        let run_id = manifest.run_id;
        repository
            .insert_manifest(manifest)
            .await
            .expect("manifest");
        let checkpoint = EtlCheckpoint::new(
            run_id,
            "users",
            CheckpointToken::parse("pk-200").expect("checkpoint"),
            2,
        )
        .expect("checkpoint");
        repository
            .save_checkpoint(checkpoint.clone())
            .await
            .expect("checkpoint");
        assert_eq!(
            repository
                .save_checkpoint(EtlCheckpoint {
                    processed_rows: 1,
                    ..checkpoint
                })
                .await,
            Err(PortError::Conflict)
        );

        let mut state = repository.cutover_state().await.expect("state");
        state
            .transition(CutoverPhase::ShadowRead, CutoverEvidence::default())
            .expect("transition");
        repository
            .compare_and_set_cutover(0, state)
            .await
            .expect("compare and set");
        assert_eq!(
            repository.compare_and_set_cutover(0, state).await,
            Err(PortError::Conflict)
        );
    }

    #[test]
    fn adapter_errors_redact_internal_details_from_display() {
        let error = PortError::Adapter("token=super-secret".into());
        assert_eq!(error.to_string(), "adapter failure");
        assert_eq!(format!("{error:?}"), "Adapter([redacted])");
        assert!(error.retryable());
    }
}
