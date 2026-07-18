//! Source-pinned contracts for Cap's three library membership ID reads.
//!
//! Cap's actions require only a browser session and then query caller-supplied
//! folder, organization, or space IDs without an authorization predicate.
//! Frame preserves the successful result and failure envelopes but closes that
//! cross-tenant disclosure: the D1 port must prove the actor's live active
//! organization and the requested scope before returning any legacy video ID.

use async_trait::async_trait;
use thiserror::Error;

pub const LEGACY_LIBRARY_ID_READ_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_GET_FOLDER_VIDEO_IDS_OPERATION_ID: &str = "cap-v1-b1027c7caafb92e2";
pub const LEGACY_GET_ORGANIZATION_VIDEO_IDS_OPERATION_ID: &str = "cap-v1-cc52545598164806";
pub const LEGACY_GET_SPACE_VIDEO_IDS_OPERATION_ID: &str = "cap-v1-a8ace95c6ab712f6";
pub const LEGACY_GET_FOLDER_VIDEO_IDS_IDENTITY: &str =
    "action://apps/web/actions/folders/get-folder-videos.ts#getFolderVideoIds";
pub const LEGACY_GET_ORGANIZATION_VIDEO_IDS_IDENTITY: &str =
    "action://apps/web/actions/organizations/get-organization-videos.ts#getOrganizationVideoIds";
pub const LEGACY_GET_SPACE_VIDEO_IDS_IDENTITY: &str =
    "action://apps/web/actions/spaces/get-space-videos.ts#getSpaceVideoIds";
pub const LEGACY_LIBRARY_ID_READ_POLICY: &str = "organization_library.v1";
pub const LEGACY_LIBRARY_ID_READ_MAX_BODY_BYTES: usize = 1024;
pub const LEGACY_LIBRARY_ID_READ_MAX_ID_BYTES: usize = 256;
pub const LEGACY_LIBRARY_ID_READ_NO_PROTECTED_GATES: &[&str] = &[];

pub const LEGACY_GET_FOLDER_VIDEO_IDS_SOURCE_MANIFEST_SHA256: &str =
    "bab8979189df2978ae381303e209a0b4ee5832e3fba5949761976ab7e3b5d41b";
pub const LEGACY_GET_ORGANIZATION_VIDEO_IDS_SOURCE_MANIFEST_SHA256: &str =
    "cef5fc71ef1b95f9747d010bd3a04a744d33200f3d4427fb1a16fbc4fe72722b";
pub const LEGACY_GET_SPACE_VIDEO_IDS_SOURCE_MANIFEST_SHA256: &str =
    "0bf24570c914feabd6be1be0f83ff7c3567059e86098fc6f4e1ce0f5a948dcb4";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryIdReadSourceRoleV1 {
    Action,
    Caller,
    Authentication,
    Schema,
    Database,
    Identifier,
    DependencyLock,
}

impl LegacyLibraryIdReadSourceRoleV1 {
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
pub struct LegacyLibraryIdReadSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyLibraryIdReadSourceRoleV1,
}

const SESSION_SOURCE: LegacyLibraryIdReadSourcePinV1 = LegacyLibraryIdReadSourcePinV1 {
    path: "packages/database/auth/session.ts",
    symbol: "getCurrentUser",
    sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    role: LegacyLibraryIdReadSourceRoleV1::Authentication,
};
const AUTH_OPTIONS_SOURCE: LegacyLibraryIdReadSourcePinV1 = LegacyLibraryIdReadSourcePinV1 {
    path: "packages/database/auth/auth-options.ts",
    symbol: "authOptions+session callback",
    sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    role: LegacyLibraryIdReadSourceRoleV1::Authentication,
};
const SCHEMA_SOURCE: LegacyLibraryIdReadSourcePinV1 = LegacyLibraryIdReadSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "users+folders+sharedVideos+spaces+spaceVideos+videos",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    role: LegacyLibraryIdReadSourceRoleV1::Schema,
};
const DATABASE_SOURCE: LegacyLibraryIdReadSourcePinV1 = LegacyLibraryIdReadSourcePinV1 {
    path: "packages/database/index.ts",
    symbol: "db",
    sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
    role: LegacyLibraryIdReadSourceRoleV1::Database,
};
const SPACE_ID_SOURCE: LegacyLibraryIdReadSourcePinV1 = LegacyLibraryIdReadSourcePinV1 {
    path: "packages/web-domain/src/Space.ts",
    symbol: "SpaceIdOrOrganisationId",
    sha256: "ad9cb2ae26767bebf00640846bce4cab6feee6a6308ac0d7b068cd6e006542c3",
    role: LegacyLibraryIdReadSourceRoleV1::Identifier,
};
const VIDEO_ID_SOURCE: LegacyLibraryIdReadSourcePinV1 = LegacyLibraryIdReadSourcePinV1 {
    path: "packages/web-domain/src/Video.ts",
    symbol: "VideoId",
    sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    role: LegacyLibraryIdReadSourceRoleV1::Identifier,
};
const LOCK_SOURCE: LegacyLibraryIdReadSourcePinV1 = LegacyLibraryIdReadSourcePinV1 {
    path: "pnpm-lock.yaml",
    symbol: "drizzle-orm+mysql2+next-auth resolutions",
    sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
    role: LegacyLibraryIdReadSourceRoleV1::DependencyLock,
};

pub const LEGACY_GET_FOLDER_VIDEO_IDS_SOURCES: &[LegacyLibraryIdReadSourcePinV1] = &[
    LegacyLibraryIdReadSourcePinV1 {
        path: "apps/web/actions/folders/get-folder-videos.ts",
        symbol: "getFolderVideoIds",
        sha256: "ff36b7e0c86d6dbb44096b342c2beaf8d3b50e31924c6ac7b41681b7e2f47d43",
        role: LegacyLibraryIdReadSourceRoleV1::Action,
    },
    LegacyLibraryIdReadSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/folder/[folderId]/AddVideosButton.tsx",
        symbol: "getEntityVideoIds",
        sha256: "a526a81701c68b9d76367817164fcd0b72e7e3c930d89de03f782c6f388ff871",
        role: LegacyLibraryIdReadSourceRoleV1::Caller,
    },
    SESSION_SOURCE,
    AUTH_OPTIONS_SOURCE,
    SCHEMA_SOURCE,
    DATABASE_SOURCE,
    LegacyLibraryIdReadSourcePinV1 {
        path: "packages/web-domain/src/Folder.ts",
        symbol: "FolderId",
        sha256: "4201376991878efc79979f77901908d542573f5b0f9e1ca6b6b246e04d881e9e",
        role: LegacyLibraryIdReadSourceRoleV1::Identifier,
    },
    SPACE_ID_SOURCE,
    VIDEO_ID_SOURCE,
    LOCK_SOURCE,
];

pub const LEGACY_GET_ORGANIZATION_VIDEO_IDS_SOURCES: &[LegacyLibraryIdReadSourcePinV1] = &[
    LegacyLibraryIdReadSourcePinV1 {
        path: "apps/web/actions/organizations/get-organization-videos.ts",
        symbol: "getOrganizationVideoIds",
        sha256: "96bf7670e2f5b8664c4cfc31b71faf9f10a3558f0460d3ffbd3b5f81c70b16d0",
        role: LegacyLibraryIdReadSourceRoleV1::Action,
    },
    LegacyLibraryIdReadSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosToOrganizationDialog.tsx",
        symbol: "getEntityVideoIds",
        sha256: "c8bc5ef4dc2cc0dc8f452d2769be9c3be49e8be6204cb5d9c2b9bdd0d327efd7",
        role: LegacyLibraryIdReadSourceRoleV1::Caller,
    },
    SESSION_SOURCE,
    AUTH_OPTIONS_SOURCE,
    SCHEMA_SOURCE,
    DATABASE_SOURCE,
    LegacyLibraryIdReadSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        symbol: "OrganisationId",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
        role: LegacyLibraryIdReadSourceRoleV1::Identifier,
    },
    VIDEO_ID_SOURCE,
    LOCK_SOURCE,
];

pub const LEGACY_GET_SPACE_VIDEO_IDS_SOURCES: &[LegacyLibraryIdReadSourcePinV1] = &[
    LegacyLibraryIdReadSourcePinV1 {
        path: "apps/web/actions/spaces/get-space-videos.ts",
        symbol: "getSpaceVideoIds",
        sha256: "a1968a5dbf067c86a8146df3240cb8d44ce120c0508324fb44fcc82d698c7da0",
        role: LegacyLibraryIdReadSourceRoleV1::Action,
    },
    LegacyLibraryIdReadSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/spaces/[spaceId]/components/AddVideosDialog.tsx",
        symbol: "getEntityVideoIds",
        sha256: "238104cd063757bb8bf785f94acf5c75ccb7b9ef14b7ef519636925b091a9201",
        role: LegacyLibraryIdReadSourceRoleV1::Caller,
    },
    SESSION_SOURCE,
    AUTH_OPTIONS_SOURCE,
    SCHEMA_SOURCE,
    DATABASE_SOURCE,
    SPACE_ID_SOURCE,
    VIDEO_ID_SOURCE,
    LOCK_SOURCE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyLibraryIdReadActionV1 {
    Folder,
    Organization,
    Space,
}

impl LegacyLibraryIdReadActionV1 {
    #[must_use]
    pub fn parse(operation_id: &str) -> Option<Self> {
        match operation_id {
            LEGACY_GET_FOLDER_VIDEO_IDS_OPERATION_ID => Some(Self::Folder),
            LEGACY_GET_ORGANIZATION_VIDEO_IDS_OPERATION_ID => Some(Self::Organization),
            LEGACY_GET_SPACE_VIDEO_IDS_OPERATION_ID => Some(Self::Space),
            _ => None,
        }
    }

    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::Folder => LEGACY_GET_FOLDER_VIDEO_IDS_OPERATION_ID,
            Self::Organization => LEGACY_GET_ORGANIZATION_VIDEO_IDS_OPERATION_ID,
            Self::Space => LEGACY_GET_SPACE_VIDEO_IDS_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn identity(self) -> &'static str {
        match self {
            Self::Folder => LEGACY_GET_FOLDER_VIDEO_IDS_IDENTITY,
            Self::Organization => LEGACY_GET_ORGANIZATION_VIDEO_IDS_IDENTITY,
            Self::Space => LEGACY_GET_SPACE_VIDEO_IDS_IDENTITY,
        }
    }

    #[must_use]
    pub const fn required_input_failure(self) -> &'static str {
        match self {
            Self::Folder => "Folder ID is required",
            Self::Organization => "Organization ID is required",
            Self::Space => "Space ID is required",
        }
    }

    #[must_use]
    pub const fn stable_read_failure(self) -> &'static str {
        match self {
            Self::Folder => "Failed to fetch folder videos",
            Self::Organization => "Failed to fetch organization videos",
            Self::Space => "Failed to fetch space videos",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyLibraryIdReadProfileV1 {
    pub action: LegacyLibraryIdReadActionV1,
    pub auth: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_LIBRARY_ID_READ_PROFILES: &[LegacyLibraryIdReadProfileV1] = &[
    LegacyLibraryIdReadProfileV1 {
        action: LegacyLibraryIdReadActionV1::Folder,
        auth: "host_only_browser_session",
        success: "source_success_true_and_unordered_legacy_video_id_array",
        validation: "nonempty_bounded_folder_id_and_bounded_scope_id",
        authorization: "actor_active_organization_and_exact_folder_namespace",
        idempotency: "read_only_client_key_forbidden_retry_safe",
        failure: "source_failure_object_with_stable_non_driver_message",
    },
    LegacyLibraryIdReadProfileV1 {
        action: LegacyLibraryIdReadActionV1::Organization,
        auth: "host_only_browser_session",
        success: "source_success_true_and_unordered_root_legacy_video_id_array",
        validation: "nonempty_bounded_organization_id",
        authorization: "requested_organization_must_be_actor_active_organization",
        idempotency: "read_only_client_key_forbidden_retry_safe",
        failure: "source_failure_object_with_stable_non_driver_message",
    },
    LegacyLibraryIdReadProfileV1 {
        action: LegacyLibraryIdReadActionV1::Space,
        auth: "host_only_browser_session",
        success: "source_success_true_and_unordered_root_legacy_video_id_array",
        validation: "nonempty_bounded_space_or_organization_id",
        authorization: "actor_active_organization_and_live_space_access",
        idempotency: "read_only_client_key_forbidden_retry_safe",
        failure: "source_failure_object_with_stable_non_driver_message",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyLibraryIdReadPrincipalV1 {
    pub actor_id: String,
    pub active_organization_id: String,
    pub active_legacy_organization_id: String,
}

impl LegacyLibraryIdReadPrincipalV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_id(&self.actor_id)
            && valid_id(&self.active_organization_id)
            && valid_id(&self.active_legacy_organization_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyLibraryIdReadInputV1 {
    Folder {
        legacy_folder_id: String,
        legacy_space_or_organization_id: String,
    },
    Organization {
        legacy_organization_id: String,
    },
    Space {
        legacy_space_or_organization_id: String,
    },
}

impl LegacyLibraryIdReadInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyLibraryIdReadActionV1 {
        match self {
            Self::Folder { .. } => LegacyLibraryIdReadActionV1::Folder,
            Self::Organization { .. } => LegacyLibraryIdReadActionV1::Organization,
            Self::Space { .. } => LegacyLibraryIdReadActionV1::Space,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyLibraryIdReadResultV1 {
    Success { data: Vec<String> },
    Failure { error: &'static str },
}

impl LegacyLibraryIdReadResultV1 {
    #[must_use]
    pub const fn success(&self) -> bool {
        matches!(self, Self::Success { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyLibraryIdReadPortErrorV1 {
    #[error("scope is not visible to this actor")]
    NotVisible,
    #[error("library read authority is unavailable")]
    Unavailable,
    #[error("library read projection is corrupt")]
    Corrupt,
}

#[async_trait]
pub trait LegacyLibraryIdReadPortV1: Send + Sync {
    async fn folder_video_ids(
        &self,
        principal: &LegacyLibraryIdReadPrincipalV1,
        legacy_folder_id: &str,
        legacy_space_or_organization_id: &str,
    ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1>;

    async fn organization_video_ids(
        &self,
        principal: &LegacyLibraryIdReadPrincipalV1,
        legacy_organization_id: &str,
    ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1>;

    async fn space_video_ids(
        &self,
        principal: &LegacyLibraryIdReadPrincipalV1,
        legacy_space_or_organization_id: &str,
    ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1>;
}

pub struct LegacyLibraryIdReadServiceV1<'a, P> {
    port: &'a P,
}

impl<'a, P> LegacyLibraryIdReadServiceV1<'a, P>
where
    P: LegacyLibraryIdReadPortV1,
{
    #[must_use]
    pub const fn new(port: &'a P) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        principal: Option<&LegacyLibraryIdReadPrincipalV1>,
        input: &LegacyLibraryIdReadInputV1,
    ) -> LegacyLibraryIdReadResultV1 {
        let action = input.action();
        let Some(principal) = principal.filter(|principal| principal.valid()) else {
            return LegacyLibraryIdReadResultV1::Failure {
                error: "Unauthorized",
            };
        };
        if !valid_input(input) {
            return LegacyLibraryIdReadResultV1::Failure {
                error: action.required_input_failure(),
            };
        }
        let result = match input {
            LegacyLibraryIdReadInputV1::Folder {
                legacy_folder_id,
                legacy_space_or_organization_id,
            } => {
                self.port
                    .folder_video_ids(principal, legacy_folder_id, legacy_space_or_organization_id)
                    .await
            }
            LegacyLibraryIdReadInputV1::Organization {
                legacy_organization_id,
            } => {
                self.port
                    .organization_video_ids(principal, legacy_organization_id)
                    .await
            }
            LegacyLibraryIdReadInputV1::Space {
                legacy_space_or_organization_id,
            } => {
                self.port
                    .space_video_ids(principal, legacy_space_or_organization_id)
                    .await
            }
        };
        match result {
            Ok(data) if data.iter().all(|value| valid_id(value)) => {
                LegacyLibraryIdReadResultV1::Success { data }
            }
            Ok(_) | Err(_) => LegacyLibraryIdReadResultV1::Failure {
                error: action.stable_read_failure(),
            },
        }
    }
}

fn valid_input(input: &LegacyLibraryIdReadInputV1) -> bool {
    match input {
        LegacyLibraryIdReadInputV1::Folder {
            legacy_folder_id,
            legacy_space_or_organization_id,
        } => valid_id(legacy_folder_id) && valid_id(legacy_space_or_organization_id),
        LegacyLibraryIdReadInputV1::Organization {
            legacy_organization_id,
        } => valid_id(legacy_organization_id),
        LegacyLibraryIdReadInputV1::Space {
            legacy_space_or_organization_id,
        } => valid_id(legacy_space_or_organization_id),
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= LEGACY_LIBRARY_ID_READ_MAX_ID_BYTES
}

#[cfg(test)]
mod tests {
    use std::{fmt::Write as _, sync::Mutex};

    use sha2::{Digest, Sha256};

    use super::*;

    #[derive(Debug, Default)]
    struct FakePort {
        calls: Mutex<Vec<LegacyLibraryIdReadActionV1>>,
        error: Option<LegacyLibraryIdReadPortErrorV1>,
    }

    impl FakePort {
        fn failing(error: LegacyLibraryIdReadPortErrorV1) -> Self {
            Self {
                calls: Mutex::default(),
                error: Some(error),
            }
        }
    }

    #[async_trait]
    impl LegacyLibraryIdReadPortV1 for FakePort {
        async fn folder_video_ids(
            &self,
            _principal: &LegacyLibraryIdReadPrincipalV1,
            _legacy_folder_id: &str,
            _legacy_space_or_organization_id: &str,
        ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1> {
            self.calls
                .lock()
                .expect("calls")
                .push(LegacyLibraryIdReadActionV1::Folder);
            self.error
                .map_or_else(|| Ok(vec!["video-b".into(), "video-a".into()]), Err)
        }

        async fn organization_video_ids(
            &self,
            _principal: &LegacyLibraryIdReadPrincipalV1,
            _legacy_organization_id: &str,
        ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1> {
            self.calls
                .lock()
                .expect("calls")
                .push(LegacyLibraryIdReadActionV1::Organization);
            self.error.map_or_else(|| Ok(vec!["video-a".into()]), Err)
        }

        async fn space_video_ids(
            &self,
            _principal: &LegacyLibraryIdReadPrincipalV1,
            _legacy_space_or_organization_id: &str,
        ) -> Result<Vec<String>, LegacyLibraryIdReadPortErrorV1> {
            self.calls
                .lock()
                .expect("calls")
                .push(LegacyLibraryIdReadActionV1::Space);
            self.error.map_or_else(|| Ok(Vec::new()), Err)
        }
    }

    fn principal() -> LegacyLibraryIdReadPrincipalV1 {
        LegacyLibraryIdReadPrincipalV1 {
            actor_id: "actor".into(),
            active_organization_id: "organization".into(),
            active_legacy_organization_id: "legacy-organization".into(),
        }
    }

    fn manifest(sources: &[LegacyLibraryIdReadSourcePinV1]) -> String {
        let mut digest = Sha256::new();
        digest.update(b"frame-cap-library-id-read-source-manifest-v1\0");
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
    fn profiles_pin_all_three_distinct_actions_and_security_closure() {
        assert_eq!(LEGACY_LIBRARY_ID_READ_PROFILES.len(), 3);
        assert_eq!(LEGACY_LIBRARY_ID_READ_NO_PROTECTED_GATES, &[] as &[&str]);
        for action in [
            LegacyLibraryIdReadActionV1::Folder,
            LegacyLibraryIdReadActionV1::Organization,
            LegacyLibraryIdReadActionV1::Space,
        ] {
            assert_eq!(
                LegacyLibraryIdReadActionV1::parse(action.operation_id()),
                Some(action)
            );
            assert!(action.identity().starts_with("action://"));
        }
        assert_eq!(
            manifest(LEGACY_GET_FOLDER_VIDEO_IDS_SOURCES),
            LEGACY_GET_FOLDER_VIDEO_IDS_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            manifest(LEGACY_GET_ORGANIZATION_VIDEO_IDS_SOURCES),
            LEGACY_GET_ORGANIZATION_VIDEO_IDS_SOURCE_MANIFEST_SHA256
        );
        assert_eq!(
            manifest(LEGACY_GET_SPACE_VIDEO_IDS_SOURCES),
            LEGACY_GET_SPACE_VIDEO_IDS_SOURCE_MANIFEST_SHA256
        );
    }

    #[tokio::test]
    async fn missing_session_and_empty_required_ids_preserve_source_failure_objects() {
        let port = FakePort::default();
        let service = LegacyLibraryIdReadServiceV1::new(&port);
        let input = LegacyLibraryIdReadInputV1::Organization {
            legacy_organization_id: "organization".into(),
        };
        assert_eq!(
            service.execute(None, &input).await,
            LegacyLibraryIdReadResultV1::Failure {
                error: "Unauthorized"
            }
        );
        assert_eq!(
            service
                .execute(
                    Some(&principal()),
                    &LegacyLibraryIdReadInputV1::Space {
                        legacy_space_or_organization_id: String::new(),
                    },
                )
                .await,
            LegacyLibraryIdReadResultV1::Failure {
                error: "Space ID is required"
            }
        );
        assert!(port.calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn source_order_is_not_fabricated_for_unordered_id_queries() {
        let port = FakePort::default();
        let service = LegacyLibraryIdReadServiceV1::new(&port);
        let result = service
            .execute(
                Some(&principal()),
                &LegacyLibraryIdReadInputV1::Folder {
                    legacy_folder_id: "folder".into(),
                    legacy_space_or_organization_id: "space".into(),
                },
            )
            .await;
        assert_eq!(
            result,
            LegacyLibraryIdReadResultV1::Success {
                data: vec!["video-b".into(), "video-a".into()]
            }
        );
    }

    #[tokio::test]
    async fn authority_and_storage_failures_are_stable_and_non_disclosing() {
        for error in [
            LegacyLibraryIdReadPortErrorV1::NotVisible,
            LegacyLibraryIdReadPortErrorV1::Unavailable,
            LegacyLibraryIdReadPortErrorV1::Corrupt,
        ] {
            let port = FakePort::failing(error);
            let result = LegacyLibraryIdReadServiceV1::new(&port)
                .execute(
                    Some(&principal()),
                    &LegacyLibraryIdReadInputV1::Organization {
                        legacy_organization_id: "organization".into(),
                    },
                )
                .await;
            assert_eq!(
                result,
                LegacyLibraryIdReadResultV1::Failure {
                    error: "Failed to fetch organization videos"
                }
            );
        }
    }

    #[tokio::test]
    async fn oversized_input_and_corrupt_output_never_cross_the_boundary() {
        let port = FakePort::default();
        let result = LegacyLibraryIdReadServiceV1::new(&port)
            .execute(
                Some(&principal()),
                &LegacyLibraryIdReadInputV1::Folder {
                    legacy_folder_id: "x".repeat(LEGACY_LIBRARY_ID_READ_MAX_ID_BYTES + 1),
                    legacy_space_or_organization_id: "scope".into(),
                },
            )
            .await;
        assert_eq!(
            result,
            LegacyLibraryIdReadResultV1::Failure {
                error: "Folder ID is required"
            }
        );
        assert!(port.calls.lock().expect("calls").is_empty());
    }
}
