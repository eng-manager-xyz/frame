use frame_client::{
    ApiVersion, CaptionTrack, FrameOrigin, PlaybackDescriptor, PublicShareSummary,
    ShareAvailability,
};

use crate::config::{Deployment, RuntimeConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatedRoute {
    Dashboard,
    Library,
    Spaces,
    Folders,
    Imports,
    Settings,
    Developer,
    Billing,
    Admin,
}

impl AuthenticatedRoute {
    pub const ALL: [Self; 9] = [
        Self::Dashboard,
        Self::Library,
        Self::Spaces,
        Self::Folders,
        Self::Imports,
        Self::Settings,
        Self::Developer,
        Self::Billing,
        Self::Admin,
    ];

    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Dashboard => "/dashboard",
            Self::Library => "/library",
            Self::Spaces => "/spaces",
            Self::Folders => "/folders",
            Self::Imports => "/imports",
            Self::Settings => "/settings",
            Self::Developer => "/developer",
            Self::Billing => "/billing",
            Self::Admin => "/admin",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Library => "Library",
            Self::Spaces => "Spaces",
            Self::Folders => "Folders",
            Self::Imports => "Imports",
            Self::Settings => "Settings",
            Self::Developer => "Developer",
            Self::Billing => "Billing",
            Self::Admin => "Admin",
        }
    }

    #[must_use]
    pub const fn permitted_for(self, role: WorkspaceRole) -> bool {
        match self {
            Self::Dashboard | Self::Library | Self::Spaces | Self::Folders => true,
            Self::Imports | Self::Settings => !matches!(role, WorkspaceRole::Member),
            Self::Developer | Self::Admin => {
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
}

impl WorkspaceRole {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Owner => "Owner",
            Self::Admin => "Admin",
            Self::Member => "Member",
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
    pub recordings: Vec<RecordingListItem>,
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
            recordings: Vec::new(),
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
        if summary.validate(&origin).is_err() {
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
                path: "/api/v1/public/shares/fixture-public/playback".into(),
                content_type: "video/mp4".into(),
                supports_range: true,
                captions: vec![CaptionTrack {
                    path: "/api/v1/public/shares/fixture-public/captions/en.vtt".into(),
                    language: "en".into(),
                    label: "English".into(),
                    default: true,
                }],
            }),
        },
        "fixture-processing" => PublicShareSummary {
            api_version: ApiVersion::current(),
            availability: ShareAvailability::Processing,
            title: None,
            description: None,
            canonical_url: Some(canonical_url),
            duration_ms: None,
            playback: None,
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
            ..ConfigValues::default()
        })
        .expect("production config")
    }

    #[test]
    fn role_matrix_is_least_privilege() {
        assert!(AuthenticatedRoute::Library.permitted_for(WorkspaceRole::Member));
        assert!(!AuthenticatedRoute::Imports.permitted_for(WorkspaceRole::Member));
        assert!(!AuthenticatedRoute::Billing.permitted_for(WorkspaceRole::Admin));
        assert!(AuthenticatedRoute::Billing.permitted_for(WorkspaceRole::Owner));
        assert!(AuthenticatedRoute::Admin.permitted_for(WorkspaceRole::Admin));
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
