//! Source-pinned local contracts for Cap operations whose final result depends
//! on an external integration provider.
//!
//! This module is intentionally provider-agnostic. It authenticates the shape
//! of the request presented to the D1 adapter, removes secret material before
//! persistence, and derives stable replay digests. It never treats an outbox
//! insertion as provider success.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::{Host, Url};

pub const LEGACY_PROTECTED_INTEGRATIONS_CAP_COMMIT: &str =
    "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_PROTECTED_INTEGRATIONS_OPERATION_COUNT: usize = 45;
pub const LEGACY_PROTECTED_INTEGRATIONS_MAX_BODY_BYTES: usize = 8 * 1_024 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedIntegrationKindV1 {
    Route,
    Rpc,
    ServerAction,
    Workflow,
}

impl LegacyProtectedIntegrationKindV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::Rpc => "rpc",
            Self::ServerAction => "server_action",
            Self::Workflow => "workflow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedIntegrationAuthV1 {
    Session,
    SessionOrApiKey,
    AnonymousOrSessionOrApiKey,
    SignedState,
    Public,
    PublicOrSession,
    ParentReceipt,
    SignedWebhook,
}

impl LegacyProtectedIntegrationAuthV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::SessionOrApiKey => "session_or_api_key",
            Self::AnonymousOrSessionOrApiKey => "anonymous_or_session_or_api_key",
            Self::SignedState => "signed_state",
            Self::Public => "public",
            Self::PublicOrSession => "public_or_session",
            Self::ParentReceipt => "parent_receipt",
            Self::SignedWebhook => "signed_webhook",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedIntegrationAuthorityV1 {
    Session,
    SessionOrOrganizationMember,
    SessionOrOrganizationOwner,
    OrganizationMember,
    OrganizationManager,
    OrganizationOwner,
    SpaceManager,
    VideoViewer,
    Public,
    SignedState,
    SignedStateOrOrganizationOwner,
    ParentReceipt,
    SignedWebhook,
}

impl LegacyProtectedIntegrationAuthorityV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::SessionOrOrganizationMember => "session_or_organization_member",
            Self::SessionOrOrganizationOwner => "session_or_organization_owner",
            Self::OrganizationMember => "organization_member",
            Self::OrganizationManager => "organization_manager",
            Self::OrganizationOwner => "organization_owner",
            Self::SpaceManager => "space_manager",
            Self::VideoViewer => "video_viewer",
            Self::Public => "public",
            Self::SignedState => "signed_state",
            Self::SignedStateOrOrganizationOwner => "signed_state_or_organization_owner",
            Self::ParentReceipt => "parent_receipt",
            Self::SignedWebhook => "signed_webhook",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedIntegrationEntitlementV1 {
    None,
    CapInternal,
    Pro,
    SubscriptionRead,
    SubscriptionManage,
}

/// Exact durable credential admitted by the HTTP/action adapter. The broad
/// carrier class (for example `session_or_api_key`) is not sufficient for a
/// durable provider intent: replay and evidence must remain bound to the
/// concrete session, API key, signed OAuth state, or signed endpoint that was
/// verified at ingress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedIntegrationCredentialKindV1 {
    None,
    SessionToken,
    ApiKey,
    SignedState,
    SignedEndpoint,
}

impl LegacyProtectedIntegrationCredentialKindV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SessionToken => "session_token",
            Self::ApiKey => "api_key",
            Self::SignedState => "signed_state",
            Self::SignedEndpoint => "signed_endpoint",
        }
    }
}

/// A password/public/owner policy decision is intentionally separate from
/// authentication. `getVideoStatus` may require both a live session and one
/// of these exact proofs; collapsing the two would allow password proof to
/// replace authentication (or vice versa).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedIntegrationPolicyProofV1 {
    /// Native Frame video id after the Cap NanoID alias has been resolved.
    pub target_id: String,
    pub kind: String,
    /// Native video/space id whose current revision is authoritative.
    pub subject_id: String,
    pub revision: i64,
    /// SHA-256 of the matched stored password hash, or a deterministic digest
    /// of the non-secret owner/public policy snapshot. Raw hashes never enter
    /// an envelope or D1.
    pub audit_digest: String,
}

/// Payload-conditional source authority captured at admission and rechecked
/// by the D1 live-authority view. Examples include a long desktop recording's
/// Pro gate, a private-to-public space transition, and the current paid-seat
/// floor. The vector is server-derived; public decoders never accept it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedIntegrationConditionalBindingV1 {
    pub kind: String,
    pub subject_id: String,
    pub revision: i64,
    pub value: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedIntegrationEntitlementBindingV1 {
    pub kind: String,
    pub subject_id: String,
    pub revision: i64,
    pub expires_at_ms: Option<i64>,
}

impl LegacyProtectedIntegrationEntitlementV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::CapInternal => "cap_internal",
            Self::Pro => "pro",
            Self::SubscriptionRead => "subscription_read",
            Self::SubscriptionManage => "subscription_manage",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedIntegrationIdempotencyV1 {
    Required,
    Optional,
    Forbidden,
}

/// Identifies the server-owned replay namespace used by a released Cap
/// carrier. No protected-integration caller is required to invent an HTTP
/// idempotency header: ordinary routes/actions/RPCs use a generated receipt
/// plus a bounded request-digest claim, while signed webhooks and durable
/// workflows use identifiers already present in their authenticated payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedIntegrationReplayOriginV1 {
    Generated,
    Natural,
}

impl LegacyProtectedIntegrationReplayOriginV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Generated => "generated",
            Self::Natural => "natural",
        }
    }
}

impl LegacyProtectedIntegrationIdempotencyV1 {
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
pub struct LegacyProtectedIntegrationProfileV1 {
    pub operation_id: &'static str,
    pub kind: LegacyProtectedIntegrationKindV1,
    pub method: &'static str,
    pub path: &'static str,
    pub auth: LegacyProtectedIntegrationAuthV1,
    pub authority: LegacyProtectedIntegrationAuthorityV1,
    pub idempotency: LegacyProtectedIntegrationIdempotencyV1,
    pub max_body_bytes: usize,
    pub provider: &'static str,
    pub required_paths: &'static [&'static str],
    pub target_pointer: Option<&'static str>,
    pub tenant_pointer: Option<&'static str>,
    pub source_path: &'static str,
    pub source_symbol: &'static str,
    pub source_sha256: &'static str,
}

macro_rules! profile {
    ($id:literal,$kind:ident,$method:literal,$path:literal,$auth:ident,$authority:ident,
     $idem:ident,$max:expr,$provider:literal,[$($required:literal),* $(,)?],$target:expr,$tenant:expr,
     $source:literal,$symbol:literal,$sha:literal) => {
        LegacyProtectedIntegrationProfileV1 {
            operation_id: $id,
            kind: LegacyProtectedIntegrationKindV1::$kind,
            method: $method,
            path: $path,
            auth: LegacyProtectedIntegrationAuthV1::$auth,
            authority: LegacyProtectedIntegrationAuthorityV1::$authority,
            idempotency: LegacyProtectedIntegrationIdempotencyV1::$idem,
            max_body_bytes: $max,
            provider: $provider,
            required_paths: &[$($required),*],
            target_pointer: $target,
            tenant_pointer: $tenant,
            source_path: $source,
            source_symbol: $symbol,
            source_sha256: $sha,
        }
    };
}

pub const LEGACY_PROTECTED_INTEGRATION_PROFILES: &[LegacyProtectedIntegrationProfileV1] = &[
    profile!(
        "cap-v1-30b7af7323aa2c37",
        Route,
        "POST",
        "/api/desktop/feedback",
        SessionOrApiKey,
        Session,
        Required,
        262_144,
        "resend_email",
        ["/feedback"],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/root.ts",
        "POST /feedback",
        "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a"
    ),
    profile!(
        "cap-v1-dfbbc4c0b56179d1",
        Route,
        "POST",
        "/api/desktop/logs",
        AnonymousOrSessionOrApiKey,
        Public,
        Required,
        262_144,
        "discord_webhook",
        ["/log"],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/root.ts",
        "Discord diagnostics webhook provider effect",
        "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a"
    ),
    profile!(
        "cap-v1-10180c4650ffde88",
        Route,
        "GET",
        "/api/desktop/plan",
        SessionOrApiKey,
        Session,
        Forbidden,
        0,
        "stripe_subscription_query",
        [],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/root.ts",
        "GET /plan",
        "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a"
    ),
    profile!(
        "cap-v1-9d91d42d52472a83",
        Route,
        "POST",
        "/api/desktop/s3/config",
        SessionOrApiKey,
        Session,
        Required,
        8_388_608,
        "sealed_s3_configuration",
        [
            "/provider",
            "/accessKeyId",
            "/secretAccessKey",
            "/bucketName",
            "/region"
        ],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/s3Config.ts",
        "POST /",
        "6df15f697b051d90382a4743e8cd5db422ba25571466160c72d90193e7314a6d"
    ),
    profile!(
        "cap-v1-58ec99a456d61373",
        Route,
        "DELETE",
        "/api/desktop/s3/config/delete",
        SessionOrApiKey,
        Session,
        Required,
        8_388_608,
        "sealed_s3_configuration",
        [],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/s3Config.ts",
        "DELETE /delete",
        "6df15f697b051d90382a4743e8cd5db422ba25571466160c72d90193e7314a6d"
    ),
    profile!(
        "cap-v1-c6214b213eaa2360",
        Route,
        "GET",
        "/api/desktop/s3/config/get",
        SessionOrApiKey,
        SessionOrOrganizationMember,
        Forbidden,
        8_388_608,
        "sealed_s3_configuration",
        [],
        None,
        Some("/orgId"),
        "apps/web/app/api/desktop/[...route]/s3Config.ts",
        "GET /get",
        "6df15f697b051d90382a4743e8cd5db422ba25571466160c72d90193e7314a6d"
    ),
    profile!(
        "cap-v1-2d1396c2f68299f9",
        Route,
        "POST",
        "/api/desktop/s3/config/test",
        SessionOrApiKey,
        Session,
        Required,
        8_388_608,
        "s3_head_bucket",
        ["/accessKeyId", "/secretAccessKey", "/bucketName", "/region"],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/s3Config.ts",
        "POST /test",
        "6df15f697b051d90382a4743e8cd5db422ba25571466160c72d90193e7314a6d"
    ),
    profile!(
        "cap-v1-49531a09fd9433e7",
        Route,
        "GET",
        "/api/desktop/storage/google-drive/callback",
        SignedState,
        SignedStateOrOrganizationOwner,
        Forbidden,
        8_388_608,
        "google_drive_oauth_exchange",
        [],
        None,
        Some("/orgId"),
        "apps/web/app/api/desktop/[...route]/storage.ts",
        "GET /google-drive/callback",
        "5e6fb13fe1f1176349a455d8c4ee4f1fea56fb53c095599b0aa990113ebd0886"
    ),
    profile!(
        "cap-v1-679e4241ef5e7383",
        Route,
        "POST",
        "/api/desktop/storage/google-drive/connect",
        SessionOrApiKey,
        SessionOrOrganizationOwner,
        Required,
        8_388_608,
        "google_drive_oauth_authorize",
        [],
        None,
        Some("/orgId"),
        "apps/web/app/api/desktop/[...route]/storage.ts",
        "POST /google-drive/connect",
        "5e6fb13fe1f1176349a455d8c4ee4f1fea56fb53c095599b0aa990113ebd0886"
    ),
    profile!(
        "cap-v1-5ef3570390b8c80c",
        Route,
        "DELETE",
        "/api/desktop/storage/google-drive/disconnect",
        SessionOrApiKey,
        Session,
        Required,
        8_388_608,
        "google_drive_disconnect",
        [],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/storage.ts",
        "DELETE /google-drive/disconnect",
        "5e6fb13fe1f1176349a455d8c4ee4f1fea56fb53c095599b0aa990113ebd0886"
    ),
    profile!(
        "cap-v1-8d5930c717418665",
        Route,
        "POST",
        "/api/desktop/storage/google-drive/test",
        SessionOrApiKey,
        Session,
        Required,
        8_388_608,
        "google_drive_identity_query",
        [],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/storage.ts",
        "POST /google-drive/test",
        "5e6fb13fe1f1176349a455d8c4ee4f1fea56fb53c095599b0aa990113ebd0886"
    ),
    profile!(
        "cap-v1-0b36c9acda9bd6a2",
        Route,
        "GET",
        "/api/desktop/storage/integrations",
        SessionOrApiKey,
        SessionOrOrganizationMember,
        Forbidden,
        8_388_608,
        "google_drive_quota_query",
        [],
        None,
        Some("/orgId"),
        "apps/web/app/api/desktop/[...route]/storage.ts",
        "GET /integrations",
        "5e6fb13fe1f1176349a455d8c4ee4f1fea56fb53c095599b0aa990113ebd0886"
    ),
    profile!(
        "cap-v1-2e4ee222efc29606",
        Route,
        "GET",
        "/api/desktop/user/profile/image",
        SessionOrApiKey,
        Session,
        Forbidden,
        0,
        "profile_image_fetch",
        [],
        None,
        None,
        "apps/web/app/api/desktop/[...route]/root.ts",
        "GET /user/profile/image",
        "c6f9ca2108849b75a00762b79af45b0523dd246bc118a2805cb57948f6ea2e7a"
    ),
    profile!(
        "cap-v1-60f863b2cb19353f",
        Route,
        "GET",
        "/api/desktop/video/create",
        SessionOrApiKey,
        OrganizationMember,
        Forbidden,
        0,
        "storage_signing_email_and_short_link",
        [],
        Some("/videoId"),
        Some("/orgId"),
        "apps/web/app/api/desktop/[...route]/video.ts",
        "GET /create",
        "03e50223fb6968dafdbaa8a8c8cb537c46be27a0c88b9c92e004afa95f7c013d"
    ),
    profile!(
        "cap-v1-f0a00e93ab606a52",
        Route,
        "POST",
        "/api/loom/video",
        SessionOrApiKey,
        OrganizationMember,
        Required,
        262_144,
        "loom_import",
        [
            "/cap/orgId",
            "/loom/userId",
            "/loom/orgId",
            "/loom/video/id",
            "/loom/video/name",
            "/loom/video/downloadUrl"
        ],
        Some("/loom/video/id"),
        Some("/cap/orgId"),
        "packages/web-domain/src/Loom.ts",
        "importVideo",
        "eeb572a6baa46459d7479376f6c805ddb9066da87e0e27c6d9d7c06869d72ab7"
    ),
    profile!(
        "cap-v1-05776c542380771e",
        Route,
        "PATCH",
        "/api/mobile/user/active-organization",
        SessionOrApiKey,
        OrganizationMember,
        Optional,
        262_144,
        "image_signing_and_bootstrap",
        ["/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/app/api/mobile/[...route]/route.ts",
        "mobile handler:setActiveOrganization",
        "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79"
    ),
    profile!(
        "cap-v1-8a1e6c87b4426f93",
        Route,
        "GET",
        "/api/releases/tauri/:version/:target/:arch",
        Public,
        Public,
        Forbidden,
        0,
        "github_releases",
        ["/version", "/target", "/arch"],
        None,
        None,
        "apps/web/app/api/releases/tauri/[version]/[target]/[arch]/route.ts",
        "GET",
        "06b0e9a10d4d58b3f4d84514e7a2b256ec7138ac0b1b9b1addeed5c458891554"
    ),
    profile!(
        "cap-v1-221a713f60d7528f",
        Route,
        "GET",
        "/api/tools/loom-download",
        Public,
        Public,
        Forbidden,
        0,
        "loom_download",
        ["/id"],
        Some("/id"),
        None,
        "apps/web/app/api/tools/loom-download/route.ts",
        "GET",
        "72066e4fe3bde90ebefe0360c4ba45767c3f9a3ceca89abcdbdfd77ecba0e585"
    ),
    profile!(
        "cap-v1-5af545d5d20508bd",
        Route,
        "POST",
        "/api/webhooks/media-server/multipart/:action",
        SignedWebhook,
        SignedWebhook,
        Required,
        1_048_576,
        "media_server_webhook",
        ["/action"],
        Some("/videoId"),
        None,
        "apps/web/app/api/webhooks/media-server/multipart/[action]/route.ts",
        "POST",
        "1d279d3511c308ecee79fd1c60637a8ba3da0191db0009f73eac0a2aea8d3764"
    ),
    profile!(
        "cap-v1-17d69edf5d3b06bb",
        Route,
        "POST",
        "/api/webhooks/media-server/progress",
        SignedWebhook,
        SignedWebhook,
        Required,
        1_048_576,
        "media_server_webhook",
        ["/videoId"],
        Some("/videoId"),
        None,
        "apps/web/app/api/webhooks/media-server/progress/route.ts",
        "POST",
        "b8a3bb3b895bb4ee0c4df94412c5581b362ed2d2661443f10b314a5162140ec7"
    ),
    profile!(
        "cap-v1-5cd4cac9da73f975",
        Rpc,
        "RPC",
        "/api/erpc#OrganisationSoftDelete",
        Session,
        OrganizationOwner,
        Required,
        262_144,
        "object_storage_and_tinybird_delete",
        ["/id"],
        None,
        Some("/id"),
        "apps/web/app/api/erpc/route.ts",
        "Effect RPC HTTP transport",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783"
    ),
    profile!(
        "cap-v1-422afbf05f09bf4f",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/loom.ts#downloadLoomVideo",
        Public,
        Public,
        Required,
        262_144,
        "loom_download",
        ["/url"],
        None,
        None,
        "apps/web/actions/loom.ts",
        "downloadLoomVideo",
        "024611ae7f3c03fe50c0bf91a474acebdcfc4eb43679ecdae51e1bfd4a0820d1"
    ),
    profile!(
        "cap-v1-cb3fade2af06d6bd",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/loom.ts#importFromLoom",
        Session,
        OrganizationMember,
        Required,
        262_144,
        "loom_import",
        ["/loomUrl", "/orgId"],
        None,
        Some("/orgId"),
        "apps/web/actions/loom.ts",
        "importFromLoom",
        "024611ae7f3c03fe50c0bf91a474acebdcfc4eb43679ecdae51e1bfd4a0820d1"
    ),
    profile!(
        "cap-v1-d062d262b013a0cd",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/loom.ts#importFromLoomCsv",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "loom_csv_import",
        ["/rows", "/orgId"],
        None,
        Some("/orgId"),
        "apps/web/actions/loom.ts",
        "importFromLoomCsv",
        "024611ae7f3c03fe50c0bf91a474acebdcfc4eb43679ecdae51e1bfd4a0820d1"
    ),
    profile!(
        "cap-v1-6446dc02a25cef2f",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/check-domain.ts#checkOrganizationDomain",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "vercel_domain_query",
        ["/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/check-domain.ts",
        "checkOrganizationDomain",
        "cc554561663d82c088b3610b2cd6461594402063ac3b8acec8b2cc9baebfceb1"
    ),
    profile!(
        "cap-v1-0c233c1115838206",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/create-space.ts#createSpace",
        Session,
        OrganizationMember,
        Optional,
        2_097_152,
        "space_icon_storage",
        ["/name"],
        None,
        None,
        "apps/web/actions/organization/create-space.ts",
        "createSpace",
        "fb38d13cc669ee5f76b2432afc0ff24bf5706144e686039433cc7ffde83a499d"
    ),
    profile!(
        "cap-v1-3f2885312f79698e",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/get-subscription-details.ts#getSubscriptionDetails",
        Session,
        OrganizationOwner,
        Required,
        262_144,
        "stripe_subscription_query",
        ["/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/get-subscription-details.ts",
        "getSubscriptionDetails",
        "92d0fd2c931f9fa11b538ec2efcabb525689adc8fc175107fde3e8ea31bf4f43"
    ),
    profile!(
        "cap-v1-55b98b0f419abf86",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/remove-domain.ts#removeOrganizationDomain",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "vercel_domain_delete",
        ["/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/remove-domain.ts",
        "Vercel DELETE provider effect",
        "18dfdd282bec9153e067b5f683756bb6bee9a2f8c58ccc8cfe034a88c51010cc"
    ),
    profile!(
        "cap-v1-f0ba260c29c295f3",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/send-invites.ts#sendOrganizationInvites",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "resend_invite_email",
        ["/inviteInputs", "/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/send-invites.ts",
        "sendOrganizationInvites",
        "c873f9bfbe3ef4b5b070225712d770eef1dcf51de9e05f64ca6e8b94fe54a01b"
    ),
    profile!(
        "cap-v1-4054bc310aa16e98",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/storage.ts#getOrganizationGoogleDrivePickerToken",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "google_drive_access_token",
        ["/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/storage.ts",
        "getOrganizationGoogleDrivePickerToken",
        "25c64e9cacfe2048160d6a8fb37c95b75ec06f07de8f04b94fad939f40a86de5"
    ),
    profile!(
        "cap-v1-83721706f7b0e2e6",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/storage.ts#listOrganizationGoogleDriveFolders",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "google_drive_folder_list",
        ["/organizationId"],
        Some("/parentId"),
        Some("/organizationId"),
        "apps/web/actions/organization/storage.ts",
        "listOrganizationGoogleDriveFolders",
        "25c64e9cacfe2048160d6a8fb37c95b75ec06f07de8f04b94fad939f40a86de5"
    ),
    profile!(
        "cap-v1-9d5f521a096649ec",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/storage.ts#removeOrganizationS3Config",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "sealed_s3_configuration",
        ["/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/storage.ts",
        "removeOrganizationS3Config",
        "25c64e9cacfe2048160d6a8fb37c95b75ec06f07de8f04b94fad939f40a86de5"
    ),
    profile!(
        "cap-v1-3badb07a1a6fe5f9",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/storage.ts#saveOrganizationS3Config",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "sealed_s3_configuration",
        [
            "/organizationId",
            "/provider",
            "/accessKeyId",
            "/secretAccessKey",
            "/bucketName",
            "/region"
        ],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/storage.ts",
        "saveOrganizationS3Config",
        "25c64e9cacfe2048160d6a8fb37c95b75ec06f07de8f04b94fad939f40a86de5"
    ),
    profile!(
        "cap-v1-50d0ffd9f5f7bcb6",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/storage.ts#setOrganizationGoogleDriveLocation",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "google_drive_location",
        ["/organizationId", "/folderId"],
        Some("/folderId"),
        Some("/organizationId"),
        "apps/web/actions/organization/storage.ts",
        "setOrganizationGoogleDriveLocation",
        "25c64e9cacfe2048160d6a8fb37c95b75ec06f07de8f04b94fad939f40a86de5"
    ),
    profile!(
        "cap-v1-1186b0fb0aaa2f4d",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/storage.ts#testOrganizationS3Config",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "s3_head_bucket",
        ["/organizationId", "/bucketName", "/region"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/storage.ts",
        "testOrganizationS3Config",
        "25c64e9cacfe2048160d6a8fb37c95b75ec06f07de8f04b94fad939f40a86de5"
    ),
    profile!(
        "cap-v1-0823c0b806bd38a5",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/update-domain.ts#updateDomain",
        Session,
        OrganizationManager,
        Required,
        262_144,
        "vercel_domain_mutation",
        ["/domain", "/organizationId"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/domain-utils.ts",
        "addDomain+checkDomainStatus",
        "7354ea80694e452fbece2c22a7cc60355fb72d6dcd12e0385936157f5dcb0995"
    ),
    profile!(
        "cap-v1-aa00fc906599e89c",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/update-seat-quantity.ts#previewSeatChange",
        Session,
        OrganizationOwner,
        Required,
        262_144,
        "stripe_invoice_preview",
        ["/organizationId", "/newQuantity"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/update-seat-quantity.ts",
        "previewSeatChange",
        "c15d440e4fbc2a25796818dd07424d3deeeb00bfb28597257063a0a1645e0d95"
    ),
    profile!(
        "cap-v1-17470f7df902263e",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/update-seat-quantity.ts#updateSeatQuantity",
        Session,
        OrganizationOwner,
        Required,
        262_144,
        "stripe_subscription_update",
        ["/organizationId", "/newQuantity"],
        None,
        Some("/organizationId"),
        "apps/web/actions/organization/update-seat-quantity.ts",
        "updateSeatQuantity",
        "c15d440e4fbc2a25796818dd07424d3deeeb00bfb28597257063a0a1645e0d95"
    ),
    profile!(
        "cap-v1-3a394a2798233b0b",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/organization/update-space.ts#updateSpace",
        Session,
        SpaceManager,
        Optional,
        2_097_152,
        "space_icon_storage",
        ["/id", "/name"],
        Some("/id"),
        None,
        "apps/web/actions/organization/space-authorization.ts",
        "space management authorization",
        "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445"
    ),
    profile!(
        "cap-v1-8160d7c3ce8507d9",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/send-download-link.ts#sendDownloadLink",
        Public,
        Public,
        Required,
        8_388_608,
        "resend_download_email",
        ["/email"],
        None,
        None,
        "apps/web/actions/send-download-link.ts",
        "sendDownloadLink",
        "61a1be37fd874652d6384fd70ae5fcd9c5cd6a2bc0f3dca6f03f6595d138bbf1"
    ),
    profile!(
        "cap-v1-d9b654b30f6c362a",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/videos/get-status.ts#getVideoStatus",
        PublicOrSession,
        VideoViewer,
        Required,
        262_144,
        "transcription_and_ai_dispatch",
        ["/videoId"],
        Some("/videoId"),
        None,
        "apps/web/actions/videos/get-status.ts",
        "getVideoStatus",
        "601e57f50366b96accc4232ed054538a1adea3210533f1de6272ea1ea13f60f7"
    ),
    profile!(
        "cap-v1-5e7e4265d65c8365",
        ServerAction,
        "ACTION",
        "action://apps/web/app/(org)/dashboard/_components/Navbar/server.ts#createSpace",
        Session,
        OrganizationMember,
        Optional,
        2_097_152,
        "space_icon_storage",
        ["/name"],
        None,
        None,
        "apps/web/actions/organization/create-space.ts",
        "delegated createSpace action",
        "fb38d13cc669ee5f76b2432afc0ff24bf5706144e686039433cc7ffde83a499d"
    ),
    profile!(
        "cap-v1-d05af581fbeb145e",
        ServerAction,
        "ACTION",
        "action://apps/web/app/(org)/dashboard/_components/Navbar/server.ts#updateSpace",
        Session,
        SpaceManager,
        Optional,
        2_097_152,
        "space_icon_storage",
        ["/id", "/name"],
        Some("/id"),
        None,
        "apps/web/actions/organization/space-authorization.ts",
        "space management authorization",
        "2a656f25f7c73f2342104127d818a56fffd7d05768d787489b65e08f70a43445"
    ),
    profile!(
        "cap-v1-b9fcb0fbd25b2234",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/workflows/import-loom-video.ts#importLoomVideoWorkflow",
        ParentReceipt,
        ParentReceipt,
        Required,
        262_144,
        "loom_storage_and_media_dispatch",
        ["/videoId", "/userId", "/rawFileKey", "/loomVideoId"],
        Some("/videoId"),
        None,
        "apps/web/workflows/import-loom-video.ts",
        "importLoomVideoWorkflow",
        "a76ea2aa58b5540328cb095a92bd8a7a9b14ba08c365a771e46d44c19c2dc972"
    ),
    profile!(
        "cap-v1-bd1b9d67380624f7",
        Workflow,
        "WORKFLOW",
        "workflow://packages/web-domain/src/Loom.ts#LoomImportVideo",
        ParentReceipt,
        ParentReceipt,
        Required,
        262_144,
        "loom_import",
        [
            "/cap/userId",
            "/cap/orgId",
            "/loom/orgId",
            "/loom/video/id",
            "/loom/video/downloadUrl"
        ],
        Some("/loom/video/id"),
        Some("/loom/orgId"),
        "packages/web-domain/src/Loom.ts",
        "LoomImportVideo",
        "eeb572a6baa46459d7479376f6c805ddb9066da87e0e27c6d9d7c06869d72ab7"
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedIntegrationPrincipalV1 {
    pub class: LegacyProtectedIntegrationAuthV1,
    pub actor_id: Option<String>,
    pub tenant_id: Option<String>,
    pub credential_kind: LegacyProtectedIntegrationCredentialKindV1,
    /// Exact durable session/API-key identity, or a stable signed authority
    /// name. Anonymous/public requests leave it absent.
    pub credential_subject_id: Option<String>,
    /// Hash-key version for a session or signed authority. Legacy API keys are
    /// unversioned and therefore leave this absent.
    pub credential_key_version: Option<i64>,
    /// SHA-256 of the exact authenticated credential. Raw credentials must
    /// never be passed to this contract.
    pub credential_digest: Option<String>,
    /// Signed OAuth state expires after ten minutes and remains time-bounded
    /// through provider evidence. Other credential kinds leave this absent.
    pub credential_expires_at_ms: Option<i64>,
    #[serde(default)]
    pub policy_proofs: Vec<LegacyProtectedIntegrationPolicyProofV1>,
    /// Workflows copy the initiating receipt's exact entitlement. Ordinary
    /// carriers leave this absent and the D1 adapter derives their pinned
    /// source entitlement from current authority.
    #[serde(default)]
    pub inherited_entitlement_binding: Option<LegacyProtectedIntegrationEntitlementBindingV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyProtectedIntegrationEnvelopeV1 {
    pub source_operation_id: String,
    pub principal: LegacyProtectedIntegrationPrincipalV1,
    pub replay_origin: LegacyProtectedIntegrationReplayOriginV1,
    /// A fresh server-generated receipt nonce. Generated replay also receives
    /// an atomic `(operation, principal, request_digest)` D1 claim, so an exact
    /// retry reaches pending work instead of creating an unreachable receipt.
    pub request_nonce: String,
    pub payload: Value,
    /// Randomized opaque location of the exact provider request. Released Cap
    /// callers never supply this value; a server-owned narrow vault does.
    pub sealed_request_ref: String,
    /// Deterministic digest of the exact typed plaintext request. This digest,
    /// not the randomized opaque reference, enters canonical request identity.
    pub sealed_request_digest: String,
    /// Exact transport-body digest for signed webhooks. Other carriers may set
    /// it when the wire body is meaningful, but it is never raw body material.
    pub transport_body_digest: Option<String>,
    /// Durable workflow authority is inherited from an initiating protected
    /// integration receipt. Workflow callers provide only this opaque parent
    /// binding; the D1 adapter reloads the actor/tenant/credential tuple.
    pub parent_family: Option<String>,
    pub parent_receipt_id: Option<String>,
    pub parent_request_digest: Option<String>,
    /// Exact authority decision on the immutable parent. A child computes its
    /// own operation-specific binding; this separate digest prevents a valid
    /// parent id/request pair from being combined with another parent's
    /// authority.
    pub parent_authority_binding_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyProtectedIntegrationValidatedV1 {
    pub request_json: String,
    pub request_digest: String,
    pub principal_digest: String,
    pub replay_key_digest: String,
    pub replay_origin: LegacyProtectedIntegrationReplayOriginV1,
    /// Native authenticated tenant supplied by the credential/session layer.
    pub authenticated_tenant_id: Option<String>,
    /// Source-domain identifiers are preserved separately for the D1 alias
    /// resolver; they must never be compared directly with native UUIDs.
    pub legacy_target_id: Option<String>,
    pub legacy_tenant_id: Option<String>,
    pub parent_family: Option<String>,
    pub parent_receipt_id: Option<String>,
    pub parent_request_digest: Option<String>,
    pub parent_authority_binding_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyProtectedIntegrationValidationErrorV1 {
    #[error("unknown protected integration operation")]
    UnknownOperation,
    #[error("protected integration authentication class does not match")]
    AuthenticationMismatch,
    #[error("invalid protected integration principal")]
    InvalidPrincipal,
    #[error("invalid protected integration payload")]
    InvalidPayload,
    #[error("protected integration payload is too large")]
    PayloadTooLarge,
    #[error("invalid protected integration idempotency key")]
    InvalidIdempotency,
    #[error("sealed provider request is required")]
    SealedRequestRequired,
}

#[must_use]
pub fn legacy_protected_integration_profile(
    operation_id: &str,
) -> Option<&'static LegacyProtectedIntegrationProfileV1> {
    LEGACY_PROTECTED_INTEGRATION_PROFILES
        .iter()
        .find(|profile| profile.operation_id == operation_id)
}

/// Domain of the source target identifier. Only values declared `video` or
/// `space` are eligible for a Cap-NanoID-to-native-UUID alias lookup; external
/// Loom identifiers are never confused with Frame business authority.
#[must_use]
pub fn legacy_protected_integration_target_domain(operation_id: &str) -> &'static str {
    match operation_id {
        "cap-v1-60f863b2cb19353f"
        | "cap-v1-5af545d5d20508bd"
        | "cap-v1-17d69edf5d3b06bb"
        | "cap-v1-d9b654b30f6c362a"
        | "cap-v1-b9fcb0fbd25b2234" => "video",
        "cap-v1-3a394a2798233b0b" | "cap-v1-d05af581fbeb145e" => "space",
        "cap-v1-f0a00e93ab606a52" | "cap-v1-bd1b9d67380624f7" => "external",
        _ => "none",
    }
}

/// Domain of the source tenant identifier. The browser/session tenant is
/// already native; this classification applies only to payload identifiers.
#[must_use]
pub fn legacy_protected_integration_tenant_domain(operation_id: &str) -> &'static str {
    match operation_id {
        "cap-v1-c6214b213eaa2360"
        | "cap-v1-49531a09fd9433e7"
        | "cap-v1-679e4241ef5e7383"
        | "cap-v1-0b36c9acda9bd6a2"
        | "cap-v1-60f863b2cb19353f"
        | "cap-v1-f0a00e93ab606a52"
        | "cap-v1-05776c542380771e"
        | "cap-v1-5cd4cac9da73f975"
        | "cap-v1-cb3fade2af06d6bd"
        | "cap-v1-d062d262b013a0cd"
        | "cap-v1-6446dc02a25cef2f"
        | "cap-v1-3f2885312f79698e"
        | "cap-v1-55b98b0f419abf86"
        | "cap-v1-f0ba260c29c295f3"
        | "cap-v1-4054bc310aa16e98"
        | "cap-v1-83721706f7b0e2e6"
        | "cap-v1-9d5f521a096649ec"
        | "cap-v1-3badb07a1a6fe5f9"
        | "cap-v1-50d0ffd9f5f7bcb6"
        | "cap-v1-1186b0fb0aaa2f4d"
        | "cap-v1-0823c0b806bd38a5"
        | "cap-v1-aa00fc906599e89c"
        | "cap-v1-17470f7df902263e"
        | "cap-v1-bd1b9d67380624f7" => "organization",
        _ => "none",
    }
}

/// Replay policy derived from the pinned carrier, never from a new public
/// header. Signed deliveries and durable workflows have natural identities;
/// all other released carriers use bounded generated continuation.
#[must_use]
pub const fn legacy_protected_integration_replay_origin(
    profile: &LegacyProtectedIntegrationProfileV1,
) -> LegacyProtectedIntegrationReplayOriginV1 {
    if matches!(
        profile.auth,
        LegacyProtectedIntegrationAuthV1::SignedWebhook
    ) || matches!(profile.kind, LegacyProtectedIntegrationKindV1::Workflow)
    {
        LegacyProtectedIntegrationReplayOriginV1::Natural
    } else {
        LegacyProtectedIntegrationReplayOriginV1::Generated
    }
}

/// Bind the exact plaintext request accepted by the provider vault. The
/// canonical JSON encoding is internal and deterministic; D1 receives only
/// this digest and an opaque randomized reference.
pub fn legacy_protected_integration_plaintext_request_digest(
    operation_id: &str,
    payload: &Value,
    transport_body_digest: Option<&str>,
) -> Result<String, LegacyProtectedIntegrationValidationErrorV1> {
    if !payload.is_object() || transport_body_digest.is_some_and(|value| !valid_digest(value)) {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    let exact = json!({
        "schema_version": "frame.legacy-protected-integration-provider-request.v1",
        "source_operation_id": operation_id,
        "payload": payload,
        "transport_body_digest": transport_body_digest,
    });
    let encoded = serde_json::to_vec(&exact)
        .map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    Ok(digest(&encoded))
}

/// Local entitlement checks observed in the pinned Cap sources. Conditional
/// space/video feature checks remain part of their payload-specific executor;
/// this function captures the unconditional gates that must pass before an
/// intent is staged.
#[must_use]
pub fn legacy_protected_integration_entitlement(
    operation_id: &str,
) -> LegacyProtectedIntegrationEntitlementV1 {
    match operation_id {
        "cap-v1-f0a00e93ab606a52" => LegacyProtectedIntegrationEntitlementV1::CapInternal,
        "cap-v1-679e4241ef5e7383"
        | "cap-v1-cb3fade2af06d6bd"
        | "cap-v1-d062d262b013a0cd"
        | "cap-v1-4054bc310aa16e98"
        | "cap-v1-83721706f7b0e2e6"
        | "cap-v1-9d5f521a096649ec"
        | "cap-v1-3badb07a1a6fe5f9"
        | "cap-v1-50d0ffd9f5f7bcb6"
        | "cap-v1-1186b0fb0aaa2f4d"
        | "cap-v1-0823c0b806bd38a5" => LegacyProtectedIntegrationEntitlementV1::Pro,
        "cap-v1-3f2885312f79698e" => LegacyProtectedIntegrationEntitlementV1::SubscriptionRead,
        "cap-v1-aa00fc906599e89c" | "cap-v1-17470f7df902263e" => {
            LegacyProtectedIntegrationEntitlementV1::SubscriptionManage
        }
        _ => LegacyProtectedIntegrationEntitlementV1::None,
    }
}

/// Canonical binding copied into the immutable receipt. D1 independently
/// re-evaluates every component, but this digest prevents an executor from
/// mixing evidence from a different actor, credential, policy proof, scope,
/// entitlement, or payload-conditional authority decision.
pub fn legacy_protected_integration_authority_binding_digest(
    profile: &LegacyProtectedIntegrationProfileV1,
    principal: &LegacyProtectedIntegrationPrincipalV1,
    tenant_id: Option<&str>,
    target_id: Option<&str>,
    entitlement_binding: Option<&LegacyProtectedIntegrationEntitlementBindingV1>,
    conditional_bindings: &[LegacyProtectedIntegrationConditionalBindingV1],
) -> Result<String, LegacyProtectedIntegrationValidationErrorV1> {
    validate_policy_proofs(&principal.policy_proofs)?;
    validate_conditional_bindings(conditional_bindings)?;
    let expected_entitlement = legacy_protected_integration_entitlement(profile.operation_id);
    let binding_shape_valid = |binding: &LegacyProtectedIntegrationEntitlementBindingV1| {
        matches!(
            binding.kind.as_str(),
            "cap_internal" | "pro" | "subscription_read" | "subscription_manage" | "ai_owner"
        ) && valid_credential_subject(&binding.subject_id)
            && (0..=9_007_199_254_740_991).contains(&binding.revision)
            && binding
                .expires_at_ms
                .is_none_or(|value| (1..=9_007_199_254_740_991).contains(&value))
    };
    let entitlement_valid = if profile.auth == LegacyProtectedIntegrationAuthV1::ParentReceipt {
        entitlement_binding.is_none_or(binding_shape_valid)
    } else {
        match (expected_entitlement, entitlement_binding) {
            (LegacyProtectedIntegrationEntitlementV1::None, None) => true,
            (expected, Some(binding)) => {
                binding.kind == expected.as_str() && binding_shape_valid(binding)
            }
            _ => false,
        }
    };
    if !entitlement_valid {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal);
    }
    let material = json!({
        "schema_version": "frame.legacy-protected-integration-authority.v1",
        "source_operation_id": profile.operation_id,
        "auth_class": profile.auth.as_str(),
        "authority_class": profile.authority.as_str(),
        "actor_id": principal.actor_id,
        "tenant_id": tenant_id,
        "target_id": target_id,
        "credential_kind": principal.credential_kind.as_str(),
        "credential_subject_id": principal.credential_subject_id,
        "credential_key_version": principal.credential_key_version,
        "credential_digest": principal.credential_digest,
        "credential_expires_at_ms": principal.credential_expires_at_ms,
        "policy_proofs": principal.policy_proofs,
        "entitlement_class": expected_entitlement.as_str(),
        "entitlement_binding": entitlement_binding,
        "conditional_bindings": conditional_bindings,
    });
    let encoded = serde_json::to_vec(&material)
        .map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)?;
    Ok(digest(&encoded))
}

pub fn validate_legacy_protected_integration_envelope(
    envelope: &LegacyProtectedIntegrationEnvelopeV1,
) -> Result<LegacyProtectedIntegrationValidatedV1, LegacyProtectedIntegrationValidationErrorV1> {
    let profile = legacy_protected_integration_profile(&envelope.source_operation_id)
        .ok_or(LegacyProtectedIntegrationValidationErrorV1::UnknownOperation)?;
    validate_principal(profile, &envelope.principal)?;
    validate_replay(profile, envelope)?;

    let encoded = serde_json::to_vec(&envelope.payload)
        .map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    if encoded.len() > profile.max_body_bytes.max(4_096)
        || encoded.len() > LEGACY_PROTECTED_INTEGRATIONS_MAX_BODY_BYTES
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::PayloadTooLarge);
    }
    for pointer in profile.required_paths {
        let Some(value) = envelope.payload.pointer(pointer) else {
            return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
        };
        if value.is_null()
            || (value.as_str().is_some_and(|value| value.trim().is_empty())
                && !(profile.operation_id == "cap-v1-f0a00e93ab606a52"
                    && *pointer == "/loom/video/name"))
            || value.as_array().is_some_and(Vec::is_empty)
        {
            return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
        }
    }
    validate_operation_specific(profile, &envelope.payload)?;

    let legacy_target_id = extract_string(&envelope.payload, profile.target_pointer)?;
    let legacy_tenant_id = extract_string(&envelope.payload, profile.tenant_pointer)?;

    if !valid_sealed_request_ref(&envelope.sealed_request_ref)
        || !valid_digest(&envelope.sealed_request_digest)
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::SealedRequestRequired);
    }
    if matches!(
        profile.auth,
        LegacyProtectedIntegrationAuthV1::SignedWebhook
    ) && envelope
        .transport_body_digest
        .as_deref()
        .is_none_or(|value| !valid_digest(value))
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    if envelope
        .transport_body_digest
        .as_deref()
        .is_some_and(|value| !valid_digest(value))
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    match (
        profile.kind,
        envelope.parent_family.as_deref(),
        envelope.parent_receipt_id.as_deref(),
        envelope.parent_request_digest.as_deref(),
        envelope.parent_authority_binding_digest.as_deref(),
    ) {
        (
            LegacyProtectedIntegrationKindV1::Workflow,
            Some("protected_integrations" | "protected_media"),
            Some(receipt_id),
            Some(request_digest),
            Some(authority_digest),
        ) if valid_uuid(receipt_id)
            && valid_digest(request_digest)
            && valid_digest(authority_digest) => {}
        (LegacyProtectedIntegrationKindV1::Workflow, _, _, _, _) => {
            return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal);
        }
        (_, None, None, None, None) => {}
        _ => return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal),
    }
    let expected_plaintext_digest = legacy_protected_integration_plaintext_request_digest(
        profile.operation_id,
        &envelope.payload,
        envelope.transport_body_digest.as_deref(),
    )?;
    if envelope.sealed_request_digest != expected_plaintext_digest {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }

    // The exact provider input is available only through the server-owned
    // vault. Persist one digest for the entire payload so unknown fields are
    // digest-only by construction instead of relying on a fragile key list.
    let payload_digest = digest(&encoded);
    let request = json!({
        "schema_version": "frame.legacy-protected-integration-request.v1",
        "source_operation_id": profile.operation_id,
        "payload": {
            "digest_only": true,
            "sha256": payload_digest,
        },
        "sealed_request_digest": envelope.sealed_request_digest,
        "transport_body_digest": envelope.transport_body_digest,
        "parent_family": envelope.parent_family,
        "parent_receipt_id": envelope.parent_receipt_id,
        "parent_request_digest": envelope.parent_request_digest,
        "parent_authority_binding_digest": envelope.parent_authority_binding_digest,
    });
    let request_json = serde_json::to_string(&request)
        .map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    let request_digest = digest(request_json.as_bytes());
    let principal_json = serde_json::to_vec(&envelope.principal)
        .map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)?;
    let principal_digest = digest(&principal_json);
    let replay_key_digest = match envelope.replay_origin {
        LegacyProtectedIntegrationReplayOriginV1::Generated => {
            digest(envelope.request_nonce.as_bytes())
        }
        LegacyProtectedIntegrationReplayOriginV1::Natural => {
            natural_replay_key_digest(profile, envelope)?
        }
    };
    Ok(LegacyProtectedIntegrationValidatedV1 {
        request_json,
        request_digest,
        principal_digest,
        replay_key_digest,
        replay_origin: envelope.replay_origin,
        authenticated_tenant_id: envelope.principal.tenant_id.clone(),
        legacy_target_id,
        legacy_tenant_id,
        parent_family: envelope.parent_family.clone(),
        parent_receipt_id: envelope.parent_receipt_id.clone(),
        parent_request_digest: envelope.parent_request_digest.clone(),
        parent_authority_binding_digest: envelope.parent_authority_binding_digest.clone(),
    })
}

fn validate_principal(
    profile: &LegacyProtectedIntegrationProfileV1,
    principal: &LegacyProtectedIntegrationPrincipalV1,
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    if profile.auth != principal.class {
        return Err(LegacyProtectedIntegrationValidationErrorV1::AuthenticationMismatch);
    }
    let actor_valid = principal
        .actor_id
        .as_deref()
        .is_some_and(|actor| !actor.trim().is_empty() && actor.len() <= 255);
    let subject = principal.credential_subject_id.as_deref();
    let version = principal.credential_key_version;
    let credential_digest = principal.credential_digest.as_deref();
    let expiry = principal.credential_expires_at_ms;
    let none = principal.credential_kind == LegacyProtectedIntegrationCredentialKindV1::None
        && subject.is_none()
        && version.is_none()
        && credential_digest.is_none()
        && expiry.is_none();
    let session = principal.credential_kind
        == LegacyProtectedIntegrationCredentialKindV1::SessionToken
        && subject.is_some_and(valid_uuid)
        && version.is_some_and(|value| (1..=65_535).contains(&value))
        && credential_digest.is_some_and(valid_digest)
        && expiry.is_none();
    let api_key = principal.credential_kind == LegacyProtectedIntegrationCredentialKindV1::ApiKey
        && subject.is_some_and(valid_credential_subject)
        && version.is_none()
        && credential_digest.is_some_and(valid_digest)
        && expiry.is_none();
    let signed_state = principal.credential_kind
        == LegacyProtectedIntegrationCredentialKindV1::SignedState
        && subject == Some("google-drive-oauth-state.v1")
        && version == Some(1)
        && credential_digest.is_some_and(valid_digest)
        && expiry.is_some_and(|value| (1..=9_007_199_254_740_991).contains(&value));
    let signed_endpoint = principal.credential_kind
        == LegacyProtectedIntegrationCredentialKindV1::SignedEndpoint
        && subject == Some("media-server-webhook.endpoint.v1")
        && version == Some(1)
        && credential_digest.is_some_and(valid_digest)
        && expiry.is_none();

    validate_policy_proofs(&principal.policy_proofs)?;
    let inherited_entitlement_valid = match (
        profile.auth,
        principal.inherited_entitlement_binding.as_ref(),
    ) {
        (_, None) => true,
        (LegacyProtectedIntegrationAuthV1::ParentReceipt, Some(binding)) => {
            matches!(
                binding.kind.as_str(),
                "cap_internal" | "pro" | "subscription_read" | "subscription_manage" | "ai_owner"
            ) && valid_credential_subject(&binding.subject_id)
                && (0..=9_007_199_254_740_991).contains(&binding.revision)
                && binding
                    .expires_at_ms
                    .is_none_or(|value| (1..=9_007_199_254_740_991).contains(&value))
        }
        (_, Some(_)) => false,
    };
    let policy_allowed = matches!(
        profile.authority,
        LegacyProtectedIntegrationAuthorityV1::VideoViewer
            | LegacyProtectedIntegrationAuthorityV1::ParentReceipt
    ) || principal.policy_proofs.is_empty();
    let valid = policy_allowed
        && inherited_entitlement_valid
        && match profile.auth {
            LegacyProtectedIntegrationAuthV1::Session => actor_valid && session,
            LegacyProtectedIntegrationAuthV1::SessionOrApiKey => {
                actor_valid && (session || api_key)
            }
            LegacyProtectedIntegrationAuthV1::AnonymousOrSessionOrApiKey => {
                (principal.actor_id.is_none() && principal.tenant_id.is_none() && none)
                    || (actor_valid && (session || api_key))
            }
            LegacyProtectedIntegrationAuthV1::SignedState => actor_valid && signed_state,
            LegacyProtectedIntegrationAuthV1::SignedWebhook => {
                principal.actor_id.is_none() && principal.tenant_id.is_none() && signed_endpoint
            }
            LegacyProtectedIntegrationAuthV1::Public => {
                principal.actor_id.is_none() && principal.tenant_id.is_none() && none
            }
            LegacyProtectedIntegrationAuthV1::PublicOrSession => {
                (principal.actor_id.is_none() && principal.tenant_id.is_none() && none)
                    || (actor_valid && session)
            }
            LegacyProtectedIntegrationAuthV1::ParentReceipt => {
                (principal.actor_id.is_none() && (none || signed_endpoint))
                    || (actor_valid && (session || api_key || signed_state))
            }
        };
    if valid {
        Ok(())
    } else {
        Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)
    }
}

fn validate_policy_proofs(
    proofs: &[LegacyProtectedIntegrationPolicyProofV1],
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    if proofs.len() > 8
        || proofs.iter().any(|proof| {
            !valid_uuid(&proof.target_id)
                || !valid_uuid(&proof.subject_id)
                || !(0..=9_007_199_254_740_991).contains(&proof.revision)
                || !valid_digest(&proof.audit_digest)
                || !matches!(
                    proof.kind.as_str(),
                    "owner_bypass"
                        | "video_password"
                        | "space_password"
                        | "unprotected_video_policy"
                )
        })
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal);
    }
    Ok(())
}

fn validate_conditional_bindings(
    bindings: &[LegacyProtectedIntegrationConditionalBindingV1],
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    if bindings.len() > 8
        || bindings.iter().any(|binding| {
            !valid_credential_subject(&binding.subject_id)
                || !(0..=9_007_199_254_740_991).contains(&binding.revision)
                || binding
                    .value
                    .is_some_and(|value| !(0..=9_007_199_254_740_991).contains(&value))
                || !matches!(
                    binding.kind.as_str(),
                    "video_existing_owner"
                        | "video_new_organization_member"
                        | "video_duration_pro"
                        | "space_password_pro"
                        | "space_settings_pro"
                        | "space_publish_owner_pro"
                        | "seat_capacity"
                )
        })
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal);
    }
    Ok(())
}

fn validate_replay(
    profile: &LegacyProtectedIntegrationProfileV1,
    envelope: &LegacyProtectedIntegrationEnvelopeV1,
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    if envelope.request_nonce.trim().is_empty() || envelope.request_nonce.len() > 512 {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidIdempotency);
    }
    if envelope.replay_origin != legacy_protected_integration_replay_origin(profile) {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidIdempotency);
    }
    Ok(())
}

fn natural_replay_key_digest(
    profile: &LegacyProtectedIntegrationProfileV1,
    envelope: &LegacyProtectedIntegrationEnvelopeV1,
) -> Result<String, LegacyProtectedIntegrationValidationErrorV1> {
    let material = if matches!(
        profile.auth,
        LegacyProtectedIntegrationAuthV1::SignedWebhook
    ) {
        json!({
            "kind": "signed_webhook_delivery",
            "operation": profile.operation_id,
            "transport_body_digest": envelope.transport_body_digest,
        })
    } else {
        match profile.operation_id {
            "cap-v1-b9fcb0fbd25b2234" => json!({
                "kind": "loom_import_workflow",
                "video_id": envelope.payload.pointer("/videoId"),
                "parent_family": envelope.parent_family,
                "parent_receipt_id": envelope.parent_receipt_id,
                "parent_request_digest": envelope.parent_request_digest,
                "parent_authority_binding_digest": envelope.parent_authority_binding_digest,
            }),
            "cap-v1-bd1b9d67380624f7" => json!({
                "kind": "loom_domain_import_workflow",
                "user_id": envelope.payload.pointer("/cap/userId"),
                "organization_id": envelope.payload.pointer("/loom/orgId"),
                "video_id": envelope.payload.pointer("/loom/video/id"),
                "attempt": envelope.payload.pointer("/attempt"),
            }),
            _ => return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidIdempotency),
        }
    };
    let encoded = serde_json::to_vec(&material)
        .map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidIdempotency)?;
    Ok(digest(&encoded))
}

fn validate_operation_specific(
    profile: &LegacyProtectedIntegrationProfileV1,
    payload: &Value,
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    match profile.operation_id {
        "cap-v1-8a1e6c87b4426f93" => {
            exact_object_keys(payload, &["version", "target", "arch"])?;
            for key in ["version", "target", "arch"] {
                if payload
                    .get(key)
                    .and_then(Value::as_str)
                    .is_none_or(|value| {
                        value.is_empty()
                            || value.len() > 128
                            || !value.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
                            })
                    })
                {
                    return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
                }
            }
        }
        "cap-v1-221a713f60d7528f" => {
            exact_object_keys(payload, &["id", "name"])?;
            validate_loom_identifier(payload.pointer("/id"))?;
            if payload.pointer("/name").is_some_and(|value| {
                value.as_str().is_none_or(|name| {
                    name.len() > 255
                        || name.chars().any(char::is_control)
                        || name.contains(['/', '\\'])
                })
            }) {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-422afbf05f09bf4f" => {
            exact_object_keys(payload, &["url"])?;
            validate_public_loom_url(payload.pointer("/url"))?;
        }
        "cap-v1-8160d7c3ce8507d9" => {
            exact_object_keys(payload, &["email"])?;
            if payload
                .pointer("/email")
                .and_then(Value::as_str)
                .is_none_or(|email| {
                    email.len() > 254
                        || email.contains('<')
                        || !email.contains('@')
                        || !email
                            .rsplit_once('@')
                            .is_some_and(|(_, domain)| domain.contains('.'))
                })
            {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-d9b654b30f6c362a" => {
            exact_object_keys(payload, &["videoId"])?;
        }
        "cap-v1-0c233c1115838206" | "cap-v1-5e7e4265d65c8365" => {
            if payload.pointer("/organizationId").is_some() || payload.pointer("/orgId").is_some() {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-60f863b2cb19353f" => {
            exact_object_keys(
                payload,
                &[
                    "recordingMode",
                    "isScreenshot",
                    "videoId",
                    "name",
                    "durationInSecs",
                    "width",
                    "height",
                    "fps",
                    "orgId",
                    "_frame",
                ],
            )?;
            if ["/videoId", "/orgId"].iter().any(|pointer| {
                payload.pointer(pointer).is_some_and(|value| {
                    value
                        .as_str()
                        .is_none_or(|value| !valid_legacy_nanoid(value))
                })
            }) || payload.pointer("/recordingMode").is_some_and(|value| {
                !matches!(
                    value.as_str(),
                    Some("hls" | "desktopMP4" | "desktopSegments")
                )
            }) || payload
                .pointer("/isScreenshot")
                .and_then(Value::as_bool)
                .is_none()
                || payload
                    .pointer("/name")
                    .is_some_and(|value| value.as_str().is_none_or(|name| name.len() > 255))
                || ["/durationInSecs", "/width", "/height", "/fps"]
                    .iter()
                    .any(|pointer| {
                        payload.pointer(pointer).is_some_and(|value| {
                            value.as_f64().is_none_or(|number| !number.is_finite())
                        })
                    })
                || payload.pointer("/_frame").is_none_or(|context| {
                    exact_object_keys(
                        context,
                        &[
                            "clientSupportsGoogleDriveUpload",
                            "clientSupportsUploadProgress",
                        ],
                    )
                    .is_err()
                        || context
                            .pointer("/clientSupportsGoogleDriveUpload")
                            .and_then(Value::as_bool)
                            .is_none()
                        || context
                            .pointer("/clientSupportsUploadProgress")
                            .and_then(Value::as_bool)
                            .is_none()
                })
            {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-f0a00e93ab606a52" => {
            let cap_org_id = payload.pointer("/cap/orgId").and_then(Value::as_str);
            let loom_org_id = payload.pointer("/loom/orgId").and_then(Value::as_str);
            if ["/cap/orgId", "/loom/userId", "/loom/orgId"]
                .iter()
                .any(|pointer| {
                    payload
                        .pointer(pointer)
                        .and_then(Value::as_str)
                        .is_none_or(|value| !valid_legacy_nanoid(value))
                })
                || cap_org_id != loom_org_id
                || payload
                    .pointer("/loom/video/name")
                    .is_none_or(|value| !value.is_string())
                || [
                    "/loom/video/width",
                    "/loom/video/height",
                    "/loom/video/fps",
                    "/loom/video/durationSecs",
                    "/attempt",
                ]
                .iter()
                .any(|pointer| {
                    payload
                        .pointer(pointer)
                        .is_some_and(|value| !value.is_number())
                })
            {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
            validate_loom_identifier(payload.pointer("/loom/video/id"))?;
            validate_external_download_url(payload.pointer("/loom/video/downloadUrl"))?;
        }
        "cap-v1-49531a09fd9433e7" => {
            let has_exchange = payload.pointer("/code").and_then(Value::as_str).is_some()
                && payload.pointer("/state").and_then(Value::as_str).is_some();
            if !has_exchange || payload.pointer("/error").is_some() {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-b9fcb0fbd25b2234" => {
            let user_id = payload.pointer("/userId").and_then(Value::as_str);
            let video_id = payload.pointer("/videoId").and_then(Value::as_str);
            let raw_file_key = payload.pointer("/rawFileKey").and_then(Value::as_str);
            let expected_prefix = user_id
                .zip(video_id)
                .map(|(user_id, video_id)| format!("{user_id}/{video_id}/raw-upload."));
            if user_id.is_none_or(|value| !valid_legacy_nanoid(value))
                || video_id.is_none_or(|value| !valid_legacy_nanoid(value))
                || payload
                    .pointer("/loomDownloadUrl")
                    .and_then(Value::as_str)
                    .is_none()
                || raw_file_key.is_none_or(|raw_file_key| {
                    expected_prefix.as_deref().is_none_or(|prefix| {
                        raw_file_key.strip_prefix(prefix).is_none_or(|extension| {
                            extension.is_empty()
                                || extension.len() > 16
                                || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
                        })
                    })
                })
            {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
            validate_loom_identifier(payload.pointer("/loomVideoId"))?;
        }
        "cap-v1-bd1b9d67380624f7" => {
            if payload
                .pointer("/attempt")
                .is_some_and(|value| !value.is_number())
                || ["/cap/userId", "/cap/orgId", "/loom/orgId"]
                    .iter()
                    .any(|pointer| {
                        payload
                            .pointer(pointer)
                            .and_then(Value::as_str)
                            .is_none_or(|value| !valid_legacy_nanoid(value))
                    })
            {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
            validate_loom_identifier(payload.pointer("/loom/video/id"))?;
            validate_external_download_url(payload.pointer("/loom/video/downloadUrl"))?;
        }
        "cap-v1-d062d262b013a0cd" => {
            if payload
                .pointer("/rows")
                .and_then(Value::as_array)
                .is_none_or(|rows| rows.is_empty() || rows.len() > 500)
            {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-aa00fc906599e89c" | "cap-v1-17470f7df902263e" => {
            if payload
                .pointer("/newQuantity")
                .and_then(Value::as_i64)
                .is_none_or(|quantity| !(1..=500).contains(&quantity))
            {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        "cap-v1-0823c0b806bd38a5" => {
            let valid = payload
                .pointer("/domain")
                .and_then(Value::as_str)
                .is_some_and(|domain| {
                    domain.len() <= 253
                        && !domain.contains('/')
                        && domain.contains('.')
                        && domain
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
                });
            if !valid {
                return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
            }
        }
        _ => {}
    }
    Ok(())
}

fn exact_object_keys(
    payload: &Value,
    allowed: &[&str],
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    let object = payload
        .as_object()
        .ok_or(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    Ok(())
}

fn validate_loom_identifier(
    value: Option<&Value>,
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    if value.and_then(Value::as_str).is_none_or(|identifier| {
        !(10..=128).contains(&identifier.len())
            || !identifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }) {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    Ok(())
}

fn validate_public_loom_url(
    value: Option<&Value>,
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    let parsed =
        Url::parse(raw).map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    if parsed.scheme() != "https"
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.host_str(), Some("loom.com" | "www.loom.com"))
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    let segments = parsed
        .path_segments()
        .ok_or(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() != 2 || !matches!(segments[0], "share" | "session") {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    validate_loom_identifier(Some(&Value::String(segments[1].to_owned())))
}

fn validate_external_download_url(
    value: Option<&Value>,
) -> Result<(), LegacyProtectedIntegrationValidationErrorV1> {
    let raw = value
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 8_192)
        .ok_or(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    let parsed =
        Url::parse(raw).map_err(|_| LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    if parsed.scheme() != "https"
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.fragment().is_some()
    {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    let host = parsed
        .host()
        .ok_or(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?;
    let unsafe_host = match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            domain != "loom.com" && !domain.ends_with(".loom.com")
        }
        Host::Ipv4(_) | Host::Ipv6(_) => true,
    };
    if unsafe_host {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    Ok(())
}

fn extract_string(
    payload: &Value,
    pointer: Option<&str>,
) -> Result<Option<String>, LegacyProtectedIntegrationValidationErrorV1> {
    let Some(pointer) = pointer else {
        return Ok(None);
    };
    let Some(value) = payload.pointer(pointer) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)?
        .trim();
    if value.is_empty() || value.len() > 512 {
        return Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload);
    }
    Ok(Some(value.to_owned()))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn valid_credential_subject(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_legacy_nanoid(value: &str) -> bool {
    value.len() == 15
        && value.bytes().all(|byte| {
            matches!(
                byte,
                b'0'..=b'9'
                    | b'a'..=b'h'
                    | b'j'
                    | b'k'
                    | b'm'
                    | b'n'
                    | b'p'..=b't'
                    | b'v'..=b'z'
            )
        })
}

fn valid_sealed_request_ref(value: &str) -> bool {
    value
        .strip_prefix("frame-pi-request-v1:")
        .is_some_and(valid_digest)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> LegacyProtectedIntegrationPrincipalV1 {
        LegacyProtectedIntegrationPrincipalV1 {
            class: LegacyProtectedIntegrationAuthV1::Session,
            actor_id: Some("00000000-0000-7000-8000-000000000001".into()),
            tenant_id: None,
            credential_kind: LegacyProtectedIntegrationCredentialKindV1::SessionToken,
            credential_subject_id: Some("00000000-0000-7000-8000-000000000011".into()),
            credential_key_version: Some(1),
            credential_digest: Some("1".repeat(64)),
            credential_expires_at_ms: None,
            policy_proofs: Vec::new(),
            inherited_entitlement_binding: None,
        }
    }

    fn session_or_api_key() -> LegacyProtectedIntegrationPrincipalV1 {
        LegacyProtectedIntegrationPrincipalV1 {
            class: LegacyProtectedIntegrationAuthV1::SessionOrApiKey,
            ..session()
        }
    }

    fn workflow_parent() -> LegacyProtectedIntegrationPrincipalV1 {
        LegacyProtectedIntegrationPrincipalV1 {
            class: LegacyProtectedIntegrationAuthV1::ParentReceipt,
            actor_id: Some("00000000-0000-7000-8000-000000000001".into()),
            tenant_id: Some("00000000-0000-7000-8000-000000000010".into()),
            credential_kind: LegacyProtectedIntegrationCredentialKindV1::SessionToken,
            credential_subject_id: Some("00000000-0000-7000-8000-000000000011".into()),
            credential_key_version: Some(1),
            credential_digest: Some("1".repeat(64)),
            credential_expires_at_ms: None,
            policy_proofs: Vec::new(),
            inherited_entitlement_binding: None,
        }
    }

    fn sealed_request(
        operation_id: &str,
        payload: Value,
        principal: LegacyProtectedIntegrationPrincipalV1,
        transport_body_digest: Option<String>,
    ) -> LegacyProtectedIntegrationEnvelopeV1 {
        let sealed_request_digest = legacy_protected_integration_plaintext_request_digest(
            operation_id,
            &payload,
            transport_body_digest.as_deref(),
        )
        .expect("test request must have a deterministic digest");
        let profile = legacy_protected_integration_profile(operation_id).expect("test profile");
        let parent = (profile.kind == LegacyProtectedIntegrationKindV1::Workflow).then(|| {
            (
                "protected_media".to_owned(),
                "00000000-0000-7000-8000-000000000098".to_owned(),
                "d".repeat(64),
                "e".repeat(64),
            )
        });
        LegacyProtectedIntegrationEnvelopeV1 {
            source_operation_id: operation_id.into(),
            principal,
            replay_origin: legacy_protected_integration_replay_origin(profile),
            request_nonce: "00000000-0000-7000-8000-000000000099".into(),
            payload,
            sealed_request_ref: format!("frame-pi-request-v1:{}", "a".repeat(64)),
            sealed_request_digest,
            transport_body_digest,
            parent_family: parent.as_ref().map(|(family, _, _, _)| family.clone()),
            parent_receipt_id: parent
                .as_ref()
                .map(|(_, receipt_id, _, _)| receipt_id.clone()),
            parent_request_digest: parent
                .as_ref()
                .map(|(_, _, request_digest, _)| request_digest.clone()),
            parent_authority_binding_digest: parent
                .map(|(_, _, _, authority_digest)| authority_digest),
        }
    }

    #[test]
    fn profile_inventory_is_exact_and_unique() {
        assert_eq!(
            LEGACY_PROTECTED_INTEGRATION_PROFILES.len(),
            LEGACY_PROTECTED_INTEGRATIONS_OPERATION_COUNT
        );
        let mut ids = LEGACY_PROTECTED_INTEGRATION_PROFILES
            .iter()
            .map(|profile| profile.operation_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), LEGACY_PROTECTED_INTEGRATIONS_OPERATION_COUNT);
        assert!(LEGACY_PROTECTED_INTEGRATION_PROFILES.iter().all(|profile| {
            profile.source_sha256.len() == 64
                && profile
                    .source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
        }));
        assert_eq!(
            legacy_protected_integration_entitlement("cap-v1-f0a00e93ab606a52"),
            LegacyProtectedIntegrationEntitlementV1::CapInternal
        );
        assert_eq!(
            legacy_protected_integration_entitlement("cap-v1-0823c0b806bd38a5"),
            LegacyProtectedIntegrationEntitlementV1::Pro
        );
        assert_eq!(
            legacy_protected_integration_entitlement("cap-v1-17470f7df902263e"),
            LegacyProtectedIntegrationEntitlementV1::SubscriptionManage
        );
        assert_eq!(
            legacy_protected_integration_profile("cap-v1-dfbbc4c0b56179d1")
                .expect("desktop logs")
                .auth,
            LegacyProtectedIntegrationAuthV1::AnonymousOrSessionOrApiKey
        );
        assert_eq!(
            legacy_protected_integration_profile("cap-v1-49531a09fd9433e7")
                .expect("Google callback")
                .auth,
            LegacyProtectedIntegrationAuthV1::SignedState
        );
        for operation_id in [
            "cap-v1-8a1e6c87b4426f93",
            "cap-v1-221a713f60d7528f",
            "cap-v1-422afbf05f09bf4f",
            "cap-v1-8160d7c3ce8507d9",
        ] {
            assert_eq!(
                legacy_protected_integration_profile(operation_id)
                    .expect("public source carrier")
                    .auth,
                LegacyProtectedIntegrationAuthV1::Public
            );
        }
    }

    #[test]
    fn loom_import_workflow_uses_the_canonical_organization_tenant() {
        assert_eq!(
            legacy_protected_integration_tenant_domain("cap-v1-bd1b9d67380624f7"),
            "organization"
        );
        assert_eq!(
            legacy_protected_integration_target_domain("cap-v1-bd1b9d67380624f7"),
            "external"
        );
    }

    #[test]
    fn exact_provider_input_is_digest_only_and_requires_server_sealing() {
        let payload = json!({
            "provider":"aws", "accessKeyId":"AKIA-secret",
            "secretAccessKey":"do-not-store", "endpoint":"https://s3.example",
            "bucketName":"frame", "region":"us-east-1"
        });
        let mut envelope = sealed_request(
            "cap-v1-9d91d42d52472a83",
            payload,
            session_or_api_key(),
            None,
        );
        envelope.sealed_request_ref = "frame-pi-request-v1:not-a-digest".into();
        assert_eq!(
            validate_legacy_protected_integration_envelope(&envelope),
            Err(LegacyProtectedIntegrationValidationErrorV1::SealedRequestRequired)
        );
        envelope.sealed_request_ref = format!("frame-pi-request-v1:{}", "a".repeat(64));
        let Ok(validated) = validate_legacy_protected_integration_envelope(&envelope) else {
            panic!("valid sealed S3 envelope must pass validation");
        };
        assert!(!validated.request_json.contains("AKIA-secret"));
        assert!(!validated.request_json.contains("do-not-store"));
        assert!(validated.request_json.contains("digest_only"));
        envelope.sealed_request_ref = format!("frame-pi-request-v1:{}", "b".repeat(64));
        let Ok(resealed) = validate_legacy_protected_integration_envelope(&envelope) else {
            panic!("valid resealed S3 envelope must pass validation");
        };
        assert_eq!(validated.request_digest, resealed.request_digest);
        assert_eq!(validated.replay_key_digest, resealed.replay_key_digest);
    }

    #[test]
    fn generated_replay_preserves_separate_legacy_and_native_id_domains() {
        let mut envelope = sealed_request(
            "cap-v1-60f863b2cb19353f",
            json!({
                "orgId":"01h000000000001",
                "isScreenshot":false,
                "_frame":{
                    "clientSupportsGoogleDriveUpload":false,
                    "clientSupportsUploadProgress":true,
                },
            }),
            LegacyProtectedIntegrationPrincipalV1 {
                tenant_id: Some("org-a".into()),
                ..session_or_api_key()
            },
            None,
        );
        let first = validate_legacy_protected_integration_envelope(&envelope)
            .expect("alias resolution is deferred to D1");
        assert_eq!(first.authenticated_tenant_id.as_deref(), Some("org-a"));
        assert_eq!(first.legacy_tenant_id.as_deref(), Some("01h000000000001"));
        envelope.request_nonce = "00000000-0000-7000-8000-000000000100".into();
        let retry = validate_legacy_protected_integration_envelope(&envelope)
            .expect("generated retry validates");
        assert_eq!(first.request_digest, retry.request_digest);
        assert_ne!(first.replay_key_digest, retry.replay_key_digest);
        assert_eq!(
            first.replay_origin,
            LegacyProtectedIntegrationReplayOriginV1::Generated
        );
    }

    #[test]
    fn webhook_requires_a_digest_not_a_raw_secret() {
        let envelope = sealed_request(
            "cap-v1-17d69edf5d3b06bb",
            json!({"videoId":"video-1"}),
            LegacyProtectedIntegrationPrincipalV1 {
                class: LegacyProtectedIntegrationAuthV1::SignedWebhook,
                actor_id: None,
                tenant_id: None,
                credential_kind: LegacyProtectedIntegrationCredentialKindV1::SignedEndpoint,
                credential_subject_id: Some("media-server-webhook.endpoint.v1".into()),
                credential_key_version: Some(1),
                credential_digest: Some("webhook-secret".into()),
                credential_expires_at_ms: None,
                policy_proofs: Vec::new(),
                inherited_entitlement_binding: None,
            },
            Some("c".repeat(64)),
        );
        assert_eq!(
            validate_legacy_protected_integration_envelope(&envelope),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)
        );
    }

    #[test]
    fn concrete_credential_tuples_are_exact_and_unambiguous() {
        let mut session_envelope = sealed_request(
            "cap-v1-30b7af7323aa2c37",
            json!({"feedback":"hello"}),
            session_or_api_key(),
            None,
        );
        assert!(validate_legacy_protected_integration_envelope(&session_envelope).is_ok());

        session_envelope.principal.credential_expires_at_ms = Some(1);
        assert_eq!(
            validate_legacy_protected_integration_envelope(&session_envelope),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)
        );
        session_envelope.principal.credential_expires_at_ms = None;
        session_envelope.principal.credential_key_version = None;
        assert_eq!(
            validate_legacy_protected_integration_envelope(&session_envelope),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)
        );

        let mut api_key = session_or_api_key();
        api_key.credential_kind = LegacyProtectedIntegrationCredentialKindV1::ApiKey;
        api_key.credential_subject_id = Some("api-key-1".into());
        api_key.credential_key_version = None;
        let mut api_envelope = sealed_request(
            "cap-v1-30b7af7323aa2c37",
            json!({"feedback":"hello"}),
            api_key,
            None,
        );
        assert!(validate_legacy_protected_integration_envelope(&api_envelope).is_ok());
        api_envelope.principal.credential_key_version = Some(1);
        assert_eq!(
            validate_legacy_protected_integration_envelope(&api_envelope),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)
        );

        let signed = LegacyProtectedIntegrationPrincipalV1 {
            class: LegacyProtectedIntegrationAuthV1::SignedState,
            actor_id: Some("00000000-0000-7000-8000-000000000001".into()),
            tenant_id: None,
            credential_kind: LegacyProtectedIntegrationCredentialKindV1::SignedState,
            credential_subject_id: Some("google-drive-oauth-state.v1".into()),
            credential_key_version: Some(1),
            credential_digest: Some("2".repeat(64)),
            credential_expires_at_ms: Some(1_800_000_000_000),
            policy_proofs: Vec::new(),
            inherited_entitlement_binding: None,
        };
        let mut state_envelope = sealed_request(
            "cap-v1-49531a09fd9433e7",
            json!({"code":"oauth-code","state":"signed-state"}),
            signed,
            None,
        );
        assert!(validate_legacy_protected_integration_envelope(&state_envelope).is_ok());
        state_envelope.principal.credential_expires_at_ms = None;
        assert_eq!(
            validate_legacy_protected_integration_envelope(&state_envelope),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPrincipal)
        );
    }

    #[test]
    fn authority_binding_changes_with_the_exact_credential() {
        let profile = legacy_protected_integration_profile("cap-v1-30b7af7323aa2c37")
            .expect("feedback profile");
        let first = session_or_api_key();
        let mut rotated = first.clone();
        rotated.credential_key_version = Some(2);
        rotated.credential_digest = Some("2".repeat(64));
        let first_digest = legacy_protected_integration_authority_binding_digest(
            profile,
            &first,
            None,
            None,
            None,
            &[],
        )
        .expect("first binding");
        let rotated_digest = legacy_protected_integration_authority_binding_digest(
            profile,
            &rotated,
            None,
            None,
            None,
            &[],
        )
        .expect("rotated binding");
        assert_ne!(first_digest, rotated_digest);
    }

    #[test]
    fn public_loom_request_rejects_ssrf_and_unknown_fields() {
        for url in [
            "http://www.loom.com/share/abcdefghij",
            "https://user@www.loom.com/share/abcdefghij",
            "https://127.0.0.1/share/abcdefghij",
            "https://www.loom.com.evil.test/share/abcdefghij",
            "https://www.loom.com/share/abcdefghij?next=http://127.0.0.1",
        ] {
            let envelope = sealed_request(
                "cap-v1-422afbf05f09bf4f",
                json!({"url":url}),
                public_principal_for_test(LegacyProtectedIntegrationAuthV1::Public),
                None,
            );
            assert_eq!(
                validate_legacy_protected_integration_envelope(&envelope),
                Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
            );
        }
        let unknown = sealed_request(
            "cap-v1-422afbf05f09bf4f",
            json!({"url":"https://www.loom.com/share/abcdefghij","redirect":"https://evil.test"}),
            public_principal_for_test(LegacyProtectedIntegrationAuthV1::Public),
            None,
        );
        assert_eq!(
            validate_legacy_protected_integration_envelope(&unknown),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
        );
    }

    #[test]
    fn loom_domain_attempt_is_part_of_natural_replay_identity() {
        let payload = json!({
            "cap":{"userId":"01h000000000101","orgId":"01h000000000001"},
            "loom":{"orgId":"01h000000000001","video":{"id":"loom-video-1","downloadUrl":"https://cdn.loom.com/video"}},
        });
        let parent = workflow_parent();
        let first = sealed_request(
            "cap-v1-bd1b9d67380624f7",
            payload.clone(),
            parent.clone(),
            None,
        );
        let first = validate_legacy_protected_integration_envelope(&first)
            .expect("default attempt validates");
        let retry = sealed_request(
            "cap-v1-bd1b9d67380624f7",
            json!({
                "cap":{"userId":"01h000000000101","orgId":"01h000000000001"},
                "loom":{"orgId":"01h000000000001","video":{"id":"loom-video-1","downloadUrl":"https://cdn.loom.com/video"}},
                "attempt":1,
            }),
            parent,
            None,
        );
        let retry = validate_legacy_protected_integration_envelope(&retry)
            .expect("explicit retry attempt validates");
        assert_ne!(first.replay_key_digest, retry.replay_key_digest);
    }

    #[test]
    fn loom_workflows_reject_identity_paths_and_unsafe_downloads() {
        let b9_payload = json!({
            "videoId":"01h000000000004",
            "userId":"01h000000000101",
            "rawFileKey":"01h000000000101/01h000000000004/raw-upload.mp4",
            "bucketId":null,
            "loomDownloadUrl":"unused-source-value",
            "loomVideoId":"loom-video-1",
        });
        assert!(
            validate_legacy_protected_integration_envelope(&sealed_request(
                "cap-v1-b9fcb0fbd25b2234",
                b9_payload.clone(),
                workflow_parent(),
                None,
            ))
            .is_ok()
        );
        for raw_file_key in [
            "01h000000000102/01h000000000004/raw-upload.mp4",
            "01h000000000101/01h000000000005/raw-upload.mp4",
            "../01h000000000004/raw-upload.mp4",
            "01h000000000101/01h000000000004/raw-upload.m$p4",
        ] {
            let mut payload = b9_payload.clone();
            payload["rawFileKey"] = Value::String(raw_file_key.into());
            assert_eq!(
                validate_legacy_protected_integration_envelope(&sealed_request(
                    "cap-v1-b9fcb0fbd25b2234",
                    payload,
                    workflow_parent(),
                    None,
                )),
                Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
            );
        }

        let bd1_payload = json!({
            "cap":{"userId":"01h000000000101","orgId":"01h000000000001"},
            "loom":{"orgId":"01h000000000001","video":{
                "id":"loom-video-1","downloadUrl":"https://cdn.loom.com/video?token=signed"
            }},
        });
        assert!(
            validate_legacy_protected_integration_envelope(&sealed_request(
                "cap-v1-bd1b9d67380624f7",
                bd1_payload.clone(),
                workflow_parent(),
                None,
            ))
            .is_ok()
        );
        let mut missing_cap_org = bd1_payload.clone();
        missing_cap_org["cap"]
            .as_object_mut()
            .expect("cap object")
            .remove("orgId");
        assert_eq!(
            validate_legacy_protected_integration_envelope(&sealed_request(
                "cap-v1-bd1b9d67380624f7",
                missing_cap_org,
                workflow_parent(),
                None,
            )),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
        );
        for unsafe_url in [
            "http://cdn.loom.com/video",
            "https://user@cdn.loom.com/video",
            "https://127.0.0.1/video",
            "https://10.0.0.1/video",
            "https://[::1]/video",
            "https://metadata.internal/video",
        ] {
            let mut payload = bd1_payload.clone();
            payload["loom"]["video"]["downloadUrl"] = Value::String(unsafe_url.into());
            assert_eq!(
                validate_legacy_protected_integration_envelope(&sealed_request(
                    "cap-v1-bd1b9d67380624f7",
                    payload,
                    workflow_parent(),
                    None,
                )),
                Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
            );
        }
    }

    #[test]
    fn selector_free_space_create_and_f60_source_shapes_are_exact() {
        for operation_id in ["cap-v1-0c233c1115838206", "cap-v1-5e7e4265d65c8365"] {
            let profile =
                legacy_protected_integration_profile(operation_id).expect("create-space profile");
            assert_eq!(
                profile.authority,
                LegacyProtectedIntegrationAuthorityV1::OrganizationMember
            );
            assert_eq!(profile.required_paths, &["/name"]);
            assert_eq!(profile.tenant_pointer, None);
            assert_eq!(
                legacy_protected_integration_tenant_domain(operation_id),
                "none"
            );
            assert!(
                validate_legacy_protected_integration_envelope(&sealed_request(
                    operation_id,
                    json!({"name":"source-selected tenant"}),
                    session(),
                    None,
                ))
                .is_ok()
            );
            for selector in ["organizationId", "orgId"] {
                let mut payload = json!({"name":"forged selector"});
                payload[selector] = json!("01h000000000001");
                assert_eq!(
                    validate_legacy_protected_integration_envelope(&sealed_request(
                        operation_id,
                        payload,
                        session(),
                        None,
                    )),
                    Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
                );
            }
        }

        let f60 = json!({
            "recordingMode":"desktopMP4",
            "isScreenshot":false,
            "videoId":"01h000000000004",
            "name":"Recording",
            "durationInSecs":300.5,
            "width":1920,
            "height":1080,
            "fps":60,
            "orgId":"01h000000000001",
            "_frame":{
                "clientSupportsGoogleDriveUpload":true,
                "clientSupportsUploadProgress":true,
            },
        });
        assert!(
            validate_legacy_protected_integration_envelope(&sealed_request(
                "cap-v1-60f863b2cb19353f",
                f60.clone(),
                session_or_api_key(),
                None,
            ))
            .is_ok()
        );
        for mutation in [
            json!({"unexpected":true}),
            json!({"durationInSecs":"300.5"}),
            json!({"videoId":"not-a-cap-id"}),
            json!({"_frame":{"clientSupportsGoogleDriveUpload":true}}),
        ] {
            let mut invalid = f60.clone();
            invalid
                .as_object_mut()
                .expect("f60 object")
                .extend(mutation.as_object().expect("mutation object").clone());
            assert_eq!(
                validate_legacy_protected_integration_envelope(&sealed_request(
                    "cap-v1-60f863b2cb19353f",
                    invalid,
                    session_or_api_key(),
                    None,
                )),
                Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
            );
        }
    }

    #[test]
    fn loom_http_parent_is_fully_bindable_and_retry_parent_scopes_replay() {
        let valid = json!({
            "cap":{"orgId":"01h000000000001"},
            "loom":{
                "userId":"01h000000000101",
                "orgId":"01h000000000001",
                "video":{
                    "id":"loom-video-1",
                    "name":"",
                    "downloadUrl":"https://cdn.loom.com/video?token=signed",
                    "width":1920,
                    "height":1080,
                    "fps":59.94,
                    "durationSecs":300.5
                }
            }
        });
        assert!(
            validate_legacy_protected_integration_envelope(&sealed_request(
                "cap-v1-f0a00e93ab606a52",
                valid.clone(),
                LegacyProtectedIntegrationPrincipalV1 {
                    tenant_id: Some("00000000-0000-7000-8000-000000000010".into()),
                    ..session_or_api_key()
                },
                None,
            ))
            .is_ok()
        );
        let mut mismatched = valid.clone();
        mismatched["loom"]["orgId"] = json!("01h000000000011");
        assert_eq!(
            validate_legacy_protected_integration_envelope(&sealed_request(
                "cap-v1-f0a00e93ab606a52",
                mismatched,
                session_or_api_key(),
                None,
            )),
            Err(LegacyProtectedIntegrationValidationErrorV1::InvalidPayload)
        );

        let b9_payload = json!({
            "videoId":"01h000000000004",
            "userId":"01h000000000101",
            "rawFileKey":"01h000000000101/01h000000000004/raw-upload.mp4",
            "bucketId":null,
            "loomDownloadUrl":"",
            "loomVideoId":"loom-video-1",
        });
        let same_parent = sealed_request(
            "cap-v1-b9fcb0fbd25b2234",
            b9_payload,
            workflow_parent(),
            None,
        );
        let first = validate_legacy_protected_integration_envelope(&same_parent)
            .expect("first workflow launch");
        let mut same_parent_retry = same_parent.clone();
        same_parent_retry.request_nonce = "00000000-0000-7000-8000-000000000100".into();
        let same_parent_retry = validate_legacy_protected_integration_envelope(&same_parent_retry)
            .expect("same parent retry");
        assert_eq!(first.replay_key_digest, same_parent_retry.replay_key_digest);
        let mut new_parent_retry = same_parent;
        new_parent_retry.parent_receipt_id = Some("00000000-0000-7000-8000-000000000101".into());
        let new_parent_retry = validate_legacy_protected_integration_envelope(&new_parent_retry)
            .expect("new retry parent");
        assert_ne!(first.replay_key_digest, new_parent_retry.replay_key_digest);
    }

    fn public_principal_for_test(
        class: LegacyProtectedIntegrationAuthV1,
    ) -> LegacyProtectedIntegrationPrincipalV1 {
        LegacyProtectedIntegrationPrincipalV1 {
            class,
            actor_id: None,
            tenant_id: None,
            credential_kind: LegacyProtectedIntegrationCredentialKindV1::None,
            credential_subject_id: None,
            credential_key_version: None,
            credential_digest: None,
            credential_expires_at_ms: None,
            policy_proofs: Vec::new(),
            inherited_entitlement_binding: None,
        }
    }
}
