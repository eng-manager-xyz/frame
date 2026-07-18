//! Typed browser-direct client for authenticated product surfaces.
//!
//! All paths are relative, same-origin `/api/v1/web/*` requests. The transport
//! never accepts an upstream URL, bearer credential, or tenant identifier;
//! the browser supplies the host-only session cookie and the Worker derives
//! tenant authority from the authenticated active-organization selection and
//! its current membership.

use std::{cell::RefCell, collections::BTreeSet, fmt, future::Future, pin::Pin};

use frame_client::{ApiError, RetryAdvice};
use serde::{Deserialize, Serialize};

pub const WORKSPACE_SCHEMA_V1: &str = "frame.web-workspace.v1";
pub const ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-action-request.v1";
pub const ACTION_RECEIPT_SCHEMA_V1: &str = "frame.web-action-receipt.v1";
pub const WEB_COMPATIBILITY_ACTION_REQUEST_SCHEMA_V1: &str =
    "frame.web-compatibility-action-request.v1";
pub const WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1: &str = "frame.web-folder-assignment-request.v1";
pub const WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1: &str = "frame.web-library-placement-request.v1";
pub const WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1: &str =
    "frame.web-notification-action-request.v1";
pub const WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-developer-action-request.v1";
pub const WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1: &str = "frame.web-membership-action-request.v1";
pub const LEGACY_ACTIVE_ORGANIZATION_ACTION_ID: &str = "cap-v1-a3b4c805d409bc7c";
pub const LEGACY_THEME_ACTION_ID: &str = "cap-v1-7773d3e70d1d5919";
pub const LEGACY_ADD_VIDEOS_TO_FOLDER_ACTION_ID: &str = "cap-v1-f5daa7be337a2979";
pub const LEGACY_REMOVE_VIDEOS_FROM_FOLDER_ACTION_ID: &str = "cap-v1-1af3645bf2ae7168";
pub const LEGACY_MOVE_VIDEO_TO_FOLDER_ACTION_ID: &str = "cap-v1-eaf277e644aa4b92";
pub const LEGACY_ADD_VIDEOS_TO_ORGANIZATION_ACTION_ID: &str = "cap-v1-d96a1931942eb83b";
pub const LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_ACTION_ID: &str = "cap-v1-0694e68a64976c9a";
pub const LEGACY_ADD_VIDEOS_TO_SPACE_ACTION_ID: &str = "cap-v1-bb55b5eeeb5e31ab";
pub const LEGACY_REMOVE_VIDEOS_FROM_SPACE_ACTION_ID: &str = "cap-v1-ccbe5f1381eaa1b4";
pub const LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID: &str = "cap-v1-74a775753d3863c7";
pub const LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID: &str = "cap-v1-1f6a43a05f2f297c";
pub const LEGACY_CREATE_DEVELOPER_APP_ACTION_ID: &str = "cap-v1-f303e703a4237888";
pub const LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID: &str = "cap-v1-87fd6af55b891cb9";
pub const LEGACY_DELETE_DEVELOPER_APP_ACTION_ID: &str = "cap-v1-9833b16bb80a3299";
pub const LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID: &str = "cap-v1-aa86dd3d5351ec06";
pub const LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID: &str = "cap-v1-f7d8036af53d0eb9";
pub const LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID: &str = "cap-v1-1f1465957551f1c4";
pub const LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID: &str = "cap-v1-8328214ed9647abb";
pub const LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID: &str = "cap-v1-b822700b545118f6";
pub const LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID: &str = "cap-v1-866dbe8fbbfd7887";
pub const LEGACY_ADD_SPACE_MEMBER_ACTION_ID: &str = "cap-v1-455046db3d6ef019";
pub const LEGACY_SET_SPACE_MEMBERS_ACTION_ID: &str = "cap-v1-9fc80bdec80fb248";
pub const LEGACY_ADD_SPACE_MEMBERS_ACTION_ID: &str = "cap-v1-b177854e2386c877";
pub const LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID: &str = "cap-v1-38aff8e7221d0260";
pub const LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID: &str = "cap-v1-135614e516c47bf4";
pub const CSRF_COOKIE_NAME: &str = "__Host-frame_csrf";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_FOLDER_ASSIGNMENT_VIDEO_IDS: usize = 500;
const MAX_LIBRARY_PLACEMENT_VIDEO_IDS: usize = 500;
const MAX_DEVELOPER_APP_NAME_CHARS: usize = 255;
const MAX_DEVELOPER_LOGO_URL_CHARS: usize = 1024;
const MAX_DEVELOPER_DOMAIN_CHARS: usize = 253;
const MAX_DEVELOPER_TOP_UP_CENTS: i64 = 100_000;
const MAX_DEVELOPER_THRESHOLD_MICRO_CREDITS: i64 = 9_007_199_254_740_991;
const MAX_MEMBERSHIP_TARGETS: usize = 500;

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
            Self::Dashboard => Some(BrowserAction::SetActiveOrganization),
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
            Self::Library | Self::Space | Self::Folder | Self::Settings | Self::Analytics => None,
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
    Dashboard,
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
            Self::Dashboard => "dashboard",
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
            "dashboard" => Some(Self::Dashboard),
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
    SetActiveOrganization,
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
            Self::SetActiveOrganization => "organization.active-selection.update.v1",
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
            Self::SetActiveOrganization | Self::CompleteOnboarding | Self::UpdateAccount => true,
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
            Self::SetActiveOrganization => &[CacheDomain::Dashboard],
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
            Self::SetActiveOrganization
            | Self::CreateSpace
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
            Self::SetActiveOrganization
                | Self::CreateSpace
                | Self::CreateFolder
                | Self::UpdateAccount
                | Self::UpdateOrganization
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
    pub selection_required: bool,
    pub organizations: Vec<BrowserOrganizationChoice>,
    pub recordings: Vec<BrowserRecording>,
    pub spaces: Vec<BrowserResource>,
    pub folders: Vec<BrowserResource>,
    pub import: Option<BrowserImportProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserOrganizationChoice {
    pub id: String,
    pub name: String,
    pub active: bool,
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
            || (self.action == BrowserAction::SetActiveOrganization
                && !self.value.as_deref().is_some_and(valid_uuid))
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
pub struct BrowserAddVideosToFolderReceiptV1 {
    pub message: String,
    pub added_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRemoveVideosFromFolderReceiptV1 {
    pub message: String,
    pub removed_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserLibraryPlacementScopeV1 {
    Organization,
    Space,
}

impl BrowserLibraryPlacementScopeV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Space => "space",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserAddVideosToOrganizationReceiptV1 {
    pub message: String,
    pub total_updated: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserRemoveVideosFromOrganizationReceiptV1 {
    Removed { message: String, removed_count: u16 },
    NoMatching { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserAddVideosToSpaceReceiptV1 {
    pub message: String,
    pub valid_video_count: u16,
    pub scope: BrowserLibraryPlacementScopeV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserRemoveVideosFromSpaceReceiptV1 {
    pub message: String,
    pub deleted_count: u16,
    pub scope: BrowserLibraryPlacementScopeV1,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BrowserNotificationPreferencesUpdateV1 {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    pause_anon_views: Option<bool>,
}

impl BrowserNotificationPreferencesUpdateV1 {
    #[must_use]
    pub const fn new(
        pause_comments: bool,
        pause_replies: bool,
        pause_views: bool,
        pause_reactions: bool,
        pause_anon_views: Option<bool>,
    ) -> Self {
        Self {
            pause_comments,
            pause_replies,
            pause_views,
            pause_reactions,
            pause_anon_views,
        }
    }
}

impl fmt::Debug for BrowserNotificationPreferencesUpdateV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrowserNotificationPreferencesUpdateV1([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserDeveloperEnvironmentV1 {
    Development,
    Production,
}

impl BrowserDeveloperEnvironmentV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum BrowserDeveloperLogoPatchV1 {
    Missing,
    Null,
    Value(String),
}

impl fmt::Debug for BrowserDeveloperLogoPatchV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Missing => "Missing",
            Self::Null => "Null",
            Self::Value(_) => "Value([redacted])",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BrowserDeveloperAppPatchV1 {
    name: Option<String>,
    environment: Option<BrowserDeveloperEnvironmentV1>,
    logo_url: BrowserDeveloperLogoPatchV1,
}

impl BrowserDeveloperAppPatchV1 {
    #[must_use]
    pub fn new(
        name: Option<String>,
        environment: Option<BrowserDeveloperEnvironmentV1>,
        logo_url: BrowserDeveloperLogoPatchV1,
    ) -> Self {
        Self {
            name,
            environment,
            logo_url,
        }
    }
}

impl fmt::Debug for BrowserDeveloperAppPatchV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserDeveloperAppPatchV1")
            .field("name_present", &self.name.is_some())
            .field("environment", &self.environment)
            .field("logo_url", &self.logo_url)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BrowserDeveloperAutoTopUpPatchV1 {
    enabled: bool,
    threshold_micro_credits: Option<i64>,
    amount_cents: Option<i64>,
}

impl BrowserDeveloperAutoTopUpPatchV1 {
    #[must_use]
    pub const fn new(
        enabled: bool,
        threshold_micro_credits: Option<i64>,
        amount_cents: Option<i64>,
    ) -> Self {
        Self {
            enabled,
            threshold_micro_credits,
            amount_cents,
        }
    }
}

impl fmt::Debug for BrowserDeveloperAutoTopUpPatchV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserDeveloperAutoTopUpPatchV1")
            .field("enabled", &self.enabled)
            .field(
                "threshold_micro_credits_present",
                &self.threshold_micro_credits.is_some(),
            )
            .field("amount_cents_present", &self.amount_cents.is_some())
            .finish()
    }
}

#[derive(PartialEq, Eq)]
pub struct BrowserDeveloperKeyPairV1 {
    public_key: String,
    secret_key: String,
}

impl BrowserDeveloperKeyPairV1 {
    #[must_use]
    pub fn public_key(&self) -> &str {
        &self.public_key
    }

    #[must_use]
    pub fn secret_key(&self) -> &str {
        &self.secret_key
    }
}

impl fmt::Debug for BrowserDeveloperKeyPairV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrowserDeveloperKeyPairV1([redacted])")
    }
}

#[derive(PartialEq, Eq)]
pub struct BrowserCreatedDeveloperAppV1 {
    app_id: String,
    keys: BrowserDeveloperKeyPairV1,
}

impl BrowserCreatedDeveloperAppV1 {
    #[must_use]
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    #[must_use]
    pub const fn keys(&self) -> &BrowserDeveloperKeyPairV1 {
        &self.keys
    }
}

impl fmt::Debug for BrowserCreatedDeveloperAppV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BrowserCreatedDeveloperAppV1([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserLegacySpaceMemberRoleV1 {
    Admin,
    Member,
}

impl BrowserLegacySpaceMemberRoleV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BrowserSubmittedSpaceMemberV1 {
    user_id: String,
    role: BrowserLegacySpaceMemberRoleV1,
}

impl BrowserSubmittedSpaceMemberV1 {
    #[must_use]
    pub fn new(user_id: impl Into<String>, role: BrowserLegacySpaceMemberRoleV1) -> Self {
        Self {
            user_id: user_id.into(),
            role,
        }
    }

    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    #[must_use]
    pub const fn role(&self) -> BrowserLegacySpaceMemberRoleV1 {
        self.role
    }
}

impl fmt::Debug for BrowserSubmittedSpaceMemberV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserSubmittedSpaceMemberV1")
            .field("user", &"<redacted>")
            .field("role", &self.role)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BrowserAddedSpaceMembersV1 {
    added: Vec<String>,
    already_members: Vec<String>,
}

impl BrowserAddedSpaceMembersV1 {
    #[must_use]
    pub fn added(&self) -> &[String] {
        &self.added
    }

    #[must_use]
    pub fn already_members(&self) -> &[String] {
        &self.already_members
    }
}

impl fmt::Debug for BrowserAddedSpaceMembersV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserAddedSpaceMembersV1")
            .field("added_count", &self.added.len())
            .field("already_members_count", &self.already_members.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserThemeV1 {
    Light,
    Dark,
}

impl BrowserThemeV1 {
    #[must_use]
    pub fn parse_cookie_value(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserHttpRequest {
    pub method: BrowserHttpMethod,
    pub path: String,
    pub body: Option<String>,
    pub idempotency_key: Option<String>,
    pub csrf_protected: bool,
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
                csrf_protected: false,
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
                csrf_protected: true,
            })
            .await?;
        let receipt = decode_receipt(response, input.action)?;
        Ok(receipt)
    }

    /// Execute the frozen Cap `setTheme` action through Frame's authenticated
    /// same-origin transport. The abstract ACTION has no client idempotency
    /// key; repeated calls are intentional last-write-wins cookie replacements.
    pub async fn set_theme(&self, theme: BrowserThemeV1) -> Result<(), BrowserClientError> {
        self.compatibility_action(LEGACY_THEME_ACTION_ID, theme.as_str(), false)
            .await
    }

    /// Execute the frozen Cap Navbar active-organization action. This remains
    /// distinct from Frame's UUID/revision-based native organization mutation.
    pub async fn set_legacy_active_organization(
        &self,
        legacy_organization_id: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(legacy_organization_id) {
            return Err(BrowserClientError::Invalid);
        }
        self.compatibility_action(
            LEGACY_ACTIVE_ORGANIZATION_ACTION_ID,
            legacy_organization_id,
            true,
        )
        .await
    }

    /// Execute Cap's frozen `addVideosToFolder` action through the
    /// authenticated same-origin compatibility boundary.
    pub async fn add_videos_to_folder(
        &self,
        folder_id: &str,
        video_ids: &[String],
        scope_id: &str,
        idempotency_key: &str,
    ) -> Result<BrowserAddVideosToFolderReceiptV1, BrowserClientError> {
        let receipt = self
            .folder_videos_action(
                FolderVideosActionV1::Add,
                folder_id,
                video_ids,
                scope_id,
                idempotency_key,
            )
            .await?;
        Ok(BrowserAddVideosToFolderReceiptV1 {
            message: receipt.message,
            added_count: receipt.affected_count,
        })
    }

    /// Execute Cap's frozen `removeVideosFromFolder` action through the
    /// authenticated same-origin compatibility boundary.
    pub async fn remove_videos_from_folder(
        &self,
        folder_id: &str,
        video_ids: &[String],
        scope_id: &str,
        idempotency_key: &str,
    ) -> Result<BrowserRemoveVideosFromFolderReceiptV1, BrowserClientError> {
        let receipt = self
            .folder_videos_action(
                FolderVideosActionV1::Remove,
                folder_id,
                video_ids,
                scope_id,
                idempotency_key,
            )
            .await?;
        Ok(BrowserRemoveVideosFromFolderReceiptV1 {
            message: receipt.message,
            removed_count: receipt.affected_count,
        })
    }

    /// Execute Cap's frozen `moveVideoToFolder` action. `folder_id` is a
    /// required-nullable field: `None` is serialized as JSON `null` to request
    /// a root move, never omitted. `scope_id` remains an optional Cap scope.
    pub async fn move_video_to_folder(
        &self,
        video_id: &str,
        folder_id: Option<&str>,
        scope_id: Option<&str>,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(video_id)
            || folder_id.is_some_and(|value| !valid_cap_nanoid(value))
            || scope_id.is_some_and(|value| !valid_cap_nanoid(value))
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&MoveVideoToFolderRequestWire {
            schema_version: WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1,
            video_id,
            folder_id,
            scope_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        // A valid idempotent mutation can commit even when transport or
        // decoding fails, so no cached workspace crosses the send boundary.
        self.cache.borrow_mut().clear();
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!(
                    "/api/v1/web/compatibility-actions/{LEGACY_MOVE_VIDEO_TO_FOLDER_ACTION_ID}"
                ),
                body: Some(body),
                idempotency_key: Some(idempotency_key.to_owned()),
                csrf_protected: true,
            })
            .await?;
        if response.status == 204 && response.body.is_empty() {
            Ok(())
        } else if response.status == 204 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        }
    }

    async fn folder_videos_action(
        &self,
        action: FolderVideosActionV1,
        folder_id: &str,
        video_ids: &[String],
        scope_id: &str,
        idempotency_key: &str,
    ) -> Result<FolderVideosReceiptV1, BrowserClientError> {
        let expected_count =
            validate_folder_videos_input(folder_id, video_ids, scope_id, idempotency_key)?;
        let body = serde_json::to_string(&FolderVideosRequestWire {
            schema_version: WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1,
            folder_id,
            video_ids,
            scope_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        // Clear before sending, including for uncertain transport outcomes.
        self.cache.borrow_mut().clear();
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!(
                    "/api/v1/web/compatibility-actions/{}",
                    action.operation_id()
                ),
                body: Some(body),
                idempotency_key: Some(idempotency_key.to_owned()),
                csrf_protected: true,
            })
            .await?;
        decode_folder_videos_receipt(response, action, expected_count)
    }

    /// Execute Cap's frozen `addVideosToOrganization` action. The source
    /// success object has no numeric field, so the typed receipt validates and
    /// projects the exact count-bound message.
    pub async fn add_videos_to_organization(
        &self,
        organization_id: &str,
        video_ids: &[String],
        idempotency_key: &str,
    ) -> Result<BrowserAddVideosToOrganizationReceiptV1, BrowserClientError> {
        match self
            .library_placement_action(
                LibraryPlacementActionV1::AddToOrganization,
                organization_id,
                video_ids,
                BrowserLibraryPlacementScopeV1::Organization,
                idempotency_key,
            )
            .await?
        {
            LibraryPlacementReceiptV1::OrganizationAdded {
                message,
                total_updated,
            } => Ok(BrowserAddVideosToOrganizationReceiptV1 {
                message,
                total_updated,
            }),
            _ => Err(BrowserClientError::Unavailable),
        }
    }

    /// Execute Cap's frozen `removeVideosFromOrganization` action. A source
    /// success can either report a partial removal or the distinct no-match
    /// result; neither shape includes a numeric JSON field.
    pub async fn remove_videos_from_organization(
        &self,
        organization_id: &str,
        video_ids: &[String],
        idempotency_key: &str,
    ) -> Result<BrowserRemoveVideosFromOrganizationReceiptV1, BrowserClientError> {
        match self
            .library_placement_action(
                LibraryPlacementActionV1::RemoveFromOrganization,
                organization_id,
                video_ids,
                BrowserLibraryPlacementScopeV1::Organization,
                idempotency_key,
            )
            .await?
        {
            LibraryPlacementReceiptV1::OrganizationRemoved {
                message,
                removed_count,
            } => Ok(BrowserRemoveVideosFromOrganizationReceiptV1::Removed {
                message,
                removed_count,
            }),
            LibraryPlacementReceiptV1::OrganizationNoMatching { message } => {
                Ok(BrowserRemoveVideosFromOrganizationReceiptV1::NoMatching { message })
            }
            _ => Err(BrowserClientError::Unavailable),
        }
    }

    /// Execute Cap's frozen `addVideosToSpace` action. The source accepts
    /// either the active organization pseudo-space or a real space; `scope`
    /// binds the exact expected success label and is never sent as authority.
    pub async fn add_videos_to_space(
        &self,
        scope_id: &str,
        scope: BrowserLibraryPlacementScopeV1,
        video_ids: &[String],
        idempotency_key: &str,
    ) -> Result<BrowserAddVideosToSpaceReceiptV1, BrowserClientError> {
        match self
            .library_placement_action(
                LibraryPlacementActionV1::AddToSpace,
                scope_id,
                video_ids,
                scope,
                idempotency_key,
            )
            .await?
        {
            LibraryPlacementReceiptV1::ScopeAdded {
                message,
                valid_video_count,
            } => Ok(BrowserAddVideosToSpaceReceiptV1 {
                message,
                valid_video_count,
                scope,
            }),
            _ => Err(BrowserClientError::Unavailable),
        }
    }

    /// Execute Cap's frozen `removeVideosFromSpace` action. Unlike the other
    /// three actions, this source success includes an explicit `deletedCount`
    /// field, which must equal the canonical submitted video count.
    pub async fn remove_videos_from_space(
        &self,
        scope_id: &str,
        scope: BrowserLibraryPlacementScopeV1,
        video_ids: &[String],
        idempotency_key: &str,
    ) -> Result<BrowserRemoveVideosFromSpaceReceiptV1, BrowserClientError> {
        match self
            .library_placement_action(
                LibraryPlacementActionV1::RemoveFromSpace,
                scope_id,
                video_ids,
                scope,
                idempotency_key,
            )
            .await?
        {
            LibraryPlacementReceiptV1::ScopeRemoved {
                message,
                deleted_count,
            } => Ok(BrowserRemoveVideosFromSpaceReceiptV1 {
                message,
                deleted_count,
                scope,
            }),
            _ => Err(BrowserClientError::Unavailable),
        }
    }

    async fn library_placement_action(
        &self,
        action: LibraryPlacementActionV1,
        scope_id: &str,
        video_ids: &[String],
        expected_scope: BrowserLibraryPlacementScopeV1,
        idempotency_key: &str,
    ) -> Result<LibraryPlacementReceiptV1, BrowserClientError> {
        let expected_count =
            validate_library_placement_input(scope_id, video_ids, idempotency_key)?;
        let body = match action {
            LibraryPlacementActionV1::AddToOrganization
            | LibraryPlacementActionV1::RemoveFromOrganization => {
                serde_json::to_string(&OrganizationLibraryPlacementRequestWire {
                    schema_version: WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1,
                    organization_id: scope_id,
                    video_ids,
                    idempotency_key,
                })
            }
            LibraryPlacementActionV1::AddToSpace | LibraryPlacementActionV1::RemoveFromSpace => {
                serde_json::to_string(&ScopeLibraryPlacementRequestWire {
                    schema_version: WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1,
                    scope_id,
                    video_ids,
                    idempotency_key,
                })
            }
        }
        .map_err(|_| BrowserClientError::Unavailable)?;
        // Any valid send can commit before transport or receipt decoding
        // fails. No cached workspace may cross this uncertainty boundary.
        self.cache.borrow_mut().clear();
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!(
                    "/api/v1/web/compatibility-actions/{}",
                    action.operation_id()
                ),
                body: Some(body),
                idempotency_key: Some(idempotency_key.to_owned()),
                csrf_protected: true,
            })
            .await?;
        decode_library_placement_receipt(response, action, expected_scope, expected_count)
    }

    /// Execute Cap's optional-selector `markAsRead` action. `None` preserves
    /// the source bulk form, while a present selector must be a Cap NanoID.
    pub async fn mark_notifications_read(
        &self,
        notification_id: Option<&str>,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if notification_id.is_some_and(|value| !valid_cap_nanoid(value))
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&MarkNotificationsReadRequestWire {
            schema_version: WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1,
            notification_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        self.notification_action(
            LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID,
            body,
            idempotency_key,
        )
        .await
    }

    /// Execute Cap's `updatePreferences` action while preserving the source
    /// distinction between an absent `pauseAnonViews` property and `false`.
    pub async fn update_notification_preferences(
        &self,
        preferences: BrowserNotificationPreferencesUpdateV1,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_compatibility_idempotency_key(idempotency_key) {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&UpdateNotificationPreferencesRequestWire {
            schema_version: WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1,
            notifications: NotificationPreferencesWire {
                pause_comments: preferences.pause_comments,
                pause_replies: preferences.pause_replies,
                pause_views: preferences.pause_views,
                pause_reactions: preferences.pause_reactions,
                pause_anon_views: preferences.pause_anon_views,
            },
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        self.notification_action(
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID,
            body,
            idempotency_key,
        )
        .await
    }

    async fn notification_action(
        &self,
        operation_id: &'static str,
        body: String,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        // Both source actions invalidate dashboard data. Clear every cached
        // workspace before the send because a transport failure can hide an
        // already-committed mutation.
        self.cache.borrow_mut().clear();
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!("/api/v1/web/compatibility-actions/{operation_id}"),
                body: Some(body),
                idempotency_key: Some(idempotency_key.to_owned()),
                csrf_protected: true,
            })
            .await?;
        if response.status == 204 && response.body.is_empty() {
            Ok(())
        } else if response.status == 204 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        }
    }

    /// Create a user-owned developer application and receive its one-time
    /// plaintext key pair. The returned credentials are deliberately redacted
    /// from every `Debug` implementation.
    pub async fn create_developer_app(
        &self,
        name: &str,
        environment: BrowserDeveloperEnvironmentV1,
        idempotency_key: &str,
    ) -> Result<BrowserCreatedDeveloperAppV1, BrowserClientError> {
        if !valid_developer_name(name) || !valid_compatibility_idempotency_key(idempotency_key) {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&CreateDeveloperAppRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            name,
            environment: environment.as_str(),
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .developer_action(DeveloperActionV1::CreateApp, body, idempotency_key)
            .await?;
        decode_created_developer_app(response)
    }

    /// Patch only fields that are present. `logo_url` preserves the legacy
    /// three-way distinction between a missing property, explicit null, and a
    /// concrete value; an entirely empty patch remains a successful no-op.
    pub async fn update_developer_app(
        &self,
        app_id: &str,
        patch: BrowserDeveloperAppPatchV1,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(app_id)
            || patch
                .name
                .as_deref()
                .is_some_and(|value| !valid_developer_name(value))
            || matches!(
                &patch.logo_url,
                BrowserDeveloperLogoPatchV1::Value(value)
                    if value.chars().count() > MAX_DEVELOPER_LOGO_URL_CHARS
            )
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let logo_url = match &patch.logo_url {
            BrowserDeveloperLogoPatchV1::Missing => None,
            BrowserDeveloperLogoPatchV1::Null => Some(None),
            BrowserDeveloperLogoPatchV1::Value(value) => Some(Some(value.as_str())),
        };
        let body = serde_json::to_string(&UpdateDeveloperAppRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            app_id,
            name: patch.name.as_deref(),
            environment: patch.environment.map(BrowserDeveloperEnvironmentV1::as_str),
            logo_url,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .developer_action(DeveloperActionV1::UpdateApp, body, idempotency_key)
            .await?;
        decode_developer_success(response)
    }

    pub async fn delete_developer_app(
        &self,
        app_id: &str,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        self.developer_app_only_action(DeveloperActionV1::DeleteApp, app_id, idempotency_key)
            .await
    }

    pub async fn add_developer_domain(
        &self,
        app_id: &str,
        domain: &str,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(app_id)
            || !valid_developer_origin(domain)
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&AddDeveloperDomainRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            app_id,
            domain,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .developer_action(DeveloperActionV1::AddDomain, body, idempotency_key)
            .await?;
        decode_developer_success(response)
    }

    pub async fn remove_developer_domain(
        &self,
        app_id: &str,
        domain_id: &str,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(app_id)
            || !valid_cap_nanoid(domain_id)
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&RemoveDeveloperDomainRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            app_id,
            domain_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .developer_action(DeveloperActionV1::RemoveDomain, body, idempotency_key)
            .await?;
        decode_developer_success(response)
    }

    pub async fn regenerate_developer_keys(
        &self,
        app_id: &str,
        idempotency_key: &str,
    ) -> Result<BrowserDeveloperKeyPairV1, BrowserClientError> {
        if !valid_cap_nanoid(app_id) || !valid_compatibility_idempotency_key(idempotency_key) {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&DeveloperAppOnlyRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            app_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .developer_action(DeveloperActionV1::RegenerateKeys, body, idempotency_key)
            .await?;
        decode_developer_key_pair(response)
    }

    pub async fn delete_developer_video(
        &self,
        app_id: &str,
        video_id: &str,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(app_id)
            || !valid_cap_nanoid(video_id)
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&DeleteDeveloperVideoRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            app_id,
            video_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .developer_action(DeveloperActionV1::DeleteVideo, body, idempotency_key)
            .await?;
        decode_developer_success(response)
    }

    pub async fn update_developer_auto_top_up(
        &self,
        app_id: &str,
        patch: BrowserDeveloperAutoTopUpPatchV1,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(app_id)
            || patch
                .threshold_micro_credits
                .is_some_and(|value| !(0..=MAX_DEVELOPER_THRESHOLD_MICRO_CREDITS).contains(&value))
            || patch
                .amount_cents
                .is_some_and(|value| !(1..=MAX_DEVELOPER_TOP_UP_CENTS).contains(&value))
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&UpdateDeveloperAutoTopUpRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            app_id,
            enabled: patch.enabled,
            threshold_micro_credits: patch.threshold_micro_credits,
            amount_cents: patch.amount_cents,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .developer_action(DeveloperActionV1::UpdateAutoTopUp, body, idempotency_key)
            .await?;
        decode_developer_success(response)
    }

    async fn developer_app_only_action(
        &self,
        action: DeveloperActionV1,
        app_id: &str,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(app_id) || !valid_compatibility_idempotency_key(idempotency_key) {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&DeveloperAppOnlyRequestWire {
            schema_version: WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1,
            app_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self.developer_action(action, body, idempotency_key).await?;
        decode_developer_success(response)
    }

    async fn developer_action(
        &self,
        action: DeveloperActionV1,
        body: String,
        idempotency_key: &str,
    ) -> Result<BrowserHttpResponse, BrowserClientError> {
        // The mutation may commit before transport or response decoding fails.
        // Developer credentials and projections therefore never share a cache
        // epoch with a sent action.
        self.cache.borrow_mut().clear();
        self.transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!(
                    "/api/v1/web/compatibility-actions/{}",
                    action.operation_id()
                ),
                body: Some(body),
                idempotency_key: Some(idempotency_key.to_owned()),
                csrf_protected: true,
            })
            .await
    }

    pub async fn remove_organization_invite(
        &self,
        invite_id: &str,
        organization_id: &str,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(invite_id)
            || !valid_cap_nanoid(organization_id)
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&RemoveOrganizationInviteRequestWire {
            schema_version: WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            invite_id,
            organization_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .membership_action(
                MembershipActionV1::RemoveOrganizationInvite,
                body,
                idempotency_key,
            )
            .await?;
        decode_membership_success(response)
    }

    pub async fn add_space_member(
        &self,
        space_id: &str,
        user_id: &str,
        role: BrowserLegacySpaceMemberRoleV1,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(space_id)
            || !valid_cap_nanoid(user_id)
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&AddSpaceMemberRequestWire {
            schema_version: WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            space_id,
            user_id,
            role,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .membership_action(MembershipActionV1::AddSpaceMember, body, idempotency_key)
            .await?;
        decode_membership_success(response)
    }

    pub async fn add_space_members(
        &self,
        space_id: &str,
        user_ids: &[String],
        role: BrowserLegacySpaceMemberRoleV1,
        idempotency_key: &str,
    ) -> Result<BrowserAddedSpaceMembersV1, BrowserClientError> {
        if !valid_cap_nanoid(space_id)
            || user_ids.len() > MAX_MEMBERSHIP_TARGETS
            || user_ids.iter().any(|value| !valid_cap_nanoid(value))
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&AddSpaceMembersRequestWire {
            schema_version: WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            space_id,
            user_ids,
            role,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .membership_action(MembershipActionV1::AddSpaceMembers, body, idempotency_key)
            .await?;
        decode_add_space_members(response, user_ids)
    }

    pub async fn batch_remove_space_members(
        &self,
        member_ids: &[String],
        idempotency_key: &str,
    ) -> Result<Vec<String>, BrowserClientError> {
        if member_ids.len() > MAX_MEMBERSHIP_TARGETS
            || member_ids.iter().any(|value| !valid_cap_nanoid(value))
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&BatchRemoveSpaceMembersRequestWire {
            schema_version: WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            member_ids,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .membership_action(
                MembershipActionV1::BatchRemoveSpaceMembers,
                body,
                idempotency_key,
            )
            .await?;
        decode_removed_space_members(response, member_ids)
    }

    pub async fn remove_space_member(
        &self,
        member_id: &str,
        idempotency_key: &str,
    ) -> Result<(), BrowserClientError> {
        if !valid_cap_nanoid(member_id) || !valid_compatibility_idempotency_key(idempotency_key) {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&RemoveSpaceMemberRequestWire {
            schema_version: WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            member_id,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .membership_action(MembershipActionV1::RemoveSpaceMember, body, idempotency_key)
            .await?;
        decode_membership_success(response)
    }

    /// Replace the selected space's member set. `role` and `members` preserve
    /// source presence: missing `role` defaults to member; present `members`
    /// wins over `user_ids`, including when it is an empty array.
    pub async fn set_space_members(
        &self,
        space_id: &str,
        user_ids: &[String],
        role: Option<BrowserLegacySpaceMemberRoleV1>,
        members: Option<&[BrowserSubmittedSpaceMemberV1]>,
        idempotency_key: &str,
    ) -> Result<u32, BrowserClientError> {
        if !valid_cap_nanoid(space_id)
            || user_ids.len() > MAX_MEMBERSHIP_TARGETS
            || user_ids.iter().any(|value| !valid_cap_nanoid(value))
            || members.is_some_and(|values| {
                values.len() > MAX_MEMBERSHIP_TARGETS
                    || values
                        .iter()
                        .any(|member| !valid_cap_nanoid(member.user_id()))
            })
            || !valid_compatibility_idempotency_key(idempotency_key)
        {
            return Err(BrowserClientError::Invalid);
        }
        let body = serde_json::to_string(&SetSpaceMembersRequestWire {
            schema_version: WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
            space_id,
            user_ids,
            role,
            members,
            idempotency_key,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        let response = self
            .membership_action(MembershipActionV1::SetSpaceMembers, body, idempotency_key)
            .await?;
        decode_set_space_members(response)
    }

    async fn membership_action(
        &self,
        action: MembershipActionV1,
        body: String,
        idempotency_key: &str,
    ) -> Result<BrowserHttpResponse, BrowserClientError> {
        // Membership mutations invalidate authority and workspace projections.
        // Clear before sending because a committed response can be lost.
        self.cache.borrow_mut().clear();
        self.transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!(
                    "/api/v1/web/compatibility-actions/{}",
                    action.operation_id()
                ),
                body: Some(body),
                idempotency_key: Some(idempotency_key.to_owned()),
                csrf_protected: true,
            })
            .await
    }

    async fn compatibility_action(
        &self,
        operation_id: &str,
        value: &str,
        invalidate_workspace_cache: bool,
    ) -> Result<(), BrowserClientError> {
        let body = serde_json::to_string(&CompatibilityActionRequestWire {
            schema_version: WEB_COMPATIBILITY_ACTION_REQUEST_SCHEMA_V1,
            value,
        })
        .map_err(|_| BrowserClientError::Unavailable)?;
        if invalidate_workspace_cache {
            // A transport failure can hide a committed last-write-wins
            // selection, so no workspace envelope survives the send boundary.
            self.cache.borrow_mut().clear();
        }
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: format!("/api/v1/web/compatibility-actions/{operation_id}"),
                body: Some(body),
                idempotency_key: None,
                csrf_protected: true,
            })
            .await?;
        if response.status == 204 && response.body.is_empty() {
            Ok(())
        } else if response.status == 204 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        }
    }

    /// Revoke the current host-only browser session through the same
    /// double-submit CSRF boundary as every authenticated mutation.
    pub async fn logout(&self) -> Result<(), BrowserClientError> {
        self.cache.borrow_mut().clear();
        let response = self
            .transport
            .send(BrowserHttpRequest {
                method: BrowserHttpMethod::Post,
                path: "/api/v1/web/auth/logout".into(),
                body: None,
                idempotency_key: None,
                csrf_protected: true,
            })
            .await?;
        if (200..400).contains(&response.status) {
            Ok(())
        } else {
            Err(decode_error(response))
        }
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
    selection_required: bool,
    organizations: Vec<OrganizationChoiceWire>,
    recordings: Vec<RecordingWire>,
    spaces: Vec<ResourceWire>,
    folders: Vec<ResourceWire>,
    import: Option<ImportWire>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrganizationChoiceWire {
    id: String,
    name: String,
    active: bool,
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

#[derive(Debug, Serialize)]
struct CompatibilityActionRequestWire<'a> {
    schema_version: &'static str,
    value: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderVideosActionV1 {
    Add,
    Remove,
}

impl FolderVideosActionV1 {
    const fn operation_id(self) -> &'static str {
        match self {
            Self::Add => LEGACY_ADD_VIDEOS_TO_FOLDER_ACTION_ID,
            Self::Remove => LEGACY_REMOVE_VIDEOS_FROM_FOLDER_ACTION_ID,
        }
    }

    const fn past_tense(self) -> &'static str {
        match self {
            Self::Add => "added",
            Self::Remove => "removed",
        }
    }

    const fn preposition(self) -> &'static str {
        match self {
            Self::Add => "to",
            Self::Remove => "from",
        }
    }
}

#[derive(Debug, Serialize)]
struct FolderVideosRequestWire<'a> {
    schema_version: &'static str,
    folder_id: &'a str,
    video_ids: &'a [String],
    scope_id: &'a str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct MoveVideoToFolderRequestWire<'a> {
    schema_version: &'static str,
    video_id: &'a str,
    folder_id: Option<&'a str>,
    scope_id: Option<&'a str>,
    idempotency_key: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AddVideosToFolderReceiptWire {
    success: bool,
    message: String,
    added_count: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoveVideosFromFolderReceiptWire {
    success: bool,
    message: String,
    removed_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FolderVideosReceiptV1 {
    message: String,
    affected_count: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LibraryPlacementActionV1 {
    AddToOrganization,
    RemoveFromOrganization,
    AddToSpace,
    RemoveFromSpace,
}

impl LibraryPlacementActionV1 {
    const fn operation_id(self) -> &'static str {
        match self {
            Self::AddToOrganization => LEGACY_ADD_VIDEOS_TO_ORGANIZATION_ACTION_ID,
            Self::RemoveFromOrganization => LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_ACTION_ID,
            Self::AddToSpace => LEGACY_ADD_VIDEOS_TO_SPACE_ACTION_ID,
            Self::RemoveFromSpace => LEGACY_REMOVE_VIDEOS_FROM_SPACE_ACTION_ID,
        }
    }
}

#[derive(Debug, Serialize)]
struct OrganizationLibraryPlacementRequestWire<'a> {
    schema_version: &'static str,
    organization_id: &'a str,
    video_ids: &'a [String],
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct ScopeLibraryPlacementRequestWire<'a> {
    schema_version: &'static str,
    scope_id: &'a str,
    video_ids: &'a [String],
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct MarkNotificationsReadRequestWire<'a> {
    schema_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification_id: Option<&'a str>,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct UpdateNotificationPreferencesRequestWire<'a> {
    schema_version: &'static str,
    notifications: NotificationPreferencesWire,
    idempotency_key: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationPreferencesWire {
    pause_comments: bool,
    pause_replies: bool,
    pause_views: bool,
    pause_reactions: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pause_anon_views: Option<bool>,
}

impl fmt::Debug for NotificationPreferencesWire {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NotificationPreferencesWire([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeveloperActionV1 {
    CreateApp,
    UpdateApp,
    DeleteApp,
    AddDomain,
    RemoveDomain,
    RegenerateKeys,
    DeleteVideo,
    UpdateAutoTopUp,
}

impl DeveloperActionV1 {
    const fn operation_id(self) -> &'static str {
        match self {
            Self::CreateApp => LEGACY_CREATE_DEVELOPER_APP_ACTION_ID,
            Self::UpdateApp => LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID,
            Self::DeleteApp => LEGACY_DELETE_DEVELOPER_APP_ACTION_ID,
            Self::AddDomain => LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID,
            Self::RemoveDomain => LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID,
            Self::RegenerateKeys => LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID,
            Self::DeleteVideo => LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID,
            Self::UpdateAutoTopUp => LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID,
        }
    }
}

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
    const fn operation_id(self) -> &'static str {
        match self {
            Self::RemoveOrganizationInvite => LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID,
            Self::AddSpaceMember => LEGACY_ADD_SPACE_MEMBER_ACTION_ID,
            Self::SetSpaceMembers => LEGACY_SET_SPACE_MEMBERS_ACTION_ID,
            Self::AddSpaceMembers => LEGACY_ADD_SPACE_MEMBERS_ACTION_ID,
            Self::BatchRemoveSpaceMembers => LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID,
            Self::RemoveSpaceMember => LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID,
        }
    }
}

#[derive(Debug, Serialize)]
struct CreateDeveloperAppRequestWire<'a> {
    schema_version: &'static str,
    name: &'a str,
    environment: &'static str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct UpdateDeveloperAppRequestWire<'a> {
    schema_version: &'static str,
    app_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logo_url: Option<Option<&'a str>>,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct DeveloperAppOnlyRequestWire<'a> {
    schema_version: &'static str,
    app_id: &'a str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct AddDeveloperDomainRequestWire<'a> {
    schema_version: &'static str,
    app_id: &'a str,
    domain: &'a str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct RemoveDeveloperDomainRequestWire<'a> {
    schema_version: &'static str,
    app_id: &'a str,
    domain_id: &'a str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct DeleteDeveloperVideoRequestWire<'a> {
    schema_version: &'static str,
    app_id: &'a str,
    video_id: &'a str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct UpdateDeveloperAutoTopUpRequestWire<'a> {
    schema_version: &'static str,
    app_id: &'a str,
    enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    threshold_micro_credits: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    amount_cents: Option<i64>,
    idempotency_key: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreatedDeveloperAppReceiptWire {
    app_id: String,
    public_key: String,
    secret_key: String,
}

impl fmt::Debug for CreatedDeveloperAppReceiptWire {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CreatedDeveloperAppReceiptWire([redacted])")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DeveloperKeyPairReceiptWire {
    public_key: String,
    secret_key: String,
}

impl fmt::Debug for DeveloperKeyPairReceiptWire {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DeveloperKeyPairReceiptWire([redacted])")
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeveloperSuccessReceiptWire {
    success: bool,
}

#[derive(Debug, Serialize)]
struct RemoveOrganizationInviteRequestWire<'a> {
    schema_version: &'static str,
    invite_id: &'a str,
    organization_id: &'a str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct AddSpaceMemberRequestWire<'a> {
    schema_version: &'static str,
    space_id: &'a str,
    user_id: &'a str,
    role: BrowserLegacySpaceMemberRoleV1,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct AddSpaceMembersRequestWire<'a> {
    schema_version: &'static str,
    space_id: &'a str,
    user_ids: &'a [String],
    role: BrowserLegacySpaceMemberRoleV1,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct BatchRemoveSpaceMembersRequestWire<'a> {
    schema_version: &'static str,
    member_ids: &'a [String],
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct RemoveSpaceMemberRequestWire<'a> {
    schema_version: &'static str,
    member_id: &'a str,
    idempotency_key: &'a str,
}

#[derive(Debug, Serialize)]
struct SetSpaceMembersRequestWire<'a> {
    schema_version: &'static str,
    space_id: &'a str,
    user_ids: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<BrowserLegacySpaceMemberRoleV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    members: Option<&'a [BrowserSubmittedSpaceMemberV1]>,
    idempotency_key: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MembershipSuccessReceiptWire {
    success: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetSpaceMembersReceiptWire {
    success: bool,
    count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AddSpaceMembersReceiptWire {
    success: bool,
    added: Vec<String>,
    already_members: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemovedSpaceMembersReceiptWire {
    success: bool,
    removed: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LibraryPlacementMessageReceiptWire {
    success: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RemoveVideosFromSpaceReceiptWire {
    success: bool,
    message: String,
    deleted_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LibraryPlacementReceiptV1 {
    OrganizationAdded {
        message: String,
        total_updated: u16,
    },
    OrganizationRemoved {
        message: String,
        removed_count: u16,
    },
    OrganizationNoMatching {
        message: String,
    },
    ScopeAdded {
        message: String,
        valid_video_count: u16,
    },
    ScopeRemoved {
        message: String,
        deleted_count: u16,
    },
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
        || wire.organizations.is_empty()
        || wire.organizations.len() > 50
        || wire
            .organizations
            .iter()
            .any(|value| !valid_uuid(&value.id) || !valid_label(&value.name, 160))
        || if wire.selection_required {
            wire.organizations.iter().any(|value| value.active)
                || !wire.recordings.is_empty()
                || !wire.spaces.is_empty()
                || !wire.folders.is_empty()
                || wire.import.is_some()
        } else {
            wire.organizations
                .iter()
                .filter(|value| value.active)
                .count()
                != 1
        }
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
    let organizations = wire
        .organizations
        .into_iter()
        .map(|choice| BrowserOrganizationChoice {
            id: choice.id,
            name: choice.name,
            active: choice.active,
        })
        .collect();
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
        selection_required: wire.selection_required,
        organizations,
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

fn validate_folder_videos_input(
    folder_id: &str,
    video_ids: &[String],
    scope_id: &str,
    idempotency_key: &str,
) -> Result<u16, BrowserClientError> {
    if !valid_cap_nanoid(folder_id)
        || !valid_cap_nanoid(scope_id)
        || video_ids.is_empty()
        || video_ids.len() > MAX_FOLDER_ASSIGNMENT_VIDEO_IDS
        || video_ids.iter().any(|value| !valid_cap_nanoid(value))
        || !valid_compatibility_idempotency_key(idempotency_key)
    {
        return Err(BrowserClientError::Invalid);
    }
    let unique_count = video_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .len();
    u16::try_from(unique_count).map_err(|_| BrowserClientError::Invalid)
}

fn decode_folder_videos_receipt(
    response: BrowserHttpResponse,
    action: FolderVideosActionV1,
    expected_count: u16,
) -> Result<FolderVideosReceiptV1, BrowserClientError> {
    if response.status != 200 {
        return Err(decode_error(response));
    }
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(BrowserClientError::Unavailable);
    }
    let (success, message, affected_count) = match action {
        FolderVideosActionV1::Add => {
            let wire = serde_json::from_str::<AddVideosToFolderReceiptWire>(&response.body)
                .map_err(|_| BrowserClientError::Unavailable)?;
            (wire.success, wire.message, wire.added_count)
        }
        FolderVideosActionV1::Remove => {
            let wire = serde_json::from_str::<RemoveVideosFromFolderReceiptWire>(&response.body)
                .map_err(|_| BrowserClientError::Unavailable)?;
            (wire.success, wire.message, wire.removed_count)
        }
    };
    let expected_message = format!(
        "{expected_count} video{} {} {} folder",
        if expected_count == 1 { "" } else { "s" },
        action.past_tense(),
        action.preposition(),
    );
    if !success || affected_count != expected_count || message != expected_message {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(FolderVideosReceiptV1 {
        message,
        affected_count,
    })
}

fn validate_library_placement_input(
    scope_id: &str,
    video_ids: &[String],
    idempotency_key: &str,
) -> Result<u16, BrowserClientError> {
    if !valid_cap_nanoid(scope_id)
        || video_ids.is_empty()
        || video_ids.len() > MAX_LIBRARY_PLACEMENT_VIDEO_IDS
        || video_ids.iter().any(|value| !valid_cap_nanoid(value))
        || !valid_compatibility_idempotency_key(idempotency_key)
    {
        return Err(BrowserClientError::Invalid);
    }
    let unique_count = video_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .len();
    u16::try_from(unique_count).map_err(|_| BrowserClientError::Invalid)
}

fn decode_library_placement_receipt(
    response: BrowserHttpResponse,
    action: LibraryPlacementActionV1,
    expected_scope: BrowserLibraryPlacementScopeV1,
    expected_count: u16,
) -> Result<LibraryPlacementReceiptV1, BrowserClientError> {
    if response.status != 200 {
        return Err(decode_error(response));
    }
    if response.body.len() > MAX_RESPONSE_BYTES {
        return Err(BrowserClientError::Unavailable);
    }
    match action {
        LibraryPlacementActionV1::AddToOrganization => {
            if expected_scope != BrowserLibraryPlacementScopeV1::Organization {
                return Err(BrowserClientError::Unavailable);
            }
            let wire = serde_json::from_str::<LibraryPlacementMessageReceiptWire>(&response.body)
                .map_err(|_| BrowserClientError::Unavailable)?;
            let expected_message = format!(
                "{expected_count} video{} {} now in organization root",
                if expected_count == 1 { "" } else { "s" },
                if expected_count == 1 { "is" } else { "are" },
            );
            if !wire.success || wire.message != expected_message {
                return Err(BrowserClientError::Unavailable);
            }
            Ok(LibraryPlacementReceiptV1::OrganizationAdded {
                message: wire.message,
                total_updated: expected_count,
            })
        }
        LibraryPlacementActionV1::RemoveFromOrganization => {
            if expected_scope != BrowserLibraryPlacementScopeV1::Organization {
                return Err(BrowserClientError::Unavailable);
            }
            let wire = serde_json::from_str::<LibraryPlacementMessageReceiptWire>(&response.body)
                .map_err(|_| BrowserClientError::Unavailable)?;
            if !wire.success {
                return Err(BrowserClientError::Unavailable);
            }
            if wire.message == "No matching shared videos found in organization" {
                return Ok(LibraryPlacementReceiptV1::OrganizationNoMatching {
                    message: wire.message,
                });
            }
            let removed_count = (1..=expected_count)
                .find(|count| {
                    wire.message
                        == format!(
                            "{count} video{} removed from organization",
                            if *count == 1 { "" } else { "s" },
                        )
                })
                .ok_or(BrowserClientError::Unavailable)?;
            Ok(LibraryPlacementReceiptV1::OrganizationRemoved {
                message: wire.message,
                removed_count,
            })
        }
        LibraryPlacementActionV1::AddToSpace => {
            let wire = serde_json::from_str::<LibraryPlacementMessageReceiptWire>(&response.body)
                .map_err(|_| BrowserClientError::Unavailable)?;
            let expected_message = format!(
                "{expected_count} video{} added to {}",
                if expected_count == 1 { "" } else { "s" },
                expected_scope.as_str(),
            );
            if !wire.success || wire.message != expected_message {
                return Err(BrowserClientError::Unavailable);
            }
            Ok(LibraryPlacementReceiptV1::ScopeAdded {
                message: wire.message,
                valid_video_count: expected_count,
            })
        }
        LibraryPlacementActionV1::RemoveFromSpace => {
            let wire = serde_json::from_str::<RemoveVideosFromSpaceReceiptWire>(&response.body)
                .map_err(|_| BrowserClientError::Unavailable)?;
            let expected_message = format!(
                "Removed {expected_count} video(s) from {} and folders",
                expected_scope.as_str(),
            );
            if !wire.success
                || wire.deleted_count != expected_count
                || wire.message != expected_message
            {
                return Err(BrowserClientError::Unavailable);
            }
            Ok(LibraryPlacementReceiptV1::ScopeRemoved {
                message: wire.message,
                deleted_count: wire.deleted_count,
            })
        }
    }
}

fn decode_created_developer_app(
    response: BrowserHttpResponse,
) -> Result<BrowserCreatedDeveloperAppV1, BrowserClientError> {
    if response.status != 200 || response.body.len() > MAX_RESPONSE_BYTES {
        return if response.status == 200 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        };
    }
    let wire = serde_json::from_str::<CreatedDeveloperAppReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    if !valid_cap_nanoid(&wire.app_id)
        || !valid_developer_key("cpk_", &wire.public_key)
        || !valid_developer_key("csk_", &wire.secret_key)
        || wire.public_key == wire.secret_key
    {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(BrowserCreatedDeveloperAppV1 {
        app_id: wire.app_id,
        keys: BrowserDeveloperKeyPairV1 {
            public_key: wire.public_key,
            secret_key: wire.secret_key,
        },
    })
}

fn decode_developer_key_pair(
    response: BrowserHttpResponse,
) -> Result<BrowserDeveloperKeyPairV1, BrowserClientError> {
    if response.status != 200 || response.body.len() > MAX_RESPONSE_BYTES {
        return if response.status == 200 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        };
    }
    let wire = serde_json::from_str::<DeveloperKeyPairReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    if !valid_developer_key("cpk_", &wire.public_key)
        || !valid_developer_key("csk_", &wire.secret_key)
        || wire.public_key == wire.secret_key
    {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(BrowserDeveloperKeyPairV1 {
        public_key: wire.public_key,
        secret_key: wire.secret_key,
    })
}

fn decode_developer_success(response: BrowserHttpResponse) -> Result<(), BrowserClientError> {
    if response.status != 200 || response.body.len() > MAX_RESPONSE_BYTES {
        return if response.status == 200 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        };
    }
    let wire = serde_json::from_str::<DeveloperSuccessReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    if wire.success {
        Ok(())
    } else {
        Err(BrowserClientError::Unavailable)
    }
}

fn decode_membership_success(response: BrowserHttpResponse) -> Result<(), BrowserClientError> {
    if response.status != 200 || response.body.len() > MAX_RESPONSE_BYTES {
        return if response.status == 200 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        };
    }
    let wire = serde_json::from_str::<MembershipSuccessReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    if wire.success {
        Ok(())
    } else {
        Err(BrowserClientError::Unavailable)
    }
}

fn decode_add_space_members(
    response: BrowserHttpResponse,
    submitted_user_ids: &[String],
) -> Result<BrowserAddedSpaceMembersV1, BrowserClientError> {
    if response.status != 200 || response.body.len() > MAX_RESPONSE_BYTES {
        return if response.status == 200 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        };
    }
    let wire = serde_json::from_str::<AddSpaceMembersReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    let existing = wire.already_members.iter().collect::<BTreeSet<_>>();
    let expected_added = submitted_user_ids
        .iter()
        .filter(|value| !existing.contains(value))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !wire.success
        || wire.added.len() > MAX_MEMBERSHIP_TARGETS
        || wire.added.iter().any(|value| !valid_cap_nanoid(value))
        || wire
            .already_members
            .iter()
            .any(|value| !valid_cap_nanoid(value))
        || (wire.added.is_empty() && wire.already_members != submitted_user_ids)
        || (!wire.added.is_empty()
            && (existing.len() != wire.already_members.len()
                || wire.added.iter().map(String::as_str).ne(expected_added)))
    {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(BrowserAddedSpaceMembersV1 {
        added: wire.added,
        already_members: wire.already_members,
    })
}

fn decode_removed_space_members(
    response: BrowserHttpResponse,
    submitted_member_ids: &[String],
) -> Result<Vec<String>, BrowserClientError> {
    if response.status != 200 || response.body.len() > MAX_RESPONSE_BYTES {
        return if response.status == 200 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        };
    }
    let wire = serde_json::from_str::<RemovedSpaceMembersReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    if !wire.success
        || wire.removed.len() > MAX_MEMBERSHIP_TARGETS
        || wire.removed.iter().any(|value| !valid_cap_nanoid(value))
        || (!wire.removed.is_empty() && wire.removed != submitted_member_ids)
    {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(wire.removed)
}

fn decode_set_space_members(response: BrowserHttpResponse) -> Result<u32, BrowserClientError> {
    if response.status != 200 || response.body.len() > MAX_RESPONSE_BYTES {
        return if response.status == 200 {
            Err(BrowserClientError::Unavailable)
        } else {
            Err(decode_error(response))
        };
    }
    let wire = serde_json::from_str::<SetSpaceMembersReceiptWire>(&response.body)
        .map_err(|_| BrowserClientError::Unavailable)?;
    if !wire.success || wire.count == 0 || wire.count as usize > MAX_MEMBERSHIP_TARGETS + 1 {
        return Err(BrowserClientError::Unavailable);
    }
    Ok(wire.count)
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
            "invalid_body" | "invalid_query" | "invalid_compatibility_action" => {
                BrowserClientError::Invalid
            }
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

fn valid_compatibility_idempotency_key(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
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
    uuid::Uuid::parse_str(value)
        .is_ok_and(|uuid| !uuid.is_nil() && uuid.as_hyphenated().to_string() == value)
}

fn valid_cap_nanoid(value: &str) -> bool {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    value.len() == 15 && value.bytes().all(|byte| ALPHABET.contains(&byte))
}

fn valid_developer_name(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= MAX_DEVELOPER_APP_NAME_CHARS
}

fn valid_developer_origin(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized.chars().count() > MAX_DEVELOPER_DOMAIN_CHARS {
        return false;
    }
    let Some(authority) = normalized
        .strip_prefix("https://")
        .or_else(|| normalized.strip_prefix("http://"))
    else {
        return false;
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    !host.is_empty()
        && host.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        && port.is_none_or(|value| {
            !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn valid_developer_key(prefix: &str, value: &str) -> bool {
    const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
    value.len() == prefix.len() + 30
        && value.starts_with(prefix)
        && value[prefix.len()..]
            .bytes()
            .all(|byte| ALPHABET.contains(&byte))
}

#[cfg(any(target_arch = "wasm32", test))]
fn valid_compatibility_action_path(path: &str) -> bool {
    matches!(
        path.strip_prefix("/api/v1/web/compatibility-actions/"),
        Some(
            LEGACY_ACTIVE_ORGANIZATION_ACTION_ID
                | LEGACY_THEME_ACTION_ID
                | LEGACY_ADD_VIDEOS_TO_FOLDER_ACTION_ID
                | LEGACY_REMOVE_VIDEOS_FROM_FOLDER_ACTION_ID
                | LEGACY_MOVE_VIDEO_TO_FOLDER_ACTION_ID
                | LEGACY_ADD_VIDEOS_TO_ORGANIZATION_ACTION_ID
                | LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_ACTION_ID
                | LEGACY_ADD_VIDEOS_TO_SPACE_ACTION_ID
                | LEGACY_REMOVE_VIDEOS_FROM_SPACE_ACTION_ID
                | LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID
                | LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID
                | LEGACY_CREATE_DEVELOPER_APP_ACTION_ID
                | LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID
                | LEGACY_DELETE_DEVELOPER_APP_ACTION_ID
                | LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID
                | LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID
                | LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID
                | LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID
                | LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID
                | LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID
                | LEGACY_ADD_SPACE_MEMBER_ACTION_ID
                | LEGACY_SET_SPACE_MEMBERS_ACTION_ID
                | LEGACY_ADD_SPACE_MEMBERS_ACTION_ID
                | LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID
                | LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID
        )
    )
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
        && !request.path.starts_with("/api/v1/web/actions/")
        && !valid_compatibility_action_path(&request.path)
        && request.path != "/api/v1/web/auth/logout")
        || request.path.contains(['#', '\\'])
        || request.path.starts_with("//")
        || request.idempotency_key.is_some() && !request.csrf_protected
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
    if request.csrf_protected {
        let csrf = browser_cookie(CSRF_COOKIE_NAME).ok_or(BrowserClientError::Forbidden)?;
        headers
            .set("x-frame-csrf", &csrf)
            .map_err(|_| BrowserClientError::Unavailable)?;
    }
    if let Some(idempotency_key) = &request.idempotency_key {
        headers
            .set("idempotency-key", idempotency_key)
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

#[cfg(target_arch = "wasm32")]
pub(crate) fn browser_theme_cookie() -> Option<BrowserThemeV1> {
    browser_cookie("theme").and_then(|value| BrowserThemeV1::parse_cookie_value(&value))
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
                "selection_required": false,
                "organizations": [{
                    "id": "018f47a6-7b1c-7f55-8f39-8f8a86900003",
                    "name": "Frame workspace",
                    "active": true,
                }],
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

        fn folder_body(
            request: &BrowserHttpRequest,
        ) -> Result<serde_json::Value, BrowserClientError> {
            let body = serde_json::from_str::<serde_json::Value>(
                request.body.as_deref().ok_or(BrowserClientError::Invalid)?,
            )
            .map_err(|_| BrowserClientError::Invalid)?;
            if body
                .get("schema_version")
                .and_then(serde_json::Value::as_str)
                != Some(WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1)
                || body
                    .get("idempotency_key")
                    .and_then(serde_json::Value::as_str)
                    != request.idempotency_key.as_deref()
            {
                return Err(BrowserClientError::Invalid);
            }
            Ok(body)
        }

        fn folder_video_count(request: &BrowserHttpRequest) -> Result<u16, BrowserClientError> {
            let body = Self::folder_body(request)?;
            let videos = body
                .get("video_ids")
                .and_then(serde_json::Value::as_array)
                .ok_or(BrowserClientError::Invalid)?;
            let count = videos
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<BTreeSet<_>>()
                .len();
            u16::try_from(count).map_err(|_| BrowserClientError::Invalid)
        }

        fn library_body(
            request: &BrowserHttpRequest,
        ) -> Result<serde_json::Value, BrowserClientError> {
            let body = serde_json::from_str::<serde_json::Value>(
                request.body.as_deref().ok_or(BrowserClientError::Invalid)?,
            )
            .map_err(|_| BrowserClientError::Invalid)?;
            if body
                .get("schema_version")
                .and_then(serde_json::Value::as_str)
                != Some(WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1)
                || body
                    .get("idempotency_key")
                    .and_then(serde_json::Value::as_str)
                    != request.idempotency_key.as_deref()
            {
                return Err(BrowserClientError::Invalid);
            }
            Ok(body)
        }

        fn library_video_count(request: &BrowserHttpRequest) -> Result<u16, BrowserClientError> {
            let body = Self::library_body(request)?;
            let videos = body
                .get("video_ids")
                .and_then(serde_json::Value::as_array)
                .ok_or(BrowserClientError::Invalid)?;
            let count = videos
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<BTreeSet<_>>()
                .len();
            u16::try_from(count).map_err(|_| BrowserClientError::Invalid)
        }

        fn notification_body(
            request: &BrowserHttpRequest,
        ) -> Result<serde_json::Value, BrowserClientError> {
            let body = serde_json::from_str::<serde_json::Value>(
                request.body.as_deref().ok_or(BrowserClientError::Invalid)?,
            )
            .map_err(|_| BrowserClientError::Invalid)?;
            if body
                .get("schema_version")
                .and_then(serde_json::Value::as_str)
                != Some(WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1)
                || body
                    .get("idempotency_key")
                    .and_then(serde_json::Value::as_str)
                    != request.idempotency_key.as_deref()
            {
                return Err(BrowserClientError::Invalid);
            }
            Ok(body)
        }

        fn developer_body(
            request: &BrowserHttpRequest,
        ) -> Result<serde_json::Value, BrowserClientError> {
            let body = serde_json::from_str::<serde_json::Value>(
                request.body.as_deref().ok_or(BrowserClientError::Invalid)?,
            )
            .map_err(|_| BrowserClientError::Invalid)?;
            if body
                .get("schema_version")
                .and_then(serde_json::Value::as_str)
                != Some(WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1)
                || body
                    .get("idempotency_key")
                    .and_then(serde_json::Value::as_str)
                    != request.idempotency_key.as_deref()
            {
                return Err(BrowserClientError::Invalid);
            }
            Ok(body)
        }

        fn membership_body(
            request: &BrowserHttpRequest,
        ) -> Result<serde_json::Value, BrowserClientError> {
            let body = serde_json::from_str::<serde_json::Value>(
                request.body.as_deref().ok_or(BrowserClientError::Invalid)?,
            )
            .map_err(|_| BrowserClientError::Invalid)?;
            if body
                .get("schema_version")
                .and_then(serde_json::Value::as_str)
                != Some(WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1)
                || body
                    .get("idempotency_key")
                    .and_then(serde_json::Value::as_str)
                    != request.idempotency_key.as_deref()
            {
                return Err(BrowserClientError::Invalid);
            }
            Ok(body)
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
                        if request.path == "/api/v1/web/auth/logout" {
                            return Ok(BrowserHttpResponse {
                                status: 200,
                                body: String::new(),
                            });
                        }
                        if let Some(operation_id) = request
                            .path
                            .strip_prefix("/api/v1/web/compatibility-actions/")
                        {
                            return match operation_id {
                                LEGACY_ADD_VIDEOS_TO_FOLDER_ACTION_ID => {
                                    let count = Self::folder_video_count(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "message": format!(
                                                "{count} video{} added to folder",
                                                if count == 1 { "" } else { "s" },
                                            ),
                                            "addedCount": count,
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_REMOVE_VIDEOS_FROM_FOLDER_ACTION_ID => {
                                    let count = Self::folder_video_count(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "message": format!(
                                                "{count} video{} removed from folder",
                                                if count == 1 { "" } else { "s" },
                                            ),
                                            "removedCount": count,
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_MOVE_VIDEO_TO_FOLDER_ACTION_ID => {
                                    let _ = Self::folder_body(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 204,
                                        body: String::new(),
                                    })
                                }
                                LEGACY_ADD_VIDEOS_TO_ORGANIZATION_ACTION_ID => {
                                    let count = Self::library_video_count(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "message": format!(
                                                "{count} video{} {} now in organization root",
                                                if count == 1 { "" } else { "s" },
                                                if count == 1 { "is" } else { "are" },
                                            ),
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_ACTION_ID => {
                                    let count = Self::library_video_count(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "message": format!(
                                                "{count} video{} removed from organization",
                                                if count == 1 { "" } else { "s" },
                                            ),
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_ADD_VIDEOS_TO_SPACE_ACTION_ID => {
                                    let count = Self::library_video_count(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "message": format!(
                                                "{count} video{} added to space",
                                                if count == 1 { "" } else { "s" },
                                            ),
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_REMOVE_VIDEOS_FROM_SPACE_ACTION_ID => {
                                    let count = Self::library_video_count(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "message": format!(
                                                "Removed {count} video(s) from space and folders"
                                            ),
                                            "deletedCount": count,
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID
                                | LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID => {
                                    let _ = Self::notification_body(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 204,
                                        body: String::new(),
                                    })
                                }
                                LEGACY_CREATE_DEVELOPER_APP_ACTION_ID => {
                                    let _ = Self::developer_body(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "appId": "0123456789abcde",
                                            "publicKey": format!("cpk_{}", "0".repeat(30)),
                                            "secretKey": format!("csk_{}", "1".repeat(30)),
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID => {
                                    let _ = Self::developer_body(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "publicKey": format!("cpk_{}", "2".repeat(30)),
                                            "secretKey": format!("csk_{}", "3".repeat(30)),
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID
                                | LEGACY_DELETE_DEVELOPER_APP_ACTION_ID
                                | LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID
                                | LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID
                                | LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID
                                | LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID => {
                                    let _ = Self::developer_body(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({"success": true}).to_string(),
                                    })
                                }
                                LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID
                                | LEGACY_ADD_SPACE_MEMBER_ACTION_ID
                                | LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID => {
                                    let _ = Self::membership_body(&request)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({"success": true}).to_string(),
                                    })
                                }
                                LEGACY_ADD_SPACE_MEMBERS_ACTION_ID => {
                                    let body = Self::membership_body(&request)?;
                                    let user_ids = body
                                        .get("user_ids")
                                        .and_then(serde_json::Value::as_array)
                                        .ok_or(BrowserClientError::Invalid)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "added": user_ids,
                                            "alreadyMembers": [],
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID => {
                                    let body = Self::membership_body(&request)?;
                                    let member_ids = body
                                        .get("member_ids")
                                        .and_then(serde_json::Value::as_array)
                                        .ok_or(BrowserClientError::Invalid)?;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "removed": member_ids,
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_SET_SPACE_MEMBERS_ACTION_ID => {
                                    let body = Self::membership_body(&request)?;
                                    let selected = body
                                        .get("members")
                                        .or_else(|| body.get("user_ids"))
                                        .and_then(serde_json::Value::as_array)
                                        .ok_or(BrowserClientError::Invalid)?;
                                    let count = selected
                                        .iter()
                                        .filter_map(|value| {
                                            value
                                                .get("userId")
                                                .or_else(|| value.get("user_id"))
                                                .and_then(serde_json::Value::as_str)
                                                .or_else(|| value.as_str())
                                        })
                                        .collect::<BTreeSet<_>>()
                                        .len()
                                        + 1;
                                    Ok(BrowserHttpResponse {
                                        status: 200,
                                        body: serde_json::json!({
                                            "success": true,
                                            "count": count,
                                        })
                                        .to_string(),
                                    })
                                }
                                LEGACY_ACTIVE_ORGANIZATION_ACTION_ID | LEGACY_THEME_ACTION_ID => {
                                    Ok(BrowserHttpResponse {
                                        status: 204,
                                        body: String::new(),
                                    })
                                }
                                _ => Err(BrowserClientError::Invalid),
                            };
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
                    value: action.requires_value().then(|| {
                        if action == BrowserAction::SetActiveOrganization {
                            "018f47a6-7b1c-7f55-8f39-8f8a86900004".into()
                        } else {
                            "Journey value".into()
                        }
                    }),
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

    #[tokio::test]
    async fn logout_uses_the_exact_csrf_protected_worker_route_and_clears_cache() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        client
            .load(BrowserSurface::Dashboard, &query(BrowserSurface::Dashboard))
            .await
            .expect("dashboard");
        assert_eq!(client.cached_entries(), 1);

        client.logout().await.expect("logout");

        assert_eq!(client.cached_entries(), 0);
        let calls = calls.borrow();
        let request = calls.last().expect("logout request");
        assert_eq!(request.method, BrowserHttpMethod::Post);
        assert_eq!(request.path, "/api/v1/web/auth/logout");
        assert!(request.body.is_none());
        assert!(request.idempotency_key.is_none());
        assert!(request.csrf_protected);
    }

    #[tokio::test]
    async fn compatibility_actions_use_exact_csrf_transport_without_idempotency() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);

        client
            .set_theme(BrowserThemeV1::Dark)
            .await
            .expect("theme action");
        client
            .set_legacy_active_organization("0123456789abcde")
            .await
            .expect("legacy selection action");

        let calls = calls.borrow();
        let theme = &calls[0];
        assert_eq!(theme.method, BrowserHttpMethod::Post);
        assert_eq!(
            theme.path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_THEME_ACTION_ID}")
        );
        assert_eq!(theme.idempotency_key, None);
        assert!(theme.csrf_protected);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(theme.body.as_deref().expect("body"))
                .expect("theme JSON"),
            serde_json::json!({
                "schema_version": WEB_COMPATIBILITY_ACTION_REQUEST_SCHEMA_V1,
                "value": "dark",
            })
        );

        let selection = &calls[1];
        assert_eq!(
            selection.path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_ACTIVE_ORGANIZATION_ACTION_ID}")
        );
        assert_eq!(selection.idempotency_key, None);
        assert!(selection.csrf_protected);
        assert!(
            selection
                .body
                .as_deref()
                .is_some_and(|body| body.contains("0123456789abcde"))
        );
    }

    #[tokio::test]
    async fn folder_assignment_actions_use_exact_typed_idempotent_transport() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let videos = vec!["1123456789abcde".into(), "2123456789abcde".into()];

        let added = client
            .add_videos_to_folder(
                "0123456789abcde",
                &videos,
                "3123456789abcde",
                "folder-add-1",
            )
            .await
            .expect("add videos");
        assert_eq!(
            added,
            BrowserAddVideosToFolderReceiptV1 {
                message: "2 videos added to folder".into(),
                added_count: 2,
            }
        );
        let removed = client
            .remove_videos_from_folder(
                "0123456789abcde",
                &videos,
                "3123456789abcde",
                "folder-remove-1",
            )
            .await
            .expect("remove videos");
        assert_eq!(
            removed,
            BrowserRemoveVideosFromFolderReceiptV1 {
                message: "2 videos removed from folder".into(),
                removed_count: 2,
            }
        );
        client
            .move_video_to_folder(
                "1123456789abcde",
                Some("4123456789abcde"),
                Some("3123456789abcde"),
                "folder-move-1",
            )
            .await
            .expect("move video");
        client
            .move_video_to_folder("1123456789abcde", None, None, "folder-root-1")
            .await
            .expect("move video to root");

        let calls = calls.borrow();
        assert_eq!(calls.len(), 4);
        for request in calls.iter() {
            assert_eq!(request.method, BrowserHttpMethod::Post);
            assert!(request.csrf_protected);
            let body = serde_json::from_str::<serde_json::Value>(
                request.body.as_deref().expect("folder body"),
            )
            .expect("folder JSON");
            assert_eq!(
                body.get("schema_version")
                    .and_then(serde_json::Value::as_str),
                Some(WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1)
            );
            assert_eq!(
                body.get("idempotency_key")
                    .and_then(serde_json::Value::as_str),
                request.idempotency_key.as_deref(),
                "header and body idempotency keys must be identical",
            );
        }
        assert_eq!(
            calls[0].path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_ADD_VIDEOS_TO_FOLDER_ACTION_ID}")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(calls[0].body.as_deref().expect("add body"))
                .expect("add JSON"),
            serde_json::json!({
                "schema_version": WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1,
                "folder_id": "0123456789abcde",
                "video_ids": ["1123456789abcde", "2123456789abcde"],
                "scope_id": "3123456789abcde",
                "idempotency_key": "folder-add-1",
            })
        );
        assert_eq!(
            calls[1].path,
            format!(
                "/api/v1/web/compatibility-actions/{LEGACY_REMOVE_VIDEOS_FROM_FOLDER_ACTION_ID}"
            )
        );
        assert_eq!(
            calls[2].path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_MOVE_VIDEO_TO_FOLDER_ACTION_ID}")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[3].body.as_deref().expect("root move body")
            )
            .expect("root move JSON"),
            serde_json::json!({
                "schema_version": WEB_FOLDER_ASSIGNMENT_REQUEST_SCHEMA_V1,
                "video_id": "1123456789abcde",
                "folder_id": null,
                "scope_id": null,
                "idempotency_key": "folder-root-1",
            }),
            "the required nullable root folder field must be present as null",
        );
    }

    #[tokio::test]
    async fn library_placement_actions_use_exact_typed_idempotent_transport() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let videos = vec!["1123456789abcde".into(), "2123456789abcde".into()];

        assert_eq!(
            client
                .add_videos_to_organization("0123456789abcde", &videos, "library-org-add-1",)
                .await
                .expect("add organization"),
            BrowserAddVideosToOrganizationReceiptV1 {
                message: "2 videos are now in organization root".into(),
                total_updated: 2,
            }
        );
        assert_eq!(
            client
                .remove_videos_from_organization(
                    "0123456789abcde",
                    &videos,
                    "library-org-remove-1",
                )
                .await
                .expect("remove organization"),
            BrowserRemoveVideosFromOrganizationReceiptV1::Removed {
                message: "2 videos removed from organization".into(),
                removed_count: 2,
            }
        );
        assert_eq!(
            client
                .add_videos_to_space(
                    "3123456789abcde",
                    BrowserLibraryPlacementScopeV1::Space,
                    &videos,
                    "library-space-add-1",
                )
                .await
                .expect("add space"),
            BrowserAddVideosToSpaceReceiptV1 {
                message: "2 videos added to space".into(),
                valid_video_count: 2,
                scope: BrowserLibraryPlacementScopeV1::Space,
            }
        );
        assert_eq!(
            client
                .remove_videos_from_space(
                    "3123456789abcde",
                    BrowserLibraryPlacementScopeV1::Space,
                    &videos,
                    "library-space-remove-1",
                )
                .await
                .expect("remove space"),
            BrowserRemoveVideosFromSpaceReceiptV1 {
                message: "Removed 2 video(s) from space and folders".into(),
                deleted_count: 2,
                scope: BrowserLibraryPlacementScopeV1::Space,
            }
        );

        let calls = calls.borrow();
        assert_eq!(calls.len(), 4);
        for request in calls.iter() {
            assert_eq!(request.method, BrowserHttpMethod::Post);
            assert!(request.csrf_protected);
            let body = serde_json::from_str::<serde_json::Value>(
                request.body.as_deref().expect("library body"),
            )
            .expect("library JSON");
            assert_eq!(
                body.get("schema_version")
                    .and_then(serde_json::Value::as_str),
                Some(WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1)
            );
            assert_eq!(
                body.get("idempotency_key")
                    .and_then(serde_json::Value::as_str),
                request.idempotency_key.as_deref(),
                "header and body idempotency keys must be identical",
            );
        }
        assert_eq!(
            calls[0].path,
            format!(
                "/api/v1/web/compatibility-actions/{LEGACY_ADD_VIDEOS_TO_ORGANIZATION_ACTION_ID}"
            )
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[0].body.as_deref().expect("organization add body")
            )
            .expect("organization add JSON"),
            serde_json::json!({
                "schema_version": WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1,
                "organization_id": "0123456789abcde",
                "video_ids": ["1123456789abcde", "2123456789abcde"],
                "idempotency_key": "library-org-add-1",
            })
        );
        assert_eq!(
            calls[1].path,
            format!(
                "/api/v1/web/compatibility-actions/{LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_ACTION_ID}"
            )
        );
        assert_eq!(
            calls[2].path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_ADD_VIDEOS_TO_SPACE_ACTION_ID}")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[2].body.as_deref().expect("space add body")
            )
            .expect("space add JSON"),
            serde_json::json!({
                "schema_version": WEB_LIBRARY_PLACEMENT_REQUEST_SCHEMA_V1,
                "scope_id": "3123456789abcde",
                "video_ids": ["1123456789abcde", "2123456789abcde"],
                "idempotency_key": "library-space-add-1",
            })
        );
        assert_eq!(
            calls[3].path,
            format!(
                "/api/v1/web/compatibility-actions/{LEGACY_REMOVE_VIDEOS_FROM_SPACE_ACTION_ID}"
            )
        );
    }

    #[tokio::test]
    async fn library_placement_decodes_no_match_partial_and_both_scope_labels() {
        let videos = vec!["1123456789abcde".into(), "2123456789abcde".into()];
        let no_match = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "No matching shared videos found in organization",
                })
                .to_string(),
            },
        });
        assert_eq!(
            no_match
                .remove_videos_from_organization("0123456789abcde", &videos, "library-no-match-1",)
                .await
                .expect("no match"),
            BrowserRemoveVideosFromOrganizationReceiptV1::NoMatching {
                message: "No matching shared videos found in organization".into(),
            }
        );

        let partial = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "1 video removed from organization",
                })
                .to_string(),
            },
        });
        assert_eq!(
            partial
                .remove_videos_from_organization("0123456789abcde", &videos, "library-partial-1",)
                .await
                .expect("partial"),
            BrowserRemoveVideosFromOrganizationReceiptV1::Removed {
                message: "1 video removed from organization".into(),
                removed_count: 1,
            }
        );

        let add_organization_scope = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "2 videos added to organization",
                })
                .to_string(),
            },
        });
        assert_eq!(
            add_organization_scope
                .add_videos_to_space(
                    "0123456789abcde",
                    BrowserLibraryPlacementScopeV1::Organization,
                    &videos,
                    "library-scope-org-add-1",
                )
                .await
                .expect("organization pseudo-space add")
                .scope,
            BrowserLibraryPlacementScopeV1::Organization
        );

        let remove_organization_scope = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "Removed 2 video(s) from organization and folders",
                    "deletedCount": 2,
                })
                .to_string(),
            },
        });
        assert_eq!(
            remove_organization_scope
                .remove_videos_from_space(
                    "0123456789abcde",
                    BrowserLibraryPlacementScopeV1::Organization,
                    &videos,
                    "library-scope-org-remove-1",
                )
                .await
                .expect("organization pseudo-space remove")
                .scope,
            BrowserLibraryPlacementScopeV1::Organization
        );
    }

    #[tokio::test]
    async fn folder_assignment_validation_is_bounded_and_never_sends_invalid_input() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let one_video = vec!["1123456789abcde".into()];
        let too_many = vec!["1123456789abcde".into(); MAX_FOLDER_ASSIGNMENT_VIDEO_IDS + 1];

        assert_eq!(
            client
                .add_videos_to_folder(
                    "not-a-cap-id",
                    &one_video,
                    "3123456789abcde",
                    "folder-add-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .add_videos_to_folder("0123456789abcde", &[], "3123456789abcde", "folder-add-1",)
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .remove_videos_from_folder(
                    "0123456789abcde",
                    &too_many,
                    "3123456789abcde",
                    "folder-remove-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .remove_videos_from_folder(
                    "0123456789abcde",
                    &["invalid-video".into()],
                    "3123456789abcde",
                    "folder-remove-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .add_videos_to_folder(
                    "0123456789abcde",
                    &one_video,
                    "invalid-scope",
                    "folder-add-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .move_video_to_folder("invalid-video", None, None, "folder-move-1",)
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .move_video_to_folder(
                    "1123456789abcde",
                    Some("invalid-folder"),
                    None,
                    "folder-move-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .move_video_to_folder(
                    "1123456789abcde",
                    None,
                    Some("invalid-scope"),
                    "folder-move-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .move_video_to_folder("1123456789abcde", None, None, "bad key")
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert!(calls.borrow().is_empty());
    }

    #[tokio::test]
    async fn library_placement_validation_is_bounded_and_never_sends_invalid_input() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let one_video = vec!["1123456789abcde".into()];
        let invalid_video = vec!["invalid-video".into()];
        let too_many = vec!["1123456789abcde".into(); MAX_LIBRARY_PLACEMENT_VIDEO_IDS + 1];

        assert_eq!(
            client
                .add_videos_to_organization("invalid-org", &one_video, "library-org-add-1")
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .remove_videos_from_organization("0123456789abcde", &[], "library-org-remove-1",)
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .add_videos_to_space(
                    "3123456789abcde",
                    BrowserLibraryPlacementScopeV1::Space,
                    &too_many,
                    "library-space-add-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .remove_videos_from_space(
                    "3123456789abcde",
                    BrowserLibraryPlacementScopeV1::Space,
                    &invalid_video,
                    "library-space-remove-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .add_videos_to_space(
                    "invalid-scope",
                    BrowserLibraryPlacementScopeV1::Space,
                    &one_video,
                    "library-space-add-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .remove_videos_from_organization("0123456789abcde", &one_video, "bad key",)
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert!(calls.borrow().is_empty());

        let duplicates = vec!["1123456789abcde".into(), "1123456789abcde".into()];
        assert_eq!(
            client
                .add_videos_to_organization("0123456789abcde", &duplicates, "library-duplicates-1",)
                .await
                .expect("source-compatible duplicate list is canonicalized"),
            BrowserAddVideosToOrganizationReceiptV1 {
                message: "1 video is now in organization root".into(),
                total_updated: 1,
            }
        );
        assert_eq!(calls.borrow().len(), 1);
    }

    #[tokio::test]
    async fn every_folder_assignment_send_clears_cache_even_when_outcome_is_uncertain() {
        let videos = vec!["1123456789abcde".into()];
        let client = BrowserAuthenticatedClient::new(FakeTransport::new(BrowserRole::Owner));
        client
            .load(BrowserSurface::Folders, &query(BrowserSurface::Folders))
            .await
            .expect("folders");
        client
            .add_videos_to_folder(
                "0123456789abcde",
                &videos,
                "3123456789abcde",
                "cache-folder-add-1",
            )
            .await
            .expect("add");
        assert_eq!(client.cached_entries(), 0);
        client
            .load(BrowserSurface::Folders, &query(BrowserSurface::Folders))
            .await
            .expect("folders");
        client
            .remove_videos_from_folder(
                "0123456789abcde",
                &videos,
                "3123456789abcde",
                "cache-folder-remove-1",
            )
            .await
            .expect("remove");
        assert_eq!(client.cached_entries(), 0);
        client
            .load(BrowserSurface::Folders, &query(BrowserSurface::Folders))
            .await
            .expect("folders");
        client
            .move_video_to_folder("1123456789abcde", None, None, "cache-folder-move-1")
            .await
            .expect("move");
        assert_eq!(client.cached_entries(), 0);

        let uncertain = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        uncertain
            .load(BrowserSurface::Folders, &query(BrowserSurface::Folders))
            .await
            .expect("uncertain folders");
        assert_eq!(
            uncertain
                .move_video_to_folder("1123456789abcde", None, None, "uncertain-folder-move-1",)
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);
    }

    #[tokio::test]
    async fn every_library_placement_send_clears_cache_even_when_outcome_is_uncertain() {
        let videos = vec!["1123456789abcde".into()];
        let client = BrowserAuthenticatedClient::new(FakeTransport::new(BrowserRole::Owner));

        client
            .load(BrowserSurface::Library, &query(BrowserSurface::Library))
            .await
            .expect("library");
        client
            .add_videos_to_organization("0123456789abcde", &videos, "cache-library-org-add-1")
            .await
            .expect("add organization");
        assert_eq!(client.cached_entries(), 0);

        client
            .load(BrowserSurface::Library, &query(BrowserSurface::Library))
            .await
            .expect("library");
        client
            .remove_videos_from_organization(
                "0123456789abcde",
                &videos,
                "cache-library-org-remove-1",
            )
            .await
            .expect("remove organization");
        assert_eq!(client.cached_entries(), 0);

        client
            .load(BrowserSurface::Spaces, &query(BrowserSurface::Spaces))
            .await
            .expect("spaces");
        client
            .add_videos_to_space(
                "3123456789abcde",
                BrowserLibraryPlacementScopeV1::Space,
                &videos,
                "cache-library-space-add-1",
            )
            .await
            .expect("add space");
        assert_eq!(client.cached_entries(), 0);

        client
            .load(BrowserSurface::Spaces, &query(BrowserSurface::Spaces))
            .await
            .expect("spaces");
        client
            .remove_videos_from_space(
                "3123456789abcde",
                BrowserLibraryPlacementScopeV1::Space,
                &videos,
                "cache-library-space-remove-1",
            )
            .await
            .expect("remove space");
        assert_eq!(client.cached_entries(), 0);

        let uncertain = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        uncertain
            .load(BrowserSurface::Library, &query(BrowserSurface::Library))
            .await
            .expect("uncertain library");
        assert_eq!(
            uncertain
                .add_videos_to_organization(
                    "0123456789abcde",
                    &videos,
                    "uncertain-library-org-add-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);

        uncertain
            .load(BrowserSurface::Library, &query(BrowserSurface::Library))
            .await
            .expect("uncertain library");
        assert_eq!(
            uncertain
                .remove_videos_from_organization(
                    "0123456789abcde",
                    &videos,
                    "uncertain-library-org-remove-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);

        uncertain
            .load(BrowserSurface::Spaces, &query(BrowserSurface::Spaces))
            .await
            .expect("uncertain spaces");
        assert_eq!(
            uncertain
                .add_videos_to_space(
                    "3123456789abcde",
                    BrowserLibraryPlacementScopeV1::Space,
                    &videos,
                    "uncertain-library-space-add-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);

        uncertain
            .load(BrowserSurface::Spaces, &query(BrowserSurface::Spaces))
            .await
            .expect("uncertain spaces");
        assert_eq!(
            uncertain
                .remove_videos_from_space(
                    "3123456789abcde",
                    BrowserLibraryPlacementScopeV1::Space,
                    &videos,
                    "uncertain-library-space-remove-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);
    }

    #[tokio::test]
    async fn invalid_legacy_selection_never_sends_and_uncertain_selection_clears_cache() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        assert_eq!(
            client.set_legacy_active_organization("not-a-cap-id").await,
            Err(BrowserClientError::Invalid)
        );
        assert!(calls.borrow().is_empty());

        client
            .load(BrowserSurface::Dashboard, &query(BrowserSurface::Dashboard))
            .await
            .expect("dashboard");
        assert_eq!(client.cached_entries(), 1);
        client
            .set_legacy_active_organization("0123456789abcde")
            .await
            .expect("selection");
        assert_eq!(client.cached_entries(), 0);

        let uncertain = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        uncertain
            .load(BrowserSurface::Dashboard, &query(BrowserSurface::Dashboard))
            .await
            .expect("uncertain dashboard");
        assert_eq!(uncertain.cached_entries(), 1);
        assert_eq!(
            uncertain
                .set_legacy_active_organization("0123456789abcde")
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);
    }

    #[derive(Clone)]
    struct FixedResponseTransport {
        response: BrowserHttpResponse,
    }

    impl BrowserAuthenticatedTransport for FixedResponseTransport {
        fn send<'a>(
            &'a self,
            _request: BrowserHttpRequest,
        ) -> BrowserFuture<'a, Result<BrowserHttpResponse, BrowserClientError>> {
            Box::pin(async move { Ok(self.response.clone()) })
        }
    }

    #[tokio::test]
    async fn compatibility_action_requires_an_empty_204_receipt() {
        for response in [
            BrowserHttpResponse {
                status: 204,
                body: "unexpected".into(),
            },
            BrowserHttpResponse {
                status: 200,
                body: String::new(),
            },
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport { response });
            assert_eq!(
                client.set_theme(BrowserThemeV1::Light).await,
                Err(BrowserClientError::Unavailable)
            );
        }
    }

    #[tokio::test]
    async fn folder_assignment_receipts_are_action_specific_and_fail_closed() {
        let videos = vec!["1123456789abcde".into()];
        for response in [
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": false,
                    "message": "1 video added to folder",
                    "addedCount": 1,
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "1 video added to folder",
                    "addedCount": 2,
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "untrusted message",
                    "addedCount": 1,
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "1 video added to folder",
                    "addedCount": 1,
                    "unexpected": true,
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 204,
                body: String::new(),
            },
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport { response });
            assert_eq!(
                client
                    .add_videos_to_folder(
                        "0123456789abcde",
                        &videos,
                        "3123456789abcde",
                        "folder-add-1",
                    )
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }

        let wrong_action = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "1 video added to folder",
                    "addedCount": 1,
                })
                .to_string(),
            },
        });
        assert_eq!(
            wrong_action
                .remove_videos_from_folder(
                    "0123456789abcde",
                    &videos,
                    "3123456789abcde",
                    "folder-remove-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );

        for response in [
            BrowserHttpResponse {
                status: 200,
                body: String::new(),
            },
            BrowserHttpResponse {
                status: 204,
                body: "unexpected".into(),
            },
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport { response });
            assert_eq!(
                client
                    .move_video_to_folder("1123456789abcde", None, None, "folder-move-1",)
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }
    }

    #[tokio::test]
    async fn notification_actions_preserve_optional_fields_and_exact_idempotent_transport() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);

        client
            .mark_notifications_read(Some("0123456789abcde"), "notification-read-1")
            .await
            .expect("selected mark read");
        client
            .mark_notifications_read(None, "notification-read-all-1")
            .await
            .expect("bulk mark read");
        client
            .update_notification_preferences(
                BrowserNotificationPreferencesUpdateV1::new(true, false, true, false, Some(false)),
                "notification-preferences-1",
            )
            .await
            .expect("preferences with anonymous flag");
        client
            .update_notification_preferences(
                BrowserNotificationPreferencesUpdateV1::new(false, true, false, true, None),
                "notification-preferences-2",
            )
            .await
            .expect("preferences without anonymous flag");

        let calls = calls.borrow();
        assert_eq!(calls.len(), 4);
        assert_eq!(
            calls[0].path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID}")
        );
        assert_eq!(calls[1].path, calls[0].path);
        assert_eq!(
            calls[2].path,
            format!(
                "/api/v1/web/compatibility-actions/{LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID}"
            )
        );
        assert_eq!(calls[3].path, calls[2].path);
        let bodies = calls
            .iter()
            .map(|call| {
                assert_eq!(call.method, BrowserHttpMethod::Post);
                assert!(call.csrf_protected);
                let body = serde_json::from_str::<serde_json::Value>(
                    call.body.as_deref().expect("notification body"),
                )
                .expect("notification JSON");
                assert_eq!(
                    body.get("idempotency_key")
                        .and_then(serde_json::Value::as_str),
                    call.idempotency_key.as_deref()
                );
                body
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bodies[0]
                .get("notification_id")
                .and_then(serde_json::Value::as_str),
            Some("0123456789abcde")
        );
        assert!(bodies[1].get("notification_id").is_none());
        assert_eq!(
            bodies[2]
                .get("notifications")
                .and_then(|value| value.get("pauseAnonViews"))
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert!(
            bodies[3]
                .get("notifications")
                .and_then(|value| value.get("pauseAnonViews"))
                .is_none()
        );
    }

    #[tokio::test]
    async fn invalid_notification_actions_never_send() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        assert_eq!(
            client
                .mark_notifications_read(Some("invalid"), "notification-read-1")
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client.mark_notifications_read(None, "short").await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .update_notification_preferences(
                    BrowserNotificationPreferencesUpdateV1::new(false, false, false, false, None,),
                    "invalid/key",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert!(calls.borrow().is_empty());
    }

    #[tokio::test]
    async fn notification_actions_clear_cache_before_uncertain_send_and_reject_nonempty_void() {
        let uncertain = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        uncertain
            .load(BrowserSurface::Dashboard, &query(BrowserSurface::Dashboard))
            .await
            .expect("dashboard");
        assert_eq!(uncertain.cached_entries(), 1);
        assert_eq!(
            uncertain
                .mark_notifications_read(None, "notification-read-all-1")
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);

        let malformed = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 204,
                body: "{}".into(),
            },
        });
        assert_eq!(
            malformed
                .update_notification_preferences(
                    BrowserNotificationPreferencesUpdateV1::new(false, false, false, false, None,),
                    "notification-preferences-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );
    }

    #[tokio::test]
    async fn developer_actions_preserve_optional_fields_keys_and_exact_idempotent_transport() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let app_id = "0123456789abcde";

        let created = client
            .create_developer_app(
                " Frame app ",
                BrowserDeveloperEnvironmentV1::Development,
                "developer-create-1",
            )
            .await
            .expect("create developer app");
        assert_eq!(created.app_id(), app_id);
        assert!(created.keys().public_key().starts_with("cpk_"));
        assert!(created.keys().secret_key().starts_with("csk_"));
        assert!(!format!("{created:?}").contains(created.keys().secret_key()));

        client
            .update_developer_app(
                app_id,
                BrowserDeveloperAppPatchV1::new(
                    Some("Renamed".into()),
                    Some(BrowserDeveloperEnvironmentV1::Production),
                    BrowserDeveloperLogoPatchV1::Null,
                ),
                "developer-update-null-1",
            )
            .await
            .expect("update with explicit null");
        client
            .update_developer_app(
                app_id,
                BrowserDeveloperAppPatchV1::new(None, None, BrowserDeveloperLogoPatchV1::Missing),
                "developer-update-noop-1",
            )
            .await
            .expect("empty update");
        client
            .delete_developer_app(app_id, "developer-delete-app-1")
            .await
            .expect("delete app");
        client
            .add_developer_domain(
                app_id,
                " HTTPS://Example.COM:443 ",
                "developer-domain-add-1",
            )
            .await
            .expect("add domain");
        client
            .remove_developer_domain(app_id, "1123456789abcde", "developer-domain-remove-1")
            .await
            .expect("remove domain");
        let keys = client
            .regenerate_developer_keys(app_id, "developer-keys-regenerate-1")
            .await
            .expect("regenerate keys");
        assert!(keys.public_key().starts_with("cpk_"));
        assert!(!format!("{keys:?}").contains(keys.secret_key()));
        client
            .delete_developer_video(app_id, "2123456789abcde", "developer-video-delete-1")
            .await
            .expect("delete developer video");
        client
            .update_developer_auto_top_up(
                app_id,
                BrowserDeveloperAutoTopUpPatchV1::new(true, Some(500), Some(1_000)),
                "developer-top-up-1",
            )
            .await
            .expect("update auto top up");

        let calls = calls.borrow();
        assert_eq!(calls.len(), 9);
        let operation_ids = [
            LEGACY_CREATE_DEVELOPER_APP_ACTION_ID,
            LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID,
            LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID,
            LEGACY_DELETE_DEVELOPER_APP_ACTION_ID,
            LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID,
            LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID,
            LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID,
            LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID,
            LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID,
        ];
        let bodies = calls
            .iter()
            .zip(operation_ids)
            .map(|(call, operation_id)| {
                assert_eq!(call.method, BrowserHttpMethod::Post);
                assert!(call.csrf_protected);
                assert_eq!(
                    call.path,
                    format!("/api/v1/web/compatibility-actions/{operation_id}")
                );
                let body = serde_json::from_str::<serde_json::Value>(
                    call.body.as_deref().expect("developer body"),
                )
                .expect("developer JSON");
                assert_eq!(
                    body.get("schema_version")
                        .and_then(serde_json::Value::as_str),
                    Some(WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1)
                );
                assert_eq!(
                    body.get("idempotency_key")
                        .and_then(serde_json::Value::as_str),
                    call.idempotency_key.as_deref()
                );
                body
            })
            .collect::<Vec<_>>();
        assert!(
            bodies[1]
                .get("logo_url")
                .is_some_and(serde_json::Value::is_null)
        );
        assert!(bodies[2].get("logo_url").is_none());
        assert!(bodies[2].get("name").is_none());
        assert!(bodies[2].get("environment").is_none());
        assert_eq!(
            bodies[8]
                .get("threshold_micro_credits")
                .and_then(serde_json::Value::as_i64),
            Some(500)
        );
    }

    #[tokio::test]
    async fn invalid_developer_actions_never_send() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        assert_eq!(
            client
                .create_developer_app(
                    "   ",
                    BrowserDeveloperEnvironmentV1::Development,
                    "developer-create-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .delete_developer_app("bad", "developer-delete-app-1")
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .add_developer_domain(
                    "0123456789abcde",
                    "https://example.com/path",
                    "developer-domain-add-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .update_developer_auto_top_up(
                    "0123456789abcde",
                    BrowserDeveloperAutoTopUpPatchV1::new(true, Some(-1), Some(0)),
                    "developer-top-up-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .update_developer_app(
                    "0123456789abcde",
                    BrowserDeveloperAppPatchV1::new(
                        None,
                        None,
                        BrowserDeveloperLogoPatchV1::Value(
                            "x".repeat(MAX_DEVELOPER_LOGO_URL_CHARS + 1),
                        ),
                    ),
                    "developer-update-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert!(calls.borrow().is_empty());
    }

    #[tokio::test]
    async fn developer_actions_clear_cache_and_reject_malformed_secret_receipts() {
        let uncertain = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        uncertain
            .load(BrowserSurface::Developer, &query(BrowserSurface::Developer))
            .await
            .expect("developer workspace");
        assert_eq!(uncertain.cached_entries(), 1);
        assert_eq!(
            uncertain
                .delete_developer_app("0123456789abcde", "developer-delete-app-1")
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);

        for body in [
            serde_json::json!({
                "appId": "0123456789abcde",
                "publicKey": "cpk_bad",
                "secretKey": format!("csk_{}", "1".repeat(30)),
            }),
            serde_json::json!({
                "appId": "0123456789abcde",
                "publicKey": format!("cpk_{}", "0".repeat(30)),
                "secretKey": format!("csk_{}", "1".repeat(30)),
                "unexpected": true,
            }),
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
                response: BrowserHttpResponse {
                    status: 200,
                    body: body.to_string(),
                },
            });
            assert_eq!(
                client
                    .create_developer_app(
                        "Frame",
                        BrowserDeveloperEnvironmentV1::Production,
                        "developer-create-1",
                    )
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }
    }

    #[tokio::test]
    async fn membership_actions_preserve_presence_and_exact_idempotent_transport() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let invite_id = "0123456789abcde";
        let organization_id = "1123456789abcde";
        let space_id = "2123456789abcde";
        let first_user = "3123456789abcde";
        let second_user = "4123456789abcde";

        client
            .remove_organization_invite(invite_id, organization_id, "membership-invite-1")
            .await
            .expect("remove organization invite");
        client
            .add_space_member(
                space_id,
                first_user,
                BrowserLegacySpaceMemberRoleV1::Admin,
                "membership-add-1",
            )
            .await
            .expect("add space member");
        let user_ids = vec![first_user.to_owned(), second_user.to_owned()];
        let members = vec![
            BrowserSubmittedSpaceMemberV1::new(first_user, BrowserLegacySpaceMemberRoleV1::Admin),
            BrowserSubmittedSpaceMemberV1::new(second_user, BrowserLegacySpaceMemberRoleV1::Member),
        ];
        assert!(!format!("{:?}", members[0]).contains(first_user));
        assert_eq!(
            client
                .set_space_members(
                    space_id,
                    &user_ids,
                    None,
                    Some(&members),
                    "membership-set-members-1",
                )
                .await
                .expect("set explicit members"),
            3
        );
        assert_eq!(
            client
                .set_space_members(
                    space_id,
                    &user_ids,
                    Some(BrowserLegacySpaceMemberRoleV1::Member),
                    None,
                    "membership-set-users-1",
                )
                .await
                .expect("set fallback users"),
            3
        );

        let calls = calls.borrow();
        assert_eq!(calls.len(), 4);
        let expected_ids = [
            LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID,
            LEGACY_ADD_SPACE_MEMBER_ACTION_ID,
            LEGACY_SET_SPACE_MEMBERS_ACTION_ID,
            LEGACY_SET_SPACE_MEMBERS_ACTION_ID,
        ];
        let bodies = calls
            .iter()
            .zip(expected_ids)
            .map(|(call, operation_id)| {
                assert_eq!(call.method, BrowserHttpMethod::Post);
                assert!(call.csrf_protected);
                assert_eq!(
                    call.path,
                    format!("/api/v1/web/compatibility-actions/{operation_id}")
                );
                let body = serde_json::from_str::<serde_json::Value>(
                    call.body.as_deref().expect("membership body"),
                )
                .expect("membership JSON");
                assert_eq!(
                    body.get("schema_version")
                        .and_then(serde_json::Value::as_str),
                    Some(WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1)
                );
                assert_eq!(
                    body.get("idempotency_key")
                        .and_then(serde_json::Value::as_str),
                    call.idempotency_key.as_deref()
                );
                body
            })
            .collect::<Vec<_>>();
        assert_eq!(
            bodies[1].get("role").and_then(serde_json::Value::as_str),
            Some("admin")
        );
        assert!(bodies[2].get("role").is_none());
        assert_eq!(
            bodies[2]
                .get("members")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(bodies[3].get("members").is_none());
        assert_eq!(
            bodies[3].get("role").and_then(serde_json::Value::as_str),
            Some("member")
        );
    }

    #[tokio::test]
    async fn bulk_membership_actions_use_exact_ids_bodies_and_receipts() {
        assert_eq!(
            LEGACY_ADD_SPACE_MEMBERS_ACTION_ID,
            "cap-v1-b177854e2386c877"
        );
        assert_eq!(
            LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID,
            "cap-v1-38aff8e7221d0260"
        );
        assert_eq!(
            LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID,
            "cap-v1-135614e516c47bf4"
        );

        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let space_id = "2123456789abcde";
        let user_ids = vec!["3123456789abcde".to_owned(), "4123456789abcde".to_owned()];
        let member_ids = vec!["5123456789abcde".to_owned(), "6123456789abcde".to_owned()];

        let added = client
            .add_space_members(
                space_id,
                &user_ids,
                BrowserLegacySpaceMemberRoleV1::Admin,
                "membership-add-many-1",
            )
            .await
            .expect("add space members");
        assert_eq!(added.added(), user_ids.as_slice());
        assert!(added.already_members().is_empty());
        let debug = format!("{added:?}");
        assert!(debug.contains("added_count: 2"));
        assert!(!user_ids.iter().any(|user_id| debug.contains(user_id)));

        assert_eq!(
            client
                .batch_remove_space_members(&member_ids, "membership-remove-many-1")
                .await
                .expect("batch remove space members"),
            member_ids
        );
        client
            .remove_space_member(&member_ids[0], "membership-remove-one-1")
            .await
            .expect("remove space member");

        let empty = Vec::new();
        let empty_add = client
            .add_space_members(
                space_id,
                &empty,
                BrowserLegacySpaceMemberRoleV1::Member,
                "membership-add-empty-1",
            )
            .await
            .expect("empty add is source-compatible");
        assert!(empty_add.added().is_empty());
        assert!(empty_add.already_members().is_empty());
        assert!(
            client
                .batch_remove_space_members(&empty, "membership-remove-empty-1")
                .await
                .expect("empty remove is source-compatible")
                .is_empty()
        );

        let calls = calls.borrow();
        assert_eq!(calls.len(), 5);
        for call in calls.iter() {
            assert_eq!(call.method, BrowserHttpMethod::Post);
            assert!(call.csrf_protected);
            let body = serde_json::from_str::<serde_json::Value>(
                call.body.as_deref().expect("membership body"),
            )
            .expect("membership JSON");
            assert_eq!(
                body.get("idempotency_key")
                    .and_then(serde_json::Value::as_str),
                call.idempotency_key.as_deref(),
                "header and body idempotency keys must be identical",
            );
        }
        assert_eq!(
            calls[0].path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_ADD_SPACE_MEMBERS_ACTION_ID}")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[0].body.as_deref().expect("add-many body")
            )
            .expect("add-many JSON"),
            serde_json::json!({
                "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
                "space_id": space_id,
                "user_ids": user_ids,
                "role": "admin",
                "idempotency_key": "membership-add-many-1",
            })
        );
        assert_eq!(
            calls[1].path,
            format!(
                "/api/v1/web/compatibility-actions/{LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID}"
            )
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[1].body.as_deref().expect("remove-many body")
            )
            .expect("remove-many JSON"),
            serde_json::json!({
                "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
                "member_ids": member_ids,
                "idempotency_key": "membership-remove-many-1",
            })
        );
        assert_eq!(
            calls[2].path,
            format!("/api/v1/web/compatibility-actions/{LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID}")
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[2].body.as_deref().expect("remove-one body")
            )
            .expect("remove-one JSON"),
            serde_json::json!({
                "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
                "member_id": "5123456789abcde",
                "idempotency_key": "membership-remove-one-1",
            })
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[3].body.as_deref().expect("empty add body")
            )
            .expect("empty add JSON"),
            serde_json::json!({
                "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
                "space_id": space_id,
                "user_ids": [],
                "role": "member",
                "idempotency_key": "membership-add-empty-1",
            })
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                calls[4].body.as_deref().expect("empty remove body")
            )
            .expect("empty remove JSON"),
            serde_json::json!({
                "schema_version": WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1,
                "member_ids": [],
                "idempotency_key": "membership-remove-empty-1",
            })
        );
    }

    #[tokio::test]
    async fn invalid_bulk_membership_actions_are_bounded_and_never_send() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        let valid_user = "3123456789abcde".to_owned();
        let users = vec![valid_user.clone()];
        let too_many = vec![valid_user; MAX_MEMBERSHIP_TARGETS + 1];

        for result in [
            client
                .add_space_members(
                    "bad",
                    &users,
                    BrowserLegacySpaceMemberRoleV1::Member,
                    "membership-add-many-1",
                )
                .await,
            client
                .add_space_members(
                    "2123456789abcde",
                    &["bad".to_owned()],
                    BrowserLegacySpaceMemberRoleV1::Member,
                    "membership-add-many-1",
                )
                .await,
            client
                .add_space_members(
                    "2123456789abcde",
                    &too_many,
                    BrowserLegacySpaceMemberRoleV1::Member,
                    "membership-add-many-1",
                )
                .await,
            client
                .add_space_members(
                    "2123456789abcde",
                    &users,
                    BrowserLegacySpaceMemberRoleV1::Member,
                    "short",
                )
                .await,
        ] {
            assert_eq!(result, Err(BrowserClientError::Invalid));
        }
        for result in [
            client
                .batch_remove_space_members(&["bad".to_owned()], "membership-remove-many-1")
                .await,
            client
                .batch_remove_space_members(&too_many, "membership-remove-many-1")
                .await,
            client.batch_remove_space_members(&users, "short").await,
        ] {
            assert_eq!(result, Err(BrowserClientError::Invalid));
        }
        assert_eq!(
            client
                .remove_space_member("bad", "membership-remove-one-1")
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client.remove_space_member("5123456789abcde", "short").await,
            Err(BrowserClientError::Invalid)
        );
        assert!(calls.borrow().is_empty());
    }

    #[tokio::test]
    async fn every_bulk_membership_send_clears_cache_before_uncertain_failure() {
        let user_ids = vec!["3123456789abcde".to_owned()];
        let member_ids = vec!["5123456789abcde".to_owned()];

        let client = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        client
            .load(BrowserSurface::Space, &query(BrowserSurface::Space))
            .await
            .expect("space workspace");
        assert_eq!(client.cached_entries(), 1);
        assert_eq!(
            client
                .add_space_members(
                    "2123456789abcde",
                    &user_ids,
                    BrowserLegacySpaceMemberRoleV1::Member,
                    "membership-add-many-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(client.cached_entries(), 0);

        let client = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        client
            .load(BrowserSurface::Space, &query(BrowserSurface::Space))
            .await
            .expect("space workspace");
        assert_eq!(client.cached_entries(), 1);
        assert_eq!(
            client
                .batch_remove_space_members(&member_ids, "membership-remove-many-1")
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(client.cached_entries(), 0);

        let client = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        client
            .load(BrowserSurface::Space, &query(BrowserSurface::Space))
            .await
            .expect("space workspace");
        assert_eq!(client.cached_entries(), 1);
        assert_eq!(
            client
                .remove_space_member("5123456789abcde", "membership-remove-one-1")
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(client.cached_entries(), 0);
    }

    #[tokio::test]
    async fn bulk_membership_receipts_match_source_shapes_and_fail_closed() {
        let new_user = "3123456789abcde";
        let other_user = "4123456789abcde";
        let existing_user = "5123456789abcde";
        let unrelated_existing_user = "6123456789abcde";
        let forged_user = "7123456789abcde";

        let mixed_users = vec![
            new_user.to_owned(),
            existing_user.to_owned(),
            existing_user.to_owned(),
        ];
        let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "added": [new_user],
                    "alreadyMembers": [existing_user, unrelated_existing_user],
                })
                .to_string(),
            },
        });
        let receipt = client
            .add_space_members(
                "2123456789abcde",
                &mixed_users,
                BrowserLegacySpaceMemberRoleV1::Member,
                "membership-add-many-1",
            )
            .await
            .expect("mixed source response");
        assert_eq!(receipt.added(), &[new_user.to_owned()]);
        assert_eq!(
            receipt.already_members(),
            &[existing_user.to_owned(), unrelated_existing_user.to_owned()]
        );

        let duplicate_existing = vec![existing_user.to_owned(), existing_user.to_owned()];
        let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "added": [],
                    "alreadyMembers": duplicate_existing,
                })
                .to_string(),
            },
        });
        let receipt = client
            .add_space_members(
                "2123456789abcde",
                &duplicate_existing,
                BrowserLegacySpaceMemberRoleV1::Member,
                "membership-add-existing-1",
            )
            .await
            .expect("all-existing source response preserves submitted duplicates");
        assert!(receipt.added().is_empty());
        assert_eq!(receipt.already_members(), duplicate_existing.as_slice());

        let add_request = vec![new_user.to_owned(), other_user.to_owned()];
        for body in [
            serde_json::json!({
                "success": false,
                "added": [new_user, other_user],
                "alreadyMembers": [],
            }),
            serde_json::json!({
                "success": true,
                "added": [new_user, other_user],
                "alreadyMembers": [],
                "unexpected": true,
            }),
            serde_json::json!({
                "success": true,
                "added": [new_user, other_user],
                "already_members": [],
            }),
            serde_json::json!({
                "success": true,
                "added": [forged_user],
                "alreadyMembers": [],
            }),
            serde_json::json!({
                "success": true,
                "added": [other_user, new_user],
                "alreadyMembers": [],
            }),
            serde_json::json!({
                "success": true,
                "added": [new_user],
                "alreadyMembers": [],
            }),
            serde_json::json!({
                "success": true,
                "added": [new_user, other_user],
                "alreadyMembers": [new_user],
            }),
            serde_json::json!({
                "success": true,
                "added": [new_user],
                "alreadyMembers": [other_user, other_user],
            }),
            serde_json::json!({
                "success": true,
                "added": [],
                "alreadyMembers": [new_user],
            }),
            serde_json::json!({
                "success": true,
                "added": vec![new_user; MAX_MEMBERSHIP_TARGETS + 1],
                "alreadyMembers": [],
            }),
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
                response: BrowserHttpResponse {
                    status: 200,
                    body: body.to_string(),
                },
            });
            assert_eq!(
                client
                    .add_space_members(
                        "2123456789abcde",
                        &add_request,
                        BrowserLegacySpaceMemberRoleV1::Member,
                        "membership-add-many-1",
                    )
                    .await,
                Err(BrowserClientError::Unavailable),
                "malformed add receipt must fail closed: {body}",
            );
        }

        let member_ids = vec!["8123456789abcde".to_owned(), "9123456789abcde".to_owned()];
        let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
            response: BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({"success": true, "removed": []}).to_string(),
            },
        });
        assert!(
            client
                .batch_remove_space_members(&member_ids, "membership-remove-none-1")
                .await
                .expect("no matching rows is a successful empty removal")
                .is_empty()
        );

        for body in [
            serde_json::json!({"success": false, "removed": member_ids}),
            serde_json::json!({"success": true, "removed": [member_ids[0]]}),
            serde_json::json!({
                "success": true,
                "removed": [member_ids[1], member_ids[0]],
            }),
            serde_json::json!({"success": true, "removed": ["bad"]}),
            serde_json::json!({
                "success": true,
                "removed": member_ids,
                "unexpected": true,
            }),
            serde_json::json!({"success": true, "memberIds": member_ids}),
            serde_json::json!({
                "success": true,
                "removed": vec![&member_ids[0]; MAX_MEMBERSHIP_TARGETS + 1],
            }),
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
                response: BrowserHttpResponse {
                    status: 200,
                    body: body.to_string(),
                },
            });
            assert_eq!(
                client
                    .batch_remove_space_members(&member_ids, "membership-remove-many-1")
                    .await,
                Err(BrowserClientError::Unavailable),
                "malformed batch-remove receipt must fail closed: {body}",
            );
        }

        for body in [
            serde_json::json!({"success": false}),
            serde_json::json!({"success": true, "unexpected": true}),
            serde_json::json!({}),
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
                response: BrowserHttpResponse {
                    status: 200,
                    body: body.to_string(),
                },
            });
            assert_eq!(
                client
                    .remove_space_member("8123456789abcde", "membership-remove-one-1")
                    .await,
                Err(BrowserClientError::Unavailable),
                "malformed single-remove receipt must fail closed: {body}",
            );
        }
    }

    #[tokio::test]
    async fn invalid_membership_actions_never_send() {
        let transport = FakeTransport::new(BrowserRole::Owner);
        let calls = Rc::clone(&transport.calls);
        let client = BrowserAuthenticatedClient::new(transport);
        assert_eq!(
            client
                .remove_organization_invite("bad", "1123456789abcde", "membership-invite-1")
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert_eq!(
            client
                .add_space_member(
                    "2123456789abcde",
                    "bad",
                    BrowserLegacySpaceMemberRoleV1::Member,
                    "membership-add-1",
                )
                .await,
            Err(BrowserClientError::Invalid)
        );
        let too_many = vec!["3123456789abcde".to_owned(); MAX_MEMBERSHIP_TARGETS + 1];
        assert_eq!(
            client
                .set_space_members("2123456789abcde", &too_many, None, None, "membership-set-1",)
                .await,
            Err(BrowserClientError::Invalid)
        );
        assert!(calls.borrow().is_empty());
    }

    #[tokio::test]
    async fn membership_actions_clear_cache_and_reject_malformed_receipts() {
        let uncertain = BrowserAuthenticatedClient::new(FakeTransport::with_uncertain_mutation(
            BrowserRole::Owner,
        ));
        uncertain
            .load(BrowserSurface::Space, &query(BrowserSurface::Space))
            .await
            .expect("space workspace");
        assert_eq!(uncertain.cached_entries(), 1);
        assert_eq!(
            uncertain
                .add_space_member(
                    "2123456789abcde",
                    "3123456789abcde",
                    BrowserLegacySpaceMemberRoleV1::Member,
                    "membership-add-1",
                )
                .await,
            Err(BrowserClientError::Unavailable)
        );
        assert_eq!(uncertain.cached_entries(), 0);

        let users = vec!["3123456789abcde".to_owned()];
        for body in [
            serde_json::json!({"success": true, "count": 0}),
            serde_json::json!({"success": true, "count": 502}),
            serde_json::json!({"success": true, "count": 2, "extra": true}),
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport {
                response: BrowserHttpResponse {
                    status: 200,
                    body: body.to_string(),
                },
            });
            assert_eq!(
                client
                    .set_space_members("2123456789abcde", &users, None, None, "membership-set-1",)
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }
    }

    #[tokio::test]
    async fn library_placement_receipts_are_action_specific_and_fail_closed() {
        let videos = vec!["1123456789abcde".into(), "2123456789abcde".into()];

        for response in [
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": false,
                    "message": "2 videos are now in organization root",
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "1 video is now in organization root",
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "2 videos are now in organization root",
                    "totalUpdated": 2,
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 204,
                body: String::new(),
            },
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport { response });
            assert_eq!(
                client
                    .add_videos_to_organization("0123456789abcde", &videos, "library-org-add-1",)
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }

        for response in [
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "3 videos removed from organization",
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "0 videos removed from organization",
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": false,
                    "message": "No matching shared videos found in organization",
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "1 video removed from organization",
                    "deletedCount": 1,
                })
                .to_string(),
            },
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport { response });
            assert_eq!(
                client
                    .remove_videos_from_organization(
                        "0123456789abcde",
                        &videos,
                        "library-org-remove-1",
                    )
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }

        for response in [
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "2 videos added to organization",
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "2 videos added to space",
                    "validVideoCount": 2,
                })
                .to_string(),
            },
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport { response });
            assert_eq!(
                client
                    .add_videos_to_space(
                        "3123456789abcde",
                        BrowserLibraryPlacementScopeV1::Space,
                        &videos,
                        "library-space-add-1",
                    )
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }

        for response in [
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "Removed 2 video(s) from space and folders",
                    "deletedCount": 1,
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "Removed 2 video(s) from organization and folders",
                    "deletedCount": 2,
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "Removed 2 video(s) from space and folders",
                })
                .to_string(),
            },
            BrowserHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "success": true,
                    "message": "Removed 2 video(s) from space and folders",
                    "deletedCount": 2,
                    "unexpected": true,
                })
                .to_string(),
            },
        ] {
            let client = BrowserAuthenticatedClient::new(FixedResponseTransport { response });
            assert_eq!(
                client
                    .remove_videos_from_space(
                        "3123456789abcde",
                        BrowserLibraryPlacementScopeV1::Space,
                        &videos,
                        "library-space-remove-1",
                    )
                    .await,
                Err(BrowserClientError::Unavailable)
            );
        }
    }

    #[test]
    fn paths_and_queries_are_relative_bounded_and_percent_encoded() {
        assert_eq!(
            BrowserSurface::from_path("/spaces/space-1"),
            Some((BrowserSurface::Space, Some("space-1".into())))
        );
        assert!(BrowserSurface::from_path("//evil.test").is_none());
        assert!(BrowserSurface::from_path("/spaces/../admin").is_none());
        assert!(valid_cap_nanoid("0123456789abcde"));
        assert!(!valid_cap_nanoid("iiiiiiiiiiiiiii"));
        assert!(!valid_cap_nanoid("0123456789abcd"));
        assert_eq!(
            BrowserThemeV1::parse_cookie_value("light"),
            Some(BrowserThemeV1::Light)
        );
        assert_eq!(
            BrowserThemeV1::parse_cookie_value("dark"),
            Some(BrowserThemeV1::Dark)
        );
        for invalid in ["", "system", "Dark", " dark", "dark;"] {
            assert_eq!(BrowserThemeV1::parse_cookie_value(invalid), None);
        }
        assert!(valid_compatibility_action_path(&format!(
            "/api/v1/web/compatibility-actions/{LEGACY_THEME_ACTION_ID}"
        )));
        assert!(valid_compatibility_action_path(&format!(
            "/api/v1/web/compatibility-actions/{LEGACY_ACTIVE_ORGANIZATION_ACTION_ID}"
        )));
        for operation_id in [
            LEGACY_ADD_VIDEOS_TO_FOLDER_ACTION_ID,
            LEGACY_REMOVE_VIDEOS_FROM_FOLDER_ACTION_ID,
            LEGACY_MOVE_VIDEO_TO_FOLDER_ACTION_ID,
            LEGACY_ADD_VIDEOS_TO_ORGANIZATION_ACTION_ID,
            LEGACY_REMOVE_VIDEOS_FROM_ORGANIZATION_ACTION_ID,
            LEGACY_ADD_VIDEOS_TO_SPACE_ACTION_ID,
            LEGACY_REMOVE_VIDEOS_FROM_SPACE_ACTION_ID,
            LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID,
            LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID,
            LEGACY_CREATE_DEVELOPER_APP_ACTION_ID,
            LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID,
            LEGACY_DELETE_DEVELOPER_APP_ACTION_ID,
            LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID,
            LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID,
            LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID,
            LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID,
            LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID,
            LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID,
            LEGACY_ADD_SPACE_MEMBER_ACTION_ID,
            LEGACY_SET_SPACE_MEMBERS_ACTION_ID,
            LEGACY_ADD_SPACE_MEMBERS_ACTION_ID,
            LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID,
            LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID,
        ] {
            assert!(valid_compatibility_action_path(&format!(
                "/api/v1/web/compatibility-actions/{operation_id}"
            )));
        }
        for hostile in [
            "/api/v1/web/compatibility-actions/cap-v1-unknown",
            "/api/v1/web/compatibility-actions/cap-v1-7773d3e70d1d5919/extra",
            "/api/v1/web/compatibility-actions/",
        ] {
            assert!(!valid_compatibility_action_path(hostile), "{hostile}");
        }
        let query =
            BrowserQuery::new(Some("quarterly update".into()), None, Some(2), None).expect("query");
        assert_eq!(query.encoded(), "?q=quarterly%20update&page=2");
    }
}
