//! Authenticated web state contracts used by the concrete browser-side adapter.
//!
//! This module deliberately contains no fixture transport and no authority.
//! A production adapter must obtain a same-origin session decision, load only
//! the authorized DTO for the selected route, and execute mutations through
//! the typed port below. ADR 0004 keeps authenticated Render SSR disabled, so
//! this module deliberately contains no credential-forwarding transport. The
//! state machines are independent of Leptos so their duplicate-submit, retry,
//! and cache invalidation rules can be tested without a browser or network.

use std::fmt;

use crate::product::{AuthenticatedRoute, WorkspaceRole, WorkspaceView};

pub const ROUTE_MATRIX_SCHEMA: &str = "frame.web-authenticated-route-matrix.v1";
pub const MAX_SEARCH_BYTES: usize = 120;
pub const MAX_PAGE: u16 = 1_000;
pub const MAX_SAFE_RETURN_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingFilter {
    All,
    Ready,
    Processing,
    Failed,
}

impl RecordingFilter {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Ready => "ready",
            Self::Processing => "processing",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: Option<&str>) -> Result<Self, QueryError> {
        match value {
            None | Some("") | Some("all") => Ok(Self::All),
            Some("ready") => Ok(Self::Ready),
            Some("processing") => Ok(Self::Processing),
            Some("failed") => Ok(Self::Failed),
            Some(_) => Err(QueryError::InvalidFilter),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    System,
    Dark,
    Light,
}

impl ThemePreference {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn parse(value: Option<&str>) -> Result<Self, QueryError> {
        match value {
            None | Some("") | Some("system") => Ok(Self::System),
            Some("dark") => Ok(Self::Dark),
            Some("light") => Ok(Self::Light),
            Some(_) => Err(QueryError::InvalidTheme),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteViewQuery {
    search: Option<String>,
    filter: RecordingFilter,
    page: u16,
    theme: ThemePreference,
}

impl Default for RouteViewQuery {
    fn default() -> Self {
        Self {
            search: None,
            filter: RecordingFilter::All,
            page: 1,
            theme: ThemePreference::System,
        }
    }
}

impl RouteViewQuery {
    pub fn parse(
        search: Option<&str>,
        filter: Option<&str>,
        page: Option<&str>,
        theme: Option<&str>,
    ) -> Result<Self, QueryError> {
        let search = search.map(str::trim).filter(|value| !value.is_empty());
        if search.is_some_and(|value| {
            value.len() > MAX_SEARCH_BYTES
                || value.chars().any(char::is_control)
                || value.contains(['<', '>'])
        }) {
            return Err(QueryError::InvalidSearch);
        }
        let page = match page {
            None | Some("") => 1,
            Some(value) => value
                .parse::<u16>()
                .ok()
                .filter(|page| (1..=MAX_PAGE).contains(page))
                .ok_or(QueryError::InvalidPage)?,
        };
        Ok(Self {
            search: search.map(str::to_owned),
            filter: RecordingFilter::parse(filter)?,
            page,
            theme: ThemePreference::parse(theme)?,
        })
    }

    #[must_use]
    pub fn search(&self) -> Option<&str> {
        self.search.as_deref()
    }

    #[must_use]
    pub const fn filter(&self) -> RecordingFilter {
        self.filter
    }

    #[must_use]
    pub const fn page(&self) -> u16 {
        self.page
    }

    #[must_use]
    pub const fn theme(&self) -> ThemePreference {
        self.theme
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    InvalidSearch,
    InvalidFilter,
    InvalidPage,
    InvalidTheme,
}

/// A same-origin path suitable for a post-authentication redirect.
///
/// Query strings are intentionally forbidden: OTPs, OAuth codes, magic-link
/// material, and email addresses must never be copied into another URL.
#[derive(Clone, PartialEq, Eq)]
pub struct SafeReturnPath(String);

impl SafeReturnPath {
    pub fn parse(value: &str) -> Result<Self, ReturnPathError> {
        if value.is_empty()
            || value.len() > MAX_SAFE_RETURN_BYTES
            || !value.starts_with('/')
            || value.starts_with("//")
            || value.contains(['\\', '?', '#'])
            || value.chars().any(char::is_control)
            || value.split('/').any(|segment| segment == "..")
        {
            return Err(ReturnPathError);
        }
        let retained = AuthenticatedRoute::ALL.iter().any(|route| {
            if value == route.path() {
                return true;
            }
            route.dynamic_prefix().is_some_and(|prefix| {
                value
                    .strip_prefix(prefix)
                    .is_some_and(safe_resource_segment)
            })
        });
        if !retained {
            return Err(ReturnPathError);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn safe_resource_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

impl fmt::Debug for SafeReturnPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SafeReturnPath(<validated>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnPathError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormPhase {
    Pristine,
    Dirty,
    Invalid,
    Pending,
    Succeeded,
    RetryableFailure,
    TerminalFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionLease {
    revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitError {
    Duplicate,
    Invalid,
    Unchanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormState {
    phase: FormPhase,
    revision: u64,
    accepted_revision: Option<u64>,
}

impl Default for FormState {
    fn default() -> Self {
        Self {
            phase: FormPhase::Pristine,
            revision: 0,
            accepted_revision: None,
        }
    }
}

impl FormState {
    #[must_use]
    pub const fn phase(self) -> FormPhase {
        self.phase
    }

    #[must_use]
    pub const fn has_unsaved_changes(self) -> bool {
        matches!(
            self.phase,
            FormPhase::Dirty
                | FormPhase::Invalid
                | FormPhase::Pending
                | FormPhase::RetryableFailure
        )
    }

    pub fn edit(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.phase = FormPhase::Dirty;
    }

    pub fn mark_invalid(&mut self) {
        self.phase = FormPhase::Invalid;
    }

    pub fn begin_submit(&mut self) -> Result<SubmissionLease, SubmitError> {
        match self.phase {
            FormPhase::Pending => return Err(SubmitError::Duplicate),
            FormPhase::Invalid => return Err(SubmitError::Invalid),
            FormPhase::Pristine | FormPhase::Succeeded => return Err(SubmitError::Unchanged),
            FormPhase::Dirty | FormPhase::RetryableFailure | FormPhase::TerminalFailure => {}
        }
        self.phase = FormPhase::Pending;
        Ok(SubmissionLease {
            revision: self.revision,
        })
    }

    pub fn complete_success(&mut self, lease: SubmissionLease) -> bool {
        if self.phase != FormPhase::Pending || lease.revision != self.revision {
            return false;
        }
        self.accepted_revision = Some(lease.revision);
        self.phase = FormPhase::Succeeded;
        true
    }

    pub fn complete_failure(&mut self, lease: SubmissionLease, retryable: bool) -> bool {
        if self.phase != FormPhase::Pending || lease.revision != self.revision {
            return false;
        }
        self.phase = if retryable {
            FormPhase::RetryableFailure
        } else {
            FormPhase::TerminalFailure
        };
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheKey {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationKind {
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

impl MutationKind {
    #[must_use]
    pub const fn invalidates(self) -> &'static [CacheKey] {
        match self {
            Self::CompleteOnboarding => &[CacheKey::Session, CacheKey::Workspace],
            Self::CreateSpace => &[CacheKey::Spaces, CacheKey::Workspace],
            Self::CreateFolder => &[CacheKey::Folders, CacheKey::Library],
            Self::StartImport => &[CacheKey::Imports, CacheKey::Library],
            Self::UpdateAccount => &[CacheKey::Account, CacheKey::Session],
            Self::UpdateOrganization => &[CacheKey::Organization, CacheKey::Workspace],
            Self::UpdateMembers => &[CacheKey::Members, CacheKey::Workspace],
            Self::UpdateStorage => &[CacheKey::Storage, CacheKey::Imports],
            Self::CreateDeveloperKey => &[CacheKey::Developer],
            Self::UpdateBilling => &[CacheKey::Billing, CacheKey::Organization],
            Self::AdminAction => &[CacheKey::Admin, CacheKey::Workspace],
        }
    }

    #[must_use]
    pub const fn permitted_for(self, role: WorkspaceRole) -> bool {
        match self {
            Self::CompleteOnboarding | Self::UpdateAccount => true,
            Self::CreateSpace
            | Self::CreateFolder
            | Self::StartImport
            | Self::UpdateOrganization
            | Self::UpdateMembers
            | Self::UpdateStorage
            | Self::CreateDeveloperKey
            | Self::AdminAction => matches!(role, WorkspaceRole::Owner | WorkspaceRole::Admin),
            Self::UpdateBilling => matches!(role, WorkspaceRole::Owner),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorCode {
    Unauthenticated,
    NotFound,
    Invalid,
    Conflict,
    RateLimited,
    Unavailable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SafeCorrelationId(String);

impl SafeCorrelationId {
    pub fn parse(value: &str) -> Result<Self, ApiContractError> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ApiContractError::InvalidCorrelationId);
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Debug for SafeCorrelationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SafeCorrelationId(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeApiError {
    pub code: ApiErrorCode,
    pub retry_after_seconds: Option<u16>,
    pub correlation_id: SafeCorrelationId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiContractError {
    InvalidCorrelationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadRequest {
    pub route: AuthenticatedRoute,
    pub query: RouteViewQuery,
    pub resource_id: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct MutationRequest {
    pub kind: MutationKind,
    pub expected_revision: u64,
    pub idempotency_key: SafeCorrelationId,
}

impl fmt::Debug for MutationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MutationRequest")
            .field("kind", &self.kind)
            .field("expected_revision", &self.expected_revision)
            .field("idempotency_key", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationReceipt {
    pub revision: u64,
    pub invalidated: &'static [CacheKey],
}

/// Production adapter boundary. Implementations authenticate before loading,
/// authorize again for each mutation, enforce CSRF and idempotency at the API
/// boundary, and expose only stable safe errors.
pub trait AuthenticatedApiPort {
    fn load(&self, request: &LoadRequest) -> Result<WorkspaceView, SafeApiError>;

    fn mutate(&self, request: &MutationRequest) -> Result<MutationReceipt, SafeApiError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_query_is_bounded_and_closed() {
        let query = RouteViewQuery::parse(
            Some("  quarterly update  "),
            Some("ready"),
            Some("3"),
            Some("light"),
        )
        .expect("valid query");
        assert_eq!(query.search(), Some("quarterly update"));
        assert_eq!(query.filter(), RecordingFilter::Ready);
        assert_eq!(query.page(), 3);
        assert_eq!(query.theme(), ThemePreference::Light);
        assert_eq!(
            RouteViewQuery::parse(None, Some("unknown"), None, None),
            Err(QueryError::InvalidFilter)
        );
        assert_eq!(
            RouteViewQuery::parse(Some(&"x".repeat(121)), None, None, None),
            Err(QueryError::InvalidSearch)
        );
        assert_eq!(
            RouteViewQuery::parse(None, None, Some("0"), None),
            Err(QueryError::InvalidPage)
        );
    }

    #[test]
    fn return_paths_reject_open_redirects_and_query_secrets() {
        assert_eq!(
            SafeReturnPath::parse("/settings/account")
                .expect("retained path")
                .as_str(),
            "/settings/account"
        );
        assert!(SafeReturnPath::parse("/spaces/fixture-space").is_ok());
        for value in [
            "https://attacker.example",
            "//attacker.example",
            "/login",
            "/dashboard?otp=123456",
            "/../admin",
            "/spaces/",
            "/spaces/one/two",
        ] {
            assert!(SafeReturnPath::parse(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn form_state_fences_duplicates_retries_and_stale_completions() {
        let mut state = FormState::default();
        assert_eq!(state.begin_submit(), Err(SubmitError::Unchanged));
        state.edit();
        let lease = state.begin_submit().expect("first submission");
        assert_eq!(state.begin_submit(), Err(SubmitError::Duplicate));
        state.edit();
        assert!(!state.complete_success(lease));
        let current = state.begin_submit().expect("new revision");
        assert!(state.complete_failure(current, true));
        assert_eq!(state.phase(), FormPhase::RetryableFailure);
        let retry = state.begin_submit().expect("bounded retry");
        assert!(state.complete_success(retry));
        assert_eq!(state.phase(), FormPhase::Succeeded);
        assert!(!state.has_unsaved_changes());
    }

    #[test]
    fn mutation_policy_is_least_privilege_and_declares_invalidation() {
        assert!(MutationKind::UpdateAccount.permitted_for(WorkspaceRole::Member));
        assert!(!MutationKind::CreateSpace.permitted_for(WorkspaceRole::Member));
        assert!(!MutationKind::UpdateBilling.permitted_for(WorkspaceRole::Admin));
        assert!(MutationKind::UpdateBilling.permitted_for(WorkspaceRole::Owner));
        assert_eq!(
            MutationKind::StartImport.invalidates(),
            &[CacheKey::Imports, CacheKey::Library]
        );
    }

    #[test]
    fn sensitive_transport_identifiers_are_redacted() {
        let identifier = SafeCorrelationId::parse("safe-correlation_1").expect("safe identifier");
        let request = MutationRequest {
            kind: MutationKind::UpdateAccount,
            expected_revision: 7,
            idempotency_key: identifier.clone(),
        };
        assert_eq!(format!("{identifier:?}"), "SafeCorrelationId(<redacted>)");
        let debug = format!("{request:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("safe-correlation_1"));
    }
}
