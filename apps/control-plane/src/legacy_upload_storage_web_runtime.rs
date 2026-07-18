//! Effect-RPC and same-origin browser ingress for upload/storage compatibility.

use frame_application::{
    LEGACY_UPLOAD_STORAGE_ACTION_SCHEMA_V1, LEGACY_UPLOAD_STORAGE_MAX_BODY_BYTES,
    LegacyCreateVideoUploadInputV1, LegacyDownloadInfoResultV1, LegacyDownloadSuccessV1,
    LegacyEffectOptionV1, LegacyMobileStorageObjectV1, LegacyUploadProgressV1,
    LegacyUploadStorageActionV1, legacy_mobile_file_extension, legacy_mobile_iso_from_millis,
    legacy_mobile_screenshot_key, legacy_upload_storage_iso_millis,
};
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{Bucket, Env, Request, Response, Result, send::IntoSendFuture};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    direct_upload_signer,
    legacy_upload_storage_runtime::{
        D1LegacyUploadStorageV1, LegacyUploadStorageFailureV1, LegacyUploadStorageReadAuthorityV1,
    },
};

const RPC_PARSE_FAILURE: &str = "Invalid Effect RPC request payload";
const RPC_UNKNOWN_TAG_FAILURE: &str = "Unknown Effect RPC request tag";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DecodedUploadStorageActionV1 {
    Create {
        input: LegacyCreateVideoUploadInputV1,
        idempotency_key: String,
    },
    Delete {
        video_id: String,
        idempotency_key: String,
    },
    Download {
        video_id: String,
    },
    DownloadInfo {
        video_id: String,
        variant: String,
    },
    Reconcile {
        video_id: String,
        idempotency_key: String,
    },
    Share {
        video_id: String,
        space_ids: Vec<String>,
        public: Option<bool>,
        idempotency_key: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateWireV1 {
    schema_version: String,
    idempotency_key: String,
    #[serde(flatten)]
    input: LegacyCreateVideoUploadInputV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationVideoWireV1 {
    schema_version: String,
    video_id: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadVideoWireV1 {
    schema_version: String,
    video_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadInfoWireV1 {
    schema_version: String,
    video_id: String,
    #[serde(default = "current_variant")]
    variant: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareWireV1 {
    schema_version: String,
    cap_id: String,
    space_ids: Vec<String>,
    public: Option<bool>,
    idempotency_key: String,
}

#[derive(Debug, Clone)]
enum RpcInputV1 {
    GetProgress {
        video_id: String,
    },
    UpdateProgress {
        video_id: String,
        uploaded: u64,
        total: u64,
        updated_at_ms: i64,
    },
    DownloadInfo {
        video_id: String,
    },
}

#[derive(Debug, Clone)]
struct DecodedRpcV1 {
    id: String,
    input: RpcInputV1,
}

#[must_use]
pub(crate) fn is_action(operation_id: &str) -> bool {
    LegacyUploadStorageActionV1::parse(operation_id).is_some()
}

#[must_use]
pub(crate) fn is_upload_storage_rpc_request(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("tag").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|tag| {
            matches!(
                tag.as_str(),
                "GetUploadProgress" | "VideoUploadProgressUpdate" | "VideoGetDownloadInfo"
            )
        })
}

pub(crate) async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedUploadStorageActionV1>> {
    let Some(action) = LegacyUploadStorageActionV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    if request.headers().get("idempotency-key")?.is_some()
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
        .map_err(|_| worker::Error::RustError("invalid upload storage body length".into()))?;
    if declared.is_some_and(|value| value == 0 || value > LEGACY_UPLOAD_STORAGE_MAX_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes = match crate::read_bounded_legacy_body(request, LEGACY_UPLOAD_STORAGE_MAX_BODY_BYTES)
        .await
    {
        Ok(bytes) if !bytes.is_empty() && declared.is_none_or(|value| value == bytes.len()) => {
            bytes
        }
        _ => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    Ok(decode_action_bytes(action, &bytes))
}

pub(crate) async fn action(
    request: &Request,
    env: &Env,
    decoded: &DecodedUploadStorageActionV1,
    now_ms: i64,
) -> Result<BrowserWebOutcome<Value>> {
    let database = env.d1("DB")?;
    let repository = D1LegacyUploadStorageV1::new(&database);
    let mutation = matches!(
        decoded,
        DecodedUploadStorageActionV1::Create { .. }
            | DecodedUploadStorageActionV1::Delete { .. }
            | DecodedUploadStorageActionV1::Reconcile { .. }
            | DecodedUploadStorageActionV1::Share { .. }
    );
    let (actor_id, proof) = if mutation {
        match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms).await?
        {
            Ok(proof) => (proof.user_id().to_string(), Some(proof)),
            Err(failure) => {
                if matches!(decoded, DecodedUploadStorageActionV1::Share { .. }) {
                    return Ok(Ok(json!({"success":false,"error":"Unauthorized"})));
                }
                return Ok(Err(failure));
            }
        }
    } else {
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => (actor_id, None),
            Err(failure) => return Ok(Err(failure)),
        }
    };
    let rate_limit = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::UploadStorage,
        &actor_id,
        now_ms,
    )
    .await?;
    if matches!(
        rate_limit,
        frame_application::RateLimitDecisionV1::Rejected { .. }
    ) {
        consume_proof(&database, proof.as_ref(), now_ms).await?;
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let outcome = match decoded {
        DecodedUploadStorageActionV1::Create {
            input,
            idempotency_key,
        } => {
            let Some(signer) = direct_upload_signer(env) else {
                return Ok(Err(BrowserWebFailure::Unavailable));
            };
            repository
                .create_upload(
                    &actor_id,
                    input,
                    idempotency_key,
                    videos_default_public(env)?,
                    &signer,
                    now_ms,
                )
                .await
                .map(|value| serde_json::to_value(value).expect("serializable upload result"))
        }
        DecodedUploadStorageActionV1::Delete {
            video_id,
            idempotency_key,
        } => {
            let plan = match repository
                .begin_delete_result(&actor_id, video_id, idempotency_key, now_ms)
                .await
            {
                Ok(plan) => plan,
                Err(failure) => {
                    consume_proof(&database, proof.as_ref(), now_ms).await?;
                    return Ok(Err(map_failure(failure)));
                }
            };
            consume_proof(&database, proof.as_ref(), now_ms).await?;
            if !plan.replayed_complete {
                let bucket = env.bucket("RECORDINGS")?;
                if delete_with_retry(&bucket, &plan.object_key).await.is_err() {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                if let Err(failure) = repository
                    .finish_delete_result(&plan.operation_id, now_ms)
                    .await
                {
                    return Ok(Err(map_failure(failure)));
                }
            }
            return Ok(Ok(json!({"success":true})));
        }
        DecodedUploadStorageActionV1::Download { video_id } => {
            download_video(&repository, env, &actor_id, video_id, now_ms).await
        }
        DecodedUploadStorageActionV1::DownloadInfo { video_id, variant } => {
            download_info(&repository, env, &actor_id, video_id, variant, now_ms).await
        }
        DecodedUploadStorageActionV1::Reconcile {
            video_id,
            idempotency_key,
        } => repository
            .reconcile_edit(&actor_id, video_id, idempotency_key, now_ms)
            .await
            .map(Value::Bool),
        DecodedUploadStorageActionV1::Share {
            video_id,
            space_ids,
            public,
            idempotency_key,
        } => match repository
            .share_cap(
                &actor_id,
                video_id,
                space_ids,
                *public,
                idempotency_key,
                now_ms,
            )
            .await
        {
            Ok(()) => Ok(json!({"success":true})),
            Err(
                LegacyUploadStorageFailureV1::Forbidden | LegacyUploadStorageFailureV1::NotFound,
            ) => Ok(json!({"success":false,"error":"Unauthorized"})),
            Err(_) => Ok(json!({
                "success":false,
                "error":"Failed to update sharing settings"
            })),
        },
    };
    if mutation {
        consume_proof(&database, proof.as_ref(), now_ms).await?;
    }
    Ok(outcome.map_err(map_failure))
}

pub(crate) async fn effect_rpc_response_from_bytes(
    bytes: &[u8],
    request: &Request,
    env: &Env,
    _request_id: &str,
) -> Result<Response> {
    let decoded = match decode_rpc_request(bytes) {
        Ok(decoded) => decoded,
        Err(RpcDecodeFailureV1::Malformed(Some(id))) => {
            return rpc_response(rpc_die(&id, RPC_PARSE_FAILURE));
        }
        Err(RpcDecodeFailureV1::Malformed(None)) => {
            return rpc_response(rpc_defect(RPC_PARSE_FAILURE));
        }
        Err(RpcDecodeFailureV1::UnknownTag) => {
            return rpc_response(rpc_defect(RPC_UNKNOWN_TAG_FAILURE));
        }
    };
    let now_ms = crate::current_time_ms()?;
    let actor =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor) => Some(actor),
            Err(BrowserWebFailure::Unauthenticated) => None,
            Err(_) => {
                return rpc_response(rpc_typed_failure(
                    &decoded.id,
                    json!({"_tag":"InternalError","type":"database"}),
                ));
            }
        };
    if matches!(decoded.input, RpcInputV1::UpdateProgress { .. }) && actor.is_none() {
        return rpc_response(rpc_typed_failure(
            &decoded.id,
            json!({"_tag":"UnauthenticatedError"}),
        ));
    }
    let database = env.d1("DB")?;
    let rate = match actor.as_deref() {
        Some(actor) => {
            compatibility_rate_limit::admit_principal(
                env,
                &database,
                CompatibilityRateLimitBucketV1::UploadStorage,
                actor,
                now_ms,
            )
            .await?
        }
        None => {
            compatibility_rate_limit::admit_edge_request(
                env,
                request,
                CompatibilityRateLimitBucketV1::UploadStorage,
                now_ms,
            )
            .await?
        }
    };
    if matches!(
        rate,
        frame_application::RateLimitDecisionV1::Rejected { .. }
    ) {
        return rpc_response(rpc_typed_failure(
            &decoded.id,
            json!({"_tag":"InternalError","type":"unknown"}),
        ));
    }
    let repository = D1LegacyUploadStorageV1::new(&database);
    let verified_password_hashes =
        crate::legacy_video_properties_web_runtime::existing_password_hashes(request, env)
            .unwrap_or_default();
    let value = match decoded.input {
        RpcInputV1::GetProgress { video_id } => {
            match repository
                .read_authority(actor.as_deref(), &video_id, &verified_password_hashes)
                .await
            {
                Ok(authority) => match progress_value(&authority) {
                    Ok(value) => rpc_success(&decoded.id, value),
                    Err(failure) => rpc_failure(&decoded.id, failure),
                },
                Err(failure) => rpc_failure(&decoded.id, failure),
            }
        }
        RpcInputV1::UpdateProgress {
            video_id,
            uploaded,
            total,
            updated_at_ms,
        } => match repository
            .progress_update(
                actor.as_deref().expect("required"),
                &video_id,
                uploaded,
                total,
                updated_at_ms,
                &decoded.id,
                now_ms,
            )
            .await
        {
            Ok(value) => rpc_success(&decoded.id, Value::Bool(value)),
            Err(failure) => rpc_failure(&decoded.id, failure),
        },
        RpcInputV1::DownloadInfo { video_id } => {
            match repository
                .read_authority(actor.as_deref(), &video_id, &verified_password_hashes)
                .await
            {
                Ok(authority) => match rpc_download_info(env, &authority, now_ms).await {
                    Ok(value) => rpc_success(&decoded.id, value),
                    Err(failure) => rpc_failure(&decoded.id, failure),
                },
                Err(failure) => rpc_failure(&decoded.id, failure),
            }
        }
    };
    rpc_response(value)
}

fn decode_action_bytes(
    action: LegacyUploadStorageActionV1,
    bytes: &[u8],
) -> BrowserWebOutcome<DecodedUploadStorageActionV1> {
    match action {
        LegacyUploadStorageActionV1::CreateVideoAndGetUploadUrl => {
            let wire: CreateWireV1 =
                serde_json::from_slice(bytes).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            if !wire.input.valid() || !valid_idempotency(&wire.idempotency_key) {
                return Err(BrowserWebFailure::Invalid);
            }
            Ok(DecodedUploadStorageActionV1::Create {
                input: wire.input,
                idempotency_key: wire.idempotency_key,
            })
        }
        LegacyUploadStorageActionV1::DeleteVideoResultFile
        | LegacyUploadStorageActionV1::ReconcileStaleEditUpload => {
            let wire: MutationVideoWireV1 =
                serde_json::from_slice(bytes).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            if !frame_application::valid_cap_id(&wire.video_id)
                || !valid_idempotency(&wire.idempotency_key)
            {
                return Err(BrowserWebFailure::Invalid);
            }
            Ok(
                if matches!(action, LegacyUploadStorageActionV1::DeleteVideoResultFile) {
                    DecodedUploadStorageActionV1::Delete {
                        video_id: wire.video_id,
                        idempotency_key: wire.idempotency_key,
                    }
                } else {
                    DecodedUploadStorageActionV1::Reconcile {
                        video_id: wire.video_id,
                        idempotency_key: wire.idempotency_key,
                    }
                },
            )
        }
        LegacyUploadStorageActionV1::DownloadVideo => {
            let wire: ReadVideoWireV1 =
                serde_json::from_slice(bytes).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            frame_application::valid_cap_id(&wire.video_id)
                .then_some(DecodedUploadStorageActionV1::Download {
                    video_id: wire.video_id,
                })
                .ok_or(BrowserWebFailure::Invalid)
        }
        LegacyUploadStorageActionV1::GetVideoDownloadInfo => {
            let wire: DownloadInfoWireV1 =
                serde_json::from_slice(bytes).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            if !frame_application::valid_cap_id(&wire.video_id)
                || !matches!(wire.variant.as_str(), "current" | "original")
            {
                return Err(BrowserWebFailure::Invalid);
            }
            Ok(DecodedUploadStorageActionV1::DownloadInfo {
                video_id: wire.video_id,
                variant: wire.variant,
            })
        }
        LegacyUploadStorageActionV1::ShareCap => {
            let wire: ShareWireV1 =
                serde_json::from_slice(bytes).map_err(|_| BrowserWebFailure::Invalid)?;
            require_schema(&wire.schema_version)?;
            if !frame_application::valid_cap_id(&wire.cap_id)
                || !valid_idempotency(&wire.idempotency_key)
            {
                return Err(BrowserWebFailure::Invalid);
            }
            Ok(DecodedUploadStorageActionV1::Share {
                video_id: wire.cap_id,
                space_ids: wire.space_ids,
                public: wire.public,
                idempotency_key: wire.idempotency_key,
            })
        }
    }
}

async fn download_video(
    repository: &D1LegacyUploadStorageV1<'_>,
    env: &Env,
    actor: &str,
    video_id: &str,
    now_ms: i64,
) -> Result<Value, LegacyUploadStorageFailureV1> {
    let authority = repository.owner_authority(actor, video_id).await?;
    let signer = direct_upload_signer(env).ok_or(LegacyUploadStorageFailureV1::Unavailable)?;
    let key = format!("{}result.mp4", authority.object_prefix);
    let capability = signer
        .sign_legacy_storage_get(
            &key,
            u64::try_from(now_ms).map_err(|_| LegacyUploadStorageFailureV1::Invalid)?,
            3_600,
        )
        .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
    serde_json::to_value(LegacyDownloadSuccessV1 {
        success: true,
        download_url: capability.url,
        filename: format!("{}.mp4", authority.title),
    })
    .map_err(|_| LegacyUploadStorageFailureV1::Corrupt)
}

async fn download_info(
    repository: &D1LegacyUploadStorageV1<'_>,
    env: &Env,
    actor: &str,
    video_id: &str,
    variant: &str,
    now_ms: i64,
) -> Result<Value, LegacyUploadStorageFailureV1> {
    let authority = repository.download_authority(actor, video_id).await?;
    if variant == "current" {
        if authority.source_type == "desktopSegments" {
            return download_failure("Video is still processing. Try again once it has finished.");
        }
        if authority
            .phase
            .as_deref()
            .is_some_and(|phase| phase != "complete")
        {
            return download_failure(if authority.phase.as_deref() == Some("error") {
                "Video processing failed before the MP4 was ready."
            } else {
                "Video is still processing. Try again once it has finished."
            });
        }
    }
    let (key, filename, unavailable) = if variant == "original" {
        let Some(key) = authority.edit_source_key.clone() else {
            return download_failure("Original video is no longer available.");
        };
        (
            key,
            format!("{} (original).mp4", authority.title),
            "Original video is no longer available.",
        )
    } else {
        (
            format!("{}result.mp4", authority.object_prefix),
            format!("{}.mp4", authority.title),
            "Video file is not available for download yet.",
        )
    };
    let prepared = async {
        let bucket = env
            .bucket("RECORDINGS")
            .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
        let exists = bucket
            .head(&key)
            .into_send()
            .await
            .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?
            .is_some();
        if !exists {
            return Ok(None);
        }
        let signer = direct_upload_signer(env).ok_or(LegacyUploadStorageFailureV1::Unavailable)?;
        let capability = signer
            .sign_legacy_storage_get(
                &key,
                u64::try_from(now_ms).map_err(|_| LegacyUploadStorageFailureV1::Invalid)?,
                3_600,
            )
            .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
        Ok::<_, LegacyUploadStorageFailureV1>(Some(capability.url))
    }
    .await;
    let download_url = match prepared {
        Ok(Some(download_url)) => download_url,
        Ok(None) => return download_failure(unavailable),
        Err(_) => return download_failure("Failed to prepare the video download."),
    };
    serde_json::to_value(LegacyDownloadInfoResultV1::Success(
        LegacyDownloadSuccessV1 {
            success: true,
            download_url,
            filename,
        },
    ))
    .map_err(|_| LegacyUploadStorageFailureV1::Corrupt)
}

fn download_failure(message: &str) -> Result<Value, LegacyUploadStorageFailureV1> {
    serde_json::to_value(LegacyDownloadInfoResultV1::Failure {
        success: false,
        error: message.into(),
    })
    .map_err(|_| LegacyUploadStorageFailureV1::Corrupt)
}

async fn rpc_download_info(
    env: &Env,
    authority: &LegacyUploadStorageReadAuthorityV1,
    now_ms: i64,
) -> Result<Value, LegacyUploadStorageFailureV1> {
    let bucket = env
        .bucket("RECORDINGS")
        .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
    let signer = direct_upload_signer(env).ok_or(LegacyUploadStorageFailureV1::Unavailable)?;
    let selected = if authority.legacy_is_screenshot == 1 {
        let listed = bucket
            .list()
            .limit(1_000)
            .prefix(&authority.object_prefix)
            .execute()
            .into_send()
            .await
            .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
        let objects = listed
            .objects()
            .into_iter()
            .map(|object| LegacyMobileStorageObjectV1 {
                key: object.key(),
                last_modified_ms: i64::try_from(object.uploaded().as_millis()).ok(),
            })
            .collect::<Vec<_>>();
        legacy_mobile_screenshot_key(&objects).map(|key| {
            (
                key.clone(),
                format!(
                    "{}.{}",
                    authority.title,
                    legacy_mobile_file_extension(&key).unwrap_or_else(|| "jpg".into())
                ),
            )
        })
    } else if authority.source_type == "webMP4" {
        let result_key = format!("{}result.mp4", authority.object_prefix);
        if bucket
            .head(&result_key)
            .into_send()
            .await
            .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?
            .is_some_and(|object| object.size() > 0)
        {
            Some((result_key, format!("{}.mp4", authority.title)))
        } else {
            authority.raw_file_key.as_ref().map(|key| {
                (
                    key.clone(),
                    format!(
                        "{}.{}",
                        authority.title,
                        legacy_mobile_file_extension(key).unwrap_or_else(|| "mp4".into())
                    ),
                )
            })
        }
    } else if authority.source_type == "desktopMP4" {
        Some((
            format!("{}result.mp4", authority.object_prefix),
            format!("{}.mp4", authority.title),
        ))
    } else {
        None
    };
    let Some((key, file_name)) = selected else {
        return Ok(json!({"_id":"Option","_tag":"None"}));
    };
    let capability = signer
        .sign_legacy_storage_get(
            &key,
            u64::try_from(now_ms).map_err(|_| LegacyUploadStorageFailureV1::Invalid)?,
            3_600,
        )
        .map_err(|_| LegacyUploadStorageFailureV1::Unavailable)?;
    Ok(
        json!({"_id":"Option","_tag":"Some","value":{"fileName":file_name,"downloadUrl":capability.url}}),
    )
}

fn progress_value(
    authority: &LegacyUploadStorageReadAuthorityV1,
) -> Result<Value, LegacyUploadStorageFailureV1> {
    let Some(uploaded) = authority.uploaded else {
        return Ok(json!({"_id":"Option","_tag":"None"}));
    };
    let total = exact_u64(
        authority
            .total
            .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
    )?;
    let uploaded = exact_u64(uploaded)?;
    let progress = exact_u64(
        authority
            .processing_progress
            .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
    )?;
    let value = LegacyUploadProgressV1 {
        uploaded,
        total,
        started_at: legacy_mobile_iso_from_millis(
            authority
                .started_at_ms
                .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
        )
        .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
        updated_at: legacy_mobile_iso_from_millis(
            authority
                .upload_updated_at_ms
                .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
        )
        .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
        phase: authority
            .phase
            .clone()
            .ok_or(LegacyUploadStorageFailureV1::Corrupt)?,
        processing_progress: progress,
        processing_message: LegacyEffectOptionV1::from(authority.processing_message.clone()),
        processing_error: LegacyEffectOptionV1::from(authority.processing_error.clone()),
        has_raw_fallback: authority.raw_file_key.is_some(),
    };
    Ok(
        json!({"_id":"Option","_tag":"Some","value":serde_json::to_value(value).map_err(|_| LegacyUploadStorageFailureV1::Corrupt)?}),
    )
}

fn exact_u64(value: f64) -> Result<u64, LegacyUploadStorageFailureV1> {
    (value.is_finite() && (0.0..=9_007_199_254_740_991.0).contains(&value) && value.fract() == 0.0)
        .then_some(value as u64)
        .ok_or(LegacyUploadStorageFailureV1::Corrupt)
}

async fn delete_with_retry(bucket: &Bucket, key: &str) -> std::result::Result<(), ()> {
    for _ in 0..3 {
        if bucket.delete(key).into_send().await.is_ok() {
            return Ok(());
        }
    }
    Err(())
}

async fn consume_proof(
    database: &worker::D1Database,
    proof: Option<&frame_application::ValidatedBrowserMutationProof>,
    now_ms: i64,
) -> Result<()> {
    if let Some(proof) = proof
        && !browser_web_runtime::consume_session_grant(database, proof, now_ms).await?
    {
        return Err(worker::Error::RustError(
            "legacy upload storage proof was not consumed".into(),
        ));
    }
    Ok(())
}

fn map_failure(failure: LegacyUploadStorageFailureV1) -> BrowserWebFailure {
    match failure {
        LegacyUploadStorageFailureV1::Invalid => BrowserWebFailure::Invalid,
        LegacyUploadStorageFailureV1::Forbidden
        | LegacyUploadStorageFailureV1::PasswordNotProvided(_)
        | LegacyUploadStorageFailureV1::PasswordWrong(_) => BrowserWebFailure::Forbidden,
        LegacyUploadStorageFailureV1::NotFound => BrowserWebFailure::NotFound,
        LegacyUploadStorageFailureV1::Conflict => BrowserWebFailure::Conflict,
        LegacyUploadStorageFailureV1::UpgradeRequired
        | LegacyUploadStorageFailureV1::Corrupt
        | LegacyUploadStorageFailureV1::Unavailable => BrowserWebFailure::Unavailable,
    }
}

fn decode_rpc_request(bytes: &[u8]) -> std::result::Result<DecodedRpcV1, RpcDecodeFailureV1> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| RpcDecodeFailureV1::Malformed(None))?;
    let object = value
        .as_object()
        .ok_or(RpcDecodeFailureV1::Malformed(None))?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| valid_rpc_id(value))
        .map(str::to_owned)
        .ok_or(RpcDecodeFailureV1::Malformed(None))?;
    let malformed = || RpcDecodeFailureV1::Malformed(Some(id.clone()));
    if object.get("_tag").and_then(Value::as_str) != Some("Request")
        || !valid_rpc_headers(object.get("headers"))
    {
        return Err(malformed());
    }
    let tag = object
        .get("tag")
        .and_then(Value::as_str)
        .ok_or_else(malformed)?;
    let payload = object.get("payload").ok_or_else(malformed)?;
    let input = match tag {
        "GetUploadProgress" => RpcInputV1::GetProgress {
            video_id: cap_id(payload).ok_or_else(malformed)?,
        },
        "VideoGetDownloadInfo" => RpcInputV1::DownloadInfo {
            video_id: cap_id(payload).ok_or_else(malformed)?,
        },
        "VideoUploadProgressUpdate" => {
            let payload = payload.as_object().ok_or_else(malformed)?;
            let video_id = payload
                .get("videoId")
                .and_then(Value::as_str)
                .filter(|value| frame_application::valid_cap_id(value))
                .map(str::to_owned)
                .ok_or_else(malformed)?;
            let uploaded = payload
                .get("uploaded")
                .and_then(Value::as_u64)
                .filter(|value| *value <= 9_007_199_254_740_991)
                .ok_or_else(malformed)?;
            let total = payload
                .get("total")
                .and_then(Value::as_u64)
                .filter(|value| *value <= 9_007_199_254_740_991)
                .ok_or_else(malformed)?;
            let updated_at = payload
                .get("updatedAt")
                .and_then(Value::as_str)
                .ok_or_else(malformed)?;
            let parsed = legacy_upload_storage_iso_millis(updated_at).ok_or_else(malformed)?;
            RpcInputV1::UpdateProgress {
                video_id,
                uploaded,
                total,
                updated_at_ms: parsed,
            }
        }
        _ => return Err(RpcDecodeFailureV1::UnknownTag),
    };
    Ok(DecodedRpcV1 { id, input })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RpcDecodeFailureV1 {
    Malformed(Option<String>),
    UnknownTag,
}

fn cap_id(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| frame_application::valid_cap_id(value))
        .map(str::to_owned)
}
fn valid_rpc_id(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.len() <= 256 && digits.bytes().all(|byte| byte.is_ascii_digit())
}
fn valid_rpc_headers(value: Option<&Value>) -> bool {
    value.and_then(Value::as_array).is_some_and(|headers| {
        headers.iter().all(|entry| {
            entry.as_array().is_some_and(|pair| {
                pair.len() == 2 && pair.iter().all(|value| value.as_str().is_some())
            })
        })
    })
}
fn valid_idempotency(value: &str) -> bool {
    (1..=255).contains(&value.len()) && !value.chars().any(char::is_control)
}
fn require_schema(value: &str) -> BrowserWebOutcome<()> {
    (value == LEGACY_UPLOAD_STORAGE_ACTION_SCHEMA_V1)
        .then_some(())
        .ok_or(BrowserWebFailure::Invalid)
}
fn current_variant() -> String {
    "current".into()
}

fn rpc_success(id: &str, value: Value) -> Value {
    json!([{"_tag":"Exit","requestId":id,"exit":{"_tag":"Success","value":value}}])
}
fn rpc_typed_failure(id: &str, error: Value) -> Value {
    json!([{"_tag":"Exit","requestId":id,"exit":{"_tag":"Failure","cause":{"_tag":"Fail","error":error}}}])
}
fn rpc_die(id: &str, message: &str) -> Value {
    json!([{"_tag":"Exit","requestId":id,"exit":{"_tag":"Failure","cause":{"_tag":"Die","defect":message}}}])
}
fn rpc_defect(message: &str) -> Value {
    json!([{"_tag":"Defect","defect":message}])
}
fn rpc_failure(id: &str, failure: LegacyUploadStorageFailureV1) -> Value {
    let error = match failure {
        LegacyUploadStorageFailureV1::NotFound => json!({"_tag":"VideoNotFoundError"}),
        LegacyUploadStorageFailureV1::Forbidden => json!({"_tag":"PolicyDenied"}),
        LegacyUploadStorageFailureV1::PasswordNotProvided(id) => {
            json!({"_tag":"VerifyVideoPasswordError","id":id,"cause":"not-provided"})
        }
        LegacyUploadStorageFailureV1::PasswordWrong(id) => {
            json!({"_tag":"VerifyVideoPasswordError","id":id,"cause":"wrong-password"})
        }
        LegacyUploadStorageFailureV1::Corrupt | LegacyUploadStorageFailureV1::Unavailable => {
            json!({"_tag":"InternalError","type":"database"})
        }
        _ => json!({"_tag":"InternalError","type":"unknown"}),
    };
    rpc_typed_failure(id, error)
}

fn rpc_response(value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn videos_default_public(env: &Env) -> Result<bool> {
    let value = env
        .secret("CAP_VIDEOS_DEFAULT_PUBLIC")
        .map(|value| value.to_string())
        .or_else(|_| {
            env.var("CAP_VIDEOS_DEFAULT_PUBLIC")
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|_| "true".into());
    match value.as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(worker::Error::RustError(
            "CAP_VIDEOS_DEFAULT_PUBLIC is invalid".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_rpc_decoder_preserves_timestamp_and_clamps_later() {
        let decoded = decode_rpc_request(br#"{"_tag":"Request","id":"7","tag":"VideoUploadProgressUpdate","payload":{"videoId":"0123456789abcde","uploaded":9,"total":4,"updatedAt":"2025-01-02T03:04:05.006Z"},"headers":[]}"#).expect("decode");
        assert!(matches!(
            decoded.input,
            RpcInputV1::UpdateProgress {
                uploaded: 9,
                total: 4,
                ..
            }
        ));
    }

    #[test]
    fn action_decoders_require_frame_replay_carrier_only_for_mutations() {
        let delete = decode_action_bytes(LegacyUploadStorageActionV1::DeleteVideoResultFile, br#"{"schema_version":"frame.web-upload-storage-action-request.v1","video_id":"0123456789abcde","idempotency_key":"retry-1"}"#).expect("delete");
        assert!(matches!(
            delete,
            DecodedUploadStorageActionV1::Delete { .. }
        ));
        let read = decode_action_bytes(LegacyUploadStorageActionV1::DownloadVideo, br#"{"schema_version":"frame.web-upload-storage-action-request.v1","video_id":"0123456789abcde"}"#).expect("read");
        assert!(matches!(
            read,
            DecodedUploadStorageActionV1::Download { .. }
        ));
    }
}
