use std::{collections::HashMap, sync::RwLock};

use async_trait::async_trait;
use frame_domain::{ObjectKey, Video, VideoId};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PortError {
    #[error("resource not found")]
    NotFound,
    #[error("resource already exists")]
    Conflict,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("unsupported capability: {0}")]
    Unsupported(String),
    #[error("adapter failure: {0}")]
    Adapter(String),
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

#[derive(Debug, Default)]
pub struct MemoryVideoRepository {
    videos: RwLock<HashMap<VideoId, Video>>,
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

#[derive(Debug, Default)]
pub struct MemoryObjectStore {
    objects: RwLock<HashMap<String, Vec<u8>>>,
}

#[derive(Debug)]
pub struct MemoryMediaTransformer {
    executor: MediaExecutor,
    supported: Vec<DerivativeKind>,
    results: RwLock<HashMap<String, (MediaTransformRequest, MediaTransformResult)>>,
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
            request.output.to_string(),
            (request.clone(), result.clone()),
        );
        Ok(result)
    }
}

#[async_trait]
impl ObjectStore for MemoryObjectStore {
    async fn put(&self, key: &ObjectKey, bytes: Vec<u8>) -> Result<(), PortError> {
        self.objects
            .write()
            .map_err(lock_error)?
            .insert(key.to_string(), bytes);
        Ok(())
    }

    async fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>, PortError> {
        Ok(self
            .objects
            .read()
            .map_err(lock_error)?
            .get(key.as_str())
            .cloned())
    }

    async fn delete(&self, key: &ObjectKey) -> Result<(), PortError> {
        self.objects
            .write()
            .map_err(lock_error)?
            .remove(key.as_str());
        Ok(())
    }
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> PortError {
    PortError::Adapter(format!("in-memory adapter lock poisoned: {error}"))
}

#[cfg(test)]
mod tests {
    use frame_domain::{ObjectKey, VideoState};

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
}
