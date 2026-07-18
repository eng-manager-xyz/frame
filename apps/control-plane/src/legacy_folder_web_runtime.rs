//! Authenticated browser ingress for Cap's three folder-assignment actions.
//!
//! This module owns only the Frame HTTP carrier. It preserves the frozen
//! `server_action`/`ACTION` identity inside the compatibility registry, derives
//! actor and active tenant from trusted session state, and delegates the
//! mutation plus one-use proof consumption to the atomic D1 adapter.

use frame_application::{
    LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID, LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID,
    LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID, LegacyCallerV1, LegacyFolderAssignmentInputV1,
    LegacyFolderAssignmentSuccessV1, RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiErrorCodeV1, ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    ClientSurfaceV1, IdempotencyKey,
};
use serde::Deserialize;
use worker::{Env, Error, Request, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1,
        LegacyWebFolderAssignmentActionInvocationV1,
    },
    legacy_folder_assignment_runtime::D1LegacyFolderAssignmentAtomicPortV1,
};

pub const WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1: &str = "frame.web-folder-assignment-request.v1";

const MAX_ACTION_BODY_BYTES: usize = 256 * 1024;
const MAX_VIDEO_IDS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderAssignmentActionV1 {
    Add,
    Remove,
    Move,
}

impl FolderAssignmentActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID => Some(Self::Add),
            LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID => Some(Self::Remove),
            LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID => Some(Self::Move),
            _ => None,
        }
    }

    const fn operation_id(self) -> &'static str {
        match self {
            Self::Add => LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID,
            Self::Remove => LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID,
            Self::Move => LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID,
        }
    }
}

#[must_use]
pub fn is_action(operation_id: &str) -> bool {
    FolderAssignmentActionV1::parse(operation_id).is_some()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddRemoveRequestWireV1 {
    schema_version: String,
    folder_id: String,
    video_ids: Vec<String>,
    scope_id: String,
    idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveRequestWireV1 {
    schema_version: String,
    video_id: String,
    /// Required field whose value may be null. This preserves Cap's
    /// distinction between an omitted argument and an explicit root move.
    folder_id: serde_json::Value,
    scope_id: Option<String>,
    idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFolderAssignmentActionV1 {
    action: FolderAssignmentActionV1,
    input: LegacyFolderAssignmentInputV1,
    idempotency_key: String,
    body_length: u64,
    content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebFolderAssignmentActionEffectV1 {
    Added { added_count: u16, message: String },
    Removed { removed_count: u16, message: String },
    MoveVoid,
}

pub async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedFolderAssignmentActionV1>> {
    let Some(action) = FolderAssignmentActionV1::parse(operation_id) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    let content_type = request.headers().get("content-type")?;
    if !matches!(
        content_type.as_deref(),
        Some("application/json" | "application/json; charset=utf-8")
    ) || request
        .headers()
        .get("content-encoding")?
        .is_some_and(|encoding| encoding != "identity")
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let declared_length =
        match declared_body_length(request.headers().get("content-length")?.as_deref()) {
            Ok(length) => length,
            Err(failure) => return Ok(Err(failure)),
        };
    if declared_length.is_some_and(|length| length == 0 || length > MAX_ACTION_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes = match crate::read_bounded_legacy_body(request, MAX_ACTION_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(()) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    if bytes.is_empty()
        || bytes.len() > MAX_ACTION_BODY_BYTES
        || declared_length.is_some_and(|length| length != bytes.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let (input, idempotency_key) = match action {
        FolderAssignmentActionV1::Add | FolderAssignmentActionV1::Remove => {
            let wire = match serde_json::from_slice::<AddRemoveRequestWireV1>(&bytes) {
                Ok(wire) => wire,
                Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
            };
            if wire.schema_version != WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1
                || !valid_cap_nanoid(&wire.folder_id)
                || !valid_cap_nanoid(&wire.scope_id)
                || wire.video_ids.is_empty()
                || wire.video_ids.len() > MAX_VIDEO_IDS
                || wire.video_ids.iter().any(|value| !valid_cap_nanoid(value))
                || !valid_idempotency_key(&wire.idempotency_key)
            {
                return Ok(Err(BrowserWebFailure::Invalid));
            }
            let input = match action {
                FolderAssignmentActionV1::Add => LegacyFolderAssignmentInputV1::Add {
                    legacy_folder_id: wire.folder_id,
                    legacy_video_ids: wire.video_ids,
                    legacy_scope_id: wire.scope_id,
                },
                FolderAssignmentActionV1::Remove => LegacyFolderAssignmentInputV1::Remove {
                    legacy_folder_id: wire.folder_id,
                    legacy_video_ids: wire.video_ids,
                    legacy_scope_id: wire.scope_id,
                },
                FolderAssignmentActionV1::Move => unreachable!("matched add/remove branch"),
            };
            (input, wire.idempotency_key)
        }
        FolderAssignmentActionV1::Move => {
            let wire = match serde_json::from_slice::<MoveRequestWireV1>(&bytes) {
                Ok(wire) => wire,
                Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
            };
            let folder_id = match required_nullable_cap_nanoid(wire.folder_id) {
                Ok(folder_id) => folder_id,
                Err(failure) => return Ok(Err(failure)),
            };
            if wire.schema_version != WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1
                || !valid_cap_nanoid(&wire.video_id)
                || wire
                    .scope_id
                    .as_deref()
                    .is_some_and(|value| !valid_cap_nanoid(value))
                || !valid_idempotency_key(&wire.idempotency_key)
            {
                return Ok(Err(BrowserWebFailure::Invalid));
            }
            (
                LegacyFolderAssignmentInputV1::Move {
                    legacy_video_id: wire.video_id,
                    legacy_folder_id: folder_id,
                    legacy_scope_id: wire.scope_id,
                },
                wire.idempotency_key,
            )
        }
    };
    Ok(Ok(DecodedFolderAssignmentActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len()).map_err(|_| {
            Error::RustError("legacy folder-assignment body length is invalid".into())
        })?,
        content_type: content_type.expect("validated content type"),
    }))
}

pub async fn mutate(
    request: &Request,
    env: &Env,
    body: &DecodedFolderAssignmentActionV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<WebFolderAssignmentActionEffectV1>> {
    let header_key = request.headers().get("idempotency-key")?;
    if header_key.as_deref() != Some(body.idempotency_key.as_str()) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let idempotency_key = match IdempotencyKey::parse(body.idempotency_key.clone()) {
        Ok(key) => key,
        Err(_) => return Ok(Err(BrowserWebFailure::Invalid)),
    };
    let database = env.d1("DB")?;
    let transport =
        LegacyCompatibilityTransportV1::new_fail_closed(&database, compatibility_policy())
            .map_err(|_| Error::RustError("legacy compatibility registry is invalid".into()))?;
    let proof = match browser_web_runtime::authenticate_compatibility_mutation(request, env, now_ms)
        .await?
    {
        Ok(proof) => proof,
        Err(failure) => return Ok(Err(failure)),
    };
    let actor_id = proof.user_id().to_string();
    let rate_limit = match compatibility_rate_limit::admit_principal(
        env,
        &database,
        CompatibilityRateLimitBucketV1::OrganizationLibrary,
        &actor_id,
        now_ms,
    )
    .await
    {
        Ok(rate_limit) => rate_limit,
        Err(error) => {
            if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, &proof, now_ms,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Err(error);
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        if !browser_web_runtime::consume_session_grant_or_confirm_absent(&database, &proof, now_ms)
            .await?
        {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let active_organization_id =
        match browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await {
            Ok(Some(organization_id)) => organization_id,
            Ok(None) => {
                if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                    &database, &proof, now_ms,
                )
                .await?
                {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                return Ok(Err(BrowserWebFailure::NotFound));
            }
            Err(error) => {
                if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                    &database, &proof, now_ms,
                )
                .await?
                {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                return Err(error);
            }
        };
    let authenticated = match LegacyAuthenticatedContextV1::new(&actor_id, active_organization_id) {
        Ok(authenticated) => authenticated,
        Err(_) => {
            if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, &proof, now_ms,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
    };
    let security = RequestSecurityContextV1 {
        authenticated: true,
        authorized: true,
        browser_origin_valid: true,
        csrf_valid: true,
        rate_limit,
    };
    let port = D1LegacyFolderAssignmentAtomicPortV1::new(&database);
    let result = transport
        .dispatch_web_folder_assignment_action(
            &port,
            &proof,
            LegacyWebFolderAssignmentActionInvocationV1 {
                caller: web_caller(),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: body.body_length,
                    content_type: Some(body.content_type.clone()),
                    idempotency_key: Some(idempotency_key),
                    correlation_id: correlation_id.to_owned(),
                },
                security,
                authenticated,
                operation_id: body.action.operation_id(),
                input: body.input.clone(),
                idempotency_key: body.idempotency_key.clone(),
            },
        )
        .await;
    let execution = match result {
        Ok(execution) => execution,
        Err(error) => {
            if !browser_web_runtime::consume_session_grant_or_confirm_absent(
                &database, &proof, now_ms,
            )
            .await?
            {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(map_api_error(error)));
        }
    };
    let effect = match execution.success() {
        LegacyFolderAssignmentSuccessV1::Added { added_count } => {
            WebFolderAssignmentActionEffectV1::Added {
                added_count: *added_count,
                message: execution
                    .success()
                    .message()
                    .expect("added result has a message"),
            }
        }
        LegacyFolderAssignmentSuccessV1::Removed { removed_count } => {
            WebFolderAssignmentActionEffectV1::Removed {
                removed_count: *removed_count,
                message: execution
                    .success()
                    .message()
                    .expect("removed result has a message"),
            }
        }
        LegacyFolderAssignmentSuccessV1::MoveVoid => WebFolderAssignmentActionEffectV1::MoveVoid,
    };
    Ok(Ok(effect))
}

fn declared_body_length(value: Option<&str>) -> BrowserWebOutcome<Option<usize>> {
    match value {
        Some(value) => value
            .parse::<usize>()
            .map(Some)
            .map_err(|_| BrowserWebFailure::Invalid),
        None => Ok(None),
    }
}

fn valid_cap_nanoid(value: &str) -> bool {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    value.len() == 15 && value.bytes().all(|byte| ALPHABET.contains(&byte))
}

fn required_nullable_cap_nanoid(value: serde_json::Value) -> BrowserWebOutcome<Option<String>> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(value) if valid_cap_nanoid(&value) => Ok(Some(value)),
        _ => Err(BrowserWebFailure::Invalid),
    }
}

fn valid_idempotency_key(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

const fn compatibility_policy() -> ClientCompatibilityPolicyV1 {
    ClientCompatibilityPolicyV1 {
        api_major: 1,
        current_release: 2,
        previous_release: 1,
        deprecated_after_ms: None,
        retired: false,
    }
}

const fn web_caller() -> LegacyCallerV1 {
    LegacyCallerV1::Released(ClientReleaseV1 {
        surface: ClientSurfaceV1::Web,
        api_major: 1,
        release: 2,
    })
}

fn map_api_error(error: frame_domain::ApiErrorV1) -> BrowserWebFailure {
    match error.code {
        ApiErrorCodeV1::InvalidRequest => BrowserWebFailure::Invalid,
        ApiErrorCodeV1::Unauthenticated => BrowserWebFailure::Unauthenticated,
        ApiErrorCodeV1::NotFound => BrowserWebFailure::NotFound,
        ApiErrorCodeV1::Conflict => BrowserWebFailure::Conflict,
        ApiErrorCodeV1::RateLimited => BrowserWebFailure::RateLimited,
        ApiErrorCodeV1::Unsupported
        | ApiErrorCodeV1::UpgradeRequired
        | ApiErrorCodeV1::TemporarilyUnavailable
        | ApiErrorCodeV1::Indeterminate
        | ApiErrorCodeV1::Internal => BrowserWebFailure::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_is_closed_to_the_three_pinned_operation_ids() {
        for operation_id in [
            LEGACY_ADD_VIDEOS_TO_FOLDER_OPERATION_ID,
            LEGACY_REMOVE_VIDEOS_FROM_FOLDER_OPERATION_ID,
            LEGACY_MOVE_VIDEO_TO_FOLDER_OPERATION_ID,
        ] {
            assert!(is_action(operation_id));
        }
        assert!(!is_action("addVideosToFolder"));
        assert!(!is_action("cap-v1-unknown"));
    }

    #[test]
    fn move_requires_folder_presence_but_accepts_explicit_null() {
        let explicit_null = serde_json::from_str::<MoveRequestWireV1>(
            r#"{"schema_version":"frame.web-folder-assignment-request.v1","video_id":"0123456789abcde","folder_id":null,"scope_id":null,"idempotency_key":"folder-move-1"}"#,
        )
        .expect("explicit null");
        assert_eq!(explicit_null.folder_id, serde_json::Value::Null);
        assert!(
            serde_json::from_str::<MoveRequestWireV1>(
                r#"{"schema_version":"frame.web-folder-assignment-request.v1","video_id":"0123456789abcde","scope_id":null,"idempotency_key":"folder-move-1"}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn structural_bounds_are_closed_before_authentication() {
        assert!(valid_cap_nanoid("0123456789abcde"));
        assert!(!valid_cap_nanoid("0123456789abcdi"));
        assert!(valid_idempotency_key("folder-1"));
        assert!(!valid_idempotency_key("short"));
        assert_eq!(declared_body_length(None), Ok(None));
        assert_eq!(declared_body_length(Some("42")), Ok(Some(42)));
        assert_eq!(
            declared_body_length(Some("invalid")),
            Err(BrowserWebFailure::Invalid)
        );
    }

    #[test]
    fn payloads_deny_unknown_fields_and_wrong_shapes() {
        assert!(
            serde_json::from_str::<AddRemoveRequestWireV1>(
                r#"{"schema_version":"frame.web-folder-assignment-request.v1","folder_id":"0123456789abcde","video_ids":["0123456789abcdf"],"scope_id":"0123456789abcdg","idempotency_key":"folder-add-1","tenant_id":"forbidden"}"#,
            )
            .is_err()
        );
        let wrong_shape = serde_json::from_str::<MoveRequestWireV1>(
            r#"{"schema_version":"frame.web-folder-assignment-request.v1","video_id":"0123456789abcde","folder_id":[],"scope_id":null,"idempotency_key":"folder-move-1"}"#,
        )
        .expect("shape is rejected by the presence-aware validator");
        assert_eq!(
            required_nullable_cap_nanoid(wrong_shape.folder_id),
            Err(BrowserWebFailure::Invalid)
        );
    }
}
