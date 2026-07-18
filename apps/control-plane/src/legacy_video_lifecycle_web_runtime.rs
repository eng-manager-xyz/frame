//! Exact HTTP and Effect-RPC carriers for provider-free video lifecycle work.

use std::fmt::Write as _;

use frame_application::{
    LEGACY_VIDEO_LIFECYCLE_CONTENT_TYPE, LEGACY_VIDEO_LIFECYCLE_MAX_BODY_BYTES,
    LegacyExtensionInstantCreateInputV1, LegacyOrganisationImagePatchV1, LegacyOrganisationImageV1,
    LegacyVideoLifecycleSurfaceV1, legacy_organisation_icon_key,
    legacy_video_lifecycle_object_prefix, legacy_video_lifecycle_valid_cap_id,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use worker::{Env, HttpMetadata, Request, Response, Result, send::IntoSendFuture};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_extension_instant_recordings_runtime::{
        D1LegacyExtensionInstantRecordingsV1, LegacyExtensionInstantFailureV1,
        LegacyExtensionInstantLifecycleReceiptV1,
    },
    legacy_video_lifecycle_runtime::{
        D1LegacyVideoLifecycleV1, LegacyVideoLifecycleFailureV1, LegacyVideoLifecycleOperationV1,
        NewLegacyVideoLifecycleOperationV1, copy_r2_prefix, delete_r2_prefix,
    },
};

const RPC_PARSE_FAILURE: &str = "Invalid Effect RPC request payload";
const RPC_UNKNOWN_TAG_FAILURE: &str = "Unknown Effect RPC request tag";

#[derive(Debug, Clone, PartialEq)]
enum LegacyVideoLifecycleRpcInputV1 {
    OrganisationUpdate {
        organization_id: String,
        image: LegacyOrganisationImagePatchV1,
    },
    VideoDelete {
        video_id: String,
    },
    VideoDuplicate {
        video_id: String,
    },
    VideoInstantCreate {
        input: LegacyExtensionInstantCreateInputV1,
    },
}

impl LegacyVideoLifecycleRpcInputV1 {
    const fn surface(&self) -> LegacyVideoLifecycleSurfaceV1 {
        match self {
            Self::OrganisationUpdate { .. } => LegacyVideoLifecycleSurfaceV1::OrganisationUpdate,
            Self::VideoDelete { .. } => LegacyVideoLifecycleSurfaceV1::VideoDelete,
            Self::VideoDuplicate { .. } => LegacyVideoLifecycleSurfaceV1::VideoDuplicate,
            Self::VideoInstantCreate { .. } => LegacyVideoLifecycleSurfaceV1::VideoInstantCreate,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DecodedLegacyVideoLifecycleRpcV1 {
    id: String,
    input: LegacyVideoLifecycleRpcInputV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OrganisationIconOperationBindingV1 {
    rpc_id: String,
    new_icon_key: Option<String>,
    old_icon_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RpcDecodeFailureV1 {
    Malformed(Option<String>),
    UnknownTag,
}

#[must_use]
pub(crate) fn is_video_lifecycle_rpc_request(bytes: &[u8]) -> bool {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("tag").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|tag| {
            matches!(
                tag.as_str(),
                "OrganisationUpdate" | "VideoDelete" | "VideoDuplicate" | "VideoInstantCreate"
            )
        })
}

pub(crate) fn effect_rpc_get_response() -> Result<Response> {
    rpc_response(rpc_defect(RPC_PARSE_FAILURE))
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
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => actor_id,
            Err(BrowserWebFailure::Unavailable) => {
                return rpc_response(rpc_typed_failure(
                    &decoded.id,
                    json!({"_tag":"InternalError","type":"database"}),
                ));
            }
            Err(_) => {
                return rpc_response(rpc_typed_failure(
                    &decoded.id,
                    json!({"_tag":"UnauthenticatedError"}),
                ));
            }
        };
    let database = env.d1("DB")?;
    let bucket = match decoded.input.surface() {
        LegacyVideoLifecycleSurfaceV1::OrganisationUpdate => {
            CompatibilityRateLimitBucketV1::OrganizationLibrary
        }
        _ => CompatibilityRateLimitBucketV1::VideoMedia,
    };
    let admitted =
        compatibility_rate_limit::admit_principal(env, &database, bucket, &actor_id, now_ms)
            .await?;
    if matches!(
        admitted,
        frame_application::RateLimitDecisionV1::Rejected { .. }
    ) {
        return rpc_response(rpc_typed_failure(
            &decoded.id,
            json!({"_tag":"InternalError","type":"unknown"}),
        ));
    }
    let authority = D1LegacyVideoLifecycleV1::new(&database);
    let surface = decoded.input.surface();
    let request_key_digest = digest_parts(&[
        b"effect-rpc-request-id-v1",
        surface.operation_id().as_bytes(),
        decoded.id.as_bytes(),
    ]);
    let request_digest = hex_digest(bytes);
    if let Some(existing) = authority
        .operation(surface, &actor_id, &request_key_digest)
        .await
        .map_err(map_worker)?
    {
        if existing.request_digest != request_digest {
            return rpc_response(rpc_typed_failure(
                &decoded.id,
                json!({"_tag":"InternalError","type":"unknown"}),
            ));
        }
        if existing.state == "complete" {
            return replay_rpc_success(&decoded.id, surface, &existing);
        }
    }
    let result = match decoded.input {
        LegacyVideoLifecycleRpcInputV1::OrganisationUpdate {
            organization_id,
            image,
        } => organisation_update(
            env,
            &authority,
            &actor_id,
            &decoded.id,
            &request_key_digest,
            &request_digest,
            &organization_id,
            image,
            now_ms,
        )
        .await
        .map(|()| rpc_void_success(&decoded.id)),
        LegacyVideoLifecycleRpcInputV1::VideoDelete { video_id } => video_delete(
            env,
            &authority,
            LegacyVideoLifecycleSurfaceV1::VideoDelete,
            &actor_id,
            &request_key_digest,
            &request_digest,
            &video_id,
            now_ms,
        )
        .await
        .map(|()| rpc_void_success(&decoded.id)),
        LegacyVideoLifecycleRpcInputV1::VideoDuplicate { video_id } => video_duplicate(
            env,
            &authority,
            &actor_id,
            &request_key_digest,
            &request_digest,
            &video_id,
            now_ms,
        )
        .await
        .map(|()| rpc_void_success(&decoded.id)),
        LegacyVideoLifecycleRpcInputV1::VideoInstantCreate { input } => video_instant_create(
            request,
            env,
            &authority,
            &actor_id,
            &decoded.id,
            &request_key_digest,
            &request_digest,
            &input,
            now_ms,
        )
        .await
        .map(|success| rpc_value_success(&decoded.id, success)),
    };
    rpc_response(match result {
        Ok(value) => value,
        Err(failure) => rpc_failure(&decoded.id, surface, failure),
    })
}

pub(crate) async fn delete_route_response(
    request: &mut Request,
    env: &Env,
    now_ms: i64,
) -> Result<Response> {
    let actor_id =
        match browser_web_runtime::authenticate_host_only_browser_session(request, env, now_ms)
            .await?
        {
            Ok(actor_id) => actor_id,
            Err(_) => return tagged_json(401, "UnauthenticatedError"),
        };
    if request.headers().get("idempotency-key")?.is_some()
        || !request_body_is_empty(request).await?
    {
        return tagged_json(400, "BadRequest");
    }
    let video_id = request
        .url()?
        .query_pairs()
        .filter(|(name, _)| name == "videoId")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let [video_id] = video_id.as_slice() else {
        return tagged_json(400, "BadRequest");
    };
    if !legacy_video_lifecycle_valid_cap_id(video_id) {
        return tagged_json(400, "BadRequest");
    }
    let database = env.d1("DB")?;
    let admitted = compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::VideoMedia,
        &actor_id,
        now_ms,
    )
    .await?;
    if matches!(
        admitted,
        frame_application::RateLimitDecisionV1::Rejected { .. }
    ) {
        return tagged_json(429, "TooManyRequests");
    }
    let request_key_digest = digest_parts(&[
        b"video-delete-route-v1",
        actor_id.as_bytes(),
        video_id.as_bytes(),
    ]);
    let request_digest = digest_parts(&[b"DELETE", video_id.as_bytes()]);
    let authority = D1LegacyVideoLifecycleV1::new(&database);
    match video_delete(
        env,
        &authority,
        LegacyVideoLifecycleSurfaceV1::DeleteRoute,
        &actor_id,
        &request_key_digest,
        &request_digest,
        video_id,
        now_ms,
    )
    .await
    {
        Ok(()) => Ok(Response::empty()?.with_status(204)),
        Err(LegacyVideoLifecycleFailureV1::Invalid) => tagged_json(400, "BadRequest"),
        Err(LegacyVideoLifecycleFailureV1::NotFound) => tagged_json(404, "VideoNotFoundError"),
        Err(LegacyVideoLifecycleFailureV1::Forbidden) => tagged_json(403, "PolicyDenied"),
        Err(_) => tagged_json(500, "InternalError"),
    }
}

pub(crate) async fn og_response(request: &Request, env: &Env, now_ms: i64) -> Result<Response> {
    let admitted = compatibility_rate_limit::admit_edge_request(
        env,
        request,
        CompatibilityRateLimitBucketV1::VideoMedia,
        now_ms,
    )
    .await?;
    if matches!(
        admitted,
        frame_application::RateLimitDecisionV1::Rejected { .. }
    ) {
        return tagged_json(429, "TooManyRequests");
    }
    let video_ids = request
        .url()?
        .query_pairs()
        .filter(|(name, _)| name == "videoId")
        .map(|(_, value)| value.into_owned())
        .collect::<Vec<_>>();
    let video_id = match video_ids.as_slice() {
        [video_id] => video_id.as_str(),
        _ => "",
    };
    let database = env.d1("DB")?;
    let authority = D1LegacyVideoLifecycleV1::new(&database);
    let snapshot = authority.og(video_id).await.map_err(map_worker)?;
    let public = snapshot.as_ref().is_some_and(|snapshot| snapshot.public());
    let has_screenshot = if let Some(snapshot) = snapshot.as_ref().filter(|_| public) {
        let listed = env
            .bucket("RECORDINGS")?
            .list()
            .prefix(&snapshot.object_prefix)
            .limit(1_000)
            .execute()
            .into_send()
            .await?;
        listed.objects().into_iter().any(|object| {
            let key = object.key().to_ascii_lowercase();
            key.contains("screenshot")
                && matches!(
                    key.rsplit('.').next(),
                    Some("jpg" | "jpeg" | "png" | "webp")
                )
        })
    } else {
        false
    };
    let bytes = render_og_png(public, has_screenshot)
        .map_err(|_| worker::Error::RustError("legacy OG image encoding failed".into()))?;
    let mut response = Response::from_bytes(bytes)?;
    response.headers_mut().set("content-type", "image/png")?;
    response.headers_mut().set(
        "cache-control",
        "public, max-age=60, stale-while-revalidate=300",
    )?;
    response
        .headers_mut()
        .set("x-content-type-options", "nosniff")?;
    Ok(response)
}

#[allow(clippy::too_many_arguments)]
async fn video_delete(
    env: &Env,
    authority: &D1LegacyVideoLifecycleV1<'_>,
    surface: LegacyVideoLifecycleSurfaceV1,
    actor_id: &str,
    request_key_digest: &str,
    request_digest: &str,
    video_id: &str,
    now_ms: i64,
) -> std::result::Result<(), LegacyVideoLifecycleFailureV1> {
    if let Some(existing) = authority
        .operation(surface, actor_id, request_key_digest)
        .await?
    {
        if existing.request_digest != request_digest {
            return Err(LegacyVideoLifecycleFailureV1::Conflict);
        }
        if existing.state == "complete" {
            return Ok(());
        }
        let prefix = existing
            .source_prefix
            .as_deref()
            .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
        delete_r2_prefix(
            &env.bucket("RECORDINGS")
                .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?,
            prefix,
        )
        .await?;
        return authority
            .complete(&existing.operation_id, "{}", now_ms)
            .await;
    }
    let video = authority.video_for_owner(actor_id, video_id).await?;
    let operation_id = Uuid::now_v7().to_string();
    authority
        .begin_delete(
            &NewLegacyVideoLifecycleOperationV1 {
                operation_id: &operation_id,
                surface,
                action: if surface == LegacyVideoLifecycleSurfaceV1::DeleteRoute {
                    "delete_route"
                } else {
                    "video_delete"
                },
                actor_id,
                organization_id: &video.organization_id,
                mapped_video_id: Some(&video.mapped_video_id),
                legacy_video_id: Some(&video.legacy_video_id),
                request_key_digest,
                request_digest,
                destination_mapped_video_id: None,
                destination_legacy_video_id: None,
                source_prefix: Some(&video.object_prefix),
                destination_prefix: None,
                result_json: Some("{}"),
                state: "claimed",
                now_ms,
            },
            &video,
        )
        .await?;
    delete_r2_prefix(
        &env.bucket("RECORDINGS")
            .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?,
        &video.object_prefix,
    )
    .await?;
    authority.complete(&operation_id, "{}", now_ms).await
}

#[allow(clippy::too_many_arguments)]
async fn video_duplicate(
    env: &Env,
    authority: &D1LegacyVideoLifecycleV1<'_>,
    actor_id: &str,
    request_key_digest: &str,
    request_digest: &str,
    video_id: &str,
    now_ms: i64,
) -> std::result::Result<(), LegacyVideoLifecycleFailureV1> {
    if let Some(existing) = authority
        .operation(
            LegacyVideoLifecycleSurfaceV1::VideoDuplicate,
            actor_id,
            request_key_digest,
        )
        .await?
    {
        if existing.request_digest != request_digest {
            return Err(LegacyVideoLifecycleFailureV1::Conflict);
        }
        if existing.state == "complete" {
            return Ok(());
        }
        copy_existing_operation(env, authority, &existing, now_ms).await?;
        return authority
            .complete(&existing.operation_id, "{}", now_ms)
            .await;
    }
    let video = authority.video_for_owner(actor_id, video_id).await?;
    let operation_id = Uuid::now_v7().to_string();
    let destination_video_id = Uuid::now_v7().to_string();
    let destination_alias = random_cap_nanoid()?;
    let destination_prefix =
        legacy_video_lifecycle_object_prefix(&video.legacy_owner_id, &destination_alias)
            .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
    authority
        .begin_duplicate(
            &NewLegacyVideoLifecycleOperationV1 {
                operation_id: &operation_id,
                surface: LegacyVideoLifecycleSurfaceV1::VideoDuplicate,
                action: "video_duplicate",
                actor_id,
                organization_id: &video.organization_id,
                mapped_video_id: Some(&video.mapped_video_id),
                legacy_video_id: Some(&video.legacy_video_id),
                request_key_digest,
                request_digest,
                destination_mapped_video_id: Some(&destination_video_id),
                destination_legacy_video_id: Some(&destination_alias),
                source_prefix: Some(&video.object_prefix),
                destination_prefix: Some(&destination_prefix),
                result_json: Some("{}"),
                state: "claimed",
                now_ms,
            },
            &video,
        )
        .await?;
    copy_r2_prefix(
        authority,
        &env.bucket("RECORDINGS")
            .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?,
        &operation_id,
        &video.object_prefix,
        &destination_prefix,
        now_ms,
    )
    .await?;
    authority.complete(&operation_id, "{}", now_ms).await
}

async fn copy_existing_operation(
    env: &Env,
    authority: &D1LegacyVideoLifecycleV1<'_>,
    operation: &LegacyVideoLifecycleOperationV1,
    now_ms: i64,
) -> std::result::Result<(), LegacyVideoLifecycleFailureV1> {
    let source = operation
        .source_prefix
        .as_deref()
        .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
    let destination = operation
        .destination_prefix
        .as_deref()
        .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
    copy_r2_prefix(
        authority,
        &env.bucket("RECORDINGS")
            .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?,
        &operation.operation_id,
        source,
        destination,
        now_ms,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn organisation_update(
    env: &Env,
    authority: &D1LegacyVideoLifecycleV1<'_>,
    actor_id: &str,
    rpc_id: &str,
    request_key_digest: &str,
    request_digest: &str,
    requested_organization_id: &str,
    image: LegacyOrganisationImagePatchV1,
    now_ms: i64,
) -> std::result::Result<(), LegacyVideoLifecycleFailureV1> {
    let organization = authority
        .organization_for_admin(actor_id, requested_organization_id)
        .await?;
    if matches!(image, LegacyOrganisationImagePatchV1::Absent) {
        return Ok(());
    }
    let existing = authority
        .operation(
            LegacyVideoLifecycleSurfaceV1::OrganisationUpdate,
            actor_id,
            request_key_digest,
        )
        .await?;
    let (operation_id, binding, state) = if let Some(existing) = existing {
        if existing.request_digest != request_digest
            || existing.organization_id != organization.organization_id
        {
            return Err(LegacyVideoLifecycleFailureV1::Conflict);
        }
        let binding = existing
            .result_json
            .as_deref()
            .and_then(|value| {
                serde_json::from_str::<OrganisationIconOperationBindingV1>(value).ok()
            })
            .filter(|binding| binding.rpc_id == rpc_id)
            .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)?;
        (existing.operation_id, binding, existing.state)
    } else {
        let operation_id = Uuid::now_v7().to_string();
        let new_icon_key = match &image {
            LegacyOrganisationImagePatchV1::Remove => None,
            LegacyOrganisationImagePatchV1::Replace(image) => Some(
                legacy_organisation_icon_key(
                    &organization.organization_id,
                    &operation_id,
                    &image.file_name,
                )
                .ok_or(LegacyVideoLifecycleFailureV1::Invalid)?,
            ),
            LegacyOrganisationImagePatchV1::Absent => unreachable!("returned above"),
        };
        let binding = OrganisationIconOperationBindingV1 {
            rpc_id: rpc_id.to_owned(),
            new_icon_key,
            old_icon_key: organization.existing_icon_key.clone(),
        };
        let binding_json =
            serde_json::to_string(&binding).map_err(|_| LegacyVideoLifecycleFailureV1::Corrupt)?;
        authority
            .insert_operation(&NewLegacyVideoLifecycleOperationV1 {
                operation_id: &operation_id,
                surface: LegacyVideoLifecycleSurfaceV1::OrganisationUpdate,
                action: "organisation_update",
                actor_id,
                organization_id: &organization.organization_id,
                mapped_video_id: None,
                legacy_video_id: None,
                request_key_digest,
                request_digest,
                destination_mapped_video_id: None,
                destination_legacy_video_id: None,
                source_prefix: None,
                destination_prefix: None,
                result_json: Some(&binding_json),
                state: "claimed",
                now_ms,
            })
            .await?;
        (operation_id, binding, "claimed".to_owned())
    };
    let binding_json =
        serde_json::to_string(&binding).map_err(|_| LegacyVideoLifecycleFailureV1::Corrupt)?;
    if state == "claimed" {
        match (&binding.new_icon_key, &image) {
            (None, LegacyOrganisationImagePatchV1::Remove) => {}
            (Some(key), LegacyOrganisationImagePatchV1::Replace(image)) => {
                let bytes = decode_base64(&image.data)?;
                if bytes.is_empty()
                    || bytes.len() > frame_application::LEGACY_VIDEO_LIFECYCLE_MAX_IMAGE_BYTES
                    || !key.starts_with(&format!("organizations/{}/", organization.organization_id))
                {
                    return Err(LegacyVideoLifecycleFailureV1::Invalid);
                }
                env.bucket("RECORDINGS")
                    .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?
                    .put(key, bytes)
                    .http_metadata(HttpMetadata {
                        content_type: Some(image.content_type.clone()),
                        cache_control: Some("public, max-age=31536000, immutable".into()),
                        ..HttpMetadata::default()
                    })
                    .execute()
                    .into_send()
                    .await
                    .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?
                    .ok_or(LegacyVideoLifecycleFailureV1::Unavailable)?;
            }
            _ => return Err(LegacyVideoLifecycleFailureV1::Conflict),
        }
        authority
            .update_icon_and_mark_pending(
                &operation_id,
                actor_id,
                &organization.organization_id,
                binding.new_icon_key.as_deref(),
                now_ms,
            )
            .await?;
    } else if state != "storage_pending" {
        return Err(LegacyVideoLifecycleFailureV1::Corrupt);
    }
    if let Some(old_key) = deletable_old_icon_key(
        &organization.organization_id,
        binding.old_icon_key.as_deref(),
        binding.new_icon_key.as_deref(),
    ) {
        env.bucket("RECORDINGS")
            .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?
            .delete(old_key)
            .into_send()
            .await
            .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?;
    }
    authority
        .complete(&operation_id, &binding_json, now_ms)
        .await
}

#[allow(clippy::too_many_arguments)]
async fn video_instant_create(
    request: &Request,
    env: &Env,
    authority: &D1LegacyVideoLifecycleV1<'_>,
    actor_id: &str,
    rpc_id: &str,
    request_key_digest: &str,
    request_digest: &str,
    input: &LegacyExtensionInstantCreateInputV1,
    now_ms: i64,
) -> std::result::Result<Value, LegacyVideoLifecycleFailureV1> {
    let signer =
        crate::direct_upload_signer(env).ok_or(LegacyVideoLifecycleFailureV1::Unavailable)?;
    let default_public = videos_default_public(env)?;
    let database = env
        .d1("DB")
        .map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?;
    let operation_id = Uuid::now_v7().to_string();
    let receipt = LegacyExtensionInstantLifecycleReceiptV1 {
        operation_id: &operation_id,
        source_operation_id: LegacyVideoLifecycleSurfaceV1::VideoInstantCreate.operation_id(),
        request_key_digest,
        request_digest,
    };
    let create = D1LegacyExtensionInstantRecordingsV1::new(&database)
        .create(
            actor_id,
            input,
            &request
                .url()
                .map_err(|_| LegacyVideoLifecycleFailureV1::Invalid)?,
            default_public,
            &signer,
            now_ms,
            Some(&receipt),
        )
        .await;
    let success = match create {
        Ok(success) => success,
        Err(failure) => {
            // A concurrent retry can lose the unique receipt race after the
            // winning D1 batch committed. Re-read and replay that exact result;
            // every other error remains fail-closed.
            if matches!(
                failure,
                LegacyExtensionInstantFailureV1::Corrupt
                    | LegacyExtensionInstantFailureV1::Unavailable
            ) && let Some(existing) = authority
                .operation(
                    LegacyVideoLifecycleSurfaceV1::VideoInstantCreate,
                    actor_id,
                    request_key_digest,
                )
                .await?
                && existing.request_digest == request_digest
                && existing.state == "complete"
            {
                return existing
                    .result_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .ok_or(LegacyVideoLifecycleFailureV1::Corrupt);
            }
            return Err(map_instant_failure(failure));
        }
    };
    let _ = rpc_id;
    serde_json::to_value(success).map_err(|_| LegacyVideoLifecycleFailureV1::Corrupt)
}

fn replay_rpc_success(
    rpc_id: &str,
    surface: LegacyVideoLifecycleSurfaceV1,
    operation: &LegacyVideoLifecycleOperationV1,
) -> Result<Response> {
    let value = match surface {
        LegacyVideoLifecycleSurfaceV1::VideoInstantCreate => {
            let result = operation
                .result_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .ok_or_else(|| {
                    worker::Error::RustError("legacy instant replay binding is invalid".into())
                })?;
            rpc_value_success(rpc_id, result)
        }
        LegacyVideoLifecycleSurfaceV1::OrganisationUpdate
        | LegacyVideoLifecycleSurfaceV1::VideoDelete
        | LegacyVideoLifecycleSurfaceV1::VideoDuplicate => rpc_void_success(rpc_id),
        _ => rpc_defect(RPC_UNKNOWN_TAG_FAILURE),
    };
    rpc_response(value)
}

fn decode_rpc_request(
    bytes: &[u8],
) -> std::result::Result<DecodedLegacyVideoLifecycleRpcV1, RpcDecodeFailureV1> {
    if bytes.is_empty() || bytes.len() > LEGACY_VIDEO_LIFECYCLE_MAX_BODY_BYTES {
        return Err(RpcDecodeFailureV1::Malformed(None));
    }
    let value =
        serde_json::from_slice::<Value>(bytes).map_err(|_| RpcDecodeFailureV1::Malformed(None))?;
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
        || !valid_optional_string(object.get("traceId"))
        || !valid_optional_string(object.get("spanId"))
        || !valid_optional_bool(object.get("sampled"))
    {
        return Err(malformed());
    }
    let tag = object
        .get("tag")
        .and_then(Value::as_str)
        .ok_or_else(malformed)?;
    let payload = object.get("payload").ok_or_else(malformed)?;
    let input = match tag {
        "OrganisationUpdate" => decode_organisation_update(payload),
        "VideoDelete" => decode_video_id(payload)
            .map(|video_id| LegacyVideoLifecycleRpcInputV1::VideoDelete { video_id }),
        "VideoDuplicate" => decode_video_id(payload)
            .map(|video_id| LegacyVideoLifecycleRpcInputV1::VideoDuplicate { video_id }),
        "VideoInstantCreate" => decode_instant_create(payload)
            .map(|input| LegacyVideoLifecycleRpcInputV1::VideoInstantCreate { input }),
        _ => return Err(RpcDecodeFailureV1::UnknownTag),
    }
    .map_err(|_| malformed())?;
    Ok(DecodedLegacyVideoLifecycleRpcV1 { id, input })
}

fn decode_organisation_update(
    value: &Value,
) -> std::result::Result<LegacyVideoLifecycleRpcInputV1, ()> {
    let object = value.as_object().ok_or(())?;
    let organization_id = required_string(object, "id")?;
    let image = match object.get("image") {
        None => LegacyOrganisationImagePatchV1::Absent,
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("None") =>
        {
            LegacyOrganisationImagePatchV1::Remove
        }
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("Some") =>
        {
            let value = option.get("value").and_then(Value::as_object).ok_or(())?;
            let image = LegacyOrganisationImageV1 {
                data: required_string(value, "data")?,
                content_type: required_string(value, "contentType")?,
                file_name: required_string(value, "fileName")?,
            };
            if !image.valid_metadata() {
                return Err(());
            }
            LegacyOrganisationImagePatchV1::Replace(image)
        }
        _ => return Err(()),
    };
    Ok(LegacyVideoLifecycleRpcInputV1::OrganisationUpdate {
        organization_id,
        image,
    })
}

fn decode_video_id(value: &Value) -> std::result::Result<String, ()> {
    value
        .as_str()
        .filter(|value| legacy_video_lifecycle_valid_cap_id(value))
        .map(str::to_owned)
        .ok_or(())
}

fn decode_instant_create(
    value: &Value,
) -> std::result::Result<LegacyExtensionInstantCreateInputV1, ()> {
    let object = value.as_object().ok_or(())?;
    let folder_id = match object.get("folderId") {
        None => None,
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("None") =>
        {
            None
        }
        Some(Value::Object(option))
            if option.get("_tag").and_then(Value::as_str) == Some("Some") =>
        {
            Some(required_string(option, "value")?)
        }
        _ => return Err(()),
    };
    let input = LegacyExtensionInstantCreateInputV1 {
        org_id: required_string(object, "orgId")?,
        folder_id,
        duration_seconds: optional_f64(object, "durationSeconds")?,
        resolution: optional_string(object, "resolution")?,
        width: optional_f64(object, "width")?,
        height: optional_f64(object, "height")?,
        video_codec: optional_string(object, "videoCodec")?,
        audio_codec: optional_string(object, "audioCodec")?,
        supports_upload_progress: optional_bool(object, "supportsUploadProgress")?,
    };
    input.valid().then_some(input).ok_or(())
}

fn decode_base64(value: &str) -> std::result::Result<Vec<u8>, LegacyVideoLifecycleFailureV1> {
    if value.is_empty() || !value.len().is_multiple_of(4) || !value.is_ascii() {
        return Err(LegacyVideoLifecycleFailureV1::Invalid);
    }
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(value.len() / 4 * 3);
    for (index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last = index + 1 == bytes.len() / 4;
        let a = base64_digit(chunk[0]).ok_or(LegacyVideoLifecycleFailureV1::Invalid)?;
        let b = base64_digit(chunk[1]).ok_or(LegacyVideoLifecycleFailureV1::Invalid)?;
        let (c, d) = if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' || b & 0x0f != 0 {
                return Err(LegacyVideoLifecycleFailureV1::Invalid);
            }
            (None, None)
        } else {
            let c = base64_digit(chunk[2]).ok_or(LegacyVideoLifecycleFailureV1::Invalid)?;
            if chunk[3] == b'=' {
                if !last || c & 0x03 != 0 {
                    return Err(LegacyVideoLifecycleFailureV1::Invalid);
                }
                (Some(c), None)
            } else {
                (
                    Some(c),
                    Some(base64_digit(chunk[3]).ok_or(LegacyVideoLifecycleFailureV1::Invalid)?),
                )
            }
        };
        decoded.push((a << 2) | (b >> 4));
        if let Some(c) = c {
            decoded.push((b << 4) | (c >> 2));
            if let Some(d) = d {
                decoded.push((c << 6) | d);
            }
        }
    }
    Ok(decoded)
}

fn base64_digit(value: u8) -> Option<u8> {
    match value {
        b'A'..=b'Z' => Some(value - b'A'),
        b'a'..=b'z' => Some(value - b'a' + 26),
        b'0'..=b'9' => Some(value - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

fn render_og_png(
    public: bool,
    has_screenshot: bool,
) -> std::result::Result<Vec<u8>, png::EncodingError> {
    const WIDTH: u32 = 1_200;
    const HEIGHT: u32 = 630;
    let mut pixels = vec![0_u8; WIDTH as usize * HEIGHT as usize * 3];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let dx = (i64::from(x) - 640).unsigned_abs().min(900) as u32;
            let dy = (i64::from(y) - 315).unsigned_abs().min(500) as u32;
            let distance = ((dx * 3 + dy * 4) / 7).min(630);
            let index = (y as usize * WIDTH as usize + x as usize) * 3;
            pixels[index] = (211_u32.saturating_sub(distance / 5)) as u8;
            pixels[index + 1] = (229_u32.saturating_sub(distance / 4)) as u8;
            pixels[index + 2] = 255;
        }
    }
    if public {
        let shade = if has_screenshot { 32 } else { 0 };
        fill_rect(
            &mut pixels,
            WIDTH,
            90,
            47,
            1_020,
            536,
            [shade, shade, shade],
        );
        fill_triangle(
            &mut pixels,
            WIDTH,
            (530, 215),
            (530, 415),
            (710, 315),
            [255, 255, 255],
        );
    } else {
        fill_rect(&mut pixels, WIDTH, 360, 265, 480, 100, [71, 133, 255]);
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, WIDTH, HEIGHT);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&pixels)?;
    }
    Ok(encoded)
}

fn fill_rect(
    pixels: &mut [u8],
    width: u32,
    left: u32,
    top: u32,
    rectangle_width: u32,
    rectangle_height: u32,
    color: [u8; 3],
) {
    for y in top..top + rectangle_height {
        for x in left..left + rectangle_width {
            set_pixel(pixels, width, x, y, color);
        }
    }
}

fn fill_triangle(
    pixels: &mut [u8],
    width: u32,
    a: (i32, i32),
    b: (i32, i32),
    c: (i32, i32),
    color: [u8; 3],
) {
    let edge = |p1: (i32, i32), p2: (i32, i32), point: (i32, i32)| {
        (point.0 - p1.0) * (p2.1 - p1.1) - (point.1 - p1.1) * (p2.0 - p1.0)
    };
    for y in 215..=415 {
        for x in 530..=710 {
            let point = (x, y);
            let first = edge(a, b, point);
            let second = edge(b, c, point);
            let third = edge(c, a, point);
            if (first >= 0 && second >= 0 && third >= 0)
                || (first <= 0 && second <= 0 && third <= 0)
            {
                set_pixel(pixels, width, x as u32, y as u32, color);
            }
        }
    }
}

fn set_pixel(pixels: &mut [u8], width: u32, x: u32, y: u32, color: [u8; 3]) {
    let index = (y as usize * width as usize + x as usize) * 3;
    pixels[index..index + 3].copy_from_slice(&color);
}

async fn request_body_is_empty(request: &mut Request) -> Result<bool> {
    if request
        .headers()
        .get("content-length")?
        .is_some_and(|value| value != "0")
    {
        return Ok(false);
    }
    Ok(crate::read_bounded_legacy_body(request, 0).await.is_ok())
}

fn deletable_old_icon_key<'a>(
    organization_id: &str,
    old: Option<&'a str>,
    new: Option<&str>,
) -> Option<&'a str> {
    old.filter(|old| Some(*old) != new)
        .filter(|old| old.starts_with(&format!("organizations/{organization_id}/")))
        .filter(|old| !old.contains("..") && !old.contains('\\') && !old.contains("//"))
}

fn random_cap_nanoid() -> std::result::Result<String, LegacyVideoLifecycleFailureV1> {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut random = [0_u8; 15];
    getrandom::fill(&mut random).map_err(|_| LegacyVideoLifecycleFailureV1::Unavailable)?;
    let value = random
        .into_iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
        .collect::<String>();
    legacy_video_lifecycle_valid_cap_id(&value)
        .then_some(value)
        .ok_or(LegacyVideoLifecycleFailureV1::Corrupt)
}

fn map_instant_failure(failure: LegacyExtensionInstantFailureV1) -> LegacyVideoLifecycleFailureV1 {
    match failure {
        LegacyExtensionInstantFailureV1::Invalid => LegacyVideoLifecycleFailureV1::Invalid,
        LegacyExtensionInstantFailureV1::Forbidden => LegacyVideoLifecycleFailureV1::Forbidden,
        LegacyExtensionInstantFailureV1::NotFound => LegacyVideoLifecycleFailureV1::NotFound,
        LegacyExtensionInstantFailureV1::Corrupt => LegacyVideoLifecycleFailureV1::Corrupt,
        LegacyExtensionInstantFailureV1::Unavailable => LegacyVideoLifecycleFailureV1::Unavailable,
    }
}

fn map_worker(failure: LegacyVideoLifecycleFailureV1) -> worker::Error {
    worker::Error::RustError(format!("legacy video lifecycle failure: {failure:?}"))
}

fn videos_default_public(env: &Env) -> std::result::Result<bool, LegacyVideoLifecycleFailureV1> {
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
        _ => Err(LegacyVideoLifecycleFailureV1::Unavailable),
    }
}

fn rpc_void_success(id: &str) -> Value {
    json!([{"_tag":"Exit","requestId":id,"exit":{"_tag":"Success"}}])
}

fn rpc_value_success(id: &str, value: impl serde::Serialize) -> Value {
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

fn rpc_failure(
    id: &str,
    surface: LegacyVideoLifecycleSurfaceV1,
    failure: LegacyVideoLifecycleFailureV1,
) -> Value {
    let error = match failure {
        LegacyVideoLifecycleFailureV1::NotFound
            if surface == LegacyVideoLifecycleSurfaceV1::OrganisationUpdate =>
        {
            json!({"_tag":"OrgNotFoundError"})
        }
        LegacyVideoLifecycleFailureV1::NotFound => json!({"_tag":"VideoNotFoundError"}),
        LegacyVideoLifecycleFailureV1::Forbidden => json!({"_tag":"PolicyDenied"}),
        LegacyVideoLifecycleFailureV1::Unavailable => {
            json!({"_tag":"InternalError","type":"database"})
        }
        LegacyVideoLifecycleFailureV1::Invalid
        | LegacyVideoLifecycleFailureV1::Conflict
        | LegacyVideoLifecycleFailureV1::Corrupt => {
            json!({"_tag":"InternalError","type":"unknown"})
        }
    };
    rpc_typed_failure(id, error)
}

fn rpc_response(value: Value) -> Result<Response> {
    let mut response = Response::from_json(&value)?;
    response
        .headers_mut()
        .set("content-type", LEGACY_VIDEO_LIFECYCLE_CONTENT_TYPE)?;
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn tagged_json(status: u16, tag: &str) -> Result<Response> {
    let mut response = Response::from_json(&json!({"_tag":tag}))?.with_status(status);
    response
        .headers_mut()
        .set("cache-control", "no-store, max-age=0")?;
    Ok(response)
}

fn required_string(object: &Map<String, Value>, field: &str) -> std::result::Result<String, ()> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4_096)
        .map(str::to_owned)
        .ok_or(())
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
) -> std::result::Result<Option<String>, ()> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::String(value)) if value.len() <= 4_096 => Ok(Some(value.clone())),
        _ => Err(()),
    }
}

fn optional_f64(object: &Map<String, Value>, field: &str) -> std::result::Result<Option<f64>, ()> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::Number(value)) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or(())
            .map(Some),
        _ => Err(()),
    }
}

fn optional_bool(
    object: &Map<String, Value>,
    field: &str,
) -> std::result::Result<Option<bool>, ()> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(()),
    }
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

fn valid_optional_string(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.as_str().is_some())
}

fn valid_optional_bool(value: Option<&Value>) -> bool {
    value.is_none_or(|value| value.as_bool().is_some())
}

fn hex_digest(value: &[u8]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in Sha256::digest(value) {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

fn digest_parts(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
        digest.update([0]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_decoder_covers_all_four_owned_tags() {
        for body in [
            br#"{"_tag":"Request","id":"1","tag":"VideoDelete","payload":"0123456789abcde","headers":[]}"#.as_slice(),
            br#"{"_tag":"Request","id":"2","tag":"VideoDuplicate","payload":"0123456789abcde","headers":[]}"#.as_slice(),
            br#"{"_tag":"Request","id":"3","tag":"OrganisationUpdate","payload":{"id":"0123456789abcde","image":{"_tag":"None"}},"headers":[]}"#.as_slice(),
            br#"{"_tag":"Request","id":"4","tag":"VideoInstantCreate","payload":{"orgId":"0123456789abcde","folderId":{"_tag":"None"}},"headers":[]}"#.as_slice(),
        ] {
            assert!(decode_rpc_request(body).is_ok());
            assert!(is_video_lifecycle_rpc_request(body));
        }
    }

    #[test]
    fn base64_decoder_rejects_noncanonical_padding() {
        assert_eq!(decode_base64("AQID").expect("decode"), vec![1, 2, 3]);
        assert_eq!(decode_base64("AQI=").expect("decode"), vec![1, 2]);
        assert_eq!(decode_base64("AQ==").expect("decode"), vec![1]);
        assert!(decode_base64("AR==").is_err());
        assert!(decode_base64("AQI=AQID").is_err());
    }

    #[test]
    fn og_renderer_emits_a_real_1200_by_630_png() {
        let bytes = render_og_png(true, true).expect("png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().expect("width")),
            1_200
        );
        assert_eq!(
            u32::from_be_bytes(bytes[20..24].try_into().expect("height")),
            630
        );
    }
}
