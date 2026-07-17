use frame_client::{
    ApiVersion, CaptionTrack, FrameOrigin, InstantUiPhaseV1, InstantUiProgressV1,
    PlaybackDescriptor, PublicShareSummary, ShareAvailability,
};

use crate::config::{Deployment, RuntimeConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatedRoute {
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

impl AuthenticatedRoute {
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

    pub const NAVIGATION: [Self; 12] = [
        Self::Dashboard,
        Self::Library,
        Self::Spaces,
        Self::Folders,
        Self::Imports,
        Self::Settings,
        Self::StorageSettings,
        Self::Developer,
        Self::Billing,
        Self::Analytics,
        Self::Admin,
        Self::Onboarding,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
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

    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Dashboard => "/dashboard",
            Self::Library => "/library",
            Self::Spaces => "/spaces",
            Self::Space => "/spaces/fixture-space",
            Self::Folders => "/folders",
            Self::Folder => "/folders/fixture-folder",
            Self::Onboarding => "/onboarding",
            Self::Imports => "/imports",
            Self::Settings => "/settings",
            Self::AccountSettings => "/settings/account",
            Self::OrganizationSettings => "/settings/organization",
            Self::MemberSettings => "/settings/members",
            Self::StorageSettings => "/settings/storage",
            Self::Developer => "/developer",
            Self::Billing => "/billing",
            Self::Analytics => "/analytics",
            Self::Admin => "/admin",
        }
    }

    #[must_use]
    pub const fn pattern(self) -> &'static str {
        match self {
            Self::Space => "/spaces/{space_id}",
            Self::Folder => "/folders/{folder_id}",
            _ => self.path(),
        }
    }

    #[must_use]
    pub const fn dynamic_prefix(self) -> Option<&'static str> {
        match self {
            Self::Space => Some("/spaces/"),
            Self::Folder => Some("/folders/"),
            _ => None,
        }
    }

    #[must_use]
    pub const fn navigation_parent(self) -> Self {
        match self {
            Self::Space => Self::Spaces,
            Self::Folder => Self::Folders,
            Self::AccountSettings
            | Self::OrganizationSettings
            | Self::MemberSettings
            | Self::StorageSettings => Self::Settings,
            route => route,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Library => "Library",
            Self::Spaces => "Spaces",
            Self::Space => "Space",
            Self::Folders => "Folders",
            Self::Folder => "Folder",
            Self::Onboarding => "Onboarding",
            Self::Imports => "Imports",
            Self::Settings => "Settings",
            Self::AccountSettings => "Account settings",
            Self::OrganizationSettings => "Organization settings",
            Self::MemberSettings => "Members",
            Self::StorageSettings => "Storage",
            Self::Developer => "Developer",
            Self::Billing => "Billing",
            Self::Analytics => "Analytics",
            Self::Admin => "Admin",
        }
    }

    #[must_use]
    pub const fn component(self) -> &'static str {
        match self {
            Self::Dashboard => "recording-overview",
            Self::Library => "recording-library",
            Self::Spaces => "space-list",
            Self::Space => "space-detail",
            Self::Folders => "folder-list",
            Self::Folder => "folder-detail",
            Self::Onboarding => "onboarding-form",
            Self::Imports => "import-progress",
            Self::Settings => "settings-index",
            Self::AccountSettings => "account-form",
            Self::OrganizationSettings => "organization-form",
            Self::MemberSettings => "member-management",
            Self::StorageSettings => "storage-integration",
            Self::Developer => "developer-credentials",
            Self::Billing => "billing-overview",
            Self::Analytics => "analytics-overview",
            Self::Admin => "admin-operations",
        }
    }

    #[must_use]
    pub const fn critical_journey(self) -> &'static str {
        match self {
            Self::Dashboard => "inspect-recent-recordings",
            Self::Library => "search-filter-and-open-recording",
            Self::Spaces => "list-and-create-space",
            Self::Space => "inspect-space-members-and-recordings",
            Self::Folders => "list-and-create-folder",
            Self::Folder => "inspect-folder-recordings",
            Self::Onboarding => "complete-workspace-onboarding",
            Self::Imports => "start-and-monitor-import",
            Self::Settings => "choose-settings-surface",
            Self::AccountSettings => "update-account-and-revoke-sessions",
            Self::OrganizationSettings => "update-organization-policy",
            Self::MemberSettings => "invite-and-manage-members",
            Self::StorageSettings => "configure-and-verify-storage",
            Self::Developer => "create-and-revoke-api-credentials",
            Self::Billing => "inspect-plan-and-manage-billing",
            Self::Analytics => "inspect-usage-and-consent",
            Self::Admin => "inspect-and-run-audited-admin-operation",
        }
    }

    #[must_use]
    pub const fn permitted_for(self, role: WorkspaceRole) -> bool {
        match self {
            Self::Dashboard
            | Self::Library
            | Self::Spaces
            | Self::Space
            | Self::Folders
            | Self::Folder
            | Self::Onboarding
            | Self::Settings
            | Self::AccountSettings => true,
            Self::Imports
            | Self::OrganizationSettings
            | Self::MemberSettings
            | Self::StorageSettings
            | Self::Developer
            | Self::Analytics
            | Self::Admin => {
                matches!(role, WorkspaceRole::Owner | WorkspaceRole::Admin)
            }
            Self::Billing => matches!(role, WorkspaceRole::Owner),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRole {
    Owner,
    Admin,
    Member,
    Viewer,
}

impl WorkspaceRole {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Owner => "Owner",
            Self::Admin => "Admin",
            Self::Member => "Member",
            Self::Viewer => "Viewer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    Ready,
    Processing,
    Failed,
}

impl RecordingState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Processing => "Processing",
            Self::Failed => "Needs attention",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingListItem {
    pub public_id: String,
    pub title: String,
    pub state: RecordingState,
    pub duration_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportProgress {
    pub label: String,
    pub completed: u16,
    pub total: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceResource {
    pub id: String,
    pub name: String,
}

impl ImportProgress {
    #[must_use]
    pub fn percent(&self) -> u16 {
        if self.total == 0 {
            return 0;
        }
        self.completed.min(self.total).saturating_mul(100) / self.total
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceView {
    pub organization_name: String,
    pub member_label: String,
    pub role: WorkspaceRole,
    pub revision: u64,
    pub recordings: Vec<RecordingListItem>,
    pub spaces: Vec<WorkspaceResource>,
    pub folders: Vec<WorkspaceResource>,
    pub import: Option<ImportProgress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthenticatedState {
    Loading,
    Unauthenticated,
    Denied,
    Failed,
    Ready(WorkspaceView),
}

/// Deterministic data is available only for local visual/accessibility
/// fixtures. Production callers cannot select it, so it cannot become an
/// authentication bypass or render real tenant data before session bootstrap.
#[must_use]
pub fn local_authenticated_fixture(
    config: &RuntimeConfig,
    fixture: Option<&str>,
) -> AuthenticatedState {
    if config.deployment() != Deployment::Local {
        return AuthenticatedState::Unauthenticated;
    }

    match fixture {
        Some("loading") => AuthenticatedState::Loading,
        Some("denied") => AuthenticatedState::Denied,
        Some("failed") => AuthenticatedState::Failed,
        Some("owner") => AuthenticatedState::Ready(workspace_fixture(WorkspaceRole::Owner)),
        Some("admin") => AuthenticatedState::Ready(workspace_fixture(WorkspaceRole::Admin)),
        Some("member") => AuthenticatedState::Ready(workspace_fixture(WorkspaceRole::Member)),
        Some("empty") => AuthenticatedState::Ready(WorkspaceView {
            organization_name: "Local empty workspace".into(),
            member_label: "Local accessibility fixture".into(),
            role: WorkspaceRole::Owner,
            revision: 0,
            recordings: Vec::new(),
            spaces: Vec::new(),
            folders: Vec::new(),
            import: None,
        }),
        _ => AuthenticatedState::Unauthenticated,
    }
}

fn workspace_fixture(role: WorkspaceRole) -> WorkspaceView {
    WorkspaceView {
        organization_name: "Local Frame workspace".into(),
        member_label: "Local accessibility fixture".into(),
        role,
        revision: 7,
        recordings: vec![
            RecordingListItem {
                public_id: "fixture-public".into(),
                title: "Product walkthrough".into(),
                state: RecordingState::Ready,
                duration_label: Some("4 minutes, 12 seconds".into()),
            },
            RecordingListItem {
                public_id: "fixture-processing".into(),
                title: "Weekly update".into(),
                state: RecordingState::Processing,
                duration_label: None,
            },
            RecordingListItem {
                public_id: "fixture-failed".into(),
                title: "Interrupted import".into(),
                state: RecordingState::Failed,
                duration_label: None,
            },
        ],
        spaces: vec![WorkspaceResource {
            id: "fixture-space".into(),
            name: "Product".into(),
        }],
        folders: vec![WorkspaceResource {
            id: "fixture-folder".into(),
            name: "Walkthroughs".into(),
        }],
        import: Some(ImportProgress {
            label: "Local import rehearsal".into(),
            completed: 3,
            total: 5,
        }),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ShareView {
    Validated(PublicShareSummary),
    Unavailable,
}

impl ShareView {
    #[must_use]
    pub fn from_summary(config: &RuntimeConfig, summary: PublicShareSummary) -> Self {
        let Ok(origin) = frame_origin(config) else {
            return Self::Unavailable;
        };
        if summary.validate(&origin).is_err()
            || !crate::share_player::summary_is_scope_safe(&summary)
        {
            return Self::Unavailable;
        }
        Self::Validated(summary)
    }

    #[must_use]
    pub fn availability(&self) -> ShareAvailability {
        match self {
            Self::Validated(summary) => summary.availability,
            Self::Unavailable => ShareAvailability::Unavailable,
        }
    }

    #[must_use]
    pub fn matches_route(&self, route_scope: &str) -> bool {
        matches!(
            self,
            Self::Validated(summary)
                if crate::share_player::summary_matches_route(summary, route_scope)
        )
    }
}

/// The real summary will be loaded through `FrameClient::public_share` when
/// the server transport is connected. These local-only fixtures exercise the
/// exact validated DTO boundary without enabling a production data path.
#[must_use]
pub fn local_share_fixture(config: &RuntimeConfig, identifier: &str) -> ShareView {
    if config.deployment() != Deployment::Local {
        return ShareView::Unavailable;
    }

    let canonical_url = format!("{}/s/{identifier}", config.public_origin().as_str());
    let summary = match identifier {
        "fixture-public" => PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Public,
            title: Some("Local public recording".into()),
            description: Some("A provider-neutral playback fixture for local UI checks.".into()),
            canonical_url: Some(canonical_url),
            duration_ms: Some(252_000),
            playback: Some(PlaybackDescriptor {
                path: "/api/v1/public/shares/fixture-public/media".into(),
                content_type: "video/mp4".into(),
                supports_range: true,
                captions: vec![CaptionTrack {
                    path: "/api/v1/public/shares/fixture-public/captions/en".into(),
                    language: "en".into(),
                    label: "English".into(),
                    default: true,
                }],
            }),
            processing_status: None,
        },
        "fixture-processing" => PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Processing,
            title: None,
            description: None,
            canonical_url: Some(canonical_url),
            duration_ms: None,
            playback: None,
            processing_status: Some(InstantUiProgressV1 {
                schema_version: 1,
                phase: InstantUiPhaseV1::Finalizing,
                progress_basis_points: None,
                retrying: false,
                error: None,
            }),
        },
        _ => return ShareView::Unavailable,
    };
    ShareView::from_summary(config, summary)
}

fn frame_origin(config: &RuntimeConfig) -> Result<FrameOrigin, frame_client::ClientError> {
    match config.deployment() {
        Deployment::Local => FrameOrigin::for_local_testing(config.public_origin().as_str()),
        Deployment::Preview | Deployment::Production => {
            FrameOrigin::parse_https(config.public_origin().as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigValues;

    fn local_config() -> RuntimeConfig {
        RuntimeConfig::from_values(ConfigValues::default()).expect("local config")
    }

    fn production_config() -> RuntimeConfig {
        RuntimeConfig::from_values(ConfigValues {
            deployment: Some("production".into()),
            public_origin: Some("https://frame.engmanager.xyz".into()),
            api_origin: Some("https://frame.engmanager.xyz".into()),
            proxy_trust: Some("render".into()),
            ..ConfigValues::default()
        })
        .expect("production config")
    }

    #[test]
    fn role_matrix_is_least_privilege() {
        assert!(AuthenticatedRoute::Library.permitted_for(WorkspaceRole::Member));
        assert!(!AuthenticatedRoute::Imports.permitted_for(WorkspaceRole::Member));
        assert!(AuthenticatedRoute::AccountSettings.permitted_for(WorkspaceRole::Member));
        assert!(!AuthenticatedRoute::MemberSettings.permitted_for(WorkspaceRole::Member));
        assert!(!AuthenticatedRoute::Billing.permitted_for(WorkspaceRole::Admin));
        assert!(AuthenticatedRoute::Billing.permitted_for(WorkspaceRole::Owner));
        assert!(AuthenticatedRoute::Admin.permitted_for(WorkspaceRole::Admin));
    }

    #[test]
    fn committed_route_matrix_matches_the_rust_contract_exactly() {
        let matrix: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/web-authenticated/v1/route-matrix.json"
        ))
        .expect("valid route matrix JSON");
        assert_eq!(matrix["schema"], crate::authenticated::ROUTE_MATRIX_SCHEMA);
        let rows = matrix["routes"].as_array().expect("route rows");
        assert_eq!(rows.len(), AuthenticatedRoute::ALL.len());
        for route in AuthenticatedRoute::ALL {
            let row = rows
                .iter()
                .find(|row| row["name"] == route.name())
                .unwrap_or_else(|| panic!("matrix row for {}", route.name()));
            assert_eq!(row["pattern"], route.pattern());
            assert_eq!(row["fixture_path"], route.path());
            assert_eq!(row["component"], route.component());
            assert_eq!(row["journey"], route.critical_journey());
            let allowed = row["allowed_roles"].as_array().expect("allowed roles");
            for (role, name) in [
                (WorkspaceRole::Owner, "owner"),
                (WorkspaceRole::Admin, "admin"),
                (WorkspaceRole::Member, "member"),
            ] {
                assert_eq!(
                    allowed.iter().any(|candidate| candidate == name),
                    route.permitted_for(role),
                    "{} permission for {name}",
                    route.name()
                );
            }
        }
    }

    #[test]
    fn production_cannot_select_authenticated_fixtures() {
        for fixture in ["owner", "admin", "member", "empty"] {
            assert_eq!(
                local_authenticated_fixture(&production_config(), Some(fixture)),
                AuthenticatedState::Unauthenticated
            );
        }
    }

    #[test]
    fn share_fixture_is_validated_by_frame_client() {
        let public = local_share_fixture(&local_config(), "fixture-public");
        assert_eq!(public.availability(), ShareAvailability::Public);
        let unavailable = local_share_fixture(&local_config(), "../private");
        assert_eq!(unavailable.availability(), ShareAvailability::Unavailable);
    }

    #[test]
    fn descriptor_with_storage_like_path_fails_closed() {
        let config = local_config();
        let summary = PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Public,
            title: Some("Should not render".into()),
            description: None,
            canonical_url: Some("http://127.0.0.1:3000/s/public".into()),
            duration_ms: None,
            playback: Some(PlaybackDescriptor {
                path: "/api/v1/public/shares/public/object-key".into(),
                content_type: "video/mp4".into(),
                supports_range: true,
                captions: Vec::new(),
            }),
            processing_status: None,
        };
        assert!(matches!(
            ShareView::from_summary(&config, summary),
            ShareView::Unavailable
        ));
    }

    #[test]
    fn import_progress_is_bounded() {
        assert_eq!(
            ImportProgress {
                label: "bad provider count".into(),
                completed: 9,
                total: 3,
            }
            .percent(),
            100
        );
        assert_eq!(
            ImportProgress {
                label: "empty".into(),
                completed: 0,
                total: 0,
            }
            .percent(),
            0
        );
    }
}
