//! Production-shaped legacy compatibility transport and D1 execution journal.
//!
//! The pinned registry promotes only a frozen semantic adapter whose exact
//! request and response are proven against the pinned Cap source. Every other
//! endpoint evidence bit, client flag, fallback, and retirement approval stays
//! disabled. Adding an operation ID alone can never make it executable.

use async_trait::async_trait;
use frame_application::{
    LEGACY_WEB_ACTIVE_ORGANIZATION_IDENTITY, LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID,
    LEGACY_WEB_DASHBOARD_INVALIDATION_PATH, LegacyCallerV1, LegacyClientFamilyV1,
    LegacyCompatibilityOutcomeV1, LegacyCompatibilityRegistryV1, LegacyCompatibilityRequestV1,
    LegacyEndpointOutcomeV1, LegacyExecutionCommandV1, LegacyExecutionErrorV1,
    LegacyExecutionOutcomeV1, LegacyOperationContractV1, LegacyOperationEvidenceV1,
    LegacyOperationExecutionPortV1, LegacyOperationKindV1, LegacyOperationReceiptV1,
    LegacyOrganizationSelectionAdapterV1, LegacyOrganizationSelectionErrorV1,
    LegacyOrganizationSelectionOutcomeV1, LegacyOrganizationSelectionRequestV1,
    LegacyRegistryErrorV1, RequestSecurityContextV1,
};
use frame_domain::{
    ApiAuthClassV1, ApiErrorCodeV1, ApiErrorV1, ApiMutationEnvelopeV1, ApiRequestPolicyV1,
    ChecksumSha256, ClientCompatibilityPolicyV1, IdempotencyKey, IdempotencyRequirementV1,
    LegacyEndpointDispositionV1,
};
use frame_ports::LegacyOrganizationSelectionRepositoryV1;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Env, send::IntoSendFuture};

use crate::legacy_notification_preferences_runtime::{
    D1LegacyNotificationPreferencesAuthorityV1, LEGACY_NOTIFICATION_PREFERENCES_CONTENT_TYPE,
    LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID, LEGACY_NOTIFICATION_PREFERENCES_PATH,
    LegacyNotificationPreferencesAuthorityV1, exact_json_body as notification_preferences_body,
};

const REPORT: &str = include_str!("../../../fixtures/api-parity/v1/route-workflow-report.json");
const CLAIM_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_claim.sql");
const INTENT_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_intent.sql");
const COMPLETE_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_complete.sql");
const AUDIT_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_audit.sql");
const LOAD_SQL: &str = include_str!("../queries/api_workflow/legacy_execution_load.sql");
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MAX_RAW_QUERY_BYTES: usize = 4 * 1_024;
const MAX_QUERY_PAIRS: usize = 64;
const MAX_QUERY_COMPONENT_BYTES: usize = 1_024;
const MAX_RAW_BODY_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_REQUEST_HEADERS: usize = 16;
const MAX_HEADER_NAME_BYTES: usize = 64;
const MAX_HEADER_VALUE_BYTES: usize = 4 * 1_024;
const MAX_MATCHED_PARAMS: usize = 16;
const MAX_PARAM_NAME_BYTES: usize = 64;
const MAX_PARAM_VALUE_BYTES: usize = 2 * 1_024;
const MAX_AUTHORITY_ID_BYTES: usize = 256;
const MAX_REVISION_RESOURCE_BYTES: usize = 256;
const MAX_RESPONSE_BODY_BYTES: usize = 256 * 1_024;
const MAX_RESPONSE_HEADERS: usize = 16;
const ALLOWED_REQUEST_HEADERS: &[&str] = &[
    "authorization",
    "content-length",
    "content-type",
    "idempotency-key",
    "if-match",
    "origin",
];
pub const LEGACY_STATUS_OPERATION_ID: &str = "cap-v1-05b6ba3f76daac22";
pub const LEGACY_STATUS_PATH: &str = "/api/status";
const LEGACY_STATUS_SOURCE_PATH: &str = "apps/web/app/api/status/route.ts";
const LEGACY_STATUS_SOURCE_SHA256: &str =
    "ba3eb1177da489a10f74c9dbc68e0db8324b695c82499e35d6f8d9da8aaf5797";
const LEGACY_STATUS_BODY: &str = "OK";
const LEGACY_STATUS_CONTENT_TYPE: &str = "text/plain;charset=UTF-8";
pub const LEGACY_MEDIA_SERVER_ROOT_OPERATION_ID: &str = "cap-v1-ff19008f47194c43";
pub const LEGACY_MEDIA_SERVER_ROOT_PATH: &str = "/media-server";
const LEGACY_MEDIA_SERVER_ROOT_SOURCE_PATH: &str = "apps/media-server/src/app.ts";
const LEGACY_MEDIA_SERVER_ROOT_SOURCE_SHA256: &str =
    "b3ba5fc1c8e93bd6896aa4399c283cc33a73e7777275816a11334fd71b75fc57";
const LEGACY_MEDIA_SERVER_ROOT_CONTENT_TYPE: &str = "application/json";
const LEGACY_MEDIA_SERVER_ROOT_BODY: &str = r#"{"name":"@cap/media-server","version":"1.0.0","endpoints":["/health","/audio/status","/audio/check","/audio/extract","/audio/convert","/video/status","/video/probe","/video/thumbnail","/video/convert","/video/process","/video/edit","/video/process/:jobId/status","/video/process/:jobId/cancel","/video/cleanup","/video/force-cleanup"]}"#;
pub const LEGACY_CHANGELOG_STATUS_PATH: &str = "/api/changelog/status";
pub const LEGACY_CHANGELOG_STATUS_GET_OPERATION_ID: &str = "cap-v1-a1b180c5d123c870";
pub const LEGACY_CHANGELOG_STATUS_OPTIONS_OPERATION_ID: &str = "cap-v1-16668b858461f386";
const LEGACY_CHANGELOG_STATUS_SOURCE_PATH: &str = "apps/web/app/api/changelog/status/route.ts";
const LEGACY_CHANGELOG_STATUS_SOURCE_SHA256: &str =
    "c2a3c107fce46765286e5a5e14fc3b21959e22b50070ecdc45f3d3d16ea5541b";
const LEGACY_CHANGELOG_UTILITY_SOURCE_PATH: &str = "apps/web/utils/changelog.ts";
const LEGACY_CHANGELOG_UTILITY_SOURCE_SHA256: &str =
    "30e6361fb869f87654cdfdb6b5d7f1533d86359ea1820efc1818b4a517759141";
const LEGACY_CHANGELOG_LATEST_SOURCE_PATH: &str = "apps/web/content/changelog/99.mdx";
const LEGACY_CHANGELOG_LATEST_SOURCE_SHA256: &str =
    "e67f4f451c30e040bbffb70b9cbbb0e107e1aa2723b629c878c6ceeaef7e567e";
const LEGACY_CHANGELOG_LATEST_VERSION: &str = "0.5.6";
const LEGACY_CHANGELOG_CONTENT_TYPE: &str = "application/json";
const LEGACY_CHANGELOG_UPDATE_BODY: &str = r#"{"hasUpdate":true}"#;
const LEGACY_CHANGELOG_CURRENT_BODY: &str = r#"{"hasUpdate":false}"#;
const LEGACY_CHANGELOG_CORS_HEADERS: &[(&str, &str)] = &[
    ("Access-Control-Allow-Origin", "*"),
    ("Access-Control-Allow-Methods", "GET, OPTIONS"),
    ("Access-Control-Allow-Headers", "Content-Type"),
];
pub const LEGACY_CHANGELOG_FEED_PATH: &str = "/api/changelog";
pub const LEGACY_CHANGELOG_FEED_GET_OPERATION_ID: &str = "cap-v1-0fa8384f3666825b";
pub const LEGACY_CHANGELOG_FEED_OPTIONS_OPERATION_ID: &str = "cap-v1-237f41f3086a2d67";
const LEGACY_CHANGELOG_FEED_SOURCE_PATH: &str = "apps/web/app/api/changelog/route.ts";
const LEGACY_CHANGELOG_FEED_SOURCE_SHA256: &str =
    "b47371ce19a03def1b675996615e1c48af41651bc48ca479d0a97bd9e7167b04";
const LEGACY_CHANGELOG_CORS_SOURCE_PATH: &str = "apps/web/utils/cors.ts";
const LEGACY_CHANGELOG_CORS_SOURCE_SHA256: &str =
    "fff2797f5845e2fcd6c2941c166a91d616af547cba95bf74291706e541f32edc";
pub const LEGACY_CHANGELOG_FEED_SOURCE_MANIFEST_SHA256: &str =
    "dace60a24a816766681282e4569eda38e16fd85c96a9b2ab311a59351ef58b2d";
pub const LEGACY_CHANGELOG_FEED_BODY_SHA256: &str =
    "333c789a76f6f496f94e5e2a47a192fe0c9f87165971689c9c297e5eb43b7499";
const LEGACY_CHANGELOG_FEED_BODY: &str =
    include_str!("../../../fixtures/api-parity/v1/changelog-feed.json");
const LEGACY_CHANGELOG_FEED_CONTENT_TYPE: &str = "application/json";
const LEGACY_CHANGELOG_FEED_GET_MAX_RESPONSE_BYTES: usize = 128 * 1_024;
const LEGACY_CHANGELOG_ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:3001",
    "http://localhost:3000",
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
    "https://cap.so",
    "https://www.cap.so",
    "https://cap.link",
    "https://www.cap.link",
];
pub const LEGACY_MOBILE_SESSION_CONFIG_OPERATION_ID: &str = "cap-v1-4f21920a947c4c84";
pub const LEGACY_MOBILE_SESSION_CONFIG_PATH: &str = "/api/mobile/session/config";
const LEGACY_MOBILE_SESSION_CONFIG_ROUTE_SOURCE_PATH: &str =
    "apps/web/app/api/mobile/[...route]/route.ts";
const LEGACY_MOBILE_SESSION_CONFIG_ROUTE_SOURCE_SHA256: &str =
    "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79";
const LEGACY_MOBILE_SESSION_CONFIG_DOMAIN_SOURCE_PATH: &str = "packages/web-domain/src/Mobile.ts";
const LEGACY_MOBILE_SESSION_CONFIG_DOMAIN_SOURCE_SHA256: &str =
    "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b";
const LEGACY_MOBILE_SESSION_CONFIG_CONTENT_TYPE: &str = "application/json";
const LEGACY_MOBILE_SESSION_CONFIG_NONE_BODY: &str =
    r#"{"googleAuthAvailable":false,"workosAuthAvailable":false}"#;
const LEGACY_MOBILE_SESSION_CONFIG_GOOGLE_BODY: &str =
    r#"{"googleAuthAvailable":true,"workosAuthAvailable":false}"#;
const LEGACY_MOBILE_SESSION_CONFIG_WORKOS_BODY: &str =
    r#"{"googleAuthAvailable":false,"workosAuthAvailable":true}"#;
const LEGACY_MOBILE_SESSION_CONFIG_BOTH_BODY: &str =
    r#"{"googleAuthAvailable":true,"workosAuthAvailable":true}"#;
const LEGACY_NOTIFICATION_PREFERENCES_ROUTE_SOURCE_PATH: &str =
    "apps/web/app/api/notifications/preferences/route.ts";
const LEGACY_NOTIFICATION_PREFERENCES_ROUTE_SOURCE_SHA256: &str =
    "3692f8854c0c050f5168f89acb1d03dc1c31d4529000e0b5e140078e8d3ce975";
const LEGACY_NOTIFICATION_PREFERENCES_SESSION_SOURCE_PATH: &str =
    "packages/database/auth/session.ts";
const LEGACY_NOTIFICATION_PREFERENCES_SESSION_SOURCE_SHA256: &str =
    "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee";
const LEGACY_NOTIFICATION_PREFERENCES_SCHEMA_SOURCE_PATH: &str = "packages/database/schema.ts";
const LEGACY_NOTIFICATION_PREFERENCES_SCHEMA_SOURCE_SHA256: &str =
    "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9";

// Every enabled ID must also resolve to a typed registration below, match the
// pinned report identity, and carry all five exact report evidence axes. Durable adapters
// are a separate allowlist so a static read can never accidentally enter the
// D1 mutation journal.
const ENABLED_SEMANTIC_ADAPTERS: &[&str] = &[
    LEGACY_STATUS_OPERATION_ID,
    LEGACY_MEDIA_SERVER_ROOT_OPERATION_ID,
    LEGACY_CHANGELOG_STATUS_GET_OPERATION_ID,
    LEGACY_CHANGELOG_STATUS_OPTIONS_OPERATION_ID,
    LEGACY_CHANGELOG_FEED_GET_OPERATION_ID,
    LEGACY_CHANGELOG_FEED_OPTIONS_OPERATION_ID,
    LEGACY_MOBILE_SESSION_CONFIG_OPERATION_ID,
    LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID,
];
const ENABLED_DURABLE_ADAPTERS: &[&str] = &[];
const ENABLED_EXACT_BUSINESS_ADAPTERS: &[&str] = &[LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacySemanticAdapterV1 {
    PublicStatusOk,
    MediaServerRootMetadata,
    ChangelogStatusGet,
    ChangelogStatusOptions,
    ChangelogFeedGet,
    ChangelogFeedOptions,
    MobileSessionConfigGet,
    NotificationPreferencesGet,
}

#[derive(Debug, Clone, Copy)]
struct LegacySemanticSourceV1 {
    path: &'static str,
    sha256: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct LegacySemanticRegistrationV1 {
    operation_id: &'static str,
    kind: &'static str,
    method: &'static str,
    legacy_path: &'static str,
    sources: &'static [LegacySemanticSourceV1],
    clients: &'static [&'static str],
    auth: &'static str,
    rate_limit_bucket: &'static str,
    adapter: LegacySemanticAdapterV1,
}

#[derive(Debug, Clone, Copy)]
struct LegacyContractRegistrationV1 {
    operation_id: &'static str,
    kind: &'static str,
    method: &'static str,
    legacy_path: &'static str,
    sources: &'static [LegacySemanticSourceV1],
    clients: &'static [&'static str],
    auth: &'static str,
    rate_limit_bucket: &'static str,
}

impl LegacySemanticRegistrationV1 {
    const fn contract(self) -> LegacyContractRegistrationV1 {
        LegacyContractRegistrationV1 {
            operation_id: self.operation_id,
            kind: self.kind,
            method: self.method,
            legacy_path: self.legacy_path,
            sources: self.sources,
            clients: self.clients,
            auth: self.auth,
            rate_limit_bucket: self.rate_limit_bucket,
        }
    }
}

const LEGACY_WEB_ACTIVE_ORGANIZATION_RUNTIME_SOURCES: &[LegacySemanticSourceV1] = &[
    LegacySemanticSourceV1 {
        path: "apps/web/app/(org)/dashboard/_components/Navbar/server.ts",
        sha256: "a7ea138516eb20f40dad4ad53913e69b01e4f5ad8b2938eb9f5a9a98ab3a29b3",
    },
    LegacySemanticSourceV1 {
        path: "packages/database/auth/session.ts",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
    },
    LegacySemanticSourceV1 {
        path: "packages/database/schema.ts",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
    },
    LegacySemanticSourceV1 {
        path: "packages/web-domain/src/Organisation.ts",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
    },
];

const EXACT_BUSINESS_ADAPTERS: &[LegacyContractRegistrationV1] = &[LegacyContractRegistrationV1 {
    operation_id: LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID,
    kind: "server_action",
    method: "ACTION",
    legacy_path: LEGACY_WEB_ACTIVE_ORGANIZATION_IDENTITY,
    sources: LEGACY_WEB_ACTIVE_ORGANIZATION_RUNTIME_SOURCES,
    clients: &["web"],
    auth: "session",
    rate_limit_bucket: "organization_library.v1",
}];

const SEMANTIC_ADAPTERS: &[LegacySemanticRegistrationV1] = &[
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_STATUS_OPERATION_ID,
        kind: "route",
        method: "GET",
        legacy_path: LEGACY_STATUS_PATH,
        sources: &[LegacySemanticSourceV1 {
            path: LEGACY_STATUS_SOURCE_PATH,
            sha256: LEGACY_STATUS_SOURCE_SHA256,
        }],
        clients: &["web"],
        auth: "public_or_flow_token",
        rate_limit_bucket: "service_misc.v1",
        adapter: LegacySemanticAdapterV1::PublicStatusOk,
    },
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_MEDIA_SERVER_ROOT_OPERATION_ID,
        kind: "route",
        method: "GET",
        legacy_path: LEGACY_MEDIA_SERVER_ROOT_PATH,
        sources: &[LegacySemanticSourceV1 {
            path: LEGACY_MEDIA_SERVER_ROOT_SOURCE_PATH,
            sha256: LEGACY_MEDIA_SERVER_ROOT_SOURCE_SHA256,
        }],
        clients: &["internal_worker"],
        auth: "public_or_flow_token",
        rate_limit_bucket: "service_misc.v1",
        adapter: LegacySemanticAdapterV1::MediaServerRootMetadata,
    },
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_CHANGELOG_STATUS_GET_OPERATION_ID,
        kind: "route",
        method: "GET",
        legacy_path: LEGACY_CHANGELOG_STATUS_PATH,
        sources: &[
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_STATUS_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_STATUS_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_UTILITY_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_UTILITY_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_LATEST_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_LATEST_SOURCE_SHA256,
            },
        ],
        clients: &["desktop"],
        auth: "public_or_flow_token",
        rate_limit_bucket: "client_compatibility.v1",
        adapter: LegacySemanticAdapterV1::ChangelogStatusGet,
    },
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_CHANGELOG_STATUS_OPTIONS_OPERATION_ID,
        kind: "route",
        method: "OPTIONS",
        legacy_path: LEGACY_CHANGELOG_STATUS_PATH,
        sources: &[LegacySemanticSourceV1 {
            path: LEGACY_CHANGELOG_STATUS_SOURCE_PATH,
            sha256: LEGACY_CHANGELOG_STATUS_SOURCE_SHA256,
        }],
        clients: &["web"],
        auth: "public_or_flow_token",
        rate_limit_bucket: "service_misc.v1",
        adapter: LegacySemanticAdapterV1::ChangelogStatusOptions,
    },
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_CHANGELOG_FEED_GET_OPERATION_ID,
        kind: "route",
        method: "GET",
        legacy_path: LEGACY_CHANGELOG_FEED_PATH,
        sources: &[
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_FEED_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_FEED_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_UTILITY_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_UTILITY_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_CORS_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_CORS_SOURCE_SHA256,
            },
        ],
        clients: &["desktop"],
        auth: "public_or_flow_token",
        rate_limit_bucket: "client_compatibility.v1",
        adapter: LegacySemanticAdapterV1::ChangelogFeedGet,
    },
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_CHANGELOG_FEED_OPTIONS_OPERATION_ID,
        kind: "route",
        method: "OPTIONS",
        legacy_path: LEGACY_CHANGELOG_FEED_PATH,
        sources: &[
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_FEED_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_FEED_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_CHANGELOG_CORS_SOURCE_PATH,
                sha256: LEGACY_CHANGELOG_CORS_SOURCE_SHA256,
            },
        ],
        clients: &["web"],
        auth: "public_or_flow_token",
        rate_limit_bucket: "service_misc.v1",
        adapter: LegacySemanticAdapterV1::ChangelogFeedOptions,
    },
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_MOBILE_SESSION_CONFIG_OPERATION_ID,
        kind: "route",
        method: "GET",
        legacy_path: LEGACY_MOBILE_SESSION_CONFIG_PATH,
        sources: &[
            LegacySemanticSourceV1 {
                path: LEGACY_MOBILE_SESSION_CONFIG_ROUTE_SOURCE_PATH,
                sha256: LEGACY_MOBILE_SESSION_CONFIG_ROUTE_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_MOBILE_SESSION_CONFIG_DOMAIN_SOURCE_PATH,
                sha256: LEGACY_MOBILE_SESSION_CONFIG_DOMAIN_SOURCE_SHA256,
            },
        ],
        clients: &["mobile"],
        auth: "public_or_flow_token",
        rate_limit_bucket: "client_compatibility.v1",
        adapter: LegacySemanticAdapterV1::MobileSessionConfigGet,
    },
    LegacySemanticRegistrationV1 {
        operation_id: LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID,
        kind: "route",
        method: "GET",
        legacy_path: LEGACY_NOTIFICATION_PREFERENCES_PATH,
        sources: &[
            LegacySemanticSourceV1 {
                path: LEGACY_NOTIFICATION_PREFERENCES_ROUTE_SOURCE_PATH,
                sha256: LEGACY_NOTIFICATION_PREFERENCES_ROUTE_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_NOTIFICATION_PREFERENCES_SESSION_SOURCE_PATH,
                sha256: LEGACY_NOTIFICATION_PREFERENCES_SESSION_SOURCE_SHA256,
            },
            LegacySemanticSourceV1 {
                path: LEGACY_NOTIFICATION_PREFERENCES_SCHEMA_SOURCE_PATH,
                sha256: LEGACY_NOTIFICATION_PREFERENCES_SCHEMA_SOURCE_SHA256,
            },
        ],
        clients: &["web"],
        auth: "session",
        rate_limit_bucket: "collaboration_notifications.v1",
        adapter: LegacySemanticAdapterV1::NotificationPreferencesGet,
    },
];

#[derive(Deserialize)]
struct Report {
    entries: Vec<ReportRow>,
}

#[derive(Deserialize)]
struct ReportRow {
    id: String,
    kind: String,
    legacy_path: String,
    method: String,
    clients: Vec<String>,
    auth: String,
    disposition: String,
    security: ReportSecurity,
    sources: Vec<ReportSource>,
    contract_evidence: ReportEvidence,
}

#[derive(Deserialize)]
struct ReportSecurity {
    max_body_bytes: u64,
    accepted_content_types: Vec<String>,
    rate_limit_bucket: String,
    idempotency: String,
}

#[derive(Deserialize)]
struct ReportSource {
    path: String,
    sha256: String,
}

#[derive(Deserialize)]
struct ReportEvidence {
    success: String,
    validation: String,
    authorization: String,
    idempotency_retry: String,
    failure: String,
}

#[derive(Deserialize)]
struct ExecutionRow {
    request_fingerprint: String,
    reservation_digest: String,
    state: String,
    response_status: Option<u16>,
    result_digest: Option<String>,
    intent_reservation_digest: Option<String>,
    audit_reservation_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyAuthenticatedContextV1 {
    principal_id: String,
    tenant_id: Option<String>,
}

impl LegacyAuthenticatedContextV1 {
    pub fn new(
        principal_id: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> Result<Self, LegacyExecutionErrorV1> {
        let principal_id = principal_id.into();
        let tenant_id = tenant_id.into();
        if !valid_authority_id(&principal_id) || !valid_authority_id(&tenant_id) {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        Ok(Self {
            principal_id,
            tenant_id: Some(tenant_id),
        })
    }

    pub fn principal_only(principal_id: impl Into<String>) -> Result<Self, LegacyExecutionErrorV1> {
        let principal_id = principal_id.into();
        if !valid_authority_id(&principal_id) {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        Ok(Self {
            principal_id,
            tenant_id: None,
        })
    }

    #[must_use]
    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }

    #[must_use]
    pub fn tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRevisionContextV1 {
    resource_id: String,
    expected_revision: u64,
}

impl LegacyRevisionContextV1 {
    pub fn new(
        resource_id: impl Into<String>,
        expected_revision: u64,
    ) -> Result<Self, LegacyExecutionErrorV1> {
        let resource_id = resource_id.into();
        if resource_id.is_empty()
            || resource_id.len() > MAX_REVISION_RESOURCE_BYTES
            || !resource_id.is_ascii()
            || resource_id.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        Ok(Self {
            resource_id,
            expected_revision,
        })
    }

    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    #[must_use]
    pub const fn expected_revision(&self) -> u64 {
        self.expected_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMatchedParamV1 {
    name: String,
    raw_value: String,
}

impl LegacyMatchedParamV1 {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }
}

/// A canonical application/x-www-form-urlencoded multimap. Pair order and
/// duplicates are retained; lookup deliberately returns the first value, like
/// `URLSearchParams.get` in the pinned Cap routes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyCanonicalQueryV1 {
    canonical: String,
    pairs: Vec<(String, String)>,
}

impl LegacyCanonicalQueryV1 {
    fn parse(raw: &str) -> Result<Self, LegacyExecutionErrorV1> {
        if raw.len() > MAX_RAW_QUERY_BYTES
            || !raw.is_ascii()
            || raw.bytes().any(|byte| byte.is_ascii_control())
            || !valid_percent_encoding(raw)
        {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        let pairs = url::form_urlencoded::parse(raw.as_bytes())
            .into_owned()
            .collect::<Vec<_>>();
        if pairs.len() > MAX_QUERY_PAIRS
            || pairs.iter().any(|(name, value)| {
                name.len() > MAX_QUERY_COMPONENT_BYTES
                    || value.len() > MAX_QUERY_COMPONENT_BYTES
                    || name.chars().any(char::is_control)
                    || value.chars().any(char::is_control)
            })
        {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        let canonical = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                pairs
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            )
            .finish();
        Ok(Self { canonical, pairs })
    }

    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    #[must_use]
    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    #[must_use]
    pub fn first(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn all<'query>(&'query self, name: &'query str) -> impl Iterator<Item = &'query str> {
        self.pairs
            .iter()
            .filter(move |(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LegacyNormalizedHeadersV1 {
    values: Vec<(String, String)>,
}

impl std::fmt::Debug for LegacyNormalizedHeadersV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyNormalizedHeadersV1")
            .field(
                "names",
                &self
                    .values
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl LegacyNormalizedHeadersV1 {
    fn new(values: Vec<(String, String)>) -> Result<Self, LegacyExecutionErrorV1> {
        if values.len() > MAX_REQUEST_HEADERS {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        let mut normalized = Vec::with_capacity(values.len());
        for (name, value) in values {
            let name = name.to_ascii_lowercase();
            let value = value.trim_matches(|character| matches!(character, ' ' | '\t'));
            if name.is_empty()
                || name.len() > MAX_HEADER_NAME_BYTES
                || value.len() > MAX_HEADER_VALUE_BYTES
                || !valid_header_name(&name)
                || !ALLOWED_REQUEST_HEADERS.contains(&name.as_str())
                || !value.is_ascii()
                || value.bytes().any(|byte| byte.is_ascii_control())
                || normalized
                    .iter()
                    .any(|(existing, _): &(String, String)| existing == &name)
            {
                return Err(LegacyExecutionErrorV1::Invalid);
            }
            normalized.push((name, value.to_owned()));
        }
        Ok(Self { values: normalized })
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }

    #[must_use]
    pub fn values(&self) -> &[(String, String)] {
        &self.values
    }
}

pub struct LegacyHttpTransportRequestPartsV1 {
    pub method: String,
    pub raw_path: String,
    pub raw_query: String,
    pub raw_body: Vec<u8>,
    pub headers: Vec<(String, String)>,
    pub caller: LegacyCallerV1,
    pub correlation_id: String,
    pub security: RequestSecurityContextV1,
    pub scope_digest: ChecksumSha256,
    pub authenticated: Option<LegacyAuthenticatedContextV1>,
    pub revision: Option<LegacyRevisionContextV1>,
    pub fallback_origin: Option<String>,
    pub configured_origin: Option<String>,
}

pub struct LegacyHttpTransportRequestV1 {
    method: String,
    raw_path: String,
    query: LegacyCanonicalQueryV1,
    body: Vec<u8>,
    headers: LegacyNormalizedHeadersV1,
    caller: LegacyCallerV1,
    envelope: ApiMutationEnvelopeV1,
    security: RequestSecurityContextV1,
    scope_digest: ChecksumSha256,
    authenticated: Option<LegacyAuthenticatedContextV1>,
    revision: Option<LegacyRevisionContextV1>,
    fallback_origin: Option<String>,
    configured_origin: Option<String>,
}

impl LegacyHttpTransportRequestV1 {
    pub fn new(parts: LegacyHttpTransportRequestPartsV1) -> Result<Self, LegacyExecutionErrorV1> {
        if parts.method.is_empty()
            || parts.method.len() > 16
            || !parts.method.bytes().all(|byte| byte.is_ascii_uppercase())
            || parts.raw_path.is_empty()
            || parts.raw_path.len() > 2_048
            || !parts.raw_path.is_ascii()
            || parts.raw_path.contains(['?', '#'])
            || parts.raw_path.bytes().any(|byte| byte.is_ascii_control())
            || parts.raw_body.len() > MAX_RAW_BODY_BYTES
            || parts.revision.is_some() && parts.authenticated.is_none()
            || parts.security.authenticated != parts.authenticated.is_some()
            || !valid_optional_origin(parts.fallback_origin.as_deref())
            || !valid_optional_origin(parts.configured_origin.as_deref())
        {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        let query = LegacyCanonicalQueryV1::parse(&parts.raw_query)?;
        let headers = LegacyNormalizedHeadersV1::new(parts.headers)?;
        let body_length =
            u64::try_from(parts.raw_body.len()).map_err(|_| LegacyExecutionErrorV1::Invalid)?;
        if let Some(declared) = headers.get("content-length") {
            let parsed = declared
                .parse::<u64>()
                .map_err(|_| LegacyExecutionErrorV1::Invalid)?;
            if parsed != body_length || parsed.to_string() != declared {
                return Err(LegacyExecutionErrorV1::Invalid);
            }
        }
        let idempotency_key = headers
            .get("idempotency-key")
            .map(IdempotencyKey::parse)
            .transpose()
            .map_err(|_| LegacyExecutionErrorV1::Invalid)?;
        Ok(Self {
            method: parts.method,
            raw_path: parts.raw_path,
            query,
            body: parts.raw_body,
            envelope: ApiMutationEnvelopeV1 {
                content_length: body_length,
                content_type: headers.get("content-type").map(str::to_owned),
                idempotency_key,
                correlation_id: parts.correlation_id,
            },
            headers,
            caller: parts.caller,
            security: parts.security,
            scope_digest: parts.scope_digest,
            authenticated: parts.authenticated,
            revision: parts.revision,
            fallback_origin: parts.fallback_origin,
            configured_origin: parts.configured_origin,
        })
    }
}

pub struct LegacyAdapterRequestV1 {
    operation_id: String,
    method: String,
    raw_path: String,
    matched_params: Vec<LegacyMatchedParamV1>,
    query: LegacyCanonicalQueryV1,
    body: Vec<u8>,
    headers: LegacyNormalizedHeadersV1,
    scope_digest: ChecksumSha256,
    authenticated: Option<LegacyAuthenticatedContextV1>,
    revision: Option<LegacyRevisionContextV1>,
    fallback_origin: Option<String>,
    configured_origin: Option<String>,
}

impl LegacyAdapterRequestV1 {
    fn from_transport(
        input: LegacyHttpTransportRequestV1,
        registration: LegacySemanticRegistrationV1,
    ) -> Result<(Self, LegacyCompatibilityRequestV1), LegacyExecutionErrorV1> {
        let matched_params = extract_matched_params(registration.legacy_path, &input.raw_path)
            .ok_or(LegacyExecutionErrorV1::Invalid)?;
        let admission = LegacyCompatibilityRequestV1 {
            operation_id: registration.operation_id.to_owned(),
            caller: input.caller,
            envelope: input.envelope,
            security: input.security,
        };
        Ok((
            Self {
                operation_id: registration.operation_id.to_owned(),
                method: input.method,
                raw_path: input.raw_path,
                matched_params,
                query: input.query,
                body: input.body,
                headers: input.headers,
                scope_digest: input.scope_digest,
                authenticated: input.authenticated,
                revision: input.revision,
                fallback_origin: input.fallback_origin,
                configured_origin: input.configured_origin,
            },
            admission,
        ))
    }

    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    #[must_use]
    pub fn method(&self) -> &str {
        &self.method
    }

    #[must_use]
    pub fn raw_path(&self) -> &str {
        &self.raw_path
    }

    #[must_use]
    pub fn matched_params(&self) -> &[LegacyMatchedParamV1] {
        &self.matched_params
    }

    #[must_use]
    pub const fn query(&self) -> &LegacyCanonicalQueryV1 {
        &self.query
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub const fn headers(&self) -> &LegacyNormalizedHeadersV1 {
        &self.headers
    }

    #[must_use]
    pub const fn scope_digest(&self) -> &ChecksumSha256 {
        &self.scope_digest
    }

    #[must_use]
    pub const fn authenticated(&self) -> Option<&LegacyAuthenticatedContextV1> {
        self.authenticated.as_ref()
    }

    #[must_use]
    pub const fn revision(&self) -> Option<&LegacyRevisionContextV1> {
        self.revision.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyHttpTransportResponseV1 {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

impl LegacyHttpTransportResponseV1 {
    pub(crate) fn new(
        status: u16,
        content_type: Option<&str>,
        body: impl Into<Vec<u8>>,
        headers: Vec<(String, String)>,
        adapter_max_body_bytes: usize,
    ) -> Result<Self, LegacyExecutionErrorV1> {
        let body = body.into();
        if !(200..=299).contains(&status)
            || body.len() > adapter_max_body_bytes
            || body.len() > MAX_RESPONSE_BODY_BYTES
            || headers.len() > MAX_RESPONSE_HEADERS
            || content_type.is_some_and(|value| {
                value.is_empty()
                    || value.len() > MAX_HEADER_VALUE_BYTES
                    || !value.is_ascii()
                    || value.bytes().any(|byte| byte.is_ascii_control())
            })
        {
            return Err(LegacyExecutionErrorV1::Internal);
        }
        let mut seen = Vec::<String>::new();
        for (name, value) in &headers {
            let normalized = name.to_ascii_lowercase();
            if name.is_empty()
                || name.len() > MAX_HEADER_NAME_BYTES
                || value.len() > MAX_HEADER_VALUE_BYTES
                || !valid_header_name(&normalized)
                || !value.is_ascii()
                || value.bytes().any(|byte| byte.is_ascii_control())
                || seen.contains(&normalized)
            {
                return Err(LegacyExecutionErrorV1::Internal);
            }
            seen.push(normalized);
        }
        Ok(Self {
            status,
            content_type: content_type.map(str::to_owned),
            body,
            headers,
        })
    }

    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // The D1 variant is the extension point for the next typed adapter module.
pub(crate) enum LegacyAdapterAuthorityRequirementV1 {
    Static,
    D1BusinessAuthority,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LegacyStaticAdapterAuthorityV1 {
    google_auth_available: bool,
    workos_auth_available: bool,
}

impl LegacyStaticAdapterAuthorityV1 {
    fn from_worker_env(env: &Env) -> Self {
        Self {
            google_auth_available: non_empty_worker_binding(env, "GOOGLE_CLIENT_ID"),
            workos_auth_available: non_empty_worker_binding(env, "WORKOS_CLIENT_ID"),
        }
    }

    const fn mobile_auth_config(self) -> (bool, bool) {
        (self.google_auth_available, self.workos_auth_available)
    }
}

fn non_empty_worker_binding(env: &Env, name: &str) -> bool {
    env.var(name)
        .ok()
        .is_some_and(|value| binding_value_is_truthy(&value.to_string()))
}

/// Match JavaScript `Boolean(envValue)`: only the empty string is false; a
/// whitespace-bearing configured value remains true. The caller retains only
/// the boolean and never logs or stores the binding value.
const fn binding_value_is_truthy(value: &str) -> bool {
    !value.is_empty()
}

#[derive(Clone, Copy)]
#[allow(dead_code)] // The D1 variant is the extension point for the next typed adapter module.
pub(crate) enum LegacyAdapterAuthorityV1<'database> {
    Static(&'database LegacyStaticAdapterAuthorityV1),
    D1BusinessAuthority(&'database D1Database),
}

impl<'authority> LegacyAdapterAuthorityV1<'authority> {
    const fn static_authority(
        self,
    ) -> Result<&'authority LegacyStaticAdapterAuthorityV1, LegacyExecutionErrorV1> {
        match self {
            Self::Static(authority) => Ok(authority),
            Self::D1BusinessAuthority(_) => Err(LegacyExecutionErrorV1::Unsupported),
        }
    }

    const fn d1_business_authority(self) -> Result<&'authority D1Database, LegacyExecutionErrorV1> {
        match self {
            Self::D1BusinessAuthority(authority) => Ok(authority),
            Self::Static(_) => Err(LegacyExecutionErrorV1::Unsupported),
        }
    }
}

pub(crate) struct LegacyTypedAdapterExecutionV1 {
    request_fingerprint: ChecksumSha256,
    response: LegacyHttpTransportResponseV1,
    replayed: bool,
}

impl LegacyTypedAdapterExecutionV1 {
    pub(crate) fn completed(
        request_fingerprint: ChecksumSha256,
        response: LegacyHttpTransportResponseV1,
    ) -> Self {
        Self {
            request_fingerprint,
            response,
            replayed: false,
        }
    }

    #[allow(dead_code)] // Used by future typed D1 adapters, never by static adapters.
    pub(crate) fn replay(
        request_fingerprint: ChecksumSha256,
        response: LegacyHttpTransportResponseV1,
    ) -> Self {
        Self {
            request_fingerprint,
            response,
            replayed: true,
        }
    }

    #[allow(dead_code)] // Sibling typed adapter modules consume these accessors.
    #[must_use]
    pub(crate) const fn request_fingerprint(&self) -> &ChecksumSha256 {
        &self.request_fingerprint
    }

    #[allow(dead_code)]
    #[must_use]
    pub(crate) const fn response(&self) -> &LegacyHttpTransportResponseV1 {
        &self.response
    }

    #[allow(dead_code)]
    #[must_use]
    pub(crate) const fn replayed(&self) -> bool {
        self.replayed
    }
}

/// Typed semantic boundary for each promoted adapter. Durable implementations
/// must use their actual D1 business authority here; the generic compatibility
/// journal below is intentionally not an adapter authority.
#[async_trait(?Send)]
pub(crate) trait LegacyTypedAdapterV1 {
    fn operation_id(&self) -> &'static str;
    fn authority_requirement(&self) -> LegacyAdapterAuthorityRequirementV1;
    fn request_fingerprint(
        &self,
        request: &LegacyAdapterRequestV1,
        authority: LegacyAdapterAuthorityV1<'_>,
    ) -> Result<ChecksumSha256, LegacyExecutionErrorV1>;
    async fn dispatch(
        &self,
        request: &LegacyAdapterRequestV1,
        authority: LegacyAdapterAuthorityV1<'_>,
    ) -> Result<LegacyTypedAdapterExecutionV1, LegacyExecutionErrorV1>;
}

#[async_trait(?Send)]
impl LegacyTypedAdapterV1 for LegacySemanticAdapterV1 {
    fn operation_id(&self) -> &'static str {
        match self {
            Self::PublicStatusOk => LEGACY_STATUS_OPERATION_ID,
            Self::MediaServerRootMetadata => LEGACY_MEDIA_SERVER_ROOT_OPERATION_ID,
            Self::ChangelogStatusGet => LEGACY_CHANGELOG_STATUS_GET_OPERATION_ID,
            Self::ChangelogStatusOptions => LEGACY_CHANGELOG_STATUS_OPTIONS_OPERATION_ID,
            Self::ChangelogFeedGet => LEGACY_CHANGELOG_FEED_GET_OPERATION_ID,
            Self::ChangelogFeedOptions => LEGACY_CHANGELOG_FEED_OPTIONS_OPERATION_ID,
            Self::MobileSessionConfigGet => LEGACY_MOBILE_SESSION_CONFIG_OPERATION_ID,
            Self::NotificationPreferencesGet => LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID,
        }
    }

    fn authority_requirement(&self) -> LegacyAdapterAuthorityRequirementV1 {
        match self {
            Self::NotificationPreferencesGet => {
                LegacyAdapterAuthorityRequirementV1::D1BusinessAuthority
            }
            _ => LegacyAdapterAuthorityRequirementV1::Static,
        }
    }

    fn request_fingerprint(
        &self,
        request: &LegacyAdapterRequestV1,
        authority: LegacyAdapterAuthorityV1<'_>,
    ) -> Result<ChecksumSha256, LegacyExecutionErrorV1> {
        if request.operation_id() != self.operation_id()
            || !request.body().is_empty()
            || !request.matched_params().is_empty()
            || request.revision().is_some()
        {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        if *self == Self::NotificationPreferencesGet {
            let authenticated = request
                .authenticated()
                .ok_or(LegacyExecutionErrorV1::Invalid)?;
            if authenticated.tenant_id().is_some() {
                return Err(LegacyExecutionErrorV1::Invalid);
            }
            authority.d1_business_authority()?;
            return Ok(notification_preferences_request_fingerprint(
                request,
                authenticated,
            ));
        }
        if request.authenticated().is_some() {
            return Err(LegacyExecutionErrorV1::Invalid);
        }
        Ok(semantic_request_fingerprint(
            *self,
            request,
            authority.static_authority()?,
        ))
    }

    async fn dispatch(
        &self,
        request: &LegacyAdapterRequestV1,
        authority: LegacyAdapterAuthorityV1<'_>,
    ) -> Result<LegacyTypedAdapterExecutionV1, LegacyExecutionErrorV1> {
        let request_fingerprint = self.request_fingerprint(request, authority)?;
        if *self == Self::NotificationPreferencesGet {
            let database = authority.d1_business_authority()?;
            let actor_id = request
                .authenticated()
                .ok_or(LegacyExecutionErrorV1::Invalid)?
                .principal_id();
            let preferences = D1LegacyNotificationPreferencesAuthorityV1::new(database)
                .read_for_actor(actor_id)
                .await
                .map_err(|_| LegacyExecutionErrorV1::Internal)?;
            let response = LegacyHttpTransportResponseV1::new(
                200,
                Some(LEGACY_NOTIFICATION_PREFERENCES_CONTENT_TYPE),
                notification_preferences_body(preferences)
                    .map_err(|_| LegacyExecutionErrorV1::Internal)?,
                Vec::new(),
                64 * 1_024,
            )?;
            return Ok(LegacyTypedAdapterExecutionV1::completed(
                request_fingerprint,
                response,
            ));
        }
        let static_authority = authority.static_authority()?;
        let response = semantic_response(*self, request, static_authority)?;
        Ok(LegacyTypedAdapterExecutionV1::completed(
            request_fingerprint,
            response,
        ))
    }
}

struct LegacyTypedAdapterRegistryV1;

impl LegacyTypedAdapterRegistryV1 {
    fn resolve(operation_id: &str) -> Option<LegacySemanticRegistrationV1> {
        SEMANTIC_ADAPTERS.iter().copied().find(|registration| {
            registration.operation_id == operation_id
                && registration.adapter.operation_id() == operation_id
        })
    }

    async fn dispatch(
        registration: LegacySemanticRegistrationV1,
        request: &LegacyAdapterRequestV1,
        static_authority: &LegacyStaticAdapterAuthorityV1,
        database: Option<&D1Database>,
    ) -> Result<LegacyTypedAdapterExecutionV1, LegacyExecutionErrorV1> {
        let authority = match registration.adapter.authority_requirement() {
            LegacyAdapterAuthorityRequirementV1::Static => {
                LegacyAdapterAuthorityV1::Static(static_authority)
            }
            LegacyAdapterAuthorityRequirementV1::D1BusinessAuthority => {
                LegacyAdapterAuthorityV1::D1BusinessAuthority(
                    database.ok_or(LegacyExecutionErrorV1::TemporarilyUnavailable)?,
                )
            }
        };
        let expected = registration
            .adapter
            .request_fingerprint(request, authority)?;
        let execution = registration.adapter.dispatch(request, authority).await?;
        if execution.request_fingerprint != expected {
            return Err(LegacyExecutionErrorV1::Internal);
        }
        Ok(execution)
    }
}

fn semantic_registration(operation_id: &str) -> Option<LegacySemanticRegistrationV1> {
    LegacyTypedAdapterRegistryV1::resolve(operation_id)
}

fn contract_registration(operation_id: &str) -> Option<LegacyContractRegistrationV1> {
    semantic_registration(operation_id)
        .map(LegacySemanticRegistrationV1::contract)
        .or_else(|| {
            EXACT_BUSINESS_ADAPTERS
                .iter()
                .copied()
                .find(|registration| registration.operation_id == operation_id)
        })
}

fn semantic_response(
    adapter: LegacySemanticAdapterV1,
    request: &LegacyAdapterRequestV1,
    static_authority: &LegacyStaticAdapterAuthorityV1,
) -> Result<LegacyHttpTransportResponseV1, LegacyExecutionErrorV1> {
    let query_version = request.query().first("version");
    let request_origin = request.headers().get("origin");
    let allow_origin = changelog_feed_allow_origin(
        request_origin,
        request.fallback_origin.as_deref(),
        request.configured_origin.as_deref(),
    );
    match adapter {
        LegacySemanticAdapterV1::PublicStatusOk => LegacyHttpTransportResponseV1::new(
            200,
            Some(LEGACY_STATUS_CONTENT_TYPE),
            LEGACY_STATUS_BODY.as_bytes().to_vec(),
            Vec::new(),
            64 * 1_024,
        ),
        LegacySemanticAdapterV1::MediaServerRootMetadata => LegacyHttpTransportResponseV1::new(
            200,
            Some(LEGACY_MEDIA_SERVER_ROOT_CONTENT_TYPE),
            LEGACY_MEDIA_SERVER_ROOT_BODY.as_bytes().to_vec(),
            Vec::new(),
            64 * 1_024,
        ),
        LegacySemanticAdapterV1::ChangelogStatusGet => LegacyHttpTransportResponseV1::new(
            200,
            Some(LEGACY_CHANGELOG_CONTENT_TYPE),
            if query_version.is_some_and(|version| {
                !version.is_empty() && version == LEGACY_CHANGELOG_LATEST_VERSION
            }) {
                LEGACY_CHANGELOG_UPDATE_BODY
            } else {
                LEGACY_CHANGELOG_CURRENT_BODY
            }
            .as_bytes()
            .to_vec(),
            static_headers(LEGACY_CHANGELOG_CORS_HEADERS),
            64 * 1_024,
        ),
        LegacySemanticAdapterV1::ChangelogStatusOptions => LegacyHttpTransportResponseV1::new(
            204,
            None,
            Vec::new(),
            static_headers(LEGACY_CHANGELOG_CORS_HEADERS),
            64 * 1_024,
        ),
        LegacySemanticAdapterV1::ChangelogFeedGet => LegacyHttpTransportResponseV1::new(
            200,
            Some(LEGACY_CHANGELOG_FEED_CONTENT_TYPE),
            LEGACY_CHANGELOG_FEED_BODY.as_bytes().to_vec(),
            changelog_feed_headers(allow_origin, false),
            LEGACY_CHANGELOG_FEED_GET_MAX_RESPONSE_BYTES,
        ),
        LegacySemanticAdapterV1::ChangelogFeedOptions => LegacyHttpTransportResponseV1::new(
            204,
            None,
            Vec::new(),
            changelog_feed_headers(allow_origin, true),
            64 * 1_024,
        ),
        LegacySemanticAdapterV1::MobileSessionConfigGet => {
            let (google_auth_available, workos_auth_available) =
                static_authority.mobile_auth_config();
            let body = match (google_auth_available, workos_auth_available) {
                (false, false) => LEGACY_MOBILE_SESSION_CONFIG_NONE_BODY,
                (true, false) => LEGACY_MOBILE_SESSION_CONFIG_GOOGLE_BODY,
                (false, true) => LEGACY_MOBILE_SESSION_CONFIG_WORKOS_BODY,
                (true, true) => LEGACY_MOBILE_SESSION_CONFIG_BOTH_BODY,
            };
            LegacyHttpTransportResponseV1::new(
                200,
                Some(LEGACY_MOBILE_SESSION_CONFIG_CONTENT_TYPE),
                body.as_bytes().to_vec(),
                Vec::new(),
                64 * 1_024,
            )
        }
        LegacySemanticAdapterV1::NotificationPreferencesGet => {
            Err(LegacyExecutionErrorV1::Unsupported)
        }
    }
}

fn notification_preferences_request_fingerprint(
    request: &LegacyAdapterRequestV1,
    authenticated: &LegacyAuthenticatedContextV1,
) -> ChecksumSha256 {
    let mut canonical = Vec::with_capacity(
        request.operation_id().len()
            + request.method().len()
            + request.raw_path().len()
            + authenticated.principal_id().len()
            + 48,
    );
    for part in [
        "legacy-notification-preferences-v1",
        request.operation_id(),
        request.method(),
        request.raw_path(),
        authenticated.principal_id(),
    ] {
        canonical.extend_from_slice(part.as_bytes());
        canonical.push(0);
    }
    ChecksumSha256::digest_bytes(&canonical)
}

fn static_headers(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect()
}

fn changelog_feed_headers(allow_origin: &str, preflight: bool) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Access-Control-Allow-Origin".into(), allow_origin.into()),
        ("Access-Control-Allow-Credentials".into(), "true".into()),
    ];
    if preflight {
        headers.extend([
            ("Access-Control-Allow-Methods".into(), "GET, OPTIONS".into()),
            ("Access-Control-Allow-Headers".into(), "Content-Type".into()),
        ]);
    }
    headers
}

fn changelog_feed_allow_origin<'origin>(
    request_origin: Option<&'origin str>,
    fallback_origin: Option<&'origin str>,
    configured_origin: Option<&str>,
) -> &'origin str {
    let allowed = |origin: &str| {
        configured_origin == Some(origin) || LEGACY_CHANGELOG_ALLOWED_ORIGINS.contains(&origin)
    };
    request_origin
        .filter(|origin| !origin.is_empty() && allowed(origin))
        .or_else(|| fallback_origin.filter(|origin| allowed(origin)))
        .unwrap_or("null")
}

fn semantic_request_fingerprint(
    adapter: LegacySemanticAdapterV1,
    request: &LegacyAdapterRequestV1,
    static_authority: &LegacyStaticAdapterAuthorityV1,
) -> ChecksumSha256 {
    let allow_origin = changelog_feed_allow_origin(
        request.headers().get("origin"),
        request.fallback_origin.as_deref(),
        request.configured_origin.as_deref(),
    );
    let semantic_variant = match adapter {
        LegacySemanticAdapterV1::PublicStatusOk => "status-ok",
        LegacySemanticAdapterV1::MediaServerRootMetadata => "media-server-root",
        LegacySemanticAdapterV1::ChangelogStatusGet
            if request.query().first("version").is_some_and(|version| {
                !version.is_empty() && version == LEGACY_CHANGELOG_LATEST_VERSION
            }) =>
        {
            "has-update"
        }
        LegacySemanticAdapterV1::ChangelogStatusGet => "current",
        LegacySemanticAdapterV1::ChangelogStatusOptions => "status-preflight",
        LegacySemanticAdapterV1::ChangelogFeedGet => allow_origin,
        LegacySemanticAdapterV1::ChangelogFeedOptions => allow_origin,
        LegacySemanticAdapterV1::MobileSessionConfigGet => {
            match static_authority.mobile_auth_config() {
                (false, false) => "google=false;workos=false",
                (true, false) => "google=true;workos=false",
                (false, true) => "google=false;workos=true",
                (true, true) => "google=true;workos=true",
            }
        }
        LegacySemanticAdapterV1::NotificationPreferencesGet => "d1-user-preferences",
    };
    let mut canonical = Vec::with_capacity(
        request.operation_id().len()
            + request.method().len()
            + request.raw_path().len()
            + semantic_variant.len()
            + 32,
    );
    for part in [
        "legacy-typed-adapter-v1",
        request.operation_id(),
        request.method(),
        request.raw_path(),
        semantic_variant,
    ] {
        canonical.extend_from_slice(part.as_bytes());
        canonical.push(0);
    }
    ChecksumSha256::digest_bytes(&canonical)
}

/// Synthetic compatibility-journal harness retained for local concurrency and
/// corruption proofs. It is not part of the typed adapter registry, its
/// durable allowlist is permanently empty, and it cannot establish business
/// success evidence or promote a report row.
pub struct D1LegacyOperationExecutionPortV1<'database> {
    database: &'database D1Database,
}

impl<'database> D1LegacyOperationExecutionPortV1<'database> {
    #[must_use]
    pub const fn new_fail_closed(database: &'database D1Database) -> Self {
        Self { database }
    }

    fn statement(
        &self,
        sql: &str,
        bindings: &[JsValue],
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.database
            .prepare(sql)
            .bind(bindings)
            .map_err(|_| LegacyExecutionErrorV1::Internal)
    }

    async fn load(
        &self,
        scope_digest: &str,
        operation_id: &str,
        idempotency_key_digest: &str,
    ) -> Result<Option<ExecutionRow>, LegacyExecutionErrorV1> {
        self.statement(
            LOAD_SQL,
            &[
                JsValue::from_str(scope_digest),
                JsValue::from_str(operation_id),
                JsValue::from_str(idempotency_key_digest),
            ],
        )?
        .first::<ExecutionRow>(None)
        .into_send()
        .await
        .map_err(|_| LegacyExecutionErrorV1::Internal)
    }
}

#[async_trait]
impl LegacyOperationExecutionPortV1 for D1LegacyOperationExecutionPortV1<'_> {
    async fn execute_once(
        &self,
        command: &LegacyExecutionCommandV1,
    ) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1> {
        if !ENABLED_DURABLE_ADAPTERS.contains(&command.operation_id()) {
            return Err(LegacyExecutionErrorV1::Unsupported);
        }

        let now_ms = current_time_ms()?;
        let idempotency_key_digest = digest_parts(
            "legacy-idempotency-v1",
            &[
                command.scope_digest().as_str(),
                command.operation_id(),
                command.idempotency_key().map_or(
                    command.correlation_id(),
                    frame_domain::IdempotencyKey::expose,
                ),
            ],
        );
        let reservation_nonce = Uuid::now_v7().to_string();
        let reservation_digest = digest_parts(
            "legacy-reservation-v1",
            &[reservation_nonce.as_str(), command.operation_id()],
        );
        let result_digest = digest_parts(
            "legacy-accepted-result-v1",
            &[
                command.scope_digest().as_str(),
                command.operation_id(),
                idempotency_key_digest.as_str(),
                command.request_fingerprint().as_str(),
            ],
        );
        let audit_id = digest_parts(
            "legacy-audit-v1",
            &[reservation_digest.as_str(), result_digest.as_str()],
        );
        let correlation_digest = digest_parts("legacy-correlation-v1", &[command.correlation_id()]);
        let values = BoundExecutionValues {
            scope_digest: command.scope_digest().as_str(),
            operation_id: command.operation_id(),
            idempotency_key_digest: &idempotency_key_digest,
            request_fingerprint: command.request_fingerprint().as_str(),
            reservation_digest: &reservation_digest,
            response_status: 202,
            result_digest: &result_digest,
            now_ms,
            audit_id: &audit_id,
            audit_action: command.audit_action(),
            correlation_digest: &correlation_digest,
        };
        let statements = vec![
            self.claim_statement(&values)?,
            self.intent_statement(&values)?,
            self.complete_statement(&values)?,
            self.audit_statement(&values)?,
        ];
        let results = self
            .database
            .batch(statements)
            .into_send()
            .await
            .map_err(|_| LegacyExecutionErrorV1::Internal)?;
        if results.len() != 4 || results.iter().any(|result| !result.success()) {
            return Err(LegacyExecutionErrorV1::Internal);
        }

        let row = self
            .load(
                command.scope_digest().as_str(),
                command.operation_id(),
                &idempotency_key_digest,
            )
            .await?
            .ok_or(LegacyExecutionErrorV1::Internal)?;
        execution_outcome(
            row,
            command.request_fingerprint().as_str(),
            &reservation_digest,
        )
    }
}

struct BoundExecutionValues<'value> {
    scope_digest: &'value str,
    operation_id: &'value str,
    idempotency_key_digest: &'value str,
    request_fingerprint: &'value str,
    reservation_digest: &'value str,
    response_status: u16,
    result_digest: &'value str,
    now_ms: i64,
    audit_id: &'value str,
    audit_action: &'value str,
    correlation_digest: &'value str,
}

impl D1LegacyOperationExecutionPortV1<'_> {
    fn claim_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            CLAIM_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_f64(values.now_ms as f64),
            ],
        )
    }

    fn intent_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            INTENT_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_f64(values.now_ms as f64),
            ],
        )
    }

    fn complete_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            COMPLETE_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_f64(f64::from(values.response_status)),
                JsValue::from_str(values.result_digest),
                JsValue::from_f64(values.now_ms as f64),
            ],
        )
    }

    fn audit_statement(
        &self,
        values: &BoundExecutionValues<'_>,
    ) -> Result<D1PreparedStatement, LegacyExecutionErrorV1> {
        self.statement(
            AUDIT_SQL,
            &[
                JsValue::from_str(values.scope_digest),
                JsValue::from_str(values.operation_id),
                JsValue::from_str(values.idempotency_key_digest),
                JsValue::from_str(values.reservation_digest),
                JsValue::from_str(values.request_fingerprint),
                JsValue::from_f64(f64::from(values.response_status)),
                JsValue::from_str(values.result_digest),
                JsValue::from_f64(values.now_ms as f64),
                JsValue::from_str(values.audit_id),
                JsValue::from_str(values.audit_action),
                JsValue::from_str(values.correlation_digest),
            ],
        )
    }
}

pub struct LegacyCompatibilityTransportV1<'database> {
    registry: LegacyCompatibilityRegistryV1,
    static_authority: LegacyStaticAdapterAuthorityV1,
    database: Option<&'database D1Database>,
}

/// Internal server-action invocation. No HTTP path is synthesized for an
/// `ACTION` identity; a future Leptos action ingress must construct this only
/// after its trusted session boundary has populated the exact request facts.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct LegacyWebActiveOrganizationActionInvocationV1 {
    pub(crate) caller: LegacyCallerV1,
    pub(crate) envelope: ApiMutationEnvelopeV1,
    pub(crate) security: RequestSecurityContextV1,
    /// Trusted session context produced by the authentication ingress. The
    /// mutation actor is derived from this value and is never caller-supplied a
    /// second time.
    pub(crate) authenticated: LegacyAuthenticatedContextV1,
    pub(crate) legacy_organization_id: String,
    pub(crate) occurred_at: frame_domain::TimestampMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum LegacyWebActiveOrganizationActionEffectV1 {
    /// Consume this effect, invalidate `/dashboard`, then resolve the external
    /// JavaScript server action with `undefined`.
    InvalidateThenResolveVoid { path: &'static str },
}

#[cfg_attr(not(test), allow(dead_code))]
struct LegacyExactOperationAdmissionV1 {
    operation_id: &'static str,
    kind: LegacyOperationKindV1,
    method: &'static str,
    legacy_identity: &'static str,
    caller: LegacyCallerV1,
    envelope: ApiMutationEnvelopeV1,
    security: RequestSecurityContextV1,
}

impl LegacyCompatibilityTransportV1<'static> {
    /// Construct the exact static compatibility surface without introducing a
    /// database dependency into a legacy operation that never had one.
    pub fn new_static_only(
        compatibility: ClientCompatibilityPolicyV1,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        let registry = fail_closed_registry(compatibility)?;
        Ok(Self {
            registry,
            static_authority: LegacyStaticAdapterAuthorityV1::default(),
            database: None,
        })
    }

    /// Construct the static compatibility surface with presence-only Worker
    /// configuration. No binding value crosses this typed authority boundary.
    pub fn new_static_from_worker_env(
        compatibility: ClientCompatibilityPolicyV1,
        env: &Env,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        let registry = fail_closed_registry(compatibility)?;
        Ok(Self {
            registry,
            static_authority: LegacyStaticAdapterAuthorityV1::from_worker_env(env),
            database: None,
        })
    }

    #[cfg(test)]
    fn new_static_with_authority(
        compatibility: ClientCompatibilityPolicyV1,
        static_authority: LegacyStaticAdapterAuthorityV1,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        let registry = fail_closed_registry(compatibility)?;
        Ok(Self {
            registry,
            static_authority,
            database: None,
        })
    }
}

impl<'database> LegacyCompatibilityTransportV1<'database> {
    pub fn new_fail_closed(
        database: &'database D1Database,
        compatibility: ClientCompatibilityPolicyV1,
    ) -> Result<Self, LegacyRegistryErrorV1> {
        let registry = fail_closed_registry(compatibility)?;
        Ok(Self {
            registry,
            static_authority: LegacyStaticAdapterAuthorityV1::default(),
            database: Some(database),
        })
    }

    pub async fn dispatch_http(
        &self,
        input: LegacyHttpTransportRequestV1,
    ) -> Result<LegacyEndpointOutcomeV1, ApiErrorV1> {
        let execution = self.dispatch_typed(input).await?;
        let receipt = LegacyOperationReceiptV1::new(
            execution.response.status(),
            ChecksumSha256::digest_bytes(execution.response.body()),
        )
        .map_err(|_| public_error(ApiErrorCodeV1::Internal, "legacy-adapter-receipt"))?;
        Ok(if execution.replayed {
            LegacyEndpointOutcomeV1::Replay(receipt)
        } else {
            LegacyEndpointOutcomeV1::Completed(receipt)
        })
    }

    pub async fn dispatch_http_response(
        &self,
        input: LegacyHttpTransportRequestV1,
    ) -> Result<LegacyHttpTransportResponseV1, ApiErrorV1> {
        Ok(self.dispatch_typed(input).await?.response)
    }

    /// Resolve an operation by all four frozen identity components before
    /// applying compatibility admission. This boundary is transport-neutral:
    /// HTTP callers continue through `resolve_http`, while server actions use
    /// `ServerAction + ACTION + action://...#symbol` without inventing a path.
    #[cfg_attr(not(test), allow(dead_code))]
    fn admit_exact_operation(
        &self,
        request: LegacyExactOperationAdmissionV1,
    ) -> Result<(), ApiErrorV1> {
        let correlation_id = request.envelope.correlation_id.clone();
        let registration = contract_registration(request.operation_id)
            .filter(|registration| {
                registration.kind == "server_action"
                    && request.kind == LegacyOperationKindV1::ServerAction
                    && registration.method == request.method
                    && registration.legacy_path == request.legacy_identity
            })
            .ok_or_else(|| public_error(ApiErrorCodeV1::NotFound, &correlation_id))?;
        let contract = self
            .registry
            .resolve_exact(request.kind, request.method, request.legacy_identity)
            .filter(|contract| contract.id() == request.operation_id)
            .ok_or_else(|| public_error(ApiErrorCodeV1::NotFound, &correlation_id))?;
        let compatibility_request = LegacyCompatibilityRequestV1 {
            operation_id: registration.operation_id.to_owned(),
            caller: request.caller,
            envelope: request.envelope,
            security: request.security,
        };
        match self.registry.admit(&compatibility_request)? {
            LegacyCompatibilityOutcomeV1::ServeFrame(admission)
                if admission.operation_id == contract.id() =>
            {
                Ok(())
            }
            LegacyCompatibilityOutcomeV1::ServeFrame(_)
            | LegacyCompatibilityOutcomeV1::UseLegacyFallback(_)
            | LegacyCompatibilityOutcomeV1::RetirementResponse(_) => Err(public_error(
                ApiErrorCodeV1::TemporarilyUnavailable,
                &correlation_id,
            )),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn dispatch_web_active_organization_action<Repository>(
        &self,
        repository: &Repository,
        invocation: LegacyWebActiveOrganizationActionInvocationV1,
    ) -> Result<LegacyWebActiveOrganizationActionEffectV1, LegacyOrganizationSelectionErrorV1>
    where
        Repository: LegacyOrganizationSelectionRepositoryV1,
    {
        if !invocation.security.authenticated {
            return Err(LegacyOrganizationSelectionErrorV1::Unauthorized);
        }
        let actor_id = frame_domain::UserId::parse(invocation.authenticated.principal_id())
            .map_err(|_| LegacyOrganizationSelectionErrorV1::Unauthorized)?;
        let selection = LegacyOrganizationSelectionRequestV1 {
            credential: Some(frame_application::LegacyOrganizationSelectionCredentialV1::Session),
            actor_id: Some(actor_id),
            legacy_organization_id: invocation.legacy_organization_id,
            occurred_at: invocation.occurred_at,
        };
        self.admit_exact_operation(LegacyExactOperationAdmissionV1 {
            operation_id: LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID,
            kind: LegacyOperationKindV1::ServerAction,
            method: "ACTION",
            legacy_identity: LEGACY_WEB_ACTIVE_ORGANIZATION_IDENTITY,
            caller: invocation.caller,
            envelope: invocation.envelope,
            security: invocation.security,
        })
        .map_err(action_admission_error)?;
        match LegacyOrganizationSelectionAdapterV1::web_navbar_server_action()
            .execute(repository, &selection)
            .await?
        {
            LegacyOrganizationSelectionOutcomeV1::WebActionVoid { invalidate_path }
                if invalidate_path == LEGACY_WEB_DASHBOARD_INVALIDATION_PATH =>
            {
                Ok(
                    LegacyWebActiveOrganizationActionEffectV1::InvalidateThenResolveVoid {
                        path: invalidate_path,
                    },
                )
            }
            LegacyOrganizationSelectionOutcomeV1::WebActionVoid { .. } => {
                Err(LegacyOrganizationSelectionErrorV1::Internal)
            }
        }
    }

    async fn dispatch_typed(
        &self,
        input: LegacyHttpTransportRequestV1,
    ) -> Result<LegacyTypedAdapterExecutionV1, ApiErrorV1> {
        let correlation_id = input.envelope.correlation_id.clone();
        let operation_id = self
            .registry
            .resolve_http(&input.method, &input.raw_path)
            .map(|contract| contract.id().to_owned())
            .ok_or_else(|| public_error(ApiErrorCodeV1::NotFound, &correlation_id))?;
        let registration = semantic_registration(&operation_id)
            .ok_or_else(|| public_error(ApiErrorCodeV1::Unsupported, &correlation_id))?;
        let (adapter_request, admission_request) =
            LegacyAdapterRequestV1::from_transport(input, registration).map_err(|error| {
                public_error(legacy_execution_error_code(error), &correlation_id)
            })?;
        match self.registry.admit(&admission_request)? {
            LegacyCompatibilityOutcomeV1::ServeFrame(_) => {}
            LegacyCompatibilityOutcomeV1::UseLegacyFallback(_)
            | LegacyCompatibilityOutcomeV1::RetirementResponse(_) => {
                return Err(public_error(
                    ApiErrorCodeV1::TemporarilyUnavailable,
                    &correlation_id,
                ));
            }
        }
        LegacyTypedAdapterRegistryV1::dispatch(
            registration,
            &adapter_request,
            &self.static_authority,
            self.database,
        )
        .await
        .map_err(|error| public_error(legacy_execution_error_code(error), &correlation_id))
    }
}

fn fail_closed_registry(
    compatibility: ClientCompatibilityPolicyV1,
) -> Result<LegacyCompatibilityRegistryV1, LegacyRegistryErrorV1> {
    let report: Report =
        serde_json::from_str(REPORT).map_err(|_| LegacyRegistryErrorV1::InvalidContract)?;
    let contracts = report
        .entries
        .into_iter()
        .map(evidence_gated_contract)
        .collect::<Result<Vec<_>, _>>()?;
    LegacyCompatibilityRegistryV1::new(contracts, compatibility)
}

fn evidence_gated_contract(
    row: ReportRow,
) -> Result<LegacyOperationContractV1, LegacyRegistryErrorV1> {
    let registration = contract_registration(&row.id);
    let enabled = ENABLED_SEMANTIC_ADAPTERS.contains(&row.id.as_str())
        || ENABLED_EXACT_BUSINESS_ADAPTERS.contains(&row.id.as_str());
    if enabled != registration.is_some() {
        return Err(LegacyRegistryErrorV1::InvalidContract);
    }
    let endpoint_promoted = if let Some(registration) = registration {
        let sources_match = registration.sources.iter().all(|expected| {
            row.sources
                .iter()
                .any(|source| source.path == expected.path && source.sha256 == expected.sha256)
        });
        if registration.kind != row.kind
            || registration.method != row.method
            || registration.legacy_path != row.legacy_path
            || !row
                .clients
                .iter()
                .map(String::as_str)
                .eq(registration.clients.iter().copied())
            || row.auth != registration.auth
            || row.disposition != "replace"
            || row.security.max_body_bytes != 0
            || !row.security.accepted_content_types.is_empty()
            || row.security.rate_limit_bucket != registration.rate_limit_bucket
            || row.security.idempotency != "forbidden"
            || row.contract_evidence.success != "local_contract"
            || row.contract_evidence.validation != "local_contract"
            || row.contract_evidence.authorization != "local_contract"
            || row.contract_evidence.idempotency_retry != "local_contract"
            || row.contract_evidence.failure != "local_contract"
            || !sources_match
        {
            return Err(LegacyRegistryErrorV1::InvalidContract);
        }
        true
    } else {
        if row.contract_evidence.success == "local_contract" {
            return Err(LegacyRegistryErrorV1::InvalidContract);
        }
        false
    };
    let kind = match row.kind.as_str() {
        "route" => LegacyOperationKindV1::HttpRoute,
        "rpc" => LegacyOperationKindV1::Rpc,
        "server_action" => LegacyOperationKindV1::ServerAction,
        "workflow" => LegacyOperationKindV1::Workflow,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let auth = match row.auth.as_str() {
        "public_or_flow_token" => ApiAuthClassV1::Public,
        "optional_session_or_share_capability" => ApiAuthClassV1::OptionalSession,
        "session" => ApiAuthClassV1::Session,
        "session_or_api_key" => ApiAuthClassV1::SessionOrApiKey,
        "developer_api_key" => ApiAuthClassV1::ApiKey,
        "internal_service" => ApiAuthClassV1::Worker,
        "signed_webhook" => ApiAuthClassV1::Webhook,
        "scheduler_secret" => ApiAuthClassV1::Scheduler,
        "admin_session" => ApiAuthClassV1::Admin,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let idempotency = match row.security.idempotency.as_str() {
        "required" => IdempotencyRequirementV1::Required,
        "optional" => IdempotencyRequirementV1::Optional,
        "forbidden" => IdempotencyRequirementV1::Forbidden,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let disposition = match row.disposition.as_str() {
        "replace" | "protected_parity_required" => LegacyEndpointDispositionV1::Replace,
        "migrate" => LegacyEndpointDispositionV1::Migrate,
        "retire" => LegacyEndpointDispositionV1::Retire,
        _ => return Err(LegacyRegistryErrorV1::InvalidContract),
    };
    let clients = row
        .clients
        .into_iter()
        .map(|client| match client.as_str() {
            "web" => Ok(LegacyClientFamilyV1::Web),
            "desktop" => Ok(LegacyClientFamilyV1::Desktop),
            "mobile" => Ok(LegacyClientFamilyV1::Mobile),
            "extension" => Ok(LegacyClientFamilyV1::Extension),
            "developer" => Ok(LegacyClientFamilyV1::Developer),
            "internal_worker" => Ok(LegacyClientFamilyV1::InternalWorker),
            "provider" => Ok(LegacyClientFamilyV1::Provider),
            "scheduler" => Ok(LegacyClientFamilyV1::Scheduler),
            _ => Err(LegacyRegistryErrorV1::InvalidContract),
        })
        .collect::<Result<Vec<_>, _>>()?;
    LegacyOperationContractV1::new(
        row.id.clone(),
        kind,
        row.method,
        row.legacy_path,
        clients,
        ApiRequestPolicyV1 {
            auth,
            max_body_bytes: row.security.max_body_bytes,
            accepted_content_types: row.security.accepted_content_types,
            idempotency,
            rate_limit_bucket: row.security.rate_limit_bucket,
            audit_action: format!("legacy.{}", row.id),
        },
        disposition,
        LegacyOperationEvidenceV1 {
            endpoint_contract_proven: endpoint_promoted,
            client_family_enabled: endpoint_promoted,
            legacy_fallback_available: false,
            retirement_approved: false,
        },
    )
}

fn execution_outcome(
    row: ExecutionRow,
    expected_request_fingerprint: &str,
    reservation_digest: &str,
) -> Result<LegacyExecutionOutcomeV1, LegacyExecutionErrorV1> {
    if row.request_fingerprint != expected_request_fingerprint {
        return Err(LegacyExecutionErrorV1::Conflict);
    }
    if row.state == "pending" {
        return Ok(LegacyExecutionOutcomeV1::InFlight);
    }
    let (Some(status), Some(result_digest)) = (row.response_status, row.result_digest) else {
        return Err(LegacyExecutionErrorV1::Internal);
    };
    if row.state != "complete"
        || row.intent_reservation_digest.as_deref() != Some(row.reservation_digest.as_str())
        || row.audit_reservation_digest.as_deref() != Some(row.reservation_digest.as_str())
    {
        return Err(LegacyExecutionErrorV1::Internal);
    }
    let receipt = LegacyOperationReceiptV1::new(
        status,
        ChecksumSha256::parse(result_digest).map_err(|_| LegacyExecutionErrorV1::Internal)?,
    )
    .map_err(|_| LegacyExecutionErrorV1::Internal)?;
    if row.reservation_digest == reservation_digest {
        Ok(LegacyExecutionOutcomeV1::Completed(receipt))
    } else {
        Ok(LegacyExecutionOutcomeV1::Replay(receipt))
    }
}

fn current_time_ms() -> Result<i64, LegacyExecutionErrorV1> {
    let value = js_sys::Date::now();
    if !value.is_finite() || value < 0.0 || value > MAX_SAFE_INTEGER as f64 {
        return Err(LegacyExecutionErrorV1::Internal);
    }
    Ok(value as i64)
}

fn legacy_execution_error_code(error: LegacyExecutionErrorV1) -> ApiErrorCodeV1 {
    match error {
        LegacyExecutionErrorV1::Invalid => ApiErrorCodeV1::InvalidRequest,
        LegacyExecutionErrorV1::NotFound => ApiErrorCodeV1::NotFound,
        LegacyExecutionErrorV1::Conflict => ApiErrorCodeV1::Conflict,
        LegacyExecutionErrorV1::Unsupported => ApiErrorCodeV1::Unsupported,
        LegacyExecutionErrorV1::TemporarilyUnavailable => ApiErrorCodeV1::TemporarilyUnavailable,
        LegacyExecutionErrorV1::Indeterminate => ApiErrorCodeV1::Indeterminate,
        LegacyExecutionErrorV1::Internal => ApiErrorCodeV1::Internal,
    }
}

fn valid_authority_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_AUTHORITY_ID_BYTES
        && value.is_ascii()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'@')
        })
}

fn valid_optional_origin(value: Option<&str>) -> bool {
    value.is_none_or(|origin| {
        !origin.is_empty()
            && origin.len() <= MAX_HEADER_VALUE_BYTES
            && origin.is_ascii()
            && !origin.bytes().any(|byte| byte.is_ascii_control())
    })
}

fn valid_header_name(value: &str) -> bool {
    value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
    })
}

fn valid_percent_encoding(value: &str) -> bool {
    value.split(['&', '=']).all(|component| {
        let bytes = component.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'%' if index + 2 < bytes.len() => {
                    let Some(high) = hex_value(bytes[index + 1]) else {
                        return false;
                    };
                    let Some(low) = hex_value(bytes[index + 2]) else {
                        return false;
                    };
                    decoded.push(high * 16 + low);
                    index += 3;
                }
                b'%' => return false,
                b'+' => {
                    decoded.push(b' ');
                    index += 1;
                }
                byte => {
                    decoded.push(byte);
                    index += 1;
                }
            }
        }
        std::str::from_utf8(&decoded).is_ok_and(|decoded| !decoded.chars().any(char::is_control))
    })
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn extract_matched_params(pattern: &str, raw_path: &str) -> Option<Vec<LegacyMatchedParamV1>> {
    let expected = pattern.split('/').skip(1).collect::<Vec<_>>();
    let actual = raw_path.split('/').skip(1).collect::<Vec<_>>();
    let mut actual_index = 0;
    let mut matched = Vec::<LegacyMatchedParamV1>::new();
    for (expected_index, segment) in expected.iter().enumerate() {
        if let Some(raw_name) = segment.strip_prefix(':') {
            let wildcard = raw_name.ends_with('*');
            let name = raw_name.strip_suffix('*').unwrap_or(raw_name);
            if name.is_empty()
                || name.len() > MAX_PARAM_NAME_BYTES
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || matched.iter().any(|parameter| parameter.name == name)
                || wildcard && expected_index + 1 != expected.len()
            {
                return None;
            }
            let raw_value = if wildcard {
                let remainder = actual.get(actual_index..)?;
                if remainder.is_empty() || !remainder.iter().all(|value| safe_param_segment(value))
                {
                    return None;
                }
                actual_index = actual.len();
                remainder.join("/")
            } else {
                let value = *actual.get(actual_index)?;
                if !safe_param_segment(value) {
                    return None;
                }
                actual_index += 1;
                value.to_owned()
            };
            if raw_value.len() > MAX_PARAM_VALUE_BYTES || matched.len() >= MAX_MATCHED_PARAMS {
                return None;
            }
            matched.push(LegacyMatchedParamV1 {
                name: name.to_owned(),
                raw_value,
            });
        } else {
            if actual.get(actual_index).copied() != Some(*segment) {
                return None;
            }
            actual_index += 1;
        }
    }
    (actual_index == actual.len()).then_some(matched)
}

fn safe_param_segment(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn digest_parts(domain: &str, parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    for part in parts {
        digest.update([0]);
        digest.update(part.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn public_error(code: ApiErrorCodeV1, correlation_id: &str) -> ApiErrorV1 {
    ApiErrorV1::new(code, correlation_id, None).unwrap_or_else(|_| {
        ApiErrorV1::new(code, "invalid-correlation", None).expect("fixed correlation ID is valid")
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn action_admission_error(error: ApiErrorV1) -> LegacyOrganizationSelectionErrorV1 {
    match error.code {
        ApiErrorCodeV1::Unauthenticated => LegacyOrganizationSelectionErrorV1::Unauthorized,
        ApiErrorCodeV1::NotFound => LegacyOrganizationSelectionErrorV1::OrganizationNotFound,
        ApiErrorCodeV1::RateLimited | ApiErrorCodeV1::TemporarilyUnavailable => {
            LegacyOrganizationSelectionErrorV1::AuthorityUnavailable
        }
        ApiErrorCodeV1::InvalidRequest
        | ApiErrorCodeV1::Conflict
        | ApiErrorCodeV1::Unsupported
        | ApiErrorCodeV1::UpgradeRequired
        | ApiErrorCodeV1::Indeterminate
        | ApiErrorCodeV1::Internal => LegacyOrganizationSelectionErrorV1::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use frame_application::RateLimitDecisionV1;
    use frame_domain::{
        ClientCompatibilityPolicyV1, ClientReleaseV1, ClientSurfaceV1, IdempotencyKey,
        OrganizationOperationId, OrganizationRevision, TimestampMillis, UserId,
    };
    use frame_ports::{
        LegacySetActiveOrganizationCommandV1, OrganizationMutationReceipt,
        OrganizationMutationResult, OrganizationPortError,
    };
    use futures::executor::block_on;

    use super::*;

    #[derive(Default)]
    struct RecordingSelectionRepository {
        commands: Mutex<Vec<LegacySetActiveOrganizationCommandV1>>,
    }

    #[async_trait]
    impl LegacyOrganizationSelectionRepositoryV1 for RecordingSelectionRepository {
        async fn legacy_set_active_organization(
            &self,
            command: LegacySetActiveOrganizationCommandV1,
        ) -> Result<OrganizationMutationReceipt, OrganizationPortError> {
            self.commands.lock().expect("commands").push(command);
            Ok(OrganizationMutationReceipt {
                operation_id: OrganizationOperationId::new(),
                result: OrganizationMutationResult::Applied,
                subject_id: command.actor_id.to_string(),
                committed_at: command.occurred_at,
                resulting_revision: OrganizationRevision::new(8).expect("revision"),
                authority_version: OrganizationRevision::new(13).expect("authority"),
                replayed: false,
            })
        }
    }

    fn compatibility() -> ClientCompatibilityPolicyV1 {
        ClientCompatibilityPolicyV1 {
            api_major: 1,
            current_release: 42,
            previous_release: 41,
            deprecated_after_ms: Some(1_900_000_000_000),
            retired: false,
        }
    }

    #[test]
    fn production_registry_promotes_only_the_source_pinned_exact_contracts() {
        let registry = fail_closed_registry(compatibility()).expect("registry");
        assert_eq!(registry.len(), 288);
        let report = serde_json::from_str::<Report>(REPORT).expect("report");
        let mut promoted = 0;
        let mut fail_closed = 0;
        for contract in &report.entries {
            let stored = registry.contract(&contract.id).expect("contract");
            let expected = ENABLED_SEMANTIC_ADAPTERS.contains(&contract.id.as_str())
                || ENABLED_EXACT_BUSINESS_ADAPTERS.contains(&contract.id.as_str());
            assert_eq!(stored.evidence().endpoint_contract_proven, expected);
            assert_eq!(stored.evidence().client_family_enabled, expected);
            assert!(!stored.evidence().legacy_fallback_available);
            assert!(!stored.evidence().retirement_approved);
            if expected {
                assert_eq!(contract.contract_evidence.success, "local_contract");
                assert_eq!(contract.contract_evidence.validation, "local_contract");
                assert_eq!(contract.contract_evidence.authorization, "local_contract");
                assert_eq!(
                    contract.contract_evidence.idempotency_retry,
                    "local_contract"
                );
                assert_eq!(contract.contract_evidence.failure, "local_contract");
            } else {
                let caller = match stored.clients()[0] {
                    LegacyClientFamilyV1::Web => released_caller(ClientSurfaceV1::Web),
                    LegacyClientFamilyV1::Desktop => released_caller(ClientSurfaceV1::Desktop),
                    LegacyClientFamilyV1::Mobile => released_caller(ClientSurfaceV1::Mobile),
                    LegacyClientFamilyV1::Extension => released_caller(ClientSurfaceV1::Extension),
                    LegacyClientFamilyV1::Developer => released_caller(ClientSurfaceV1::Developer),
                    LegacyClientFamilyV1::InternalWorker => LegacyCallerV1::InternalWorker,
                    LegacyClientFamilyV1::Provider => LegacyCallerV1::Provider,
                    LegacyClientFamilyV1::Scheduler => LegacyCallerV1::Scheduler,
                };
                let error = registry
                    .admit(&request_for(stored, caller))
                    .expect_err("every unpromoted production operation must fail closed");
                assert_eq!(error.code, ApiErrorCodeV1::TemporarilyUnavailable);
                fail_closed += 1;
            }
            promoted += usize::from(expected);
        }
        assert_eq!(promoted, 9);
        assert_eq!(fail_closed, 279);
        assert_eq!(
            ENABLED_SEMANTIC_ADAPTERS,
            [
                LEGACY_STATUS_OPERATION_ID,
                LEGACY_MEDIA_SERVER_ROOT_OPERATION_ID,
                LEGACY_CHANGELOG_STATUS_GET_OPERATION_ID,
                LEGACY_CHANGELOG_STATUS_OPTIONS_OPERATION_ID,
                LEGACY_CHANGELOG_FEED_GET_OPERATION_ID,
                LEGACY_CHANGELOG_FEED_OPTIONS_OPERATION_ID,
                LEGACY_MOBILE_SESSION_CONFIG_OPERATION_ID,
                LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID,
            ]
        );
        assert_eq!(
            ENABLED_EXACT_BUSINESS_ADAPTERS,
            [LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID]
        );
        assert!(ENABLED_DURABLE_ADAPTERS.is_empty());

        let status = registry
            .contract(LEGACY_STATUS_OPERATION_ID)
            .expect("status contract");
        assert_eq!(status.method(), "GET");
        assert_eq!(status.legacy_identity(), LEGACY_STATUS_PATH);
        assert!(matches!(
            registry.admit(&request_for(status, released_caller(ClientSurfaceV1::Web))),
            Ok(frame_application::LegacyCompatibilityOutcomeV1::ServeFrame(
                _
            ))
        ));
        let web_action = registry
            .resolve_exact(
                LegacyOperationKindV1::ServerAction,
                "ACTION",
                LEGACY_WEB_ACTIVE_ORGANIZATION_IDENTITY,
            )
            .expect("web active-organization action");
        assert_eq!(web_action.id(), LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID);
        let mobile_active = registry
            .contract(frame_application::LEGACY_MOBILE_ACTIVE_ORGANIZATION_OPERATION_ID)
            .expect("mobile active-organization route");
        assert!(!mobile_active.evidence().endpoint_contract_proven);
        let mobile = registry
            .contract(LEGACY_MOBILE_SESSION_CONFIG_OPERATION_ID)
            .expect("mobile config contract");
        assert_eq!(mobile.method(), "GET");
        assert_eq!(mobile.legacy_identity(), LEGACY_MOBILE_SESSION_CONFIG_PATH);
        assert!(matches!(
            registry.admit(&request_for(
                mobile,
                released_caller(ClientSurfaceV1::Mobile)
            )),
            Ok(frame_application::LegacyCompatibilityOutcomeV1::ServeFrame(
                _
            ))
        ));
        for registration in SEMANTIC_ADAPTERS {
            assert_eq!(
                registration.adapter.operation_id(),
                registration.operation_id
            );
            let expected_authority =
                if registration.operation_id == LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID {
                    LegacyAdapterAuthorityRequirementV1::D1BusinessAuthority
                } else {
                    LegacyAdapterAuthorityRequirementV1::Static
                };
            assert_eq!(
                registration.adapter.authority_requirement(),
                expected_authority
            );
        }
    }

    fn released_caller(surface: ClientSurfaceV1) -> LegacyCallerV1 {
        LegacyCallerV1::Released(ClientReleaseV1 {
            surface,
            api_major: 1,
            release: 42,
        })
    }

    fn request_for(
        contract: &LegacyOperationContractV1,
        caller: LegacyCallerV1,
    ) -> LegacyCompatibilityRequestV1 {
        LegacyCompatibilityRequestV1 {
            operation_id: contract.id().to_owned(),
            caller,
            envelope: ApiMutationEnvelopeV1 {
                content_length: 0,
                content_type: None,
                idempotency_key: None,
                correlation_id: "legacy-runtime-test".into(),
            },
            security: RequestSecurityContextV1 {
                authenticated: true,
                authorized: true,
                browser_origin_valid: true,
                csrf_valid: true,
                rate_limit: RateLimitDecisionV1::Allowed,
            },
        }
    }

    fn static_transport_request(
        method: &str,
        raw_path: &str,
        caller: LegacyCallerV1,
        content_length: u64,
        idempotency_key: Option<IdempotencyKey>,
        security: RequestSecurityContextV1,
    ) -> LegacyHttpTransportRequestV1 {
        let body_length = usize::try_from(content_length).expect("test body length");
        let mut headers = Vec::new();
        if content_length > 0 {
            headers.push(("content-length".into(), content_length.to_string()));
        }
        if let Some(key) = idempotency_key {
            headers.push(("idempotency-key".into(), key.expose().to_owned()));
        }
        LegacyHttpTransportRequestV1::new(LegacyHttpTransportRequestPartsV1 {
            method: method.into(),
            raw_path: raw_path.into(),
            raw_query: String::new(),
            raw_body: vec![0; body_length],
            headers,
            caller,
            correlation_id: "legacy-static-transport".into(),
            security,
            scope_digest: ChecksumSha256::digest_bytes(
                format!("legacy-static-scope\0{raw_path}").as_bytes(),
            ),
            authenticated: None,
            revision: None,
            fallback_origin: None,
            configured_origin: None,
        })
        .expect("valid bounded test request")
    }

    fn with_query_version(
        mut request: LegacyHttpTransportRequestV1,
        version: Option<&str>,
    ) -> LegacyHttpTransportRequestV1 {
        let raw_query = version.map_or_else(String::new, |version| {
            url::form_urlencoded::Serializer::new(String::new())
                .append_pair("version", version)
                .finish()
        });
        request.query = LegacyCanonicalQueryV1::parse(&raw_query).expect("valid test query");
        request
    }

    fn with_cors(
        mut request: LegacyHttpTransportRequestV1,
        request_origin: Option<&str>,
        original_origin: &str,
        configured_origin: &str,
    ) -> LegacyHttpTransportRequestV1 {
        let mut headers = request.headers.values().to_vec();
        headers.retain(|(name, _)| name != "origin");
        if let Some(origin) = request_origin {
            headers.push(("origin".into(), origin.into()));
        }
        request.headers = LegacyNormalizedHeadersV1::new(headers).expect("valid test CORS headers");
        request.fallback_origin = Some(original_origin.into());
        request.configured_origin = Some(configured_origin.into());
        request
    }

    fn adapter_request(
        adapter: LegacySemanticAdapterV1,
        request: LegacyHttpTransportRequestV1,
    ) -> LegacyAdapterRequestV1 {
        let registration = semantic_registration(adapter.operation_id()).expect("registration");
        LegacyAdapterRequestV1::from_transport(request, registration)
            .expect("typed adapter request")
            .0
    }

    const fn allowed_public_security() -> RequestSecurityContextV1 {
        RequestSecurityContextV1 {
            authenticated: false,
            authorized: true,
            browser_origin_valid: true,
            csrf_valid: true,
            rate_limit: RateLimitDecisionV1::Allowed,
        }
    }

    const fn allowed_session_security() -> RequestSecurityContextV1 {
        RequestSecurityContextV1 {
            authenticated: true,
            authorized: true,
            browser_origin_valid: true,
            csrf_valid: true,
            rate_limit: RateLimitDecisionV1::Allowed,
        }
    }

    fn request_parts() -> LegacyHttpTransportRequestPartsV1 {
        LegacyHttpTransportRequestPartsV1 {
            method: "GET".into(),
            raw_path: LEGACY_STATUS_PATH.into(),
            raw_query: String::new(),
            raw_body: Vec::new(),
            headers: Vec::new(),
            caller: released_caller(ClientSurfaceV1::Web),
            correlation_id: "bounded-request-test".into(),
            security: allowed_public_security(),
            scope_digest: ChecksumSha256::digest_bytes(b"bounded-request-test"),
            authenticated: None,
            revision: None,
            fallback_origin: None,
            configured_origin: None,
        }
    }

    #[test]
    fn canonical_query_preserves_duplicate_order_and_rejects_hostile_encodings() {
        let query = LegacyCanonicalQueryV1::parse("tag=b&tag=a&value=%41+z&tag=c")
            .expect("canonical query");
        assert_eq!(query.canonical(), "tag=b&tag=a&value=A+z&tag=c");
        assert_eq!(query.first("tag"), Some("b"));
        assert_eq!(query.all("tag").collect::<Vec<_>>(), ["b", "a", "c"]);

        for hostile in ["bad=%", "bad=%0A", "bad=%FF", "bad=\n", "snowman=☃"] {
            assert_eq!(
                LegacyCanonicalQueryV1::parse(hostile),
                Err(LegacyExecutionErrorV1::Invalid),
                "hostile query must fail: {hostile:?}"
            );
        }
        let too_many = (0..=MAX_QUERY_PAIRS)
            .map(|index| format!("p{index}=v"))
            .collect::<Vec<_>>()
            .join("&");
        assert_eq!(
            LegacyCanonicalQueryV1::parse(&too_many),
            Err(LegacyExecutionErrorV1::Invalid)
        );
        let oversized = format!("value={}", "a".repeat(MAX_QUERY_COMPONENT_BYTES + 1));
        assert_eq!(
            LegacyCanonicalQueryV1::parse(&oversized),
            Err(LegacyExecutionErrorV1::Invalid)
        );
    }

    #[test]
    fn exact_path_params_are_bounded_and_hostile_segments_fail_closed() {
        let matched = extract_matched_params(
            "/api/videos/:video_id/parts/:tail*",
            "/api/videos/video_7/parts/segment-1/chunk_2",
        )
        .expect("exact params");
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].name(), "video_id");
        assert_eq!(matched[0].raw_value(), "video_7");
        assert_eq!(matched[1].name(), "tail");
        assert_eq!(matched[1].raw_value(), "segment-1/chunk_2");

        for hostile in [
            "/api/videos/%2F/parts/segment",
            "/api/videos/../parts/segment",
            "/api/videos/video/parts//segment",
            "/api/videos/video/parts/segment\\escape",
        ] {
            assert!(
                extract_matched_params("/api/videos/:video_id/parts/:tail*", hostile).is_none(),
                "hostile path must fail: {hostile}"
            );
        }
        assert!(extract_matched_params("/api/:id/:id", "/api/one/two").is_none());
        assert!(extract_matched_params("/api/:tail*/suffix", "/api/one/suffix").is_none());
    }

    #[test]
    fn mobile_session_binding_truthiness_matches_javascript_boolean_strings() {
        assert!(!binding_value_is_truthy(""));
        assert!(binding_value_is_truthy("client-id"));
        assert!(binding_value_is_truthy(" "));
    }

    #[test]
    fn normalized_headers_reject_duplicates_unknowns_controls_and_size_abuse() {
        let headers = LegacyNormalizedHeadersV1::new(vec![
            ("Origin".into(), "  https://cap.so\t".into()),
            ("Content-Length".into(), "0".into()),
        ])
        .expect("normalized headers");
        assert_eq!(headers.get("origin"), Some("https://cap.so"));
        assert_eq!(headers.get("content-length"), Some("0"));

        for values in [
            vec![("origin".into(), "a".into()), ("Origin".into(), "b".into())],
            vec![("x-unbounded".into(), "value".into())],
            vec![("origin".into(), "https://cap.so\r\ninjected: yes".into())],
            vec![("origin".into(), "a".repeat(MAX_HEADER_VALUE_BYTES + 1))],
        ] {
            assert_eq!(
                LegacyNormalizedHeadersV1::new(values),
                Err(LegacyExecutionErrorV1::Invalid)
            );
        }
    }

    #[test]
    fn bounded_request_binds_body_length_auth_tenant_and_revision() {
        let mut mismatch = request_parts();
        mismatch.raw_body = b"x".to_vec();
        mismatch.headers = vec![("content-length".into(), "0".into())];
        assert!(LegacyHttpTransportRequestV1::new(mismatch).is_err());

        let mut duplicate_length = request_parts();
        duplicate_length.headers = vec![
            ("content-length".into(), "0".into()),
            ("Content-Length".into(), "0".into()),
        ];
        assert!(LegacyHttpTransportRequestV1::new(duplicate_length).is_err());

        let mut oversized = request_parts();
        oversized.raw_body = vec![0; MAX_RAW_BODY_BYTES + 1];
        assert!(LegacyHttpTransportRequestV1::new(oversized).is_err());

        let actor = LegacyAuthenticatedContextV1::new("user_7", "tenant_9").expect("actor");
        let revision = LegacyRevisionContextV1::new("video_12", 44).expect("revision");
        let mut authenticated = request_parts();
        authenticated.security.authenticated = true;
        authenticated.authenticated = Some(actor.clone());
        authenticated.revision = Some(revision.clone());
        let authenticated = LegacyHttpTransportRequestV1::new(authenticated).expect("request");
        assert_eq!(
            authenticated.authenticated.as_ref(),
            Some(&actor),
            "principal and tenant remain bound"
        );
        assert_eq!(authenticated.revision.as_ref(), Some(&revision));

        let principal_only =
            LegacyAuthenticatedContextV1::principal_only("user_preferences").expect("principal");
        assert_eq!(principal_only.principal_id(), "user_preferences");
        assert_eq!(principal_only.tenant_id(), None);

        let mut missing_actor = request_parts();
        missing_actor.security.authenticated = true;
        assert!(LegacyHttpTransportRequestV1::new(missing_actor).is_err());
        let mut untrusted_actor = request_parts();
        untrusted_actor.authenticated = Some(actor);
        assert!(LegacyHttpTransportRequestV1::new(untrusted_actor).is_err());
        let mut revision_without_actor = request_parts();
        revision_without_actor.revision = Some(revision);
        assert!(LegacyHttpTransportRequestV1::new(revision_without_actor).is_err());
        assert!(LegacyAuthenticatedContextV1::new("bad\nprincipal", "tenant").is_err());
    }

    #[test]
    fn adapter_alone_derives_fingerprint_from_canonical_semantic_inputs() {
        let static_authority = LegacyStaticAdapterAuthorityV1::default();
        let authority = LegacyAdapterAuthorityV1::Static(&static_authority);
        let adapter = LegacySemanticAdapterV1::ChangelogStatusGet;
        let request = |raw_query: &str| {
            let mut request = static_transport_request(
                "GET",
                LEGACY_CHANGELOG_STATUS_PATH,
                released_caller(ClientSurfaceV1::Desktop),
                0,
                None,
                allowed_public_security(),
            );
            request.query = LegacyCanonicalQueryV1::parse(raw_query).expect("query");
            adapter_request(adapter, request)
        };
        let current = request("version=0.5.5&version=0.5.6");
        let current_equivalent = request("version=0.5.4&ignored=value");
        let update = request("version=0.5.6&version=0.5.5");
        assert_eq!(
            adapter
                .request_fingerprint(&current, authority)
                .expect("fingerprint"),
            adapter
                .request_fingerprint(&current_equivalent, authority)
                .expect("fingerprint")
        );
        assert_ne!(
            adapter
                .request_fingerprint(&current, authority)
                .expect("fingerprint"),
            adapter
                .request_fingerprint(&update, authority)
                .expect("fingerprint")
        );

        let feed_adapter = LegacySemanticAdapterV1::ChangelogFeedGet;
        let feed = |origin: &str, query: &str| {
            let mut request = with_cors(
                static_transport_request(
                    "GET",
                    LEGACY_CHANGELOG_FEED_PATH,
                    released_caller(ClientSurfaceV1::Desktop),
                    0,
                    None,
                    allowed_public_security(),
                ),
                Some(origin),
                "https://frame.engmanager.xyz",
                "https://frame.engmanager.xyz",
            );
            request.query = LegacyCanonicalQueryV1::parse(query).expect("query");
            adapter_request(feed_adapter, request)
        };
        let cap = feed("https://cap.so", "ignored=one");
        let cap_equivalent = feed("https://cap.so", "ignored=two&ignored=three");
        let cap_link = feed("https://cap.link", "ignored=one");
        assert_eq!(
            feed_adapter
                .request_fingerprint(&cap, authority)
                .expect("fingerprint"),
            feed_adapter
                .request_fingerprint(&cap_equivalent, authority)
                .expect("fingerprint")
        );
        assert_ne!(
            feed_adapter
                .request_fingerprint(&cap, authority)
                .expect("fingerprint"),
            feed_adapter
                .request_fingerprint(&cap_link, authority)
                .expect("fingerprint")
        );
    }

    #[test]
    fn owned_response_enforces_body_and_header_bounds() {
        let mut source = b"OK".to_vec();
        let response = LegacyHttpTransportResponseV1::new(
            200,
            Some("text/plain"),
            source.clone(),
            vec![("X-Test".into(), "value".into())],
            2,
        )
        .expect("owned response");
        source[0] = b'X';
        assert_eq!(response.body(), b"OK");
        assert!(
            LegacyHttpTransportResponseV1::new(200, None, b"too large".to_vec(), Vec::new(), 2)
                .is_err()
        );
        assert!(
            LegacyHttpTransportResponseV1::new(
                200,
                None,
                Vec::new(),
                vec![("X-Test".into(), "a".into()), ("x-test".into(), "b".into())],
                2,
            )
            .is_err()
        );
    }

    #[test]
    fn generic_d1_journal_cannot_promote_a_typed_business_adapter() {
        assert!(ENABLED_DURABLE_ADAPTERS.is_empty());
        assert_eq!(
            SEMANTIC_ADAPTERS
                .iter()
                .filter(|registration| {
                    registration.adapter.authority_requirement()
                        == LegacyAdapterAuthorityRequirementV1::D1BusinessAuthority
                })
                .map(|registration| registration.operation_id)
                .collect::<Vec<_>>(),
            [LEGACY_NOTIFICATION_PREFERENCES_OPERATION_ID]
        );
        let report = serde_json::from_str::<Report>(REPORT).expect("report");
        let pending = report
            .entries
            .iter()
            .find(|row| {
                !ENABLED_SEMANTIC_ADAPTERS.contains(&row.id.as_str())
                    && !ENABLED_EXACT_BUSINESS_ADAPTERS.contains(&row.id.as_str())
            })
            .expect("pending row");
        assert!(semantic_registration(&pending.id).is_none());
    }

    #[test]
    fn notification_preferences_fingerprint_binds_only_exact_actor_route_semantics() {
        let adapter = LegacySemanticAdapterV1::NotificationPreferencesGet;
        let request = |principal: &str, raw_query: &str| {
            let authenticated =
                LegacyAuthenticatedContextV1::principal_only(principal).expect("principal");
            let mut transport = static_transport_request(
                "GET",
                LEGACY_NOTIFICATION_PREFERENCES_PATH,
                released_caller(ClientSurfaceV1::Web),
                0,
                None,
                allowed_public_security(),
            );
            transport.query = LegacyCanonicalQueryV1::parse(raw_query).expect("query");
            transport.security = allowed_session_security();
            transport.authenticated = Some(authenticated.clone());
            let request = adapter_request(adapter, transport);
            let fingerprint =
                notification_preferences_request_fingerprint(&request, &authenticated);
            (request, fingerprint)
        };
        let (_, actor_a) = request("actor-a", "ignored=one");
        let (_, actor_a_equivalent) = request("actor-a", "ignored=two&ignored=three");
        let (_, actor_b) = request("actor-b", "ignored=one");
        assert_eq!(actor_a, actor_a_equivalent);
        assert_ne!(actor_a, actor_b);

        let (tenant_bound, _) = request("actor-a", "");
        let mut tenant_bound = tenant_bound;
        tenant_bound.authenticated =
            Some(LegacyAuthenticatedContextV1::new("actor-a", "tenant-a").expect("tenant context"));
        let static_authority = LegacyStaticAdapterAuthorityV1::default();
        assert_eq!(
            adapter.request_fingerprint(
                &tenant_bound,
                LegacyAdapterAuthorityV1::Static(&static_authority)
            ),
            Err(LegacyExecutionErrorV1::Invalid)
        );
    }

    #[test]
    fn web_active_organization_dispatches_only_as_an_exact_action_identity() {
        let transport = LegacyCompatibilityTransportV1::new_static_only(compatibility())
            .expect("compatibility transport");
        assert!(
            transport
                .registry
                .resolve_http("ACTION", LEGACY_WEB_ACTIVE_ORGANIZATION_IDENTITY)
                .is_none(),
            "an ACTION identity must never be synthesized as an HTTP path"
        );
        assert!(
            contract_registration(
                frame_application::LEGACY_MOBILE_ACTIVE_ORGANIZATION_OPERATION_ID
            )
            .is_none(),
            "blocked mobile bootstrap must not enter the exact runtime allowlist"
        );

        let repository = RecordingSelectionRepository::default();
        let actor_id =
            UserId::parse("018f6f65-7d5d-7d46-a3e1-4e7da76f36a8").expect("actor identifier");
        let effect = block_on(
            transport.dispatch_web_active_organization_action(
                &repository,
                LegacyWebActiveOrganizationActionInvocationV1 {
                    caller: released_caller(ClientSurfaceV1::Web),
                    envelope: ApiMutationEnvelopeV1 {
                        content_length: 0,
                        content_type: None,
                        idempotency_key: None,
                        correlation_id: "web-active-organization-action".into(),
                    },
                    security: allowed_session_security(),
                    authenticated: LegacyAuthenticatedContextV1::new(
                        actor_id.to_string(),
                        "2a6a8a87-d5ca-8c83-8666-2e92c2a69404",
                    )
                    .expect("trusted session"),
                    legacy_organization_id: "0123456789abcde".into(),
                    occurred_at: TimestampMillis::new(1_700_000_000_000).expect("timestamp"),
                },
            ),
        )
        .expect("exact action");
        assert_eq!(
            effect,
            LegacyWebActiveOrganizationActionEffectV1::InvalidateThenResolveVoid {
                path: "/dashboard"
            }
        );
        let commands = repository.commands.lock().expect("commands");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].actor_id, actor_id);
        assert_eq!(
            commands[0].active_organization_id.to_string(),
            "2a6a8a87-d5ca-8c83-8666-2e92c2a69404"
        );
        drop(commands);

        let unbound_actor = block_on(
            transport.dispatch_web_active_organization_action(
                &repository,
                LegacyWebActiveOrganizationActionInvocationV1 {
                    caller: released_caller(ClientSurfaceV1::Web),
                    envelope: ApiMutationEnvelopeV1 {
                        content_length: 0,
                        content_type: None,
                        idempotency_key: None,
                        correlation_id: "unbound-session-actor".into(),
                    },
                    security: allowed_session_security(),
                    authenticated: LegacyAuthenticatedContextV1::new("actor-a", "tenant-a")
                        .expect("syntactically bounded context"),
                    legacy_organization_id: "0123456789abcde".into(),
                    occurred_at: TimestampMillis::new(1_700_000_000_001).expect("timestamp"),
                },
            ),
        )
        .expect_err("the mutation actor must derive from a typed Frame session principal");
        assert_eq!(
            unbound_actor,
            LegacyOrganizationSelectionErrorV1::Unauthorized
        );
        assert_eq!(repository.commands.lock().expect("commands").len(), 1);

        let wrong_identity = transport
            .admit_exact_operation(LegacyExactOperationAdmissionV1 {
                operation_id: LEGACY_WEB_ACTIVE_ORGANIZATION_OPERATION_ID,
                kind: LegacyOperationKindV1::ServerAction,
                method: "ACTION",
                legacy_identity: "/api/mobile/user/active-organization",
                caller: released_caller(ClientSurfaceV1::Web),
                envelope: ApiMutationEnvelopeV1 {
                    content_length: 0,
                    content_type: None,
                    idempotency_key: None,
                    correlation_id: "wrong-action-identity".into(),
                },
                security: allowed_session_security(),
            })
            .expect_err("wrong action identity");
        assert_eq!(wrong_identity.code, ApiErrorCodeV1::NotFound);
    }

    #[test]
    fn static_transport_serves_all_seven_exact_source_pinned_success_contracts() {
        let transport = LegacyCompatibilityTransportV1::new_static_only(compatibility())
            .expect("static transport");
        let status = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_STATUS_PATH,
            released_caller(ClientSurfaceV1::Web),
            0,
            None,
            allowed_public_security(),
        )))
        .expect("exact status response");
        assert_eq!(status.status(), 200);
        assert_eq!(status.content_type(), Some(LEGACY_STATUS_CONTENT_TYPE));
        assert_eq!(status.body(), LEGACY_STATUS_BODY.as_bytes());

        let media = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_MEDIA_SERVER_ROOT_PATH,
            LegacyCallerV1::InternalWorker,
            0,
            None,
            allowed_public_security(),
        )))
        .expect("exact media-server root response");
        assert_eq!(media.status(), 200);
        assert_eq!(
            media.content_type(),
            Some(LEGACY_MEDIA_SERVER_ROOT_CONTENT_TYPE)
        );
        assert_eq!(media.body(), LEGACY_MEDIA_SERVER_ROOT_BODY.as_bytes());

        let changelog = block_on(transport.dispatch_http_response(with_query_version(
            static_transport_request(
                "GET",
                LEGACY_CHANGELOG_STATUS_PATH,
                released_caller(ClientSurfaceV1::Desktop),
                0,
                None,
                allowed_public_security(),
            ),
            Some(LEGACY_CHANGELOG_LATEST_VERSION),
        )))
        .expect("exact changelog update response");
        assert_eq!(changelog.status(), 200);
        assert_eq!(
            changelog.content_type(),
            Some(LEGACY_CHANGELOG_CONTENT_TYPE)
        );
        assert_eq!(changelog.body(), LEGACY_CHANGELOG_UPDATE_BODY.as_bytes());
        assert_eq!(
            changelog.headers(),
            static_headers(LEGACY_CHANGELOG_CORS_HEADERS)
        );

        let preflight = block_on(transport.dispatch_http_response(static_transport_request(
            "OPTIONS",
            LEGACY_CHANGELOG_STATUS_PATH,
            released_caller(ClientSurfaceV1::Web),
            0,
            None,
            allowed_public_security(),
        )))
        .expect("exact changelog preflight response");
        assert_eq!(preflight.status(), 204);
        assert_eq!(preflight.content_type(), None);
        assert_eq!(preflight.body(), b"");
        assert_eq!(
            preflight.headers(),
            static_headers(LEGACY_CHANGELOG_CORS_HEADERS)
        );

        let feed = block_on(transport.dispatch_http_response(with_cors(
            static_transport_request(
                "GET",
                LEGACY_CHANGELOG_FEED_PATH,
                released_caller(ClientSurfaceV1::Desktop),
                0,
                None,
                allowed_public_security(),
            ),
            Some("https://cap.so"),
            "https://frame.engmanager.xyz",
            "https://frame.engmanager.xyz",
        )))
        .expect("exact changelog feed response");
        assert_eq!(feed.status(), 200);
        assert_eq!(
            feed.content_type(),
            Some(LEGACY_CHANGELOG_FEED_CONTENT_TYPE)
        );
        assert_eq!(feed.body(), LEGACY_CHANGELOG_FEED_BODY.as_bytes());
        assert_eq!(
            feed.headers(),
            [
                (
                    "Access-Control-Allow-Origin".into(),
                    "https://cap.so".into()
                ),
                ("Access-Control-Allow-Credentials".into(), "true".into()),
            ]
        );

        let feed_preflight = block_on(transport.dispatch_http_response(with_cors(
            static_transport_request(
                "OPTIONS",
                LEGACY_CHANGELOG_FEED_PATH,
                released_caller(ClientSurfaceV1::Web),
                0,
                None,
                allowed_public_security(),
            ),
            Some("https://untrusted.invalid"),
            "https://frame.engmanager.xyz",
            "https://frame.engmanager.xyz",
        )))
        .expect("exact changelog feed preflight response");
        assert_eq!(feed_preflight.status(), 204);
        assert_eq!(feed_preflight.content_type(), None);
        assert_eq!(feed_preflight.body(), b"");
        assert_eq!(
            feed_preflight.headers(),
            [
                (
                    "Access-Control-Allow-Origin".into(),
                    "https://frame.engmanager.xyz".into(),
                ),
                ("Access-Control-Allow-Credentials".into(), "true".into()),
                ("Access-Control-Allow-Methods".into(), "GET, OPTIONS".into()),
                ("Access-Control-Allow-Headers".into(), "Content-Type".into()),
            ]
        );

        let mobile = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_MOBILE_SESSION_CONFIG_PATH,
            released_caller(ClientSurfaceV1::Mobile),
            0,
            None,
            allowed_public_security(),
        )))
        .expect("exact mobile session config response");
        assert_eq!(mobile.status(), 200);
        assert_eq!(
            mobile.content_type(),
            Some(LEGACY_MOBILE_SESSION_CONFIG_CONTENT_TYPE)
        );
        assert_eq!(
            mobile.body(),
            LEGACY_MOBILE_SESSION_CONFIG_NONE_BODY.as_bytes()
        );
    }

    #[test]
    fn mobile_session_config_binds_all_four_env_states_to_exact_json_and_fingerprints() {
        let adapter = LegacySemanticAdapterV1::MobileSessionConfigGet;
        let mut fingerprints = Vec::new();
        for (google_auth_available, workos_auth_available, expected_body) in [
            (false, false, LEGACY_MOBILE_SESSION_CONFIG_NONE_BODY),
            (true, false, LEGACY_MOBILE_SESSION_CONFIG_GOOGLE_BODY),
            (false, true, LEGACY_MOBILE_SESSION_CONFIG_WORKOS_BODY),
            (true, true, LEGACY_MOBILE_SESSION_CONFIG_BOTH_BODY),
        ] {
            let static_authority = LegacyStaticAdapterAuthorityV1 {
                google_auth_available,
                workos_auth_available,
            };
            let authority = LegacyAdapterAuthorityV1::Static(&static_authority);
            let typed_request = adapter_request(
                adapter,
                static_transport_request(
                    "GET",
                    LEGACY_MOBILE_SESSION_CONFIG_PATH,
                    released_caller(ClientSurfaceV1::Mobile),
                    0,
                    None,
                    allowed_public_security(),
                ),
            );
            let fingerprint = adapter
                .request_fingerprint(&typed_request, authority)
                .expect("authority-bound mobile config fingerprint");
            assert_eq!(
                adapter
                    .request_fingerprint(&typed_request, authority)
                    .expect("stable mobile config fingerprint"),
                fingerprint
            );
            fingerprints.push(fingerprint);

            let transport = LegacyCompatibilityTransportV1::new_static_with_authority(
                compatibility(),
                static_authority,
            )
            .expect("mobile config transport");
            let response = block_on(transport.dispatch_http_response(static_transport_request(
                "GET",
                LEGACY_MOBILE_SESSION_CONFIG_PATH,
                released_caller(ClientSurfaceV1::Mobile),
                0,
                None,
                allowed_public_security(),
            )))
            .expect("exact mobile config response");
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.content_type(),
                Some(LEGACY_MOBILE_SESSION_CONFIG_CONTENT_TYPE)
            );
            assert_eq!(response.body(), expected_body.as_bytes());
            assert!(serde_json::from_slice::<serde_json::Value>(response.body()).is_ok());
        }
        for (index, left) in fingerprints.iter().enumerate() {
            assert!(
                fingerprints
                    .iter()
                    .skip(index + 1)
                    .all(|right| left != right),
                "each server-side configuration must have a distinct fingerprint"
            );
        }
    }

    #[test]
    fn mobile_session_config_ignores_unmodeled_query_and_origin_without_caller_control() {
        let static_authority = LegacyStaticAdapterAuthorityV1 {
            google_auth_available: true,
            workos_auth_available: false,
        };
        let transport = LegacyCompatibilityTransportV1::new_static_with_authority(
            compatibility(),
            static_authority,
        )
        .expect("mobile config transport");
        let baseline = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_MOBILE_SESSION_CONFIG_PATH,
            released_caller(ClientSurfaceV1::Mobile),
            0,
            None,
            allowed_public_security(),
        )))
        .expect("baseline mobile config");
        let mut hostile = with_cors(
            static_transport_request(
                "GET",
                LEGACY_MOBILE_SESSION_CONFIG_PATH,
                released_caller(ClientSurfaceV1::Mobile),
                0,
                None,
                allowed_public_security(),
            ),
            Some("https://attacker.invalid"),
            "https://attacker.invalid",
            "https://attacker.invalid",
        );
        hostile.query = LegacyCanonicalQueryV1::parse(
            "googleAuthAvailable=false&workosAuthAvailable=true&duplicate=one&duplicate=two",
        )
        .expect("bounded hostile query");
        let hostile = block_on(transport.dispatch_http_response(hostile))
            .expect("unmodeled inputs are ignored like the pinned Effect endpoint");
        assert_eq!(hostile, baseline);
        assert_eq!(
            hostile.body(),
            LEGACY_MOBILE_SESSION_CONFIG_GOOGLE_BODY.as_bytes()
        );
    }

    #[test]
    fn exact_static_validation_and_idempotency_retry_axes_are_local() {
        let transport = LegacyCompatibilityTransportV1::new_static_only(compatibility())
            .expect("static transport");
        for (method, path, caller) in [
            (
                "GET",
                LEGACY_STATUS_PATH,
                released_caller(ClientSurfaceV1::Web),
            ),
            (
                "GET",
                LEGACY_MEDIA_SERVER_ROOT_PATH,
                LegacyCallerV1::InternalWorker,
            ),
            (
                "GET",
                LEGACY_CHANGELOG_STATUS_PATH,
                released_caller(ClientSurfaceV1::Desktop),
            ),
            (
                "OPTIONS",
                LEGACY_CHANGELOG_STATUS_PATH,
                released_caller(ClientSurfaceV1::Web),
            ),
            (
                "GET",
                LEGACY_CHANGELOG_FEED_PATH,
                released_caller(ClientSurfaceV1::Desktop),
            ),
            (
                "OPTIONS",
                LEGACY_CHANGELOG_FEED_PATH,
                released_caller(ClientSurfaceV1::Web),
            ),
            (
                "GET",
                LEGACY_MOBILE_SESSION_CONFIG_PATH,
                released_caller(ClientSurfaceV1::Mobile),
            ),
        ] {
            let oversized = block_on(transport.dispatch_http_response(static_transport_request(
                method,
                path,
                caller.clone(),
                1,
                None,
                allowed_public_security(),
            )))
            .expect_err("static request body must remain empty");
            assert_eq!(oversized.code, ApiErrorCodeV1::InvalidRequest);

            let keyed = block_on(transport.dispatch_http_response(static_transport_request(
                method,
                path,
                caller.clone(),
                0,
                Some(IdempotencyKey::parse("static-must-not-have-a-key").expect("key")),
                allowed_public_security(),
            )))
            .expect_err("static idempotency key must remain forbidden");
            assert_eq!(keyed.code, ApiErrorCodeV1::InvalidRequest);

            let first = block_on(transport.dispatch_http_response(static_transport_request(
                method,
                path,
                caller.clone(),
                0,
                None,
                allowed_public_security(),
            )))
            .expect("first static response");
            let retry = block_on(transport.dispatch_http_response(static_transport_request(
                method,
                path,
                caller,
                0,
                None,
                allowed_public_security(),
            )))
            .expect("retry static response");
            assert_eq!(retry, first);
        }
    }

    #[test]
    fn changelog_status_preserves_url_search_params_update_semantics() {
        let transport = LegacyCompatibilityTransportV1::new_static_only(compatibility())
            .expect("static transport");
        for (version, expected_body, expected_digest) in [
            (
                None,
                LEGACY_CHANGELOG_CURRENT_BODY,
                "50ed2e4ebb8b5c4d7a48d238523ff03be36fb40273c7dc0671cca063d6007ad0",
            ),
            (
                Some(""),
                LEGACY_CHANGELOG_CURRENT_BODY,
                "50ed2e4ebb8b5c4d7a48d238523ff03be36fb40273c7dc0671cca063d6007ad0",
            ),
            (
                Some("0.5.5"),
                LEGACY_CHANGELOG_CURRENT_BODY,
                "50ed2e4ebb8b5c4d7a48d238523ff03be36fb40273c7dc0671cca063d6007ad0",
            ),
            (
                Some(LEGACY_CHANGELOG_LATEST_VERSION),
                LEGACY_CHANGELOG_UPDATE_BODY,
                "74bb21a06e526f3870d9b7ff94e075af5243e2f6e4e8a176ff75168131206058",
            ),
        ] {
            let response = block_on(transport.dispatch_http_response(with_query_version(
                static_transport_request(
                    "GET",
                    LEGACY_CHANGELOG_STATUS_PATH,
                    released_caller(ClientSurfaceV1::Desktop),
                    0,
                    None,
                    allowed_public_security(),
                ),
                version,
            )))
            .expect("changelog status response");
            assert_eq!(response.body(), expected_body.as_bytes());
            assert_eq!(
                response.headers(),
                static_headers(LEGACY_CHANGELOG_CORS_HEADERS)
            );
            assert_eq!(
                ChecksumSha256::digest_bytes(response.body()),
                ChecksumSha256::parse(expected_digest).expect("pinned body digest")
            );
        }
    }

    #[test]
    fn changelog_feed_preserves_pinned_body_query_and_cors_semantics() {
        assert_eq!(LEGACY_CHANGELOG_FEED_BODY.len(), 88_817);
        assert_eq!(
            ChecksumSha256::digest_bytes(LEGACY_CHANGELOG_FEED_BODY.as_bytes()),
            ChecksumSha256::parse(LEGACY_CHANGELOG_FEED_BODY_SHA256)
                .expect("pinned changelog feed body digest")
        );
        assert_eq!(
            LEGACY_CHANGELOG_FEED_SOURCE_MANIFEST_SHA256,
            "dace60a24a816766681282e4569eda38e16fd85c96a9b2ab311a59351ef58b2d"
        );

        let transport = LegacyCompatibilityTransportV1::new_static_only(compatibility())
            .expect("static transport");
        for (request_origin, original_origin, configured_origin, expected) in [
            (
                Some("https://cap.link"),
                "https://frame.engmanager.xyz",
                "https://frame.engmanager.xyz",
                "https://cap.link",
            ),
            (
                Some("https://frame.engmanager.xyz"),
                "https://frame.engmanager.xyz",
                "https://frame.engmanager.xyz",
                "https://frame.engmanager.xyz",
            ),
            (
                Some("https://untrusted.invalid"),
                "https://untrusted.invalid",
                "https://frame.engmanager.xyz",
                "null",
            ),
            (
                None,
                "http://localhost:3000",
                "https://frame.engmanager.xyz",
                "http://localhost:3000",
            ),
        ] {
            let response = block_on(transport.dispatch_http_response(with_cors(
                with_query_version(
                    static_transport_request(
                        "GET",
                        LEGACY_CHANGELOG_FEED_PATH,
                        released_caller(ClientSurfaceV1::Desktop),
                        0,
                        None,
                        allowed_public_security(),
                    ),
                    Some("ignored-exactly-like-the-pinned-route"),
                ),
                request_origin,
                original_origin,
                configured_origin,
            )))
            .expect("changelog feed response");
            assert_eq!(response.body(), LEGACY_CHANGELOG_FEED_BODY.as_bytes());
            assert_eq!(
                response.headers().first(),
                Some(&("Access-Control-Allow-Origin".into(), expected.into()))
            );
        }
    }

    #[test]
    fn exact_static_authorization_and_failure_axes_fail_closed() {
        let transport = LegacyCompatibilityTransportV1::new_static_only(compatibility())
            .expect("static transport");
        let wrong_family = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_MEDIA_SERVER_ROOT_PATH,
            released_caller(ClientSurfaceV1::Web),
            0,
            None,
            allowed_public_security(),
        )))
        .expect_err("cross-family request must be hidden");
        assert_eq!(wrong_family.code, ApiErrorCodeV1::NotFound);

        let wrong_changelog_family =
            block_on(transport.dispatch_http_response(static_transport_request(
                "GET",
                LEGACY_CHANGELOG_STATUS_PATH,
                released_caller(ClientSurfaceV1::Web),
                0,
                None,
                allowed_public_security(),
            )))
            .expect_err("changelog GET must remain desktop-only");
        assert_eq!(wrong_changelog_family.code, ApiErrorCodeV1::NotFound);
        let wrong_feed_family =
            block_on(transport.dispatch_http_response(static_transport_request(
                "GET",
                LEGACY_CHANGELOG_FEED_PATH,
                released_caller(ClientSurfaceV1::Web),
                0,
                None,
                allowed_public_security(),
            )))
            .expect_err("changelog feed GET must remain desktop-only");
        assert_eq!(wrong_feed_family.code, ApiErrorCodeV1::NotFound);
        let wrong_mobile_family =
            block_on(transport.dispatch_http_response(static_transport_request(
                "GET",
                LEGACY_MOBILE_SESSION_CONFIG_PATH,
                released_caller(ClientSurfaceV1::Web),
                0,
                None,
                allowed_public_security(),
            )))
            .expect_err("mobile config GET must remain mobile-only");
        assert_eq!(wrong_mobile_family.code, ApiErrorCodeV1::NotFound);
        let obsolete_mobile = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_MOBILE_SESSION_CONFIG_PATH,
            LegacyCallerV1::Released(ClientReleaseV1 {
                surface: ClientSurfaceV1::Mobile,
                api_major: 1,
                release: 40,
            }),
            0,
            None,
            allowed_public_security(),
        )))
        .expect_err("obsolete mobile release must fail closed");
        assert_eq!(obsolete_mobile.code, ApiErrorCodeV1::UpgradeRequired);

        let mut denied = allowed_public_security();
        denied.authorized = false;
        let denied = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_STATUS_PATH,
            released_caller(ClientSurfaceV1::Web),
            0,
            None,
            denied,
        )))
        .expect_err("authorization denial must not disclose the operation");
        assert_eq!(denied.code, ApiErrorCodeV1::NotFound);

        let mut denied_mobile_security = allowed_public_security();
        denied_mobile_security.authorized = false;
        let denied_mobile = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_MOBILE_SESSION_CONFIG_PATH,
            released_caller(ClientSurfaceV1::Mobile),
            0,
            None,
            denied_mobile_security,
        )))
        .expect_err("mobile authorization denial must not disclose the operation");
        assert_eq!(denied_mobile.code, ApiErrorCodeV1::NotFound);

        let mut rate_limited = allowed_public_security();
        rate_limited.rate_limit = RateLimitDecisionV1::Rejected {
            retry_after_ms: 1_000,
        };
        let rate_limited = block_on(transport.dispatch_http_response(static_transport_request(
            "GET",
            LEGACY_STATUS_PATH,
            released_caller(ClientSurfaceV1::Web),
            0,
            None,
            rate_limited,
        )))
        .expect_err("rate-limit failure must be stable");
        assert_eq!(rate_limited.code, ApiErrorCodeV1::RateLimited);
        assert_eq!(rate_limited.retry_after_ms, Some(1_000));

        let mut rate_limited_mobile_security = allowed_public_security();
        rate_limited_mobile_security.rate_limit = RateLimitDecisionV1::Rejected {
            retry_after_ms: 2_000,
        };
        let rate_limited_mobile =
            block_on(transport.dispatch_http_response(static_transport_request(
                "GET",
                LEGACY_MOBILE_SESSION_CONFIG_PATH,
                released_caller(ClientSurfaceV1::Mobile),
                0,
                None,
                rate_limited_mobile_security,
            )))
            .expect_err("mobile rate-limit failure must be stable");
        assert_eq!(rate_limited_mobile.code, ApiErrorCodeV1::RateLimited);
        assert_eq!(rate_limited_mobile.retry_after_ms, Some(2_000));

        for (method, raw_path, caller) in [
            (
                "POST",
                LEGACY_STATUS_PATH,
                released_caller(ClientSurfaceV1::Web),
            ),
            ("GET", "/api/status/", released_caller(ClientSurfaceV1::Web)),
            (
                "POST",
                LEGACY_MEDIA_SERVER_ROOT_PATH,
                LegacyCallerV1::InternalWorker,
            ),
            ("GET", "/media-server/", LegacyCallerV1::InternalWorker),
            (
                "POST",
                LEGACY_CHANGELOG_STATUS_PATH,
                released_caller(ClientSurfaceV1::Desktop),
            ),
            (
                "GET",
                "/api/changelog/status/",
                released_caller(ClientSurfaceV1::Desktop),
            ),
            (
                "POST",
                LEGACY_CHANGELOG_FEED_PATH,
                released_caller(ClientSurfaceV1::Desktop),
            ),
            (
                "GET",
                "/api/changelog/",
                released_caller(ClientSurfaceV1::Desktop),
            ),
            (
                "POST",
                LEGACY_MOBILE_SESSION_CONFIG_PATH,
                released_caller(ClientSurfaceV1::Mobile),
            ),
            (
                "GET",
                "/api/mobile/session/config/",
                released_caller(ClientSurfaceV1::Mobile),
            ),
        ] {
            let unknown = block_on(transport.dispatch_http_response(static_transport_request(
                method,
                raw_path,
                caller,
                0,
                None,
                allowed_public_security(),
            )))
            .expect_err("non-exact identity must fail closed");
            assert_eq!(unknown.code, ApiErrorCodeV1::NotFound);
        }
    }

    #[test]
    fn d1_queries_bind_every_transition_to_the_winning_reservation() {
        assert!(CLAIM_SQL.starts_with("INSERT OR IGNORE"));
        assert!(CLAIM_SQL.contains("RETURNING reservation_digest"));
        for query in [INTENT_SQL, COMPLETE_SQL, AUDIT_SQL] {
            assert!(query.contains("reservation_digest = ?4"));
            assert!(query.contains("request_fingerprint = ?5"));
        }
        assert!(COMPLETE_SQL.contains("state = 'pending'"));
        assert!(AUDIT_SQL.contains("state = 'complete'"));
        assert!(LOAD_SQL.contains("LEFT JOIN legacy_api_execution_intents_v1"));
        assert!(LOAD_SQL.contains("LEFT JOIN legacy_api_execution_audit_v1"));
    }

    #[test]
    fn operation_outcome_rejects_conflict_partial_commit_and_corruption() {
        let complete = ExecutionRow {
            request_fingerprint: "02".repeat(32),
            reservation_digest: "03".repeat(32),
            state: "complete".into(),
            response_status: Some(202),
            result_digest: Some("04".repeat(32)),
            intent_reservation_digest: Some("03".repeat(32)),
            audit_reservation_digest: Some("03".repeat(32)),
        };
        assert!(matches!(
            execution_outcome(complete, &"02".repeat(32), &"03".repeat(32)),
            Ok(LegacyExecutionOutcomeV1::Completed(_))
        ));

        let partial = ExecutionRow {
            request_fingerprint: "02".repeat(32),
            reservation_digest: "03".repeat(32),
            state: "complete".into(),
            response_status: Some(202),
            result_digest: Some("04".repeat(32)),
            intent_reservation_digest: None,
            audit_reservation_digest: Some("03".repeat(32)),
        };
        assert_eq!(
            execution_outcome(partial, &"02".repeat(32), &"03".repeat(32)),
            Err(LegacyExecutionErrorV1::Internal)
        );

        let conflict = ExecutionRow {
            request_fingerprint: "ff".repeat(32),
            reservation_digest: "03".repeat(32),
            state: "pending".into(),
            response_status: None,
            result_digest: None,
            intent_reservation_digest: None,
            audit_reservation_digest: None,
        };
        assert_eq!(
            execution_outcome(conflict, &"02".repeat(32), &"03".repeat(32)),
            Err(LegacyExecutionErrorV1::Conflict)
        );
    }
}
