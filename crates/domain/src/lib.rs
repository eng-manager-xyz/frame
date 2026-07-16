use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

mod api_workflow;
mod backfill;
mod business;
mod contracts;
mod identity;
mod multipart;
mod organization;
mod public_collaboration;
mod storage;
mod storage_governance;

pub use api_workflow::*;
pub use backfill::*;
pub use business::*;
pub use contracts::*;
pub use identity::*;
pub use multipart::*;
pub use organization::*;
pub use public_collaboration::*;
pub use storage::*;
pub use storage_governance::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VideoId(Uuid);

impl VideoId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for VideoId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for VideoId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for VideoId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoState {
    Pending,
    Uploading,
    Processing,
    Ready,
    Failed,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Video {
    pub id: VideoId,
    pub owner_id: String,
    pub title: String,
    pub state: VideoState,
    pub object_key: Option<ObjectKey>,
    pub created_at_ms: u64,
}

impl Video {
    pub fn transition_to(&mut self, next: VideoState) -> Result<(), TransitionError> {
        let allowed = matches!(
            (self.state, next),
            (
                VideoState::Pending,
                VideoState::Uploading | VideoState::Failed | VideoState::Deleted
            ) | (
                VideoState::Uploading,
                VideoState::Processing | VideoState::Failed | VideoState::Deleted
            ) | (
                VideoState::Processing,
                VideoState::Ready | VideoState::Failed | VideoState::Deleted
            ) | (
                VideoState::Ready,
                VideoState::Processing | VideoState::Deleted
            ) | (
                VideoState::Failed,
                VideoState::Uploading | VideoState::Processing | VideoState::Deleted
            )
        );

        if !allowed {
            return Err(TransitionError {
                from: self.state,
                to: next,
            });
        }

        self.state = next;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn parse(value: impl Into<String>) -> Result<Self, ObjectKeyError> {
        let value = value.into();
        let invalid_segment = value
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."));
        let invalid_character = !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'));

        if value.is_empty()
            || value.len() > 1_024
            || value.starts_with('/')
            || invalid_segment
            || invalid_character
        {
            return Err(ObjectKeyError);
        }

        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ObjectKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ObjectKey([redacted])")
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("invalid video-state transition from {from:?} to {to:?}")]
pub struct TransitionError {
    pub from: VideoState,
    pub to: VideoState,
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("object keys must be relative ASCII paths, at most 1024 bytes, with safe non-dot segments")]
pub struct ObjectKeyError;

#[cfg(test)]
mod tests {
    use super::*;

    fn video(state: VideoState) -> Video {
        Video {
            id: VideoId::new(),
            owner_id: "user-1".into(),
            title: "Demo".into(),
            state,
            object_key: None,
            created_at_ms: 0,
        }
    }

    #[test]
    fn accepts_expected_happy_path() {
        let mut video = video(VideoState::Pending);
        video
            .transition_to(VideoState::Uploading)
            .expect("pending to uploading");
        video
            .transition_to(VideoState::Processing)
            .expect("uploading to processing");
        video
            .transition_to(VideoState::Ready)
            .expect("processing to ready");
        assert_eq!(video.state, VideoState::Ready);
    }

    #[test]
    fn rejects_terminal_state_transition() {
        let mut video = video(VideoState::Deleted);
        let error = video
            .transition_to(VideoState::Ready)
            .expect_err("deleted is terminal");
        assert_eq!(error.from, VideoState::Deleted);
    }

    #[test]
    fn validates_object_keys() {
        assert!(ObjectKey::parse("users/u1/videos/v1/source.webm").is_ok());
        assert!(ObjectKey::parse("/absolute.webm").is_err());
        assert!(ObjectKey::parse("users/u1/../secret").is_err());
        assert!(ObjectKey::parse("users//source.webm").is_err());
        assert!(ObjectKey::parse("users/./source.webm").is_err());
        assert!(ObjectKey::parse("users\\u1\\source.webm").is_err());
        assert!(ObjectKey::parse("users/u1/source.webm?token=secret").is_err());
        let key = ObjectKey::parse("users/u1/videos/v1/source.webm").expect("valid key");
        assert_eq!(format!("{key:?}"), "ObjectKey([redacted])");
    }
}
