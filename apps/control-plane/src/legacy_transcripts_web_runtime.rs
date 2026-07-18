//! Exact HTTP and compatibility-action carrier for Cap transcript operations.

use frame_application::{
    LEGACY_AVAILABLE_TRANSLATIONS_OPERATION_ID, LEGACY_EDIT_TRANSCRIPT_OPERATION_ID,
    LEGACY_GET_TRANSCRIPT_OPERATION_ID, LEGACY_TRANSCRIPT_ACTION_SCHEMA_V1,
    LEGACY_TRANSCRIPT_MAX_BODY_BYTES, LEGACY_TRANSCRIPT_MAX_OBJECT_BYTES,
    LEGACY_TRANSLATE_TRANSCRIPT_OPERATION_ID, LegacyAvailableTranslationsResultV1,
    LegacyTranscriptResultV1, LegacyTranscriptSurfaceV1, legacy_available_translations_from_keys,
    legacy_transcript_language_name, legacy_update_vtt_entry_text,
};
use frame_domain::IdempotencyKey;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use worker::{Env, HttpMetadata, Request, Response, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_transcripts_runtime::{
        D1LegacyTranscriptAuthorityV1, LegacyTranscriptOperationRowV1,
        LegacyTranscriptRuntimeErrorV1, LegacyTranscriptVideoV1, NewLegacyTranscriptOperationV1,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodedLegacyTranscriptActionV1 {
    Edit {
        video_id: String,
        entry_id: u64,
        new_text: String,
        idempotency_key: String,
    },
    Get {
        video_id: String,
    },
    AvailableTranslations {
        video_id: String,
    },
    Translate {
        video_id: String,
        target_language: String,
        idempotency_key: String,
    },
}

impl DecodedLegacyTranscriptActionV1 {
    #[cfg(test)]
    const fn surface(&self) -> LegacyTranscriptSurfaceV1 {
        match self {
            Self::Edit { .. } => LegacyTranscriptSurfaceV1::Edit,
            Self::Get { .. } => LegacyTranscriptSurfaceV1::Get,
            Self::AvailableTranslations { .. } => LegacyTranscriptSurfaceV1::AvailableTranslations,
            Self::Translate { .. } => LegacyTranscriptSurfaceV1::Translate,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditWireV1 {
    schema_version: String,
    video_id: String,
    entry_id: u64,
    new_text: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetWireV1 {
    schema_version: String,
    video_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranslateWireV1 {
    schema_version: String,
    video_id: String,
    target_language: String,
    idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum LegacyTranscriptActionResultV1 {
    Transcript(LegacyTranscriptResultV1),
    AvailableTranslations(LegacyAvailableTranslationsResultV1),
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    matches!(
        operation_id,
        LEGACY_EDIT_TRANSCRIPT_OPERATION_ID
            | LEGACY_GET_TRANSCRIPT_OPERATION_ID
            | LEGACY_AVAILABLE_TRANSLATIONS_OPERATION_ID
            | LEGACY_TRANSLATE_TRANSCRIPT_OPERATION_ID
    )
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedLegacyTranscriptActionV1>> {
    let Some(surface) = LegacyTranscriptSurfaceV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    if surface == LegacyTranscriptSurfaceV1::Retry
        || request.headers().get("idempotency-key")?.is_some()
        || !matches!(
            request.headers().get("content-type")?.as_deref(),
            Some("application/json" | "application/json; charset=utf-8")
        )
        || request
            .headers()
            .get("content-encoding")?
            .is_some_and(|value| value != "identity")
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let declared = request
        .headers()
        .get("content-length")?
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| worker::Error::RustError("invalid transcript content length".into()))?;
    if declared.is_some_and(|value| value == 0 || value > LEGACY_TRANSCRIPT_MAX_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let body =
        match crate::read_bounded_legacy_body(request, LEGACY_TRANSCRIPT_MAX_BODY_BYTES).await {
            Ok(body) => body,
            Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
        };
    if body.is_empty()
        || body.len() > LEGACY_TRANSCRIPT_MAX_BODY_BYTES
        || declared.is_some_and(|value| value != body.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    Ok(decode_action_bytes(surface, &body))
}

pub(crate) async fn action(
    request: &Request,
    env: &Env,
    decoded: &DecodedLegacyTranscriptActionV1,
    now_ms: i64,
) -> Result<BrowserWebOutcome<LegacyTranscriptActionResultV1>> {
    match decoded {
        DecodedLegacyTranscriptActionV1::Get { video_id } => {
            get_transcript(request, env, video_id, now_ms)
                .await
                .map(|outcome| outcome.map(LegacyTranscriptActionResultV1::Transcript))
        }
        DecodedLegacyTranscriptActionV1::AvailableTranslations { video_id } => {
            available_translations(request, env, video_id, now_ms)
                .await
                .map(|outcome| outcome.map(LegacyTranscriptActionResultV1::AvailableTranslations))
        }
        DecodedLegacyTranscriptActionV1::Edit {
            video_id,
            entry_id,
            new_text,
            idempotency_key,
        } => edit_transcript(
            request,
            env,
            video_id,
            *entry_id,
            new_text,
            idempotency_key,
            now_ms,
        )
        .await
        .map(|outcome| outcome.map(LegacyTranscriptActionResultV1::Transcript)),
        DecodedLegacyTranscriptActionV1::Translate {
            video_id,
            target_language,
            idempotency_key,
        } => translate_transcript(
            request,
            env,
            video_id,
            target_language,
            idempotency_key,
            now_ms,
        )
        .await
        .map(|outcome| outcome.map(LegacyTranscriptActionResultV1::Transcript)),
    }
}

pub(crate) async fn retry_response(
    request: &Request,
    env: &Env,
    video_id: &str,
    now_ms: i64,
) -> Result<Response> {
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => actor_id,
            Err(_) => return exact_json(401, json!({"error": "Unauthorized"})),
        };
    let Some(idempotency_key) = request.headers().get("idempotency-key")? else {
        return exact_json(400, json!({"error": "Idempotency-Key is required"}));
    };
    if IdempotencyKey::parse(&idempotency_key).is_err() {
        return exact_json(400, json!({"error": "Invalid Idempotency-Key"}));
    }
    let database = env.d1("DB")?;
    if rejected(
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::CollaborationNotifications,
            &actor_id,
            now_ms,
        )
        .await?,
    ) {
        return exact_json(429, json!({"error": "Too many requests"}));
    }
    let authority = D1LegacyTranscriptAuthorityV1::new(&database);
    let video = match authority.video(video_id).await {
        Ok(video) => video,
        Err(LegacyTranscriptRuntimeErrorV1::NotFound | LegacyTranscriptRuntimeErrorV1::Invalid) => {
            return exact_json(404, json!({"error": "Video not found"}));
        }
        Err(_) => return exact_json(500, json!({"error": "Internal server error"})),
    };
    if !video.is_owner(Some(&actor_id)) {
        return exact_json(403, json!({"error": "Unauthorized"}));
    }
    let request_digest = digest_parts(&[b"retry", video_id.as_bytes()]);
    let actor_scope = digest_parts(&[b"actor", actor_id.as_bytes()]);
    let idempotency_digest = digest_parts(&[b"idempotency", idempotency_key.as_bytes()]);
    let operation = match authority
        .claim_operation(&NewLegacyTranscriptOperationV1 {
            operation_id: &Uuid::new_v4().to_string(),
            surface: LegacyTranscriptSurfaceV1::Retry,
            actor_scope_digest: &actor_scope,
            actor_id: Some(&actor_id),
            video: &video,
            idempotency_key_digest: &idempotency_digest,
            request_digest: &request_digest,
            object_key: &video.original_object_key().expect("validated key"),
            target_language: None,
            entry_id: None,
            replacement_text: None,
            now_ms,
        })
        .await
    {
        Ok(operation) => operation,
        Err(LegacyTranscriptRuntimeErrorV1::Conflict) => {
            return exact_json(409, json!({"error": "Idempotency conflict"}));
        }
        Err(_) => return exact_json(500, json!({"error": "Internal server error"})),
    };
    if let Some(result) = operation.completed_result().ok().flatten() {
        return exact_json(
            200,
            serde_json::to_value(result)
                .unwrap_or_else(|_| json!({"error":"Internal server error"})),
        );
    }
    if authority.reset_retry_status(&video, now_ms).await.is_err() {
        return exact_json(500, json!({"error": "Internal server error"}));
    }
    let result = LegacyTranscriptResultV1 {
        success: true,
        content: None,
        translated_vtt: None,
        message: "Transcription retry triggered".into(),
    };
    if authority
        .complete(&operation, &result, now_ms)
        .await
        .is_err()
    {
        return exact_json(500, json!({"error": "Internal server error"}));
    }
    exact_json(200, serde_json::to_value(result).unwrap_or(Value::Null))
}

fn decode_action_bytes(
    surface: LegacyTranscriptSurfaceV1,
    body: &[u8],
) -> BrowserWebOutcome<DecodedLegacyTranscriptActionV1> {
    match surface {
        LegacyTranscriptSurfaceV1::Edit => {
            let wire: EditWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            require_video_id(&wire.video_id)?;
            if wire.new_text.trim().is_empty()
                || wire.new_text.len() > LEGACY_TRANSCRIPT_MAX_BODY_BYTES
                || wire.entry_id > 9_007_199_254_740_991
                || IdempotencyKey::parse(&wire.idempotency_key).is_err()
            {
                return Err(BrowserWebFailure::Invalid);
            }
            Ok(DecodedLegacyTranscriptActionV1::Edit {
                video_id: wire.video_id,
                entry_id: wire.entry_id,
                new_text: wire.new_text,
                idempotency_key: wire.idempotency_key,
            })
        }
        LegacyTranscriptSurfaceV1::Get => {
            let wire: GetWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            require_video_id(&wire.video_id)?;
            Ok(DecodedLegacyTranscriptActionV1::Get {
                video_id: wire.video_id,
            })
        }
        LegacyTranscriptSurfaceV1::AvailableTranslations => {
            let wire: GetWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            require_video_id(&wire.video_id)?;
            Ok(DecodedLegacyTranscriptActionV1::AvailableTranslations {
                video_id: wire.video_id,
            })
        }
        LegacyTranscriptSurfaceV1::Translate => {
            let wire: TranslateWireV1 =
                serde_json::from_slice(body).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            require_video_id(&wire.video_id)?;
            if legacy_transcript_language_name(&wire.target_language).is_none()
                || IdempotencyKey::parse(&wire.idempotency_key).is_err()
            {
                return Err(BrowserWebFailure::Invalid);
            }
            Ok(DecodedLegacyTranscriptActionV1::Translate {
                video_id: wire.video_id,
                target_language: wire.target_language,
                idempotency_key: wire.idempotency_key,
            })
        }
        LegacyTranscriptSurfaceV1::Retry => Err(BrowserWebFailure::NotFound),
    }
}

async fn get_transcript(
    request: &Request,
    env: &Env,
    video_id: &str,
    now_ms: i64,
) -> Result<BrowserWebOutcome<LegacyTranscriptResultV1>> {
    let actor_id = optional_actor(request, env, now_ms).await?;
    let database = env.d1("DB")?;
    if !admit_read(
        env,
        request,
        &database,
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
        actor_id.as_deref(),
        now_ms,
    )
    .await?
    {
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let authority = D1LegacyTranscriptAuthorityV1::new(&database);
    let video = match visible_video(&authority, request, env, video_id, actor_id.as_deref()).await {
        Ok(video) => video,
        Err(
            LegacyTranscriptRuntimeErrorV1::NotFound | LegacyTranscriptRuntimeErrorV1::Unauthorized,
        ) => {
            return Ok(Ok(LegacyTranscriptResultV1::failure("Video not found")));
        }
        Err(_) => return Ok(Err(BrowserWebFailure::Unavailable)),
    };
    if video.transcription_status.as_deref() != Some("COMPLETE") {
        return Ok(Ok(LegacyTranscriptResultV1::failure(
            "Transcript is not ready yet",
        )));
    }
    let key = video.original_object_key().expect("validated key");
    match read_text_object(env, &key).await {
        Ok(Some((content, _, _))) => Ok(Ok(LegacyTranscriptResultV1 {
            success: true,
            content: Some(content),
            translated_vtt: None,
            message: "Transcript retrieved successfully".into(),
        })),
        Ok(None) => Ok(Ok(LegacyTranscriptResultV1::failure(
            "Transcript file not found",
        ))),
        Err(_) => Ok(Ok(LegacyTranscriptResultV1::failure(
            "Failed to fetch transcript",
        ))),
    }
}

async fn available_translations(
    request: &Request,
    env: &Env,
    video_id: &str,
    now_ms: i64,
) -> Result<BrowserWebOutcome<LegacyAvailableTranslationsResultV1>> {
    let actor_id = optional_actor(request, env, now_ms).await?;
    let database = env.d1("DB")?;
    if !admit_read(
        env,
        request,
        &database,
        CompatibilityRateLimitBucketV1::VideoMedia,
        actor_id.as_deref(),
        now_ms,
    )
    .await?
    {
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let authority = D1LegacyTranscriptAuthorityV1::new(&database);
    let video = match visible_video(&authority, request, env, video_id, actor_id.as_deref()).await {
        Ok(video) => video,
        Err(
            LegacyTranscriptRuntimeErrorV1::NotFound | LegacyTranscriptRuntimeErrorV1::Unauthorized,
        ) => {
            return Ok(Ok(LegacyAvailableTranslationsResultV1::failure(
                "Video not found",
            )));
        }
        Err(_) => return Ok(Err(BrowserWebFailure::Unavailable)),
    };
    let bucket = match env.bucket("RECORDINGS") {
        Ok(bucket) => bucket,
        Err(_) => {
            return Ok(Ok(LegacyAvailableTranslationsResultV1::failure(
                "Failed to list translations",
            )));
        }
    };
    let prefix = format!("{}transcription", video.object_prefix);
    let listed = match bucket.list().prefix(prefix).limit(50).execute().await {
        Ok(listed) => listed,
        Err(_) => {
            return Ok(Ok(LegacyAvailableTranslationsResultV1::failure(
                "Failed to list translations",
            )));
        }
    };
    let (has_original, translations) = legacy_available_translations_from_keys(
        listed.objects().into_iter().map(|object| object.key()),
    );
    Ok(Ok(LegacyAvailableTranslationsResultV1 {
        success: true,
        has_original,
        translations,
        message: None,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn edit_transcript(
    request: &Request,
    env: &Env,
    video_id: &str,
    entry_id: u64,
    new_text: &str,
    idempotency_key: &str,
    now_ms: i64,
) -> Result<BrowserWebOutcome<LegacyTranscriptResultV1>> {
    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(proof) => proof,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor_id = proof.user_id().to_string();
    let database = env.d1("DB")?;
    if rejected(
        compatibility_rate_limit::admit_principal(
            env,
            &database,
            CompatibilityRateLimitBucketV1::CollaborationNotifications,
            &actor_id,
            now_ms,
        )
        .await?,
    ) {
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let authority = D1LegacyTranscriptAuthorityV1::new(&database);
    let video = match authority.video(video_id).await {
        Ok(video) => video,
        Err(LegacyTranscriptRuntimeErrorV1::NotFound) => {
            return Ok(Ok(LegacyTranscriptResultV1::failure("Video not found")));
        }
        Err(_) => return Ok(Err(BrowserWebFailure::Unavailable)),
    };
    if !video.is_owner(Some(&actor_id)) {
        return Ok(Ok(LegacyTranscriptResultV1::failure(
            "You don't have permission to edit this transcript",
        )));
    }
    let object_key = video.original_object_key().expect("validated key");
    let operation = claim(
        &authority,
        LegacyTranscriptSurfaceV1::Edit,
        Some(&actor_id),
        &video,
        idempotency_key,
        &object_key,
        None,
        Some(entry_id),
        Some(new_text),
        now_ms,
    )
    .await?;
    if !browser_web_runtime::consume_session_grant_or_confirm_absent(&database, &proof, now_ms)
        .await?
    {
        return Ok(Err(BrowserWebFailure::Unauthenticated));
    }
    match operation.completed_result() {
        Ok(Some(result)) => return Ok(Ok(result)),
        Ok(None) => {}
        Err(error) => return Ok(Err(map_runtime(error))),
    }
    let Some((content, prior_etag, _)) = read_text_object(env, &object_key)
        .await
        .map_err(|_| worker::Error::RustError("transcript storage read failed".into()))?
    else {
        let result = LegacyTranscriptResultV1::failure("Transcript file not found");
        authority
            .complete(&operation, &result, now_ms)
            .await
            .map_err(map_worker)?;
        return Ok(Ok(result));
    };
    let (updated, changed) = legacy_update_vtt_entry_text(&content, entry_id, new_text);
    if !changed {
        let result = LegacyTranscriptResultV1::failure("Transcript entry not found");
        authority
            .complete(&operation, &result, now_ms)
            .await
            .map_err(map_worker)?;
        return Ok(Ok(result));
    }
    let bucket = env.bucket("RECORDINGS")?;
    let bytes = updated.as_bytes();
    let object = bucket
        .put(&object_key, bytes.to_vec())
        .http_metadata(HttpMetadata {
            content_type: Some("text/vtt".into()),
            cache_control: Some("private, no-store".into()),
            ..HttpMetadata::default()
        })
        .execute()
        .await?
        .ok_or_else(|| {
            worker::Error::RustError("transcript storage write was not applied".into())
        })?;
    let sha256 = hex_digest(bytes);
    authority
        .mark_storage_applied(
            &operation,
            Some(&prior_etag),
            &object.http_etag(),
            &sha256,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            now_ms,
        )
        .await
        .map_err(map_worker)?;
    let result = LegacyTranscriptResultV1 {
        success: true,
        content: None,
        translated_vtt: None,
        message: "Transcript entry updated successfully".into(),
    };
    authority
        .complete(&operation, &result, now_ms)
        .await
        .map_err(map_worker)?;
    Ok(Ok(result))
}

async fn translate_transcript(
    request: &Request,
    env: &Env,
    video_id: &str,
    target_language: &str,
    idempotency_key: &str,
    now_ms: i64,
) -> Result<BrowserWebOutcome<LegacyTranscriptResultV1>> {
    if env.secret("GROQ_API_KEY").is_err() && env.secret("FRAME_GROQ_API_KEY").is_err() {
        return Ok(Ok(LegacyTranscriptResultV1::failure(
            "Translation service not configured",
        )));
    }
    let actor_id = optional_actor(request, env, now_ms).await?;
    let database = env.d1("DB")?;
    if !admit_read(
        env,
        request,
        &database,
        CompatibilityRateLimitBucketV1::CollaborationNotifications,
        actor_id.as_deref(),
        now_ms,
    )
    .await?
    {
        return Ok(Ok(LegacyTranscriptResultV1::failure("Too many requests")));
    }
    let authority = D1LegacyTranscriptAuthorityV1::new(&database);
    let video = match visible_video(&authority, request, env, video_id, actor_id.as_deref()).await {
        Ok(video) => video,
        Err(
            LegacyTranscriptRuntimeErrorV1::NotFound | LegacyTranscriptRuntimeErrorV1::Unauthorized,
        ) => {
            return Ok(Ok(LegacyTranscriptResultV1::failure("Video not found")));
        }
        Err(_) => return Ok(Err(BrowserWebFailure::Unavailable)),
    };
    let target_key = video
        .translated_object_key(target_language)
        .ok_or_else(|| worker::Error::RustError("unsupported transcript language".into()))?;
    if let Ok(Some((content, _, _))) = read_text_object(env, &target_key).await {
        return Ok(Ok(LegacyTranscriptResultV1 {
            success: true,
            content: None,
            translated_vtt: Some(content),
            message: "Retrieved cached translation".into(),
        }));
    }
    let source_key = video.original_object_key().expect("validated key");
    if !matches!(read_text_object(env, &source_key).await, Ok(Some(_))) {
        return Ok(Ok(LegacyTranscriptResultV1::failure(
            "Original transcript not found",
        )));
    }
    let operation = claim(
        &authority,
        LegacyTranscriptSurfaceV1::Translate,
        actor_id.as_deref(),
        &video,
        idempotency_key,
        &target_key,
        Some(target_language),
        None,
        None,
        now_ms,
    )
    .await?;
    match operation.completed_result() {
        Ok(Some(result)) => return Ok(Ok(result)),
        Ok(None) => {}
        Err(error) => return Ok(Err(map_runtime(error))),
    }
    authority
        .queue_translation(&operation, &source_key, target_language, now_ms)
        .await
        .map_err(map_worker)?;
    // Provider execution is an explicit protected gate. The durable outbox is
    // locally complete, while the observable action stays on Cap's provider
    // failure branch until an approved worker records a translated object.
    Ok(Ok(LegacyTranscriptResultV1::failure("Translation failed")))
}

async fn visible_video(
    authority: &D1LegacyTranscriptAuthorityV1<'_>,
    request: &Request,
    env: &Env,
    video_id: &str,
    actor_id: Option<&str>,
) -> std::result::Result<LegacyTranscriptVideoV1, LegacyTranscriptRuntimeErrorV1> {
    let video = authority.video(video_id).await?;
    let verified =
        crate::legacy_video_properties_web_runtime::existing_password_hashes(request, env)
            .unwrap_or_default();
    if !authority.can_view(&video, actor_id, &verified).await? {
        return Err(LegacyTranscriptRuntimeErrorV1::Unauthorized);
    }
    Ok(video)
}

#[allow(clippy::too_many_arguments)]
async fn claim(
    authority: &D1LegacyTranscriptAuthorityV1<'_>,
    surface: LegacyTranscriptSurfaceV1,
    actor_id: Option<&str>,
    video: &LegacyTranscriptVideoV1,
    idempotency_key: &str,
    object_key: &str,
    target_language: Option<&str>,
    entry_id: Option<u64>,
    replacement_text: Option<&str>,
    now_ms: i64,
) -> Result<LegacyTranscriptOperationRowV1> {
    let actor_scope = digest_parts(&[
        b"actor-scope",
        actor_id.unwrap_or("anonymous").as_bytes(),
        idempotency_key.as_bytes(),
    ]);
    let idempotency_digest = digest_parts(&[b"idempotency", idempotency_key.as_bytes()]);
    let mut request_parts: Vec<&[u8]> = vec![
        surface.operation_id().as_bytes(),
        video.legacy_video_id.as_bytes(),
        object_key.as_bytes(),
    ];
    if let Some(value) = target_language {
        request_parts.push(value.as_bytes());
    }
    let entry_text = entry_id.map(|value| value.to_string());
    if let Some(value) = entry_text.as_deref() {
        request_parts.push(value.as_bytes());
    }
    if let Some(value) = replacement_text {
        request_parts.push(value.as_bytes());
    }
    let request_digest = digest_parts(&request_parts);
    authority
        .claim_operation(&NewLegacyTranscriptOperationV1 {
            operation_id: &Uuid::new_v4().to_string(),
            surface,
            actor_scope_digest: &actor_scope,
            actor_id,
            video,
            idempotency_key_digest: &idempotency_digest,
            request_digest: &request_digest,
            object_key,
            target_language,
            entry_id,
            replacement_text,
            now_ms,
        })
        .await
        .map_err(map_worker)
}

async fn optional_actor(request: &Request, env: &Env, now_ms: i64) -> Result<Option<String>> {
    match browser_web_runtime::authenticate_compatibility_read(request, env, now_ms).await? {
        Ok(actor_id) => Ok(Some(actor_id)),
        Err(BrowserWebFailure::Unauthenticated | BrowserWebFailure::Invalid) => Ok(None),
        Err(_) => Err(worker::Error::RustError(
            "optional transcript authentication unavailable".into(),
        )),
    }
}

async fn admit_read(
    env: &Env,
    request: &Request,
    database: &worker::D1Database,
    bucket: CompatibilityRateLimitBucketV1,
    actor_id: Option<&str>,
    now_ms: i64,
) -> Result<bool> {
    let decision = match actor_id {
        Some(actor_id) => {
            compatibility_rate_limit::admit_principal(env, database, bucket, actor_id, now_ms)
                .await?
        }
        None => compatibility_rate_limit::admit_edge_request(env, request, bucket, now_ms).await?,
    };
    Ok(!rejected(decision))
}

async fn read_text_object(env: &Env, key: &str) -> Result<Option<(String, String, u64)>> {
    let Some(object) = env.bucket("RECORDINGS")?.get(key).execute().await? else {
        return Ok(None);
    };
    if object.size() > LEGACY_TRANSCRIPT_MAX_OBJECT_BYTES {
        return Err(worker::Error::RustError(
            "transcript object exceeds local bound".into(),
        ));
    }
    let etag = object.http_etag();
    let size = object.size();
    let text = object
        .body()
        .ok_or_else(|| worker::Error::RustError("transcript object has no body".into()))?
        .text()
        .await?;
    Ok(Some((text, etag, size)))
}

fn require_schema(value: &str) -> BrowserWebOutcome<()> {
    (value == LEGACY_TRANSCRIPT_ACTION_SCHEMA_V1)
        .then_some(())
        .ok_or(BrowserWebFailure::Invalid)
}

fn require_video_id(value: &str) -> BrowserWebOutcome<()> {
    (!value.is_empty() && value.len() <= 1020 && !value.chars().any(char::is_control))
        .then_some(())
        .ok_or(BrowserWebFailure::Invalid)
}

fn rejected(decision: frame_application::RateLimitDecisionV1) -> bool {
    matches!(
        decision,
        frame_application::RateLimitDecisionV1::Rejected { .. }
    )
}

fn digest_parts(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-legacy-transcript-v1\0");
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    hex_bytes(&digest.finalize())
}

fn hex_digest(bytes: &[u8]) -> String {
    hex_bytes(&Sha256::digest(bytes))
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 15)]));
    }
    output
}

fn map_runtime(error: LegacyTranscriptRuntimeErrorV1) -> BrowserWebFailure {
    match error {
        LegacyTranscriptRuntimeErrorV1::Invalid => BrowserWebFailure::Invalid,
        LegacyTranscriptRuntimeErrorV1::NotFound => BrowserWebFailure::NotFound,
        LegacyTranscriptRuntimeErrorV1::Unauthorized => BrowserWebFailure::Forbidden,
        LegacyTranscriptRuntimeErrorV1::Conflict => BrowserWebFailure::Conflict,
        LegacyTranscriptRuntimeErrorV1::Unavailable | LegacyTranscriptRuntimeErrorV1::Corrupt => {
            BrowserWebFailure::Unavailable
        }
    }
}

fn map_worker(error: LegacyTranscriptRuntimeErrorV1) -> worker::Error {
    worker::Error::RustError(format!("legacy transcript authority failed: {error:?}"))
}

fn exact_json(status: u16, body: Value) -> Result<Response> {
    let mut response = Response::from_json(&body)?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "private, no-store")?;
    response
        .headers_mut()
        .set("x-content-type-options", "nosniff")?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_wire_corrects_optional_auth_read_and_keeps_mutation_idempotency() {
        assert!(matches!(
            decode_action_bytes(
                LegacyTranscriptSurfaceV1::Get,
                br#"{"schema_version":"frame.web-transcript-action-request.v1","video_id":"video"}"#,
            ),
            Ok(DecodedLegacyTranscriptActionV1::Get { .. })
        ));
        assert!(matches!(
            decode_action_bytes(
                LegacyTranscriptSurfaceV1::AvailableTranslations,
                br#"{"schema_version":"frame.web-transcript-action-request.v1","video_id":"video"}"#,
            ),
            Ok(DecodedLegacyTranscriptActionV1::AvailableTranslations { .. })
        ));
        assert!(decode_action_bytes(
            LegacyTranscriptSurfaceV1::Edit,
            br#"{"schema_version":"frame.web-transcript-action-request.v1","video_id":"video","entry_id":1,"new_text":"new","idempotency_key":"0190f5de-3000-7000-8000-000000000001"}"#,
        )
        .is_ok());
        assert!(decode_action_bytes(
            LegacyTranscriptSurfaceV1::Translate,
            br#"{"schema_version":"frame.web-transcript-action-request.v1","video_id":"video","target_language":"xx","idempotency_key":"0190f5de-3000-7000-8000-000000000001"}"#,
        )
        .is_err());
    }

    #[test]
    fn operation_ids_are_closed() {
        assert!(is_action(LEGACY_EDIT_TRANSCRIPT_OPERATION_ID));
        assert!(is_action(LEGACY_GET_TRANSCRIPT_OPERATION_ID));
        assert!(is_action(LEGACY_AVAILABLE_TRANSLATIONS_OPERATION_ID));
        assert!(is_action(LEGACY_TRANSLATE_TRANSCRIPT_OPERATION_ID));
        assert!(!is_action(
            frame_application::LEGACY_RETRY_TRANSCRIPTION_OPERATION_ID
        ));
        assert_eq!(
            DecodedLegacyTranscriptActionV1::Get {
                video_id: "v".into()
            }
            .surface(),
            LegacyTranscriptSurfaceV1::Get
        );
    }
}
