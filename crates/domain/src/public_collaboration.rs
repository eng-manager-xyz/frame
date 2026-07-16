//! Provider-neutral public collaboration contracts.
//!
//! These wire types carry no database keys, provider identifiers, network
//! fingerprints, or authenticated tenant selectors.  A server first resolves
//! the share capability to its tenant/video authority, then validates the
//! bounded command here before persistence.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PUBLIC_COLLABORATION_SCHEMA_V1: &str = "frame.public-collaboration.v1";
pub const MAX_PUBLIC_COMMENT_BYTES_V1: usize = 4_000;
pub const MAX_TRANSCRIPT_SEGMENTS_V1: usize = 20_000;
pub const MAX_TRANSCRIPT_TEXT_BYTES_V1: usize = 1_000_000;
pub const MAX_PUBLIC_MEDIA_DURATION_MS_V1: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicCommentKindV1 {
    Text,
    Reaction,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCommentCommandV1 {
    pub idempotency_key: String,
    pub kind: PublicCommentKindV1,
    pub body: String,
    pub timeline_ms: Option<u64>,
}

impl PublicCommentCommandV1 {
    pub fn validate(&self, duration_ms: u64) -> Result<(), PublicCollaborationErrorV1> {
        validate_uuid(&self.idempotency_key)?;
        if duration_ms > MAX_PUBLIC_MEDIA_DURATION_MS_V1
            || self
                .timeline_ms
                .is_some_and(|position| position > duration_ms)
        {
            return Err(PublicCollaborationErrorV1::InvalidTimeline);
        }
        match self.kind {
            PublicCommentKindV1::Text => {
                if self.body.trim().is_empty()
                    || self.body.len() > MAX_PUBLIC_COMMENT_BYTES_V1
                    || self.body.chars().any(|character| {
                        character.is_control() && !matches!(character, '\n' | '\t')
                    })
                {
                    return Err(PublicCollaborationErrorV1::InvalidComment);
                }
            }
            PublicCommentKindV1::Reaction => {
                if self.body.is_empty()
                    || self.body.len() > 16
                    || self.body.chars().count() > 8
                    || self.body.chars().any(|character| {
                        character.is_ascii_alphanumeric()
                            || character.is_whitespace()
                            || character.is_control()
                    })
                {
                    return Err(PublicCollaborationErrorV1::InvalidComment);
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn payload_digest(&self) -> String {
        let mut digest = Sha256::new();
        append(&mut digest, b"frame-public-comment-v1");
        append(&mut digest, self.idempotency_key.as_bytes());
        append(
            &mut digest,
            match self.kind {
                PublicCommentKindV1::Text => b"text",
                PublicCommentKindV1::Reaction => b"reaction",
            },
        );
        append(&mut digest, self.body.as_bytes());
        append(
            &mut digest,
            &self.timeline_ms.unwrap_or(u64::MAX).to_be_bytes(),
        );
        format!("{:x}", digest.finalize())
    }
}

impl core::fmt::Debug for PublicCommentCommandV1 {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PublicCommentCommandV1")
            .field("idempotency_key", &"<redacted>")
            .field("kind", &self.kind)
            .field("body", &"<redacted>")
            .field("timeline_ms", &self.timeline_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicCommentStateV1 {
    Published,
    PendingModeration,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCommentV1 {
    pub id: String,
    pub kind: PublicCommentKindV1,
    pub body: String,
    pub timeline_ms: Option<u64>,
    pub state: PublicCommentStateV1,
    pub created_at_ms: u64,
}

impl core::fmt::Debug for PublicCommentV1 {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PublicCommentV1")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("body", &"<redacted>")
            .field("timeline_ms", &self.timeline_ms)
            .field("state", &self.state)
            .field("created_at_ms", &self.created_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCommentListV1 {
    pub schema_version: String,
    pub comments: Vec<PublicCommentV1>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicTranscriptSegmentV1 {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicTranscriptV1 {
    pub schema_version: String,
    pub language: String,
    pub duration_ms: u64,
    pub revision: u64,
    pub segments: Vec<PublicTranscriptSegmentV1>,
}

impl PublicTranscriptV1 {
    pub fn validate(&self) -> Result<(), PublicCollaborationErrorV1> {
        if self.schema_version != PUBLIC_COLLABORATION_SCHEMA_V1
            || self.revision == 0
            || !valid_language(&self.language)
            || self.duration_ms > MAX_PUBLIC_MEDIA_DURATION_MS_V1
        {
            return Err(PublicCollaborationErrorV1::InvalidTranscript);
        }
        if self.segments.len() > MAX_TRANSCRIPT_SEGMENTS_V1
            || self
                .segments
                .iter()
                .map(|segment| segment.text.len())
                .sum::<usize>()
                > MAX_TRANSCRIPT_TEXT_BYTES_V1
        {
            return Err(PublicCollaborationErrorV1::InvalidTranscript);
        }
        let mut previous_end = 0;
        for (index, segment) in self.segments.iter().enumerate() {
            if segment.start_ms >= segment.end_ms
                || segment.end_ms > self.duration_ms
                || (index > 0 && segment.start_ms < previous_end)
                || segment.text.trim().is_empty()
                || segment.text.len() > MAX_PUBLIC_COMMENT_BYTES_V1
                || segment
                    .text
                    .chars()
                    .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
                || segment.speaker.as_deref().is_some_and(|speaker| {
                    speaker.trim().is_empty()
                        || speaker.len() > 80
                        || speaker.chars().any(char::is_control)
                })
            {
                return Err(PublicCollaborationErrorV1::InvalidTranscript);
            }
            previous_end = segment.end_ms;
        }
        Ok(())
    }
}

impl core::fmt::Debug for PublicTranscriptV1 {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PublicTranscriptV1")
            .field("language", &self.language)
            .field("duration_ms", &self.duration_ms)
            .field("revision", &self.revision)
            .field("segment_count", &self.segments.len())
            .field("text", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicConsentDecisionV1 {
    Grant,
    Deny,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAnalyticsConsentCommandV1 {
    pub idempotency_key: String,
    pub policy_version: String,
    pub decision: PublicConsentDecisionV1,
}

impl PublicAnalyticsConsentCommandV1 {
    pub fn validate(&self) -> Result<(), PublicCollaborationErrorV1> {
        validate_uuid(&self.idempotency_key)?;
        validate_policy_version(&self.policy_version)
    }

    #[must_use]
    pub fn payload_digest(&self) -> String {
        let mut digest = Sha256::new();
        append(&mut digest, b"frame-public-analytics-consent-v1");
        append(&mut digest, self.idempotency_key.as_bytes());
        append(&mut digest, self.policy_version.as_bytes());
        append(
            &mut digest,
            match self.decision {
                PublicConsentDecisionV1::Grant => b"grant",
                PublicConsentDecisionV1::Deny => b"deny",
            },
        );
        format!("{:x}", digest.finalize())
    }
}

impl core::fmt::Debug for PublicAnalyticsConsentCommandV1 {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PublicAnalyticsConsentCommandV1")
            .field("idempotency_key", &"<redacted>")
            .field("policy_version", &self.policy_version)
            .field("decision", &self.decision)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAnalyticsConsentV1 {
    pub schema_version: String,
    pub policy_version: String,
    pub granted: bool,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicAnalyticsEventKindV1 {
    PlaybackStarted,
    PlaybackPaused,
    PlaybackCompleted,
    PlaybackError,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAnalyticsEventCommandV1 {
    pub idempotency_key: String,
    pub policy_version: String,
    pub sequence: u64,
    pub kind: PublicAnalyticsEventKindV1,
    pub position_ms: Option<u64>,
    pub occurred_at_ms: u64,
}

impl PublicAnalyticsEventCommandV1 {
    pub fn validate(
        &self,
        duration_ms: u64,
        now_ms: u64,
    ) -> Result<(), PublicCollaborationErrorV1> {
        validate_uuid(&self.idempotency_key)?;
        validate_policy_version(&self.policy_version)?;
        if self.sequence == 0
            || duration_ms > MAX_PUBLIC_MEDIA_DURATION_MS_V1
            || self
                .position_ms
                .is_some_and(|position| position > duration_ms)
            || self.occurred_at_ms > now_ms.saturating_add(30_000)
            || now_ms.saturating_sub(self.occurred_at_ms) > 5 * 60_000
        {
            return Err(PublicCollaborationErrorV1::InvalidAnalytics);
        }
        Ok(())
    }

    #[must_use]
    pub fn payload_digest(&self) -> String {
        let mut digest = Sha256::new();
        append(&mut digest, b"frame-public-analytics-event-v1");
        append(&mut digest, self.idempotency_key.as_bytes());
        append(&mut digest, self.policy_version.as_bytes());
        append(&mut digest, &self.sequence.to_be_bytes());
        append(
            &mut digest,
            match self.kind {
                PublicAnalyticsEventKindV1::PlaybackStarted => b"playback_started",
                PublicAnalyticsEventKindV1::PlaybackPaused => b"playback_paused",
                PublicAnalyticsEventKindV1::PlaybackCompleted => b"playback_completed",
                PublicAnalyticsEventKindV1::PlaybackError => b"playback_error",
            },
        );
        append(
            &mut digest,
            &self.position_ms.unwrap_or(u64::MAX).to_be_bytes(),
        );
        append(&mut digest, &self.occurred_at_ms.to_be_bytes());
        format!("{:x}", digest.finalize())
    }
}

impl core::fmt::Debug for PublicAnalyticsEventCommandV1 {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PublicAnalyticsEventCommandV1")
            .field("idempotency_key", &"<redacted>")
            .field("policy_version", &self.policy_version)
            .field("sequence", &self.sequence)
            .field("kind", &self.kind)
            .field("position_ms", &self.position_ms)
            .field("occurred_at_ms", &self.occurred_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicCollaborationGrantV1 {
    pub schema_version: String,
    pub token: String,
    pub expires_at_ms: u64,
    pub comments_enabled: bool,
    pub analytics_enabled: bool,
    pub analytics_policy_version: String,
}

impl core::fmt::Debug for PublicCollaborationGrantV1 {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PublicCollaborationGrantV1")
            .field("schema_version", &self.schema_version)
            .field("token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("comments_enabled", &self.comments_enabled)
            .field("analytics_enabled", &self.analytics_enabled)
            .field("analytics_policy_version", &self.analytics_policy_version)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCollaborationErrorV1 {
    InvalidIdentifier,
    InvalidComment,
    InvalidTimeline,
    InvalidTranscript,
    InvalidAnalytics,
}

fn validate_uuid(value: &str) -> Result<(), PublicCollaborationErrorV1> {
    let bytes = value.as_bytes();
    let valid = bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
        });
    if valid {
        Ok(())
    } else {
        Err(PublicCollaborationErrorV1::InvalidIdentifier)
    }
}

fn validate_policy_version(value: &str) -> Result<(), PublicCollaborationErrorV1> {
    if (3..=64).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
    {
        Ok(())
    } else {
        Err(PublicCollaborationErrorV1::InvalidAnalytics)
    }
}

fn valid_language(value: &str) -> bool {
    (2..=35).contains(&value.len())
        && value.split('-').all(|part| {
            (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn append(digest: &mut Sha256, value: &[u8]) {
    digest.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    digest.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPERATION: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900111";

    #[test]
    fn comments_are_bounded_and_payload_digest_is_stable() {
        let command = PublicCommentCommandV1 {
            idempotency_key: OPERATION.into(),
            kind: PublicCommentKindV1::Text,
            body: "A useful note".into(),
            timeline_ms: Some(500),
        };
        assert!(command.validate(1_000).is_ok());
        assert_eq!(command.payload_digest(), command.clone().payload_digest());
        let mut changed = command.clone();
        changed.body.push('!');
        assert_ne!(command.payload_digest(), changed.payload_digest());
        changed.body = "x".repeat(MAX_PUBLIC_COMMENT_BYTES_V1 + 1);
        assert_eq!(
            changed.validate(1_000),
            Err(PublicCollaborationErrorV1::InvalidComment)
        );
    }

    #[test]
    fn transcript_rejects_overlap_ordering_and_unbounded_content() {
        let document = PublicTranscriptV1 {
            schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
            language: "en-US".into(),
            duration_ms: 2_000,
            revision: 1,
            segments: vec![
                PublicTranscriptSegmentV1 {
                    start_ms: 0,
                    end_ms: 900,
                    speaker: Some("Host".into()),
                    text: "Hello".into(),
                },
                PublicTranscriptSegmentV1 {
                    start_ms: 1_000,
                    end_ms: 1_900,
                    speaker: None,
                    text: "World".into(),
                },
            ],
        };
        assert!(document.validate().is_ok());
        let mut invalid = document;
        invalid.segments[1].start_ms = 800;
        assert_eq!(
            invalid.validate(),
            Err(PublicCollaborationErrorV1::InvalidTranscript)
        );
    }

    #[test]
    fn public_grant_debug_never_discloses_the_bearer() {
        let grant = PublicCollaborationGrantV1 {
            schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
            token: "secret-capability-value".into(),
            expires_at_ms: 10,
            comments_enabled: true,
            analytics_enabled: true,
            analytics_policy_version: "analytics-v1".into(),
        };
        let rendered = format!("{grant:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("secret-capability-value"));
    }

    #[test]
    fn analytics_requires_current_consent_shape_and_bounded_clock() {
        let command = PublicAnalyticsEventCommandV1 {
            idempotency_key: OPERATION.into(),
            policy_version: "analytics-v1".into(),
            sequence: 1,
            kind: PublicAnalyticsEventKindV1::PlaybackStarted,
            position_ms: Some(0),
            occurred_at_ms: 10_000,
        };
        assert!(command.validate(1_000, 10_001).is_ok());
        let mut stale = command;
        stale.occurred_at_ms = 1;
        assert_eq!(
            stale.validate(1_000, 600_001),
            Err(PublicCollaborationErrorV1::InvalidAnalytics)
        );
    }
}
