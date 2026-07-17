//! Typed browser-direct client for authenticated product surfaces.
//!
//! All paths are relative, same-origin `/api/v1/web/*` requests. The transport
//! never accepts an upstream URL, bearer credential, or tenant identifier;
//! the browser supplies the host-only session cookie and the Worker derives
//! tenant authority from the authenticated active-organization selection and
//! its current membership.

use std::{cell::RefCell, future::Future, pin::Pin};

use frame_client::{ApiError, RetryAdvice};
use serde::{Deserialize, Serialize};

pub const WORKSPACE_SCHEMA_V1: &str = "frame.web-workspace.v1";
pub const ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-action-request.v1";
pub const ACTION_RECEIPT_SCHEMA_V1: &str = "frame.web-action-receipt.v1";
pub const CSRF_COOKIE_NAME: &str = "__Host-frame_csrf";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub type BrowserFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserSurface {
    Dashboard,
    Library,
    Spaces,
    Space,
    Folders,
    Folder,
    Onboarding,
    Imports,
    Settings,
    AccountSettings,
    OrganizationSettings,
    MemberSettings,
    StorageSettings,
    Developer,
    Billing,
    Analytics,
    Admin,
}

impl BrowserSurface {
    pub const ALL: [Self; 17] = [
        Self::Dashboard,
        Self::Library,
        Self::Spaces,
        Self::Space,
        Self::Folders,
        Self::Folder,
        Self::Onboarding,
        Self::Imports,
        Self::Settings,
        Self::AccountSettings,
        Self::OrganizationSettings,
        Self::MemberSettings,
        Self::StorageSettings,
        Self::Developer,
        Self::Billing,
        Self::Analytics,
        Self::Admin,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Library => "library",
            Self::Spaces => "spaces",
            Self::Space => "space",
            Self::Folders => "folders",
            Self::Folder => "folder",
            Self::Onboarding => "onboarding",
            Self::Imports => "imports",
            Self::Settings => "settings",
            Self::AccountSettings => "account_settings",
            Self::OrganizationSettings => "organization_settings",
            Self::MemberSettings => "member_settings",
            Self::StorageSettings => "storage_settings",
            Self::Developer => "developer",
            Self::Billing => "billing",
            Self::Analytics => "analytics",
            Self::Admin => "admin",
        }
    }

    pub fn from_path(path: &str) -> Option<(Self, Option<String>)> {
        let exact = match path {
            "/dashboard" => Some(Self::Dashboard),
            "/library" => Some(Self::Library),
            "/spaces" => Some(Self::Spaces),
            "/folders" => Some(Self::Folders),
            "/onboarding" => Some(Self::Onboarding),
            "/imports" => Some(Self::Imports),
            "/settings" => Some(Self::Settings),
            "/settings/account" => Some(Self::AccountSettings),
            "/settings/organization" => Some(Self::OrganizationSettings),
            "/settings/members" => Some(Self::MemberSettings),
            "/settings/storage" => Some(Self::StorageSettings),
            "/developer" => Some(Self::Developer),
            "/billing" => Some(Self::Billing),
            "/analytics" => Some(Self::Analytics),
            "/admin" => Some(Self::Admin),
            _ => None,
        };
        if let Some(surface) = exact {
            return Some((surface, None));
        }
        for (prefix, surface) in [("/spaces/", Self::Space), ("/folders/", Self::Folder)] {
            if let Some(resource) = path.strip_prefix(prefix)
                && valid_resource_id(resource)
            {
                return Some((surface, Some(resource.to_owned())));
            }
        }
        None
    }

    #[must_use]
    pub const fn permitted_for(self, role: BrowserRole) -> bool {
        match self {
            Self::Dashboard
            | Self::Library
            | Self::Spaces
            | Self::Space
            | Self::Folders
            | Self::Folder
            | Self::Onboarding
            | Self::Settings
            | Self::AccountSettings => {
                matches!(
                    role,
                    BrowserRole::Owner | BrowserRole::Admin | BrowserRole::Member
                )
            }
            Self::Imports
            | Self::OrganizationSettings
            | Self::MemberSettings
            | Self::StorageSettings
            | Self::Developer
            | Self::Analytics
            | Self::Admin => matches!(role, BrowserRole::Owner | BrowserRole::Admin),
            Self::Billing => matches!(role, BrowserRole::Owner),
        }
    }

    #[must_use]
    pub const fn action(self) -> Option<BrowserAction> {
        match self {
            Self::Spaces => Some(BrowserAction::CreateSpace),
            Self::Folders => Some(BrowserAction::CreateFolder),
            Self::Onboarding => Some(BrowserAction::CompleteOnboarding),
            Self::Imports => Some(BrowserAction::StartImport),
            Self::AccountSettings => Some(BrowserAction::UpdateAccount),
            Self::OrganizationSettings => Some(BrowserAction::UpdateOrganization),
            Self::MemberSettings => Some(BrowserAction::UpdateMembers),
            Self::StorageSettings => Some(BrowserAction::UpdateStorage),
            Self::Developer => Some(BrowserAction::CreateDeveloperKey),
            Self::Billing => Some(BrowserAction::UpdateBilling),
            Self::Admin => Some(BrowserAction::AdminAction),
            Self::Dashboard
            | Self::Library
            | Self::Space
            | Self::Folder
            | Self::Settings
            | Self::Analytics => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrowserRole {
    Owner,
    Admin,
    Member,
}

impl BrowserRole {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheDomain {
    Session,
    Workspace,
    Library,
    Spaces,
    Folders,
    Imports,
    Account,
    Organization,
    Members,
    Storage,
    Developer,
    Billing,
    Analytics,
    Admin,
}

impl CacheDomain {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Workspace => "workspace",
            Self::Library => "library",
            Self::Spaces => "spaces",
            Self::Folders => "folders",
            Self::Imports => "imports",
            Self::Account => "account",
            Self::Organization => "organization",
            Self::Members => "members",
            Self::Storage => "storage",
            Self::Developer => "developer",
            Self::Billing => "billing",
            Self::Analytics => "analytics",
            Self::Admin => "admin",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "session" => Some(Self::Session),
            "workspace" => Some(Self::Workspace),
            "library" => Some(Self::Library),
            "spaces" => Some(Self::Spaces),
            "folders" => Some(Self::Folders),
            "imports" => Some(Self::Imports),
            "account" => Some(Self::Account),
            "organization" => Some(Self::Organization),
            "members" => Some(Self::Members),
            "storage" => Some(Self::Storage),
            "developer" => Some(Self::Developer),
            "billing" => Some(Self::Billing),
            "analytics" => Some(Self::Analytics),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserAction {
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

impl BrowserAction {
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
    pub const fn permitted_for(self, role: BrowserRole) -> bool {
        match self {
            Self::CompleteOnboarding | Self::UpdateAccount => true,
            Self::CreateSpace
            | Self::CreateFolder
            | Self::StartImport
            | Self::UpdateOrganization
            | Self::UpdateMembers
            | Self::UpdateStorage
            | Self::CreateDeveloperKey
            | Self::AdminAction => matches!(role, BrowserRole::Owner | BrowserRole::Admin),
            Self::UpdateBilling => matches!(role, BrowserRole::Owner),
        }
    }

    #[must_use]
    pub const fn invalidates(self) -> &'static [CacheDomain] {
        match self {
            Self::CompleteOnboarding => &[CacheDomain::Session, CacheDomain::Workspace],
            Self::CreateSpace => &[CacheDomain::Spaces, CacheDomain::Workspace],
            Self::CreateFolder => &[CacheDomain::Folders, CacheDomain::Library],
            Self::StartImport => &[CacheDomain::Imports, CacheDomain::Library],
            Self::UpdateAccount => &[CacheDomain::Account, CacheDomain::Session],
            Self::UpdateOrganization => &[CacheDomain::Organization, CacheDomain::Workspace],
            Self::UpdateMembers => &[CacheDomain::Members, CacheDomain::Workspace],
            Self::UpdateStorage => &[CacheDomain::Storage, CacheDomain::Imports],
            Self::CreateDeveloperKey => &[CacheDomain::Developer],
            Self::UpdateBilling => &[CacheDomain::Billing, CacheDomain::Organization],
            Self::AdminAction => &[CacheDomain::Admin, CacheDomain::Workspace],
        }
    }

    #[must_use]
    pub const fn effect_state(self) -> BrowserActionEffectState {
        match self {
            Self::CreateSpace
            | Self::CreateFolder
            | Self::StartImport
            | Self::UpdateAccount
            | Self::UpdateOrganization => BrowserActionEffectState::Applied,
            Self::CompleteOnboarding
            | Self::UpdateMembers
            | Self::UpdateStorage
            | Self::CreateDeveloperKey
            | Self::UpdateBilling
            | Self::AdminAction => BrowserActionEffectState::PendingProtectedExecution,
        }
    }

    #[must_use]
    pub const fn requires_value(self) -> bool {
        matches!(
            self,
            Self::CreateSpace | Self::CreateFolder | Self::UpdateAccount | Self::UpdateOrganization
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserActionEffectState {
    Applied,
    PendingProtectedExecution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserQuery {
    pub search: Option<String>,
    pub filter: String,
    pub page: u16,
    pub resource_id: Option<String>,
}

impl BrowserQuery {
    pub fn new(
        search: Option<String>,
        filter: Option<String>,
        page: Option<u16>,
        resource_id: Option<String>,
    ) -> Result<Self, BrowserClientError> {
        let search = search
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let filter = filter.unwrap_or_else(|| "all".into());
        let page = page.unwrap_or(1);
        if search.as_ref().is_some_and(|value| {
            value.len() > 120 || value.chars().any(char::is_control) || value.contains(['<', '>'])
        }) || !matches!(filter.as_str(), "all" | "ready" | "processing" | "failed")
            || !(1..=1_000).contains(&page)
            || resource_id
                .as_deref()
                .is_some_and(|value| !valid_resource_id(value))
        {
            return Err(BrowserClientError::Invalid);
        }
        Ok(Self {
            search,
            filter,
            page,
            resource_id,
        })
    }

    fn encoded(&self) -> String {
        let mut parts = Vec::with_capacity(4);
        if let Some(search) = &self.search {
            parts.push(format!("q={}", percent_encode(search)));
        }
        if self.filter != "all" {
            parts.push(format!("filter={}", self.filter));
        }
        if self.page != 1 {
            parts.push(format!("page={}", self.page));
        }
        if let Some(resource_id) = &self.resource_id {
            parts.push(format!("resource_id={resource_id}"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("?{}", parts.join("&"))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserWorkspace {
    pub organization_name: String,
    pub member_label: String,
    pub role: BrowserRole,
    pub revision: u64,
    pub selection_revision: u64,
    pub selection_context: String,
    pub recordings: Vec<BrowserRecording>,
    pub spaces: Vec<BrowserResource>,
    pub folders: Vec<BrowserResource>,
    pub import: Option<BrowserImportProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRecording {
    pub id: String,
    pub title: String,
    pub state: String,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserResource {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserImportProgress {
    pub completed: u16,
    pub total: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserMutationInput {
    pub action: BrowserAction,
    pub expected_revision: u64,
    pub selection_revision: u64,
    pub selection_context: String,
    pub idempotency_key: String,
    pub value: Option<String>,
    pub resource_id: Option<String>,
}

impl BrowserMutationInput {
    fn validate(&self) -> Result<(), BrowserClientError> {
        if !valid_idempotency_key(&self.idempotency_key)
            || self.selection_revision > 9_007_199_254_740_991
            || !valid_selection_context(&self.selection_context)
            || self.value.as_ref().is_some_and(|value| {
                value.trim() != value
                    || value.is_empty()
                    || value.len() > 120
                    || value.chars().any(char::is_control)
                    || value.contains(['<', '>'])
            })
            || (self.action.requires_value() && self.value.is_none())
            || self
                .resource_id
                .as_deref()
                .is_some_and(|value| !valid_resource_id(value))
            || (self.action != BrowserAction::CreateFolder && self.resource_id.is_some())
        {
            return Err(BrowserClientError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserMutationReceipt {
    pub revision: u64,
    pub effect_state: BrowserActionEffectState,
    pub invalidated: Vec<CacheDomain>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserClientError {
    Unauthenticated,
    Forbidden,
    NotFound,
    Invalid,
    Conflict,
    RateLimited,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserHttpRequest {
    pub method: BrowserHttpMethod,
    pub path: String,
    pub body: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserHttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserHttpResponse {
    pub status: u16,
    pub body: String,
}

pub trait BrowserAuthenticatedTransport {
    fn send<'a>(
        &'a self,
        request: BrowserHttpRequest,
    ) -> BrowserFuture<'a, Result<BrowserHttpResponse, BrowserClientError>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedWorkspace {
    surface: BrowserSurface,
    query: BrowserQuery,
    workspace: BrowserWorkspace,
}

pub struct BrowserAuthenticatedClient<T> {
    transport: T,
    cache: RefCell<Vec<CachedWorkspace>>,
}

impl<T> BrowserAuthenticatedClient<T>
where
    T: BrowserAuthenticatedTransport,
{
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            cache: RefCell::new(Vec::new()),
        }
    }

    pub async fn load(
        &self,
        surface: BrowserSurface,
        query: &BrowserQuery,
    ) -> Result<BrowserWorkspace, BrowserClientError> {
        if matches!(surface, BrowserSurface::Space | BrowserSurface::Folder)
            != query.resource_id.is_some()
        {
            return Err(BrowserClientError::Invalid);
        }
        if let Some(cached) = self
            .cache
            .borrow()
            .iter()
            .find(|cached| cached.surface == surface && cached.query == *query)
        {
            return Ok(cached.workspace.clone());
        }
        let path = format!(
            "/api/v1/web/workspace/{}{}",
            surface.as_str(),
            query.encoded()
        );
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Get,
                path,
                body: None,
                idempotency_key: None,
            })
            .await?;
        let workspace = decode_workspace(response)?;
        if !surface.permitted_for(workspace.role) {
            return Err(BrowserClientError::NotFound);
        }
        self.cache.borrow_mut().push(CachedWorkspace {
            surface,
            query: query.clone(),
            workspace: workspace.clone(),
        });
        Ok(workspace)
    }

    pub async fn mutate(
        &self,
        input: &BrowserMutationInput,
    ) -> Result<BrowserMutationReceipt, BrowserClientError> {
        input.validate()?;
        let body = serde_json::to_string(&ActionRequestWire {
            schema_version: ACTION_REQUEST_SCHEMA_V1,
            expected_revision: input.expected_revision,
            selection_revision: input.selection_revision,
            selection_context: &input.selection_context,
            idempotency_key: &input.idempotency_key,
            value: input.value.as_deref(),
            resource_id: input.resource_id.as_deref(),
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        // Once a valid mutation leaves this client, its outcome may be
        // uncertain even when transport or receipt decoding fails. Never
        // retain a workspace envelope across that boundary.
        self.cache.borrow_mut().clear();
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!("/api/v1/web/actions/{}", input.action.as_str()),
                body: Some(body),
                idempotency_key: Some(input.idempotency_key.clone()),
            })
            .await?;
        let receipt = decode_receipt(response, input.action)?;
        Ok(receipt)
    }

    #[cfg(test)]
    fn cached_entries(&self) -> usize {
        self.cache.borrow().len()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceWire {
    schema_version: String,
    organization_name: String,
    member_label: String,
    role: String,
    revision: u64,
    selection_revision: u64,
    selection_context: String,
    recordings: Vec<RecordingWire>,
    spaces: Vec<ResourceWire>,
    folders: Vec<ResourceWire>,
    import: Option<ImportWire>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordingWire {
    id: String,
    title: String,
    state: String,
    duration_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceWire {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportWire {
    completed: u16,
    total: u16,
}

#[derive(Debug, Serialize)]
struct ActionRequestWire<'a> {
    schema_version: &'static str,
    expected_revision: u64,
    selection_revision: u64,
    selection_context: &'a str,
    idempotency_key: &'a str,
    value: Option<&'a str>,
    resource_id: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionReceiptWire {
    schema_version: String,
    action: String,
    effect_state: String,
    revision: u64,
    invalidated: Vec<String>,
}

fn decode_workspace(response: BrowserHttpResponse) -> Result<BrowserWorkspace, BrowserClientError> {
    if response.status != 200 {
        return Err(decode_error(response));
    }
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(BrowserClientError::Unavailable);
    }
    let wire = serde_json::from_str::<WorkspaceWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    let role = BrowserRole::parse(&wire.role).ok_or(BrowserClientError::Unavailable)?;
    if wire.schema_version != WORKSPACE_SCHEMA_V1
        || !valid_label(&wire.organization_name, 160)
        || !valid_label(&wire.member_label, 160)
        || wire.selection_revision > 9_007_199_254_740_991
        || !valid_selection_context(&wire.selection_context)
        || wire.recordings.len() > 20
        || wire.spaces.len() > 50
        || wire.folders.len() > 50
        || wire
            .import
            .as_ref()
            .is_some_and(|value| value.total == 0 || value.completed > value.total)
    {
        return Err(BrowserClientError::Unavailable);
    }
    let recordings = wire
        .recordings
        .into_iter()
        .map(|recording| {
            if !valid_resource_id(&recording.id)
                || !valid_label(&recording.title, 512)
                || !matches!(recording.state.as_str(), "ready" | "processing" | "failed")
            {
                return Err(BrowserClientError::Unavailable);
            }
            Ok(BrowserRecording {
                id: recording.id,
                title: recording.title,
                state: recording.state,
                duration_ms: recording.duration_ms,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let decode_resources = |resources: Vec<ResourceWire>| {
        resources
            .into_iter()
            .map(|resource| {
                if !valid_resource_id(&resource.id) || !valid_label(&resource.name, 255) {
                    return Err(BrowserClientError::Unavailable);
                }
                Ok(BrowserResource {
                    id: resource.id,
                    name: resource.name,
                })
            })
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(BrowserWorkspace {
        organization_name: wire.organization_name,
        member_label: wire.member_label,
        role,
        revision: wire.revision,
        selection_revision: wire.selection_revision,
        selection_context: wire.selection_context,
        recordings,
        spaces: decode_resources(wire.spaces)?,
        folders: decode_resources(wire.folders)?,
        import: wire.import.map(|value| BrowserImportProgress {
            completed: value.completed,
            total: value.total,
        }),
    })
}

fn decode_receipt(
    response: BrowserHttpResponse,
    action: BrowserAction,
) -> Result<BrowserMutationReceipt, BrowserClientError> {
    if !matches!(response.status, 200 | 202) {
        return Err(decode_error(response));
    }
    let status = response.status;
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(BrowserClientError::Unavailable);
    }
    let wire = serde_json::from_str::<ActionReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    let invalidated = wire
        .invalidated
        .iter()
        .map(|value| CacheDomain::parse(value).ok_or(BrowserClientError::Unavailable))
        .collect::<Result<Vec<_>, _>>()?;
    let effect_state = match wire.effect_state.as_str() {
        "applied" => BrowserActionEffectState::Applied,
        "pending_protected_execution" => BrowserActionEffectState::PendingProtectedExecution,
        _ => return Err(BrowserClientError::Unavailable),
    };
    if wire.schema_version != ACTION_RECEIPT_SCHEMA_V1
        || wire.action != action.as_str()
        || effect_state != action.effect_state()
        || !matches!(
            (effect_state, status),
            (BrowserActionEffectState::Applied, 200)
                | (BrowserActionEffectState::PendingProtectedExecution, 202)
        )
        || invalidated != action.invalidates()
    {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(BrowserMutationReceipt {
        revision: wire.revision,
        effect_state,
        invalidated,
    })
}

fn decode_error(response: BrowserHttpResponse) -> BrowserClientError {
    if response.body.len() <= MAX_RESPONSE_BYTES
        && let Ok(error) = serde_json::from_str::<ApiError>(&response.body)
        && error.validate().is_ok()
    {
        return match error.code.as_str() {
            "unauthenticated" => BrowserClientError::Unauthenticated,
            "origin_forbidden" | "csrf_rejected" | "forbidden" => BrowserClientError::Forbidden,
            "not_found" => BrowserClientError::NotFound,
            "invalid_body" | "invalid_query" => BrowserClientError::Invalid,
            "conflict" => BrowserClientError::Conflict,
            "rate_limited" => BrowserClientError::RateLimited,
            _ if error.retry == RetryAdvice::Later => BrowserClientError::Unavailable,
            _ => BrowserClientError::Unavailable,
        };
    }
    match response.status {
        401 => BrowserClientError::Unauthenticated,
        403 => BrowserClientError::Forbidden,
        404 => BrowserClientError::NotFound,
        400 | 422 => BrowserClientError::Invalid,
        409 => BrowserClientError::Conflict,
        429 => BrowserClientError::RateLimited,
        _ => BrowserClientError::Unavailable,
    }
}

fn valid_idempotency_key(value: &str) -> bool {
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

fn valid_label(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
        && !value.contains(['<', '>'])
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug, Clone, Copy, Default)]
pub struct WasmSameOriginTransport;

#[cfg(target_arch = "wasm32")]
impl BrowserAuthenticatedTransport for WasmSameOriginTransport {
    fn send<'a>(
        &'a self,
        request: BrowserHttpRequest,
    ) -> BrowserFuture<'a, Result<BrowserHttpResponse, BrowserClientError>> {
        Box::pin(async move { wasm_send(request).await })
    }
}

#[cfg(target_arch = "wasm32")]
async fn wasm_send(request: BrowserHttpRequest) -> Result<BrowserHttpResponse, BrowserClientError> {
    use wasm_bindgen::{JsCast, JsValue};

    if (!request.path.starts_with("/api/v1/web/workspace/")
        && !request.path.starts_with("/api/v1/web/actions/"))
        || request.path.contains(['#', '\\'])
        || request.path.starts_with("//")
    {
        return Err(BrowserClientError::Invalid);
    }
    let init = web_sys::RequestInit::new();
    init.set_method(match request.method {
        BrowserHttpMethod::Get => "GET",
        BrowserHttpMethod::Post => "POST",
    });
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let headers = web_sys::Headers::new().map_err(|_| BrowserClientError::Unavailable)?;
    headers
        .set("accept", "application/json")
        .map_err(|_| BrowserClientError::Unavailable)?;
    if let Some(body) = &request.body {
        init.set_body(&JsValue::from_str(body));
        headers
            .set("content-type", "application/json")
            .map_err(|_| BrowserClientError::Unavailable)?;
    }
    if let Some(idempotency_key) = &request.idempotency_key {
        let csrf = browser_cookie(CSRF_COOKIE_NAME).ok_or(BrowserClientError::Forbidden)?;
        headers
            .set("idempotency-key", idempotency_key)
            .map_err(|_| BrowserClientError::Unavailable)?;
        headers
            .set("x-frame-csrf", &csrf)
            .map_err(|_| BrowserClientError::Unavailable)?;
    }
    init.set_headers(&headers);
    let web_request = web_sys::Request::new_with_str_and_init(&request.path, &init)
        .map_err(|_| BrowserClientError::Unavailable)?;
    let response = wasm_bindgen_futures::JsFuture::from(
        web_sys::window()
            .ok_or(BrowserClientError::Unavailable)?
            .fetch_with_request(&web_request),
    )
    .await
    .map_err(|_| BrowserClientError::Unavailable)?
    .dyn_into::<web_sys::Response>()
    .map_err(|_| BrowserClientError::Unavailable)?;
    let status = response.status();
    let body = wasm_bindgen_futures::JsFuture::from(
        response
            .text()
            .map_err(|_| BrowserClientError::Unavailable)?,
    )
    .await
    .map_err(|_| BrowserClientError::Unavailable)?
    .as_string()
    .ok_or(BrowserClientError::Unavailable)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(BrowserHttpResponse { status, body })
}

#[cfg(target_arch = "wasm32")]
fn browser_cookie(name: &str) -> Option<String> {
    use wasm_bindgen::JsValue;

    let document = web_sys::window()?.document()?;
    let cookies = js_sys::Reflect::get(document.as_ref(), &JsValue::from_str("cookie"))
        .ok()?
        .as_string()?;
    let mut found = None;
    for pair in cookies.split(';') {
        let (candidate, value) = pair.trim().split_once('=')?;
        if candidate != name {
            continue;
        }
        if found.is_some() || value.is_empty() || value.len() > 512 {
            return None;
        }
        found = Some(value.to_owned());
    }
    found
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        role: BrowserRole,
        fail_mutation: bool,
        calls: Rc<RefCell<Vec<BrowserHttpRequest>>>,
    }

    impl FakeTransport {
        fn new(role: BrowserRole) -> Self {
            Self {
                role,
                fail_mutation: false,
                calls: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn with_uncertain_mutation(role: BrowserRole) -> Self {
            Self {
                role,
                fail_mutation: true,
                calls: Rc::new(RefCell::new(Vec::new())),
            }
        }

        fn workspace_json(&self) -> String {
            serde_json::json!({
                "schema_version": WORKSPACE_SCHEMA_V1,
                "organization_name": "Frame workspace",
                "member_label": "Fixture member",
                "role": self.role.as_str(),
                "revision": 7,
                "selection_revision": 3,
                "selection_context": "a".repeat(64),
                "recordings": [{
                    "id": "recording-1",
                    "title": "Quarterly update",
                    "state": "ready",
                    "duration_ms": 42_000,
                }],
                "spaces": [{"id": "space-1", "name": "Product"}],
                "folders": [{"id": "folder-1", "name": "Updates"}],
                "import": {"completed": 1, "total": 2},
            })
            .to_string()
        }
    }

    impl BrowserAuthenticatedTransport for FakeTransport {
        fn send<'a>(
            &'a self,
            request: BrowserHttpRequest,
        ) -> BrowserFuture<'a, Result<BrowserHttpResponse, BrowserClientError>> {
            self.calls.borrow_mut().push(request.clone());
            Box::pin(async move {
                match request.method {
                    BrowserHttpMethod::Get => Ok(BrowserHttpResponse {
                        status: 200,
                        body: self.workspace_json(),
                    }),
                    BrowserHttpMethod::Post => {
                        if self.fail_mutation {
                            return Err(BrowserClientError::Unavailable);
                        }
                        let action = request
                            .path
                            .strip_prefix("/api/v1/web/actions/")
                            .ok_or(BrowserClientError::Invalid)?;
                        let action = BrowserSurface::ALL
                            .iter()
                            .filter_map(|surface| surface.action())
                            .find(|candidate| candidate.as_str() == action)
                            .ok_or(BrowserClientError::Invalid)?;
                        Ok(BrowserHttpResponse {
                            status: match action.effect_state() {
                                BrowserActionEffectState::Applied => 200,
                                BrowserActionEffectState::PendingProtectedExecution => 202,
                            },
                            body: serde_json::json!({
                                "schema_version": ACTION_RECEIPT_SCHEMA_V1,
                                "action": action.as_str(),
                                "effect_state": match action.effect_state() {
                                    BrowserActionEffectState::Applied => "applied",
                                    BrowserActionEffectState::PendingProtectedExecution => "pending_protected_execution",
                                },
                                "revision": 8,
                                "invalidated": action.invalidates().iter().map(|value| value.as_str()).collect::<Vec<_>>(),
                            })
                            .to_string(),
                        })
                    }
                }
            })
        }
    }

    fn query(surface: BrowserSurface) -> BrowserQuery {
        BrowserQuery::new(
            None,
            None,
            None,
            match surface {
                BrowserSurface::Space => Some("space-1".into()),
                BrowserSurface::Folder => Some("folder-1".into()),
                _ => None,
            },
        )
        .expect("query")
    }

    #[tokio::test]
    async fn every_retained_surface_executes_owner_admin_member_or_denied_load() {
        for role in [BrowserRole::Owner, BrowserRole::Admin, BrowserRole::Member] {
            for surface in BrowserSurface::ALL {
                let client = BrowserAuthenticatedClient::new(FakeTransport::new(role));
                let result = client.load(surface, &query(surface)).await;
                assert_eq!(
                    result.is_ok(),
                    surface.permitted_for(role),
                    "{surface:?}/{role:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn every_retained_action_has_exact_role_and_invalidation_contract() {
        for role in [BrowserRole::Owner, BrowserRole::Admin, BrowserRole::Member] {
            for surface in BrowserSurface::ALL {
                let Some(action) = surface.action() else {
                    continue;
                };
                assert!(
                    !action.permitted_for(role) || surface.permitted_for(role),
                    "an action cannot exceed its route grant: {surface:?}/{role:?}",
                );
                if !action.permitted_for(role) {
                    continue;
                }
                let transport = FakeTransport::new(role);
                let client = BrowserAuthenticatedClient::new(transport);
                let input = BrowserMutationInput {
                    action,
                    expected_revision: 7,
                    selection_revision: 3,
                    selection_context: "a".repeat(64),
                    idempotency_key: format!("journey-{}-{}", surface.as_str(), role.as_str()),
                    value: action.requires_value().then(|| "Journey value".into()),
                    resource_id: (action == BrowserAction::CreateFolder).then(|| "space-1".into()),
                };
                let receipt = client.mutate(&input).await.expect("allowed action");
                assert_eq!(receipt.invalidated, action.invalidates());
                assert_eq!(receipt.effect_state, action.effect_state());
            }
        }
    }

    #[tokio::test]
    async fn successful_action_invalidates_every_workspace_envelope() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        client
            .load(BrowserSurface::Spaces, &query(BrowserSurface::Spaces))
            .await
            .expect("spaces");
        client
            .load(BrowserSurface::Billing, &query(BrowserSurface::Billing))
            .await
            .expect("billing");
        assert_eq!(client.cached_entries(), 2);
        client
            .mutate(&BrowserMutationInput {
                action: BrowserAction::CreateSpace,
                expected_revision: 7,
                selection_revision: 3,
                selection_context: "a".repeat(64),
                idempotency_key: "cache-create-space-1".into(),
                value: Some("New space".into()),
                resource_id: None,
            })
            .await
            .expect("mutation");
        assert_eq!(client.cached_entries(), 0);
        client
            .load(BrowserSurface::Billing, &query(BrowserSurface::Billing))
            .await
            .expect("reloaded billing");
        assert_eq!(
            calls.borrow().len(),
            4,
            "every workspace cache must refresh"
        );
    }

    #[tokio::test]
    async fn uncertain_action_outcome_invalidates_every_workspace_envelope() {
        let client = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        client
            .load(BrowserSurface::Spaces, &query(BrowserSurface::Spaces))
            .await
            .expect("spaces");
        client
            .load(BrowserSurface::Billing, &query(BrowserSurface::Billing))
            .await
            .expect("billing");
        assert_eq!(client.cached_entries(), 2);
        let result = client
            .mutate(&BrowserMutationInput {
                action: BrowserAction::CreateSpace,
                expected_revision: 7,
                selection_revision: 3,
                selection_context: "a".repeat(64),
                idempotency_key: "cache-uncertain-space-1".into(),
                value: Some("Uncertain space".into()),
                resource_id: None,
            })
            .await;
        assert_eq!(result, Err(BrowserClientError::Unavailable));
        assert_eq!(client.cached_entries(), 0);
    }

    #[test]
    fn paths_and_queries_are_relative_bounded_and_percent_encoded() {
        assert_eq!(
            BrowserSurface::from_path("/spaces/space-1"),
            Some((BrowserSurface::Space, Some("space-1".into())))
        );
        assert!(BrowserSurface::from_path("//evil.test").is_none());
        assert!(BrowserSurface::from_path("/spaces/../admin").is_none());
        let query =
            BrowserQuery::new(Some("quarterly update".into()), None, Some(2), None).expect("query");
        assert_eq!(query.encoded(), "?q=quarterly%20update&page=2");
    }
}
