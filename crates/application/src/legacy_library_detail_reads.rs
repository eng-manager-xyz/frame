//! Source-pinned contracts for Cap's library-detail and dashboard search reads.
//!
//! `getUserVideos` uses a caller-supplied organization/space only to choose a
//! folder join and otherwise returns every video owned by the actor, including
//! videos in other tenants. Frame preserves its projection, counts, upload
//! marker, folder branch, and effective-date ordering while requiring the
//! requested scope and every returned video to belong to the actor's live
//! active organization. `searchDashboardVideos` already contains tenant and
//! visibility predicates; Frame reasserts those predicates in D1 and preserves
//! its JavaScript normalization, LIKE escaping, prefix rank, and eight-row cap.

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

pub const LEGACY_LIBRARY_DETAIL_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_GET_USER_VIDEOS_OPERATION_ID: &str = "cap-v1-17a71c3e18600d06";
pub const LEGACY_SEARCH_DASHBOARD_VIDEOS_OPERATION_ID: &str = "cap-v1-39e8966f308c1528";
pub const LEGACY_GET_USER_VIDEOS_IDENTITY: &str =
    "action://apps/web/actions/spaces/get-user-videos.ts#getUserVideos";
pub const LEGACY_SEARCH_DASHBOARD_VIDEOS_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/_components/Navbar/search.ts#searchDashboardVideos";
pub const LEGACY_LIBRARY_DETAIL_POLICY: &str = "organization_library.v1";
pub const LEGACY_LIBRARY_DETAIL_MAX_BODY_BYTES: usize = 2 * 1024;
pub const LEGACY_LIBRARY_DETAIL_MAX_ID_BYTES: usize = 256;
pub const LEGACY_DASHBOARD_SEARCH_MAX_QUERY_UTF16: usize = 80;
pub const LEGACY_DASHBOARD_SEARCH_MIN_QUERY_UTF16: usize = 2;
pub const LEGACY_DASHBOARD_SEARCH_MAX_RESULTS: usize = 8;
pub const LEGACY_LIBRARY_DETAIL_NO_PROTECTED_GATES: &[&str] = &[];

pub const LEGACY_GET_USER_VIDEOS_SOURCE_MANIFEST_SHA256: &str =
    "960be699e80eb782d5a04cc7880310b03bf762983372efee39e07efbec355bbf";
pub const LEGACY_SEARCH_DASHBOARD_VIDEOS_SOURCE_MANIFEST_SHA256: &str =
    "4a760c534a6b32ad0d75602d86c18ada4331c7ff5e8409c3792f6ccae0ece544";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryDetailSourceRoleV1 {
    Action,
    Caller,
    Authentication,
    Schema,
    Database,
    Identifier,
    DependencyLock,
}

impl LegacyLibraryDetailSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Caller => "caller",
            Self::Authentication => "authentication",
            Self::Schema => "schema",
            Self::Database => "database",
            Self::Identifier => "identifier",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryDetailSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyLibraryDetailSourceRoleV1,
}

const SESSION_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/database/auth/session.ts",
    symbol: "getCurrentUser",
    sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    role: LegacyLibraryDetailSourceRoleV1::Authentication,
};
const AUTH_OPTIONS_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/database/auth/auth-options.ts",
    symbol: "authOptions+session callback",
    sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    role: LegacyLibraryDetailSourceRoleV1::Authentication,
};
const SCHEMA_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "users+organizations+memberships+spaces+folders+videos+placements+comments+uploads",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyLibraryDetailSourceRoleV1::Schema,
};
const DATABASE_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/database/index.ts",
    symbol: "db",
    sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    role: LegacyLibraryDetailSourceRoleV1::Database,
};
const SPACE_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/web-domain/src/Space.ts",
    symbol: "SpaceIdOrOrganisationId",
    sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    role: LegacyLibraryDetailSourceRoleV1::Identifier,
};
const VIDEO_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/web-domain/src/Video.ts",
    symbol: "VideoId",
    sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    role: LegacyLibraryDetailSourceRoleV1::Identifier,
};
const USER_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/web-domain/src/User.ts",
    symbol: "UserId",
    sha256: "5b3374425a4c9df1501af34c8f1f780c3f7612f093cd2ff0ed5c442e41e7cee1",
    role: LegacyLibraryDetailSourceRoleV1::Identifier,
};
const WEB_PACKAGE_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "apps/web/package.json",
    symbol: "server-action runtime dependencies",
    sha256: "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
    role: LegacyLibraryDetailSourceRoleV1::DependencyLock,
};
const DATABASE_PACKAGE_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "packages/database/package.json",
    symbol: "drizzle database dependencies",
    sha256: "95629fc376bfc4df4f9f69a28a874e8bcf8496ccec276fd2168cfc9720e4a057",
    role: LegacyLibraryDetailSourceRoleV1::DependencyLock,
};
const LOCK_SOURCE: LegacyLibraryDetailSourcePinV1 = LegacyLibraryDetailSourcePinV1 {
    path: "pnpm-lock.yaml",
    symbol: "drizzle-orm+mysql2+next-auth resolutions",
    sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    role: LegacyLibraryDetailSourceRoleV1::DependencyLock,
};

pub const LEGACY_GET_USER_VIDEOS_SOURCES: &[LegacyLibraryDetailSourcePinV1] = &[
    LegacyLibraryDetailSourcePinV1 {
        path: "apps/web/actions/spaces/get-user-videos.ts",
        symbol: "getUserVideos",
        sha256: "c6607b999cc7ed0bc687d94fb791c55c2a97c0a6142c9ba60b977aac05d80a5e",
        role: LegacyLibraryDetailSourceRoleV1::Action,
    },
    LegacyLibraryDetailSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosDialog.tsx",
        symbol: "getVideos",
        sha256: "238104cd063757bb8bf785f94acf5c75ccb7b9ef14b7ef519636925b091a9201",
        role: LegacyLibraryDetailSourceRoleV1::Caller,
    },
    LegacyLibraryDetailSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosToOrganizationDialog.tsx",
        symbol: "getVideos",
        sha256: "c8bc5ef4dc2cc0dc8f452d2769be9c3be49e8be6204cb5d9c2b9bdd0d327efd7",
        role: LegacyLibraryDetailSourceRoleV1::Caller,
    },
    LegacyLibraryDetailSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/folder/[folderId]/AddVideosButton.tsx",
        symbol: "getVideos",
        sha256: "a526a81701c68b9d76367817164fcd0b72e7e3c930d89de03f782c6f388ff871",
        role: LegacyLibraryDetailSourceRoleV1::Caller,
    },
    LegacyLibraryDetailSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosDialogBase.tsx",
        symbol: "VideoData+user-videos query",
        sha256: "f0af1cb1bb501582cc83c4c68b9613e7c8b21823d246b18d29edb405b0777c89",
        role: LegacyLibraryDetailSourceRoleV1::Caller,
    },
    SESSION_SOURCE,
    AUTH_OPTIONS_SOURCE,
    SCHEMA_SOURCE,
    DATABASE_SOURCE,
    SPACE_SOURCE,
    VIDEO_SOURCE,
    USER_SOURCE,
    WEB_PACKAGE_SOURCE,
    DATABASE_PACKAGE_SOURCE,
    LOCK_SOURCE,
];

pub const LEGACY_SEARCH_DASHBOARD_VIDEOS_SOURCES: &[LegacyLibraryDetailSourcePinV1] = &[
    LegacyLibraryDetailSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/_components/Navbar/search.ts",
        symbol: "searchDashboardVideos",
        sha256: "210244ffd7180d957960f27f1d7b7f420bf301daf29dc2d389fa51125fd2c44f",
        role: LegacyLibraryDetailSourceRoleV1::Action,
    },
    LegacyLibraryDetailSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/_components/Navbar/DashboardSearch.tsx",
        symbol: "video search debounce+cache+projection",
        sha256: "2b5a3a4027023c4f2dc61cee8673ab52a69aae6bb2601f900e41f57ac196c3da",
        role: LegacyLibraryDetailSourceRoleV1::Caller,
    },
    SESSION_SOURCE,
    AUTH_OPTIONS_SOURCE,
    SCHEMA_SOURCE,
    DATABASE_SOURCE,
    VIDEO_SOURCE,
    USER_SOURCE,
    WEB_PACKAGE_SOURCE,
    DATABASE_PACKAGE_SOURCE,
    LOCK_SOURCE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryDetailActionV1 {
    GetUserVideos,
    SearchDashboardVideos,
}

impl LegacyLibraryDetailActionV1 {
    #[must_use]
    pub fn parse(operation_id: &str) -> Option<Self> {
        match operation_id {
            LEGACY_GET_USER_VIDEOS_OPERATION_ID => Some(Self::GetUserVideos),
            LEGACY_SEARCH_DASHBOARD_VIDEOS_OPERATION_ID => Some(Self::SearchDashboardVideos),
            _ => None,
        }
    }

    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::GetUserVideos => LEGACY_GET_USER_VIDEOS_OPERATION_ID,
            Self::SearchDashboardVideos => LEGACY_SEARCH_DASHBOARD_VIDEOS_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn identity(self) -> &'static str {
        match self {
            Self::GetUserVideos => LEGACY_GET_USER_VIDEOS_IDENTITY,
            Self::SearchDashboardVideos => LEGACY_SEARCH_DASHBOARD_VIDEOS_IDENTITY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryDetailProfileV1 {
    pub action: LegacyLibraryDetailActionV1,
    pub auth: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_LIBRARY_DETAIL_PROFILES: &[LegacyLibraryDetailProfileV1] = &[
    LegacyLibraryDetailProfileV1 {
        action: LegacyLibraryDetailActionV1::GetUserVideos,
        auth: "host_only_browser_session",
        success: "success_true_owned_video_projection_effective_date_desc",
        validation: "bounded_scope_id_and_lossless_projection",
        authorization: "active_tenant_and_requested_root_or_visible_space",
        idempotency: "read_only_client_key_forbidden_retry_safe",
        failure: "source_catch_all_success_false_failed_to_fetch_videos",
    },
    LegacyLibraryDetailProfileV1 {
        action: LegacyLibraryDetailActionV1::SearchDashboardVideos,
        auth: "host_only_browser_session_or_empty_array",
        success: "bare_array_prefix_rank_then_effective_date_desc_limit_eight",
        validation: "javascript_whitespace_collapse_trim_utf16_slice_80_minimum_2",
        authorization: "active_tenant_membership_and_owned_shared_or_visible_space",
        idempotency: "read_only_client_key_forbidden_retry_safe",
        failure: "empty_for_missing_context_otherwise_redacted_unavailable",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyLibraryDetailPrincipalV1 {
    pub actor_id: String,
    pub active_organization_id: String,
    pub active_legacy_organization_id: String,
}

impl LegacyLibraryDetailPrincipalV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.actor_id)
            && valid_id(&self.active_organization_id)
            && valid_cap_nanoid(&self.active_legacy_organization_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyLibraryDetailInputV1 {
    GetUserVideos { legacy_scope_id: String },
    SearchDashboardVideos { query: String },
}

impl LegacyLibraryDetailInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyLibraryDetailActionV1 {
        match self {
            Self::GetUserVideos { .. } => LegacyLibraryDetailActionV1::GetUserVideos,
            Self::SearchDashboardVideos { .. } => {
                LegacyLibraryDetailActionV1::SearchDashboardVideos
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyUserVideoProjectionV1 {
    pub id: String,
    pub owner_id: String,
    pub name: String,
    pub created_at_ms: i64,
    pub metadata: Option<Value>,
    pub is_screenshot: bool,
    pub total_comments: u64,
    pub total_reactions: u64,
    pub owner_name: String,
    pub folder_name: Option<String>,
    pub folder_color: Option<String>,
    pub has_active_upload: bool,
    pub effective_created_at_ms: i64,
}

impl LegacyUserVideoProjectionV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_cap_nanoid(&self.id)
            && valid_cap_nanoid(&self.owner_id)
            && self.name.len() <= 255
            && valid_timestamp(self.created_at_ms)
            && valid_timestamp(self.effective_created_at_ms)
            && self.owner_name.len() <= 255
            && self
                .folder_name
                .as_ref()
                .is_none_or(|value| value.len() <= 255)
            && self
                .folder_color
                .as_deref()
                .is_none_or(|value| matches!(value, "normal" | "blue" | "red" | "yellow"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyDashboardVideoSearchProjectionV1 {
    pub id: String,
    pub name: String,
    pub owner_name: Option<String>,
    pub created_at_ms: i64,
    pub duration_seconds: Option<f64>,
    pub is_screenshot: bool,
    pub effective_created_at_ms: i64,
}

impl LegacyDashboardVideoSearchProjectionV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_cap_nanoid(&self.id)
            && self.name.len() <= 255
            && self
                .owner_name
                .as_ref()
                .is_none_or(|value| value.len() <= 255)
            && valid_timestamp(self.created_at_ms)
            && valid_timestamp(self.effective_created_at_ms)
            && self.duration_seconds.is_none_or(f64::is_finite)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyLibraryDetailResultV1 {
    GetUserVideosSuccess {
        data: Vec<LegacyUserVideoProjectionV1>,
    },
    GetUserVideosFailure,
    SearchDashboardVideos {
        data: Vec<LegacyDashboardVideoSearchProjectionV1>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDashboardVideoQueryV1 {
    pub normalized: String,
    pub contains_pattern: String,
    pub starts_with_pattern: String,
}

impl NormalizedDashboardVideoQueryV1 {
    #[must_use]
    pub fn is_searchable(&self) -> bool {
        self.normalized.encode_utf16().count() >= LEGACY_DASHBOARD_SEARCH_MIN_QUERY_UTF16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyLibraryDetailPortErrorV1 {
    #[error("library scope is not visible")]
    NotVisible,
    #[error("library detail read is unavailable")]
    Unavailable,
    #[error("library detail projection is corrupt")]
    Corrupt,
}

#[async_trait]
pub trait LegacyLibraryDetailReadPortV1: Send + Sync {
    async fn get_user_videos(
        &self,
        principal: &LegacyLibraryDetailPrincipalV1,
        legacy_scope_id: &str,
    ) -> Result<Vec<LegacyUserVideoProjectionV1>, LegacyLibraryDetailPortErrorV1>;

    async fn search_dashboard_videos(
        &self,
        principal: &LegacyLibraryDetailPrincipalV1,
        query: &NormalizedDashboardVideoQueryV1,
    ) -> Result<Vec<LegacyDashboardVideoSearchProjectionV1>, LegacyLibraryDetailPortErrorV1>;
}

pub struct LegacyLibraryDetailReadServiceV1<'a, P> {
    port: &'a P,
}

impl<'a, P> LegacyLibraryDetailReadServiceV1<'a, P>
where
    P: LegacyLibraryDetailReadPortV1,
{
    #[must_use]
    pub const fn new(port: &'a P) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        principal: Option<&LegacyLibraryDetailPrincipalV1>,
        input: &LegacyLibraryDetailInputV1,
    ) -> Result<LegacyLibraryDetailResultV1, LegacyLibraryDetailPortErrorV1> {
        match input {
            LegacyLibraryDetailInputV1::GetUserVideos { legacy_scope_id } => {
                let Some(principal) = principal.filter(|value| value.valid()) else {
                    return Ok(LegacyLibraryDetailResultV1::GetUserVideosFailure);
                };
                if !valid_id(legacy_scope_id) {
                    return Ok(LegacyLibraryDetailResultV1::GetUserVideosFailure);
                }
                match self.port.get_user_videos(principal, legacy_scope_id).await {
                    Ok(data) if data.iter().all(LegacyUserVideoProjectionV1::valid) => {
                        Ok(LegacyLibraryDetailResultV1::GetUserVideosSuccess { data })
                    }
                    Ok(_) | Err(_) => Ok(LegacyLibraryDetailResultV1::GetUserVideosFailure),
                }
            }
            LegacyLibraryDetailInputV1::SearchDashboardVideos { query } => {
                let normalized = normalize_dashboard_video_query(query);
                let Some(principal) = principal.filter(|value| value.valid()) else {
                    return Ok(LegacyLibraryDetailResultV1::SearchDashboardVideos {
                        data: Vec::new(),
                    });
                };
                if !normalized.is_searchable() {
                    return Ok(LegacyLibraryDetailResultV1::SearchDashboardVideos {
                        data: Vec::new(),
                    });
                }
                let data = self
                    .port
                    .search_dashboard_videos(principal, &normalized)
                    .await?;
                if data.len() > LEGACY_DASHBOARD_SEARCH_MAX_RESULTS
                    || !data
                        .iter()
                        .all(LegacyDashboardVideoSearchProjectionV1::valid)
                {
                    return Err(LegacyLibraryDetailPortErrorV1::Corrupt);
                }
                Ok(LegacyLibraryDetailResultV1::SearchDashboardVideos { data })
            }
        }
    }
}

/// Implements ECMAScript's `trim().replace(/\s+/g, " ").slice(0, 80)` for
/// valid Unicode scalar input. If the UTF-16 boundary would split a surrogate
/// pair, the complete scalar is excluded because Rust strings cannot contain
/// an unpaired surrogate.
#[must_use]
pub fn normalize_dashboard_video_query(value: &str) -> NormalizedDashboardVideoQueryV1 {
    let mut collapsed = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if javascript_whitespace(character) {
            if !collapsed.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            collapsed.push(' ');
            pending_space = false;
        }
        collapsed.push(character);
    }
    let mut normalized = String::new();
    let mut utf16_units = 0;
    for character in collapsed.chars() {
        let width = character.len_utf16();
        if utf16_units + width > LEGACY_DASHBOARD_SEARCH_MAX_QUERY_UTF16 {
            break;
        }
        normalized.push(character);
        utf16_units += width;
    }
    let escaped = escape_like_pattern(&normalized);
    NormalizedDashboardVideoQueryV1 {
        normalized,
        contains_pattern: format!("%{escaped}%"),
        starts_with_pattern: format!("{escaped}%"),
    }
}

fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '!' | '%' | '_') {
            escaped.push('!');
        }
        escaped.push(character);
    }
    escaped
}

const fn javascript_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LEGACY_LIBRARY_DETAIL_MAX_ID_BYTES
        && value.is_ascii()
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn valid_cap_nanoid(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

const fn valid_timestamp(value: i64) -> bool {
    value >= 0 && value <= 253_402_300_799_999
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write as _, sync::Mutex};

    use sha2::{Digest, Sha256};

    use super::*;

    #[derive(Default)]
    struct FakePort {
        calls: Mutex<Vec<LegacyLibraryDetailActionV1>>,
        error: Option<LegacyLibraryDetailPortErrorV1>,
    }

    #[async_trait]
    impl LegacyLibraryDetailReadPortV1 for FakePort {
        async fn get_user_videos(
            &self,
            _principal: &LegacyLibraryDetailPrincipalV1,
            _legacy_scope_id: &str,
        ) -> Result<Vec<LegacyUserVideoProjectionV1>, LegacyLibraryDetailPortErrorV1> {
            self.calls
                .lock()
                .expect("calls")
                .push(LegacyLibraryDetailActionV1::GetUserVideos);
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(vec![user_video(2), user_video(1)])
        }

        async fn search_dashboard_videos(
            &self,
            _principal: &LegacyLibraryDetailPrincipalV1,
            query: &NormalizedDashboardVideoQueryV1,
        ) -> Result<Vec<LegacyDashboardVideoSearchProjectionV1>, LegacyLibraryDetailPortErrorV1>
        {
            self.calls
                .lock()
                .expect("calls")
                .push(LegacyLibraryDetailActionV1::SearchDashboardVideos);
            assert_eq!(query.normalized, "100% _ ready");
            assert_eq!(query.contains_pattern, "%100!% !_ ready%");
            if let Some(error) = self.error {
                return Err(error);
            }
            Ok(vec![search_video()])
        }
    }

    fn principal() -> LegacyLibraryDetailPrincipalV1 {
        LegacyLibraryDetailPrincipalV1 {
            actor_id: "actor-uuid".into(),
            active_organization_id: "organization-uuid".into(),
            active_legacy_organization_id: "0123456789abcdf".into(),
        }
    }

    fn user_video(effective: i64) -> LegacyUserVideoProjectionV1 {
        LegacyUserVideoProjectionV1 {
            id: "0123456789abcdf".into(),
            owner_id: "0123456789abcdf".into(),
            name: "Demo".into(),
            created_at_ms: 1,
            metadata: Some(serde_json::json!({"customCreatedAt":"2025-01-01T00:00:00Z"})),
            is_screenshot: false,
            total_comments: 2,
            total_reactions: 1,
            owner_name: String::new(),
            folder_name: None,
            folder_color: None,
            has_active_upload: true,
            effective_created_at_ms: effective,
        }
    }

    fn search_video() -> LegacyDashboardVideoSearchProjectionV1 {
        LegacyDashboardVideoSearchProjectionV1 {
            id: "0123456789abcdf".into(),
            name: "Demo".into(),
            owner_name: None,
            created_at_ms: 1,
            duration_seconds: Some(1.25),
            is_screenshot: false,
            effective_created_at_ms: 2,
        }
    }

    fn manifest(sources: &[LegacyLibraryDetailSourcePinV1]) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame-cap-library-detail-source-manifest-v1\0");
        for source in sources {
            digest.update(source.path.as_bytes());
            digest.update([0]);
            digest.update(source.sha256.as_bytes());
            digest.update([0]);
            digest.update(source.role.stable_code().as_bytes());
            digest.update(b"\n");
        }
        let mut encoded = String::with_capacity(64);
        for byte in digest.finalize() {
            write!(&mut encoded, "{byte:02x}").expect("digest");
        }
        encoded
    }

    #[test]
    fn source_manifests_and_profiles_are_complete_and_provider_free() {
        assert_eq!(LEGACY_LIBRARY_DETAIL_PROFILES.len(), 2);
        assert_eq!(LEGACY_GET_USER_VIDEOS_SOURCES.len(), 15);
        assert_eq!(LEGACY_SEARCH_DASHBOARD_VIDEOS_SOURCES.len(), 11);
        assert_eq!(LEGACY_LIBRARY_DETAIL_NO_PROTECTED_GATES, &[] as &[&str]);
        assert_eq!(
            manifest(LEGACY_GET_USER_VIDEOS_SOURCES),
            LEGACY_GET_USER_VIDEOS_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            manifest(LEGACY_SEARCH_DASHBOARD_VIDEOS_SOURCES),
            LEGACY_SEARCH_DASHBOARD_VIDEOS_SOURCE_MANIFEST_SHA256
        );
    }

    #[test]
    fn normalization_matches_javascript_whitespace_utf16_and_like_escape() {
        let normalized = normalize_dashboard_video_query(" \u{feff}100%\t_\nready  ");
        assert_eq!(normalized.normalized, "100% _ ready");
        assert_eq!(normalized.contains_pattern, "%100!% !_ ready%");
        assert_eq!(normalized.starts_with_pattern, "100!% !_ ready%");
        let astral = normalize_dashboard_video_query(&format!("{}x", "😀".repeat(40)));
        assert_eq!(astral.normalized.encode_utf16().count(), 80);
        assert!(!astral.normalized.ends_with('x'));
    }

    #[tokio::test]
    async fn get_user_videos_preserves_source_catch_all_and_database_order() {
        let port = FakePort::default();
        let service = LegacyLibraryDetailReadServiceV1::new(&port);
        assert_eq!(
            service
                .execute(
                    None,
                    &LegacyLibraryDetailInputV1::GetUserVideos {
                        legacy_scope_id: "scope".into(),
                    },
                )
                .await,
            Ok(LegacyLibraryDetailResultV1::GetUserVideosFailure)
        );
        let result = service
            .execute(
                Some(&principal()),
                &LegacyLibraryDetailInputV1::GetUserVideos {
                    legacy_scope_id: "scope".into(),
                },
            )
            .await
            .expect("source success object");
        let LegacyLibraryDetailResultV1::GetUserVideosSuccess { data } = result else {
            panic!("expected user videos");
        };
        assert_eq!(data[0].effective_created_at_ms, 2);
        assert_eq!(data[1].effective_created_at_ms, 1);

        let failing = FakePort {
            error: Some(LegacyLibraryDetailPortErrorV1::Unavailable),
            ..FakePort::default()
        };
        assert_eq!(
            LegacyLibraryDetailReadServiceV1::new(&failing)
                .execute(
                    Some(&principal()),
                    &LegacyLibraryDetailInputV1::GetUserVideos {
                        legacy_scope_id: "scope".into(),
                    },
                )
                .await,
            Ok(LegacyLibraryDetailResultV1::GetUserVideosFailure)
        );
    }

    #[tokio::test]
    async fn search_short_or_unauthenticated_is_empty_and_failures_propagate() {
        let port = FakePort::default();
        let service = LegacyLibraryDetailReadServiceV1::new(&port);
        for (principal, query) in [(None, "ready"), (Some(principal()), " x ")] {
            assert_eq!(
                service
                    .execute(
                        principal.as_ref(),
                        &LegacyLibraryDetailInputV1::SearchDashboardVideos {
                            query: query.into(),
                        },
                    )
                    .await,
                Ok(LegacyLibraryDetailResultV1::SearchDashboardVideos { data: vec![] })
            );
        }
        assert!(port.calls.lock().expect("calls").is_empty());
        let searched = service
            .execute(
                Some(&principal()),
                &LegacyLibraryDetailInputV1::SearchDashboardVideos {
                    query: " 100%\t_ ready ".into(),
                },
            )
            .await
            .expect("search result");
        assert!(matches!(
            searched,
            LegacyLibraryDetailResultV1::SearchDashboardVideos { data } if data.len() == 1
        ));
    }
}
