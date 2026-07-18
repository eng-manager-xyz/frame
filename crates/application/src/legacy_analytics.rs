//! Source-pinned contracts for Cap's retained analytics surfaces.
//!
//! Six operations cross the Tinybird/email/notification provider boundary and
//! deliberately return a pending outcome until a real provider executor proves
//! completion. The signup marker is a provider-free seven-day D1 compare-and-
//! swap and is the only operation in this family eligible for exact local serve.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{IdempotencyKey, OrganizationOperationId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const LEGACY_ANALYTICS_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";

pub const LEGACY_ANALYTICS_VIDEO_COUNT_OPERATION_ID: &str = "cap-v1-c8a43dc80c502b6d";
pub const LEGACY_ANALYTICS_TRACK_OPERATION_ID: &str = "cap-v1-51dc2aa9f19a48cc";
pub const LEGACY_ANALYTICS_DASHBOARD_OPERATION_ID: &str = "cap-v1-9b093898957efebb";
pub const LEGACY_ANALYTICS_VIDEO_HTTP_OPERATION_ID: &str = "cap-v1-be2ea6b474aae7c9";
pub const LEGACY_ANALYTICS_VIDEO_RPC_OPERATION_ID: &str = "cap-v1-7c47f9a2a9a24ac0";
pub const LEGACY_ANALYTICS_SIGNUP_OPERATION_ID: &str = "cap-v1-dd88ded400188c1e";
pub const LEGACY_ANALYTICS_VIDEO_ACTION_OPERATION_ID: &str = "cap-v1-9186738740a1ece1";

pub const LEGACY_ANALYTICS_VIDEO_COUNT_PATH: &str = "/api/analytics";
pub const LEGACY_ANALYTICS_TRACK_PATH: &str = "/api/analytics/track";
pub const LEGACY_ANALYTICS_DASHBOARD_PATH: &str = "/api/dashboard/analytics";
pub const LEGACY_ANALYTICS_VIDEO_HTTP_PATH: &str = "/api/video/analytics";
pub const LEGACY_ANALYTICS_VIDEO_RPC_PATH: &str = "/api/erpc#VideosGetAnalytics";
pub const LEGACY_ANALYTICS_SIGNUP_PATH: &str =
    "action://apps/web/actions/analytics/track-user-signed-up.ts#checkAndMarkUserSignedUpTracked";
pub const LEGACY_ANALYTICS_VIDEO_ACTION_PATH: &str =
    "action://apps/web/actions/videos/get-analytics.ts#getVideoAnalytics";

pub const LEGACY_ANALYTICS_MAX_BODY_BYTES: usize = 256 * 1_024;
pub const LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS: u16 = 90;
pub const LEGACY_ANALYTICS_MAX_RANGE_DAYS: u16 = 90;
pub const LEGACY_ANALYTICS_SIGNUP_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
pub const LEGACY_ANALYTICS_VIEW_DELAY_MS: i64 = 2 * 60 * 1_000;
pub const LEGACY_ANALYTICS_ANON_NOTIFICATION_CUTOFF_MS: i64 = 1_772_582_400_000;
pub const LEGACY_ANALYTICS_ANON_NOTIFICATION_WINDOW_MS: i64 = 5 * 60 * 1_000;
pub const LEGACY_ANALYTICS_ANON_NOTIFICATION_LIMIT: u16 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyAnalyticsSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
}

const TINYBIRD: LegacyAnalyticsSourcePinV1 = LegacyAnalyticsSourcePinV1 {
    path: "packages/web-backend/src/Tinybird/index.ts",
    symbol: "Tinybird.appendEvents+querySql+response normalization",
    sha256: "d92c0740b6f04e2c455742e24799f1923661984447e7b2e1a3d9a3cc5c723f41",
};
const VIDEO_ACTION: LegacyAnalyticsSourcePinV1 = LegacyAnalyticsSourcePinV1 {
    path: "apps/web/actions/videos/get-analytics.ts",
    symbol: "getVideoAnalytics+range normalization+aggregate/raw fallback",
    sha256: "3663ca3504f6450ac3ad05054bd7c178e04f00b2f73a40c2e9d72aef8ae2a415",
};
const VIDEO_POLICY: LegacyAnalyticsSourcePinV1 = LegacyAnalyticsSourcePinV1 {
    path: "packages/web-backend/src/Videos/VideosPolicy.ts",
    symbol: "buildCanView+optional-auth public/password policy",
    sha256: "39e4b55f59e0758450d76401706cb2d258c8fe850fef91f395662df9146f7540",
};
const VIDEO_REPO: LegacyAnalyticsSourcePinV1 = LegacyAnalyticsSourcePinV1 {
    path: "packages/web-backend/src/Videos/VideosRepo.ts",
    symbol: "VideosRepo.getById",
    sha256: "9d444fe29cb6f22e033da1e16757e3bde2f523f22f812eeba87cca05c56d63b1",
};
const DATABASE_SCHEMA: LegacyAnalyticsSourcePinV1 = LegacyAnalyticsSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "users+videos+notifications+videoUploads",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
};

pub const LEGACY_ANALYTICS_VIDEO_COUNT_SOURCES: &[LegacyAnalyticsSourcePinV1] = &[
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/api/analytics/route.ts",
        symbol: "GET+parseRangeParam+canView non-disclosure",
        sha256: "6b50a1bc796d2fc9405e90fa7a20b04067bb5e400796a9e7edef2522c5b633f9",
    },
    VIDEO_ACTION,
    TINYBIRD,
    VIDEO_POLICY,
    VIDEO_REPO,
];

pub const LEGACY_ANALYTICS_TRACK_SOURCES: &[LegacyAnalyticsSourcePinV1] = &[
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/api/analytics/track/route.ts",
        symbol: "POST+normalization+view suppression+effects",
        sha256: "92979446f291dcdfe09d6ac660252ac8769b6823ca3a89e43f6fe4ef02ff0680",
    },
    TINYBIRD,
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/lib/Notification.ts",
        symbol: "createAnonymousViewNotification+sendFirstViewEmail",
        sha256: "5678bf28d261c9be6854a5a67827d090998bbfefa7cc4fb5cdf18a739fb4993e",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/lib/anonymous-names.ts",
        symbol: "getAnonymousName+getSessionHash",
        sha256: "3f8a61df017b155809d86e8442baee45014d3e7a82130ad3a02f3acd1e261209",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/s/[videoId]/Share.tsx",
        symbol: "ensureAnalyticsSessionId+trackVideoView",
        sha256: "79586b9e1b39c6fd91d471d11e56b722139bcc9bf33f6d95708cca1028e85baa",
    },
    DATABASE_SCHEMA,
];

pub const LEGACY_ANALYTICS_DASHBOARD_SOURCES: &[LegacyAnalyticsSourcePinV1] = &[
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/api/dashboard/analytics/route.ts",
        symbol: "GET+active organization+range fallback",
        sha256: "80ab94d8b16eb01377d2bdad94fb71f3d628be562c321472bcf519efdf4e2c02",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/analytics/data.ts",
        symbol: "getOrgAnalyticsData+normalizers+Tinybird query family",
        sha256: "a197d55c8553045383e897d3613cd562ee1998896ed3811deae72a5f99f826bf",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/analytics/types.ts",
        symbol: "AnalyticsRange+OrgAnalyticsResponse",
        sha256: "9ee52909c63b75a069049d0bdc70081a7fb54513684ba0a488b68198d2284ded",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/(org)/dashboard/analytics/components/AnalyticsDashboard.tsx",
        symbol: "dashboard analytics caller/query serialization",
        sha256: "74567e59e0a78e37c3fd2ab0e0d08ffcf1fa1aed7551a2865fb048e1592da6d0",
    },
    TINYBIRD,
];

pub const LEGACY_ANALYTICS_VIDEO_HTTP_SOURCES: &[LegacyAnalyticsSourcePinV1] = &[
    VIDEO_ACTION,
    LegacyAnalyticsSourcePinV1 {
        path: "packages/web-api-contract-effect/src/index.ts",
        symbol: "getAnalytics",
        sha256: "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "packages/web-api-contract/src/index.ts",
        symbol: "GET /video/analytics",
        sha256: "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
    },
    TINYBIRD,
    VIDEO_POLICY,
];

pub const LEGACY_ANALYTICS_VIDEO_RPC_SOURCES: &[LegacyAnalyticsSourcePinV1] = &[
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/api/erpc/route.ts",
        symbol: "Effect RPC HTTP transport",
        sha256: "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "packages/web-backend/src/Rpcs.ts",
        symbol: "RpcsLive+RpcAuthMiddlewareLive",
        sha256: "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "packages/web-backend/src/Videos/VideosRpcs.ts",
        symbol: "VideosGetAnalytics",
        sha256: "6edf9add90a28c542fb53c9a7bfa858bc89290e2a0fbeec827210bd5af189623",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "packages/web-backend/src/Videos/index.ts",
        symbol: "Videos.getAnalyticsBulk",
        sha256: "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        symbol: "VideosGetAnalytics schema",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/lib/Requests/AnalyticsRequest.ts",
        symbol: "AnalyticsRequest.DataLoaderResolver",
        sha256: "4523b0bdd68cba229cea9cf6a8475603aaa2e9cdcb0c97e725b30c14d55cb38c",
    },
    TINYBIRD,
    VIDEO_POLICY,
];

pub const LEGACY_ANALYTICS_SIGNUP_SOURCES: &[LegacyAnalyticsSourcePinV1] = &[
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/actions/analytics/track-user-signed-up.ts",
        symbol: "checkAndMarkUserSignedUpTracked",
        sha256: "f73188fc0f3e91f34c37c82a4a18fc7fd11cf3a17554af6f3a5db82512f6d0de",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/Layout/PosthogIdentify.tsx",
        symbol: "PosthogIdentify caller",
        sha256: "7577e3cb7ff8bced61cd7e3f0ea4948b36bee2aaa75b6e01571b3f4f6dc68e95",
    },
    DATABASE_SCHEMA,
];

pub const LEGACY_ANALYTICS_VIDEO_ACTION_SOURCES: &[LegacyAnalyticsSourcePinV1] = &[
    VIDEO_ACTION,
    TINYBIRD,
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/s/[videoId]/_components/tabs/Activity/Analytics.tsx",
        symbol: "getVideoAnalytics caller",
        sha256: "d2eaef0c0d094ba4f3aac5f8fbd27d213f4f170217e0db12947e7dc947757c65",
    },
    LegacyAnalyticsSourcePinV1 {
        path: "apps/web/app/s/[videoId]/page.tsx",
        symbol: "viewsPromise caller",
        sha256: "b9c6e5d777ed424edd14c8840c02cf66bab3f8f33060efdef739de59e7e4d673",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyAnalyticsSurfaceV1 {
    VideoCount,
    Track,
    Dashboard,
    VideoHttp,
    VideoRpc,
    Signup,
    VideoAction,
}

impl LegacyAnalyticsSurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::VideoCount => LEGACY_ANALYTICS_VIDEO_COUNT_OPERATION_ID,
            Self::Track => LEGACY_ANALYTICS_TRACK_OPERATION_ID,
            Self::Dashboard => LEGACY_ANALYTICS_DASHBOARD_OPERATION_ID,
            Self::VideoHttp => LEGACY_ANALYTICS_VIDEO_HTTP_OPERATION_ID,
            Self::VideoRpc => LEGACY_ANALYTICS_VIDEO_RPC_OPERATION_ID,
            Self::Signup => LEGACY_ANALYTICS_SIGNUP_OPERATION_ID,
            Self::VideoAction => LEGACY_ANALYTICS_VIDEO_ACTION_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::VideoCount => LEGACY_ANALYTICS_VIDEO_COUNT_PATH,
            Self::Track => LEGACY_ANALYTICS_TRACK_PATH,
            Self::Dashboard => LEGACY_ANALYTICS_DASHBOARD_PATH,
            Self::VideoHttp => LEGACY_ANALYTICS_VIDEO_HTTP_PATH,
            Self::VideoRpc => LEGACY_ANALYTICS_VIDEO_RPC_PATH,
            Self::Signup => LEGACY_ANALYTICS_SIGNUP_PATH,
            Self::VideoAction => LEGACY_ANALYTICS_VIDEO_ACTION_PATH,
        }
    }

    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::VideoCount | Self::Dashboard | Self::VideoHttp => "GET",
            Self::Track => "POST",
            Self::VideoRpc => "RPC",
            Self::Signup | Self::VideoAction => "ACTION",
        }
    }

    #[must_use]
    pub const fn provider_execution_required(self) -> bool {
        !matches!(self, Self::Signup)
    }

    #[must_use]
    pub const fn mutation(self) -> bool {
        matches!(self, Self::Track | Self::Signup)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegacyAnalyticsDashboardRangeV1 {
    Hours24,
    Days7,
    Days30,
    Lifetime,
}

impl LegacyAnalyticsDashboardRangeV1 {
    #[must_use]
    pub const fn source_value(self) -> &'static str {
        match self {
            Self::Hours24 => "24h",
            Self::Days7 => "7d",
            Self::Days30 => "30d",
            Self::Lifetime => "lifetime",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyAnalyticsNetworkV1 {
    pub user_agent: Option<String>,
    pub request_hostname: Option<String>,
    pub country: Option<String>,
    pub region: Option<String>,
    pub encoded_city: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyAnalyticsTrackInputV1 {
    pub video_id: String,
    pub organization_id: Option<String>,
    pub owner_id: Option<String>,
    pub session_id: Option<String>,
    pub pathname: Option<String>,
    pub hostname: Option<String>,
    pub body_user_agent: Option<String>,
    /// This is the canonical ISO string produced by JavaScript `Date#toISOString`.
    pub occurred_at_iso: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyAnalyticsInputV1 {
    VideoCount {
        video_id: String,
        range: Option<String>,
    },
    Track(LegacyAnalyticsTrackInputV1),
    Dashboard {
        requested_organization_id: Option<String>,
        space_id: Option<String>,
        video_id: Option<String>,
        range: Option<String>,
    },
    VideoHttp {
        video_id: String,
    },
    VideoRpc {
        video_ids: Vec<String>,
    },
    Signup,
    VideoAction {
        video_id: String,
        range_days: Option<f64>,
    },
}

impl LegacyAnalyticsInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyAnalyticsSurfaceV1 {
        match self {
            Self::VideoCount { .. } => LegacyAnalyticsSurfaceV1::VideoCount,
            Self::Track(_) => LegacyAnalyticsSurfaceV1::Track,
            Self::Dashboard { .. } => LegacyAnalyticsSurfaceV1::Dashboard,
            Self::VideoHttp { .. } => LegacyAnalyticsSurfaceV1::VideoHttp,
            Self::VideoRpc { .. } => LegacyAnalyticsSurfaceV1::VideoRpc,
            Self::Signup => LegacyAnalyticsSurfaceV1::Signup,
            Self::VideoAction { .. } => LegacyAnalyticsSurfaceV1::VideoAction,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyAnalyticsRequestV1 {
    pub actor_id: Option<String>,
    pub active_organization_id: Option<String>,
    pub password_grant_digest: Option<String>,
    pub idempotency_key: Option<String>,
    pub now_ms: i64,
    pub now_iso: String,
    pub network: LegacyAnalyticsNetworkV1,
    pub input: LegacyAnalyticsInputV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyAnalyticsUserAgentV1 {
    pub raw: String,
    pub browser: String,
    pub operating_system: String,
    pub device: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyAnalyticsTrackEventV1 {
    pub timestamp: String,
    pub session_id: Option<String>,
    pub video_id: String,
    pub requested_organization_id: Option<String>,
    pub requested_owner_id: Option<String>,
    pub pathname: String,
    pub hostname: String,
    pub country: String,
    pub region: String,
    pub city: String,
    pub user_agent: LegacyAnalyticsUserAgentV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyAnalyticsCommandInputV1 {
    VideoCount {
        video_id: String,
        range_days: u16,
    },
    Track(Box<LegacyAnalyticsTrackEventV1>),
    Dashboard {
        requested_organization_id: Option<String>,
        space_id: Option<String>,
        video_id: Option<String>,
        range: LegacyAnalyticsDashboardRangeV1,
    },
    VideoHttp {
        video_id: String,
        range_days: u16,
    },
    VideoRpc {
        video_ids: Vec<String>,
        range_days: u16,
    },
    Signup,
    VideoAction {
        video_id: String,
        range_days: u16,
    },
}

impl LegacyAnalyticsCommandInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyAnalyticsSurfaceV1 {
        match self {
            Self::VideoCount { .. } => LegacyAnalyticsSurfaceV1::VideoCount,
            Self::Track(_) => LegacyAnalyticsSurfaceV1::Track,
            Self::Dashboard { .. } => LegacyAnalyticsSurfaceV1::Dashboard,
            Self::VideoHttp { .. } => LegacyAnalyticsSurfaceV1::VideoHttp,
            Self::VideoRpc { .. } => LegacyAnalyticsSurfaceV1::VideoRpc,
            Self::Signup => LegacyAnalyticsSurfaceV1::Signup,
            Self::VideoAction { .. } => LegacyAnalyticsSurfaceV1::VideoAction,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct LegacyAnalyticsCommandV1 {
    operation_id: Option<OrganizationOperationId>,
    actor_id: Option<String>,
    active_organization_id: Option<String>,
    password_grant_digest: Option<String>,
    idempotency_key: Option<IdempotencyKey>,
    principal_digest: String,
    request_digest: String,
    now_ms: i64,
    input: LegacyAnalyticsCommandInputV1,
}

impl fmt::Debug for LegacyAnalyticsCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyAnalyticsCommandV1")
            .field("operation_id", &self.operation_id)
            .field("surface", &self.input.surface())
            .field("actor_present", &self.actor_id.is_some())
            .field(
                "active_organization_present",
                &self.active_organization_id.is_some(),
            )
            .field(
                "password_grant_present",
                &self.password_grant_digest.is_some(),
            )
            .field("request_digest", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl LegacyAnalyticsCommandV1 {
    #[must_use]
    pub const fn operation_id(&self) -> Option<OrganizationOperationId> {
        self.operation_id
    }
    #[must_use]
    pub fn actor_id(&self) -> Option<&str> {
        self.actor_id.as_deref()
    }
    #[must_use]
    pub fn active_organization_id(&self) -> Option<&str> {
        self.active_organization_id.as_deref()
    }
    #[must_use]
    pub fn password_grant_digest(&self) -> Option<&str> {
        self.password_grant_digest.as_deref()
    }
    #[must_use]
    pub fn principal_digest(&self) -> &str {
        &self.principal_digest
    }
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }
    #[must_use]
    pub const fn now_ms(&self) -> i64 {
        self.now_ms
    }
    #[must_use]
    pub fn input(&self) -> &LegacyAnalyticsCommandInputV1 {
        &self.input
    }
    #[must_use]
    pub fn execution_key_digest(&self) -> Option<String> {
        self.operation_id.map(|operation_id| {
            let value = self
                .idempotency_key
                .as_ref()
                .map_or_else(|| operation_id.to_string(), |key| key.expose().to_owned());
            digest_fields(
                b"frame.legacy-analytics.execution-key.v1\0",
                &[self.input.surface().operation_id(), &value],
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyAnalyticsResultV1 {
    ProviderPending { operation_id: String },
    TrackSkipped,
    SignupTracking { should_track: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyAnalyticsOutcomeV1 {
    pub result: LegacyAnalyticsResultV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyAnalyticsPortErrorV1 {
    NotFound,
    Unauthorized,
    Forbidden,
    Conflict,
    ProviderRequired,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyAnalyticsErrorV1 {
    #[error("invalid analytics request")]
    InvalidInput,
    #[error("video not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("idempotency conflict")]
    Conflict,
    #[error("analytics provider execution required")]
    ProviderRequired,
    #[error("analytics authority unavailable")]
    Unavailable,
    #[error("analytics authority corrupt")]
    Internal,
}

#[async_trait(?Send)]
pub trait LegacyAnalyticsPortV1 {
    async fn execute(
        &self,
        command: LegacyAnalyticsCommandV1,
    ) -> Result<LegacyAnalyticsOutcomeV1, LegacyAnalyticsPortErrorV1>;
}

pub struct LegacyAnalyticsAdapterV1<'a, Port> {
    port: &'a Port,
}

impl<'a, Port: LegacyAnalyticsPortV1> LegacyAnalyticsAdapterV1<'a, Port> {
    #[must_use]
    pub const fn new(port: &'a Port) -> Self {
        Self { port }
    }

    pub async fn execute(
        &self,
        request: LegacyAnalyticsRequestV1,
    ) -> Result<LegacyAnalyticsOutcomeV1, LegacyAnalyticsErrorV1> {
        self.port
            .execute(prepare(request)?)
            .await
            .map_err(map_port_error)
    }
}

fn prepare(
    request: LegacyAnalyticsRequestV1,
) -> Result<LegacyAnalyticsCommandV1, LegacyAnalyticsErrorV1> {
    if !(0..=9_007_199_254_740_991).contains(&request.now_ms)
        || !canonical_iso(&request.now_iso)
        || request
            .actor_id
            .as_deref()
            .is_some_and(|value| !valid_identifier(value))
        || request
            .active_organization_id
            .as_deref()
            .is_some_and(|value| !valid_identifier(value))
        || request
            .password_grant_digest
            .as_deref()
            .is_some_and(|value| {
                value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    {
        return Err(LegacyAnalyticsErrorV1::InvalidInput);
    }

    let surface = request.input.surface();
    if !surface.mutation() && request.idempotency_key.is_some() {
        return Err(LegacyAnalyticsErrorV1::InvalidInput);
    }
    let operation_id = surface
        .provider_execution_required()
        .then(OrganizationOperationId::new);
    let idempotency_key = request
        .idempotency_key
        .map(IdempotencyKey::parse)
        .transpose()
        .map_err(|_| LegacyAnalyticsErrorV1::InvalidInput)?;

    let input = normalize_input(request.input, &request.network, &request.now_iso)?;
    let encoded = serde_json::to_string(&input).map_err(|_| LegacyAnalyticsErrorV1::Internal)?;
    let actor = request.actor_id.as_deref().unwrap_or("anonymous");
    let principal_digest = digest_fields(b"frame.legacy-analytics.principal.v1\0", &[actor]);
    let request_digest = digest_fields(
        b"frame.legacy-analytics.request.v1\0",
        &[surface.operation_id(), actor, &encoded],
    );
    Ok(LegacyAnalyticsCommandV1 {
        operation_id,
        actor_id: request.actor_id,
        active_organization_id: request.active_organization_id,
        password_grant_digest: request
            .password_grant_digest
            .map(|value| value.to_ascii_lowercase()),
        idempotency_key,
        principal_digest,
        request_digest,
        now_ms: request.now_ms,
        input,
    })
}

fn normalize_input(
    input: LegacyAnalyticsInputV1,
    network: &LegacyAnalyticsNetworkV1,
    now_iso: &str,
) -> Result<LegacyAnalyticsCommandInputV1, LegacyAnalyticsErrorV1> {
    Ok(match input {
        LegacyAnalyticsInputV1::VideoCount { video_id, range } => {
            require_identifier(&video_id)?;
            LegacyAnalyticsCommandInputV1::VideoCount {
                video_id,
                range_days: normalize_route_range(range.as_deref()),
            }
        }
        LegacyAnalyticsInputV1::Track(input) => {
            require_identifier(&input.video_id)?;
            let organization_id = input.organization_id.filter(|value| !value.is_empty());
            let owner_id = input.owner_id.filter(|value| !value.is_empty());
            for value in [organization_id.as_deref(), owner_id.as_deref()] {
                if value.is_some_and(|value| !valid_identifier(value)) {
                    return Err(LegacyAnalyticsErrorV1::InvalidInput);
                }
            }
            let timestamp = input.occurred_at_iso.unwrap_or_else(|| now_iso.to_owned());
            if !canonical_iso(&timestamp) {
                return Err(LegacyAnalyticsErrorV1::InvalidInput);
            }
            let raw_user_agent = sanitize_string(network.user_agent.as_deref())
                .or_else(|| sanitize_string(input.body_user_agent.as_deref()))
                .unwrap_or_else(|| "unknown".into());
            let hostname = sanitize_string(input.hostname.as_deref())
                .or_else(|| sanitize_string(network.request_hostname.as_deref()))
                .unwrap_or_default();
            let pathname = input
                .pathname
                .unwrap_or_else(|| format!("/s/{}", input.video_id));
            if pathname.len() > 8_192 || pathname.chars().any(char::is_control) {
                return Err(LegacyAnalyticsErrorV1::InvalidInput);
            }
            LegacyAnalyticsCommandInputV1::Track(Box::new(LegacyAnalyticsTrackEventV1 {
                timestamp,
                session_id: normalize_session_id(input.session_id.as_deref()),
                video_id: input.video_id,
                requested_organization_id: organization_id,
                requested_owner_id: owner_id,
                pathname,
                hostname,
                country: sanitize_string(network.country.as_deref()).unwrap_or_default(),
                region: sanitize_string(network.region.as_deref()).unwrap_or_default(),
                city: sanitize_string(
                    network
                        .encoded_city
                        .as_deref()
                        .map(decode_uri_component_or_original)
                        .as_deref(),
                )
                .unwrap_or_default(),
                user_agent: classify_user_agent(&raw_user_agent),
            }))
        }
        LegacyAnalyticsInputV1::Dashboard {
            requested_organization_id,
            space_id,
            video_id,
            range,
        } => {
            for value in [
                requested_organization_id.as_deref(),
                space_id.as_deref().filter(|value| !value.is_empty()),
                video_id.as_deref().filter(|value| !value.is_empty()),
            ] {
                if value.is_some_and(|value| !valid_identifier(value)) {
                    return Err(LegacyAnalyticsErrorV1::InvalidInput);
                }
            }
            LegacyAnalyticsCommandInputV1::Dashboard {
                requested_organization_id,
                space_id: space_id.filter(|value| !value.is_empty()),
                video_id: video_id.filter(|value| !value.is_empty()),
                range: normalize_dashboard_range(range.as_deref()),
            }
        }
        LegacyAnalyticsInputV1::VideoHttp { video_id } => {
            require_identifier(&video_id)?;
            LegacyAnalyticsCommandInputV1::VideoHttp {
                video_id,
                range_days: LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS,
            }
        }
        LegacyAnalyticsInputV1::VideoRpc { video_ids } => {
            if video_ids.len() > 50 || video_ids.iter().any(|value| !valid_identifier(value)) {
                return Err(LegacyAnalyticsErrorV1::InvalidInput);
            }
            LegacyAnalyticsCommandInputV1::VideoRpc {
                video_ids,
                range_days: LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS,
            }
        }
        LegacyAnalyticsInputV1::Signup => LegacyAnalyticsCommandInputV1::Signup,
        LegacyAnalyticsInputV1::VideoAction {
            video_id,
            range_days,
        } => {
            require_identifier(&video_id)?;
            LegacyAnalyticsCommandInputV1::VideoAction {
                video_id,
                range_days: normalize_numeric_range(range_days),
            }
        }
    })
}

#[must_use]
pub fn normalize_route_range(value: Option<&str>) -> u16 {
    let Some(mut value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS;
    };
    if value.ends_with(['d', 'D']) {
        value = &value[..value.len() - 1];
    }
    let bytes = value.as_bytes();
    let mut index = usize::from(
        bytes
            .first()
            .is_some_and(|byte| matches!(byte, b'+' | b'-')),
    );
    let start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == start {
        return LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS;
    }
    let parsed = value[..index].parse::<i128>().ok();
    match parsed {
        Some(value) if value > 0 => {
            value.clamp(1, i128::from(LEGACY_ANALYTICS_MAX_RANGE_DAYS)) as u16
        }
        _ => LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS,
    }
}

#[must_use]
pub fn normalize_numeric_range(value: Option<f64>) -> u16 {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS;
    };
    let value = value.floor();
    if value <= 0.0 {
        LEGACY_ANALYTICS_DEFAULT_RANGE_DAYS
    } else {
        value.min(f64::from(LEGACY_ANALYTICS_MAX_RANGE_DAYS)) as u16
    }
}

#[must_use]
pub fn normalize_dashboard_range(value: Option<&str>) -> LegacyAnalyticsDashboardRangeV1 {
    match value {
        Some("24h") => LegacyAnalyticsDashboardRangeV1::Hours24,
        Some("30d") => LegacyAnalyticsDashboardRangeV1::Days30,
        Some("lifetime") => LegacyAnalyticsDashboardRangeV1::Lifetime,
        _ => LegacyAnalyticsDashboardRangeV1::Days7,
    }
}

#[must_use]
pub fn sanitize_string(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || value == "unknown" {
        None
    } else {
        Some(truncate_utf16(value, 256))
    }
}

#[must_use]
pub fn normalize_session_id(value: Option<&str>) -> Option<String> {
    let value = truncate_utf16(value?.trim(), 128);
    (!value.is_empty() && value != "anonymous").then_some(value)
}

fn truncate_utf16(value: &str, limit: usize) -> String {
    let mut units = 0;
    value
        .chars()
        .take_while(|character| {
            let next = units + character.len_utf16();
            if next > limit {
                false
            } else {
                units = next;
                true
            }
        })
        .collect()
}

#[must_use]
pub fn decode_uri_component_or_original(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(high) = bytes.get(index + 1).and_then(|byte| hex_nibble(*byte)) else {
                return value.to_owned();
            };
            let Some(low) = bytes.get(index + 2).and_then(|byte| hex_nibble(*byte)) else {
                return value.to_owned();
            };
            output.push((high << 4) | low);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).unwrap_or_else(|_| value.to_owned())
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[must_use]
pub fn classify_user_agent(value: &str) -> LegacyAnalyticsUserAgentV1 {
    let lower = value.to_ascii_lowercase();
    let browser = if lower.contains("edg/") {
        "Edge"
    } else if lower.contains("opr/") || lower.contains("opera") {
        "Opera"
    } else if lower.contains("firefox/") || lower.contains("fxios/") {
        "Firefox"
    } else if lower.contains("chrome/") || lower.contains("crios/") {
        "Chrome"
    } else if lower.contains("safari/") && lower.contains("version/") {
        "Safari"
    } else {
        "unknown"
    };
    let operating_system = if lower.contains("windows nt") {
        "Windows"
    } else if lower.contains("android") {
        "Android"
    } else if lower.contains("iphone") || lower.contains("ipad") {
        "iOS"
    } else if lower.contains("mac os x") || lower.contains("macintosh") {
        "Mac OS"
    } else if lower.contains("linux") {
        "Linux"
    } else {
        "unknown"
    };
    let device = if lower.contains("ipad") || lower.contains("tablet") {
        "tablet"
    } else if lower.contains("mobile") || lower.contains("iphone") {
        "mobile"
    } else {
        "desktop"
    };
    LegacyAnalyticsUserAgentV1 {
        raw: value.to_owned(),
        browser: browser.into(),
        operating_system: operating_system.into(),
        device: device.into(),
    }
}

#[must_use]
pub fn escape_clickhouse_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "''")
}

const ANIMALS: [&str; 32] = [
    "Walrus",
    "Capybara",
    "Narwhal",
    "Quokka",
    "Axolotl",
    "Pangolin",
    "Okapi",
    "Platypus",
    "Wombat",
    "Chinchilla",
    "Manatee",
    "Flamingo",
    "Hedgehog",
    "Otter",
    "Puffin",
    "Raccoon",
    "Sloth",
    "Chameleon",
    "Penguin",
    "Koala",
    "Red Panda",
    "Seahorse",
    "Toucan",
    "Lemur",
    "Armadillo",
    "Alpaca",
    "Meerkat",
    "Ibex",
    "Tapir",
    "Kiwi",
    "Gecko",
    "Bison",
];

#[must_use]
pub fn analytics_session_digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

#[must_use]
pub fn anonymous_viewer_name(session_id: &str) -> String {
    let digest = Sha256::digest(session_id.as_bytes());
    let index = usize::from(digest[3] & 0x1f);
    format!("Anonymous {}", ANIMALS[index])
}

#[must_use]
pub fn normalize_os_name(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "Unknown".into();
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains("mac") || matches!(lower.as_str(), "macos" | "mac os" | "darwin") {
        "macOS".into()
    } else if lower.contains("windows") || lower == "win" {
        "Windows".into()
    } else if lower.contains("linux") {
        "Linux".into()
    } else if lower.contains("android") {
        "Android".into()
    } else if lower.contains("ios") || lower == "iphone os" {
        "iOS".into()
    } else if lower.contains("ubuntu") {
        "Ubuntu".into()
    } else if lower.contains("fedora") {
        "Fedora".into()
    } else {
        value.into()
    }
}

#[must_use]
pub fn normalize_device_name(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "Unknown".into();
    }
    match value.to_ascii_lowercase().as_str() {
        "desktop" | "desktop computer" | "pc" => "Desktop".into(),
        "mobile" | "smartphone" | "phone" => "Mobile".into(),
        "tablet" | "ipad" => "Tablet".into(),
        _ => value.into(),
    }
}

#[must_use]
pub fn normalize_browser_name(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "Unknown".into();
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains("chrome") && !lower.contains("chromium") {
        "Chrome".into()
    } else if lower.contains("firefox") {
        "Firefox".into()
    } else if lower.contains("safari") {
        "Safari".into()
    } else if lower.contains("edge") {
        "Edge".into()
    } else if lower.contains("opera") {
        "Opera".into()
    } else if lower.contains("brave") {
        "Brave".into()
    } else if lower.contains("internet explorer") || lower == "ie" {
        "Internet Explorer".into()
    } else {
        value.into()
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && !value.chars().any(char::is_control)
}

fn require_identifier(value: &str) -> Result<(), LegacyAnalyticsErrorV1> {
    valid_identifier(value)
        .then_some(())
        .ok_or(LegacyAnalyticsErrorV1::InvalidInput)
}

fn canonical_iso(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 20 | 24)
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(10) == Some(&b'T')
        && bytes.get(13) == Some(&b':')
        && bytes.get(16) == Some(&b':')
        && bytes.last() == Some(&b'Z')
        && (bytes.len() == 20 || bytes.get(19) == Some(&b'.'))
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19 | 23) || byte.is_ascii_digit()
        })
}

fn digest_fields(prefix: &[u8], fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(prefix);
    for field in fields {
        digest.update(field.as_bytes());
        digest.update(b"\0");
    }
    format!("{:x}", digest.finalize())
}

fn map_port_error(error: LegacyAnalyticsPortErrorV1) -> LegacyAnalyticsErrorV1 {
    match error {
        LegacyAnalyticsPortErrorV1::NotFound => LegacyAnalyticsErrorV1::NotFound,
        LegacyAnalyticsPortErrorV1::Unauthorized => LegacyAnalyticsErrorV1::Unauthorized,
        LegacyAnalyticsPortErrorV1::Forbidden => LegacyAnalyticsErrorV1::Forbidden,
        LegacyAnalyticsPortErrorV1::Conflict => LegacyAnalyticsErrorV1::Conflict,
        LegacyAnalyticsPortErrorV1::ProviderRequired => LegacyAnalyticsErrorV1::ProviderRequired,
        LegacyAnalyticsPortErrorV1::Unavailable => LegacyAnalyticsErrorV1::Unavailable,
        LegacyAnalyticsPortErrorV1::Corrupt => LegacyAnalyticsErrorV1::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_range_normalization_matches_parse_int_floor_and_clamps() {
        assert_eq!(normalize_route_range(None), 90);
        assert_eq!(normalize_route_range(Some(" 7D ")), 7);
        assert_eq!(normalize_route_range(Some("12days")), 12);
        assert_eq!(normalize_route_range(Some("1.9d")), 1);
        assert_eq!(normalize_route_range(Some("0d")), 90);
        assert_eq!(normalize_route_range(Some("999d")), 90);
        assert_eq!(normalize_numeric_range(Some(7.9)), 7);
        assert_eq!(normalize_numeric_range(Some(f64::NAN)), 90);
        assert_eq!(normalize_numeric_range(Some(-1.0)), 90);
    }

    #[test]
    fn tracking_strings_preserve_source_trim_unknown_and_utf16_limits() {
        assert_eq!(sanitize_string(Some(" unknown ")), None);
        assert_eq!(normalize_session_id(Some(" anonymous ")), None);
        assert_eq!(
            normalize_session_id(Some(" session ")).as_deref(),
            Some("session")
        );
        let emoji = "😀".repeat(200);
        let Some(sanitized_emoji) = sanitize_string(Some(&emoji)) else {
            panic!("bounded emoji input must remain present");
        };
        assert_eq!(sanitized_emoji.encode_utf16().count(), 256);
        assert_eq!(
            decode_uri_component_or_original("San%20Jos%C3%A9"),
            "San José"
        );
        assert_eq!(decode_uri_component_or_original("bad%2"), "bad%2");

        let Ok(normalized) = normalize_input(
            LegacyAnalyticsInputV1::Track(LegacyAnalyticsTrackInputV1 {
                video_id: "video".into(),
                organization_id: Some(String::new()),
                owner_id: Some(String::new()),
                session_id: None,
                pathname: None,
                hostname: None,
                body_user_agent: None,
                occurred_at_iso: None,
            }),
            &LegacyAnalyticsNetworkV1 {
                user_agent: None,
                request_hostname: None,
                country: None,
                region: None,
                encoded_city: None,
            },
            "2026-03-09T00:00:00.000Z",
        ) else {
            panic!("valid tracking input must normalize");
        };
        let LegacyAnalyticsCommandInputV1::Track(event) = normalized else {
            panic!("track input must remain track input");
        };
        assert_eq!(event.requested_organization_id, None);
        assert_eq!(event.requested_owner_id, None);
    }

    #[test]
    fn anonymous_identity_and_user_agent_classification_are_deterministic() {
        assert_eq!(analytics_session_digest("fixture").len(), 64);
        assert!(anonymous_viewer_name("fixture").starts_with("Anonymous "));
        let chrome = classify_user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/126.0 Safari/537.36",
        );
        assert_eq!(chrome.browser, "Chrome");
        assert_eq!(chrome.operating_system, "Windows");
        assert_eq!(chrome.device, "desktop");
        let iphone = classify_user_agent(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5) Version/17.5 Mobile Safari/604.1",
        );
        assert_eq!(iphone.browser, "Safari");
        assert_eq!(iphone.operating_system, "iOS");
        assert_eq!(iphone.device, "mobile");
    }

    #[test]
    fn dashboard_labels_and_clickhouse_literals_match_source_ordering() {
        assert_eq!(
            normalize_dashboard_range(Some("24h")),
            LegacyAnalyticsDashboardRangeV1::Hours24
        );
        assert_eq!(
            normalize_dashboard_range(Some("bad")),
            LegacyAnalyticsDashboardRangeV1::Days7
        );
        assert_eq!(normalize_os_name("darwin"), "macOS");
        assert_eq!(normalize_device_name("PC"), "Desktop");
        assert_eq!(normalize_browser_name("Google Chrome"), "Chrome");
        assert_eq!(escape_clickhouse_literal("a\\'b"), "a\\\\''b");
    }

    #[test]
    fn provider_gates_and_source_closures_are_explicit() {
        assert!(!LegacyAnalyticsSurfaceV1::Signup.provider_execution_required());
        for surface in [
            LegacyAnalyticsSurfaceV1::VideoCount,
            LegacyAnalyticsSurfaceV1::Track,
            LegacyAnalyticsSurfaceV1::Dashboard,
            LegacyAnalyticsSurfaceV1::VideoHttp,
            LegacyAnalyticsSurfaceV1::VideoRpc,
            LegacyAnalyticsSurfaceV1::VideoAction,
        ] {
            assert!(surface.provider_execution_required());
        }
        assert_eq!(LEGACY_ANALYTICS_VIDEO_COUNT_SOURCES.len(), 5);
        assert_eq!(LEGACY_ANALYTICS_TRACK_SOURCES.len(), 6);
        assert_eq!(LEGACY_ANALYTICS_DASHBOARD_SOURCES.len(), 5);
        assert_eq!(LEGACY_ANALYTICS_VIDEO_HTTP_SOURCES.len(), 5);
        assert_eq!(LEGACY_ANALYTICS_VIDEO_RPC_SOURCES.len(), 8);
        assert_eq!(LEGACY_ANALYTICS_SIGNUP_SOURCES.len(), 3);
        assert_eq!(LEGACY_ANALYTICS_VIDEO_ACTION_SOURCES.len(), 4);
    }
}
