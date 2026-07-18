//! Authenticated browser ingress for Cap's six membership write actions.
//!
//! The carrier accepts only the frozen Frame wire schema, derives actor and
//! active organization from the host-only browser session, and delegates the
//! authority fence, mutation, audit, invalidation, replay journal, and one-use
//! proof consumption to the atomic D1 adapter.

use std::fmt;

use frame_application::{
    LEGACY_ADD_SPACE_MEMBER_OPERATION_ID, LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID,
    LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID, LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID,
    LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID, LEGACY_SET_SPACE_MEMBERS_OPERATION_ID, LegacyCallerV1,
    LegacyMembershipInputV1, LegacyMembershipSuccessV1, LegacySubmittedSpaceMemberV1,
    RateLimitDecisionV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiErrorCodeV1, ApiMutationEnvelopeV1, ClientCompatibilityPolicyV1, ClientReleaseV1,
    ClientSurfaceV1, IdempotencyKey,
};
use serde::{Deserialize, Deserializer};
use worker::{Env, Error, Request, Result};

use crate::{
    browser_web_runtime::{self, BrowserWebFailure, BrowserWebOutcome},
    compatibility_rate_limit::{self, CompatibilityRateLimitBucketV1},
    legacy_compatibility_runtime::{
        LegacyAuthenticatedContextV1, LegacyCompatibilityTransportV1,
        LegacyWebMembershipActionInvocationV1,
    },
    legacy_membership_actions_runtime::D1LegacyMembershipAtomicPortV1,
};

pub const WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-membership-action-request.v1";

const MAX_ACTION_BODY_BYTES: usize = 256 * 1024;
const MAX_MEMBERSHIP_TARGETS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MembershipActionV1 {
    RemoveOrganizationInvite,
    AddSpaceMember,
    SetSpaceMembers,
    AddSpaceMembers,
    BatchRemoveSpaceMembers,
    RemoveSpaceMember,
}

impl MembershipActionV1 {
    fn parse(value: &str) -> Option<Self> {
        match value {
            LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID => Some(Self::RemoveOrganizationInvite),
            LEGACY_ADD_SPACE_MEMBER_OPERATION_ID => Some(Self::AddSpaceMember),
            LEGACY_SET_SPACE_MEMBERS_OPERATION_ID => Some(Self::SetSpaceMembers),
            LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID => Some(Self::AddSpaceMembers),
            LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID => Some(Self::BatchRemoveSpaceMembers),
            LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID => Some(Self::RemoveSpaceMember),
            _ => None,
        }
    }

    const fn operation_id(self) -> &'static str {
        match self {
            Self::RemoveOrganizationInvite => LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID,
            Self::AddSpaceMember => LEGACY_ADD_SPACE_MEMBER_OPERATION_ID,
            Self::SetSpaceMembers => LEGACY_SET_SPACE_MEMBERS_OPERATION_ID,
            Self::AddSpaceMembers => LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID,
            Self::BatchRemoveSpaceMembers => LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID,
            Self::RemoveSpaceMember => LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID,
        }
    }
}

#[must_use]
pub fn is_action(operation_id: &str) -> bool {
    MembershipActionV1::parse(operation_id).is_some()
}

#[derive(Clone, PartialEq, Eq, Default)]
enum OptionalJsonFieldV1 {
    #[default]
    Missing,
    Present(serde_json::Value),
}

impl fmt::Debug for OptionalJsonFieldV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "Missing",
            Self::Present(_) => "Present([redacted])",
        })
    }
}

impl<'de> Deserialize<'de> for OptionalJsonFieldV1 {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde_json::Value::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveOrganizationInviteRequestWireV1 {
    schema_version: String,
    invite_id: String,
    organization_id: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddSpaceMemberRequestWireV1 {
    schema_version: String,
    space_id: String,
    user_id: String,
    role: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddSpaceMembersRequestWireV1 {
    schema_version: String,
    space_id: String,
    user_ids: Vec<String>,
    role: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchRemoveSpaceMembersRequestWireV1 {
    schema_version: String,
    member_ids: Vec<String>,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveSpaceMemberRequestWireV1 {
    schema_version: String,
    member_id: String,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetSpaceMembersRequestWireV1 {
    schema_version: String,
    space_id: String,
    user_ids: Vec<String>,
    #[serde(default)]
    role: OptionalJsonFieldV1,
    #[serde(default)]
    members: OptionalJsonFieldV1,
    idempotency_key: String,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmittedSpaceMemberWireV1 {
    user_id: String,
    role: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DecodedMembershipActionV1 {
    action: MembershipActionV1,
    input: LegacyMembershipInputV1,
    idempotency_key: String,
    body_length: u64,
    content_type: String,
}

impl fmt::Debug for DecodedMembershipActionV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedMembershipActionV1")
            .field("action", &self.action)
            .field("input", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("body_length", &self.body_length)
            .field("content_type", &self.content_type)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum WebMembershipActionEffectV1 {
    SuccessObject,
    SpaceMembersSet {
        count: u32,
    },
    SpaceMembersAdded {
        added: Vec<String>,
        already_members: Vec<String>,
    },
    SpaceMembersRemoved {
        removed_member_ids: Vec<String>,
    },
}

impl fmt::Debug for WebMembershipActionEffectV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SuccessObject => formatter.write_str("SuccessObject"),
            Self::SpaceMembersSet { count } => formatter
                .debug_struct("SpaceMembersSet")
                .field("count", count)
                .finish(),
            Self::SpaceMembersAdded {
                added,
                already_members,
            } => formatter
                .debug_struct("SpaceMembersAdded")
                .field("added_count", &added.len())
                .field("already_members_count", &already_members.len())
                .finish(),
            Self::SpaceMembersRemoved { removed_member_ids } => formatter
                .debug_struct("SpaceMembersRemoved")
                .field("removed_count", &removed_member_ids.len())
                .finish(),
        }
    }
}

pub async fn decode_action_request(
    request: &mut Request,
    operation_id: &str,
) -> Result<BrowserWebOutcome<DecodedMembershipActionV1>> {
    let Some(action) = MembershipActionV1::parse(operation_id) else {
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
    let (input, idempotency_key) = match decode_wire(action, &bytes) {
        Ok(decoded) => decoded,
        Err(failure) => return Ok(Err(failure)),
    };
    Ok(Ok(DecodedMembershipActionV1 {
        action,
        input,
        idempotency_key,
        body_length: u64::try_from(bytes.len())
            .map_err(|_| Error::RustError("legacy membership body length is invalid".into()))?,
        content_type: content_type.expect("validated content type"),
    }))
}

pub async fn mutate(
    request: &Request,
    env: &Env,
    body: &DecodedMembershipActionV1,
    now_ms: i64,
    correlation_id: &str,
) -> Result<BrowserWebOutcome<WebMembershipActionEffectV1>> {
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
            if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Err(error);
        }
    };
    if matches!(rate_limit, RateLimitDecisionV1::Rejected { .. }) {
        if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        return Ok(Err(BrowserWebFailure::RateLimited));
    }
    let active_organization_id =
        match browser_web_runtime::trusted_active_organization_id(&database, &actor_id).await {
            Ok(Some(organization_id)) => organization_id,
            Ok(None) => {
                if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                return Ok(Err(BrowserWebFailure::NotFound));
            }
            Err(error) => {
                if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
                    return Ok(Err(BrowserWebFailure::Unavailable));
                }
                return Err(error);
            }
        };
    let authenticated = match LegacyAuthenticatedContextV1::new(&actor_id, active_organization_id) {
        Ok(authenticated) => authenticated,
        Err(_) => {
            if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
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
    let port = D1LegacyMembershipAtomicPortV1::new(&database);
    let result = transport
        .dispatch_web_membership_action(
            &port,
            &proof,
            LegacyWebMembershipActionInvocationV1 {
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
            if !consume_or_confirm_absent(&database, &proof, now_ms).await? {
                return Ok(Err(BrowserWebFailure::Unavailable));
            }
            return Ok(Err(map_api_error(error)));
        }
    };
    Ok(Ok(project_success(execution.success())))
}

fn project_success(success: LegacyMembershipSuccessV1) -> WebMembershipActionEffectV1 {
    match success {
        LegacyMembershipSuccessV1::InviteRemoved
        | LegacyMembershipSuccessV1::SpaceMemberAdded
        | LegacyMembershipSuccessV1::SpaceMemberRemoved => {
            WebMembershipActionEffectV1::SuccessObject
        }
        LegacyMembershipSuccessV1::SpaceMembersSet { count } => {
            WebMembershipActionEffectV1::SpaceMembersSet { count }
        }
        LegacyMembershipSuccessV1::SpaceMembersAdded {
            added,
            already_members,
        } => WebMembershipActionEffectV1::SpaceMembersAdded {
            added,
            already_members,
        },
        LegacyMembershipSuccessV1::SpaceMembersRemoved { removed_member_ids } => {
            WebMembershipActionEffectV1::SpaceMembersRemoved {
                removed_member_ids: removed_member_ids
                    .into_iter()
                    .map(|value| value.legacy_id().to_owned())
                    .collect(),
            }
        }
    }
}

async fn consume_or_confirm_absent(
    database: &worker::D1Database,
    proof: &frame_application::ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<bool> {
    browser_web_runtime::consume_session_grant_or_confirm_absent(database, proof, now_ms).await
}

fn decode_wire(
    action: MembershipActionV1,
    bytes: &[u8],
) -> BrowserWebOutcome<(LegacyMembershipInputV1, String)> {
    match action {
        MembershipActionV1::RemoveOrganizationInvite => {
            let wire = serde_json::from_slice::<RemoveOrganizationInviteRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.invite_id)?;
            validate_cap_nanoid(&wire.organization_id)?;
            Ok((
                LegacyMembershipInputV1::RemoveOrganizationInvite {
                    legacy_invite_id: wire.invite_id,
                    legacy_organization_id: wire.organization_id,
                },
                wire.idempotency_key,
            ))
        }
        MembershipActionV1::AddSpaceMember => {
            let wire = serde_json::from_slice::<AddSpaceMemberRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.space_id)?;
            validate_cap_nanoid(&wire.user_id)?;
            validate_role(&wire.role)?;
            Ok((
                LegacyMembershipInputV1::AddSpaceMember {
                    legacy_space_id: wire.space_id,
                    legacy_user_id: wire.user_id,
                    role: wire.role,
                },
                wire.idempotency_key,
            ))
        }
        MembershipActionV1::AddSpaceMembers => {
            let wire = serde_json::from_slice::<AddSpaceMembersRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.space_id)?;
            if wire.user_ids.len() > MAX_MEMBERSHIP_TARGETS {
                return Err(BrowserWebFailure::Invalid);
            }
            for user_id in &wire.user_ids {
                validate_cap_nanoid(user_id)?;
            }
            validate_role(&wire.role)?;
            Ok((
                LegacyMembershipInputV1::AddSpaceMembers {
                    legacy_space_id: wire.space_id,
                    legacy_user_ids: wire.user_ids,
                    role: wire.role,
                },
                wire.idempotency_key,
            ))
        }
        MembershipActionV1::BatchRemoveSpaceMembers => {
            let wire = serde_json::from_slice::<BatchRemoveSpaceMembersRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            if wire.member_ids.len() > MAX_MEMBERSHIP_TARGETS {
                return Err(BrowserWebFailure::Invalid);
            }
            for member_id in &wire.member_ids {
                validate_cap_nanoid(member_id)?;
            }
            Ok((
                LegacyMembershipInputV1::BatchRemoveSpaceMembers {
                    legacy_member_ids: wire.member_ids,
                },
                wire.idempotency_key,
            ))
        }
        MembershipActionV1::RemoveSpaceMember => {
            let wire = serde_json::from_slice::<RemoveSpaceMemberRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.member_id)?;
            Ok((
                LegacyMembershipInputV1::RemoveSpaceMember {
                    legacy_member_id: wire.member_id,
                },
                wire.idempotency_key,
            ))
        }
        MembershipActionV1::SetSpaceMembers => {
            let wire = serde_json::from_slice::<SetSpaceMembersRequestWireV1>(bytes)
                .map_err(|_| BrowserWebFailure::Invalid)?;
            validate_common(&wire.schema_version, &wire.idempotency_key)?;
            validate_cap_nanoid(&wire.space_id)?;
            if wire.user_ids.len() > MAX_MEMBERSHIP_TARGETS {
                return Err(BrowserWebFailure::Invalid);
            }
            for user_id in &wire.user_ids {
                validate_cap_nanoid(user_id)?;
            }
            let role = optional_role(wire.role)?;
            let members = optional_members(wire.members)?;
            Ok((
                LegacyMembershipInputV1::SetSpaceMembers {
                    legacy_space_id: wire.space_id,
                    legacy_user_ids: wire.user_ids,
                    role,
                    members,
                },
                wire.idempotency_key,
            ))
        }
    }
}

fn optional_role(value: OptionalJsonFieldV1) -> BrowserWebOutcome<Option<String>> {
    match value {
        OptionalJsonFieldV1::Missing => Ok(None),
        OptionalJsonFieldV1::Present(serde_json::Value::String(role)) => {
            validate_role(&role)?;
            Ok(Some(role))
        }
        OptionalJsonFieldV1::Present(_) => Err(BrowserWebFailure::Invalid),
    }
}

fn optional_members(
    value: OptionalJsonFieldV1,
) -> BrowserWebOutcome<Option<Vec<LegacySubmittedSpaceMemberV1>>> {
    let OptionalJsonFieldV1::Present(value) = value else {
        return Ok(None);
    };
    let values = serde_json::from_value::<Vec<SubmittedSpaceMemberWireV1>>(value)
        .map_err(|_| BrowserWebFailure::Invalid)?;
    if values.len() > MAX_MEMBERSHIP_TARGETS {
        return Err(BrowserWebFailure::Invalid);
    }
    values
        .into_iter()
        .map(|member| {
            validate_cap_nanoid(&member.user_id)?;
            validate_role(&member.role)?;
            Ok(LegacySubmittedSpaceMemberV1 {
                legacy_user_id: member.user_id,
                role: member.role,
            })
        })
        .collect::<BrowserWebOutcome<Vec<_>>>()
        .map(Some)
}

fn validate_common(schema_version: &str, idempotency_key: &str) -> BrowserWebOutcome<()> {
    if schema_version != WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1
        || !valid_idempotency_key(idempotency_key)
    {
        return Err(BrowserWebFailure::Invalid);
    }
    Ok(())
}

fn validate_cap_nanoid(value: &str) -> BrowserWebOutcome<()> {
    if valid_cap_nanoid(value) {
        Ok(())
    } else {
        Err(BrowserWebFailure::Invalid)
    }
}

fn validate_role(value: &str) -> BrowserWebOutcome<()> {
    if matches!(value, "admin" | "member") {
        Ok(())
    } else {
        Err(BrowserWebFailure::Invalid)
    }
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
    fn selector_is_closed_to_six_exact_actions() {
        assert_eq!(
            LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID,
            "cap-v1-b177854e2386c877"
        );
        assert_eq!(
            LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID,
            "cap-v1-38aff8e7221d0260"
        );
        assert_eq!(
            LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID,
            "cap-v1-135614e516c47bf4"
        );
        assert!(is_action(LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID));
        assert!(is_action(LEGACY_ADD_SPACE_MEMBER_OPERATION_ID));
        assert!(is_action(LEGACY_SET_SPACE_MEMBERS_OPERATION_ID));
        assert!(is_action(LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID));
        assert!(is_action(LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID));
        assert!(is_action(LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID));
        assert!(!is_action("setSpaceMembers"));
        assert!(!is_action("cap-v1-unknown"));
    }

    #[test]
    fn bulk_add_and_removal_wires_are_exact_bounded_and_presence_safe() {
        let add = br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":["3123456789abcde"],"role":"admin","idempotency_key":"membership-add-many-1"}"#;
        let (input, key) = decode_wire(MembershipActionV1::AddSpaceMembers, add).expect("bulk add");
        assert_eq!(key, "membership-add-many-1");
        assert!(matches!(
            input,
            LegacyMembershipInputV1::AddSpaceMembers {
                legacy_user_ids,
                role,
                ..
            } if legacy_user_ids == ["3123456789abcde"] && role == "admin"
        ));

        let empty_add = br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":[],"role":"member","idempotency_key":"membership-add-empty-1"}"#;
        let (input, _) = decode_wire(MembershipActionV1::AddSpaceMembers, empty_add)
            .expect("empty add is an exact no-op");
        assert!(matches!(
            input,
            LegacyMembershipInputV1::AddSpaceMembers {
                legacy_user_ids,
                ..
            } if legacy_user_ids.is_empty()
        ));

        let batch = br#"{"schema_version":"frame.web-membership-action-request.v1","member_ids":[],"idempotency_key":"membership-remove-many-1"}"#;
        let (input, _) = decode_wire(MembershipActionV1::BatchRemoveSpaceMembers, batch)
            .expect("empty batch is an exact no-op");
        assert!(matches!(
            input,
            LegacyMembershipInputV1::BatchRemoveSpaceMembers { legacy_member_ids }
                if legacy_member_ids.is_empty()
        ));

        let remove = br#"{"schema_version":"frame.web-membership-action-request.v1","member_id":"4123456789abcde","idempotency_key":"membership-remove-one-1"}"#;
        let (input, _) =
            decode_wire(MembershipActionV1::RemoveSpaceMember, remove).expect("single remove");
        assert!(matches!(
            input,
            LegacyMembershipInputV1::RemoveSpaceMember { legacy_member_id }
                if legacy_member_id == "4123456789abcde"
        ));

        let invalid = br#"{"schema_version":"frame.web-membership-action-request.v1","memberIds":[],"idempotency_key":"membership-remove-many-1"}"#;
        assert_eq!(
            decode_wire(MembershipActionV1::BatchRemoveSpaceMembers, invalid),
            Err(BrowserWebFailure::Invalid)
        );

        let maximum = vec!["3123456789abcde"; MAX_MEMBERSHIP_TARGETS];
        let maximum = serde_json::to_vec(&serde_json::json!({
            "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            "space_id": "2123456789abcde",
            "user_ids": maximum,
            "role": "member",
            "idempotency_key": "membership-add-maximum-1",
        }))
        .expect("maximum body");
        let (input, _) = decode_wire(MembershipActionV1::AddSpaceMembers, &maximum)
            .expect("500 targets are valid");
        assert!(matches!(
            input,
            LegacyMembershipInputV1::AddSpaceMembers {
                legacy_user_ids,
                ..
            } if legacy_user_ids.len() == MAX_MEMBERSHIP_TARGETS
        ));

        let too_many = vec!["3123456789abcde"; MAX_MEMBERSHIP_TARGETS + 1];
        let too_many = serde_json::to_vec(&serde_json::json!({
            "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            "space_id": "2123456789abcde",
            "user_ids": too_many,
            "role": "member",
            "idempotency_key": "membership-add-too-many-1",
        }))
        .expect("oversized target body");
        assert_eq!(
            decode_wire(MembershipActionV1::AddSpaceMembers, &too_many),
            Err(BrowserWebFailure::Invalid)
        );
        let too_many_removals = serde_json::to_vec(&serde_json::json!({
            "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            "member_ids": vec!["4123456789abcde"; MAX_MEMBERSHIP_TARGETS + 1],
            "idempotency_key": "membership-remove-too-many-1",
        }))
        .expect("oversized removal body");
        assert_eq!(
            decode_wire(
                MembershipActionV1::BatchRemoveSpaceMembers,
                &too_many_removals,
            ),
            Err(BrowserWebFailure::Invalid)
        );

        for invalid in [
            br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","userIds":[],"role":"member","idempotency_key":"membership-add-many-1"}"#.as_slice(),
            br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":[],"role":"owner","idempotency_key":"membership-add-many-1"}"#.as_slice(),
        ] {
            assert_eq!(
                decode_wire(MembershipActionV1::AddSpaceMembers, invalid),
                Err(BrowserWebFailure::Invalid)
            );
        }
        let unknown = br#"{"schema_version":"frame.web-membership-action-request.v1","member_id":"4123456789abcde","idempotency_key":"membership-remove-one-1","unexpected":true}"#;
        assert_eq!(
            decode_wire(MembershipActionV1::RemoveSpaceMember, unknown),
            Err(BrowserWebFailure::Invalid)
        );
    }

    #[test]
    fn response_projection_preserves_cap_ids_order_duplicates_and_empty_noop() {
        let added = project_success(LegacyMembershipSuccessV1::SpaceMembersAdded {
            added: vec!["3123456789abcde".into()],
            already_members: vec!["4123456789abcde".into(), "5123456789abcde".into()],
        });
        let debug = format!("{added:?}");
        assert!(debug.contains("added_count: 1"));
        assert!(!debug.contains("3123456789abcde"));
        assert_eq!(
            added,
            WebMembershipActionEffectV1::SpaceMembersAdded {
                added: vec!["3123456789abcde".into()],
                already_members: vec!["4123456789abcde".into(), "5123456789abcde".into()],
            }
        );

        let removed = project_success(LegacyMembershipSuccessV1::SpaceMembersRemoved {
            removed_member_ids: vec![
                frame_application::LegacySpaceMemberIdV1::from_legacy("6123456789abcde".into())
                    .expect("first member"),
                frame_application::LegacySpaceMemberIdV1::from_legacy("6123456789abcde".into())
                    .expect("duplicate member"),
                frame_application::LegacySpaceMemberIdV1::from_legacy("7123456789abcde".into())
                    .expect("second member"),
            ],
        });
        let debug = format!("{removed:?}");
        assert!(debug.contains("removed_count: 3"));
        assert!(!debug.contains("6123456789abcde"));
        assert_eq!(
            removed,
            WebMembershipActionEffectV1::SpaceMembersRemoved {
                removed_member_ids: vec![
                    "6123456789abcde".into(),
                    "6123456789abcde".into(),
                    "7123456789abcde".into(),
                ],
            },
            "the response must expose original Cap row IDs, never mapped UUIDs",
        );
        assert_eq!(
            project_success(LegacyMembershipSuccessV1::SpaceMembersRemoved {
                removed_member_ids: Vec::new(),
            }),
            WebMembershipActionEffectV1::SpaceMembersRemoved {
                removed_member_ids: Vec::new(),
            }
        );
        assert_eq!(
            project_success(LegacyMembershipSuccessV1::SpaceMemberRemoved),
            WebMembershipActionEffectV1::SuccessObject
        );
    }

    #[test]
    fn optional_presence_is_preserved_and_explicit_null_is_rejected() {
        let missing = br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":[],"idempotency_key":"membership-set-1"}"#;
        let (input, _) = decode_wire(MembershipActionV1::SetSpaceMembers, missing)
            .expect("missing optional values");
        let LegacyMembershipInputV1::SetSpaceMembers { role, members, .. } = input else {
            panic!("set input");
        };
        assert_eq!(role, None);
        assert_eq!(members, None);

        let empty = br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":["3123456789abcde"],"members":[],"idempotency_key":"membership-set-2"}"#;
        let (input, _) =
            decode_wire(MembershipActionV1::SetSpaceMembers, empty).expect("present empty members");
        let LegacyMembershipInputV1::SetSpaceMembers { members, .. } = input else {
            panic!("set input");
        };
        assert_eq!(members, Some(Vec::new()));

        for invalid in [
            br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":[],"role":null,"idempotency_key":"membership-set-3"}"#.as_slice(),
            br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":[],"members":null,"idempotency_key":"membership-set-4"}"#.as_slice(),
        ] {
            assert_eq!(
                decode_wire(MembershipActionV1::SetSpaceMembers, invalid),
                Err(BrowserWebFailure::Invalid)
            );
        }
    }

    #[test]
    fn structural_validation_is_bounded_exact_and_redacted() {
        let valid = br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":[],"members":[{"user_id":"3123456789abcde","role":"admin"}],"idempotency_key":"membership-set-1"}"#;
        let (input, key) =
            decode_wire(MembershipActionV1::SetSpaceMembers, valid).expect("valid explicit member");
        assert_eq!(key, "membership-set-1");
        let debug = format!("{input:?}");
        assert!(!debug.contains("3123456789abcde"));
        assert!(!debug.contains("admin"));

        let unknown = br#"{"schema_version":"frame.web-membership-action-request.v1","space_id":"2123456789abcde","user_ids":[],"members":[{"user_id":"3123456789abcde","role":"admin","extra":true}],"idempotency_key":"membership-set-1"}"#;
        assert_eq!(
            decode_wire(MembershipActionV1::SetSpaceMembers, unknown),
            Err(BrowserWebFailure::Invalid)
        );
        assert!(valid_cap_nanoid("0123456789abcde"));
        assert!(!valid_cap_nanoid("0123456789abcdi"));
        assert!(valid_idempotency_key("membership-1"));
        assert!(!valid_idempotency_key("short"));
    }
}
