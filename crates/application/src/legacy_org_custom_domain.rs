//! Source-pinned contracts for Cap's organization custom-domain reads.
//!
//! Cap exposes two identities with superficially similar names but different
//! evidence. The desktop Hono handler is concrete and returns the Drizzle
//! timestamp as an ISO string. The `/api` identity exists only in ts-rest and
//! Effect declarations and claims a nullable boolean. Keeping those contracts
//! separate prevents a declaration from silently changing the proven desktop
//! wire value or being mistaken for executable source behavior.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

pub const LEGACY_ORG_CUSTOM_DOMAIN_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_OPERATION_ID: &str = "cap-v1-ed9957ac480103b9";
pub const LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_PATH: &str = "/api/desktop/org-custom-domain";
pub const LEGACY_WEB_ORG_CUSTOM_DOMAIN_OPERATION_ID: &str = "cap-v1-9323d0178c5a63b5";
pub const LEGACY_WEB_ORG_CUSTOM_DOMAIN_PATH: &str = "/api/org-custom-domain";
pub const LEGACY_ORG_CUSTOM_DOMAIN_POLICY: &str = "client_compatibility.v1";
pub const LEGACY_ORG_CUSTOM_DOMAIN_NO_PROTECTED_GATES: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrgCustomDomainSourceRoleV1 {
    DesktopHandler,
    DesktopMount,
    AuthenticationAndCors,
    PersistenceSchema,
    DesktopClient,
    DesktopQuery,
}

impl LegacyOrgCustomDomainSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::DesktopHandler => "desktop_handler",
            Self::DesktopMount => "desktop_mount",
            Self::AuthenticationAndCors => "authentication_and_cors",
            Self::PersistenceSchema => "persistence_schema",
            Self::DesktopClient => "desktop_client",
            Self::DesktopQuery => "desktop_query",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrgCustomDomainSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyOrgCustomDomainSourceRoleV1,
}

pub const LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCES: &[LegacyOrgCustomDomainSourcePinV1] = &[
    LegacyOrgCustomDomainSourcePinV1 {
        path: "apps/desktop/src/utils/queries.ts",
        symbol: "createCustomDomainQuery",
        sha256: "6d21daeae4084adbf9c65c67019b5f6b7c3ac6a5566c3b5fbe2fd3abcdcbcc1c",
        role: LegacyOrgCustomDomainSourceRoleV1::DesktopQuery,
    },
    LegacyOrgCustomDomainSourcePinV1 {
        path: "apps/desktop/src/utils/web-api.ts",
        symbol: "orgCustomDomainClient+protectedHeaders",
        sha256: "d3655b985a21a54d97b9974b17536aebab490929850baffaa5186d7a5632b45a",
        role: LegacyOrgCustomDomainSourceRoleV1::DesktopClient,
    },
    LegacyOrgCustomDomainSourcePinV1 {
        path: "apps/web/app/api/desktop/[...route]/root.ts",
        symbol: "GET /org-custom-domain",
        sha256: "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a",
        role: LegacyOrgCustomDomainSourceRoleV1::DesktopHandler,
    },
    LegacyOrgCustomDomainSourcePinV1 {
        path: "apps/web/app/api/desktop/[...route]/route.ts",
        symbol: "desktop mount+GET+OPTIONS",
        sha256: "34854ff6fc0839838165990bea1c9ebee86770b1648ec832bbbb786720c9db41",
        role: LegacyOrgCustomDomainSourceRoleV1::DesktopMount,
    },
    LegacyOrgCustomDomainSourcePinV1 {
        path: "apps/web/app/api/utils.ts",
        symbol: "getAuth+withAuth+corsMiddleware",
        sha256: "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
        role: LegacyOrgCustomDomainSourceRoleV1::AuthenticationAndCors,
    },
    LegacyOrgCustomDomainSourcePinV1 {
        path: "packages/database/schema.ts",
        symbol: "users+organizations+authApiKeys",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyOrgCustomDomainSourceRoleV1::PersistenceSchema,
    },
];

// Filled from the ordered source closure above. The checker independently
// verifies every referenced Cap file and this aggregate.
pub const LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCE_MANIFEST_SHA256: &str =
    "133b60766b5eb24790b49bc97774f461c7182cf41c7691a885d17765c872bfaf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrgCustomDomainProfileV1 {
    pub operation_id: &'static str,
    pub path: &'static str,
    pub method: &'static str,
    pub auth: &'static str,
    pub custom_domain: &'static str,
    pub domain_verified: &'static str,
    pub failure: &'static str,
    pub executable_evidence: &'static str,
}

pub const LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_PROFILE: LegacyOrgCustomDomainProfileV1 =
    LegacyOrgCustomDomainProfileV1 {
        operation_id: LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_OPERATION_ID,
        path: LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_PATH,
        method: "GET",
        auth: "session_or_36_character_api_key",
        custom_domain: "nullable_url_with_case_sensitive_http_prefixing",
        domain_verified: "nullable_iso_timestamp_string",
        failure: "401_plain_user_not_authenticated_or_500_json_fetch_failure",
        executable_evidence: "concrete_hono_handler",
    };

pub const LEGACY_WEB_ORG_CUSTOM_DOMAIN_DECLARATION_PROFILE: LegacyOrgCustomDomainProfileV1 =
    LegacyOrgCustomDomainProfileV1 {
        operation_id: LEGACY_WEB_ORG_CUSTOM_DOMAIN_OPERATION_ID,
        path: LEGACY_WEB_ORG_CUSTOM_DOMAIN_PATH,
        method: "GET",
        auth: "authorization_string_declaration_with_pinned_bearer_client",
        custom_domain: "nullable_string_declaration",
        domain_verified: "nullable_boolean_declaration",
        failure: "500_message_string_declaration",
        executable_evidence: "declaration_only_no_handler_or_boolean_derivation",
    };

#[must_use]
pub fn legacy_desktop_org_custom_domain_source_manifest() -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-desktop-org-custom-domain-source-manifest-v1\0");
    for source in LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCES {
        digest.update(source.path.as_bytes());
        digest.update([0]);
        digest.update(source.sha256.as_bytes());
        digest.update([0]);
        digest.update(source.role.stable_code().as_bytes());
        digest.update(b"\n");
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_source_closure_is_frozen_and_provider_free() {
        assert_eq!(LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCES.len(), 6);
        assert_eq!(
            legacy_desktop_org_custom_domain_source_manifest(),
            LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCE_MANIFEST_SHA256
        );
        assert!(LEGACY_ORG_CUSTOM_DOMAIN_NO_PROTECTED_GATES.is_empty());
        assert!(
            LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_SOURCES
                .iter()
                .all(|source| {
                    !source.path.contains("stripe")
                        && !source.path.contains("s3")
                        && !source.path.contains("google-drive")
                        && !source.path.contains("vercel")
                })
        );
    }

    #[test]
    fn declaration_only_web_shape_cannot_replace_desktop_runtime_shape() {
        assert_eq!(
            LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_PROFILE.domain_verified,
            "nullable_iso_timestamp_string"
        );
        assert_eq!(
            LEGACY_WEB_ORG_CUSTOM_DOMAIN_DECLARATION_PROFILE.domain_verified,
            "nullable_boolean_declaration"
        );
        assert_eq!(
            LEGACY_WEB_ORG_CUSTOM_DOMAIN_DECLARATION_PROFILE.executable_evidence,
            "declaration_only_no_handler_or_boolean_derivation"
        );
        assert_ne!(
            LEGACY_DESKTOP_ORG_CUSTOM_DOMAIN_PROFILE.domain_verified,
            LEGACY_WEB_ORG_CUSTOM_DOMAIN_DECLARATION_PROFILE.domain_verified
        );
    }
}
