//! D1-backed public comment, transcript, and consented analytics authority.

use frame_domain::{
    PUBLIC_COLLABORATION_SCHEMA_V1, PublicAnalyticsConsentCommandV1, PublicAnalyticsConsentV1,
    PublicAnalyticsEventCommandV1, PublicAnalyticsEventKindV1, PublicCollaborationGrantV1,
    PublicCommentCommandV1, PublicCommentKindV1, PublicCommentListV1, PublicCommentStateV1,
    PublicCommentV1, PublicConsentDecisionV1, PublicTranscriptV1,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Error, Result, send::IntoSendFuture};

const GRANT_TTL_MS: i64 = 2 * 60 * 60 * 1_000;
const OPERATION_TTL_MS: i64 = 24 * 60 * 60 * 1_000;
const RATE_EVENT_TTL_MS: i64 = 2 * 60 * 1_000;
const MAX_LISTED_COMMENTS: usize = 200;
const MAX_PRUNE_ROWS: i64 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCollaborationFailure {
    Invalid,
    Unavailable,
    Conflict,
    RateLimited,
}

impl PublicCollaborationFailure {
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::Invalid => 400,
            Self::Unavailable => 404,
            Self::Conflict => 409,
            Self::RateLimited => 429,
        }
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Invalid => "invalid_request",
            Self::Unavailable => "not_found",
            Self::Conflict => "idempotency_conflict",
            Self::RateLimited => "rate_limited",
        }
    }
}

pub type PublicOutcome<T> = std::result::Result<T, PublicCollaborationFailure>;

#[derive(Debug, Deserialize)]
struct SharePolicyRow {
    organization_id: String,
    anonymous_comments_enabled: i64,
    analytics_enabled: i64,
    analytics_policy_version: String,
}

#[derive(Debug, Deserialize)]
struct GrantRow {
    duration_ms: i64,
    anonymous_comments_enabled: i64,
    comment_moderation: String,
    analytics_enabled: i64,
    analytics_policy_version: String,
    analytics_retention_days: i64,
    expires_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct StoredOperationRow {
    payload_digest: String,
    response_json: String,
}

#[derive(Debug, Deserialize)]
struct CommentRow {
    id: String,
    comment_kind: String,
    body: String,
    timeline_micros: Option<i64>,
    state: String,
    created_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct TranscriptRow {
    document_json: String,
    document_checksum: String,
}

#[derive(Debug, Deserialize)]
struct RevisionRow {
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct ConsentRow {
    state: String,
    expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAnalyticsReceiptV1 {
    pub schema_version: String,
    pub recorded: bool,
    pub duplicate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicTranscriptPublishReceiptV1 {
    pub schema_version: String,
    pub revision: u64,
    pub duplicate: bool,
}

pub async fn issue_grant(
    database: &D1Database,
    share_id: &str,
    now_ms: i64,
    correlation_id: &str,
) -> Result<PublicOutcome<PublicCollaborationGrantV1>> {
    let Some(policy) = share_policy(database, share_id).await? else {
        return Ok(Err(PublicCollaborationFailure::Unavailable));
    };
    let expires_at_ms = now_ms
        .checked_add(GRANT_TTL_MS)
        .ok_or_else(|| Error::RustError("public grant expiry overflowed".into()))?;
    let token = random_token()?;
    let token_digest = digest(&token);
    let comments_enabled = policy.anonymous_comments_enabled == 1;
    let analytics_enabled = policy.analytics_enabled == 1;
    let result = batch(
        database,
        vec![
            grant_rate_statement(database, share_id, now_ms)?,
            database
                .prepare(
                    "INSERT INTO public_collaboration_grants_v1(\
                       token_digest,share_id,organization_id,comments_enabled,analytics_enabled,\
                       analytics_policy_version,issued_at_ms,expires_at_ms,revoked_at_ms\
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL)",
                )
                .bind(&[
                    JsValue::from_str(&token_digest),
                    JsValue::from_str(share_id),
                    JsValue::from_str(&policy.organization_id),
                    JsValue::from_f64(bool_number(comments_enabled)),
                    JsValue::from_f64(bool_number(analytics_enabled)),
                    JsValue::from_str(&policy.analytics_policy_version),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_f64(expires_at_ms as f64),
                ])?,
            audit_statement(
                database,
                share_id,
                &token_digest,
                "grant_issued",
                correlation_id,
                now_ms,
            )?,
        ],
    )
    .await;
    if let Err(error) = result {
        return if rate_limited(&error) {
            Ok(Err(PublicCollaborationFailure::RateLimited))
        } else {
            Err(error)
        };
    }
    Ok(Ok(PublicCollaborationGrantV1 {
        schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
        token,
        expires_at_ms: checked_u64(expires_at_ms)?,
        comments_enabled,
        analytics_enabled,
        analytics_policy_version: policy.analytics_policy_version,
    }))
}

pub async fn list_comments(
    database: &D1Database,
    share_id: &str,
) -> Result<PublicOutcome<PublicCommentListV1>> {
    if share_policy(database, share_id).await?.is_none() {
        return Ok(Err(PublicCollaborationFailure::Unavailable));
    }
    let result = database
        .prepare(
            "SELECT c.id,c.comment_kind,c.body,c.timeline_micros,m.state,c.created_at_ms \
             FROM comments c JOIN public_comment_moderation_v1 m ON m.comment_id=c.id \
             JOIN videos v ON v.id=c.video_id AND v.organization_id=c.organization_id \
             WHERE c.video_id=?1 AND c.deleted_at_ms IS NULL AND m.state='published' \
               AND v.privacy='public' AND v.deleted_at_ms IS NULL \
             ORDER BY c.created_at_ms,c.id LIMIT 201",
        )
        .bind(&[JsValue::from_str(share_id)])?
        .all()
        .into_send()
        .await?;
    if !result.success() {
        return Err(Error::RustError("public comment list failed".into()));
    }
    let rows = result.results::<CommentRow>()?;
    if rows.len() > MAX_LISTED_COMMENTS {
        return Err(Error::RustError(
            "public comment list exceeded its bound".into(),
        ));
    }
    Ok(Ok(PublicCommentListV1 {
        schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
        comments: rows
            .into_iter()
            .map(decode_comment)
            .collect::<Result<Vec<_>>>()?,
    }))
}

pub async fn create_comment(
    database: &D1Database,
    share_id: &str,
    raw_token: &str,
    command: &PublicCommentCommandV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<PublicOutcome<PublicCommentV1>> {
    let token_digest = digest(raw_token);
    let Some(grant) = live_grant(database, share_id, &token_digest, now_ms).await? else {
        return Ok(Err(PublicCollaborationFailure::Unavailable));
    };
    if grant.anonymous_comments_enabled != 1
        || command.validate(checked_u64(grant.duration_ms)?).is_err()
    {
        return Ok(Err(PublicCollaborationFailure::Invalid));
    }
    let payload_digest = command.payload_digest();
    if let Some(stored) =
        stored_comment(database, &command.idempotency_key, &token_digest, share_id).await?
    {
        return decode_replay(stored, &payload_digest);
    }

    let comment_id = Uuid::now_v7().to_string();
    let state = match grant.comment_moderation.as_str() {
        "publish" => PublicCommentStateV1::Published,
        "pre_moderate" => PublicCommentStateV1::PendingModeration,
        _ => return Err(Error::RustError("public comment policy is corrupt".into())),
    };
    let response = PublicCommentV1 {
        id: comment_id.clone(),
        kind: command.kind,
        body: command.body.clone(),
        timeline_ms: command.timeline_ms,
        state,
        created_at_ms: checked_u64(now_ms)?,
    };
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("public comment response encoding failed".into()))?;
    let result = batch(
        database,
        vec![
            database
                .prepare(
                    "INSERT INTO comments(\
                       id,video_id,parent_comment_id,author_user_id,anonymous_author_digest,body,\
                       created_at_ms,updated_at_ms,deleted_at_ms,revision,organization_id,\
                       last_operation_id,comment_kind,timeline_micros\
                     ) SELECT ?1,g.share_id,NULL,NULL,g.token_digest,?2,?3,?3,NULL,0,\
                              g.organization_id,?4,?5,?6 \
                       FROM public_collaboration_grants_v1 g JOIN videos v ON v.id=g.share_id \
                       WHERE g.token_digest=?7 AND g.share_id=?8 AND g.expires_at_ms>?3 \
                         AND g.revoked_at_ms IS NULL AND g.comments_enabled=1 \
                         AND v.privacy='public' AND v.comments_enabled=1 \
                         AND v.deleted_at_ms IS NULL",
                )
                .bind(&[
                    JsValue::from_str(&comment_id),
                    JsValue::from_str(&command.body),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_str(&command.idempotency_key),
                    JsValue::from_str(comment_kind(command.kind)),
                    timeline_micros(command.timeline_ms),
                    JsValue::from_str(&token_digest),
                    JsValue::from_str(share_id),
                ])?,
            database
                .prepare(
                    "INSERT INTO public_comment_moderation_v1(\
                       comment_id,share_id,state,decided_at_ms,revision\
                     ) VALUES (?1,?2,?3,?4,0)",
                )
                .bind(&[
                    JsValue::from_str(&comment_id),
                    JsValue::from_str(share_id),
                    JsValue::from_str(comment_state(state)),
                    if state == PublicCommentStateV1::Published {
                        JsValue::from_f64(now_ms as f64)
                    } else {
                        JsValue::NULL
                    },
                ])?,
            database
                .prepare(
                    "INSERT INTO public_comment_operations_v1(\
                       operation_id,token_digest,share_id,payload_digest,comment_id,response_json,\
                       created_at_ms,expires_at_ms\
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                )
                .bind(&[
                    JsValue::from_str(&command.idempotency_key),
                    JsValue::from_str(&token_digest),
                    JsValue::from_str(share_id),
                    JsValue::from_str(&payload_digest),
                    JsValue::from_str(&comment_id),
                    JsValue::from_str(&response_json),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_f64(now_ms.saturating_add(OPERATION_TTL_MS) as f64),
                ])?,
            rate_statement(database, &token_digest, share_id, "comment", now_ms)?,
            audit_statement(
                database,
                share_id,
                &token_digest,
                "comment_created",
                correlation_id,
                now_ms,
            )?,
        ],
    )
    .await;
    match result {
        Ok(()) => Ok(Ok(response)),
        Err(error) if rate_limited(&error) => Ok(Err(PublicCollaborationFailure::RateLimited)),
        Err(_) => {
            if let Some(stored) =
                stored_comment(database, &command.idempotency_key, &token_digest, share_id).await?
            {
                decode_replay(stored, &payload_digest)
            } else {
                Ok(Err(PublicCollaborationFailure::Conflict))
            }
        }
    }
}

pub async fn transcript(
    database: &D1Database,
    share_id: &str,
) -> Result<PublicOutcome<PublicTranscriptV1>> {
    let row = database
        .prepare(
            "SELECT t.document_json,t.document_checksum FROM public_transcripts_v1 t \
             JOIN videos v ON v.id=t.share_id \
             WHERE t.share_id=?1 AND t.is_current=1 AND v.privacy='public' \
               AND v.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(share_id)])?
        .first::<TranscriptRow>(None)
        .into_send()
        .await?;
    let Some(row) = row else {
        return Ok(Err(PublicCollaborationFailure::Unavailable));
    };
    if digest(&row.document_json) != row.document_checksum {
        return Err(Error::RustError(
            "public transcript checksum mismatch".into(),
        ));
    }
    let document = serde_json::from_str::<PublicTranscriptV1>(&row.document_json)
        .map_err(|_| Error::RustError("public transcript is corrupt".into()))?;
    document
        .validate()
        .map_err(|_| Error::RustError("public transcript contract is corrupt".into()))?;
    Ok(Ok(document))
}

#[allow(clippy::too_many_arguments)]
pub async fn publish_transcript(
    database: &D1Database,
    share_id: &str,
    organization_id: &str,
    publisher_id: &str,
    document: &PublicTranscriptV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<PublicOutcome<PublicTranscriptPublishReceiptV1>> {
    if document.validate().is_err() {
        return Ok(Err(PublicCollaborationFailure::Invalid));
    }
    let current = database
        .prepare(
            "SELECT revision FROM public_transcripts_v1 \
             WHERE share_id=?1 AND is_current=1 LIMIT 1",
        )
        .bind(&[JsValue::from_str(share_id)])?
        .first::<RevisionRow>(None)
        .into_send()
        .await?;
    let expected = current.as_ref().map_or(1_i64, |row| row.revision + 1);
    let revision = i64::try_from(document.revision)
        .map_err(|_| Error::RustError("transcript revision overflowed".into()))?;
    let encoded = serde_json::to_string(document)
        .map_err(|_| Error::RustError("transcript encoding failed".into()))?;
    let checksum = digest(&encoded);
    if revision < expected {
        let existing = database
            .prepare(
                "SELECT document_json,document_checksum FROM public_transcripts_v1 \
                 WHERE share_id=?1 AND revision=?2 LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(share_id),
                JsValue::from_f64(revision as f64),
            ])?
            .first::<TranscriptRow>(None)
            .into_send()
            .await?;
        return Ok(
            if existing.is_some_and(|row| row.document_checksum == checksum) {
                Ok(PublicTranscriptPublishReceiptV1 {
                    schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
                    revision: document.revision,
                    duplicate: true,
                })
            } else {
                Err(PublicCollaborationFailure::Conflict)
            },
        );
    }
    if revision != expected {
        return Ok(Err(PublicCollaborationFailure::Conflict));
    }

    let mut statements = Vec::new();
    if current.is_some() {
        statements.push(
            database
                .prepare(
                    "UPDATE public_transcripts_v1 SET is_current=0 \
                     WHERE share_id=?1 AND revision=?2 AND is_current=1",
                )
                .bind(&[
                    JsValue::from_str(share_id),
                    JsValue::from_f64((revision - 1) as f64),
                ])?,
        );
    }
    statements.push(
        database
            .prepare(
                "INSERT INTO public_transcripts_v1(\
                   share_id,organization_id,revision,language,duration_ms,document_json,document_checksum,\
                   is_current,published_at_ms,published_by_user_id\
                 ) SELECT ?1,?9,?2,?3,?4,?5,?6,1,?7,?8 FROM videos v \
                   JOIN organization_members m ON m.organization_id=v.organization_id \
                    AND m.user_id=?8 AND m.state='active' \
                   WHERE v.id=?1 AND v.organization_id=?9 AND v.deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(share_id),
                JsValue::from_f64(revision as f64),
                JsValue::from_str(&document.language),
                JsValue::from_f64(document.duration_ms as f64),
                JsValue::from_str(&encoded),
                JsValue::from_str(&checksum),
                JsValue::from_f64(now_ms as f64),
                JsValue::from_str(publisher_id),
                JsValue::from_str(organization_id),
            ])?,
    );
    statements.push(audit_statement(
        database,
        share_id,
        &digest(publisher_id),
        "transcript_published",
        correlation_id,
        now_ms,
    )?);
    batch(database, statements).await?;
    Ok(Ok(PublicTranscriptPublishReceiptV1 {
        schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
        revision: document.revision,
        duplicate: false,
    }))
}

pub async fn set_analytics_consent(
    database: &D1Database,
    share_id: &str,
    raw_token: &str,
    command: &PublicAnalyticsConsentCommandV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<PublicOutcome<PublicAnalyticsConsentV1>> {
    let token_digest = digest(raw_token);
    let Some(grant) = live_grant(database, share_id, &token_digest, now_ms).await? else {
        return Ok(Err(PublicCollaborationFailure::Unavailable));
    };
    if grant.analytics_enabled != 1
        || command.validate().is_err()
        || command.policy_version != grant.analytics_policy_version
    {
        return Ok(Err(PublicCollaborationFailure::Invalid));
    }
    let payload_digest = command.payload_digest();
    if let Some(stored) =
        stored_consent(database, &command.idempotency_key, &token_digest, share_id).await?
    {
        return decode_replay(stored, &payload_digest);
    }
    let response = PublicAnalyticsConsentV1 {
        schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
        policy_version: command.policy_version.clone(),
        granted: command.decision == PublicConsentDecisionV1::Grant,
        expires_at_ms: checked_u64(grant.expires_at_ms)?,
    };
    let response_json = serde_json::to_string(&response)
        .map_err(|_| Error::RustError("consent response encoding failed".into()))?;
    let result = batch(
        database,
        vec![
            database
                .prepare(
                    "INSERT INTO public_analytics_consents_v1(\
                       token_digest,share_id,policy_version,state,granted_at_ms,expires_at_ms,\
                       revision,last_operation_id\
                     ) VALUES (?1,?2,?3,?4,?5,?6,1,?7) \
                     ON CONFLICT(token_digest,share_id,policy_version) DO UPDATE SET \
                       state=excluded.state,granted_at_ms=excluded.granted_at_ms,\
                       expires_at_ms=excluded.expires_at_ms,revision=revision+1,\
                       last_operation_id=excluded.last_operation_id",
                )
                .bind(&[
                    JsValue::from_str(&token_digest),
                    JsValue::from_str(share_id),
                    JsValue::from_str(&command.policy_version),
                    JsValue::from_str(if response.granted {
                        "granted"
                    } else {
                        "denied"
                    }),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_f64(grant.expires_at_ms as f64),
                    JsValue::from_str(&command.idempotency_key),
                ])?,
            database
                .prepare(
                    "INSERT INTO public_analytics_consent_operations_v1(\
                       operation_id,token_digest,share_id,payload_digest,response_json,\
                       created_at_ms,expires_at_ms\
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                )
                .bind(&[
                    JsValue::from_str(&command.idempotency_key),
                    JsValue::from_str(&token_digest),
                    JsValue::from_str(share_id),
                    JsValue::from_str(&payload_digest),
                    JsValue::from_str(&response_json),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_f64(now_ms.saturating_add(OPERATION_TTL_MS) as f64),
                ])?,
            audit_statement(
                database,
                share_id,
                &token_digest,
                "analytics_consent",
                correlation_id,
                now_ms,
            )?,
        ],
    )
    .await;
    match result {
        Ok(()) => Ok(Ok(response)),
        Err(_) => {
            if let Some(stored) =
                stored_consent(database, &command.idempotency_key, &token_digest, share_id).await?
            {
                decode_replay(stored, &payload_digest)
            } else {
                Ok(Err(PublicCollaborationFailure::Conflict))
            }
        }
    }
}

pub async fn record_analytics(
    database: &D1Database,
    share_id: &str,
    raw_token: &str,
    command: &PublicAnalyticsEventCommandV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<PublicOutcome<PublicAnalyticsReceiptV1>> {
    let token_digest = digest(raw_token);
    let Some(grant) = live_grant(database, share_id, &token_digest, now_ms).await? else {
        return Ok(Err(PublicCollaborationFailure::Unavailable));
    };
    if grant.analytics_enabled != 1
        || command
            .validate(checked_u64(grant.duration_ms)?, checked_u64(now_ms)?)
            .is_err()
        || command.policy_version != grant.analytics_policy_version
    {
        return Ok(Err(PublicCollaborationFailure::Invalid));
    }
    let consent = database
        .prepare(
            "SELECT state,expires_at_ms FROM public_analytics_consents_v1 \
             WHERE token_digest=?1 AND share_id=?2 AND policy_version=?3 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(&token_digest),
            JsValue::from_str(share_id),
            JsValue::from_str(&command.policy_version),
        ])?
        .first::<ConsentRow>(None)
        .into_send()
        .await?;
    if !consent.is_some_and(|row| row.state == "granted" && row.expires_at_ms > now_ms) {
        return Ok(Ok(PublicAnalyticsReceiptV1 {
            schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
            recorded: false,
            duplicate: false,
        }));
    }
    let payload_digest = command.payload_digest();
    if let Some(stored) =
        stored_analytics(database, &command.idempotency_key, &token_digest, share_id).await?
    {
        return analytics_replay(stored, &payload_digest);
    }
    let retention_ms = grant
        .analytics_retention_days
        .checked_mul(24 * 60 * 60 * 1_000)
        .ok_or_else(|| Error::RustError("analytics retention overflowed".into()))?;
    let result = batch(
        database,
        vec![
            database
                .prepare(
                    "INSERT INTO public_analytics_events_v1(\
                       operation_id,token_digest,share_id,policy_version,payload_digest,sequence,\
                       kind,position_ms,occurred_at_ms,recorded_at_ms,expires_at_ms\
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                )
                .bind(&[
                    JsValue::from_str(&command.idempotency_key),
                    JsValue::from_str(&token_digest),
                    JsValue::from_str(share_id),
                    JsValue::from_str(&command.policy_version),
                    JsValue::from_str(&payload_digest),
                    JsValue::from_f64(command.sequence as f64),
                    JsValue::from_str(analytics_kind(command.kind)),
                    command
                        .position_ms
                        .map_or(JsValue::NULL, |value| JsValue::from_f64(value as f64)),
                    JsValue::from_f64(command.occurred_at_ms as f64),
                    JsValue::from_f64(now_ms as f64),
                    JsValue::from_f64(now_ms.saturating_add(retention_ms) as f64),
                ])?,
            rate_statement(database, &token_digest, share_id, "analytics", now_ms)?,
            audit_statement(
                database,
                share_id,
                &token_digest,
                "analytics_recorded",
                correlation_id,
                now_ms,
            )?,
        ],
    )
    .await;
    match result {
        Ok(()) => Ok(Ok(PublicAnalyticsReceiptV1 {
            schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
            recorded: true,
            duplicate: false,
        })),
        Err(error) if rate_limited(&error) => Ok(Err(PublicCollaborationFailure::RateLimited)),
        Err(_) => {
            if let Some(stored) =
                stored_analytics(database, &command.idempotency_key, &token_digest, share_id)
                    .await?
            {
                analytics_replay(stored, &payload_digest)
            } else {
                Ok(Err(PublicCollaborationFailure::Conflict))
            }
        }
    }
}

pub async fn prune_expired(database: &D1Database, now_ms: i64) -> Result<()> {
    batch(
        database,
        vec![
            prune_statement(database, "analytics_events", now_ms)?,
            prune_statement(database, "grant_rate_events", now_ms)?,
            prune_statement(database, "rate_events", now_ms)?,
            prune_statement(database, "comment_operations", now_ms)?,
            prune_statement(database, "consent_operations", now_ms)?,
            prune_statement(database, "consents", now_ms)?,
            prune_statement(database, "grants", now_ms)?,
        ],
    )
    .await
}

async fn share_policy(database: &D1Database, share_id: &str) -> Result<Option<SharePolicyRow>> {
    database
        .prepare(
            "SELECT v.organization_id,\
                    p.anonymous_comments_enabled,p.analytics_enabled,p.analytics_policy_version \
             FROM videos v JOIN organizations o ON o.id=v.organization_id \
             JOIN public_collaboration_policies_v1 p ON p.organization_id=v.organization_id \
             WHERE v.id=?1 AND v.privacy='public' AND v.state='ready' \
               AND v.deleted_at_ms IS NULL AND o.status='active' LIMIT 1",
        )
        .bind(&[JsValue::from_str(share_id)])?
        .first::<SharePolicyRow>(None)
        .into_send()
        .await
}

async fn live_grant(
    database: &D1Database,
    share_id: &str,
    token_digest: &str,
    now_ms: i64,
) -> Result<Option<GrantRow>> {
    database
        .prepare(
            "SELECT COALESCE(v.duration_ms,0) AS duration_ms,p.anonymous_comments_enabled,\
                    p.comment_moderation,p.analytics_enabled,p.analytics_policy_version,\
                    p.analytics_retention_days,g.expires_at_ms \
             FROM public_collaboration_grants_v1 g JOIN videos v ON v.id=g.share_id \
             JOIN organizations o ON o.id=g.organization_id \
             JOIN public_collaboration_policies_v1 p ON p.organization_id=g.organization_id \
             WHERE g.token_digest=?1 AND g.share_id=?2 AND g.expires_at_ms>?3 \
               AND g.revoked_at_ms IS NULL AND v.privacy='public' AND v.state='ready' \
               AND v.deleted_at_ms IS NULL AND o.status='active' \
               AND g.analytics_policy_version=p.analytics_policy_version LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(token_digest),
            JsValue::from_str(share_id),
            JsValue::from_f64(now_ms as f64),
        ])?
        .first::<GrantRow>(None)
        .into_send()
        .await
}

async fn stored_comment(
    database: &D1Database,
    operation_id: &str,
    token_digest: &str,
    share_id: &str,
) -> Result<Option<StoredOperationRow>> {
    stored_operation(database, "comment", operation_id, token_digest, share_id).await
}

async fn stored_consent(
    database: &D1Database,
    operation_id: &str,
    token_digest: &str,
    share_id: &str,
) -> Result<Option<StoredOperationRow>> {
    stored_operation(database, "consent", operation_id, token_digest, share_id).await
}

async fn stored_analytics(
    database: &D1Database,
    operation_id: &str,
    token_digest: &str,
    share_id: &str,
) -> Result<Option<StoredOperationRow>> {
    stored_operation(database, "analytics", operation_id, token_digest, share_id).await
}

async fn stored_operation(
    database: &D1Database,
    kind: &str,
    operation_id: &str,
    token_digest: &str,
    share_id: &str,
) -> Result<Option<StoredOperationRow>> {
    let sql = match kind {
        "comment" => {
            "SELECT payload_digest,response_json FROM public_comment_operations_v1 \
             WHERE operation_id=?1 AND token_digest=?2 AND share_id=?3 LIMIT 1"
        }
        "consent" => {
            "SELECT payload_digest,response_json FROM public_analytics_consent_operations_v1 \
             WHERE operation_id=?1 AND token_digest=?2 AND share_id=?3 LIMIT 1"
        }
        "analytics" => {
            "SELECT payload_digest,'{}' AS response_json FROM public_analytics_events_v1 \
             WHERE operation_id=?1 AND token_digest=?2 AND share_id=?3 LIMIT 1"
        }
        _ => return Err(Error::RustError("unsupported operation kind".into())),
    };
    database
        .prepare(sql)
        .bind(&[
            JsValue::from_str(operation_id),
            JsValue::from_str(token_digest),
            JsValue::from_str(share_id),
        ])?
        .first::<StoredOperationRow>(None)
        .into_send()
        .await
}

fn decode_replay<T: DeserializeOwned>(
    stored: StoredOperationRow,
    payload_digest: &str,
) -> Result<PublicOutcome<T>> {
    if stored.payload_digest != payload_digest {
        return Ok(Err(PublicCollaborationFailure::Conflict));
    }
    serde_json::from_str(&stored.response_json)
        .map(Ok)
        .map_err(|_| Error::RustError("stored public operation is corrupt".into()))
}

fn analytics_replay(
    stored: StoredOperationRow,
    payload_digest: &str,
) -> Result<PublicOutcome<PublicAnalyticsReceiptV1>> {
    Ok(if stored.payload_digest == payload_digest {
        Ok(PublicAnalyticsReceiptV1 {
            schema_version: PUBLIC_COLLABORATION_SCHEMA_V1.into(),
            recorded: true,
            duplicate: true,
        })
    } else {
        Err(PublicCollaborationFailure::Conflict)
    })
}

fn decode_comment(row: CommentRow) -> Result<PublicCommentV1> {
    let timeline_ms = match row.timeline_micros {
        None => None,
        Some(value) if value >= 0 && value % 1_000 == 0 => Some(checked_u64(value)? / 1_000),
        Some(_) => {
            return Err(Error::RustError(
                "public comment timeline is corrupt".into(),
            ));
        }
    };
    Ok(PublicCommentV1 {
        id: row.id,
        kind: match row.comment_kind.as_str() {
            "text" => PublicCommentKindV1::Text,
            "emoji" => PublicCommentKindV1::Reaction,
            _ => return Err(Error::RustError("public comment kind is corrupt".into())),
        },
        body: row.body,
        timeline_ms,
        state: match row.state.as_str() {
            "published" => PublicCommentStateV1::Published,
            "pending_moderation" => PublicCommentStateV1::PendingModeration,
            _ => return Err(Error::RustError("public comment state is corrupt".into())),
        },
        created_at_ms: checked_u64(row.created_at_ms)?,
    })
}

fn rate_statement(
    database: &D1Database,
    token_digest: &str,
    share_id: &str,
    action: &str,
    now_ms: i64,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "INSERT INTO public_collaboration_rate_events_v1(\
               id,token_digest,share_id,action,accepted_at_ms,expires_at_ms\
             ) VALUES (?1,?2,?3,?4,?5,?6)",
        )
        .bind(&[
            JsValue::from_str(&Uuid::now_v7().to_string()),
            JsValue::from_str(token_digest),
            JsValue::from_str(share_id),
            JsValue::from_str(action),
            JsValue::from_f64(now_ms as f64),
            JsValue::from_f64(now_ms.saturating_add(RATE_EVENT_TTL_MS) as f64),
        ])
}

fn grant_rate_statement(
    database: &D1Database,
    share_id: &str,
    now_ms: i64,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "INSERT INTO public_collaboration_grant_rate_v1(\
               id,share_id,accepted_at_ms,expires_at_ms\
             ) VALUES (?1,?2,?3,?4)",
        )
        .bind(&[
            JsValue::from_str(&Uuid::now_v7().to_string()),
            JsValue::from_str(share_id),
            JsValue::from_f64(now_ms as f64),
            JsValue::from_f64(now_ms.saturating_add(RATE_EVENT_TTL_MS) as f64),
        ])
}

fn audit_statement(
    database: &D1Database,
    share_id: &str,
    token_digest: &str,
    action: &str,
    correlation_id: &str,
    now_ms: i64,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "INSERT INTO public_collaboration_audit_v1(\
               id,share_id,token_digest,action,outcome,correlation_id,occurred_at_ms\
             ) VALUES (?1,?2,?3,?4,'applied',?5,?6)",
        )
        .bind(&[
            JsValue::from_str(&Uuid::now_v7().to_string()),
            JsValue::from_str(share_id),
            JsValue::from_str(token_digest),
            JsValue::from_str(action),
            JsValue::from_str(correlation_id),
            JsValue::from_f64(now_ms as f64),
        ])
}

fn prune_statement(database: &D1Database, kind: &str, now_ms: i64) -> Result<D1PreparedStatement> {
    let sql = match kind {
        "analytics_events" => {
            "DELETE FROM public_analytics_events_v1 WHERE operation_id IN (SELECT operation_id FROM public_analytics_events_v1 WHERE expires_at_ms<?1 ORDER BY expires_at_ms,operation_id LIMIT ?2)"
        }
        "grant_rate_events" => {
            "DELETE FROM public_collaboration_grant_rate_v1 WHERE id IN (SELECT id FROM public_collaboration_grant_rate_v1 WHERE expires_at_ms<?1 ORDER BY expires_at_ms,id LIMIT ?2)"
        }
        "rate_events" => {
            "DELETE FROM public_collaboration_rate_events_v1 WHERE id IN (SELECT id FROM public_collaboration_rate_events_v1 WHERE expires_at_ms<?1 ORDER BY expires_at_ms,id LIMIT ?2)"
        }
        "comment_operations" => {
            "DELETE FROM public_comment_operations_v1 WHERE operation_id IN (SELECT operation_id FROM public_comment_operations_v1 WHERE expires_at_ms<?1 ORDER BY expires_at_ms,operation_id LIMIT ?2)"
        }
        "consent_operations" => {
            "DELETE FROM public_analytics_consent_operations_v1 WHERE operation_id IN (SELECT operation_id FROM public_analytics_consent_operations_v1 WHERE expires_at_ms<?1 ORDER BY expires_at_ms,operation_id LIMIT ?2)"
        }
        "consents" => {
            "DELETE FROM public_analytics_consents_v1 WHERE rowid IN (SELECT rowid FROM public_analytics_consents_v1 WHERE expires_at_ms<?1 ORDER BY expires_at_ms,token_digest LIMIT ?2)"
        }
        "grants" => {
            "DELETE FROM public_collaboration_grants_v1 WHERE token_digest IN (SELECT token_digest FROM public_collaboration_grants_v1 WHERE expires_at_ms<?1 ORDER BY expires_at_ms,token_digest LIMIT ?2)"
        }
        _ => return Err(Error::RustError("unsupported retention kind".into())),
    };
    database.prepare(sql).bind(&[
        JsValue::from_f64(now_ms as f64),
        JsValue::from_f64(MAX_PRUNE_ROWS as f64),
    ])
}

async fn batch(database: &D1Database, statements: Vec<D1PreparedStatement>) -> Result<()> {
    let results = database.batch(statements).into_send().await?;
    if results.is_empty() || results.iter().any(|result| !result.success()) {
        return Err(Error::RustError("public collaboration batch failed".into()));
    }
    Ok(())
}

fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| Error::RustError("public capability entropy unavailable".into()))?;
    Ok(hex(&bytes))
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    const LOWER: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(LOWER[usize::from(byte >> 4)]));
        output.push(char::from(LOWER[usize::from(byte & 0x0f)]));
    }
    output
}

fn checked_u64(value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| Error::RustError("public collaboration value is corrupt".into()))
}

const fn bool_number(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}

const fn comment_kind(kind: PublicCommentKindV1) -> &'static str {
    match kind {
        PublicCommentKindV1::Text => "text",
        PublicCommentKindV1::Reaction => "emoji",
    }
}

const fn comment_state(state: PublicCommentStateV1) -> &'static str {
    match state {
        PublicCommentStateV1::Published => "published",
        PublicCommentStateV1::PendingModeration => "pending_moderation",
    }
}

const fn analytics_kind(kind: PublicAnalyticsEventKindV1) -> &'static str {
    match kind {
        PublicAnalyticsEventKindV1::PlaybackStarted => "playback_started",
        PublicAnalyticsEventKindV1::PlaybackPaused => "playback_paused",
        PublicAnalyticsEventKindV1::PlaybackCompleted => "playback_completed",
        PublicAnalyticsEventKindV1::PlaybackError => "playback_error",
    }
}

fn timeline_micros(value: Option<u64>) -> JsValue {
    value.map_or(JsValue::NULL, |value| {
        JsValue::from_f64(value.saturating_mul(1_000) as f64)
    })
}

fn rate_limited(error: &Error) -> bool {
    let error = error.to_string();
    error.contains("frame_public_collaboration_rate_limited_v1")
        || error.contains("frame_public_collaboration_grant_rate_limited_v1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_tokens_are_random_shaped_and_only_digest_is_stable() {
        let token = random_token().expect("token");
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(digest(&token).len(), 64);
    }

    #[test]
    fn public_failures_are_closed_and_non_disclosing() {
        assert_eq!(PublicCollaborationFailure::Unavailable.status(), 404);
        assert_eq!(PublicCollaborationFailure::Unavailable.code(), "not_found");
        assert_eq!(PublicCollaborationFailure::RateLimited.status(), 429);
    }

    #[test]
    fn retention_and_capability_lifetimes_are_bounded() {
        assert_eq!(MAX_PRUNE_ROWS, 500);
        assert_eq!(GRANT_TTL_MS, 7_200_000);
        assert_eq!(OPERATION_TTL_MS, 86_400_000);
    }
}
