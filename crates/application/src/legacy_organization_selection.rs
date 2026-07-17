//! Audited compatibility contract for Cap active-organization selection.
//!
//! The Navbar server action and mobile HTTP route share one active-only write,
//! but they do not share authentication, authorization, error, or output
//! semantics. This module keeps those differences typed and prevents the
//! blocked mobile projection from being promoted with a web-shaped response.

use std::fmt;

use frame_domain::{LegacyCapNanoId, OrganizationId, TimestampMillis, UserId};
use frame_ports::{
    LegacyOrganizationSelectionAuthorizationV1, LegacyOrganizationSelectionLifecycleV1,
    LegacyOrganizationSelectionRepositoryV1, LegacySetActiveOrganizationCommandV1,
    OrganizationPortError,
};
use thiserror::Error;

pub const LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID: &str = "cap-v1-a3b4c805d409bc7c";
pub const LEGACY_WEB_ACTIVE_ORGANIZATION_IDENTITY: &str =
    "action://apps/web/app/(org)/dashboard/_components/Navbar/server.ts#updateActiveOrganization";
pub const LEGACY_MOBILE_ACTIVE_ORGANIZATION_OPERATION_ID: &str = "cap-v1-05776c542380771e";
pub const LEGACY_MOBILE_ACTIVE_ORGANIZATION_PATH: &str = "/api/mobile/user/active-organization";
pub const LEGACY_WEB_DASHBOARD_INVALIDATION_PATH: &str = "/dashboard";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrganizationSelectionSourcePinV1 {
    pub path: &'static str,
    pub sha256: &'static str,
}

pub const LEGACY_WEB_ACTIVE_ORGANIZATION_SOURCES: &[LegacyOrganizationSelectionSourcePinV1] = &[
    LegacyOrganizationSelectionSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/_components/Navbar/server.ts",
        sha256: "a7ea138516eb20f40dad4ad53913e69b01e4f5ad8b2938eb9f5a9a98ab3a29b3",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
];

pub const LEGACY_MOBILE_ACTIVE_ORGANIZATION_SOURCES: &[LegacyOrganizationSelectionSourcePinV1] = &[
    LegacyOrganizationSelectionSourcePinV1 {
        path: "apps/web/app/api/mobile/[...route]/route.ts",
        sha256: "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/web-domain/src/Mobile.ts",
        sha256: "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "apps/web/lib/server.ts",
        sha256: "f24b68bbb31c99ddfa7983a468aa80d293da56d1652e8a0a0a28506e5a9cd63e",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/web-backend/src/Auth.ts",
        sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/web-domain/src/Authentication.ts",
        sha256: "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/web-backend/src/ImageUploads/index.ts",
        sha256: "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/web-backend/src/S3Buckets/index.ts",
        sha256: "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
    },
    LegacyOrganizationSelectionSourcePinV1 {
        path: "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
        sha256: "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionSurfaceV1 {
    WebNavbarServerAction,
    MobilePatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionAuthenticationV1 {
    SessionOnly,
    SessionOrApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionCredentialV1 {
    Session,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionInputV1 {
    BrandedStringArgument,
    JsonOrganizationId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionUnauthenticatedV1 {
    ThrownUnauthorized,
    HttpUnauthorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionTargetDenialV1 {
    ThrownOrganizationNotFound,
    HttpForbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionAuthorityFailureV1 {
    ThrownProviderError,
    HttpInternalServerError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionOutputV1 {
    VoidWithDashboardInvalidation,
    FreshMobileBootstrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionRetryV1 {
    LastWriteWinsWithoutClientIdempotency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileBootstrapBlockerV1 {
    ProviderImageUrlSigning,
    NullableSpaceRootFolders,
}

pub const LEGACY_MOBILE_BOOTSTRAP_BLOCKERS: &[LegacyMobileBootstrapBlockerV1] = &[
    LegacyMobileBootstrapBlockerV1::ProviderImageUrlSigning,
    LegacyMobileBootstrapBlockerV1::NullableSpaceRootFolders,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionProjectionV1 {
    Exact,
    Blocked(&'static [LegacyMobileBootstrapBlockerV1]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrganizationSelectionProfileV1 {
    pub operation_id: &'static str,
    pub kind: &'static str,
    pub method: &'static str,
    pub legacy_identity: &'static str,
    pub sources: &'static [LegacyOrganizationSelectionSourcePinV1],
    pub authentication: LegacyOrganizationSelectionAuthenticationV1,
    pub authorization: LegacyOrganizationSelectionAuthorizationV1,
    pub lifecycle: LegacyOrganizationSelectionLifecycleV1,
    pub input: LegacyOrganizationSelectionInputV1,
    pub unauthenticated: LegacyOrganizationSelectionUnauthenticatedV1,
    pub target_denial: LegacyOrganizationSelectionTargetDenialV1,
    pub authority_failure: LegacyOrganizationSelectionAuthorityFailureV1,
    pub output: LegacyOrganizationSelectionOutputV1,
    pub retry: LegacyOrganizationSelectionRetryV1,
    pub projection: LegacyOrganizationSelectionProjectionV1,
}

pub const LEGACY_WEB_ACTIVE_ORGANIZATION_PROFILE: LegacyOrganizationSelectionProfileV1 =
    LegacyOrganizationSelectionProfileV1 {
        operation_id: LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID,
        kind: "server_action",
        method: "ACTION",
        legacy_identity: LEGACY_WEB_ACTIVE_ORGANIZATION_IDENTITY,
        sources: LEGACY_WEB_ACTIVE_ORGANIZATION_SOURCES,
        authentication: LegacyOrganizationSelectionAuthenticationV1::SessionOnly,
        authorization: LegacyOrganizationSelectionAuthorizationV1::ActiveMembership,
        lifecycle: LegacyOrganizationSelectionLifecycleV1::Any,
        input: LegacyOrganizationSelectionInputV1::BrandedStringArgument,
        unauthenticated: LegacyOrganizationSelectionUnauthenticatedV1::ThrownUnauthorized,
        target_denial: LegacyOrganizationSelectionTargetDenialV1::ThrownOrganizationNotFound,
        authority_failure: LegacyOrganizationSelectionAuthorityFailureV1::ThrownProviderError,
        output: LegacyOrganizationSelectionOutputV1::VoidWithDashboardInvalidation,
        retry: LegacyOrganizationSelectionRetryV1::LastWriteWinsWithoutClientIdempotency,
        projection: LegacyOrganizationSelectionProjectionV1::Exact,
    };

pub const LEGACY_MOBILE_ACTIVE_ORGANIZATION_PROFILE: LegacyOrganizationSelectionProfileV1 =
    LegacyOrganizationSelectionProfileV1 {
        operation_id: LEGACY_MOBILE_ACTIVE_ORGANIZATION_OPERATION_ID,
        kind: "route",
        method: "PATCH",
        legacy_identity: LEGACY_MOBILE_ACTIVE_ORGANIZATION_PATH,
        sources: LEGACY_MOBILE_ACTIVE_ORGANIZATION_SOURCES,
        authentication: LegacyOrganizationSelectionAuthenticationV1::SessionOrApiKey,
        authorization: LegacyOrganizationSelectionAuthorizationV1::OwnerOrActiveMembership,
        lifecycle: LegacyOrganizationSelectionLifecycleV1::ActiveOnly,
        input: LegacyOrganizationSelectionInputV1::JsonOrganizationId,
        unauthenticated: LegacyOrganizationSelectionUnauthenticatedV1::HttpUnauthorized,
        target_denial: LegacyOrganizationSelectionTargetDenialV1::HttpForbidden,
        authority_failure: LegacyOrganizationSelectionAuthorityFailureV1::HttpInternalServerError,
        output: LegacyOrganizationSelectionOutputV1::FreshMobileBootstrap,
        retry: LegacyOrganizationSelectionRetryV1::LastWriteWinsWithoutClientIdempotency,
        projection: LegacyOrganizationSelectionProjectionV1::Blocked(
            LEGACY_MOBILE_BOOTSTRAP_BLOCKERS,
        ),
    };

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyOrganizationSelectionRequestV1 {
    pub credential: Option<LegacyOrganizationSelectionCredentialV1>,
    pub actor_id: Option<UserId>,
    pub legacy_organization_id: String,
    pub occurred_at: TimestampMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionOutcomeV1 {
    /// Internal completion marker. The server-action ingress must consume this,
    /// invalidate the path, and resolve the JavaScript action with `undefined`.
    WebActionVoid { invalidate_path: &'static str },
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyOrganizationSelectionErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Organization not found")]
    OrganizationNotFound,
    #[error("Forbidden")]
    Forbidden,
    #[error("the exact mobile bootstrap projection is unavailable")]
    ProjectionUnavailable(&'static [LegacyMobileBootstrapBlockerV1]),
    #[error("the organization selection authority is unavailable")]
    AuthorityUnavailable,
    #[error("the organization selection authority failed")]
    Internal,
}

impl fmt::Debug for LegacyOrganizationSelectionErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "Unauthorized",
            Self::OrganizationNotFound => "OrganizationNotFound",
            Self::Forbidden => "Forbidden",
            Self::ProjectionUnavailable(_) => "ProjectionUnavailable",
            Self::AuthorityUnavailable => "AuthorityUnavailable",
            Self::Internal => "Internal",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrganizationSelectionAdapterV1 {
    surface: LegacyOrganizationSelectionSurfaceV1,
}

impl LegacyOrganizationSelectionAdapterV1 {
    #[must_use]
    pub const fn web_navbar_server_action() -> Self {
        Self {
            surface: LegacyOrganizationSelectionSurfaceV1::WebNavbarServerAction,
        }
    }

    #[must_use]
    pub const fn mobile_patch() -> Self {
        Self {
            surface: LegacyOrganizationSelectionSurfaceV1::MobilePatch,
        }
    }

    #[must_use]
    pub const fn profile(self) -> &'static LegacyOrganizationSelectionProfileV1 {
        match self.surface {
            LegacyOrganizationSelectionSurfaceV1::WebNavbarServerAction => {
                &LEGACY_WEB_ACTIVE_ORGANIZATION_PROFILE
            }
            LegacyOrganizationSelectionSurfaceV1::MobilePatch => {
                &LEGACY_MOBILE_ACTIVE_ORGANIZATION_PROFILE
            }
        }
    }

    /// Validate the surface credential and deterministically map the Cap
    /// NanoID into the UUID used by the Frame migration. Invalid arbitrary
    /// strings intentionally collapse into the surface's target-denial error,
    /// matching the source query/schema behavior without disclosing existence.
    pub fn prepare_mutation(
        self,
        request: &LegacyOrganizationSelectionRequestV1,
    ) -> Result<LegacySetActiveOrganizationCommandV1, LegacyOrganizationSelectionErrorV1> {
        let profile = self.profile();
        let Some(actor_id) = request.actor_id else {
            return Err(LegacyOrganizationSelectionErrorV1::Unauthorized);
        };
        let credential_allowed = matches!(
            (profile.authentication, request.credential),
            (
                LegacyOrganizationSelectionAuthenticationV1::SessionOnly,
                Some(LegacyOrganizationSelectionCredentialV1::Session)
            ) | (
                LegacyOrganizationSelectionAuthenticationV1::SessionOrApiKey,
                Some(
                    LegacyOrganizationSelectionCredentialV1::Session
                        | LegacyOrganizationSelectionCredentialV1::ApiKey
                )
            )
        );
        if !credential_allowed {
            return Err(LegacyOrganizationSelectionErrorV1::Unauthorized);
        }
        let legacy_id = LegacyCapNanoId::parse(request.legacy_organization_id.clone())
            .map_err(|_| self.target_denial())?;
        let active_organization_id = OrganizationId::parse(&legacy_id.mapped_uuid().to_string())
            .map_err(|_| LegacyOrganizationSelectionErrorV1::Internal)?;
        Ok(LegacySetActiveOrganizationCommandV1 {
            actor_id,
            active_organization_id,
            authorization: profile.authorization,
            lifecycle: profile.lifecycle,
            occurred_at: request.occurred_at,
        })
    }

    pub async fn execute<Repository>(
        self,
        repository: &Repository,
        request: &LegacyOrganizationSelectionRequestV1,
    ) -> Result<LegacyOrganizationSelectionOutcomeV1, LegacyOrganizationSelectionErrorV1>
    where
        Repository: LegacyOrganizationSelectionRepositoryV1,
    {
        let command = self.prepare_mutation(request)?;
        if let LegacyOrganizationSelectionProjectionV1::Blocked(blockers) =
            self.profile().projection
        {
            // Never perform the write when the route cannot return the exact
            // post-write bootstrap. This adapter remains fail-closed and must
            // not be placed in a promoted runtime allowlist.
            return Err(LegacyOrganizationSelectionErrorV1::ProjectionUnavailable(
                blockers,
            ));
        }
        repository
            .legacy_set_active_organization(command)
            .await
            .map_err(|error| self.repository_error(error))?;
        Ok(LegacyOrganizationSelectionOutcomeV1::WebActionVoid {
            invalidate_path: LEGACY_WEB_DASHBOARD_INVALIDATION_PATH,
        })
    }

    fn target_denial(self) -> LegacyOrganizationSelectionErrorV1 {
        match self.profile().target_denial {
            LegacyOrganizationSelectionTargetDenialV1::ThrownOrganizationNotFound => {
                LegacyOrganizationSelectionErrorV1::OrganizationNotFound
            }
            LegacyOrganizationSelectionTargetDenialV1::HttpForbidden => {
                LegacyOrganizationSelectionErrorV1::Forbidden
            }
        }
    }

    fn repository_error(self, error: OrganizationPortError) -> LegacyOrganizationSelectionErrorV1 {
        match error {
            OrganizationPortError::AccessDenied | OrganizationPortError::StaleAuthority => {
                self.target_denial()
            }
            OrganizationPortError::Unavailable => {
                LegacyOrganizationSelectionErrorV1::AuthorityUnavailable
            }
            OrganizationPortError::Conflict
            | OrganizationPortError::Invalid
            | OrganizationPortError::RetentionLocked
            | OrganizationPortError::Corrupt => LegacyOrganizationSelectionErrorV1::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use frame_domain::{OrganizationOperationId, OrganizationRevision};
    use frame_ports::{OrganizationMutationReceipt, OrganizationMutationResult};

    use super::*;

    struct RecordingRepository {
        commands: Mutex<Vec<LegacySetActiveOrganizationCommandV1>>,
        result: Result<(), OrganizationPortError>,
    }

    impl RecordingRepository {
        fn successful() -> Self {
            Self {
                commands: Mutex::new(Vec::new()),
                result: Ok(()),
            }
        }
    }

    #[async_trait]
    impl LegacyOrganizationSelectionRepositoryV1 for RecordingRepository {
        async fn legacy_set_active_organization(
            &self,
            command: LegacySetActiveOrganizationCommandV1,
        ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
            self.commands.lock().expect("commands").push(command);
            self.result?;
            Ok(OrganizationMutationReceipt {
                operation_id: OrganizationOperationId::new(),
                result: OrganizationMutationResult::Applied,
                subject_id: command.actor_id.to_string(),
                committed_at: command.occurred_at,
                resulting_revision: OrganizationRevision::new(1).expect("revision"),
                authority_version: OrganizationRevision::INITIAL,
                replayed: false,
            })
        }
    }

    fn actor() -> UserId {
        UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a8").expect("actor")
    }

    fn request(
        credential: LegacyOrganizationSelectionCredentialV1,
    ) -> LegacyOrganizationSelectionRequestV1 {
        LegacyOrganizationSelectionRequestV1 {
            credential: Some(credential),
            actor_id: Some(actor()),
            legacy_organization_id: "0123456789abcde".into(),
            occurred_at: TimestampMillis::new(1_700_000_000_000).expect("timestamp"),
        }
    }

    #[test]
    fn profiles_pin_the_authority_and_output_asymmetry() {
        let web = LegacyOrganizationSelectionAdapterV1::web_navbar_server_action().profile();
        assert_eq!(
            web.authentication,
            LegacyOrganizationSelectionAuthenticationV1::SessionOnly
        );
        assert_eq!(
            web.authorization,
            LegacyOrganizationSelectionAuthorizationV1::ActiveMembership
        );
        assert_eq!(web.lifecycle, LegacyOrganizationSelectionLifecycleV1::Any);
        assert_eq!(
            web.target_denial,
            LegacyOrganizationSelectionTargetDenialV1::ThrownOrganizationNotFound
        );
        assert_eq!(
            web.output,
            LegacyOrganizationSelectionOutputV1::VoidWithDashboardInvalidation
        );
        assert_eq!(
            web.projection,
            LegacyOrganizationSelectionProjectionV1::Exact
        );

        let mobile = LegacyOrganizationSelectionAdapterV1::mobile_patch().profile();
        assert_eq!(
            mobile.authentication,
            LegacyOrganizationSelectionAuthenticationV1::SessionOrApiKey
        );
        assert_eq!(
            mobile.authorization,
            LegacyOrganizationSelectionAuthorizationV1::OwnerOrActiveMembership
        );
        assert_eq!(
            mobile.lifecycle,
            LegacyOrganizationSelectionLifecycleV1::ActiveOnly
        );
        assert_eq!(
            mobile.target_denial,
            LegacyOrganizationSelectionTargetDenialV1::HttpForbidden
        );
        assert_eq!(
            mobile.output,
            LegacyOrganizationSelectionOutputV1::FreshMobileBootstrap
        );
        assert_eq!(
            mobile.projection,
            LegacyOrganizationSelectionProjectionV1::Blocked(LEGACY_MOBILE_BOOTSTRAP_BLOCKERS)
        );
    }

    #[test]
    fn source_pins_are_complete_sha256_values() {
        for pin in LEGACY_WEB_ACTIVE_ORGANIZATION_SOURCES
            .iter()
            .chain(LEGACY_MOBILE_ACTIVE_ORGANIZATION_SOURCES)
        {
            assert!(!pin.path.is_empty());
            assert_eq!(pin.sha256.len(), 64);
            assert!(
                pin.sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
        }
    }

    #[test]
    fn cap_target_is_mapped_and_client_cas_fields_do_not_exist() {
        let command = LegacyOrganizationSelectionAdapterV1::web_navbar_server_action()
            .prepare_mutation(&request(LegacyOrganizationSelectionCredentialV1::Session))
            .expect("command");
        assert_eq!(
            command.active_organization_id.to_string(),
            "2a6a8a87-d5ca-8c83-8666-2e92c2a69404"
        );
        assert_eq!(
            command.authorization,
            LegacyOrganizationSelectionAuthorizationV1::ActiveMembership
        );
        assert_eq!(
            command.lifecycle,
            LegacyOrganizationSelectionLifecycleV1::Any
        );
    }

    #[test]
    fn malformed_targets_collapse_to_each_surfaces_denial() {
        let mut web_request = request(LegacyOrganizationSelectionCredentialV1::Session);
        web_request.legacy_organization_id = "arbitrary-string".into();
        assert_eq!(
            LegacyOrganizationSelectionAdapterV1::web_navbar_server_action()
                .prepare_mutation(&web_request),
            Err(LegacyOrganizationSelectionErrorV1::OrganizationNotFound)
        );
        let mut mobile_request = request(LegacyOrganizationSelectionCredentialV1::ApiKey);
        mobile_request.legacy_organization_id = "arbitrary-string".into();
        assert_eq!(
            LegacyOrganizationSelectionAdapterV1::mobile_patch().prepare_mutation(&mobile_request),
            Err(LegacyOrganizationSelectionErrorV1::Forbidden)
        );
    }

    #[tokio::test]
    async fn web_executes_once_and_returns_only_the_void_action_marker() {
        let repository = RecordingRepository::successful();
        let outcome = LegacyOrganizationSelectionAdapterV1::web_navbar_server_action()
            .execute(
                &repository,
                &request(LegacyOrganizationSelectionCredentialV1::Session),
            )
            .await
            .expect("selection");
        assert_eq!(
            outcome,
            LegacyOrganizationSelectionOutcomeV1::WebActionVoid {
                invalidate_path: "/dashboard"
            }
        );
        assert_eq!(repository.commands.lock().expect("commands").len(), 1);
    }

    #[tokio::test]
    async fn mobile_projection_blocks_before_any_mutation() {
        let repository = RecordingRepository::successful();
        let error = LegacyOrganizationSelectionAdapterV1::mobile_patch()
            .execute(
                &repository,
                &request(LegacyOrganizationSelectionCredentialV1::ApiKey),
            )
            .await
            .expect_err("mobile projection must remain blocked");
        assert_eq!(
            error,
            LegacyOrganizationSelectionErrorV1::ProjectionUnavailable(
                LEGACY_MOBILE_BOOTSTRAP_BLOCKERS
            )
        );
        assert!(repository.commands.lock().expect("commands").is_empty());
    }

    #[test]
    fn web_rejects_api_keys_while_mobile_accepts_them() {
        assert_eq!(
            LegacyOrganizationSelectionAdapterV1::web_navbar_server_action()
                .prepare_mutation(&request(LegacyOrganizationSelectionCredentialV1::ApiKey)),
            Err(LegacyOrganizationSelectionErrorV1::Unauthorized)
        );
        assert!(
            LegacyOrganizationSelectionAdapterV1::mobile_patch()
                .prepare_mutation(&request(LegacyOrganizationSelectionCredentialV1::ApiKey))
                .is_ok()
        );
    }
}
