//! Source-pinned contracts for Cap's developer REST/SDK APIs and storage cron.
//!
//! The public SDK uses `cpk_` credentials and, for production applications,
//! requires an exact configured Origin. The REST surface uses `csk_` credentials.
//! The daily cron uses a constant-time Bearer secret comparison. Released clients
//! do not send idempotency headers, so mutation keys are optional at the carrier;
//! supplied keys are durable and absent keys receive a server execution key.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{IdempotencyKey, OrganizationOperationId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const LEGACY_DEVELOPER_API_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";

// Canonical IDs are sha256("route\0METHOD\0PATH")[:16]. Earlier issue text
// carried lookalike aliases; the parity catalog intentionally keeps these IDs.
pub const LEGACY_DEVELOPER_STORAGE_CRON_OPERATION_ID: &str = "cap-v1-0f178cf038854d4a";
pub const LEGACY_DEVELOPER_MULTIPART_ABORT_OPERATION_ID: &str = "cap-v1-5914aa6459d24ff1";
pub const LEGACY_DEVELOPER_MULTIPART_COMPLETE_OPERATION_ID: &str = "cap-v1-5c98b9755e4643ba";
pub const LEGACY_DEVELOPER_MULTIPART_INITIATE_OPERATION_ID: &str = "cap-v1-0d3940728bc19e0e";
pub const LEGACY_DEVELOPER_MULTIPART_PRESIGN_OPERATION_ID: &str = "cap-v1-b6fe5aec600a2e1a";
pub const LEGACY_DEVELOPER_VIDEO_CREATE_OPERATION_ID: &str = "cap-v1-c904ef9c11983a40";
pub const LEGACY_DEVELOPER_USAGE_OPERATION_ID: &str = "cap-v1-cbf22d62a64d3486";
pub const LEGACY_DEVELOPER_VIDEOS_LIST_OPERATION_ID: &str = "cap-v1-6e2296f9695261a3";
pub const LEGACY_DEVELOPER_VIDEO_DELETE_OPERATION_ID: &str = "cap-v1-1cbfe3ecac36f198";
pub const LEGACY_DEVELOPER_VIDEO_GET_OPERATION_ID: &str = "cap-v1-aed411f91e977fe5";
pub const LEGACY_DEVELOPER_VIDEO_STATUS_OPERATION_ID: &str = "cap-v1-718e84b39180c0ac";

pub const LEGACY_DEVELOPER_STORAGE_CRON_PATH: &str = "/api/cron/developer-storage";
pub const LEGACY_DEVELOPER_MULTIPART_ABORT_PATH: &str =
    "/api/developer/sdk/v1/upload/multipart/abort";
pub const LEGACY_DEVELOPER_MULTIPART_COMPLETE_PATH: &str =
    "/api/developer/sdk/v1/upload/multipart/complete";
pub const LEGACY_DEVELOPER_MULTIPART_INITIATE_PATH: &str =
    "/api/developer/sdk/v1/upload/multipart/initiate";
pub const LEGACY_DEVELOPER_MULTIPART_PRESIGN_PATH: &str =
    "/api/developer/sdk/v1/upload/multipart/presign-part";
pub const LEGACY_DEVELOPER_VIDEO_CREATE_PATH: &str = "/api/developer/sdk/v1/videos/create";
pub const LEGACY_DEVELOPER_USAGE_PATH: &str = "/api/developer/v1/usage";
pub const LEGACY_DEVELOPER_VIDEOS_LIST_PATH: &str = "/api/developer/v1/videos";
pub const LEGACY_DEVELOPER_VIDEO_ITEM_PATH: &str = "/api/developer/v1/videos/:id";
pub const LEGACY_DEVELOPER_VIDEO_STATUS_PATH: &str = "/api/developer/v1/videos/:id/status";

pub const LEGACY_DEVELOPER_API_MAX_BODY_BYTES: usize = 256 * 1_024;
pub const LEGACY_DEVELOPER_API_JSON_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_DEVELOPER_METADATA_MAX_UTF16: usize = 8_192;
pub const LEGACY_DEVELOPER_MAX_DURATION_SECONDS: f64 = 4.0 * 60.0 * 60.0;
pub const LEGACY_DEVELOPER_MIN_BALANCE_MICROCREDITS: i64 = 5_000;
pub const LEGACY_DEVELOPER_MICROCREDITS_PER_MINUTE: f64 = 5_000.0;
pub const LEGACY_DEVELOPER_STORAGE_RATE_NUMERATOR: i64 = 333;
pub const LEGACY_DEVELOPER_STORAGE_RATE_DENOMINATOR: i64 = 100;
pub const LEGACY_DEVELOPER_MULTIPART_URL_TTL_SECONDS: u32 = 900;
pub const LEGACY_DEVELOPER_NO_PROTECTED_GATES: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDeveloperApiSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
}

const API_UTILS: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "apps/web/app/api/utils.ts",
    symbol: "developerRateLimiter+developerSdkCors+withDeveloperPublicAuth+withDeveloperSecretAuth",
    sha256: "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
};
const KEY_HASH: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "apps/web/lib/developer-key-hash.ts",
    symbol: "hashKey",
    sha256: "ecc93fc2828647aeaa88dcb9dda0cb2fbcb8b87d4f1a326476878834c06620b1",
};
const DATABASE_SCHEMA: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "developerApps+developerApiKeys+developerVideos+developerCreditAccounts+developerCreditTransactions+developerDailyStorageSnapshots",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
};
const DATABASE: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "packages/database/index.ts",
    symbol: "db",
    sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
};
const IDENTIFIERS: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "packages/database/helpers.ts",
    symbol: "nanoId",
    sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
};
const SDK_MOUNT: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "apps/web/app/api/developer/sdk/v1/[...route]/route.ts",
    symbol: "developer SDK basePath+routes+methods",
    sha256: "d7bff3e0512f37b7991b6728d573be787b3b70dde1fccc7fab445ce263942da4",
};
const REST_MOUNT: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "apps/web/app/api/developer/v1/[...route]/route.ts",
    symbol: "developer REST basePath+routes+methods",
    sha256: "f1ab0e78c1c9ec590b51e5f635cdae7a5fac08c01666c8a0bcf51fe42d100681",
};
const SDK_CLIENT: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "packages/sdk-recorder/src/upload/multipart-client.ts",
    symbol: "MultipartClient",
    sha256: "dfca27f63a9ac358a4001d87d5bd7952b2a884e174773aea70addd6753af3458",
};
const API_DOCS: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "apps/web/content/docs/api/rest-api.mdx",
    symbol: "REST API+SDK API wire documentation",
    sha256: "2098d91af1ee4ac099ed944a03733bfdcd9974ccf4b7fb8463fa049f8aea6d11",
};
const UPLOAD_HANDLER: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "apps/web/app/api/developer/sdk/v1/[...route]/upload.ts",
    symbol: "developer multipart handlers+billing",
    sha256: "0beb3b5366236ba86540500fa854c5a95f29a7a624abbdb838f3b3715cdd83e0",
};
const BUCKET_SERVICE: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "packages/web-backend/src/S3Buckets/index.ts",
    symbol: "S3Buckets.getBucketAccess",
    sha256: "5fc970066be2551488eb3d9e5bcdd1a8255798da53c9b3f4e5c0048c03551b7f",
};
const BUCKET_MULTIPART: LegacyDeveloperApiSourcePinV1 = LegacyDeveloperApiSourcePinV1 {
    path: "packages/web-backend/src/S3Buckets/S3BucketAccess.ts",
    symbol: "multipart.create+getPresignedUploadPartUrl+complete+abort",
    sha256: "d14f27a6e81e9e13c4108aaceb0098875808440b9397620a83f0d17d4c27cd3b",
};

pub const LEGACY_DEVELOPER_MULTIPART_SOURCES: &[LegacyDeveloperApiSourcePinV1] = &[
    UPLOAD_HANDLER,
    SDK_MOUNT,
    API_UTILS,
    KEY_HASH,
    DATABASE_SCHEMA,
    DATABASE,
    IDENTIFIERS,
    BUCKET_SERVICE,
    BUCKET_MULTIPART,
    SDK_CLIENT,
    API_DOCS,
];

pub const LEGACY_DEVELOPER_VIDEO_CREATE_SOURCES: &[LegacyDeveloperApiSourcePinV1] = &[
    LegacyDeveloperApiSourcePinV1 {
        path: "apps/web/app/api/developer/sdk/v1/[...route]/video-create.ts",
        symbol: "POST /create",
        sha256: "0b79fae22402cc26b5f13b6b185ec74f832f5bf587dc079c2b76b09bfe16405d",
    },
    SDK_MOUNT,
    API_UTILS,
    KEY_HASH,
    DATABASE_SCHEMA,
    DATABASE,
    IDENTIFIERS,
    SDK_CLIENT,
    API_DOCS,
    LegacyDeveloperApiSourcePinV1 {
        path: "packages/env/index.ts",
        symbol: "buildEnv.NEXT_PUBLIC_WEB_URL",
        sha256: "c15990c4bfb98c65518003ba9692dd8d2c173c36e78991be1f519cce89e96dc9",
    },
];

pub const LEGACY_DEVELOPER_USAGE_SOURCES: &[LegacyDeveloperApiSourcePinV1] = &[
    LegacyDeveloperApiSourcePinV1 {
        path: "apps/web/app/api/developer/v1/[...route]/usage.ts",
        symbol: "GET /",
        sha256: "e5e9962180456949598932306c50f664a873575e758e9d2fbf8a55c3d277a828",
    },
    REST_MOUNT,
    API_UTILS,
    KEY_HASH,
    DATABASE_SCHEMA,
    DATABASE,
    API_DOCS,
];

pub const LEGACY_DEVELOPER_VIDEOS_SOURCES: &[LegacyDeveloperApiSourcePinV1] = &[
    LegacyDeveloperApiSourcePinV1 {
        path: "apps/web/app/api/developer/v1/[...route]/videos.ts",
        symbol: "GET /+GET /:id+DELETE /:id+GET /:id/status",
        sha256: "d89f31167fd69f1955dfa6ec52c0449aabdccb08ebe4e8662a2075b0405514f9",
    },
    REST_MOUNT,
    API_UTILS,
    KEY_HASH,
    DATABASE_SCHEMA,
    DATABASE,
    API_DOCS,
];

pub const LEGACY_DEVELOPER_STORAGE_CRON_SOURCES: &[LegacyDeveloperApiSourcePinV1] = &[
    LegacyDeveloperApiSourcePinV1 {
        path: "apps/web/app/api/cron/developer-storage/route.ts",
        symbol: "GET",
        sha256: "362c91fcda48e52ff3287a7ac4a53ffd32f59613169ceb021a2d5d8907293fe8",
    },
    DATABASE_SCHEMA,
    DATABASE,
    IDENTIFIERS,
    LegacyDeveloperApiSourcePinV1 {
        path: "apps/web/__tests__/unit/developer-cron-storage.test.ts",
        symbol: "developer-storage cron job",
        sha256: "618f6ba76fcbe104e9429514c5b7d6f6d7aaeb5e7377b9bacb6aba3789962d55",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperApiSurfaceV1 {
    StorageCron,
    MultipartAbort,
    MultipartComplete,
    MultipartInitiate,
    MultipartPresign,
    VideoCreate,
    Usage,
    VideosList,
    VideoDelete,
    VideoGet,
    VideoStatus,
}

impl LegacyDeveloperApiSurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::StorageCron => LEGACY_DEVELOPER_STORAGE_CRON_OPERATION_ID,
            Self::MultipartAbort => LEGACY_DEVELOPER_MULTIPART_ABORT_OPERATION_ID,
            Self::MultipartComplete => LEGACY_DEVELOPER_MULTIPART_COMPLETE_OPERATION_ID,
            Self::MultipartInitiate => LEGACY_DEVELOPER_MULTIPART_INITIATE_OPERATION_ID,
            Self::MultipartPresign => LEGACY_DEVELOPER_MULTIPART_PRESIGN_OPERATION_ID,
            Self::VideoCreate => LEGACY_DEVELOPER_VIDEO_CREATE_OPERATION_ID,
            Self::Usage => LEGACY_DEVELOPER_USAGE_OPERATION_ID,
            Self::VideosList => LEGACY_DEVELOPER_VIDEOS_LIST_OPERATION_ID,
            Self::VideoDelete => LEGACY_DEVELOPER_VIDEO_DELETE_OPERATION_ID,
            Self::VideoGet => LEGACY_DEVELOPER_VIDEO_GET_OPERATION_ID,
            Self::VideoStatus => LEGACY_DEVELOPER_VIDEO_STATUS_OPERATION_ID,
        }
    }
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::StorageCron => LEGACY_DEVELOPER_STORAGE_CRON_PATH,
            Self::MultipartAbort => LEGACY_DEVELOPER_MULTIPART_ABORT_PATH,
            Self::MultipartComplete => LEGACY_DEVELOPER_MULTIPART_COMPLETE_PATH,
            Self::MultipartInitiate => LEGACY_DEVELOPER_MULTIPART_INITIATE_PATH,
            Self::MultipartPresign => LEGACY_DEVELOPER_MULTIPART_PRESIGN_PATH,
            Self::VideoCreate => LEGACY_DEVELOPER_VIDEO_CREATE_PATH,
            Self::Usage => LEGACY_DEVELOPER_USAGE_PATH,
            Self::VideosList => LEGACY_DEVELOPER_VIDEOS_LIST_PATH,
            Self::VideoDelete | Self::VideoGet => LEGACY_DEVELOPER_VIDEO_ITEM_PATH,
            Self::VideoStatus => LEGACY_DEVELOPER_VIDEO_STATUS_PATH,
        }
    }
    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::StorageCron
            | Self::Usage
            | Self::VideosList
            | Self::VideoGet
            | Self::VideoStatus => "GET",
            Self::VideoDelete => "DELETE",
            _ => "POST",
        }
    }
    #[must_use]
    pub const fn mutation(self) -> bool {
        matches!(
            self,
            Self::MultipartAbort
                | Self::MultipartComplete
                | Self::MultipartInitiate
                | Self::MultipartPresign
                | Self::VideoCreate
                | Self::VideoDelete
        )
    }
    #[must_use]
    pub const fn rate_limit_bucket(self) -> &'static str {
        match self {
            Self::StorageCron
            | Self::MultipartAbort
            | Self::MultipartComplete
            | Self::MultipartInitiate
            | Self::MultipartPresign => "upload_storage.v1",
            _ => "developer_api.v1",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyDeveloperPartV1 {
    pub part_number: u16,
    pub etag: String,
    pub size: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyDeveloperApiInputV1 {
    StorageCron {
        snapshot_day: String,
    },
    MultipartAbort {
        video_id: String,
        upload_id: String,
    },
    MultipartComplete {
        video_id: String,
        upload_id: String,
        parts: Vec<LegacyDeveloperPartV1>,
        duration_seconds: f64,
        width: Option<f64>,
        height: Option<f64>,
        fps: Option<f64>,
    },
    MultipartInitiate {
        video_id: String,
        content_type: Option<String>,
    },
    MultipartPresign {
        video_id: String,
        upload_id: String,
        part_number: u16,
    },
    VideoCreate {
        name: Option<String>,
        external_user_id: Option<String>,
        metadata: Option<Value>,
    },
    Usage,
    VideosList {
        external_user_id: Option<String>,
        limit: u16,
        offset: u32,
    },
    VideoDelete {
        video_id: String,
    },
    VideoGet {
        video_id: String,
    },
    VideoStatus {
        video_id: String,
    },
}

impl LegacyDeveloperApiInputV1 {
    #[must_use]
    pub const fn surface(&self) -> LegacyDeveloperApiSurfaceV1 {
        match self {
            Self::StorageCron { .. } => LegacyDeveloperApiSurfaceV1::StorageCron,
            Self::MultipartAbort { .. } => LegacyDeveloperApiSurfaceV1::MultipartAbort,
            Self::MultipartComplete { .. } => LegacyDeveloperApiSurfaceV1::MultipartComplete,
            Self::MultipartInitiate { .. } => LegacyDeveloperApiSurfaceV1::MultipartInitiate,
            Self::MultipartPresign { .. } => LegacyDeveloperApiSurfaceV1::MultipartPresign,
            Self::VideoCreate { .. } => LegacyDeveloperApiSurfaceV1::VideoCreate,
            Self::Usage => LegacyDeveloperApiSurfaceV1::Usage,
            Self::VideosList { .. } => LegacyDeveloperApiSurfaceV1::VideosList,
            Self::VideoDelete { .. } => LegacyDeveloperApiSurfaceV1::VideoDelete,
            Self::VideoGet { .. } => LegacyDeveloperApiSurfaceV1::VideoGet,
            Self::VideoStatus { .. } => LegacyDeveloperApiSurfaceV1::VideoStatus,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyDeveloperApiRequestV1 {
    pub app_id: Option<String>,
    pub input: LegacyDeveloperApiInputV1,
    pub idempotency_key: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct LegacyDeveloperApiCommandV1 {
    operation_id: Option<OrganizationOperationId>,
    app_id: Option<String>,
    input: LegacyDeveloperApiInputV1,
    idempotency_key: Option<IdempotencyKey>,
    request_digest: String,
}

impl fmt::Debug for LegacyDeveloperApiCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyDeveloperApiCommandV1")
            .field("operation_id", &self.operation_id)
            .field("surface", &self.input.surface())
            .field("app_id", &self.app_id)
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("request_digest", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl LegacyDeveloperApiCommandV1 {
    #[must_use]
    pub const fn operation_id(&self) -> Option<OrganizationOperationId> {
        self.operation_id
    }
    #[must_use]
    pub fn app_id(&self) -> Option<&str> {
        self.app_id.as_deref()
    }
    #[must_use]
    pub fn input(&self) -> &LegacyDeveloperApiInputV1 {
        &self.input
    }
    #[must_use]
    pub fn request_digest(&self) -> &str {
        &self.request_digest
    }
    #[must_use]
    pub fn idempotency_key_digest(&self) -> Option<String> {
        self.idempotency_key.as_ref().map(|key| {
            digest_fields(
                b"frame.legacy-developer-api.idempotency.v1\0",
                &[key.expose()],
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDeveloperVideoV1 {
    pub id: String,
    pub app_id: String,
    pub external_user_id: Option<String>,
    pub name: String,
    pub duration: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub fps: Option<f64>,
    pub s3_key: Option<String>,
    pub transcription_status: Option<String>,
    pub metadata: Option<Value>,
    pub deleted_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDeveloperUsageV1 {
    pub balance_micro_credits: i64,
    pub balance_dollars: String,
    pub total_videos: i64,
    pub total_duration_minutes: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDeveloperVideoStatusV1 {
    pub id: String,
    pub duration: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub transcription_status: Option<String>,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyDeveloperApiResultV1 {
    Cron {
        date: String,
        apps_processed: u32,
    },
    Success,
    UploadInitiated {
        upload_id: String,
    },
    PartPresigned {
        presigned_url: String,
    },
    VideoCreated {
        video_id: String,
        s3_key: String,
        share_url: String,
        embed_url: String,
    },
    Usage(LegacyDeveloperUsageV1),
    Videos(Vec<LegacyDeveloperVideoV1>),
    Video(Box<LegacyDeveloperVideoV1>),
    VideoStatus(LegacyDeveloperVideoStatusV1),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyDeveloperApiOutcomeV1 {
    pub result: LegacyDeveloperApiResultV1,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDeveloperApiPortErrorV1 {
    NotFound,
    NoStorageKey,
    InsufficientCredits,
    CreditAccountMissing,
    Conflict,
    Provider,
    Unavailable,
    Corrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyDeveloperApiErrorV1 {
    #[error("Invalid request")]
    InvalidInput,
    #[error("Invalid public key")]
    InvalidPublicKey,
    #[error("Invalid or revoked public key")]
    RevokedPublicKey,
    #[error("Invalid secret key")]
    InvalidSecretKey,
    #[error("Invalid or revoked secret key")]
    RevokedSecretKey,
    #[error("Origin header required for production apps")]
    OriginRequired,
    #[error("Origin not allowed")]
    OriginNotAllowed,
    #[error("Unauthorized")]
    CronUnauthorized,
    #[error("Server misconfiguration")]
    Misconfigured,
    #[error("Video not found")]
    VideoNotFound,
    #[error("Video has no S3 key")]
    NoStorageKey,
    #[error("Insufficient credits")]
    InsufficientCredits,
    #[error("Credit account not found")]
    CreditAccountMissing,
    #[error("Idempotency conflict")]
    Conflict,
    #[error("provider effect failed")]
    Provider,
    #[error("service unavailable")]
    Unavailable,
    #[error("internal state is corrupt")]
    Internal,
}

#[async_trait(?Send)]
pub trait LegacyDeveloperApiPortV1 {
    async fn execute(
        &self,
        command: LegacyDeveloperApiCommandV1,
    ) -> Result<LegacyDeveloperApiOutcomeV1, LegacyDeveloperApiPortErrorV1>;
}

pub struct LegacyDeveloperApiAdapterV1<'a, Port> {
    port: &'a Port,
}
impl<'a, Port: LegacyDeveloperApiPortV1> LegacyDeveloperApiAdapterV1<'a, Port> {
    #[must_use]
    pub const fn new(port: &'a Port) -> Self {
        Self { port }
    }
    pub async fn execute(
        &self,
        request: LegacyDeveloperApiRequestV1,
    ) -> Result<LegacyDeveloperApiOutcomeV1, LegacyDeveloperApiErrorV1> {
        self.port
            .execute(prepare(request)?)
            .await
            .map_err(map_port_error)
    }
}

fn prepare(
    request: LegacyDeveloperApiRequestV1,
) -> Result<LegacyDeveloperApiCommandV1, LegacyDeveloperApiErrorV1> {
    validate_input(&request.input)?;
    let surface = request.input.surface();
    if surface != LegacyDeveloperApiSurfaceV1::StorageCron
        && request
            .app_id
            .as_deref()
            .is_none_or(|value| !valid_uuid_shape(value))
    {
        return Err(LegacyDeveloperApiErrorV1::InvalidInput);
    }
    if !surface.mutation() && request.idempotency_key.is_some() {
        return Err(LegacyDeveloperApiErrorV1::InvalidInput);
    }
    let operation_id = surface.mutation().then(OrganizationOperationId::new);
    let idempotency_key = if surface.mutation() {
        Some(
            IdempotencyKey::parse(request.idempotency_key.unwrap_or_else(|| {
                format!(
                    "developer-auto:{}",
                    operation_id.expect("mutation operation")
                )
            }))
            .map_err(|_| LegacyDeveloperApiErrorV1::InvalidInput)?,
        )
    } else {
        None
    };
    let encoded = fingerprint_payload(&request.input)?;
    let request_digest = digest_fields(
        b"frame.legacy-developer-api.request.v1\0",
        &[
            surface.operation_id(),
            request.app_id.as_deref().unwrap_or("cron"),
            &encoded,
        ],
    );
    Ok(LegacyDeveloperApiCommandV1 {
        operation_id,
        app_id: request.app_id,
        input: request.input,
        idempotency_key,
        request_digest,
    })
}

fn validate_input(input: &LegacyDeveloperApiInputV1) -> Result<(), LegacyDeveloperApiErrorV1> {
    let valid_id = |value: &str| {
        !value.is_empty() && value.len() <= 1_024 && !value.chars().any(char::is_control)
    };
    let finite = |value: Option<f64>| value.is_none_or(f64::is_finite);
    let valid = match input {
        LegacyDeveloperApiInputV1::StorageCron { snapshot_day } => valid_day(snapshot_day),
        LegacyDeveloperApiInputV1::MultipartAbort {
            video_id,
            upload_id,
        } => valid_id(video_id) && valid_id(upload_id),
        LegacyDeveloperApiInputV1::MultipartInitiate {
            video_id,
            content_type,
        } => {
            valid_id(video_id)
                && content_type
                    .as_deref()
                    .is_none_or(|value| value.len() <= 255 && !value.chars().any(char::is_control))
        }
        LegacyDeveloperApiInputV1::MultipartPresign {
            video_id,
            upload_id,
            part_number,
        } => valid_id(video_id) && valid_id(upload_id) && (1..=10_000).contains(part_number),
        LegacyDeveloperApiInputV1::MultipartComplete {
            video_id,
            upload_id,
            parts,
            duration_seconds,
            width,
            height,
            fps,
        } => {
            valid_id(video_id)
                && valid_id(upload_id)
                && duration_seconds.is_finite()
                && *duration_seconds > 0.0
                && finite(*width)
                && finite(*height)
                && finite(*fps)
                && parts.len() <= 10_000
                && parts.iter().all(|part| {
                    (1..=10_000).contains(&part.part_number)
                        && part.size.is_finite()
                        && part.size >= 0.0
                        && part.etag.len() <= 1_024
                        && !part.etag.chars().any(char::is_control)
                })
        }
        LegacyDeveloperApiInputV1::VideoCreate {
            name,
            external_user_id,
            metadata,
        } => {
            name.as_deref().is_none_or(|value| utf16_len(value) <= 255)
                && external_user_id
                    .as_deref()
                    .is_none_or(|value| utf16_len(value) <= 255)
                && metadata.as_ref().is_none_or(|value| {
                    value.is_object()
                        && serde_json::to_string(value).is_ok_and(|json| {
                            utf16_len(&json) <= LEGACY_DEVELOPER_METADATA_MAX_UTF16
                        })
                })
        }
        LegacyDeveloperApiInputV1::Usage => true,
        LegacyDeveloperApiInputV1::VideosList {
            external_user_id,
            limit,
            ..
        } => {
            (1..=100).contains(limit)
                && external_user_id
                    .as_deref()
                    .is_none_or(|value| value.len() <= 1_024)
        }
        LegacyDeveloperApiInputV1::VideoDelete { video_id }
        | LegacyDeveloperApiInputV1::VideoGet { video_id }
        | LegacyDeveloperApiInputV1::VideoStatus { video_id } => valid_id(video_id),
    };
    valid
        .then_some(())
        .ok_or(LegacyDeveloperApiErrorV1::InvalidInput)
}

fn fingerprint_payload(
    input: &LegacyDeveloperApiInputV1,
) -> Result<String, LegacyDeveloperApiErrorV1> {
    let encoded = format!("{input:?}");
    (encoded.len() <= 2_000_000)
        .then_some(encoded)
        .ok_or(LegacyDeveloperApiErrorV1::InvalidInput)
}

fn map_port_error(error: LegacyDeveloperApiPortErrorV1) -> LegacyDeveloperApiErrorV1 {
    match error {
        LegacyDeveloperApiPortErrorV1::NotFound => LegacyDeveloperApiErrorV1::VideoNotFound,
        LegacyDeveloperApiPortErrorV1::NoStorageKey => LegacyDeveloperApiErrorV1::NoStorageKey,
        LegacyDeveloperApiPortErrorV1::InsufficientCredits => {
            LegacyDeveloperApiErrorV1::InsufficientCredits
        }
        LegacyDeveloperApiPortErrorV1::CreditAccountMissing => {
            LegacyDeveloperApiErrorV1::CreditAccountMissing
        }
        LegacyDeveloperApiPortErrorV1::Conflict => LegacyDeveloperApiErrorV1::Conflict,
        LegacyDeveloperApiPortErrorV1::Provider => LegacyDeveloperApiErrorV1::Provider,
        LegacyDeveloperApiPortErrorV1::Unavailable => LegacyDeveloperApiErrorV1::Unavailable,
        LegacyDeveloperApiPortErrorV1::Corrupt => LegacyDeveloperApiErrorV1::Internal,
    }
}

fn valid_day(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| match index {
            4 | 7 => byte == b'-',
            _ => byte.is_ascii_digit(),
        })
}
fn valid_uuid_shape(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}
fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}
fn digest_fields(prefix: &[u8], fields: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(prefix);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_ids_match_the_stable_catalog_and_supplied_aliases_do_not() {
        assert_eq!(
            LEGACY_DEVELOPER_MULTIPART_ABORT_OPERATION_ID,
            "cap-v1-5914aa6459d24ff1"
        );
        assert_ne!(
            LEGACY_DEVELOPER_MULTIPART_ABORT_OPERATION_ID,
            "cap-v1-5914c280b14ba739"
        );
    }
    #[test]
    fn source_validation_preserves_metadata_utf16_and_billing_bounds() {
        let metadata = Value::Object(serde_json::Map::from_iter([(
            "emoji".into(),
            Value::String("😀".into()),
        )]));
        assert!(
            validate_input(&LegacyDeveloperApiInputV1::VideoCreate {
                name: Some("Demo".into()),
                external_user_id: None,
                metadata: Some(metadata)
            })
            .is_ok()
        );
        assert!(
            validate_input(&LegacyDeveloperApiInputV1::MultipartComplete {
                video_id: "video".into(),
                upload_id: "upload".into(),
                parts: vec![LegacyDeveloperPartV1 {
                    part_number: 1,
                    etag: "etag".into(),
                    size: 5_242_880.0
                }],
                duration_seconds: 1.0,
                width: None,
                height: None,
                fps: None
            })
            .is_ok()
        );
    }
    #[test]
    fn reads_forbid_idempotency_while_mutations_accept_absent_keys() {
        let app = "01900000-0000-7000-8000-000000000001".to_owned();
        let read = LegacyDeveloperApiRequestV1 {
            app_id: Some(app.clone()),
            input: LegacyDeveloperApiInputV1::Usage,
            idempotency_key: Some("developer-read-key".into()),
        };
        assert_eq!(
            prepare(read).expect_err("developer reads must reject idempotency keys"),
            LegacyDeveloperApiErrorV1::InvalidInput
        );
        let write = LegacyDeveloperApiRequestV1 {
            app_id: Some(app),
            input: LegacyDeveloperApiInputV1::VideoDelete {
                video_id: "video".into(),
            },
            idempotency_key: None,
        };
        assert!(
            prepare(write)
                .expect("write")
                .idempotency_key_digest()
                .is_some()
        );
    }
}
