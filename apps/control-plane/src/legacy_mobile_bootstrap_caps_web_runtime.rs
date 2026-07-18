//! Exact HTTP, D1, and R2 carrier for Cap mobile bootstrap and cap reads.

use frame_application::{
    LEGACY_MOBILE_CAPS_DEFAULT_LIMIT, LEGACY_MOBILE_CAPS_DEFAULT_PAGE,
    LEGACY_MOBILE_CAPS_MAX_LIMIT, LEGACY_MOBILE_CAPS_MAX_PAGE, LEGACY_MOBILE_R2_GET_TTL_SECONDS,
    LegacyMobileBootstrapCapsOperationV1, LegacyMobileBootstrapOrganizationV1,
    LegacyMobileBootstrapResponseV1, LegacyMobileBootstrapUserV1, LegacyMobileCapCommentAuthorV1,
    LegacyMobileCapCommentV1, LegacyMobileCapDetailV1, LegacyMobileCapsListResponseV1,
    LegacyMobileDownloadResponseV1, LegacyMobileImageLocationV1, LegacyMobilePlaybackResponseV1,
    LegacyMobileSessionErrorV1, LegacyMobileStorageObjectV1, LegacyMobileSuccessResponseV1,
    LegacyMobileVideoSourceV1, RateLimitDecisionV1, legacy_mobile_file_extension,
    legacy_mobile_image_location, legacy_mobile_iso_from_millis, legacy_mobile_metadata_projection,
    legacy_mobile_middleware_api_key, legacy_mobile_positive_integer, legacy_mobile_screenshot_key,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use worker::{Bucket, Env, Request, Response, Result, send::IntoSendFuture};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_mobile_bootstrap_caps_runtime::{
        D1LegacyMobileBootstrapCapsV1, LegacyMobileBootstrapCapsRuntimeFailureV1,
        LegacyMobileCapCommentRowV1,
    },
    legacy_mobile_session_runtime::{
        D1LegacyMobileSessionV1, LegacyMobileSessionActorV1, LegacyMobileSessionRuntimeFailureV1,
    },
};

type HttpOutcome<T> = std::result::Result<T, LegacyMobileSessionErrorV1>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyMobileBootstrapCapsRouteV1<'a> {
    Bootstrap,
    List,
    Get { video_id: &'a str },
    Delete { video_id: &'a str },
    Download { video_id: &'a str },
    Playback { video_id: &'a str },
}

impl LegacyMobileBootstrapCapsRouteV1<'_> {
    const fn operation(self) -> LegacyMobileBootstrapCapsOperationV1 {
        match self {
            Self::Bootstrap => LegacyMobileBootstrapCapsOperationV1::Bootstrap,
            Self::List => LegacyMobileBootstrapCapsOperationV1::List,
            Self::Get { .. } => LegacyMobileBootstrapCapsOperationV1::Get,
            Self::Delete { .. } => LegacyMobileBootstrapCapsOperationV1::Delete,
            Self::Download { .. } => LegacyMobileBootstrapCapsOperationV1::Download,
            Self::Playback { .. } => LegacyMobileBootstrapCapsOperationV1::Playback,
        }
    }
}

pub(crate) async fn response(
    request: &mut Request,
    env: &Env,
    route: LegacyMobileBootstrapCapsRouteV1<'_>,
    now_ms: i64,
) -> Result<Response> {
    match handle(request, env, route, now_ms).await {
        Ok(response) => Ok(response),
        Err(error) => error_response(error),
    }
}

async fn handle(
    request: &mut Request,
    env: &Env,
    route: LegacyMobileBootstrapCapsRouteV1<'_>,
    now_ms: i64,
) -> HttpOutcome<Response> {
    if route.operation().method() != request.method().to_string() {
        return Err(LegacyMobileSessionErrorV1::NotFound);
    }
    let database = env
        .d1("DB")
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let edge = compatibility_rate_limit::admit_edge_request(
        env,
        request,
        CompatibilityRateLimitBucketV1::ClientCompatibility,
        now_ms,
    )
    .await
    .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    if matches!(edge, RateLimitDecisionV1::Rejected { .. }) {
        return http_json_response(429, &json!({"_tag": "TooManyRequests"}));
    }
    let actor = authenticate(request, env, &database, now_ms).await?;
    let principal = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::ClientCompatibility,
        &actor.mapped_user_id,
        now_ms,
    )
    .await
    .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    if matches!(principal, RateLimitDecisionV1::Rejected { .. }) {
        return http_json_response(429, &json!({"_tag": "TooManyRequests"}));
    }
    reject_bodyless_carriers(request).await?;
    let authority = D1LegacyMobileBootstrapCapsV1::new(&database);
    match route {
        LegacyMobileBootstrapCapsRouteV1::Bootstrap => {
            bootstrap(env, &authority, &actor, now_ms).await
        }
        LegacyMobileBootstrapCapsRouteV1::List => {
            caps_list(request, env, &authority, &actor, now_ms).await
        }
        LegacyMobileBootstrapCapsRouteV1::Get { video_id } => {
            cap_detail(env, &authority, &actor, video_id, now_ms).await
        }
        LegacyMobileBootstrapCapsRouteV1::Delete { video_id } => {
            delete_cap(env, &authority, &actor, video_id, now_ms).await
        }
        LegacyMobileBootstrapCapsRouteV1::Download { video_id } => {
            download(env, &authority, &actor, video_id, now_ms).await
        }
        LegacyMobileBootstrapCapsRouteV1::Playback { video_id } => {
            playback(env, &authority, &actor, video_id, now_ms).await
        }
    }
}

async fn authenticate(
    request: &Request,
    env: &Env,
    database: &worker::D1Database,
    now_ms: i64,
) -> HttpOutcome<LegacyMobileSessionActorV1> {
    let authorization = request
        .headers()
        .get("authorization")
        .map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?;
    let sessions = D1LegacyMobileSessionV1::new(database);
    if let Some(api_key) = legacy_mobile_middleware_api_key(authorization.as_deref()) {
        return sessions
            .api_key_actor(api_key, now_ms)
            .await
            .map_err(map_session_runtime)?
            .ok_or(LegacyMobileSessionErrorV1::Unauthorized);
    }
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await
            .map_err(|_| LegacyMobileSessionErrorV1::Internal)?
        {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unavailable) => {
                return Err(LegacyMobileSessionErrorV1::Internal);
            }
            Err(_) => return Err(LegacyMobileSessionErrorV1::Unauthorized),
        };
    sessions
        .session_actor(&actor_id)
        .await
        .map_err(map_session_runtime)?
        .ok_or(LegacyMobileSessionErrorV1::Unauthorized)
}

async fn bootstrap(
    env: &Env,
    authority: &D1LegacyMobileBootstrapCapsV1<'_>,
    actor: &LegacyMobileSessionActorV1,
    now_ms: i64,
) -> HttpOutcome<Response> {
    let profile = authority
        .actor(&actor.mapped_user_id)
        .await
        .map_err(map_runtime)?
        .ok_or(LegacyMobileSessionErrorV1::Unauthorized)?;
    if profile.mapped_user_id != actor.mapped_user_id {
        return Err(LegacyMobileSessionErrorV1::Internal);
    }
    let legacy_user_id = profile
        .legacy_user_id
        .filter(|value| value == &actor.legacy_user_id)
        .ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let organization_rows = authority
        .organizations(&actor.mapped_user_id)
        .await
        .map_err(map_runtime)?;
    let active_index = profile
        .active_organization_id
        .as_deref()
        .and_then(|active| {
            organization_rows
                .iter()
                .position(|organization| organization.mapped_organization_id == active)
        })
        .or_else(|| (!organization_rows.is_empty()).then_some(0));
    let active = active_index.and_then(|index| organization_rows.get(index));
    let active_legacy_id = active.map(|row| row.legacy_organization_id.clone());
    let user_active_organization_id = active_legacy_id
        .clone()
        .or(profile.active_legacy_organization_id.clone())
        .ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let signer = R2GetSignerV1::from_env(env);
    let image_url = resolve_required_image(profile.image_key.as_deref(), signer.as_ref(), now_ms)?;
    let mut organizations = Vec::with_capacity(organization_rows.len());
    for row in &organization_rows {
        organizations.push(LegacyMobileBootstrapOrganizationV1 {
            id: row.legacy_organization_id.clone(),
            name: row.name.clone(),
            icon_url: resolve_required_image(row.icon_key.as_deref(), signer.as_ref(), now_ms)?,
            role: row.effective_role.clone(),
        });
    }
    let root_folders = if let Some(active) = active {
        authority
            .root_folders(&actor.mapped_user_id, &active.mapped_organization_id)
            .await
            .map_err(map_runtime)?
    } else {
        Vec::new()
    };
    http_json_response(
        200,
        &serde_json::to_value(LegacyMobileBootstrapResponseV1 {
            user: LegacyMobileBootstrapUserV1 {
                id: legacy_user_id,
                name: profile.display_name,
                email: profile.email,
                image_url,
                active_organization_id: user_active_organization_id,
            },
            organizations,
            active_organization_id: active_legacy_id,
            root_folders,
        })
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?,
    )
}

async fn caps_list(
    request: &Request,
    env: &Env,
    authority: &D1LegacyMobileBootstrapCapsV1<'_>,
    actor: &LegacyMobileSessionActorV1,
    now_ms: i64,
) -> HttpOutcome<Response> {
    let profile = authority
        .actor(&actor.mapped_user_id)
        .await
        .map_err(map_runtime)?
        .ok_or(LegacyMobileSessionErrorV1::Unauthorized)?;
    let organization_id = profile
        .active_organization_id
        .ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let query = list_query(request).map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?;
    let page = legacy_mobile_positive_integer(
        query.page.as_deref(),
        LEGACY_MOBILE_CAPS_DEFAULT_PAGE,
        LEGACY_MOBILE_CAPS_MAX_PAGE,
    );
    let limit = legacy_mobile_positive_integer(
        query.limit.as_deref(),
        LEGACY_MOBILE_CAPS_DEFAULT_LIMIT,
        LEGACY_MOBILE_CAPS_MAX_LIMIT,
    );
    let offset = (page - 1) * limit;
    let folder_id = query.folder_id.filter(|value| !value.is_empty());
    let page_rows = authority
        .caps(
            &actor.mapped_user_id,
            &organization_id,
            folder_id.as_deref(),
            limit,
            offset,
        )
        .await
        .map_err(map_runtime)?;
    let bucket = env
        .bucket("RECORDINGS")
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let signer = R2GetSignerV1::from_env(env);
    let web_url = web_url(env)?;
    let mut caps = Vec::with_capacity(page_rows.rows.len());
    for row in page_rows.rows {
        let thumbnail = thumbnail_best_effort(
            &bucket,
            signer.as_ref(),
            row.object_prefix.as_deref(),
            now_ms,
        )
        .await;
        caps.push(
            row.projection()
                .map_err(map_runtime)?
                .into_summary(&web_url, thumbnail)
                .ok_or(LegacyMobileSessionErrorV1::Internal)?,
        );
    }
    let folders = if folder_id.is_none() {
        authority
            .root_folders(&actor.mapped_user_id, &organization_id)
            .await
            .map_err(map_runtime)?
    } else {
        Vec::new()
    };
    let total = page_rows.total;
    http_json_response(
        200,
        &serde_json::to_value(LegacyMobileCapsListResponseV1 {
            folders,
            caps,
            page: f64::from(page),
            limit: f64::from(limit),
            total: total as f64,
            has_more: i64::from(page) * i64::from(limit) < total,
        })
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?,
    )
}

async fn cap_detail(
    env: &Env,
    authority: &D1LegacyMobileBootstrapCapsV1<'_>,
    actor: &LegacyMobileSessionActorV1,
    video_id: &str,
    now_ms: i64,
) -> HttpOutcome<Response> {
    let row = authority
        .cap(&actor.mapped_user_id, video_id)
        .await
        .map_err(map_runtime)?;
    let bucket = env
        .bucket("RECORDINGS")
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let signer = R2GetSignerV1::from_env(env);
    let thumbnail = thumbnail_best_effort(
        &bucket,
        signer.as_ref(),
        row.object_prefix.as_deref(),
        now_ms,
    )
    .await;
    let web_url = web_url(env)?;
    let cap = row
        .projection()
        .map_err(map_runtime)?
        .into_summary(&web_url, thumbnail)
        .ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let metadata_value = row
        .metadata_json
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let metadata = legacy_mobile_metadata_projection(metadata_value.as_ref());
    if row.transcription_status.as_deref().is_some_and(|value| {
        !matches!(
            value,
            "PROCESSING" | "COMPLETE" | "ERROR" | "SKIPPED" | "NO_AUDIO"
        )
    }) {
        return Err(LegacyMobileSessionErrorV1::Internal);
    }
    let comment_rows = authority.comments(video_id).await.map_err(map_runtime)?;
    let mut comments = Vec::with_capacity(comment_rows.len());
    for comment in comment_rows {
        comments.push(comment_projection(comment, signer.as_ref(), now_ms)?);
    }
    let share_url = format!("{}/s/{video_id}", web_url.trim_end_matches('/'));
    http_json_response(
        200,
        &serde_json::to_value(LegacyMobileCapDetailV1 {
            cap,
            summary: metadata.summary,
            chapters: metadata.chapters,
            transcription_status: row.transcription_status,
            comments,
            share_url,
        })
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?,
    )
}

async fn playback(
    env: &Env,
    authority: &D1LegacyMobileBootstrapCapsV1<'_>,
    actor: &LegacyMobileSessionActorV1,
    video_id: &str,
    now_ms: i64,
) -> HttpOutcome<Response> {
    let row = authority
        .cap(&actor.mapped_user_id, video_id)
        .await
        .map_err(map_runtime)?;
    let prefix = row
        .object_prefix
        .as_deref()
        .ok_or(LegacyMobileSessionErrorV1::NotFound)?;
    let source = row.source().map_err(map_runtime)?;
    let bucket = env
        .bucket("RECORDINGS")
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let signer = R2GetSignerV1::from_env(env).ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let url = if source == LegacyMobileVideoSourceV1::DesktopSegments {
        format!(
            "{}/api/playlist?videoId={video_id}&videoType=segments-master",
            web_url(env)?.trim_end_matches('/')
        )
    } else {
        let key = source
            .playback_object_key(prefix)
            .ok_or(LegacyMobileSessionErrorV1::NotFound)?;
        signer
            .sign_get(&key, now_ms)
            .ok_or(LegacyMobileSessionErrorV1::Internal)?
    };
    let transcript_key = format!("{prefix}transcription.vtt");
    let transcript_url = match bucket.head(&transcript_key).into_send().await {
        Ok(Some(_)) => signer.sign_get(&transcript_key, now_ms),
        _ => None,
    };
    http_json_response(
        200,
        &serde_json::to_value(LegacyMobilePlaybackResponseV1 {
            kind: source.playback_kind().to_owned(),
            url,
            transcript_url,
        })
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?,
    )
}

async fn download(
    env: &Env,
    authority: &D1LegacyMobileBootstrapCapsV1<'_>,
    actor: &LegacyMobileSessionActorV1,
    video_id: &str,
    now_ms: i64,
) -> HttpOutcome<Response> {
    let row = authority
        .cap(&actor.mapped_user_id, video_id)
        .await
        .map_err(map_runtime)?;
    let prefix = row
        .object_prefix
        .as_deref()
        .ok_or(LegacyMobileSessionErrorV1::NotFound)?;
    let bucket = env
        .bucket("RECORDINGS")
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let signer = R2GetSignerV1::from_env(env).ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let (key, extension) = if row.screenshot().map_err(map_runtime)? {
        let objects = screenshot_objects(&bucket, prefix).await?;
        let key =
            legacy_mobile_screenshot_key(&objects).ok_or(LegacyMobileSessionErrorV1::NotFound)?;
        let extension = legacy_mobile_file_extension(&key).unwrap_or_else(|| "jpg".into());
        (key, extension)
    } else {
        let source = row.source().map_err(map_runtime)?;
        if !source.download_is_mp4() {
            return Err(LegacyMobileSessionErrorV1::NotFound);
        }
        let result_key = format!("{prefix}result.mp4");
        if source == LegacyMobileVideoSourceV1::WebMp4 {
            let usable_result = bucket
                .head(&result_key)
                .into_send()
                .await
                .map_err(|_| LegacyMobileSessionErrorV1::Internal)?
                .is_some_and(|object| object.size() > 0);
            if usable_result {
                (result_key, "mp4".into())
            } else if let Some(raw) = row.raw_file_key.clone() {
                let extension = legacy_mobile_file_extension(&raw).unwrap_or_else(|| "mp4".into());
                (raw, extension)
            } else {
                (result_key, "mp4".into())
            }
        } else {
            (result_key, "mp4".into())
        }
    };
    let url = signer
        .sign_get(&key, now_ms)
        .ok_or(LegacyMobileSessionErrorV1::Internal)?;
    http_json_response(
        200,
        &serde_json::to_value(LegacyMobileDownloadResponseV1 {
            file_name: format!("{}.{}", row.title, extension),
            url,
        })
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?,
    )
}

async fn delete_cap(
    env: &Env,
    authority: &D1LegacyMobileBootstrapCapsV1<'_>,
    actor: &LegacyMobileSessionActorV1,
    video_id: &str,
    now_ms: i64,
) -> HttpOutcome<Response> {
    // Cap resolves storage authority before its database mutation. Provider
    // I/O still begins only after the D1 tombstone commits.
    let bucket = env
        .bucket("RECORDINGS")
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let continuation = authority
        .begin_delete(&actor.mapped_user_id, video_id, now_ms)
        .await
        .map_err(map_runtime)?;
    let listed = bucket
        .list()
        .limit(1_000)
        .prefix(&continuation.object_prefix)
        .execute()
        .into_send()
        .await
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let keys = listed
        .objects()
        .into_iter()
        .map(|object| object.key())
        .collect::<Vec<_>>();
    if keys.iter().any(|key| {
        !key.starts_with(&continuation.object_prefix)
            || key.len() > 2_048
            || key.bytes().any(|byte| byte.is_ascii_control())
    }) {
        return Err(LegacyMobileSessionErrorV1::Internal);
    }
    if !keys.is_empty() {
        bucket
            .delete_multiple(keys)
            .into_send()
            .await
            .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    }
    authority
        .complete_delete(&continuation.operation_id, now_ms)
        .await
        .map_err(map_runtime)?;
    http_json_response(
        200,
        &serde_json::to_value(LegacyMobileSuccessResponseV1 { success: true })
            .map_err(|_| LegacyMobileSessionErrorV1::Internal)?,
    )
}

fn comment_projection(
    row: LegacyMobileCapCommentRowV1,
    signer: Option<&R2GetSignerV1>,
    now_ms: i64,
) -> HttpOutcome<LegacyMobileCapCommentV1> {
    let created_at = legacy_mobile_iso_from_millis(row.created_at_ms)
        .ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let updated_at = legacy_mobile_iso_from_millis(row.updated_at_ms)
        .ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let image_url = row
        .author_image
        .as_deref()
        .and_then(|value| resolve_image_best_effort(value, signer, now_ms));
    Ok(LegacyMobileCapCommentV1 {
        id: row.legacy_comment_id,
        video_id: row.legacy_video_id,
        comment_type: row.comment_kind,
        content: row.content,
        timestamp: row.source_timestamp,
        parent_comment_id: row.legacy_parent_comment_id,
        created_at,
        updated_at,
        author: LegacyMobileCapCommentAuthorV1 {
            id: row.legacy_author_id,
            name: row.author_name,
            image_url,
        },
    })
}

async fn thumbnail_best_effort(
    bucket: &Bucket,
    signer: Option<&R2GetSignerV1>,
    prefix: Option<&str>,
    now_ms: i64,
) -> Option<String> {
    let prefix = prefix?;
    let signer = signer?;
    let objects = screenshot_objects(bucket, prefix).await.ok()?;
    let key = legacy_mobile_screenshot_key(&objects)?;
    signer.sign_get(&key, now_ms)
}

async fn screenshot_objects(
    bucket: &Bucket,
    prefix: &str,
) -> HttpOutcome<Vec<LegacyMobileStorageObjectV1>> {
    let listed = bucket
        .list()
        .limit(1_000)
        .prefix(prefix)
        .execute()
        .into_send()
        .await
        .map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    let mut objects = Vec::new();
    for object in listed.objects() {
        let key = object.key();
        if !key.starts_with(prefix) || key.len() > 2_048 {
            return Err(LegacyMobileSessionErrorV1::Internal);
        }
        objects.push(LegacyMobileStorageObjectV1 {
            key,
            last_modified_ms: i64::try_from(object.uploaded().as_millis()).ok(),
        });
    }
    Ok(objects)
}

#[derive(Debug, Default)]
struct ListQueryV1 {
    folder_id: Option<String>,
    page: Option<String>,
    limit: Option<String>,
}

fn list_query(request: &Request) -> Result<ListQueryV1> {
    let mut query = ListQueryV1::default();
    for (key, value) in request.url()?.query_pairs() {
        match key.as_ref() {
            "folderId" => query.folder_id = Some(value.into_owned()),
            "page" => query.page = Some(value.into_owned()),
            "limit" => query.limit = Some(value.into_owned()),
            _ => {}
        }
    }
    Ok(query)
}

async fn reject_bodyless_carriers(request: &mut Request) -> HttpOutcome<()> {
    let headers = request.headers();
    if headers
        .get("idempotency-key")
        .map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?
        .is_some()
        || headers
            .get("content-type")
            .map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?
            .is_some()
        || headers
            .get("transfer-encoding")
            .map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?
            .is_some()
        || headers
            .get("content-encoding")
            .map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?
            .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(LegacyMobileSessionErrorV1::BadRequest);
    }
    if headers
        .get("content-length")
        .map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?
        .is_some_and(|value| value.parse::<u64>() != Ok(0))
    {
        return Err(LegacyMobileSessionErrorV1::BadRequest);
    }
    let body = request.bytes().await;
    let body = body.map_err(|_| LegacyMobileSessionErrorV1::BadRequest)?;
    if !body.is_empty() {
        return Err(LegacyMobileSessionErrorV1::BadRequest);
    }
    Ok(())
}

fn web_url(env: &Env) -> HttpOutcome<String> {
    let value = env_value(env, "WEB_URL").ok_or(LegacyMobileSessionErrorV1::Internal)?;
    let url = url::Url::parse(&value).map_err(|_| LegacyMobileSessionErrorV1::Internal)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(LegacyMobileSessionErrorV1::Internal);
    }
    Ok(url.origin().ascii_serialization())
}

fn resolve_required_image(
    value: Option<&str>,
    signer: Option<&R2GetSignerV1>,
    now_ms: i64,
) -> HttpOutcome<Option<String>> {
    value
        .map(|value| {
            resolve_image_best_effort(value, signer, now_ms)
                .ok_or(LegacyMobileSessionErrorV1::Internal)
        })
        .transpose()
}

fn resolve_image_best_effort(
    value: &str,
    signer: Option<&R2GetSignerV1>,
    now_ms: i64,
) -> Option<String> {
    match legacy_mobile_image_location(value) {
        LegacyMobileImageLocationV1::ExternalUrl(url) => Some(url),
        LegacyMobileImageLocationV1::PrivateObjectKey(key) => signer?.sign_get(&key, now_ms),
    }
}

fn map_runtime(failure: LegacyMobileBootstrapCapsRuntimeFailureV1) -> LegacyMobileSessionErrorV1 {
    match failure {
        LegacyMobileBootstrapCapsRuntimeFailureV1::NotFound => LegacyMobileSessionErrorV1::NotFound,
        LegacyMobileBootstrapCapsRuntimeFailureV1::Corrupt
        | LegacyMobileBootstrapCapsRuntimeFailureV1::Unavailable => {
            LegacyMobileSessionErrorV1::Internal
        }
    }
}

fn map_session_runtime(failure: LegacyMobileSessionRuntimeFailureV1) -> LegacyMobileSessionErrorV1 {
    match failure {
        LegacyMobileSessionRuntimeFailureV1::Forbidden => LegacyMobileSessionErrorV1::Unauthorized,
        LegacyMobileSessionRuntimeFailureV1::Corrupt
        | LegacyMobileSessionRuntimeFailureV1::Unavailable => LegacyMobileSessionErrorV1::Internal,
    }
}

fn error_response(error: LegacyMobileSessionErrorV1) -> Result<Response> {
    json_response(error.status(), &json!({"_tag": error.tag()}))
}

fn http_json_response(status: u16, value: &Value) -> HttpOutcome<Response> {
    json_response(status, value).map_err(|_| LegacyMobileSessionErrorV1::Internal)
}

fn json_response(status: u16, value: &Value) -> Result<Response> {
    let mut response = Response::from_json(value)?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn env_value(env: &Env, name: &str) -> Option<String> {
    env.secret(name)
        .map(|value| value.to_string())
        .or_else(|_| env.var(name).map(|value| value.to_string()))
        .ok()
}

#[derive(Clone)]
struct R2GetSignerV1 {
    account_id: String,
    bucket_name: String,
    access_key_id: String,
    secret_access_key: String,
}

impl R2GetSignerV1 {
    fn from_env(env: &Env) -> Option<Self> {
        let account_id = env_value(env, "FRAME_R2_ACCOUNT_ID")?;
        let bucket_name = env_value(env, "FRAME_R2_BUCKET_NAME")?;
        let access_key_id = env_value(env, "FRAME_R2_ACCESS_KEY_ID")?;
        let secret_access_key = env_value(env, "FRAME_R2_SECRET_ACCESS_KEY")?;
        if account_id.len() != 32
            || !account_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !(3..=63).contains(&bucket_name.len())
            || !bucket_name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || bucket_name.starts_with('-')
            || bucket_name.ends_with('-')
            || !(16..=128).contains(&access_key_id.len())
            || !(32..=256).contains(&secret_access_key.len())
            || !access_key_id.bytes().all(|byte| byte.is_ascii_graphic())
            || !secret_access_key
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
        {
            return None;
        }
        Some(Self {
            account_id,
            bucket_name,
            access_key_id,
            secret_access_key,
        })
    }

    fn sign_get(&self, key: &str, now_ms: i64) -> Option<String> {
        if !(0..=253_402_300_799_999).contains(&now_ms) || !valid_object_key(key) {
            return None;
        }
        let (date, timestamp) = aws_timestamp(now_ms as u64)?;
        let host = format!("{}.r2.cloudflarestorage.com", self.account_id);
        let canonical_uri = format!(
            "/{}/{}",
            percent_encode(&self.bucket_name),
            key.split('/')
                .map(percent_encode)
                .collect::<Vec<_>>()
                .join("/")
        );
        let scope = format!("{date}/auto/s3/aws4_request");
        let mut query = [
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_owned()),
            (
                "X-Amz-Credential",
                format!("{}/{}", self.access_key_id, scope),
            ),
            ("X-Amz-Date", timestamp.clone()),
            (
                "X-Amz-Expires",
                LEGACY_MOBILE_R2_GET_TTL_SECONDS.to_string(),
            ),
            ("X-Amz-Content-Sha256", "UNSIGNED-PAYLOAD".to_owned()),
            ("X-Amz-SignedHeaders", "host".to_owned()),
        ];
        query.sort_by(|left, right| left.0.cmp(right.0));
        let canonical_query = query
            .iter()
            .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        let canonical_request = format!(
            "GET\n{canonical_uri}\n{canonical_query}\nhost:{host}\n\nhost\nUNSIGNED-PAYLOAD"
        );
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
            hex(&Sha256::digest(canonical_request.as_bytes()))
        );
        let date_key = hmac_sha256(
            format!("AWS4{}", self.secret_access_key).as_bytes(),
            date.as_bytes(),
        );
        let region_key = hmac_sha256(&date_key, b"auto");
        let service_key = hmac_sha256(&region_key, b"s3");
        let signing_key = hmac_sha256(&service_key, b"aws4_request");
        let signature = hex(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));
        Some(format!(
            "https://{host}{canonical_uri}?{canonical_query}&X-Amz-Signature={signature}"
        ))
    }
}

fn valid_object_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2_048
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.split('/').any(|part| matches!(part, "." | ".."))
        && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(b"0123456789ABCDEF"[usize::from(byte >> 4)]));
            output.push(char::from(b"0123456789ABCDEF"[usize::from(byte & 0x0f)]));
        }
    }
    output
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0_u8; 64];
    if key.len() > normalized.len() {
        normalized[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner_key = [0x36_u8; 64];
    let mut outer_key = [0x5c_u8; 64];
    for index in 0..64 {
        inner_key[index] ^= normalized[index];
        outer_key[index] ^= normalized[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_key);
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(outer_key);
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(value: &[u8]) -> String {
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        output.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    output
}

fn aws_timestamp(now_ms: u64) -> Option<(String, String)> {
    let seconds = i64::try_from(now_ms / 1_000).ok()?;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days)?;
    let hour = day_seconds / 3_600;
    let minute = day_seconds % 3_600 / 60;
    let second = day_seconds % 60;
    let date = format!("{year:04}{month:02}{day:02}");
    Some((
        date.clone(),
        format!("{date}T{hour:02}{minute:02}{second:02}Z"),
    ))
}

fn civil_from_days(days_since_epoch: i64) -> Option<(i64, i64, i64)> {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month_prime = (5 * doy + 2) / 153;
    let day = doy - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (2000..=9999).contains(&year).then_some((year, month, day))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> R2GetSignerV1 {
        R2GetSignerV1 {
            account_id: "0123456789abcdef0123456789abcdef".into(),
            bucket_name: "frame-recordings".into(),
            access_key_id: "frame-test-access-key-0001".into(),
            secret_access_key: "frame-test-secret-material-that-is-long-enough-0001".into(),
        }
    }

    #[test]
    fn signed_get_binds_only_host_so_range_remains_client_selectable() {
        let url = signer()
            .sign_get("owner/video/result.mp4", 1_735_787_045_006)
            .expect("signed GET");
        assert!(url.contains("X-Amz-SignedHeaders=host"));
        assert!(!url.to_ascii_lowercase().contains("range"));
        assert!(url.contains("/frame-recordings/owner/video/result.mp4"));
    }

    #[test]
    fn bodyless_routes_reject_dangerous_key_shapes_before_signing() {
        assert!(valid_object_key("owner/video/result.mp4"));
        assert!(!valid_object_key("owner/../secret"));
        assert!(!valid_object_key("/absolute"));
    }
}
