//! Browser-direct authenticated web boundary required by ADR 0004.
//!
//! Render never receives or forwards credentials. The browser sends its
//! host-only session cookie directly to this Worker, which reuses the D1 auth
//! repository for every session decision. Mutations additionally require the
//! exact Origin, `Sec-Fetch-Site: same-origin`, double-submit CSRF, and consume
//! the repository-minted one-use grant in the same D1 batch as the product
//! effect and idempotency receipt.

use frame_application::{
    AuthFailure, AuthHashKey, AuthHashKeyRing, AuthPolicy, AuthService, BrowserMutationRequest,
    OAuthProviderPolicy, ValidatedBrowserMutationProof,
};
use frame_domain::{
    ApiKeySecret, AuthClientKind, CorrelationId, CsrfToken, DurationMillis, ExactBrowserOrigin,
    ExactOAuthCallbackUrl, FetchSite, HashKeyVersion, MultiRateLimitPolicy, OAuthAudience,
    OAuthProvider, OAuthState, OpaqueAuthToken, PkceVerifier, RateLimitPolicy,
    SealedDeliveryEnvelope, TimestampMillis, VerificationChannel, VerificationSecret,
};
use frame_ports::{
    AuthDeliverySealer, AuthSecretSource, Clock, PortError, VerificationDeliveryMaterial,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Env, Error, Request, Result, send::IntoSendFuture};

use crate::{auth_repository::D1AuthStateRepository, authenticated_web_runtime};

pub const WEB_ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-action-request.v1";
pub const WEB_ACTION_RECEIPT_SCHEMA_V1: &str = "frame.web-action-receipt.v1";
pub const SESSION_COOKIE_NAME: &str = "__Host-frame_session";
pub const CSRF_COOKIE_NAME: &str = "__Host-frame_csrf";

const AUTH_KEYRING_SECRET: &str = "FRAME_AUTH_HASH_KEYRING_V1";
const MAX_ACTION_VALUE_BYTES: usize = 120;
const MAX_ACTION_BODY_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserWebFailure {
    Unauthenticated,
    Forbidden,
    Invalid,
    Conflict,
    NotFound,
    Unavailable,
}

pub type BrowserWebOutcome<T> = std::result::Result<T, BrowserWebFailure>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebActionRequestV1 {
    pub schema_version: String,
    pub expected_revision: u64,
    pub selection_revision: u64,
    pub selection_context: String,
    pub idempotency_key: String,
    pub value: Option<String>,
    pub resource_id: Option<String>,
}

impl WebActionRequestV1 {
    pub fn validate(&self, action: WebAction) -> BrowserWebOutcome<()> {
        let value = self.value.as_deref();
        if self.schema_version != WEB_ACTION_REQUEST_SCHEMA_V1
            || self.expected_revision > 9_007_199_254_740_991
            || self.selection_revision > 9_007_199_254_740_991
            || !valid_selection_context(&self.selection_context)
            || !valid_operation_id(&self.idempotency_key)
            || value.is_some_and(|value| {
                value.trim() != value
                    || value.is_empty()
                    || value.len() > MAX_ACTION_VALUE_BYTES
                    || value.chars().any(char::is_control)
                    || value.contains(['<', '>'])
            })
            || self
                .resource_id
                .as_deref()
                .is_some_and(|value| !valid_resource_id(value))
            || (action.requires_value() && value.is_none())
            || (action != WebAction::CreateFolder && self.resource_id.is_some())
        {
            return Err(BrowserWebFailure::Invalid);
        }
        Ok(())
    }

    #[must_use]
    pub fn encoded_len(&self) -> usize {
        serde_json::to_vec(self).map_or(MAX_ACTION_BODY_BYTES + 1, |body| body.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebActionReceiptV1 {
    pub schema_version: String,
    pub action: String,
    pub effect_state: WebActionEffectState,
    pub revision: u64,
    pub invalidated: Vec<String>,
}

impl WebActionReceiptV1 {
    fn validate(&self, expected_action: WebAction) -> BrowserWebOutcome<()> {
        if self.schema_version != WEB_ACTION_RECEIPT_SCHEMA_V1
            || self.action != expected_action.as_str()
            || self.effect_state != expected_action.effect_state()
            || self.revision > 9_007_199_254_740_991
            || self.invalidated != expected_action.invalidated()
        {
            return Err(BrowserWebFailure::Unavailable);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebActionEffectState {
    Applied,
    PendingProtectedExecution,
}

impl WebActionEffectState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::PendingProtectedExecution => "pending_protected_execution",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebAction {
    CompleteOnboarding,
    CreateSpace,
    CreateFolder,
    StartImport,
    UpdateAccount,
    UpdateOrganization,
    UpdateMembers,
    UpdateStorage,
    CreateDeveloperKey,
    UpdateBilling,
    AdminAction,
}

impl WebAction {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "organization.onboarding.complete.v1" => Some(Self::CompleteOnboarding),
            "organization.spaces.create.v1" => Some(Self::CreateSpace),
            "organization.folders.create.v1" => Some(Self::CreateFolder),
            "business.imports.start.v1" => Some(Self::StartImport),
            "identity.account.update.v1" => Some(Self::UpdateAccount),
            "organization.settings.update.v1" => Some(Self::UpdateOrganization),
            "organization.members.manage.v1" => Some(Self::UpdateMembers),
            "business.storage.configure.v1" => Some(Self::UpdateStorage),
            "business.developer.credentials.manage.v1" => Some(Self::CreateDeveloperKey),
            "business.billing.manage.v1" => Some(Self::UpdateBilling),
            "business.admin.execute.v1" => Some(Self::AdminAction),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CompleteOnboarding => "organization.onboarding.complete.v1",
            Self::CreateSpace => "organization.spaces.create.v1",
            Self::CreateFolder => "organization.folders.create.v1",
            Self::StartImport => "business.imports.start.v1",
            Self::UpdateAccount => "identity.account.update.v1",
            Self::UpdateOrganization => "organization.settings.update.v1",
            Self::UpdateMembers => "organization.members.manage.v1",
            Self::UpdateStorage => "business.storage.configure.v1",
            Self::CreateDeveloperKey => "business.developer.credentials.manage.v1",
            Self::UpdateBilling => "business.billing.manage.v1",
            Self::AdminAction => "business.admin.execute.v1",
        }
    }

    #[must_use]
    pub fn permitted_for(self, role: &str) -> bool {
        match self {
            Self::CompleteOnboarding | Self::UpdateAccount => {
                matches!(role, "owner" | "admin" | "member")
            }
            Self::CreateSpace
            | Self::CreateFolder
            | Self::StartImport
            | Self::UpdateOrganization
            | Self::UpdateMembers
            | Self::UpdateStorage
            | Self::CreateDeveloperKey
            | Self::AdminAction => matches!(role, "owner" | "admin"),
            Self::UpdateBilling => role == "owner",
        }
    }

    #[must_use]
    const fn requires_value(self) -> bool {
        matches!(
            self,
            Self::CreateSpace | Self::CreateFolder | Self::UpdateAccount | Self::UpdateOrganization
        )
    }

    #[must_use]
    pub fn invalidated(self) -> Vec<String> {
        let values: &[&str] = match self {
            Self::CompleteOnboarding => &["session", "workspace"],
            Self::CreateSpace => &["spaces", "workspace"],
            Self::CreateFolder => &["folders", "library"],
            Self::StartImport => &["imports", "library"],
            Self::UpdateAccount => &["account", "session"],
            Self::UpdateOrganization => &["organization", "workspace"],
            Self::UpdateMembers => &["members", "workspace"],
            Self::UpdateStorage => &["storage", "imports"],
            Self::CreateDeveloperKey => &["developer"],
            Self::UpdateBilling => &["billing", "organization"],
            Self::AdminAction => &["admin", "workspace"],
        };
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[must_use]
    pub const fn effect_state(self) -> WebActionEffectState {
        match self {
            Self::CreateSpace
            | Self::CreateFolder
            | Self::StartImport
            | Self::UpdateAccount
            | Self::UpdateOrganization => WebActionEffectState::Applied,
            Self::CompleteOnboarding
            | Self::UpdateMembers
            | Self::UpdateStorage
            | Self::CreateDeveloperKey
            | Self::UpdateBilling
            | Self::AdminAction => WebActionEffectState::PendingProtectedExecution,
        }
    }
}

#[derive(Debug)]
struct BrowserAdmission {
    token: OpaqueAuthToken,
    user_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct MembershipRow {
    organization_id: String,
    role: String,
    revision: i64,
    selection_revision: i64,
}

#[derive(Debug, Deserialize)]
struct RevisionRow {
    revision: i64,
}

#[derive(Debug, Deserialize)]
struct ExistingOperationRow {
    request_digest: String,
    state: String,
    response_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SpaceRow {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HashKeyRingWire {
    active: HashKeyWire,
    #[serde(default)]
    fallback: Vec<HashKeyWire>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HashKeyWire {
    version: u16,
    material_hex: String,
}

#[derive(Debug, Clone, Copy)]
struct FixedClock(TimestampMillis);

impl Clock for FixedClock {
    fn now(&self) -> std::result::Result<TimestampMillis, PortError> {
        Ok(self.0)
    }
}

/// Session verification never invokes these issuance methods. Keeping them
/// fail-closed makes accidental expansion of this adapter unavailable rather
/// than generating weak material.
#[derive(Debug, Clone, Copy)]
struct UnavailableSecretSource;

impl AuthSecretSource for UnavailableSecretSource {
    fn session_token(&self) -> std::result::Result<OpaqueAuthToken, PortError> {
        Err(unavailable_port())
    }

    fn csrf_token(&self) -> std::result::Result<CsrfToken, PortError> {
        Err(unavailable_port())
    }

    fn api_key(&self) -> std::result::Result<ApiKeySecret, PortError> {
        Err(unavailable_port())
    }

    fn oauth_state(&self) -> std::result::Result<OAuthState, PortError> {
        Err(unavailable_port())
    }

    fn pkce_verifier(&self) -> std::result::Result<PkceVerifier, PortError> {
        Err(unavailable_port())
    }

    fn verification_secret(
        &self,
        _: VerificationChannel,
    ) -> std::result::Result<VerificationSecret, PortError> {
        Err(unavailable_port())
    }
}

#[derive(Debug, Clone, Copy)]
struct UnavailableDeliverySealer;

impl AuthDeliverySealer for UnavailableDeliverySealer {
    fn seal(
        &self,
        _: &VerificationDeliveryMaterial,
        _: TimestampMillis,
    ) -> std::result::Result<SealedDeliveryEnvelope, PortError> {
        Err(unavailable_port())
    }
}

fn unavailable_port() -> PortError {
    PortError::Unsupported("browser session verifier cannot issue auth material".into())
}

pub async fn decode_action_request(
    request: &mut Request,
) -> Result<BrowserWebOutcome<WebActionRequestV1>> {
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
    let declared_length = request
        .headers()
        .get("content-length")?
        .map(|value| value.parse::<usize>())
        .transpose()
        .ok()
        .flatten();
    if declared_length.is_some_and(|length| length == 0 || length > MAX_ACTION_BODY_BYTES) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    let bytes = request.bytes().await?;
    if bytes.is_empty()
        || bytes.len() > MAX_ACTION_BODY_BYTES
        || declared_length.is_some_and(|length| length != bytes.len())
    {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    Ok(
        serde_json::from_slice::<WebActionRequestV1>(&bytes)
            .map_err(|_| BrowserWebFailure::Invalid),
    )
}

pub async fn load(
    request: &Request,
    env: &Env,
    surface: &str,
    query: &authenticated_web_runtime::WebLoadQuery,
    now_ms: i64,
) -> Result<BrowserWebOutcome<authenticated_web_runtime::WebWorkspaceV1>> {
    let admission = match authenticate_read(request, env, now_ms).await? {
        Ok(admission) => admission,
        Err(failure) => return Ok(Err(failure)),
    };
    let database = env.d1("DB")?;
    let membership = match active_membership(&database, &admission.user_id).await? {
        Some(membership) => membership,
        None => return Ok(Err(BrowserWebFailure::NotFound)),
    };
    let selection_revision = u64::try_from(membership.selection_revision)
        .map_err(|_| Error::RustError("browser organization selection is invalid".into()))?;
    let membership_revision = u64::try_from(membership.revision)
        .map_err(|_| Error::RustError("browser organization membership is invalid".into()))?;
    let selection_context = selection_context(
        &admission.user_id,
        &membership.organization_id,
        selection_revision,
    );
    match authenticated_web_runtime::load(
        &database,
        authenticated_web_runtime::WebLoadAuthority {
            tenant_id: &membership.organization_id,
            user_id: &admission.user_id,
            selection_revision,
            selection_context: &selection_context,
            membership_role: &membership.role,
            membership_revision,
        },
        surface,
        query,
    )
    .await?
    {
        Ok(workspace) => {
            let current_membership = active_membership(&database, &admission.user_id).await?;
            if !load_authority_is_current(&membership, current_membership.as_ref(), &workspace.role)
            {
                return Ok(Err(BrowserWebFailure::NotFound));
            }
            Ok(Ok(workspace))
        }
        Err(authenticated_web_runtime::WebLoadFailure::Invalid) => {
            Ok(Err(BrowserWebFailure::Invalid))
        }
        Err(authenticated_web_runtime::WebLoadFailure::Unavailable) => {
            Ok(Err(BrowserWebFailure::NotFound))
        }
    }
}

pub async fn mutate(
    request: &Request,
    env: &Env,
    action_text: &str,
    body: &WebActionRequestV1,
    now_ms: i64,
) -> Result<BrowserWebOutcome<WebActionReceiptV1>> {
    let Some(action) = WebAction::parse(action_text) else {
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    if body.encoded_len() > MAX_ACTION_BODY_BYTES || body.validate(action).is_err() {
        return Ok(Err(BrowserWebFailure::Invalid));
    }
    if request.headers().get("idempotency-key")?.as_deref() != Some(body.idempotency_key.as_str()) {
        return Ok(Err(BrowserWebFailure::Invalid));
    }

    let admission = match authenticate_read(request, env, now_ms).await? {
        Ok(admission) => admission,
        Err(failure) => return Ok(Err(failure)),
    };
    let database = env.d1("DB")?;
    let membership = match active_membership(&database, &admission.user_id).await? {
        Some(membership) => membership,
        None => return Ok(Err(BrowserWebFailure::NotFound)),
    };
    let selection_revision = u64::try_from(membership.selection_revision)
        .map_err(|_| Error::RustError("browser organization selection is invalid".into()))?;
    if body.selection_revision != selection_revision
        || body.selection_context
            != selection_context(
                &admission.user_id,
                &membership.organization_id,
                selection_revision,
            )
    {
        return Ok(Err(BrowserWebFailure::Conflict));
    }
    if !action.permitted_for(&membership.role) {
        return Ok(Err(BrowserWebFailure::NotFound));
    }
    let proof = match authenticate_mutation(request, env, &admission, now_ms).await? {
        Ok(proof) => proof,
        Err(failure) => return Ok(Err(failure)),
    };
    if proof.user_id().to_string() != admission.user_id {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }

    execute_action(&database, &membership, action, body, proof, now_ms).await
}

async fn authenticate_read(
    request: &Request,
    env: &Env,
    now_ms: i64,
) -> Result<BrowserWebOutcome<BrowserAdmission>> {
    if forbidden_browser_headers(request)? || !same_origin_fetch(request)? {
        return Ok(Err(BrowserWebFailure::Forbidden));
    }
    if let Some(origin) = request.headers().get("origin")?
        && origin != request_origin(request)?
    {
        return Ok(Err(BrowserWebFailure::Forbidden));
    }
    let Some(token_text) = unique_cookie(request, SESSION_COOKIE_NAME)? else {
        return Ok(Err(BrowserWebFailure::Unauthenticated));
    };
    let Ok(token) = OpaqueAuthToken::parse(token_text) else {
        return Ok(Err(BrowserWebFailure::Unauthenticated));
    };
    let now = TimestampMillis::new(now_ms)
        .map_err(|_| Error::RustError("browser auth clock is invalid".into()))?;
    let database = env.d1("DB")?;
    let repository = D1AuthStateRepository::new(&database);
    let clock = FixedClock(now);
    let secrets = UnavailableSecretSource;
    let sealer = UnavailableDeliverySealer;
    let hash_keys = match auth_hash_keyring(env) {
        Ok(keys) => keys,
        Err(failure) => return Ok(Err(failure)),
    };
    let policy = match verifier_policy() {
        Ok(policy) => policy,
        Err(failure) => return Ok(Err(failure)),
    };
    let service = AuthService::new(&repository, &clock, &secrets, &sealer, hash_keys, policy);
    match service.authenticate(&token, CorrelationId::new()).await {
        Ok(identity) if identity.client_kind() == AuthClientKind::Browser => {
            Ok(Ok(BrowserAdmission {
                token,
                user_id: identity.user_id().to_string(),
            }))
        }
        Ok(_) | Err(AuthFailure::Unauthenticated | AuthFailure::RequestRejected) => {
            Ok(Err(BrowserWebFailure::Unauthenticated))
        }
        Err(AuthFailure::InvalidRequest) => Ok(Err(BrowserWebFailure::Invalid)),
        Err(AuthFailure::RateLimited) => Ok(Err(BrowserWebFailure::Forbidden)),
        Err(AuthFailure::Unavailable) => Ok(Err(BrowserWebFailure::Unavailable)),
    }
}

async fn authenticate_mutation(
    request: &Request,
    env: &Env,
    admission: &BrowserAdmission,
    now_ms: i64,
) -> Result<BrowserWebOutcome<ValidatedBrowserMutationProof>> {
    let origin = request.headers().get("origin")?;
    let Some(origin) =
        origin.filter(|origin| origin == &request_origin(request).unwrap_or_default())
    else {
        return Ok(Err(BrowserWebFailure::Forbidden));
    };
    if !same_origin_fetch(request)? {
        return Ok(Err(BrowserWebFailure::Forbidden));
    }
    let Some(csrf_cookie_text) = unique_cookie(request, CSRF_COOKIE_NAME)? else {
        return Ok(Err(BrowserWebFailure::Forbidden));
    };
    let Some(csrf_header_text) = request.headers().get("x-frame-csrf")? else {
        return Ok(Err(BrowserWebFailure::Forbidden));
    };
    let (Ok(csrf_cookie), Ok(csrf_header)) = (
        CsrfToken::parse(csrf_cookie_text),
        CsrfToken::parse(csrf_header_text),
    ) else {
        return Ok(Err(BrowserWebFailure::Forbidden));
    };
    let now = TimestampMillis::new(now_ms)
        .map_err(|_| Error::RustError("browser auth clock is invalid".into()))?;
    let database = env.d1("DB")?;
    let repository = D1AuthStateRepository::new(&database);
    let clock = FixedClock(now);
    let secrets = UnavailableSecretSource;
    let sealer = UnavailableDeliverySealer;
    let hash_keys = match auth_hash_keyring(env) {
        Ok(keys) => keys,
        Err(failure) => return Ok(Err(failure)),
    };
    let policy = match verifier_policy() {
        Ok(policy) => policy,
        Err(failure) => return Ok(Err(failure)),
    };
    let service = AuthService::new(&repository, &clock, &secrets, &sealer, hash_keys, policy);
    match service
        .validate_browser_mutation(
            &admission.token,
            BrowserMutationRequest {
                origin: &origin,
                fetch_site: FetchSite::SameOrigin,
                csrf_cookie: &csrf_cookie,
                csrf_header: &csrf_header,
            },
            CorrelationId::new(),
        )
        .await
    {
        Ok(proof) => Ok(Ok(proof)),
        Err(AuthFailure::Unauthenticated) => Ok(Err(BrowserWebFailure::Unauthenticated)),
        Err(AuthFailure::RequestRejected | AuthFailure::RateLimited) => {
            Ok(Err(BrowserWebFailure::Forbidden))
        }
        Err(AuthFailure::InvalidRequest) => Ok(Err(BrowserWebFailure::Invalid)),
        Err(AuthFailure::Unavailable) => Ok(Err(BrowserWebFailure::Unavailable)),
    }
}

async fn active_membership(database: &D1Database, user_id: &str) -> Result<Option<MembershipRow>> {
    let membership = database
        .prepare(
            "SELECT m.organization_id,m.role,m.revision, \
                    u.organization_preference_revision AS selection_revision \
             FROM users u \
             JOIN organization_members m ON m.user_id=u.id \
               AND m.organization_id=u.active_organization_id AND m.state='active' \
               AND m.role IN ('owner','admin','member') \
             JOIN organizations o ON o.id=m.organization_id AND o.status='active' \
             WHERE u.id=?1 AND u.status='active' AND u.deleted_at_ms IS NULL LIMIT 1",
        )
        .bind(&[JsValue::from_str(user_id)])?
        .first::<MembershipRow>(None)
        .await?;
    Ok(membership.filter(|row| {
        valid_uuid(&row.organization_id)
            && supported_browser_role(&row.role)
            && (0..=9_007_199_254_740_991).contains(&row.revision)
            && (0..=9_007_199_254_740_991).contains(&row.selection_revision)
    }))
}

fn supported_browser_role(role: &str) -> bool {
    matches!(role, "owner" | "admin" | "member")
}

fn load_authority_is_current(
    expected: &MembershipRow,
    current: Option<&MembershipRow>,
    workspace_role: &str,
) -> bool {
    current == Some(expected) && workspace_role == expected.role
}

async fn execute_action(
    database: &D1Database,
    membership: &MembershipRow,
    action: WebAction,
    body: &WebActionRequestV1,
    proof: ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<BrowserWebOutcome<WebActionReceiptV1>> {
    let user_id = proof.user_id().to_string();
    let request_digest = request_digest(action, body)?;
    if let Some(existing) = existing_operation(
        database,
        &membership.organization_id,
        &user_id,
        action,
        &body.idempotency_key,
    )
    .await?
    {
        let consumed = consume_grant(database, membership, &proof, now_ms).await?;
        if !consumed {
            return Ok(Err(BrowserWebFailure::Unavailable));
        }
        if existing.request_digest != request_digest {
            return Ok(Err(BrowserWebFailure::Conflict));
        }
        let Some(response_json) = existing
            .response_json
            .filter(|_| existing.state == "complete")
        else {
            return Ok(Err(BrowserWebFailure::Conflict));
        };
        let receipt = serde_json::from_str::<WebActionReceiptV1>(&response_json)
            .map_err(|_| Error::RustError("stored browser action receipt is corrupt".into()))?;
        return Ok(receipt
            .validate(action)
            .map(|()| receipt)
            .map_err(|_| BrowserWebFailure::Unavailable));
    }

    let revision = database
        .prepare("SELECT revision FROM organizations WHERE id=?1 AND status='active' LIMIT 1")
        .bind(&[JsValue::from_str(&membership.organization_id)])?
        .first::<RevisionRow>(None)
        .await?;
    let Some(revision) = revision.and_then(|row| u64::try_from(row.revision).ok()) else {
        let _ = consume_grant(database, membership, &proof, now_ms).await?;
        return Ok(Err(BrowserWebFailure::NotFound));
    };
    if revision != body.expected_revision {
        let _ = consume_grant(database, membership, &proof, now_ms).await?;
        return Ok(Err(BrowserWebFailure::Conflict));
    }
    let Some(next_revision) = revision.checked_add(1) else {
        let _ = consume_grant(database, membership, &proof, now_ms).await?;
        return Ok(Err(BrowserWebFailure::Conflict));
    };

    let operation_id = uuid::Uuid::now_v7().to_string();
    let product_id = uuid::Uuid::now_v7().to_string();
    let folder_space = if action == WebAction::CreateFolder {
        select_folder_space(
            database,
            &membership.organization_id,
            body.resource_id.as_deref(),
        )
        .await?
    } else {
        None
    };
    if action == WebAction::CreateFolder && folder_space.is_none() {
        let _ = consume_grant(database, membership, &proof, now_ms).await?;
        return Ok(Err(BrowserWebFailure::Invalid));
    }

    let receipt = WebActionReceiptV1 {
        schema_version: WEB_ACTION_RECEIPT_SCHEMA_V1.into(),
        action: action.as_str().into(),
        effect_state: action.effect_state(),
        revision: next_revision,
        invalidated: action.invalidated(),
    };
    let response_json = serde_json::to_string(&receipt)
        .map_err(|_| Error::RustError("browser action receipt is unavailable".into()))?;
    let value_json = serde_json::to_string(&serde_json::json!({
        "value": body.value,
        "resource_id": body.resource_id,
    }))
    .map_err(|_| Error::RustError("browser action effect is unavailable".into()))?;

    let mut statements = Vec::with_capacity(18);
    push_action_assertions(
        database,
        &mut statements,
        &operation_id,
        membership,
        body.expected_revision,
        &proof,
        now_ms,
    )?;
    statements.push(
        database
            .prepare(
                "INSERT INTO authenticated_web_action_operations_v1( \
                   operation_id,organization_id,user_id,action,idempotency_key,request_digest, \
                   state,response_json,created_at_ms,completed_at_ms) \
                 VALUES (?1,?2,?3,?4,?5,?6,'claimed',NULL,?7,NULL)",
            )
            .bind(&[
                JsValue::from_str(&operation_id),
                JsValue::from_str(&membership.organization_id),
                JsValue::from_str(&user_id),
                JsValue::from_str(action.as_str()),
                JsValue::from_str(&body.idempotency_key),
                JsValue::from_str(&request_digest),
                JsValue::from_f64(now_ms as f64),
            ])?,
    );
    let has_product_effect = push_product_effect(
        database,
        &mut statements,
        action,
        body,
        membership,
        &user_id,
        &product_id,
        folder_space.as_deref(),
        now_ms,
    )?;
    if has_product_effect != (action.effect_state() == WebActionEffectState::Applied) {
        return Err(Error::RustError(
            "browser action effect disposition is inconsistent".into(),
        ));
    }
    if has_product_effect {
        statements.push(change_assertion_statement(
            database,
            &operation_id,
            "product_effect",
        )?);
    }
    statements.push(
        database
            .prepare(
                "INSERT INTO authenticated_web_action_effects_v1( \
                   operation_id,organization_id,user_id,action,effect_state,value_json,created_at_ms) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )
            .bind(&[
                JsValue::from_str(&operation_id),
                JsValue::from_str(&membership.organization_id),
                JsValue::from_str(&user_id),
                JsValue::from_str(action.as_str()),
                JsValue::from_str(action.effect_state().as_str()),
                JsValue::from_str(&value_json),
                JsValue::from_f64(now_ms as f64),
            ])?,
    );
    statements.push(change_assertion_statement(
        database,
        &operation_id,
        "action_effect",
    )?);
    statements.push(
        database
            .prepare(
                "UPDATE organizations SET revision=revision+1,updated_at_ms=?3 \
                 WHERE id=?1 AND revision=?2 AND status='active'",
            )
            .bind(&[
                JsValue::from_str(&membership.organization_id),
                JsValue::from_f64(body.expected_revision as f64),
                JsValue::from_f64(now_ms as f64),
            ])?,
    );
    statements.push(change_assertion_statement(
        database,
        &operation_id,
        "organization_update",
    )?);
    statements.push(
        database
            .prepare(
                "UPDATE authenticated_web_action_operations_v1 \
                 SET state='complete',response_json=?2,completed_at_ms=?3 \
                 WHERE operation_id=?1 AND state='claimed'",
            )
            .bind(&[
                JsValue::from_str(&operation_id),
                JsValue::from_str(&response_json),
                JsValue::from_f64(now_ms as f64),
            ])?,
    );
    statements.push(change_assertion_statement(
        database,
        &operation_id,
        "operation_complete",
    )?);
    statements.push(grant_delete_statement(database, &proof)?);
    statements.push(change_assertion_statement(
        database,
        &operation_id,
        "grant_consumed",
    )?);
    statements.push(
        database
            .prepare("DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?1")
            .bind(&[JsValue::from_str(&operation_id)])?,
    );

    let results = match database.batch(statements).into_send().await {
        Ok(results) => results,
        Err(_) => return Ok(Err(BrowserWebFailure::Unavailable)),
    };
    if results.is_empty() || results.iter().any(|result| !result.success()) {
        return Ok(Err(BrowserWebFailure::Unavailable));
    }
    Ok(Ok(receipt))
}

fn push_action_assertions(
    database: &D1Database,
    statements: &mut Vec<D1PreparedStatement>,
    operation_id: &str,
    membership: &MembershipRow,
    expected_revision: u64,
    proof: &ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<()> {
    statements.push(
        database
            .prepare(
                "INSERT INTO authenticated_web_action_assertions_v1( \
                   operation_id,assertion_kind,expected_count,actual_count) \
                 VALUES (?1,'organization_revision',?2,( \
                   SELECT revision FROM organizations WHERE id=?3 AND status='active'))",
            )
            .bind(&[
                JsValue::from_str(operation_id),
                JsValue::from_f64(expected_revision as f64),
                JsValue::from_str(&membership.organization_id),
            ])?,
    );
    push_current_authority_assertions(
        database,
        statements,
        operation_id,
        membership,
        proof,
        now_ms,
    )?;
    Ok(())
}

fn push_current_authority_assertions(
    database: &D1Database,
    statements: &mut Vec<D1PreparedStatement>,
    operation_id: &str,
    membership: &MembershipRow,
    proof: &ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<()> {
    statements.push(
        database
            .prepare(
                "INSERT INTO authenticated_web_action_assertions_v1( \
                   operation_id,assertion_kind,expected_count,actual_count) \
                 VALUES (?1,'selection_authority',1,(SELECT COUNT(*) \
                   FROM users u \
                   JOIN organizations o ON o.id=u.active_organization_id AND o.status='active' \
                   WHERE u.id=?2 AND u.status='active' AND u.deleted_at_ms IS NULL \
                     AND u.active_organization_id=?3 \
                     AND u.organization_preference_revision=?4))",
            )
            .bind(&[
                JsValue::from_str(operation_id),
                JsValue::from_str(&proof.user_id().to_string()),
                JsValue::from_str(&membership.organization_id),
                JsValue::from_f64(membership.selection_revision as f64),
            ])?,
    );
    statements.push(
        database
            .prepare(
                "INSERT INTO authenticated_web_action_assertions_v1( \
                   operation_id,assertion_kind,expected_count,actual_count) \
                 VALUES (?1,'membership_authority',1,(SELECT COUNT(*) \
                   FROM organization_members m \
                   JOIN organizations o ON o.id=m.organization_id AND o.status='active' \
                   JOIN users u ON u.id=m.user_id AND u.status='active' \
                     AND u.deleted_at_ms IS NULL \
                   WHERE m.organization_id=?2 AND m.user_id=?3 AND m.state='active' \
                     AND m.role=?4 AND m.revision=?5))",
            )
            .bind(&[
                JsValue::from_str(operation_id),
                JsValue::from_str(&membership.organization_id),
                JsValue::from_str(&proof.user_id().to_string()),
                JsValue::from_str(&membership.role),
                JsValue::from_f64(membership.revision as f64),
            ])?,
    );
    statements.push(grant_assertion_statement(
        database,
        operation_id,
        proof,
        now_ms,
    )?);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_product_effect(
    database: &D1Database,
    statements: &mut Vec<D1PreparedStatement>,
    action: WebAction,
    body: &WebActionRequestV1,
    membership: &MembershipRow,
    user_id: &str,
    product_id: &str,
    folder_space: Option<&str>,
    now_ms: i64,
) -> Result<bool> {
    let statement = match action {
        WebAction::CreateSpace => database
            .prepare(
                "INSERT INTO spaces(id,organization_id,created_by_user_id,name,is_primary,is_public, \
                   settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision) \
                 VALUES (?1,?2,?3,?4,0,0,'{}',?5,?5,NULL,0)",
            )
            .bind(&[
                JsValue::from_str(product_id),
                JsValue::from_str(&membership.organization_id),
                JsValue::from_str(user_id),
                JsValue::from_str(body.value.as_deref().unwrap_or_default()),
                JsValue::from_f64(now_ms as f64),
            ])?,
        WebAction::CreateFolder => database
            .prepare(
                "INSERT INTO folders(id,organization_id,space_id,parent_id,created_by_user_id,name, \
                   is_public,settings_json,created_at_ms,updated_at_ms,deleted_at_ms,revision) \
                 SELECT ?1,?2,s.id,NULL,?4,?5,0,'{}',?6,?6,NULL,0 FROM spaces s \
                 WHERE s.id=?3 AND s.organization_id=?2 AND s.deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(product_id),
                JsValue::from_str(&membership.organization_id),
                JsValue::from_str(folder_space.unwrap_or_default()),
                JsValue::from_str(user_id),
                JsValue::from_str(body.value.as_deref().unwrap_or_default()),
                JsValue::from_f64(now_ms as f64),
            ])?,
        WebAction::StartImport => database
            .prepare(
                "INSERT INTO imported_videos(id,organization_id,video_id,provider,external_id_digest, \
                   state,idempotency_key,error_class,created_at_ms,updated_at_ms) \
                 VALUES (?1,?2,NULL,'other',NULL,'queued',?3,NULL,?4,?4)",
            )
            .bind(&[
                JsValue::from_str(product_id),
                JsValue::from_str(&membership.organization_id),
                JsValue::from_str(&body.idempotency_key),
                JsValue::from_f64(now_ms as f64),
            ])?,
        WebAction::UpdateAccount => database
            .prepare(
                "UPDATE users SET display_name=?2,updated_at_ms=?3 \
                 WHERE id=?1 AND status='active' AND deleted_at_ms IS NULL",
            )
            .bind(&[
                JsValue::from_str(user_id),
                JsValue::from_str(body.value.as_deref().unwrap_or_default()),
                JsValue::from_f64(now_ms as f64),
            ])?,
        WebAction::UpdateOrganization => database
            .prepare("UPDATE organizations SET name=?2 WHERE id=?1 AND status='active'")
            .bind(&[
                JsValue::from_str(&membership.organization_id),
                JsValue::from_str(body.value.as_deref().unwrap_or_default()),
            ])?,
        WebAction::CompleteOnboarding
        | WebAction::UpdateMembers
        | WebAction::UpdateStorage
        | WebAction::CreateDeveloperKey
        | WebAction::UpdateBilling
        | WebAction::AdminAction => return Ok(false),
    };
    statements.push(statement);
    Ok(true)
}

async fn existing_operation(
    database: &D1Database,
    organization_id: &str,
    user_id: &str,
    action: WebAction,
    idempotency_key: &str,
) -> Result<Option<ExistingOperationRow>> {
    database
        .prepare(
            "SELECT request_digest,state,response_json \
             FROM authenticated_web_action_operations_v1 \
             WHERE organization_id=?1 AND user_id=?2 AND action=?3 AND idempotency_key=?4 LIMIT 1",
        )
        .bind(&[
            JsValue::from_str(organization_id),
            JsValue::from_str(user_id),
            JsValue::from_str(action.as_str()),
            JsValue::from_str(idempotency_key),
        ])?
        .first::<ExistingOperationRow>(None)
        .await
}

async fn select_folder_space(
    database: &D1Database,
    organization_id: &str,
    requested: Option<&str>,
) -> Result<Option<String>> {
    let row = if let Some(requested) = requested {
        database
            .prepare(
                "SELECT id FROM spaces WHERE id=?1 AND organization_id=?2 AND deleted_at_ms IS NULL LIMIT 1",
            )
            .bind(&[
                JsValue::from_str(requested),
                JsValue::from_str(organization_id),
            ])?
            .first::<SpaceRow>(None)
            .await?
    } else {
        database
            .prepare(
                "SELECT id FROM spaces WHERE organization_id=?1 AND deleted_at_ms IS NULL \
                 ORDER BY is_primary DESC,created_at_ms,id LIMIT 1",
            )
            .bind(&[JsValue::from_str(organization_id)])?
            .first::<SpaceRow>(None)
            .await?
    };
    Ok(row.map(|row| row.id).filter(|id| valid_uuid(id)))
}

async fn consume_grant(
    database: &D1Database,
    membership: &MembershipRow,
    proof: &ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<bool> {
    let operation_id = uuid::Uuid::now_v7().to_string();
    let mut statements = Vec::with_capacity(6);
    push_current_authority_assertions(
        database,
        &mut statements,
        &operation_id,
        membership,
        proof,
        now_ms,
    )?;
    statements.push(grant_delete_statement(database, proof)?);
    statements.push(change_assertion_statement(
        database,
        &operation_id,
        "grant_consumed",
    )?);
    statements.push(
        database
            .prepare("DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?1")
            .bind(&[JsValue::from_str(&operation_id)])?,
    );
    let expected_results = statements.len();
    match database.batch(statements).into_send().await {
        Ok(results) => {
            Ok(results.len() == expected_results && results.iter().all(|result| result.success()))
        }
        Err(_) => Ok(false),
    }
}

fn grant_assertion_statement(
    database: &D1Database,
    operation_id: &str,
    proof: &ValidatedBrowserMutationProof,
    now_ms: i64,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "INSERT INTO authenticated_web_action_assertions_v1( \
               operation_id,assertion_kind,expected_count,actual_count) \
             VALUES (?1,'mutation_grant',1,(SELECT COUNT(*) \
               FROM auth_session_mutation_grants_v2 g \
               JOIN auth_sessions_v2 s ON s.id=g.session_id AND s.user_id=g.user_id \
               JOIN auth_identities_v2 i ON i.user_id=g.user_id \
               JOIN users u ON u.id=g.user_id AND u.status='active' \
                 AND u.deleted_at_ms IS NULL \
               WHERE g.id=?2 AND g.session_id=?3 AND g.user_id=?4 \
                 AND s.state='active' AND s.generation=g.generation \
                 AND s.token_key_version=g.token_key_version \
                 AND s.token_digest=g.token_digest \
                 AND s.session_version=i.session_version \
                 AND s.idle_expires_at_ms>?5 AND s.absolute_expires_at_ms>?5))",
        )
        .bind(&[
            JsValue::from_str(operation_id),
            JsValue::from_str(&proof.mutation_grant_id().to_string()),
            JsValue::from_str(&proof.session_id().to_string()),
            JsValue::from_str(&proof.user_id().to_string()),
            JsValue::from_f64(now_ms as f64),
        ])
}

fn grant_delete_statement(
    database: &D1Database,
    proof: &ValidatedBrowserMutationProof,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "DELETE FROM auth_session_mutation_grants_v2 \
             WHERE id=?1 AND session_id=?2 AND user_id=?3",
        )
        .bind(&[
            JsValue::from_str(&proof.mutation_grant_id().to_string()),
            JsValue::from_str(&proof.session_id().to_string()),
            JsValue::from_str(&proof.user_id().to_string()),
        ])
}

fn change_assertion_statement(
    database: &D1Database,
    operation_id: &str,
    kind: &str,
) -> Result<D1PreparedStatement> {
    database
        .prepare(
            "INSERT INTO authenticated_web_action_assertions_v1( \
               operation_id,assertion_kind,expected_count,actual_count) \
             VALUES (?1,?2,1,changes())",
        )
        .bind(&[JsValue::from_str(operation_id), JsValue::from_str(kind)])
}

fn request_digest(action: WebAction, body: &WebActionRequestV1) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(action.as_str().as_bytes());
    digest.update([0]);
    digest.update(
        serde_json::to_vec(body)
            .map_err(|_| Error::RustError("browser action request is unavailable".into()))?,
    );
    Ok(format!("{:x}", digest.finalize()))
}

fn selection_context(user_id: &str, organization_id: &str, selection_revision: u64) -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame.web.active-organization-selection.v1\0");
    for value in [user_id, organization_id] {
        digest.update((value.len() as u32).to_be_bytes());
        digest.update(value.as_bytes());
    }
    digest.update(selection_revision.to_be_bytes());
    format!("{:x}", digest.finalize())
}

fn forbidden_browser_headers(request: &Request) -> Result<bool> {
    Ok(request.headers().get("authorization")?.is_some()
        || request.headers().get("x-frame-tenant-id")?.is_some())
}

fn same_origin_fetch(request: &Request) -> Result<bool> {
    Ok(request.headers().get("sec-fetch-site")?.as_deref() == Some("same-origin"))
}

fn request_origin(request: &Request) -> Result<String> {
    let url = request.url()?;
    Ok(url.origin().ascii_serialization())
}

fn unique_cookie(request: &Request, name: &str) -> Result<Option<String>> {
    let Some(header) = request.headers().get("cookie")? else {
        return Ok(None);
    };
    let mut found = None;
    for pair in header.split(';') {
        let Some((candidate, value)) = pair.trim().split_once('=') else {
            continue;
        };
        if candidate != name {
            continue;
        }
        if found.is_some() || value.is_empty() || value.len() > 512 {
            return Ok(None);
        }
        found = Some(value.to_owned());
    }
    Ok(found)
}

fn auth_hash_keyring(env: &Env) -> BrowserWebOutcome<AuthHashKeyRing> {
    let secret = env
        .secret(AUTH_KEYRING_SECRET)
        .map_err(|_| BrowserWebFailure::Unavailable)?;
    parse_hash_keyring(&secret.to_string())
}

fn parse_hash_keyring(value: &str) -> BrowserWebOutcome<AuthHashKeyRing> {
    if value.len() > 2_048 {
        return Err(BrowserWebFailure::Unavailable);
    }
    let wire = serde_json::from_str::<HashKeyRingWire>(value)
        .map_err(|_| BrowserWebFailure::Unavailable)?;
    let active = decode_hash_key(wire.active)?;
    let fallback = wire
        .fallback
        .into_iter()
        .map(decode_hash_key)
        .collect::<BrowserWebOutcome<Vec<_>>>()?;
    AuthHashKeyRing::new(active, fallback).map_err(|_| BrowserWebFailure::Unavailable)
}

fn decode_hash_key(wire: HashKeyWire) -> BrowserWebOutcome<AuthHashKey> {
    let version = HashKeyVersion::new(wire.version).map_err(|_| BrowserWebFailure::Unavailable)?;
    let material = decode_hex(&wire.material_hex).ok_or(BrowserWebFailure::Unavailable)?;
    AuthHashKey::new(version, material).map_err(|_| BrowserWebFailure::Unavailable)
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !(64..=256).contains(&value.len()) || !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0])?;
            let low = hex_nibble(pair[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn verifier_policy() -> BrowserWebOutcome<AuthPolicy> {
    let duration = |value| DurationMillis::new(value).map_err(|_| BrowserWebFailure::Unavailable);
    let rate = |maximum| -> BrowserWebOutcome<MultiRateLimitPolicy> {
        let policy = RateLimitPolicy::new(maximum, duration(60_000)?, duration(60_000)?)
            .map_err(|_| BrowserWebFailure::Unavailable)?;
        Ok(MultiRateLimitPolicy {
            identifier: policy,
            source: policy,
            device: policy,
            global: policy,
        })
    };
    AuthPolicy::new(
        duration(30 * 60 * 1_000)?,
        duration(30 * 24 * 60 * 60 * 1_000)?,
        duration(15 * 60 * 1_000)?,
        10,
        rate(100)?,
        rate(100)?,
        rate(100)?,
        rate(100)?,
        duration(10 * 60 * 1_000)?,
        vec![OAuthProviderPolicy {
            provider: OAuthProvider::Github,
            callback_url: ExactOAuthCallbackUrl::parse(
                "https://frame.engmanager.xyz/auth/callback",
            )
            .map_err(|_| BrowserWebFailure::Unavailable)?,
            audience: OAuthAudience::parse("frame-web")
                .map_err(|_| BrowserWebFailure::Unavailable)?,
        }],
        ExactBrowserOrigin::parse("https://frame.engmanager.xyz")
            .map_err(|_| BrowserWebFailure::Unavailable)?,
    )
    .map_err(|_| BrowserWebFailure::Unavailable)
}

fn valid_operation_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_selection_context(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_resource_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|value| !value.is_nil())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(action: WebAction) -> WebActionRequestV1 {
        let user_id = "018f47a6-7b1c-7f55-8f39-8f8a86900002";
        let organization_id = "018f47a6-7b1c-7f55-8f39-8f8a86900003";
        WebActionRequestV1 {
            schema_version: WEB_ACTION_REQUEST_SCHEMA_V1.into(),
            expected_revision: 7,
            selection_revision: 3,
            selection_context: selection_context(user_id, organization_id, 3),
            idempotency_key: "018f47a6-7b1c-7f55-8f39-8f8a86900001".into(),
            value: action.requires_value().then(|| "Quarterly updates".into()),
            resource_id: None,
        }
    }

    #[test]
    fn action_inventory_and_role_policy_are_closed() {
        for action in [
            WebAction::CompleteOnboarding,
            WebAction::CreateSpace,
            WebAction::CreateFolder,
            WebAction::StartImport,
            WebAction::UpdateAccount,
            WebAction::UpdateOrganization,
            WebAction::UpdateMembers,
            WebAction::UpdateStorage,
            WebAction::CreateDeveloperKey,
            WebAction::UpdateBilling,
            WebAction::AdminAction,
        ] {
            assert_eq!(WebAction::parse(action.as_str()), Some(action));
            assert!(action.permitted_for("owner"));
            assert!(!action.invalidated().is_empty());
        }
        assert!(WebAction::UpdateAccount.permitted_for("member"));
        assert!(!WebAction::CreateSpace.permitted_for("member"));
        assert!(!WebAction::UpdateBilling.permitted_for("admin"));
        assert_eq!(
            WebAction::CreateSpace.effect_state(),
            WebActionEffectState::Applied
        );
        assert_eq!(
            WebAction::UpdateBilling.effect_state(),
            WebActionEffectState::PendingProtectedExecution
        );
        assert!(!supported_browser_role("viewer"));
        assert!(WebAction::parse("unknown.action.v1").is_none());
    }

    #[test]
    fn requests_are_bounded_and_deny_unknown_shapes() {
        for action in [
            WebAction::CompleteOnboarding,
            WebAction::CreateSpace,
            WebAction::CreateFolder,
            WebAction::StartImport,
            WebAction::UpdateAccount,
            WebAction::UpdateOrganization,
            WebAction::UpdateMembers,
            WebAction::UpdateStorage,
            WebAction::CreateDeveloperKey,
            WebAction::UpdateBilling,
            WebAction::AdminAction,
        ] {
            assert!(request(action).validate(action).is_ok(), "{action:?}");
        }
        let mut invalid = request(WebAction::CreateSpace);
        invalid.value = Some("<script>".into());
        assert_eq!(
            invalid.validate(WebAction::CreateSpace),
            Err(BrowserWebFailure::Invalid)
        );
        assert!(serde_json::from_str::<WebActionRequestV1>(
            r#"{"schema_version":"frame.web-action-request.v1","expected_revision":1,"selection_revision":3,"selection_context":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","idempotency_key":"key","value":null,"resource_id":null,"tenant_id":"forbidden"}"#,
        )
        .is_err());
    }

    #[test]
    fn active_organization_selection_context_is_opaque_and_revision_bound() {
        let user_id = "018f47a6-7b1c-7f55-8f39-8f8a86900002";
        let organization_id = "018f47a6-7b1c-7f55-8f39-8f8a86900003";
        let context = selection_context(user_id, organization_id, 3);
        assert!(valid_selection_context(&context));
        assert!(!context.contains(user_id));
        assert!(!context.contains(organization_id));
        assert_ne!(context, selection_context(user_id, organization_id, 4));
    }

    #[test]
    fn workspace_load_revalidates_exact_selection_membership_and_role() {
        let expected = MembershipRow {
            organization_id: "018f47a6-7b1c-7f55-8f39-8f8a86900003".into(),
            role: "owner".into(),
            revision: 7,
            selection_revision: 3,
        };
        assert!(load_authority_is_current(
            &expected,
            Some(&expected),
            "owner"
        ));
        for changed in [
            MembershipRow {
                selection_revision: 4,
                ..expected.clone()
            },
            MembershipRow {
                role: "member".into(),
                revision: 8,
                ..expected.clone()
            },
            MembershipRow {
                organization_id: "018f47a6-7b1c-7f55-8f39-8f8a86900004".into(),
                selection_revision: 4,
                ..expected.clone()
            },
        ] {
            assert!(!load_authority_is_current(
                &expected,
                Some(&changed),
                "owner"
            ));
        }
        assert!(!load_authority_is_current(&expected, None, "owner"));
        assert!(!load_authority_is_current(
            &expected,
            Some(&expected),
            "viewer"
        ));
    }

    #[test]
    fn hash_keyring_is_versioned_bounded_and_redacted_by_construction() {
        let encoded = format!(
            r#"{{"active":{{"version":2,"material_hex":"{}"}},"fallback":[{{"version":1,"material_hex":"{}"}}]}}"#,
            "ab".repeat(32),
            "cd".repeat(32),
        );
        let ring = parse_hash_keyring(&encoded).expect("valid keyring");
        assert_eq!(ring.active_version().get(), 2);
        assert!(parse_hash_keyring("{}").is_err());
        assert!(
            parse_hash_keyring(&format!(
                r#"{{"active":{{"version":1,"material_hex":"{}"}},"fallback":[]}}"#,
                "AA".repeat(32),
            ))
            .is_err()
        );
    }

    #[test]
    fn receipt_and_invalidation_contracts_are_exact() {
        let action = WebAction::CreateFolder;
        let receipt = WebActionReceiptV1 {
            schema_version: WEB_ACTION_RECEIPT_SCHEMA_V1.into(),
            action: action.as_str().into(),
            effect_state: action.effect_state(),
            revision: 8,
            invalidated: action.invalidated(),
        };
        assert_eq!(receipt.validate(action), Ok(()));
        assert_eq!(receipt.invalidated, ["folders", "library"]);
    }
}
