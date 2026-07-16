//! Token-gated, loopback-only repository conformance surface.
//!
//! After request-ID normalization and raw-target parsing, routing rejects this
//! exact path in production before route-specific Host, method, token, or body
//! handling. Inputs are a closed scenario enum; callers cannot supply
//! identifiers, SQL, titles, revisions, or fault statements.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::{Value, json};
use worker::{D1Database, Env, Method, Request, Response, Result};

use crate::{
    contracts::{API_SCHEMA_VERSION, constant_time_eq},
    repository::{
        AggregateRepository, RepositoryFailure, VideoPageRequest, VideoTitleCommand,
        VideoTitleWriteOutcome,
    },
};

const TOKEN_VARIABLE: &str = "FRAME_REPOSITORY_CONFORMANCE_TOKEN";
const TOKEN_HEADER: &str = "x-frame-repository-conformance-token";
const MAX_BODY_BYTES: usize = 192;

const USER_A: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900101";
const ORG_A: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900102";
const VIDEO_A: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900104";
const UPLOAD_A: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900111";
const JOB_A: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900112";
const USER_B: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900201";
const ORG_B: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900202";
const VIDEO_B: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900204";
const UPLOAD_B: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900211";
const JOB_B: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900212";
const CONTENTION_VIDEO: &str = "018f47a6-7b1c-7f55-8f39-8f8a86900304";
const MISSING_ID: &str = "018f47a6-7b1c-7f55-8f39-8f8a86909999";
const COMMAND_NOW_MS: u64 = 1_700_100_000_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceRequest {
    schema_version: u16,
    scenario: Scenario,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scenario {
    ReadsFound,
    ReadsNotFound,
    InvalidInput,
    CrossTenant,
    CorruptRows,
    DenormalizedRows,
    Deadline,
    Unavailable,
    WriteSuccess,
    WriteReplay,
    WriteSameKeyDifferentPayload,
    WriteStale,
    WriteCrossTenant,
    WriteConstraintRollback,
    WriteContentionLeft,
    WriteContentionRight,
}

impl Scenario {
    const fn name(self) -> &'static str {
        match self {
            Self::ReadsFound => "reads_found",
            Self::ReadsNotFound => "reads_not_found",
            Self::InvalidInput => "invalid_input",
            Self::CrossTenant => "cross_tenant",
            Self::CorruptRows => "corrupt_rows",
            Self::DenormalizedRows => "denormalized_rows",
            Self::Deadline => "deadline",
            Self::Unavailable => "unavailable",
            Self::WriteSuccess => "write_success",
            Self::WriteReplay => "write_replay",
            Self::WriteSameKeyDifferentPayload => "write_same_key_different_payload",
            Self::WriteStale => "write_stale",
            Self::WriteCrossTenant => "write_cross_tenant",
            Self::WriteConstraintRollback => "write_constraint_rollback",
            Self::WriteContentionLeft => "write_contention_left",
            Self::WriteContentionRight => "write_contention_right",
        }
    }
}

pub async fn response(mut request: Request, env: &Env) -> Result<Response> {
    if request.method() != Method::Post {
        return fixed_response(405, "method_not_allowed", None);
    }
    let expected = env
        .var(TOKEN_VARIABLE)
        .map(|value| value.to_string())
        .unwrap_or_default();
    let supplied = request.headers().get(TOKEN_HEADER)?.unwrap_or_default();
    if !valid_token(&expected)
        || !valid_token(&supplied)
        || !constant_time_eq(expected.as_bytes(), supplied.as_bytes())
    {
        return fixed_response(404, "not_found", None);
    }
    let content_type = request.headers().get("content-type")?.unwrap_or_default();
    let content_length = request
        .headers()
        .get("content-length")?
        .and_then(|value| value.parse::<usize>().ok());
    if content_type != "application/json"
        || content_length.is_none_or(|length| length == 0 || length > MAX_BODY_BYTES)
    {
        return fixed_response(400, "invalid_request", None);
    }
    let bytes = request.bytes().await?;
    if bytes.is_empty() || bytes.len() > MAX_BODY_BYTES {
        return fixed_response(400, "invalid_request", None);
    }
    let body = match serde_json::from_slice::<ConformanceRequest>(&bytes) {
        Ok(body) if body.schema_version == API_SCHEMA_VERSION => body,
        _ => return fixed_response(400, "invalid_request", None),
    };
    let database = env.d1("DB")?;
    let scenario = body.scenario;
    match run_scenario(&database, scenario).await {
        Ok((status, outcome, values)) => fixed_response(
            status,
            outcome,
            Some(json!({
                "schema_version": API_SCHEMA_VERSION,
                "scenario": scenario.name(),
                "values": values,
            })),
        ),
        Err(failure) => fixed_response(
            failure_status(failure),
            failure.code(),
            Some(json!({
                "schema_version": API_SCHEMA_VERSION,
                "scenario": scenario.name(),
                "retryable": failure.retryable(),
            })),
        ),
    }
}

async fn run_scenario(
    database: &D1Database,
    scenario: Scenario,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    match scenario {
        Scenario::ReadsFound => reads_found(database).await,
        Scenario::ReadsNotFound => reads_not_found(database).await,
        Scenario::InvalidInput => invalid_input(database).await,
        Scenario::CrossTenant => cross_tenant(database).await,
        Scenario::CorruptRows => corrupt_rows(database).await,
        Scenario::DenormalizedRows => denormalized_rows(database).await,
        Scenario::Deadline => deadline(database).await,
        Scenario::Unavailable => unavailable(database).await,
        Scenario::WriteSuccess
        | Scenario::WriteReplay
        | Scenario::WriteSameKeyDifferentPayload
        | Scenario::WriteStale
        | Scenario::WriteCrossTenant
        | Scenario::WriteConstraintRollback
        | Scenario::WriteContentionLeft
        | Scenario::WriteContentionRight => write(database, scenario).await,
    }
}

async fn reads_found(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let repository = AggregateRepository::new(database);
    let video = repository
        .video_for_mutation(ORG_A, VIDEO_A, USER_A)
        .await?
        .ok_or(RepositoryFailure::CorruptResult)?;
    let actor_can_update = video.actor_can_update();
    let video = video
        .public_response()
        .ok_or(RepositoryFailure::CorruptResult)?;
    let upload = repository
        .upload(ORG_A, UPLOAD_A)
        .await?
        .and_then(|row| row.public_status())
        .ok_or(RepositoryFailure::CorruptResult)?;
    let media_job = repository
        .media_job(ORG_A, JOB_A)
        .await?
        .and_then(|row| row.public_status())
        .ok_or(RepositoryFailure::CorruptResult)?;
    let worker_job = repository
        .native_worker_job(ORG_A, JOB_A)
        .await?
        .and_then(|row| row.private_response(false))
        .ok_or(RepositoryFailure::CorruptResult)?;
    let organization = repository
        .organization_snapshot(ORG_A)
        .await?
        .ok_or(RepositoryFailure::CorruptResult)?;

    let mut limit_one_cursor = None;
    let mut limit_one_ids = Vec::new();
    let mut limit_one_timestamps = Vec::new();
    for _ in 0..4 {
        let page = repository
            .video_page(
                ORG_A,
                &VideoPageRequest::new(Some(1), limit_one_cursor.as_deref())?,
            )
            .await?;
        if page.items.len() != 1 {
            return Err(RepositoryFailure::CorruptResult);
        }
        let item = page.items.first().ok_or(RepositoryFailure::CorruptResult)?;
        limit_one_ids.push(item.id.clone());
        limit_one_timestamps.push(item.created_at_ms);
        limit_one_cursor = Some(page.next_cursor.ok_or(RepositoryFailure::CorruptResult)?);
    }
    let first_limit_one_id = limit_one_ids
        .first()
        .cloned()
        .ok_or(RepositoryFailure::CorruptResult)?;
    let second_limit_one_id = limit_one_ids
        .get(1)
        .cloned()
        .ok_or(RepositoryFailure::CorruptResult)?;

    let mut cursor = None;
    let mut page_ids = Vec::new();
    let mut page_preview = Vec::new();
    loop {
        let page = repository
            .video_page(ORG_A, &VideoPageRequest::new(Some(100), cursor.as_deref())?)
            .await?;
        if page_preview.len() < 2 {
            page_preview.extend(page.items.iter().take(2 - page_preview.len()).cloned());
        }
        page_ids.extend(page.items.iter().map(|item| item.id.clone()));
        let Some(next) = page.next_cursor else {
            break;
        };
        cursor = Some(next);
    }
    let unique_page_ids = page_ids.iter().collect::<BTreeSet<_>>().len();
    let bulk = repository
        .videos_by_id(ORG_A, &[VIDEO_A.into(), VIDEO_B.into()])
        .await?;
    let generated_ids = (0..205).map(fixture_video_id).collect::<Vec<_>>();
    let bulk_boundary_count = repository.videos_by_id(ORG_A, &generated_ids).await?.len();
    Ok((
        200,
        "ok",
        json!({
            "actor_can_update": actor_can_update,
            "video": video,
            "upload": upload,
            "media_job": media_job,
            "worker_job": worker_job,
            "organization": organization,
            "limit_one": {
                "first_count": 1,
                "first_has_next": true,
                "first_id": first_limit_one_id,
                "second_count": 1,
                "second_id": second_limit_one_id,
                "sequence": limit_one_ids,
                "timestamps": limit_one_timestamps,
            },
            "page": {
                "count": page_ids.len(),
                "unique_count": unique_page_ids,
                "preview": page_preview,
            },
            "bulk": bulk,
            "bulk_boundary_count": bulk_boundary_count,
        }),
    ))
}

fn fixture_video_id(index: usize) -> String {
    format!(
        "018f47a6-7b1c-7f55-8f39-{:012x}",
        0x10_0000_u64 + index as u64
    )
}

async fn reads_not_found(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let repository = AggregateRepository::new(database);
    let outcomes = [
        repository
            .video_for_mutation(ORG_A, MISSING_ID, USER_A)
            .await?
            .is_none(),
        repository.upload(ORG_A, MISSING_ID).await?.is_none(),
        repository.media_job(ORG_A, MISSING_ID).await?.is_none(),
        repository
            .native_worker_job(ORG_A, MISSING_ID)
            .await?
            .is_none(),
        repository
            .organization_snapshot(MISSING_ID)
            .await?
            .is_none(),
        repository
            .video_page(MISSING_ID, &VideoPageRequest::new(Some(1), None)?)
            .await?
            .items
            .is_empty(),
        repository
            .videos_by_id(ORG_A, &[MISSING_ID.into()])
            .await?
            .is_empty(),
    ];
    if outcomes.iter().all(|outcome| *outcome) {
        Ok((200, "not_found", json!({ "method_count": 7 })))
    } else {
        Err(RepositoryFailure::CorruptResult)
    }
}

async fn invalid_input(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let repository = AggregateRepository::new(database);
    let page = VideoPageRequest {
        limit: 0,
        cursor: None,
    };
    let outcomes = [
        repository
            .video_for_mutation("invalid", VIDEO_A, USER_A)
            .await,
        repository.upload("invalid", UPLOAD_A).await.map(|_| None),
        repository.media_job("invalid", JOB_A).await.map(|_| None),
        repository
            .native_worker_job("invalid", JOB_A)
            .await
            .map(|_| None),
        repository
            .organization_snapshot("invalid")
            .await
            .map(|_| None),
        repository.video_page(ORG_A, &page).await.map(|_| None),
        repository
            .videos_by_id(ORG_A, &["invalid".into()])
            .await
            .map(|_| None),
    ];
    let mut padded_title = write_command(Scenario::WriteStale);
    padded_title.title = " Padded Title ".into();
    let padded_title = repository.update_video_title(&padded_title).await;
    if outcomes
        .iter()
        .all(|outcome| matches!(outcome, Err(RepositoryFailure::InvalidRequest)))
        && matches!(padded_title, Err(RepositoryFailure::InvalidRequest))
    {
        Ok((
            200,
            "repository_invalid_request",
            json!({ "method_count": 7, "padded_title_rejected": true }),
        ))
    } else {
        Err(RepositoryFailure::CorruptResult)
    }
}

async fn cross_tenant(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let repository = AggregateRepository::new(database);
    let video_hidden = repository
        .video_for_mutation(ORG_A, VIDEO_B, USER_A)
        .await?
        .is_none();
    let upload_hidden = repository.upload(ORG_A, UPLOAD_B).await?.is_none();
    let media_hidden = repository.media_job(ORG_A, JOB_B).await?.is_none();
    let worker_hidden = repository.native_worker_job(ORG_A, JOB_B).await?.is_none();
    let organization = repository
        .organization_snapshot(ORG_B)
        .await?
        .ok_or(RepositoryFailure::CorruptResult)?;
    let page = repository
        .video_page(ORG_B, &VideoPageRequest::new(Some(5), None)?)
        .await?;
    let bulk = repository.videos_by_id(ORG_A, &[VIDEO_B.into()]).await?;
    if video_hidden
        && upload_hidden
        && media_hidden
        && worker_hidden
        && organization.id == ORG_B
        && page.items.iter().all(|item| item.id == VIDEO_B)
        && bulk.is_empty()
    {
        Ok((200, "tenant_isolated", json!({ "method_count": 7 })))
    } else {
        Err(RepositoryFailure::CorruptResult)
    }
}

async fn corrupt_rows(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let repository = AggregateRepository::new(database);
    mutate_bound(
        database,
        "UPDATE video_uploads SET content_type = 'Video/webm' WHERE id = ?1 AND organization_id = ?2",
        &[UPLOAD_A, ORG_A],
    )
    .await?;
    let upload = repository.upload(ORG_A, UPLOAD_A).await;
    mutate_bound(
        database,
        "UPDATE video_uploads SET content_type = 'video/webm' WHERE id = ?1 AND organization_id = ?2",
        &[UPLOAD_A, ORG_A],
    )
    .await?;

    mutate_bound(
        database,
        "UPDATE media_jobs SET payload_json = '{\"profile\":42}' WHERE id = ?1 AND organization_id = ?2",
        &[JOB_A, ORG_A],
    )
    .await?;
    let media = repository.media_job(ORG_A, JOB_A).await;
    mutate_bound(
        database,
        "UPDATE media_jobs SET payload_json = '{\"profile\":\"thumbnail_v1\"}' WHERE id = ?1 AND organization_id = ?2",
        &[JOB_A, ORG_A],
    )
    .await?;

    mutate_bound(
        database,
        "UPDATE organizations SET name = ' ' WHERE id = ?1",
        &[ORG_A],
    )
    .await?;
    let organization = repository.organization_snapshot(ORG_A).await;
    mutate_bound(
        database,
        "UPDATE organizations SET name = 'Tenant A' WHERE id = ?1",
        &[ORG_A],
    )
    .await?;

    mutate_bound(
        database,
        "UPDATE media_jobs SET attempt = 4294967296 WHERE id = ?1 AND organization_id = ?2",
        &[JOB_A, ORG_A],
    )
    .await?;
    let media_attempt = repository.media_job(ORG_A, JOB_A).await;
    let worker_attempt = repository.native_worker_job(ORG_A, JOB_A).await;
    mutate_bound(
        database,
        "UPDATE media_jobs SET attempt = 1 WHERE id = ?1 AND organization_id = ?2",
        &[JOB_A, ORG_A],
    )
    .await?;

    let page_video = fixture_video_id(204);
    mutate_bound(
        database,
        "UPDATE videos SET title = ' ' WHERE id = ?1 AND organization_id = ?2",
        &[&page_video, ORG_A],
    )
    .await?;
    let page = repository
        .video_page(ORG_A, &VideoPageRequest::new(Some(1), None)?)
        .await;
    let bulk = repository
        .videos_by_id(ORG_A, std::slice::from_ref(&page_video))
        .await;
    mutate_bound(
        database,
        "UPDATE videos SET title = 'Page 204' WHERE id = ?1 AND organization_id = ?2",
        &[&page_video, ORG_A],
    )
    .await?;

    if [
        upload.map(|_| ()),
        media.map(|_| ()),
        organization.map(|_| ()),
        media_attempt.map(|_| ()),
        worker_attempt.map(|_| ()),
        page.map(|_| ()),
        bulk.map(|_| ()),
    ]
    .iter()
    .all(|outcome| matches!(outcome, Err(RepositoryFailure::CorruptResult)))
    {
        Ok((
            200,
            "repository_corrupt_result",
            json!({ "restored": true, "row_types": 7 }),
        ))
    } else {
        Err(RepositoryFailure::CorruptResult)
    }
}

async fn denormalized_rows(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let repository = AggregateRepository::new(database);
    mutate_bound(
        database,
        "UPDATE video_uploads SET organization_id = ?1 WHERE id = ?2",
        &[ORG_A, UPLOAD_B],
    )
    .await?;
    let upload_hidden = repository.upload(ORG_A, UPLOAD_B).await?.is_none()
        && repository.upload(ORG_B, UPLOAD_B).await?.is_none();

    mutate_bound(
        database,
        "UPDATE media_jobs SET organization_id = ?1 WHERE id = ?2",
        &[ORG_A, JOB_B],
    )
    .await?;
    let media_hidden = repository.media_job(ORG_A, JOB_B).await?.is_none()
        && repository.media_job(ORG_B, JOB_B).await?.is_none();
    let worker_hidden = repository.native_worker_job(ORG_A, JOB_B).await?.is_none()
        && repository.native_worker_job(ORG_B, JOB_B).await?.is_none();
    let organization_a = repository
        .organization_snapshot(ORG_A)
        .await?
        .ok_or(RepositoryFailure::CorruptResult)?;
    let organization_b = repository
        .organization_snapshot(ORG_B)
        .await?
        .ok_or(RepositoryFailure::CorruptResult)?;

    mutate_bound(
        database,
        "UPDATE video_uploads SET organization_id = ?1 WHERE id = ?2",
        &[ORG_B, UPLOAD_B],
    )
    .await?;
    mutate_bound(
        database,
        "UPDATE media_jobs SET organization_id = ?1 WHERE id = ?2",
        &[ORG_B, JOB_B],
    )
    .await?;
    let snapshots_exact = organization_a.active_members == 1
        && organization_a.active_videos == 207
        && organization_a.active_uploads == 0
        && organization_a.active_media_jobs == 0
        && organization_b.active_members == 1
        && organization_b.active_videos == 1
        && organization_b.active_uploads == 0
        && organization_b.active_media_jobs == 0;
    if upload_hidden && media_hidden && worker_hidden && snapshots_exact {
        Ok((
            200,
            "denormalized_rows_hidden",
            json!({
                "restored": true,
                "row_types": 3,
                "tenant_views": 2,
                "snapshots": {
                    "organization_a": {
                        "active_members": organization_a.active_members,
                        "active_videos": organization_a.active_videos,
                        "active_uploads": organization_a.active_uploads,
                        "active_media_jobs": organization_a.active_media_jobs,
                    },
                    "organization_b": {
                        "active_members": organization_b.active_members,
                        "active_videos": organization_b.active_videos,
                        "active_uploads": organization_b.active_uploads,
                        "active_media_jobs": organization_b.active_media_jobs,
                    },
                },
            }),
        ))
    } else {
        Err(RepositoryFailure::CorruptResult)
    }
}

async fn deadline(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let repository = AggregateRepository::with_query_timeout_ms(database, 0);
    match repository.organization_snapshot(ORG_A).await {
        Err(RepositoryFailure::Timeout) => Ok((
            503,
            "repository_timeout",
            json!({ "deadline_ms": 0, "query_dispatched": false }),
        )),
        _ => Err(RepositoryFailure::CorruptResult),
    }
}

async fn unavailable(
    database: &D1Database,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    database
        .exec("ALTER TABLE video_uploads RENAME TO repository_fault_video_uploads")
        .await
        .map_err(|_| RepositoryFailure::Unavailable)?;
    let outcome = AggregateRepository::new(database)
        .upload(ORG_A, UPLOAD_A)
        .await;
    database
        .exec("ALTER TABLE repository_fault_video_uploads RENAME TO video_uploads")
        .await
        .map_err(|_| RepositoryFailure::Unavailable)?;
    match outcome {
        Err(RepositoryFailure::Unavailable) => Ok((
            503,
            "repository_unavailable",
            json!({ "schema_restored": true }),
        )),
        _ => Err(RepositoryFailure::CorruptResult),
    }
}

async fn write(
    database: &D1Database,
    scenario: Scenario,
) -> std::result::Result<(u16, &'static str, Value), RepositoryFailure> {
    let command = write_command(scenario);
    let outcome = AggregateRepository::new(database)
        .update_video_title(&command)
        .await;
    match outcome {
        Ok(VideoTitleWriteOutcome::Applied {
            result,
            batch_meta_changes,
        }) => Ok((
            200,
            "applied",
            json!({
                "schema_version": result.schema_version,
                "video_id": result.video_id,
                "title": result.title,
                "revision": result.revision,
                "batch_meta_changes": batch_meta_changes,
            }),
        )),
        Ok(VideoTitleWriteOutcome::Replay(result)) => Ok((
            200,
            "replay",
            serde_json::to_value(result).map_err(|_| RepositoryFailure::CorruptResult)?,
        )),
        Ok(VideoTitleWriteOutcome::Conflict) => {
            Ok((409, "repository_conflict", json!({ "retryable": true })))
        }
        Err(RepositoryFailure::Unavailable)
            if matches!(scenario, Scenario::WriteConstraintRollback) =>
        {
            Ok((
                503,
                "repository_unavailable",
                json!({ "rollback_expected": true }),
            ))
        }
        Err(failure) => Err(failure),
    }
}

fn write_command(scenario: Scenario) -> VideoTitleCommand {
    let (tenant, video, actor, key, revision, title, suffix) = match scenario {
        Scenario::WriteSuccess => (
            ORG_A,
            VIDEO_A,
            USER_A,
            "repository-success-0001",
            4,
            "Repository \"Applied\" – O'Brien",
            "41",
        ),
        Scenario::WriteReplay => (
            ORG_A,
            VIDEO_A,
            USER_A,
            "repository-success-0001",
            4,
            "Repository \"Applied\" – O'Brien",
            "42",
        ),
        Scenario::WriteSameKeyDifferentPayload => (
            ORG_A,
            VIDEO_A,
            USER_A,
            "repository-success-0001",
            4,
            "Conflicting Payload",
            "43",
        ),
        Scenario::WriteStale => (
            ORG_A,
            VIDEO_A,
            USER_A,
            "repository-stale-0001",
            4,
            "Stale Write",
            "44",
        ),
        Scenario::WriteCrossTenant => (
            ORG_B,
            VIDEO_A,
            USER_B,
            "repository-cross-tenant-0001",
            5,
            "Cross Tenant",
            "45",
        ),
        Scenario::WriteConstraintRollback => (
            ORG_A,
            CONTENTION_VIDEO,
            USER_A,
            "repository-constraint-0001",
            10,
            "Must Roll Back",
            "46",
        ),
        Scenario::WriteContentionLeft => (
            ORG_A,
            CONTENTION_VIDEO,
            USER_A,
            "repository-contention-left-0001",
            10,
            "Contention Left",
            "47",
        ),
        Scenario::WriteContentionRight => (
            ORG_A,
            CONTENTION_VIDEO,
            USER_A,
            "repository-contention-right-0001",
            10,
            "Contention Right",
            "48",
        ),
        _ => unreachable!("write scenarios are closed above"),
    };
    VideoTitleCommand {
        tenant_id: tenant.into(),
        video_id: video.into(),
        actor_id: actor.into(),
        idempotency_key: key.into(),
        expected_revision: revision,
        title: title.into(),
        now_ms: COMMAND_NOW_MS,
        reservation_id: format!("018f47a6-7b1c-7f55-8f39-8f8a8690{suffix}01"),
        outbox_id: format!("018f47a6-7b1c-7f55-8f39-8f8a8690{suffix}02"),
        operation_id: format!("018f47a6-7b1c-7f55-8f39-8f8a8690{suffix}03"),
    }
}

async fn mutate_bound(
    database: &D1Database,
    sql: &'static str,
    values: &[&str],
) -> std::result::Result<(), RepositoryFailure> {
    let values = values
        .iter()
        .map(|value| wasm_bindgen::JsValue::from_str(value))
        .collect::<Vec<_>>();
    let statement = database
        .prepare(sql)
        .bind(&values)
        .map_err(|_| RepositoryFailure::Unavailable)?;
    let result = statement
        .run()
        .await
        .map_err(|_| RepositoryFailure::Unavailable)?;
    if result.success() {
        Ok(())
    } else {
        Err(RepositoryFailure::Unavailable)
    }
}

fn valid_token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

const fn failure_status(failure: RepositoryFailure) -> u16 {
    match failure {
        RepositoryFailure::InvalidRequest => 400,
        RepositoryFailure::Conflict => 409,
        RepositoryFailure::Timeout | RepositoryFailure::Unavailable => 503,
        RepositoryFailure::CorruptResult => 500,
    }
}

fn fixed_response(status: u16, outcome: &'static str, details: Option<Value>) -> Result<Response> {
    let value = json!({
        "schema_version": API_SCHEMA_VERSION,
        "outcome": outcome,
        "details": details,
    });
    Response::from_json(&value).map(|response| response.with_status(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_and_scenarios_are_closed_and_redacted() {
        assert!(valid_token(&"a".repeat(64)));
        assert!(!valid_token(&"A".repeat(64)));
        assert!(!valid_token("checked-in-token"));
        let encoded = serde_json::to_string(&json!({
            "schema_version": 1,
            "scenario": "reads_found",
        }))
        .expect("json");
        let request = serde_json::from_str::<ConformanceRequest>(&encoded).expect("request");
        assert!(matches!(request.scenario, Scenario::ReadsFound));
        assert!(
            serde_json::from_str::<ConformanceRequest>(
                r#"{"schema_version":1,"scenario":"reads_found","sql":"SELECT 1"}"#,
            )
            .is_err()
        );
    }
}
