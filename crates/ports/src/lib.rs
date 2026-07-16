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
    }
}
