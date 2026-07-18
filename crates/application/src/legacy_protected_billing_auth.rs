//! Source-pinned local contracts for Cap authentication, billing, and
//! administrator operations whose terminal result is externally protected.
//!
//! The contracts intentionally stop before claiming an OAuth, email, Stripe,
//! object-storage, CDN, or media-processing result. They canonicalize and
//! redact requests, bind them to an authenticated principal and replay key,
//! and describe the independent human/provider evidence required to release a
//! response.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const LEGACY_PROTECTED_BILLING_AUTH_CAP_COMMIT: &str =
    "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_PROTECTED_BILLING_AUTH_OPERATION_COUNT: usize = 17;
pub const LEGACY_PROTECTED_BILLING_AUTH_MAX_BODY_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedBillingAuthKindV1 {
    Route,
    ServerAction,
    Workflow,
}

impl LegacyProtectedBillingAuthKindV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::ServerAction => "server_action",
            Self::Workflow => "workflow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedBillingAuthAuthV1 {
    Anonymous,
    PublicOrFlowToken,
    Session,
    SessionOrApiKey,
    AdminSession,
    SignedWebhook,
}

impl LegacyProtectedBillingAuthAuthV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::PublicOrFlowToken => "public_or_flow_token",
            Self::Session => "session",
            Self::SessionOrApiKey => "session_or_api_key",
            Self::AdminSession => "admin_session",
            Self::SignedWebhook => "signed_webhook",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedBillingAuthAuthorityV1 {
    PublicFlow,
    ActiveSession,
    DeveloperAppOwner,
    MessengerAdminVideo,
    SignedStripeWebhook,
}

impl LegacyProtectedBillingAuthAuthorityV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PublicFlow => "public_flow",
            Self::ActiveSession => "active_session",
            Self::DeveloperAppOwner => "developer_app_owner",
            Self::MessengerAdminVideo => "messenger_admin_video",
            Self::SignedStripeWebhook => "signed_stripe_webhook",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedBillingAuthIdempotencyV1 {
    Required,
    Optional,
    Forbidden,
}

impl LegacyProtectedBillingAuthIdempotencyV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::Forbidden => "forbidden",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyProtectedBillingAuthSourceV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyProtectedBillingAuthProfileV1 {
    pub operation_id: &'static str,
    pub kind: LegacyProtectedBillingAuthKindV1,
    pub method: &'static str,
    pub path: &'static str,
    pub auth: LegacyProtectedBillingAuthAuthV1,
    pub authority: LegacyProtectedBillingAuthAuthorityV1,
    pub idempotency: LegacyProtectedBillingAuthIdempotencyV1,
    pub max_body_bytes: usize,
    pub accepted_content_types: &'static [&'static str],
    pub rate_limit_bucket: &'static str,
    pub provider: &'static str,
    pub human_approval_required: bool,
    pub required_paths: &'static [&'static str],
    pub target_pointer: Option<&'static str>,
    pub sources: &'static [LegacyProtectedBillingAuthSourceV1],
}

const NEXTAUTH_GET_SOURCES: &[LegacyProtectedBillingAuthSourceV1] = &[
    LegacyProtectedBillingAuthSourceV1 {
        path: "apps/web/app/api/auth/[...nextauth]/route.ts",
        symbol: "GET",
        sha256: "50811be29eebab82de18c45f77f41f68c123931252baddb9a9d8c4f13620e5ff",
    },
    LegacyProtectedBillingAuthSourceV1 {
        path: "packages/database/auth/auth-options.ts",
        symbol: "NextAuth OAuth providers",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyProtectedBillingAuthSourceV1 {
        path: "packages/database/emails/config.ts",
        symbol: "NextAuth email via Resend",
        sha256: "d7f399dcefaeb0dd9c0a048f1a7212b24af11249b85e7c31cb569b6cb0108ead",
    },
];
const NEXTAUTH_POST_SOURCES: &[LegacyProtectedBillingAuthSourceV1] = &[
    LegacyProtectedBillingAuthSourceV1 {
        path: "apps/web/app/api/auth/[...nextauth]/route.ts",
        symbol: "POST",
        sha256: "50811be29eebab82de18c45f77f41f68c123931252baddb9a9d8c4f13620e5ff",
    },
    NEXTAUTH_GET_SOURCES[1],
    NEXTAUTH_GET_SOURCES[2],
];
const DESKTOP_SUBSCRIBE_SOURCES: &[LegacyProtectedBillingAuthSourceV1] = &[
    LegacyProtectedBillingAuthSourceV1 {
        path: "apps/web/app/api/desktop/[...route]/root.ts",
        symbol: "POST /subscribe",
        sha256: "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a",
    },
    LegacyProtectedBillingAuthSourceV1 {
        path: "packages/web-api-contract-effect/src/index.ts",
        symbol: "getProSubscribeURL",
        sha256: "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
    },
    LegacyProtectedBillingAuthSourceV1 {
        path: "packages/web-api-contract/src/desktop.ts",
        symbol: "POST /desktop/subscribe",
        sha256: "e55824d1b9ba74501841905c0bc4e70179247f6cd00e6249849970898af7adb9",
    },
];

const DEVELOPER_CHECKOUT_PREFLIGHT_SOURCES: &[LegacyProtectedBillingAuthSourceV1] = &[
    LegacyProtectedBillingAuthSourceV1 {
        path: "apps/web/app/api/developer/credits/checkout/route.ts",
        symbol: "OPTIONS",
        sha256: "30e9958e3634583d0d6a581ce9a79a9edb73b2434516ddbe40dbff32c9b80fe2",
    },
    LegacyProtectedBillingAuthSourceV1 {
        path: "apps/web/app/api/utils.ts",
        symbol: "corsMiddleware",
        sha256: "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
    },
];

macro_rules! source {
    ($path:literal, $symbol:literal, $sha:literal) => {
        &[LegacyProtectedBillingAuthSourceV1 {
            path: $path,
            symbol: $symbol,
            sha256: $sha,
        }]
    };
}

macro_rules! rate_limit_bucket {
    (SignedWebhook, $human:expr) => {
        "stripe_webhook_ingress.v1"
    };
    ($auth:ident, $human:expr) => {
        if $human {
            "billing_admin.v1"
        } else {
            "auth_session.v1"
        }
    };
}

macro_rules! profile {
    ($id:literal,$kind:ident,$method:literal,$path:literal,$auth:ident,$authority:ident,
     $idem:ident,$max:expr,[$($content:literal),* $(,)?],$provider:literal,$human:expr,
     [$($required:literal),* $(,)?],$target:expr,$sources:expr) => {
        LegacyProtectedBillingAuthProfileV1 {
            operation_id: $id,
            kind: LegacyProtectedBillingAuthKindV1::$kind,
            method: $method,
            path: $path,
            auth: LegacyProtectedBillingAuthAuthV1::$auth,
            authority: LegacyProtectedBillingAuthAuthorityV1::$authority,
            idempotency: LegacyProtectedBillingAuthIdempotencyV1::$idem,
            max_body_bytes: $max,
            accepted_content_types: &[$($content),*],
            rate_limit_bucket: rate_limit_bucket!($auth, $human),
            provider: $provider,
            human_approval_required: $human,
            required_paths: &[$($required),*],
            target_pointer: $target,
            sources: $sources,
        }
    };
}

pub const LEGACY_PROTECTED_BILLING_AUTH_PROFILES: &[LegacyProtectedBillingAuthProfileV1] = &[
    profile!(
        "cap-v1-46bda1c18ffba076",
        Route,
        "GET",
        "/api/auth/:nextauth*",
        PublicOrFlowToken,
        PublicFlow,
        Forbidden,
        0,
        [],
        "nextauth_google_workos_resend",
        false,
        ["/nextauthPath"],
        None,
        NEXTAUTH_GET_SOURCES
    ),
    profile!(
        "cap-v1-82a39c991fae1050",
        Route,
        "POST",
        "/api/auth/:nextauth*",
        PublicOrFlowToken,
        PublicFlow,
        Optional,
        262_144,
        ["application/json", "application/x-www-form-urlencoded"],
        "nextauth_google_workos_resend",
        false,
        ["/nextauthPath"],
        None,
        NEXTAUTH_POST_SOURCES
    ),
    profile!(
        "cap-v1-78537fb518df75ec",
        Route,
        "POST",
        "/api/desktop/subscribe",
        SessionOrApiKey,
        ActiveSession,
        Optional,
        262_144,
        ["application/json"],
        "stripe_checkout",
        true,
        ["/priceId"],
        None,
        DESKTOP_SUBSCRIBE_SOURCES
    ),
    profile!(
        "cap-v1-572763e7b4977abd",
        Route,
        "OPTIONS",
        "/api/developer/credits/checkout",
        Anonymous,
        PublicFlow,
        Forbidden,
        0,
        [],
        "local_credentialed_cors_preflight",
        false,
        [],
        None,
        DEVELOPER_CHECKOUT_PREFLIGHT_SOURCES
    ),
    profile!(
        "cap-v1-60b06cc5ab45f187",
        Route,
        "POST",
        "/api/developer/credits/checkout",
        SessionOrApiKey,
        DeveloperAppOwner,
        Optional,
        262_144,
        ["application/json"],
        "stripe_developer_checkout",
        true,
        ["/appId", "/amountCents"],
        Some("/appId"),
        &[
            LegacyProtectedBillingAuthSourceV1 {
                path: "apps/web/app/api/developer/credits/checkout/route.ts",
                symbol: "POST",
                sha256: "30e9958e3634583d0d6a581ce9a79a9edb73b2434516ddbe40dbff32c9b80fe2"
            },
            LegacyProtectedBillingAuthSourceV1 {
                path: "apps/web/app/api/developer/credits/checkout/route.ts",
                symbol: "POST /",
                sha256: "30e9958e3634583d0d6a581ce9a79a9edb73b2434516ddbe40dbff32c9b80fe2"
            },
        ]
    ),
    profile!(
        "cap-v1-af61fa5c8fc453cf",
        Route,
        "POST",
        "/api/settings/billing/guest-checkout",
        Anonymous,
        PublicFlow,
        Optional,
        262_144,
        ["application/json"],
        "stripe_guest_checkout",
        true,
        ["/priceId"],
        None,
        source!(
            "apps/web/app/api/settings/billing/guest-checkout/route.ts",
            "POST",
            "d2c11cb5d791ba2aadbfe873211d1a303fa0992200eaa3a221b5cfe144fd1edd"
        )
    ),
    profile!(
        "cap-v1-e596f65c43ee2a82",
        Route,
        "POST",
        "/api/settings/billing/manage",
        Session,
        ActiveSession,
        Optional,
        262_144,
        ["application/json"],
        "stripe_billing_portal",
        true,
        [],
        None,
        source!(
            "apps/web/app/api/settings/billing/manage/route.ts",
            "POST",
            "5a5928a025ee875ceb9a896b15102497124187959db95b805e78406205aef313"
        )
    ),
    profile!(
        "cap-v1-96230bf1f2da3d00",
        Route,
        "POST",
        "/api/settings/billing/subscribe",
        Session,
        ActiveSession,
        Optional,
        262_144,
        ["application/json"],
        "stripe_checkout",
        true,
        ["/priceId"],
        None,
        source!(
            "apps/web/app/api/settings/billing/subscribe/route.ts",
            "POST",
            "517871dbf4c2e05371824a911a2c8585f32f0730b63d2ad4e328f6adee639398"
        )
    ),
    profile!(
        "cap-v1-856dfea22b9d979c",
        Route,
        "GET",
        "/api/settings/billing/usage",
        Session,
        ActiveSession,
        Forbidden,
        0,
        [],
        "stripe_entitlement_and_d1_usage",
        true,
        [],
        None,
        source!(
            "apps/web/app/api/settings/billing/usage/route.ts",
            "GET",
            "58b57296880e2ab21b2768a87402d1883f1bdad8791c7d3597062d69711212ac"
        )
    ),
    profile!(
        "cap-v1-1e5f228815a2a8b7",
        Route,
        "POST",
        "/api/webhooks/stripe",
        SignedWebhook,
        SignedStripeWebhook,
        Optional,
        1_048_576,
        ["application/json"],
        "stripe_webhook_reconciliation",
        true,
        ["/id", "/type", "/data/object"],
        Some("/id"),
        source!(
            "apps/web/app/api/webhooks/stripe/route.ts",
            "POST",
            "b50f0859c0b0679ff8c5a11a0660dceadfe9ec0f4a67eab95972431d83f4acd0"
        )
    ),
    profile!(
        "cap-v1-b2d19e91b05834cf",
        Route,
        "POST",
        "/api/commercial/checkout",
        Anonymous,
        PublicFlow,
        Optional,
        262_144,
        ["application/json"],
        "stripe_commercial_checkout",
        true,
        ["/type"],
        None,
        &[
            LegacyProtectedBillingAuthSourceV1 {
                path: "packages/web-api-contract-effect/src/index.ts",
                symbol: "createCommercialCheckoutUrl",
                sha256: "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0"
            },
            LegacyProtectedBillingAuthSourceV1 {
                path: "packages/web-api-contract/src/index.ts",
                symbol: "POST /commercial/checkout",
                sha256: "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e"
            },
            LegacyProtectedBillingAuthSourceV1 {
                path: "apps/web/app/api/[[...route]]/route.ts",
                symbol: "HttpLive /api mount",
                sha256: "710d4e58383e3562e4627e4e312e188dc2042a1469b72f683a9a79ee6083eb2a"
            },
            LegacyProtectedBillingAuthSourceV1 {
                path: "apps/web/components/pages/HomePage/Pricing/CommercialCard.tsx",
                symbol: "POST /api/commercial/checkout caller",
                sha256: "9f424a4dfd1df93aae495c8592dfca26f24a51f9217d3b4e507ac87f80e069fe"
            },
            LegacyProtectedBillingAuthSourceV1 {
                path: "apps/web/next.config.mjs",
                symbol: "/api/commercial/:path* rewrite",
                sha256: "c3251d5a5925ee835dbc7cd1eb77eb42335813008a163e27e7823c15b9577b1e"
            },
        ]
    ),
    profile!(
        "cap-v1-90a6eb69c3fd7b4b",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/admin/replace-video.ts#getVideoReplaceUploadUrl",
        AdminSession,
        MessengerAdminVideo,
        Required,
        262_144,
        ["application/json"],
        "object_storage_presigned_upload",
        true,
        ["/videoId"],
        Some("/videoId"),
        source!(
            "apps/web/actions/admin/replace-video.ts",
            "getVideoReplaceUploadUrl",
            "aa351bcd00264527e5e65cf5bd0ba50b4fcbffdd0d2f6827de6d11e9e7f35f5e"
        )
    ),
    profile!(
        "cap-v1-e488991f97723847",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/admin/replace-video.ts#invalidateVideoCache",
        AdminSession,
        MessengerAdminVideo,
        Required,
        262_144,
        ["application/json"],
        "cloudfront_invalidation",
        true,
        ["/videoId"],
        Some("/videoId"),
        source!(
            "apps/web/actions/admin/replace-video.ts",
            "invalidateVideoCache",
            "aa351bcd00264527e5e65cf5bd0ba50b4fcbffdd0d2f6827de6d11e9e7f35f5e"
        )
    ),
    profile!(
        "cap-v1-14ea978608dcf07e",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/admin/reprocess-video.ts#adminReprocessVideo",
        AdminSession,
        MessengerAdminVideo,
        Required,
        262_144,
        ["application/json"],
        "media_reprocess_workflow_dispatch",
        true,
        ["/videoId"],
        Some("/videoId"),
        source!(
            "apps/web/actions/admin/reprocess-video.ts",
            "adminReprocessVideo",
            "dfa8e90419ef945063db92f4831f3964ba810bfd90cffbb68cd91ed16da22cc8"
        )
    ),
    profile!(
        "cap-v1-dfd7a4c3d234ccd7",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/billing/track-meta-purchase.ts#getPurchaseForMeta",
        Session,
        ActiveSession,
        Required,
        262_144,
        ["application/json"],
        "stripe_purchase_attribution",
        true,
        [],
        None,
        source!(
            "apps/web/actions/billing/track-meta-purchase.ts",
            "getPurchaseForMeta",
            "cfa841bcadd9ef2aff9f73082b0b22f187536a405dfe88e8946548e8523bcb9d"
        )
    ),
    profile!(
        "cap-v1-0553f2fcdacfe2a9",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/manage-billing.ts#manageBilling",
        Session,
        ActiveSession,
        Required,
        262_144,
        ["application/json"],
        "stripe_billing_portal",
        true,
        [],
        None,
        source!(
            "apps/web/actions/organization/manage-billing.ts",
            "manageBilling",
            "c97be6211b98f941639ea88d67d6ddc1590be22310739c55f72ec3f64ba71d28"
        )
    ),
    profile!(
        "cap-v1-5a990f470c701cec",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/workflows/admin-reprocess-video.ts#adminReprocessVideoWorkflow",
        AdminSession,
        MessengerAdminVideo,
        Required,
        262_144,
        ["application/json"],
        "storage_media_server_and_cloudfront",
        true,
        ["/videoId"],
        Some("/videoId"),
        source!(
            "apps/web/workflows/admin-reprocess-video.ts",
            "adminReprocessVideoWorkflow",
            "f56c4d126db996b0fb8a7837326d08d1409c242fa7aa4ee1e5efae40d0c36743"
        )
    ),
];

/// Discriminates the authenticated evidence bound into a principal digest.
/// A digest alone is ambiguous: a session token and an API key may both be
/// SHA-256 values, but they have different revocation and identity domains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedBillingAuthCredentialKindV1 {
    None,
    PublicFlow,
    SessionToken,
    ApiKey,
    SignedEndpoint,
}

impl LegacyProtectedBillingAuthCredentialKindV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PublicFlow => "public_flow",
            Self::SessionToken => "session_token",
            Self::ApiKey => "api_key",
            Self::SignedEndpoint => "signed_endpoint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedBillingAuthPrincipalV1 {
    pub class: LegacyProtectedBillingAuthAuthV1,
    pub actor_id: Option<String>,
    pub credential_kind: LegacyProtectedBillingAuthCredentialKindV1,
    /// Exact durable identity within the credential kind: a session id, API
    /// key id, or stable signed endpoint identifier. Public flows have no
    /// durable subject id.
    pub credential_subject_id: Option<String>,
    /// Hash-key version for a session token. Other credential kinds leave this
    /// unset because their durable tables do not version the digest key.
    pub credential_key_version: Option<i64>,
    /// Digest of the exact authenticated credential or client-bound flow,
    /// never the secret or rotating delivery signature.
    pub credential_digest: Option<String>,
}

/// Identifies who selected the replay namespace. Generated route continuations
/// are deliberately distinct from caller keys and provider-natural identifiers
/// so D1 can apply bounded request-digest continuation only to legacy clients
/// that did not send an idempotency key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedBillingAuthReplayOriginV1 {
    Caller,
    Natural,
    Generated,
    Nonce,
}

impl LegacyProtectedBillingAuthReplayOriginV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Caller => "caller",
            Self::Natural => "natural",
            Self::Generated => "generated",
            Self::Nonce => "nonce",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyProtectedBillingAuthEnvelopeV1 {
    pub source_operation_id: String,
    pub principal: LegacyProtectedBillingAuthPrincipalV1,
    pub caller_idempotency_key: Option<String>,
    pub replay_origin: LegacyProtectedBillingAuthReplayOriginV1,
    pub request_nonce: String,
    pub payload: Value,
    /// Opaque location of the exact secret-bearing NextAuth transport. The
    /// reference itself is intentionally excluded from request identity so a
    /// vault may randomize ciphertext on an exact retry.
    #[serde(default)]
    pub sealed_request_ref: Option<String>,
    /// Digest of the plaintext typed request envelope. This deterministic
    /// binding, rather than the randomized opaque reference, enters identity.
    #[serde(default)]
    pub sealed_request_digest: Option<String>,
    /// Digest of the exact transport body. It is mandatory for signed
    /// webhooks so evidence remains bound to the bytes that were verified.
    pub transport_body_digest: Option<String>,
    /// Digest of the verified transport credential for this delivery. This is
    /// audit-only: rotating webhook signatures must never enter the canonical
    /// request or principal replay namespace.
    #[serde(default)]
    pub transport_credential_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyProtectedBillingAuthValidatedV1 {
    pub request_json: String,
    pub request_digest: String,
    pub principal_digest: String,
    pub replay_key_digest: String,
    pub replay_origin: LegacyProtectedBillingAuthReplayOriginV1,
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyProtectedBillingAuthValidationErrorV1 {
    #[error("unknown protected billing/auth operation")]
    UnknownOperation,
    #[error("protected billing/auth authentication class does not match")]
    AuthenticationMismatch,
    #[error("invalid protected billing/auth principal")]
    InvalidPrincipal,
    #[error("invalid protected billing/auth payload")]
    InvalidPayload,
    #[error("protected billing/auth payload is too large")]
    PayloadTooLarge,
    #[error("invalid protected billing/auth idempotency key")]
    InvalidIdempotency,
}

#[must_use]
pub fn legacy_protected_billing_auth_profile(
    operation_id: &str,
) -> Option<&'static LegacyProtectedBillingAuthProfileV1> {
    LEGACY_PROTECTED_BILLING_AUTH_PROFILES
        .iter()
        .find(|profile| profile.operation_id == operation_id)
}

pub fn validate_legacy_protected_billing_auth_envelope(
    envelope: &LegacyProtectedBillingAuthEnvelopeV1,
) -> Result<LegacyProtectedBillingAuthValidatedV1, LegacyProtectedBillingAuthValidationErrorV1> {
    let profile = legacy_protected_billing_auth_profile(&envelope.source_operation_id)
        .ok_or(LegacyProtectedBillingAuthValidationErrorV1::UnknownOperation)?;
    validate_principal(profile, &envelope.principal)?;
    validate_idempotency(profile, envelope)?;
    match (
        profile.auth,
        envelope.transport_credential_digest.as_deref(),
    ) {
        (LegacyProtectedBillingAuthAuthV1::SignedWebhook, Some(value)) if valid_digest(value) => {}
        (LegacyProtectedBillingAuthAuthV1::SignedWebhook, _) => {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPrincipal);
        }
        (_, None) => {}
        (_, Some(_)) => {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPrincipal);
        }
    }
    let is_nextauth = matches!(
        profile.operation_id,
        "cap-v1-46bda1c18ffba076" | "cap-v1-82a39c991fae1050"
    );
    match (
        is_nextauth,
        envelope.sealed_request_ref.as_deref(),
        envelope.sealed_request_digest.as_deref(),
    ) {
        (true, Some(reference), Some(request_digest))
            if valid_sealed_request_ref(reference) && valid_digest(request_digest) => {}
        (false, None, None) => {}
        _ => return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload),
    }

    let encoded = serde_json::to_vec(&envelope.payload)
        .map_err(|_| LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
    if encoded.len() > profile.max_body_bytes.max(4_096)
        || encoded.len() > LEGACY_PROTECTED_BILLING_AUTH_MAX_BODY_BYTES
    {
        return Err(LegacyProtectedBillingAuthValidationErrorV1::PayloadTooLarge);
    }
    if !envelope.payload.is_object() {
        return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
    }
    for pointer in profile.required_paths {
        let value = envelope
            .payload
            .pointer(pointer)
            .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
        if value.is_null()
            || value.as_str().is_some_and(|value| value.trim().is_empty())
            || value.as_array().is_some_and(Vec::is_empty)
        {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
        }
    }
    validate_operation_specific(profile, envelope)?;

    let target_id = extract_string(&envelope.payload, profile.target_pointer)?;
    let canonical_payload = if profile.operation_id == "cap-v1-1e5f228815a2a8b7" {
        canonical_stripe_event(&envelope.payload)?
    } else {
        redact_value(&envelope.payload, None)
    };
    let request_identity = json!({
        "schema_version": "frame.legacy-protected-billing-auth-request.v1",
        "source_operation_id": profile.operation_id,
        "payload": canonical_payload,
        "transport_body_digest": envelope.transport_body_digest,
        "sealed_request_digest": envelope.sealed_request_digest,
        "required_evidence": {
            "human_approval": profile.human_approval_required,
            "provider_execution": true,
        },
    });
    let request_digest = digest(
        serde_json::to_string(&request_identity)
            .map_err(|_| LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?
            .as_bytes(),
    );
    let mut persisted_request = request_identity;
    if is_nextauth {
        let object = persisted_request
            .as_object_mut()
            .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
        object.insert(
            "sealed_request_ref".into(),
            Value::String(
                envelope
                    .sealed_request_ref
                    .clone()
                    .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?,
            ),
        );
    }
    let request_json = serde_json::to_string(&persisted_request)
        .map_err(|_| LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
    let principal_json = serde_json::to_vec(&envelope.principal)
        .map_err(|_| LegacyProtectedBillingAuthValidationErrorV1::InvalidPrincipal)?;
    let principal_digest = digest(&principal_json);
    let replay_material = envelope
        .caller_idempotency_key
        .as_deref()
        .unwrap_or(&envelope.request_nonce);

    Ok(LegacyProtectedBillingAuthValidatedV1 {
        request_json,
        request_digest,
        principal_digest,
        replay_key_digest: digest(replay_material.as_bytes()),
        replay_origin: envelope.replay_origin,
        target_id,
    })
}

fn valid_sealed_request_ref(value: &str) -> bool {
    value
        .strip_prefix("frame-pba-request-v1:")
        .is_some_and(valid_digest)
}

fn validate_principal(
    profile: &LegacyProtectedBillingAuthProfileV1,
    principal: &LegacyProtectedBillingAuthPrincipalV1,
) -> Result<(), LegacyProtectedBillingAuthValidationErrorV1> {
    if profile.auth != principal.class {
        return Err(LegacyProtectedBillingAuthValidationErrorV1::AuthenticationMismatch);
    }
    let actor_valid = principal
        .actor_id
        .as_deref()
        .is_some_and(|actor| !actor.trim().is_empty() && actor.len() <= 255);
    let subject = principal.credential_subject_id.as_deref();
    let digest = principal.credential_digest.as_deref();
    let no_credential = principal.credential_kind
        == LegacyProtectedBillingAuthCredentialKindV1::None
        && subject.is_none()
        && principal.credential_key_version.is_none()
        && digest.is_none();
    let public_flow = principal.credential_kind
        == LegacyProtectedBillingAuthCredentialKindV1::PublicFlow
        && subject.is_none()
        && principal.credential_key_version.is_none()
        && digest.is_some_and(valid_digest);
    let session_token = principal.credential_kind
        == LegacyProtectedBillingAuthCredentialKindV1::SessionToken
        && subject.is_some_and(valid_session_subject)
        && principal
            .credential_key_version
            .is_some_and(|version| (1..=65_535).contains(&version))
        && digest.is_some_and(valid_digest);
    let api_key = principal.credential_kind == LegacyProtectedBillingAuthCredentialKindV1::ApiKey
        && subject.is_some_and(valid_credential_subject)
        && principal.credential_key_version.is_none()
        && digest.is_some_and(valid_digest);
    let signed_endpoint = principal.credential_kind
        == LegacyProtectedBillingAuthCredentialKindV1::SignedEndpoint
        && subject == Some("stripe-webhook.endpoint.v1")
        && principal.credential_key_version.is_none()
        && digest.is_some_and(valid_digest);
    let valid = match profile.auth {
        LegacyProtectedBillingAuthAuthV1::Anonymous => principal.actor_id.is_none() && public_flow,
        LegacyProtectedBillingAuthAuthV1::PublicOrFlowToken => {
            principal.actor_id.is_none() && (public_flow || no_credential)
        }
        LegacyProtectedBillingAuthAuthV1::Session
        | LegacyProtectedBillingAuthAuthV1::AdminSession => actor_valid && session_token,
        LegacyProtectedBillingAuthAuthV1::SessionOrApiKey => {
            actor_valid && (session_token || api_key)
        }
        LegacyProtectedBillingAuthAuthV1::SignedWebhook => {
            principal.actor_id.is_none() && signed_endpoint
        }
    };
    if !valid {
        return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPrincipal);
    }
    Ok(())
}

fn valid_session_subject(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
        && value != "00000000-0000-0000-0000-000000000000"
}

fn valid_credential_subject(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_idempotency(
    profile: &LegacyProtectedBillingAuthProfileV1,
    envelope: &LegacyProtectedBillingAuthEnvelopeV1,
) -> Result<(), LegacyProtectedBillingAuthValidationErrorV1> {
    if envelope.request_nonce.trim().is_empty() || envelope.request_nonce.len() > 512 {
        return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidIdempotency);
    }
    let key = envelope.caller_idempotency_key.as_deref();
    match envelope.replay_origin {
        LegacyProtectedBillingAuthReplayOriginV1::Caller
            if key.is_none_or(|value| value.trim().is_empty() || value.len() > 512) =>
        {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidIdempotency);
        }
        LegacyProtectedBillingAuthReplayOriginV1::Natural
            if key.is_none_or(|value| value.trim().is_empty() || value.len() > 512) =>
        {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidIdempotency);
        }
        LegacyProtectedBillingAuthReplayOriginV1::Generated
            if key.is_some()
                || profile.kind != LegacyProtectedBillingAuthKindV1::Route
                || profile.idempotency == LegacyProtectedBillingAuthIdempotencyV1::Required =>
        {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidIdempotency);
        }
        LegacyProtectedBillingAuthReplayOriginV1::Nonce if key.is_some() => {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidIdempotency);
        }
        _ => {}
    }
    match profile.idempotency {
        LegacyProtectedBillingAuthIdempotencyV1::Required
            if key.is_none_or(|value| value.trim().is_empty() || value.len() > 512) =>
        {
            Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidIdempotency)
        }
        LegacyProtectedBillingAuthIdempotencyV1::Forbidden if key.is_some() => {
            Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidIdempotency)
        }
        _ => Ok(()),
    }
}

fn validate_operation_specific(
    profile: &LegacyProtectedBillingAuthProfileV1,
    envelope: &LegacyProtectedBillingAuthEnvelopeV1,
) -> Result<(), LegacyProtectedBillingAuthValidationErrorV1> {
    let payload = &envelope.payload;
    match profile.operation_id {
        "cap-v1-46bda1c18ffba076" | "cap-v1-82a39c991fae1050" => {
            let path = payload
                .pointer("/nextauthPath")
                .and_then(Value::as_str)
                .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
            let suffix = path
                .strip_prefix("/api/auth/")
                .filter(|suffix| !suffix.is_empty() && suffix.len() <= 256)
                .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
            let head = suffix.split('/').next().unwrap_or_default();
            if !matches!(
                head,
                "callback"
                    | "csrf"
                    | "error"
                    | "providers"
                    | "session"
                    | "signin"
                    | "signout"
                    | "verify-request"
            ) || suffix.contains("..")
                || !suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_'))
            {
                return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-60b06cc5ab45f187" => {
            if payload
                .pointer("/amountCents")
                .and_then(Value::as_i64)
                .is_none_or(|amount| !(500..=100_000).contains(&amount))
            {
                return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-78537fb518df75ec" | "cap-v1-af61fa5c8fc453cf" | "cap-v1-96230bf1f2da3d00" => {
            required_bounded_string(payload, "/priceId", 255)?;
            validate_optional_quantity(payload)?;
        }
        "cap-v1-b2d19e91b05834cf" => {
            if !matches!(
                payload.pointer("/type").and_then(Value::as_str),
                Some("yearly" | "lifetime")
            ) {
                return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
            }
            validate_optional_quantity(payload)?;
        }
        "cap-v1-1e5f228815a2a8b7" => {
            if envelope
                .transport_body_digest
                .as_deref()
                .is_none_or(|value| !valid_digest(value))
                || !matches!(
                    payload.pointer("/type").and_then(Value::as_str),
                    Some(
                        "checkout.session.completed"
                            | "checkout.session.async_payment_succeeded"
                            | "customer.subscription.updated"
                            | "customer.subscription.deleted"
                    )
                )
            {
                return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-dfd7a4c3d234ccd7"
            if payload.pointer("/sessionId").is_some_and(|value| {
                !value.is_null() && value.as_str().is_none_or(|value| value.len() > 255)
            }) =>
        {
            return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
        }
        "cap-v1-5a990f470c701cec" => {
            let parent_receipt_id = payload
                .pointer("/_frameParentReceiptId")
                .and_then(Value::as_str)
                .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
            let parent_request_digest = payload
                .pointer("/_frameParentRequestDigest")
                .and_then(Value::as_str)
                .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
            if parent_receipt_id.len() != 36 || !valid_digest(parent_request_digest) {
                return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
            }
        }
        _ => {}
    }
    Ok(())
}

fn required_bounded_string(
    payload: &Value,
    pointer: &str,
    maximum: usize,
) -> Result<(), LegacyProtectedBillingAuthValidationErrorV1> {
    if payload
        .pointer(pointer)
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty() || value.len() > maximum)
    {
        return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
    }
    Ok(())
}

fn validate_optional_quantity(
    payload: &Value,
) -> Result<(), LegacyProtectedBillingAuthValidationErrorV1> {
    if payload.pointer("/quantity").is_some_and(|value| {
        value
            .as_i64()
            .is_none_or(|quantity| !(1..=100).contains(&quantity))
    }) {
        return Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload);
    }
    Ok(())
}

fn canonical_stripe_event(
    payload: &Value,
) -> Result<Value, LegacyProtectedBillingAuthValidationErrorV1> {
    let event_id = payload
        .pointer("/id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
    let event_type = payload
        .pointer("/type")
        .and_then(Value::as_str)
        .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
    let object = payload
        .pointer("/data/object")
        .and_then(Value::as_object)
        .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
    let object_digest = digest(
        serde_json::to_string(object)
            .map_err(|_| LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?
            .as_bytes(),
    );
    Ok(json!({
        "id": event_id,
        "type": event_type,
        "created": payload.pointer("/created").and_then(Value::as_i64),
        "livemode": payload.pointer("/livemode").and_then(Value::as_bool),
        "data_object_id": object.get("id").and_then(Value::as_str),
        "data_object_type": object.get("object").and_then(Value::as_str),
        "data_object_sha256": object_digest,
    }))
}

fn extract_string(
    payload: &Value,
    pointer: Option<&str>,
) -> Result<Option<String>, LegacyProtectedBillingAuthValidationErrorV1> {
    let Some(pointer) = pointer else {
        return Ok(None);
    };
    let value = payload
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .ok_or(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)?;
    Ok(Some(value.to_owned()))
}

fn redact_value(value: &Value, key: Option<&str>) -> Value {
    // Persist only the source-pinned, non-secret request vocabulary in clear
    // text. Unknown fields are attacker-controlled: treating an unfamiliar
    // name as harmless would let variants such as `secretMaterial` or
    // `authorizationHeader` bypass a finite credential-name denylist. Keep a
    // digest binding for replay/conflict detection, but fail closed on the
    // stored value.
    if key.is_some_and(|key| {
        is_sensitive_key(key)
            || !is_safe_persisted_key(key)
            // Every cleartext field in the source-pinned vocabulary is a
            // scalar. Treating a recognized name as permission to recurse
            // into an attacker-supplied container would let array elements
            // (which have no key of their own) bypass the fail-closed check.
            || value.is_array()
            || value.is_object()
    }) && !value.is_null()
    {
        let encoded = serde_json::to_vec(value).unwrap_or_default();
        return json!({"redacted":true,"sha256":digest(&encoded)});
    }
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), redact_value(value, Some(key))))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact_value(value, None))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn is_safe_persisted_key(key: &str) -> bool {
    matches!(
        key,
        "nextauthPath"
            | "redirect"
            | "priceId"
            | "quantity"
            | "isOnBoarding"
            | "appId"
            | "amountCents"
            | "type"
            | "videoId"
            | "sessionId"
            | "_frameParentReceiptId"
            | "_frameParentRequestDigest"
    )
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "authorization"
            | "callbackurl"
            | "code"
            | "cookie"
            | "csrftoken"
            | "email"
            | "password"
            | "secret"
            | "signature"
            | "state"
            | "token"
    ) || [
        "accesstoken",
        "refreshtoken",
        "sessiontoken",
        "clientsecret",
        "webhooksecret",
        "signingsecret",
        "apikey",
        "secretkey",
        "privatekey",
        "accesskey",
        "credential",
        "credentials",
        "authorization",
        "password",
        "signature",
        "cookie",
        "token",
    ]
    .iter()
    .any(|suffix| normalized.ends_with(suffix))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(class: LegacyProtectedBillingAuthAuthV1) -> LegacyProtectedBillingAuthPrincipalV1 {
        LegacyProtectedBillingAuthPrincipalV1 {
            class,
            actor_id: Some("00000000-0000-7000-8000-000000000001".into()),
            credential_kind: LegacyProtectedBillingAuthCredentialKindV1::SessionToken,
            credential_subject_id: Some("00000000-0000-7000-8000-000000000002".into()),
            credential_key_version: Some(1),
            credential_digest: Some("a".repeat(64)),
        }
    }

    #[test]
    fn profile_inventory_is_exact_unique_and_source_pinned() {
        assert_eq!(
            LEGACY_PROTECTED_BILLING_AUTH_PROFILES.len(),
            LEGACY_PROTECTED_BILLING_AUTH_OPERATION_COUNT
        );
        let mut ids = LEGACY_PROTECTED_BILLING_AUTH_PROFILES
            .iter()
            .map(|profile| profile.operation_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), LEGACY_PROTECTED_BILLING_AUTH_OPERATION_COUNT);
        assert_eq!(
            LEGACY_PROTECTED_BILLING_AUTH_PROFILES
                .iter()
                .filter(|profile| profile.human_approval_required)
                .count(),
            14
        );
        let preflight = legacy_protected_billing_auth_profile("cap-v1-572763e7b4977abd")
            .expect("checked-in developer checkout preflight");
        assert_eq!(preflight.auth, LegacyProtectedBillingAuthAuthV1::Anonymous);
        assert_eq!(
            preflight.authority,
            LegacyProtectedBillingAuthAuthorityV1::PublicFlow
        );
        assert!(!preflight.human_approval_required);
        assert_eq!(preflight.provider, "local_credentialed_cors_preflight");
        assert_eq!(preflight.sources.len(), 2);
        assert!(
            LEGACY_PROTECTED_BILLING_AUTH_PROFILES
                .iter()
                .all(|profile| {
                    !profile.sources.is_empty()
                        && profile
                            .sources
                            .iter()
                            .all(|source| valid_digest(source.sha256))
                })
        );
    }

    #[test]
    fn released_cap_auth_and_replay_profiles_are_source_compatible() {
        let nextauth_post = legacy_protected_billing_auth_profile("cap-v1-82a39c991fae1050")
            .expect("checked-in NextAuth POST");
        assert_eq!(
            nextauth_post.accepted_content_types,
            ["application/json", "application/x-www-form-urlencoded"]
        );
        assert_eq!(
            nextauth_post.idempotency,
            LegacyProtectedBillingAuthIdempotencyV1::Optional
        );

        for operation_id in ["cap-v1-78537fb518df75ec", "cap-v1-60b06cc5ab45f187"] {
            let profile = legacy_protected_billing_auth_profile(operation_id)
                .expect("checked-in Cap API-key route");
            assert_eq!(
                profile.auth,
                LegacyProtectedBillingAuthAuthV1::SessionOrApiKey
            );
            assert_eq!(
                profile.idempotency,
                LegacyProtectedBillingAuthIdempotencyV1::Optional
            );
        }
        for operation_id in [
            "cap-v1-e596f65c43ee2a82",
            "cap-v1-96230bf1f2da3d00",
            "cap-v1-856dfea22b9d979c",
            "cap-v1-dfd7a4c3d234ccd7",
            "cap-v1-0553f2fcdacfe2a9",
        ] {
            assert_eq!(
                legacy_protected_billing_auth_profile(operation_id)
                    .expect("checked-in session billing operation")
                    .auth,
                LegacyProtectedBillingAuthAuthV1::Session
            );
        }
        for operation_id in ["cap-v1-af61fa5c8fc453cf", "cap-v1-b2d19e91b05834cf"] {
            let profile = legacy_protected_billing_auth_profile(operation_id)
                .expect("checked-in anonymous checkout");
            assert_eq!(profile.auth, LegacyProtectedBillingAuthAuthV1::Anonymous);
            assert_eq!(
                profile.authority,
                LegacyProtectedBillingAuthAuthorityV1::PublicFlow
            );
        }
        let stripe = legacy_protected_billing_auth_profile("cap-v1-1e5f228815a2a8b7")
            .expect("checked-in Stripe webhook");
        assert_eq!(stripe.rate_limit_bucket, "stripe_webhook_ingress.v1");
        assert_eq!(
            stripe.idempotency,
            LegacyProtectedBillingAuthIdempotencyV1::Optional
        );
    }

    #[test]
    fn credential_kind_binds_the_exact_revocation_domain() {
        let session_profile = legacy_protected_billing_auth_profile("cap-v1-e596f65c43ee2a82")
            .expect("checked-in session billing route");
        let mut session = principal(LegacyProtectedBillingAuthAuthV1::Session);
        assert!(validate_principal(session_profile, &session).is_ok());
        session.credential_kind = LegacyProtectedBillingAuthCredentialKindV1::ApiKey;
        session.credential_key_version = None;
        assert_eq!(
            validate_principal(session_profile, &session),
            Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPrincipal)
        );

        let dual_profile = legacy_protected_billing_auth_profile("cap-v1-78537fb518df75ec")
            .expect("checked-in session-or-api-key route");
        let mut api_key = principal(LegacyProtectedBillingAuthAuthV1::SessionOrApiKey);
        api_key.credential_kind = LegacyProtectedBillingAuthCredentialKindV1::ApiKey;
        api_key.credential_subject_id = Some("api-key-1".into());
        api_key.credential_key_version = None;
        assert!(validate_principal(dual_profile, &api_key).is_ok());
        api_key.credential_subject_id = None;
        assert_eq!(
            validate_principal(dual_profile, &api_key),
            Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPrincipal)
        );
    }

    #[test]
    fn nextauth_secrets_are_redacted() {
        let envelope = LegacyProtectedBillingAuthEnvelopeV1 {
            source_operation_id: "cap-v1-82a39c991fae1050".into(),
            principal: LegacyProtectedBillingAuthPrincipalV1 {
                class: LegacyProtectedBillingAuthAuthV1::PublicOrFlowToken,
                actor_id: None,
                credential_kind: LegacyProtectedBillingAuthCredentialKindV1::None,
                credential_subject_id: None,
                credential_key_version: None,
                credential_digest: None,
            },
            caller_idempotency_key: Some("auth-post-1".into()),
            replay_origin: LegacyProtectedBillingAuthReplayOriginV1::Caller,
            request_nonce: "nonce-1".into(),
            payload: json!({
                "nextauthPath":"/api/auth/signin/email",
                "email":"person@example.test",
                "csrfToken":"do-not-persist"
            }),
            sealed_request_ref: Some(format!("frame-pba-request-v1:{}", "a".repeat(64))),
            sealed_request_digest: Some("b".repeat(64)),
            transport_body_digest: None,
            transport_credential_digest: None,
        };
        let Ok(validated) = validate_legacy_protected_billing_auth_envelope(&envelope) else {
            panic!("valid NextAuth envelope must pass validation");
        };
        assert!(!validated.request_json.contains("person@example.test"));
        assert!(!validated.request_json.contains("do-not-persist"));
        assert!(validated.request_json.contains("frame-pba-request-v1:"));

        let mut randomized_retry = envelope.clone();
        randomized_retry.sealed_request_ref =
            Some(format!("frame-pba-request-v1:{}", "c".repeat(64)));
        let retry = validate_legacy_protected_billing_auth_envelope(&randomized_retry)
            .expect("an exact plaintext retry may use randomized sealed storage");
        assert_eq!(validated.request_digest, retry.request_digest);
        assert_ne!(validated.request_json, retry.request_json);

        randomized_retry.sealed_request_digest = Some("d".repeat(64));
        let changed = validate_legacy_protected_billing_auth_envelope(&randomized_retry)
            .expect("changed sealed plaintext remains a valid distinct request");
        assert_ne!(validated.request_digest, changed.request_digest);
    }

    #[test]
    fn hostile_nested_secret_key_variants_are_structurally_redacted() {
        let payload = json!({
            "safeLabel":"retain-me",
            "priceId":"price_safe",
            "accessToken":"access-value",
            "nested": {
                "refresh_token":"refresh-value",
                "client-secret":"client-value",
                "apiKey":"api-value",
                "deep": [{"privateKey":"private-value"}, {"credential":"credential-value"}]
            }
        });
        let redacted = serde_json::to_string(&redact_value(&payload, None))
            .expect("redacted request must encode");
        assert!(!redacted.contains("retain-me"));
        assert!(redacted.contains("price_safe"));
        for secret in [
            "access-value",
            "refresh-value",
            "client-value",
            "api-value",
            "private-value",
            "credential-value",
        ] {
            assert!(
                !redacted.contains(secret),
                "secret survived redaction: {secret}"
            );
        }
        // Unknown `safeLabel` and the unknown `nested` container are redacted
        // as complete values, while the recognized credential key is redacted
        // independently.
        assert_eq!(redacted.matches("\"redacted\":true").count(), 3);
    }

    #[test]
    fn unknown_secret_like_fields_fail_closed_even_without_a_known_suffix() {
        let payload = json!({
            "videoId":"video-safe",
            "secretMaterial":"material-value",
            "authorizationHeader":"Bearer header-value",
            "bearerValue":"bearer-value",
            "unknownContainer": {
                "innocentLooking":"nested-value",
                "array":[{"opaque":"array-value"}]
            }
        });
        let redacted = serde_json::to_string(&redact_value(&payload, None))
            .expect("redacted request must encode");
        assert!(redacted.contains("video-safe"));
        for secret in [
            "material-value",
            "header-value",
            "bearer-value",
            "nested-value",
            "array-value",
        ] {
            assert!(
                !redacted.contains(secret),
                "unknown secret survived fail-closed redaction: {secret}"
            );
        }
    }

    #[test]
    fn scalar_allowlist_names_do_not_open_container_redaction_bypasses() {
        let payload = json!({
            "priceId":["array-secret", {"videoId":"nested-secret"}],
            "redirect":{"type":"object-secret"},
            "json":{"priceId":"nextauth-body-secret"}
        });
        let redacted = serde_json::to_string(&redact_value(&payload, None))
            .expect("redacted request must encode");
        for secret in [
            "array-secret",
            "nested-secret",
            "object-secret",
            "nextauth-body-secret",
        ] {
            assert!(
                !redacted.contains(secret),
                "container-shaped scalar field survived redaction: {secret}"
            );
        }
        assert_eq!(redacted.matches("\"redacted\":true").count(), 3);
    }

    #[test]
    fn checkout_amount_and_replay_are_bound() {
        let mut envelope = LegacyProtectedBillingAuthEnvelopeV1 {
            source_operation_id: "cap-v1-60b06cc5ab45f187".into(),
            principal: principal(LegacyProtectedBillingAuthAuthV1::SessionOrApiKey),
            caller_idempotency_key: Some("checkout-1".into()),
            replay_origin: LegacyProtectedBillingAuthReplayOriginV1::Caller,
            request_nonce: "nonce-2".into(),
            payload: json!({"appId":"app-1","amountCents":499}),
            sealed_request_ref: None,
            sealed_request_digest: None,
            transport_body_digest: None,
            transport_credential_digest: None,
        };
        assert_eq!(
            validate_legacy_protected_billing_auth_envelope(&envelope),
            Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPayload)
        );
        envelope.payload["amountCents"] = json!(500);
        let Ok(validated) = validate_legacy_protected_billing_auth_envelope(&envelope) else {
            panic!("minimum checkout amount must pass validation");
        };
        assert_eq!(validated.replay_key_digest, digest(b"checkout-1"));
        assert_eq!(validated.target_id.as_deref(), Some("app-1"));
    }

    #[test]
    fn webhook_requires_verified_transport_binding_and_sanitizes_object() {
        let event = json!({
            "id":"evt_1",
            "type":"checkout.session.completed",
            "data":{"object":{"id":"cs_1","object":"checkout.session","customer_email":"secret@example.test"}}
        });
        let mut envelope = LegacyProtectedBillingAuthEnvelopeV1 {
            source_operation_id: "cap-v1-1e5f228815a2a8b7".into(),
            principal: LegacyProtectedBillingAuthPrincipalV1 {
                class: LegacyProtectedBillingAuthAuthV1::SignedWebhook,
                actor_id: None,
                credential_kind: LegacyProtectedBillingAuthCredentialKindV1::SignedEndpoint,
                credential_subject_id: Some("stripe-webhook.endpoint.v1".into()),
                credential_key_version: None,
                credential_digest: Some("a".repeat(64)),
            },
            caller_idempotency_key: Some("evt_1".into()),
            replay_origin: LegacyProtectedBillingAuthReplayOriginV1::Natural,
            request_nonce: "nonce-3".into(),
            payload: event,
            sealed_request_ref: None,
            sealed_request_digest: None,
            transport_body_digest: Some("b".repeat(64)),
            transport_credential_digest: Some("c".repeat(64)),
        };
        let Ok(validated) = validate_legacy_protected_billing_auth_envelope(&envelope) else {
            panic!("digest-bound webhook envelope must pass validation");
        };
        assert!(!validated.request_json.contains("secret@example.test"));
        assert!(validated.request_json.contains("data_object_sha256"));
        assert!(
            !validated
                .request_json
                .contains("transport_credential_digest")
        );
        envelope.transport_credential_digest = Some("d".repeat(64));
        let retry = validate_legacy_protected_billing_auth_envelope(&envelope)
            .expect("newly signed retry must preserve canonical identity");
        assert_eq!(retry.request_digest, validated.request_digest);
        assert_eq!(retry.principal_digest, validated.principal_digest);
        assert_eq!(retry.replay_key_digest, validated.replay_key_digest);
    }

    #[test]
    fn purchase_attribution_accepts_the_source_optional_null_session() {
        let envelope = LegacyProtectedBillingAuthEnvelopeV1 {
            source_operation_id: "cap-v1-dfd7a4c3d234ccd7".into(),
            principal: principal(LegacyProtectedBillingAuthAuthV1::Session),
            caller_idempotency_key: Some("purchase-attribution-1".into()),
            replay_origin: LegacyProtectedBillingAuthReplayOriginV1::Caller,
            request_nonce: "nonce-4".into(),
            payload: json!({"sessionId":null}),
            sealed_request_ref: None,
            sealed_request_digest: None,
            transport_body_digest: None,
            transport_credential_digest: None,
        };
        assert!(validate_legacy_protected_billing_auth_envelope(&envelope).is_ok());
    }

    #[test]
    fn admin_workflow_requires_the_initiating_actor() {
        let mut envelope = LegacyProtectedBillingAuthEnvelopeV1 {
            source_operation_id: "cap-v1-5a990f470c701cec".into(),
            principal: LegacyProtectedBillingAuthPrincipalV1 {
                class: LegacyProtectedBillingAuthAuthV1::AdminSession,
                actor_id: None,
                credential_kind: LegacyProtectedBillingAuthCredentialKindV1::SessionToken,
                credential_subject_id: Some("00000000-0000-7000-8000-000000000002".into()),
                credential_key_version: Some(1),
                credential_digest: Some("a".repeat(64)),
            },
            caller_idempotency_key: Some("admin-reprocess-1".into()),
            replay_origin: LegacyProtectedBillingAuthReplayOriginV1::Natural,
            request_nonce: "nonce-workflow-1".into(),
            payload: json!({
                "videoId":"video-1",
                "_frameParentReceiptId":"00000000-0000-4000-8000-000000000001",
                "_frameParentRequestDigest":"a".repeat(64),
            }),
            sealed_request_ref: None,
            sealed_request_digest: None,
            transport_body_digest: None,
            transport_credential_digest: None,
        };
        assert_eq!(
            validate_legacy_protected_billing_auth_envelope(&envelope),
            Err(LegacyProtectedBillingAuthValidationErrorV1::InvalidPrincipal)
        );
        envelope.principal.actor_id = Some("admin-1".into());
        assert!(validate_legacy_protected_billing_auth_envelope(&envelope).is_ok());
    }
}
