//! Source-pinned local contracts for Cap media and workflow operations whose
//! final effect still requires a hardware (and, for some operations, provider)
//! executor.
//!
//! This module deliberately models only the trusted local side of the
//! boundary: exact carrier metadata, bounded payload validation, canonical
//! request/principal digests, and immutable replay keys. It cannot manufacture
//! media-engine or provider completion evidence.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

pub const LEGACY_PROTECTED_MEDIA_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES: usize = 256 * 1_024;
pub const LEGACY_PROTECTED_MEDIA_OPERATION_COUNT: usize = 41;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedMediaKindV1 {
    Route,
    Rpc,
    ServerAction,
    Workflow,
}

impl LegacyProtectedMediaKindV1 {
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
pub enum LegacyProtectedMediaAuthV1 {
    SchedulerSecret,
    OptionalSessionOrShareCapability,
    Session,
    PublicOrFlowToken,
    InternalService,
    PublicEdgeOrJobCapability,
    ParentDerived,
}

impl LegacyProtectedMediaAuthV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchedulerSecret => "scheduler_secret",
            Self::OptionalSessionOrShareCapability => "optional_session_or_share_capability",
            Self::Session => "session",
            Self::PublicOrFlowToken => "public_or_flow_token",
            Self::InternalService => "internal_service",
            Self::PublicEdgeOrJobCapability => "public_edge_or_job_capability",
            Self::ParentDerived => "parent_derived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedMediaIdempotencyV1 {
    Required,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedMediaReplayOriginV1 {
    Caller,
    Natural,
    Grant,
    Workflow,
}

impl LegacyProtectedMediaReplayOriginV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Caller => "caller",
            Self::Natural => "natural",
            Self::Grant => "grant",
            Self::Workflow => "workflow",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedMediaTerminalKindV1 {
    Json,
    Redirect,
    Binary,
    EventStream,
}

impl LegacyProtectedMediaTerminalKindV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Redirect => "redirect",
            Self::Binary => "binary",
            Self::EventStream => "event_stream",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProtectedMediaExecutorKindV1 {
    Gstreamer,
    Provider,
    ControlPlane,
}

impl LegacyProtectedMediaExecutorKindV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Gstreamer => "gstreamer",
            Self::Provider => "provider",
            Self::ControlPlane => "control_plane",
        }
    }
}

impl LegacyProtectedMediaIdempotencyV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Forbidden => "forbidden",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyProtectedMediaProfileV1 {
    pub operation_id: &'static str,
    pub kind: LegacyProtectedMediaKindV1,
    pub method: &'static str,
    pub path: &'static str,
    pub auth: LegacyProtectedMediaAuthV1,
    pub idempotency: LegacyProtectedMediaIdempotencyV1,
    pub provider_execution_required: bool,
    pub required_fields: &'static [&'static str],
    pub target_field: Option<&'static str>,
    pub source_path: &'static str,
    pub source_symbol: &'static str,
    pub source_sha256: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyProtectedMediaSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
}

pub const LEGACY_PROTECTED_MEDIA_TRANSCRIBE_STATUS_ADDITIONAL_SOURCES:
    &[LegacyProtectedMediaSourcePinV1] = &[
    LegacyProtectedMediaSourcePinV1 {
        path: "packages/web-api-contract-effect/src/index.ts",
        symbol: "getTranscribeStatus",
        sha256: "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
    },
    LegacyProtectedMediaSourcePinV1 {
        path: "packages/web-api-contract/src/index.ts",
        symbol: "GET /video/transcribe/status",
        sha256: "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
    },
];

pub const LEGACY_PROTECTED_MEDIA_THUMBNAILS_RPC_ADDITIONAL_SOURCES:
    &[LegacyProtectedMediaSourcePinV1] = &[
    LegacyProtectedMediaSourcePinV1 {
        path: "packages/web-backend/src/Rpcs.ts",
        symbol: "RpcsLive+RpcAuthMiddlewareLive",
        sha256: "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
    },
    LegacyProtectedMediaSourcePinV1 {
        path: "packages/web-backend/src/Videos/VideosRpcs.ts",
        symbol: "VideosGetThumbnails",
        sha256: "6edf9add90a28c542fb53c9a7bfa858bc89290e2a0fbeec827210bd5af189623",
    },
    LegacyProtectedMediaSourcePinV1 {
        path: "packages/web-backend/src/Videos/index.ts",
        symbol: "Videos.getThumbnailURL",
        sha256: "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
    },
    LegacyProtectedMediaSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        symbol: "VideosGetThumbnails",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    },
];

impl LegacyProtectedMediaProfileV1 {
    #[must_use]
    pub fn additional_sources(self) -> &'static [LegacyProtectedMediaSourcePinV1] {
        match self.operation_id {
            "cap-v1-c471cd8f8f990fcc" => {
                LEGACY_PROTECTED_MEDIA_TRANSCRIBE_STATUS_ADDITIONAL_SOURCES
            }
            "cap-v1-aa2bd4c3be69ed42" => LEGACY_PROTECTED_MEDIA_THUMBNAILS_RPC_ADDITIONAL_SOURCES,
            _ => &[],
        }
    }

    #[must_use]
    pub fn source_count(self) -> usize {
        1 + self.additional_sources().len()
    }

    #[must_use]
    pub fn source_matches(self, path: &str, sha256: &str) -> bool {
        (self.source_path == path && self.source_sha256 == sha256)
            || self
                .additional_sources()
                .iter()
                .any(|source| source.path == path && source.sha256 == sha256)
    }

    #[must_use]
    pub fn terminal_kind(self) -> LegacyProtectedMediaTerminalKindV1 {
        match self.operation_id {
            "cap-v1-16bb19025813dbd2" | "cap-v1-2df159f71ce3ccdd" => {
                LegacyProtectedMediaTerminalKindV1::Redirect
            }
            "cap-v1-77fe8c9a4b418f53"
            | "cap-v1-a2814dde3550e586"
            | "cap-v1-9ed2e7b3f858eaaa"
            | "cap-v1-4165632f8266ae06" => LegacyProtectedMediaTerminalKindV1::Binary,
            "cap-v1-43bc9ae6aa4f44a8" => LegacyProtectedMediaTerminalKindV1::EventStream,
            _ => LegacyProtectedMediaTerminalKindV1::Json,
        }
    }

    #[must_use]
    pub fn executor_kind(self) -> LegacyProtectedMediaExecutorKindV1 {
        if self.path.starts_with("/media-server/") {
            if matches!(self.method, "GET" | "HEAD") {
                LegacyProtectedMediaExecutorKindV1::ControlPlane
            } else {
                LegacyProtectedMediaExecutorKindV1::Gstreamer
            }
        } else {
            LegacyProtectedMediaExecutorKindV1::Provider
        }
    }
}

macro_rules! media_profile {
    ($id:literal, $kind:ident, $method:literal, $path:literal, $auth:ident,
     $idem:ident, $provider:literal, [$($field:literal),* $(,)?], $target:expr,
     $source:literal, $symbol:literal, $sha:literal) => {
        LegacyProtectedMediaProfileV1 {
            operation_id: $id,
            kind: LegacyProtectedMediaKindV1::$kind,
            method: $method,
            path: $path,
            auth: LegacyProtectedMediaAuthV1::$auth,
            idempotency: LegacyProtectedMediaIdempotencyV1::$idem,
            provider_execution_required: $provider,
            required_fields: &[$($field),*],
            target_field: $target,
            source_path: $source,
            source_symbol: $symbol,
            source_sha256: $sha,
        }
    };
}

pub const LEGACY_PROTECTED_MEDIA_PROFILES: &[LegacyProtectedMediaProfileV1] = &[
    media_profile!(
        "cap-v1-44259057076456cf",
        Route,
        "GET",
        "/api/cron/finalize-stale-desktop-segments",
        SchedulerSecret,
        Forbidden,
        true,
        [],
        None,
        "apps/web/app/api/cron/finalize-stale-desktop-segments/route.ts",
        "GET",
        "c8678611b6bf043d3002c278bd7c152cfc8330884ea04722e7380ba6cdb8d6c5"
    ),
    media_profile!(
        "cap-v1-b3a632bd76471ad5",
        Route,
        "GET",
        "/api/thumbnail",
        OptionalSessionOrShareCapability,
        Forbidden,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/app/api/thumbnail/route.ts",
        "GET",
        "a7d63da8cd96e95fe8bf42c30573f569e6b4f380eedd949f6ed04c4aea0eaf5c"
    ),
    media_profile!(
        "cap-v1-c1ae43fcf8ad7018",
        Route,
        "GET",
        "/api/video/ai",
        Session,
        Forbidden,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/app/api/video/ai/route.ts",
        "GET",
        "da2c0454a0f654b03ecc5b46b80bce3c46c7c0c46a7f26d39e6b0471d46350d2"
    ),
    media_profile!(
        "cap-v1-16bb19025813dbd2",
        Route,
        "GET",
        "/api/video/preview",
        OptionalSessionOrShareCapability,
        Forbidden,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/app/api/video/preview/route.ts",
        "GET",
        "ddd21307c4413d010eca45b0c88f0b7493f0d0daf52bae27b85e4ea491e6691e"
    ),
    media_profile!(
        "cap-v1-2df159f71ce3ccdd",
        Route,
        "HEAD",
        "/api/video/preview",
        OptionalSessionOrShareCapability,
        Forbidden,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/app/api/video/preview/route.ts",
        "HEAD",
        "ddd21307c4413d010eca45b0c88f0b7493f0d0daf52bae27b85e4ea491e6691e"
    ),
    media_profile!(
        "cap-v1-c471cd8f8f990fcc",
        Route,
        "GET",
        "/api/video/transcribe/status",
        Session,
        Forbidden,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/app/api/video/transcribe/status/route.ts",
        "GET",
        "6860ac27aace6b1fea8a8b821d30d820b4fa0a04d87a05a9956d12df819261da"
    ),
    media_profile!(
        "cap-v1-39909646286251af",
        Route,
        "POST",
        "/api/videos/:videoId/retry-ai",
        Session,
        Required,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/app/api/videos/[videoId]/retry-ai/route.ts",
        "POST",
        "a9b9cf87185693edb891ac85e4650e3b5ef21c7783c56446d8dc68117dfbb8d4"
    ),
    media_profile!(
        "cap-v1-105318e146fceb4c",
        Route,
        "POST",
        "/media-server/audio/check",
        InternalService,
        Required,
        false,
        ["videoUrl"],
        None,
        "apps/media-server/src/routes/audio.ts",
        "POST /check",
        "1ea2a16ef8cf9e93373909a7766c148fe451617a2ff1c333f649623f1e8d1b43"
    ),
    media_profile!(
        "cap-v1-77fe8c9a4b418f53",
        Route,
        "POST",
        "/media-server/audio/convert",
        InternalService,
        Required,
        false,
        ["audioUrl"],
        None,
        "apps/media-server/src/routes/audio.ts",
        "POST /convert",
        "1ea2a16ef8cf9e93373909a7766c148fe451617a2ff1c333f649623f1e8d1b43"
    ),
    media_profile!(
        "cap-v1-a2814dde3550e586",
        Route,
        "POST",
        "/media-server/audio/extract",
        InternalService,
        Required,
        false,
        ["videoUrl"],
        None,
        "apps/media-server/src/routes/audio.ts",
        "POST /extract",
        "1ea2a16ef8cf9e93373909a7766c148fe451617a2ff1c333f649623f1e8d1b43"
    ),
    media_profile!(
        "cap-v1-fbd3d44a0ca1786f",
        Route,
        "GET",
        "/media-server/audio/status",
        PublicEdgeOrJobCapability,
        Forbidden,
        false,
        [],
        None,
        "apps/media-server/src/routes/audio.ts",
        "GET /status",
        "1ea2a16ef8cf9e93373909a7766c148fe451617a2ff1c333f649623f1e8d1b43"
    ),
    media_profile!(
        "cap-v1-0bf20f7e9b1a474c",
        Route,
        "GET",
        "/media-server/health",
        PublicEdgeOrJobCapability,
        Forbidden,
        false,
        [],
        None,
        "apps/media-server/src/routes/health.ts",
        "GET /",
        "7bfe0acde3975a12112a2c465cc9088a29d73f0bdf9f52d9fb01e233e636fb00"
    ),
    media_profile!(
        "cap-v1-ee9797dd352c4e11",
        Route,
        "POST",
        "/media-server/video/cleanup",
        InternalService,
        Required,
        false,
        [],
        None,
        "apps/media-server/src/routes/video.ts",
        "POST /cleanup",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-9ed2e7b3f858eaaa",
        Route,
        "POST",
        "/media-server/video/convert",
        InternalService,
        Required,
        false,
        ["videoUrl"],
        None,
        "apps/media-server/src/routes/video.ts",
        "POST /convert",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-2b48f7704d996758",
        Route,
        "POST",
        "/media-server/video/edit",
        InternalService,
        Required,
        false,
        [
            "videoId",
            "userId",
            "sourceUrl",
            "outputPresignedUrl",
            "keepRanges"
        ],
        Some("videoId"),
        "apps/media-server/src/routes/video.ts",
        "POST /edit",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-aa975a14fd384a5c",
        Route,
        "POST",
        "/media-server/video/force-cleanup",
        InternalService,
        Required,
        false,
        [],
        None,
        "apps/media-server/src/routes/video.ts",
        "POST /force-cleanup",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-bf2eb9302de590a1",
        Route,
        "POST",
        "/media-server/video/mux-segments",
        InternalService,
        Required,
        false,
        ["videoId", "userId", "videoInitUrl", "videoSegmentUrls"],
        Some("videoId"),
        "apps/media-server/src/routes/video.ts",
        "POST /mux-segments",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-ba986b8c5b07cfd6",
        Route,
        "POST",
        "/media-server/video/probe",
        InternalService,
        Required,
        false,
        ["videoUrl"],
        None,
        "apps/media-server/src/routes/video.ts",
        "POST /probe",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-320876fa0aec77cb",
        Route,
        "POST",
        "/media-server/video/process",
        InternalService,
        Required,
        false,
        ["videoId", "userId", "videoUrl", "outputPresignedUrl"],
        Some("videoId"),
        "apps/media-server/src/routes/video.ts",
        "POST /process",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-fc2e2bd0d28ffbf3",
        Route,
        "POST",
        "/media-server/video/process/:jobId/cancel",
        InternalService,
        Required,
        false,
        ["jobId"],
        Some("jobId"),
        "apps/media-server/src/routes/video.ts",
        "POST /process/:jobId/cancel",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-43bc9ae6aa4f44a8",
        Route,
        "GET",
        "/media-server/video/process/:jobId/status",
        PublicEdgeOrJobCapability,
        Forbidden,
        false,
        ["jobId"],
        Some("jobId"),
        "apps/media-server/src/routes/video.ts",
        "GET /process/:jobId/status",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-986bf73a0b5cb676",
        Route,
        "GET",
        "/media-server/video/status",
        PublicEdgeOrJobCapability,
        Forbidden,
        false,
        [],
        None,
        "apps/media-server/src/routes/video.ts",
        "GET /status",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-4165632f8266ae06",
        Route,
        "POST",
        "/media-server/video/thumbnail",
        InternalService,
        Required,
        false,
        ["videoUrl"],
        None,
        "apps/media-server/src/routes/video.ts",
        "POST /thumbnail",
        "93dec5d0e2ead00b9755d08bda9e1393a27e846d27b88293f566941ccce0aa8c"
    ),
    media_profile!(
        "cap-v1-aa2bd4c3be69ed42",
        Rpc,
        "RPC",
        "/api/erpc#VideosGetThumbnails",
        OptionalSessionOrShareCapability,
        Required,
        true,
        [],
        None,
        "apps/web/app/api/erpc/route.ts",
        "Effect RPC HTTP transport",
        "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783"
    ),
    media_profile!(
        "cap-v1-24ef9eb18c4b0555",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/video/create-for-processing.ts#createVideoForServerProcessing",
        Session,
        Required,
        true,
        ["orgId"],
        Some("orgId"),
        "apps/web/actions/video/create-for-processing.ts",
        "createVideoForServerProcessing",
        "d66d5c5e4d9b02eafa5d741dfd0131e923eec288648d526b62b03d23e8f60661"
    ),
    media_profile!(
        "cap-v1-243668046d7d1c3a",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/video/finalize-desktop-segments.ts#finalizeDesktopSegmentsRecording",
        Session,
        Required,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/actions/video/finalize-desktop-segments.ts",
        "finalizeDesktopSegmentsRecording",
        "1717bc84e1443eeaa9f2a62d2e4af038039f9acb50c5c5188dfc50e16b5bfc04"
    ),
    media_profile!(
        "cap-v1-94a9944ce37fa085",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/video/retry-processing.ts#retryVideoProcessing",
        Session,
        Required,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/actions/video/retry-processing.ts",
        "retryVideoProcessing",
        "a70bfeb6b1f092bb50933d7411841f3c07f65e58c391382e141b46e21f2d7cd6"
    ),
    media_profile!(
        "cap-v1-187fbaf66d21b311",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/video/trigger-instant-recording-processing.ts#triggerInstantRecordingProcessing",
        Session,
        Required,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/actions/video/trigger-instant-recording-processing.ts",
        "triggerInstantRecordingProcessing",
        "f570fef686a3b99d332cfe941f17054b7e6b4ac306b96f8be3ce98b418a13fb8"
    ),
    media_profile!(
        "cap-v1-4b12db3b619dce8f",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/video/trigger-processing.ts#triggerVideoProcessing",
        Session,
        Required,
        true,
        ["videoId", "rawFileKey", "bucketId"],
        Some("videoId"),
        "apps/web/actions/video/trigger-processing.ts",
        "triggerVideoProcessing",
        "dd5a6c2a8df1ef1a20c7c17b4d20664a6cd0f1c9e2044f6b35cf32539517a1c2"
    ),
    media_profile!(
        "cap-v1-8e495fce95e6282b",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/videos/save-edits.ts#restoreVideoToOriginal",
        Session,
        Required,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/actions/videos/save-edits.ts",
        "restoreVideoToOriginal",
        "4360a298c34fd1618bb3470f496c66fdb7038343ed7fe8c04e4004591b8e8a64"
    ),
    media_profile!(
        "cap-v1-a8dc17023685b8c0",
        ServerAction,
        "ACTION",
        "action://apps/web/actions/videos/save-edits.ts#saveVideoEdits",
        Session,
        Required,
        true,
        ["videoId", "editSpec"],
        Some("videoId"),
        "apps/web/actions/videos/save-edits.ts",
        "saveVideoEdits",
        "4360a298c34fd1618bb3470f496c66fdb7038343ed7fe8c04e4004591b8e8a64"
    ),
    media_profile!(
        "cap-v1-39c33826cf514552",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/lib/desktop-segments-finalization.ts#queueDesktopSegmentsFinalization",
        ParentDerived,
        Required,
        true,
        ["videoId", "userId"],
        Some("videoId"),
        "apps/web/lib/desktop-segments-finalization.ts",
        "queueDesktopSegmentsFinalization",
        "298c57c6c73cd368abc725acff12c472e953e6856075258552cb066174dd60dd"
    ),
    media_profile!(
        "cap-v1-4cff2b6f3cd102f5",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/lib/desktop-segments-recovery.ts#completeDesktopSegmentsManifestAndQueue",
        ParentDerived,
        Required,
        true,
        ["videoId"],
        Some("videoId"),
        "apps/web/lib/desktop-segments-recovery.ts",
        "completeDesktopSegmentsManifestAndQueue",
        "7678e20860c7fea9fed89689fdd452b9b437b5c1ce4eb8a607c7081d5b5d9187"
    ),
    media_profile!(
        "cap-v1-b3fac7b3df933825",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/lib/desktop-segments-recovery.ts#recoverStaleDesktopSegments",
        ParentDerived,
        Required,
        true,
        [],
        None,
        "apps/web/lib/desktop-segments-recovery.ts",
        "recoverStaleDesktopSegments",
        "7678e20860c7fea9fed89689fdd452b9b437b5c1ce4eb8a607c7081d5b5d9187"
    ),
    media_profile!(
        "cap-v1-3e0dec6125f270bf",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/lib/generate-ai.ts#startAiGeneration",
        ParentDerived,
        Required,
        true,
        ["videoId", "userId"],
        Some("videoId"),
        "apps/web/lib/generate-ai.ts",
        "startAiGeneration",
        "bcb303c00e73bd035181f9298b32afac24114b28059a39679b0476983fba0751"
    ),
    media_profile!(
        "cap-v1-6d73e4dfdca61f06",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/lib/video-processing.ts#startVideoProcessingWorkflow",
        ParentDerived,
        Required,
        true,
        ["videoId", "userId", "rawFileKey", "bucketId"],
        Some("videoId"),
        "apps/web/lib/video-processing.ts",
        "startVideoProcessingWorkflow",
        "56d755ad564725c2912a48bce70e2410b991e2bb94889aba021ad4f1ecad32a0"
    ),
    media_profile!(
        "cap-v1-59ce5faf2189c1a1",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/workflows/edit-video.ts#editVideoWorkflow",
        ParentDerived,
        Required,
        true,
        [
            "videoId",
            "userId",
            "sourceKey",
            "previousSpec",
            "editSpec",
            "keepRanges",
            "aiGenerationEnabled"
        ],
        Some("videoId"),
        "apps/web/workflows/edit-video.ts",
        "editVideoWorkflow",
        "7a2cb77bc0ac74d409eca183f41ee2db139cd1dd264d73c73259098c5db0b134"
    ),
    media_profile!(
        "cap-v1-7868ad041c2754df",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/workflows/finalize-desktop-recording.ts#finalizeDesktopRecordingWorkflow",
        ParentDerived,
        Required,
        true,
        ["videoId", "userId"],
        Some("videoId"),
        "apps/web/workflows/finalize-desktop-recording.ts",
        "finalizeDesktopRecordingWorkflow",
        "8363e6455411dc9660d2bc72964d37fb2bf9f4616c418570b0322b6a45573cb6"
    ),
    media_profile!(
        "cap-v1-c79bf3eeab46cbf0",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/workflows/generate-ai.ts#generateAiWorkflow",
        ParentDerived,
        Required,
        true,
        ["videoId", "userId"],
        Some("videoId"),
        "apps/web/workflows/generate-ai.ts",
        "generateAiWorkflow",
        "8f383e00617a32824082231d9b423f24713e805b8373df22b7803792369e9a78"
    ),
    media_profile!(
        "cap-v1-0d39ec834208980f",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/workflows/process-video.ts#processVideoWorkflow",
        ParentDerived,
        Required,
        true,
        ["videoId", "userId", "rawFileKey", "bucketId"],
        Some("videoId"),
        "apps/web/workflows/process-video.ts",
        "processVideoWorkflow",
        "972696993e47609932fedb6973f75b2c26dafdca0363b07e061d57c777d4095d"
    ),
    media_profile!(
        "cap-v1-43c049b69abb6704",
        Workflow,
        "WORKFLOW",
        "workflow://apps/web/workflows/transcribe.ts#transcribeVideoWorkflow",
        ParentDerived,
        Required,
        true,
        ["videoId", "userId", "aiGenerationEnabled"],
        Some("videoId"),
        "apps/web/workflows/transcribe.ts",
        "transcribeVideoWorkflow",
        "cd422235773217226ebb76d04f54a0ee17b903f943ebce1959e6564877f6587d"
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedMediaPolicyProofV1 {
    pub target_id: String,
    pub kind: String,
    pub subject_id: String,
    pub revision: i64,
    /// SHA-256 of the matched stored password hash, or a deterministic
    /// non-secret policy snapshot digest. The stored password hash itself is
    /// never copied into a receipt.
    pub audit_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedMediaEntitlementBindingV1 {
    pub kind: String,
    pub subject_id: String,
    pub revision: i64,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedMediaPrincipalV1 {
    pub class: String,
    pub actor_id: Option<String>,
    pub tenant_id: Option<String>,
    /// Exact credential class behind this admission. Session credentials bind
    /// the D1 session id/version/digest; service credentials bind the named
    /// secret authority; anonymous reads bind the exact video capability.
    pub credential_kind: String,
    pub credential_subject_id: Option<String>,
    pub credential_key_version: Option<i64>,
    /// Digest of the presented credential. Raw credentials never cross into
    /// the application envelope or D1.
    pub credential_digest: Option<String>,
    /// Composite video policy proofs are separate from the authentication
    /// credential so a session remains bound to its exact D1 token while also
    /// proving password/public/owner policy for every requested video.
    pub policy_proofs: Vec<LegacyProtectedMediaPolicyProofV1>,
    pub entitlement_binding: Option<LegacyProtectedMediaEntitlementBindingV1>,
}

impl LegacyProtectedMediaPrincipalV1 {
    #[must_use]
    pub fn digest(&self) -> String {
        let canonical = serde_json::to_vec(self).unwrap_or_default();
        hex_digest(&canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProtectedMediaEnvelopeV1 {
    pub source_operation_id: String,
    pub principal: LegacyProtectedMediaPrincipalV1,
    pub execution_key: String,
    pub replay_origin: LegacyProtectedMediaReplayOriginV1,
    /// Workflow requests are child-only and must name the immutable initiating
    /// receipt/request pair. Non-workflow carriers must leave both absent.
    pub parent_family: Option<String>,
    pub parent_receipt_id: Option<String>,
    pub parent_request_digest: Option<String>,
    /// Exact authority-binding digest stored by the initiating parent. The
    /// child computes its own operation-scoped authority digest separately.
    pub parent_authority_binding_digest: Option<String>,
    /// Digest of the exact live session/service/share/resource authorization
    /// decision made immediately before staging. Replay and evidence gates
    /// compare this value and independently re-evaluate live authority.
    pub authority_binding_digest: String,
    pub payload: Value,
    /// Opaque location of the exact secret-bearing payload. HTTP/RPC/action
    /// decoders never accept this value from callers; only the internal vault
    /// adapter may populate it after independently digesting the plaintext.
    pub sealed_request_ref: Option<String>,
    pub sealed_request_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyProtectedMediaValidatedV1 {
    /// Persistable allowlisted descriptor. It contains no raw URL, token,
    /// object key, edit document, webhook secret, or multipart control URL.
    pub request_descriptor_json: String,
    pub request_digest: String,
    pub payload_digest: String,
    pub principal_digest: String,
    pub execution_key_digest: String,
    pub replay_origin: LegacyProtectedMediaReplayOriginV1,
    pub authority_binding_digest: String,
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LegacyProtectedMediaValidationErrorV1 {
    #[error("unknown protected media operation")]
    UnknownOperation,
    #[error("invalid protected media payload")]
    InvalidPayload,
    #[error("protected media payload is too large")]
    PayloadTooLarge,
    #[error("invalid protected media execution key")]
    InvalidExecutionKey,
}

#[must_use]
pub fn legacy_protected_media_profile(
    operation_id: &str,
) -> Option<&'static LegacyProtectedMediaProfileV1> {
    LEGACY_PROTECTED_MEDIA_PROFILES
        .iter()
        .find(|profile| profile.operation_id == operation_id)
}

/// Match an HTTP carrier without accepting prefix/suffix lookalikes. Dynamic
/// video/job identifiers are deliberately not decoded here; the web adapter
/// extracts and validates them from the same path after this exact shape match.
#[must_use]
pub fn legacy_protected_media_route_profile(
    method: &str,
    path: &str,
) -> Option<&'static LegacyProtectedMediaProfileV1> {
    if let Some(profile) = LEGACY_PROTECTED_MEDIA_PROFILES.iter().find(|profile| {
        profile.kind == LegacyProtectedMediaKindV1::Route
            && profile.method == method
            && !profile.path.contains(':')
            && profile.path == path
    }) {
        return Some(profile);
    }
    let segments: Vec<_> = path.strip_prefix('/')?.split('/').collect();
    let operation_id = match (method, segments.as_slice()) {
        ("POST", ["api", "videos", video_id, "retry-ai"]) if !video_id.is_empty() => {
            "cap-v1-39909646286251af"
        }
        ("POST", ["media-server", "video", "process", job_id, "cancel"]) if !job_id.is_empty() => {
            "cap-v1-fc2e2bd0d28ffbf3"
        }
        ("GET", ["media-server", "video", "process", job_id, "status"]) if !job_id.is_empty() => {
            "cap-v1-43bc9ae6aa4f44a8"
        }
        _ => return None,
    };
    legacy_protected_media_profile(operation_id)
}

pub fn validate_legacy_protected_media_envelope(
    envelope: &LegacyProtectedMediaEnvelopeV1,
) -> Result<LegacyProtectedMediaValidatedV1, LegacyProtectedMediaValidationErrorV1> {
    let profile = legacy_protected_media_profile(&envelope.source_operation_id)
        .ok_or(LegacyProtectedMediaValidationErrorV1::UnknownOperation)?;
    if envelope.execution_key.trim().is_empty() || envelope.execution_key.len() > 512 {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidExecutionKey);
    }
    if profile.idempotency == LegacyProtectedMediaIdempotencyV1::Forbidden
        && !envelope.execution_key.starts_with("server-read:")
    {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidExecutionKey);
    }
    if !valid_lower_digest(&envelope.authority_binding_digest) {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    let parent_valid = match profile.kind {
        LegacyProtectedMediaKindV1::Workflow => {
            envelope.replay_origin == LegacyProtectedMediaReplayOriginV1::Workflow
                && matches!(
                    envelope.parent_family.as_deref(),
                    Some("protected_media" | "protected_integrations")
                )
                && envelope
                    .parent_receipt_id
                    .as_deref()
                    .is_some_and(valid_uuid_shape)
                && envelope
                    .parent_request_digest
                    .as_deref()
                    .is_some_and(valid_lower_digest)
                && envelope
                    .parent_authority_binding_digest
                    .as_deref()
                    .is_some_and(valid_lower_digest)
        }
        _ => {
            envelope.parent_family.is_none()
                && envelope.parent_receipt_id.is_none()
                && envelope.parent_request_digest.is_none()
                && envelope.parent_authority_binding_digest.is_none()
        }
    };
    if !parent_valid {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    if envelope.principal.credential_kind == "parent_capability"
        && (envelope.principal.actor_id.is_some()
            || envelope
                .principal
                .credential_subject_id
                .as_deref()
                .is_none_or(|subject| {
                    subject.split_once(':').is_none_or(|(family, receipt)| {
                        !matches!(family, "protected_media" | "protected_integrations")
                            || !valid_uuid_shape(receipt)
                    })
                }))
    {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    validate_principal(profile, &envelope.principal)?;

    let payload_bytes = serde_json::to_vec(&envelope.payload)
        .map_err(|_| LegacyProtectedMediaValidationErrorV1::InvalidPayload)?;
    if payload_bytes.len() > LEGACY_PROTECTED_MEDIA_MAX_BODY_BYTES {
        return Err(LegacyProtectedMediaValidationErrorV1::PayloadTooLarge);
    }
    let payload_digest = hex_digest(&payload_bytes);
    let object = envelope.payload.as_object();
    if profile.kind != LegacyProtectedMediaKindV1::Rpc && object.is_none() {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    for field in profile.required_fields {
        let Some(value) = object.and_then(|object| object.get(*field)) else {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        };
        if value.as_str().is_some_and(str::is_empty) || value.as_array().is_some_and(Vec::is_empty)
        {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        }
    }

    // The Effect RPC accepts an array of at most 50 video ids.
    if profile.operation_id == "cap-v1-aa2bd4c3be69ed42" {
        let Some(video_ids) = envelope.payload.as_array() else {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        };
        if video_ids.len() > 50
            || video_ids
                .iter()
                .any(|value| value.as_str().is_none_or(str::is_empty))
        {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        }
    }
    if let Some(object) = object {
        validate_media_payload(object)?;
        if let Some(edit_spec) = object.get("editSpec").and_then(Value::as_object) {
            validate_keep_ranges(edit_spec.get("keepRanges"))?;
        }
        validate_keep_ranges(object.get("keepRanges"))?;
    }

    let (descriptor, protected_material) = persisted_payload_descriptor(&envelope.payload)?;
    match (
        protected_material,
        envelope.sealed_request_ref.as_deref(),
        envelope.sealed_request_digest.as_deref(),
    ) {
        (true, Some(reference), Some(digest))
            if valid_sealed_request_ref(reference) && digest == payload_digest => {}
        (false, None, None) => {}
        _ => return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload),
    }
    let target_id = profile
        .target_field
        .and_then(|field| object.and_then(|object| object.get(field)))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let expected_authority_binding = legacy_protected_media_authority_binding_digest(
        profile,
        &envelope.principal,
        &envelope.payload,
    )?;
    if envelope.authority_binding_digest != expected_authority_binding {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    let request_identity = json!({
        "schema_version": "frame.legacy-protected-media-request.v2",
        "source_operation_id": profile.operation_id,
        "payload_digest": payload_digest,
        "payload_descriptor": descriptor,
        "sealed_request_digest": envelope.sealed_request_digest,
        "authority_binding_digest": envelope.authority_binding_digest,
        "parent_family": envelope.parent_family,
        "parent_receipt_id": envelope.parent_receipt_id,
        "parent_request_digest": envelope.parent_request_digest,
        "parent_authority_binding_digest": envelope.parent_authority_binding_digest,
        "terminal_kind": profile.terminal_kind().as_str(),
        "required_evidence": {
            "hardware_execution": true,
            "provider_execution": profile.provider_execution_required,
        },
    });
    let request_digest = hex_digest(
        serde_json::to_string(&request_identity)
            .map_err(|_| LegacyProtectedMediaValidationErrorV1::InvalidPayload)?
            .as_bytes(),
    );
    let mut persisted_request = request_identity;
    if let Some(reference) = envelope.sealed_request_ref.as_deref() {
        persisted_request
            .as_object_mut()
            .ok_or(LegacyProtectedMediaValidationErrorV1::InvalidPayload)?
            .insert("sealed_request_ref".into(), Value::String(reference.into()));
    }
    let request_descriptor_json = serde_json::to_string(&persisted_request)
        .map_err(|_| LegacyProtectedMediaValidationErrorV1::InvalidPayload)?;
    Ok(LegacyProtectedMediaValidatedV1 {
        request_digest,
        request_descriptor_json,
        payload_digest,
        principal_digest: envelope.principal.digest(),
        execution_key_digest: hex_digest(envelope.execution_key.as_bytes()),
        replay_origin: envelope.replay_origin,
        authority_binding_digest: envelope.authority_binding_digest.clone(),
        target_id,
    })
}

fn validate_principal(
    profile: &LegacyProtectedMediaProfileV1,
    principal: &LegacyProtectedMediaPrincipalV1,
) -> Result<(), LegacyProtectedMediaValidationErrorV1> {
    let actor_valid = principal
        .actor_id
        .as_deref()
        .is_some_and(|actor| !actor.is_empty() && actor.len() <= 255);
    let credential_valid = principal
        .credential_digest
        .as_deref()
        .is_some_and(valid_lower_digest);
    if principal.policy_proofs.len() > 50
        || principal.policy_proofs.iter().any(|proof| {
            proof.target_id.is_empty()
                || proof.target_id.len() > 255
                || proof.subject_id.len() != 36
                || !(0..=9_007_199_254_740_991).contains(&proof.revision)
                || !valid_lower_digest(&proof.audit_digest)
                || !matches!(
                    proof.kind.as_str(),
                    "owner_bypass"
                        | "video_password"
                        | "space_password"
                        | "unprotected_video_policy"
                )
        })
    {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    if principal
        .entitlement_binding
        .as_ref()
        .is_some_and(|binding| {
            binding.kind != "ai_owner"
                || binding.subject_id.len() != 36
                || !(0..=9_007_199_254_740_991).contains(&binding.revision)
                || binding
                    .expires_at_ms
                    .is_some_and(|value| !(1..=9_007_199_254_740_991).contains(&value))
        })
    {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    let binding_valid = !principal.credential_kind.is_empty()
        && principal.credential_kind.len() <= 64
        && principal
            .credential_subject_id
            .as_deref()
            .is_some_and(|value| !value.is_empty() && value.len() <= 255)
        && principal
            .credential_key_version
            .is_some_and(|version| (0..=9_007_199_254_740_991).contains(&version))
        && credential_valid;
    let session_binding = principal.credential_kind == "session_token"
        && principal
            .credential_subject_id
            .as_deref()
            .is_some_and(|value| value.len() == 36)
        && principal
            .credential_key_version
            .is_some_and(|version| (1..=65_535).contains(&version));
    let valid = match profile.auth {
        LegacyProtectedMediaAuthV1::Session => {
            principal.class == "session" && actor_valid && binding_valid && session_binding
        }
        LegacyProtectedMediaAuthV1::SchedulerSecret => {
            principal.class == "scheduler_secret"
                && !actor_valid
                && binding_valid
                && principal.credential_kind == "scheduler_secret"
                && principal.credential_subject_id.as_deref() == Some("CRON_SECRET.v1")
                && principal
                    .credential_key_version
                    .is_some_and(|version| (1..=65_535).contains(&version))
                && principal.policy_proofs.is_empty()
                && principal.entitlement_binding.is_none()
        }
        LegacyProtectedMediaAuthV1::InternalService => {
            principal.class == "internal_service"
                && !actor_valid
                && binding_valid
                && principal.credential_kind == "service_secret"
                && principal.credential_subject_id.as_deref()
                    == Some("MEDIA_SERVER_WEBHOOK_SECRET.v1")
                && principal
                    .credential_key_version
                    .is_some_and(|version| (1..=65_535).contains(&version))
                && principal.policy_proofs.is_empty()
                && principal.entitlement_binding.is_none()
        }
        LegacyProtectedMediaAuthV1::OptionalSessionOrShareCapability => {
            binding_valid
                && ((principal.class == "session" && actor_valid && session_binding)
                    || (principal.class == "optional_session_or_share_capability"
                        && !actor_valid
                        && matches!(
                            principal.credential_kind.as_str(),
                            "video_password_capability"
                                | "space_password_capability"
                                | "public_video_capability"
                        )
                        && principal
                            .credential_subject_id
                            .as_deref()
                            .is_some_and(|value| value.len() == 36)))
        }
        LegacyProtectedMediaAuthV1::PublicOrFlowToken => {
            binding_valid
                && ((principal.class == "session" && actor_valid && session_binding)
                    || (principal.class == "public_or_flow_token"
                        && !actor_valid
                        && principal.credential_kind == "flow_token"
                        && principal.credential_subject_id.as_deref()
                            == Some("FRAME_MEDIA_FLOW_TOKEN.v1")
                        && principal
                            .credential_key_version
                            .is_some_and(|version| (1..=65_535).contains(&version))))
        }
        LegacyProtectedMediaAuthV1::PublicEdgeOrJobCapability => {
            !actor_valid
                && binding_valid
                && principal.class == "public_edge_or_job_capability"
                && matches!(
                    principal.credential_kind.as_str(),
                    "edge_read" | "job_capability"
                )
                && principal.policy_proofs.is_empty()
                && principal.entitlement_binding.is_none()
        }
        LegacyProtectedMediaAuthV1::ParentDerived => {
            principal.class == "parent_derived"
                && binding_valid
                && ((actor_valid && principal.credential_kind == "session_token")
                    || (!actor_valid
                        && matches!(
                            principal.credential_kind.as_str(),
                            "scheduler_secret"
                                | "service_secret"
                                | "flow_token"
                                | "video_password_capability"
                                | "space_password_capability"
                                | "public_video_capability"
                                | "edge_read"
                                | "job_capability"
                                | "parent_capability"
                        )))
        }
    };
    if valid {
        Ok(())
    } else {
        Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload)
    }
}

pub fn legacy_protected_media_authority_binding_digest(
    profile: &LegacyProtectedMediaProfileV1,
    principal: &LegacyProtectedMediaPrincipalV1,
    payload: &Value,
) -> Result<String, LegacyProtectedMediaValidationErrorV1> {
    let video_targets = if profile.kind == LegacyProtectedMediaKindV1::Rpc {
        payload
            .as_array()
            .ok_or(LegacyProtectedMediaValidationErrorV1::InvalidPayload)?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| !value.is_empty() && value.len() <= 255)
                    .map(str::to_owned)
                    .ok_or(LegacyProtectedMediaValidationErrorV1::InvalidPayload)
            })
            .collect::<Result<Vec<_>, _>>()?
    } else if profile.target_field == Some("videoId") {
        vec![
            payload
                .get("videoId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && value.len() <= 255)
                .map(str::to_owned)
                .ok_or(LegacyProtectedMediaValidationErrorV1::InvalidPayload)?,
        ]
    } else {
        Vec::new()
    };
    let policy_required = !video_targets.is_empty()
        && matches!(
            profile.auth,
            LegacyProtectedMediaAuthV1::Session
                | LegacyProtectedMediaAuthV1::OptionalSessionOrShareCapability
                | LegacyProtectedMediaAuthV1::ParentDerived
        );
    if policy_required {
        if principal.policy_proofs.len() != video_targets.len()
            || principal
                .policy_proofs
                .iter()
                .zip(&video_targets)
                .any(|(proof, target)| &proof.target_id != target)
        {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        }
        let owner_required = profile.operation_id == "cap-v1-39909646286251af"
            || profile.kind == LegacyProtectedMediaKindV1::ServerAction;
        if owner_required
            && principal
                .policy_proofs
                .iter()
                .any(|proof| proof.kind != "owner_bypass")
        {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        }
    } else if !principal.policy_proofs.is_empty() {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    let ai_required = matches!(
        profile.operation_id,
        "cap-v1-c1ae43fcf8ad7018" | "cap-v1-39909646286251af"
    );
    if ai_required != principal.entitlement_binding.is_some() {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    let material = serde_json::to_vec(&json!({
        "domain": "frame.protected-media-live-authority.v2",
        "operation_id": profile.operation_id,
        "auth_class": profile.auth.as_str(),
        "actor_id": principal.actor_id,
        "tenant_id": principal.tenant_id,
        "credential_kind": principal.credential_kind,
        "credential_subject_id": principal.credential_subject_id,
        "credential_key_version": principal.credential_key_version,
        "credential_digest": principal.credential_digest,
        "video_targets": video_targets,
        "policy_proofs": principal.policy_proofs,
        "entitlement_binding": principal.entitlement_binding,
        "owner_required": profile.operation_id == "cap-v1-39909646286251af"
            || profile.kind == LegacyProtectedMediaKindV1::ServerAction,
    }))
    .map_err(|_| LegacyProtectedMediaValidationErrorV1::InvalidPayload)?;
    Ok(hex_digest(&material))
}

fn valid_lower_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_uuid_shape(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()
            }
        })
}

fn valid_sealed_request_ref(value: &str) -> bool {
    value
        .strip_prefix("frame-pm-request-v1:")
        .is_some_and(valid_lower_digest)
}

fn persisted_payload_descriptor(
    payload: &Value,
) -> Result<(Value, bool), LegacyProtectedMediaValidationErrorV1> {
    let Some(object) = payload.as_object() else {
        if payload.is_array() {
            return Ok((payload.clone(), false));
        }
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    };
    let mut descriptor = Map::new();
    let mut protected = false;
    for (field, value) in object {
        if protected_payload_field(field) {
            protected = true;
            let encoded = serde_json::to_vec(value)
                .map_err(|_| LegacyProtectedMediaValidationErrorV1::InvalidPayload)?;
            descriptor.insert(
                field.clone(),
                json!({"sealed": true, "sha256": hex_digest(&encoded)}),
            );
        } else if safe_payload_field(field) {
            descriptor.insert(field.clone(), value.clone());
        } else {
            // Unknown fields must not become a caller-controlled immutable D1
            // payload or an accidental new credential channel.
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        }
    }
    Ok((Value::Object(descriptor), protected))
}

fn protected_payload_field(field: &str) -> bool {
    matches!(
        field,
        "videoUrl"
            | "audioUrl"
            | "sourceUrl"
            | "outputPresignedUrl"
            | "outputVerificationUrl"
            | "thumbnailPresignedUrl"
            | "previewGifPresignedUrl"
            | "webhookUrl"
            | "webhookSecret"
            | "videoInitUrl"
            | "videoSegmentUrls"
            | "audioInitUrl"
            | "audioSegmentUrls"
            | "signPartUrl"
            | "completeUrl"
            | "abortUrl"
            | "outputUpload"
            | "rawFileKey"
            | "sourceKey"
            | "bucketId"
            | "uploadId"
            | "editSpec"
            | "previousSpec"
    )
}

fn safe_payload_field(field: &str) -> bool {
    matches!(
        field,
        "videoId"
            | "userId"
            | "jobId"
            | "orgId"
            | "duration"
            | "resolution"
            | "folderId"
            | "fallback"
            | "stream"
            | "outputFormat"
            | "bitrate"
            | "inputExtension"
            | "timestamp"
            | "width"
            | "height"
            | "quality"
            | "maxWidth"
            | "maxHeight"
            | "crf"
            | "preset"
            | "remuxOnly"
            | "keepRanges"
            | "aiGenerationEnabled"
            | "processingMessage"
            | "startFailureMessage"
            | "forceRestart"
            | "mode"
    )
}

fn validate_media_payload(
    object: &serde_json::Map<String, Value>,
) -> Result<(), LegacyProtectedMediaValidationErrorV1> {
    const URL_FIELDS: &[&str] = &[
        "videoUrl",
        "audioUrl",
        "sourceUrl",
        "outputPresignedUrl",
        "outputVerificationUrl",
        "thumbnailPresignedUrl",
        "previewGifPresignedUrl",
        "webhookUrl",
        "videoInitUrl",
        "audioInitUrl",
        "signPartUrl",
        "completeUrl",
        "abortUrl",
    ];
    for field in URL_FIELDS {
        if let Some(value) = object.get(*field) {
            let Some(value) = value.as_str() else {
                return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
            };
            if Url::parse(value).is_err() {
                return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
            }
        }
    }
    for field in ["videoSegmentUrls", "audioSegmentUrls"] {
        if let Some(values) = object.get(field) {
            let Some(values) = values.as_array() else {
                return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
            };
            if values.iter().any(|value| {
                value
                    .as_str()
                    .is_none_or(|value| Url::parse(value).is_err())
            }) {
                return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
            }
        }
    }
    validate_bounded_number(object, "width", 0.0, 2_000.0)?;
    validate_bounded_number(object, "height", 0.0, 2_000.0)?;
    validate_bounded_number(object, "quality", 1.0, 100.0)?;
    validate_bounded_number(object, "timestamp", 0.0, f64::MAX)?;
    validate_bounded_number(object, "maxWidth", 0.0, 4_096.0)?;
    validate_bounded_number(object, "maxHeight", 0.0, 4_096.0)?;
    validate_bounded_number(object, "crf", 0.0, 51.0)?;
    if object.get("preset").is_some_and(|value| {
        !matches!(
            value.as_str(),
            Some("ultrafast" | "fast" | "medium" | "slow")
        )
    }) {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    if object
        .get("outputFormat")
        .is_some_and(|value| value.as_str() != Some("mp3"))
    {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    for field in ["stream", "remuxOnly", "aiGenerationEnabled"] {
        if object.get(field).is_some_and(|value| !value.is_boolean()) {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        }
    }
    Ok(())
}

fn validate_bounded_number(
    object: &serde_json::Map<String, Value>,
    field: &str,
    minimum: f64,
    maximum: f64,
) -> Result<(), LegacyProtectedMediaValidationErrorV1> {
    if let Some(value) = object.get(field) {
        let Some(value) = value.as_f64() else {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        };
        if !(minimum..=maximum).contains(&value) {
            return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
        }
    }
    Ok(())
}

fn validate_keep_ranges(
    ranges: Option<&Value>,
) -> Result<(), LegacyProtectedMediaValidationErrorV1> {
    let Some(ranges) = ranges else {
        return Ok(());
    };
    let Some(ranges) = ranges.as_array() else {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    };
    if ranges.is_empty()
        || ranges.iter().any(|range| {
            let Some(range) = range.as_object() else {
                return true;
            };
            let start = range.get("start").and_then(Value::as_f64);
            let end = range.get("end").and_then(Value::as_f64);
            !matches!((start, end), (Some(start), Some(end)) if start >= 0.0 && end > start)
        })
    {
        return Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload);
    }
    Ok(())
}

#[must_use]
pub fn legacy_protected_media_credential_digest(credential: &str) -> String {
    hex_digest(credential.as_bytes())
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use serde_json::json;

    #[test]
    fn inventory_is_complete_unique_and_source_pinned() {
        assert_eq!(
            LEGACY_PROTECTED_MEDIA_PROFILES.len(),
            LEGACY_PROTECTED_MEDIA_OPERATION_COUNT
        );
        let mut ids = BTreeSet::new();
        let mut source_count = 0;
        for profile in LEGACY_PROTECTED_MEDIA_PROFILES {
            assert!(ids.insert(profile.operation_id));
            assert_eq!(profile.operation_id.len(), 23);
            assert_eq!(profile.source_sha256.len(), 64);
            assert!(
                profile
                    .source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            );
            assert!(!profile.source_path.is_empty());
            assert!(!profile.source_symbol.is_empty());
            source_count += profile.source_count();
        }
        assert_eq!(source_count, 47);
    }

    #[test]
    fn exact_rpc_limit_and_required_fields_are_enforced() {
        let principal = LegacyProtectedMediaPrincipalV1 {
            class: "session".into(),
            actor_id: Some("user-1".into()),
            tenant_id: None,
            credential_kind: "session_token".into(),
            credential_subject_id: Some("00000000-0000-4000-8000-000000000001".into()),
            credential_key_version: Some(1),
            credential_digest: Some("a".repeat(64)),
            policy_proofs: vec![LegacyProtectedMediaPolicyProofV1 {
                target_id: "video-1".into(),
                kind: "owner_bypass".into(),
                subject_id: "00000000-0000-4000-8000-000000000002".into(),
                revision: 0,
                audit_digest: "c".repeat(64),
            }],
            entitlement_binding: None,
        };
        let mut valid = LegacyProtectedMediaEnvelopeV1 {
            source_operation_id: "cap-v1-aa2bd4c3be69ed42".into(),
            principal: principal.clone(),
            execution_key: "rpc-1".into(),
            replay_origin: LegacyProtectedMediaReplayOriginV1::Caller,
            parent_family: None,
            parent_receipt_id: None,
            parent_request_digest: None,
            parent_authority_binding_digest: None,
            authority_binding_digest: "b".repeat(64),
            payload: json!(["video-1"]),
            sealed_request_ref: None,
            sealed_request_digest: None,
        };
        valid.authority_binding_digest = legacy_protected_media_authority_binding_digest(
            legacy_protected_media_profile(&valid.source_operation_id).expect("profile"),
            &valid.principal,
            &valid.payload,
        )
        .expect("canonical authority binding");
        assert!(validate_legacy_protected_media_envelope(&valid).is_ok());
        let too_many = LegacyProtectedMediaEnvelopeV1 {
            payload: Value::Array((0..51).map(|i| json!(format!("video-{i}"))).collect()),
            ..valid
        };
        assert_eq!(
            validate_legacy_protected_media_envelope(&too_many),
            Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload)
        );

        let missing = LegacyProtectedMediaEnvelopeV1 {
            source_operation_id: "cap-v1-320876fa0aec77cb".into(),
            principal,
            execution_key: "process-1".into(),
            replay_origin: LegacyProtectedMediaReplayOriginV1::Caller,
            parent_family: None,
            parent_receipt_id: None,
            parent_request_digest: None,
            parent_authority_binding_digest: None,
            authority_binding_digest: "b".repeat(64),
            payload: json!({"videoId":"v", "userId":"u"}),
            sealed_request_ref: None,
            sealed_request_digest: None,
        };
        assert_eq!(
            validate_legacy_protected_media_envelope(&missing),
            Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload)
        );
    }

    #[test]
    fn canonical_receipt_material_never_contains_credentials() {
        let principal = LegacyProtectedMediaPrincipalV1 {
            class: "internal_service".into(),
            actor_id: None,
            tenant_id: None,
            credential_kind: "service_secret".into(),
            credential_subject_id: Some("MEDIA_SERVER_WEBHOOK_SECRET.v1".into()),
            credential_key_version: Some(1),
            credential_digest: Some(legacy_protected_media_credential_digest("secret")),
            policy_proofs: Vec::new(),
            entitlement_binding: None,
        };
        let digest = principal.digest();
        assert_eq!(digest.len(), 64);
        assert!(!digest.contains("secret"));
    }

    #[test]
    fn workflow_requires_separate_parent_authority_binding() {
        let envelope = LegacyProtectedMediaEnvelopeV1 {
            source_operation_id: "cap-v1-6d73e4dfdca61f06".into(),
            principal: LegacyProtectedMediaPrincipalV1 {
                class: "parent_derived".into(),
                actor_id: Some("00000000-0000-4000-8000-000000000001".into()),
                tenant_id: None,
                credential_kind: "session_token".into(),
                credential_subject_id: Some("00000000-0000-4000-8000-000000000002".into()),
                credential_key_version: Some(1),
                credential_digest: Some("a".repeat(64)),
                policy_proofs: vec![LegacyProtectedMediaPolicyProofV1 {
                    target_id: "video-1".into(),
                    kind: "owner_bypass".into(),
                    subject_id: "00000000-0000-4000-8000-000000000003".into(),
                    revision: 1,
                    audit_digest: "b".repeat(64),
                }],
                entitlement_binding: None,
            },
            execution_key: "workflow:parent".into(),
            replay_origin: LegacyProtectedMediaReplayOriginV1::Workflow,
            parent_family: Some("protected_media".into()),
            parent_receipt_id: Some("00000000-0000-4000-8000-000000000004".into()),
            parent_request_digest: Some("c".repeat(64)),
            parent_authority_binding_digest: None,
            authority_binding_digest: "d".repeat(64),
            payload: json!({
                "videoId":"video-1",
                "userId":"00000000-0000-4000-8000-000000000001",
                "rawFileKey":"owner/video/raw.mp4",
                "bucketId":"bucket-1"
            }),
            sealed_request_ref: None,
            sealed_request_digest: None,
        };
        assert_eq!(
            validate_legacy_protected_media_envelope(&envelope),
            Err(LegacyProtectedMediaValidationErrorV1::InvalidPayload)
        );
    }

    #[test]
    fn route_matcher_accepts_exact_dynamic_shapes_only() {
        assert_eq!(
            legacy_protected_media_route_profile(
                "POST",
                "/media-server/video/process/job-1/cancel"
            )
            .map(|profile| profile.operation_id),
            Some("cap-v1-fc2e2bd0d28ffbf3")
        );
        assert!(
            legacy_protected_media_route_profile(
                "POST",
                "/media-server/video/process/job-1/cancel/extra"
            )
            .is_none()
        );
        assert!(legacy_protected_media_route_profile("GET", "/api/video/previewer").is_none());
    }
}
