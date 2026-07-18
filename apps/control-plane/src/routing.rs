#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    Production,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPolicy {
    pub deployment: Deployment,
    pub public_host: String,
}

impl HostPolicy {
    pub fn new(deployment: Deployment, public_host: impl Into<String>) -> Option<Self> {
        let public_host = public_host.into().to_ascii_lowercase();
        if !valid_dns_name(&public_host) && public_host != "localhost" {
            return None;
        }
        Some(Self {
            deployment,
            public_host,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRequestTarget {
    pub scheme: String,
    pub authority: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRejection {
    MalformedTarget,
    InsecureScheme,
    UnexpectedHost,
    HostHeaderMismatch,
}

pub fn parse_raw_request_target(url: &str) -> Result<RawRequestTarget, HostRejection> {
    if url.len() > 8_192 || url.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(HostRejection::MalformedTarget);
    }
    let (scheme, remainder) = url
        .split_once("://")
        .ok_or(HostRejection::MalformedTarget)?;
    if !matches!(scheme, "https" | "http") {
        return Err(HostRejection::MalformedTarget);
    }
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    if authority.is_empty()
        || authority.len() > 255
        || authority.contains('@')
        || authority.contains('\\')
        || authority.contains('%')
        || !authority.is_ascii()
    {
        return Err(HostRejection::MalformedTarget);
    }

    let suffix = &remainder[authority_end..];
    let raw_path = if suffix.starts_with('/') {
        suffix.split(['?', '#']).next().unwrap_or("/")
    } else {
        "/"
    };
    if suffix.contains('#') {
        return Err(HostRejection::MalformedTarget);
    }
    Ok(RawRequestTarget {
        scheme: scheme.to_owned(),
        authority: authority.to_ascii_lowercase(),
        path: raw_path.to_owned(),
    })
}

pub fn validate_host(
    target: &RawRequestTarget,
    host_header: Option<&str>,
    policy: &HostPolicy,
) -> Result<(), HostRejection> {
    let host_header = host_header.ok_or(HostRejection::HostHeaderMismatch)?;
    if host_header.len() > 255
        || !host_header.is_ascii()
        || host_header.contains('@')
        || host_header.contains('\\')
        || host_header.contains('%')
    {
        return Err(HostRejection::HostHeaderMismatch);
    }
    let host_header = host_header.to_ascii_lowercase();
    if host_header != target.authority {
        return Err(HostRejection::HostHeaderMismatch);
    }

    match policy.deployment {
        Deployment::Production => {
            if target.scheme != "https" {
                return Err(HostRejection::InsecureScheme);
            }
            if target.authority != policy.public_host {
                return Err(HostRejection::UnexpectedHost);
            }
        }
        Deployment::Local => {
            if !matches!(target.scheme.as_str(), "http" | "https")
                || !valid_local_authority(&target.authority, &policy.public_host)
            {
                return Err(HostRejection::UnexpectedHost);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    LegacyRoot,
    LegacyHealth,
    LegacyMediaServerRoot,
    LegacyApiStatus,
    LegacyChangelog,
    LegacyChangelogStatus,
    LegacyDownload,
    LegacyPlaylist,
    LegacyStorageObject,
    LegacyMultipartAbort,
    LegacyMultipartComplete,
    LegacyMultipartInitiate,
    LegacyMultipartPresignPart,
    LegacyRecordingComplete,
    LegacySignedUpload,
    LegacySignedUploadBatch,
    LegacyMobileSessionConfig,
    LegacyMobileEmailSessionRequest,
    LegacyMobileEmailSessionVerify,
    LegacyMobileSessionRequest,
    LegacyMobileSessionRevoke,
    LegacyMobileUploadCreate,
    LegacyMobileUploadComplete { video_id: String },
    LegacyMobileUploadProgress { video_id: String },
    LegacyMobileBootstrap,
    LegacyMobileCaps,
    LegacyMobileCap { video_id: String },
    LegacyMobileCapDownload { video_id: String },
    LegacyMobileCapPlayback { video_id: String },
    LegacyMobileFolders,
    LegacyMobileCapPassword { video_id: String },
    LegacyMobileCapSharing { video_id: String },
    LegacyMobileCapTitle { video_id: String },
    LegacyMobileCapComments { video_id: String },
    LegacyMobileCapReactions { video_id: String },
    LegacyMobileComment { comment_id: String },
    LegacyWebCommentDelete,
    LegacyAnalytics,
    LegacyAnalyticsTrack,
    LegacyDashboardAnalytics,
    LegacyVideoMetadata,
    LegacyVideoAnalytics,
    LegacyVideoDomainInfo,
    LegacyVideoDelete,
    LegacyVideoOg,
    LegacyRetryTranscription { video_id: String },
    LegacyProtectedMedia,
    LegacyProtectedIntegration { operation_id: &'static str },
    LegacyProtectedBillingAuth,
    LegacyEffectRpc,
    LegacyUserName,
    LegacyInviteAccept,
    LegacyInviteDecline,
    LegacyExtensionAuthStart,
    LegacyExtensionAuthApprove,
    LegacyExtensionAuthRevoke,
    LegacyExtensionBootstrap,
    LegacyExtensionInstantCreate,
    LegacyExtensionInstantProgress,
    LegacyExtensionInstantDelete { video_id: String },
    LegacyNotifications,
    LegacyNotificationPreferences,
    LegacyDesktopSessionRequest,
    LegacyDesktopOrgCustomDomain,
    LegacyDesktopOrganizations,
    LegacyDesktopOrganizationBranding { organization_id: String },
    LegacyDesktopStorageSetActive,
    LegacyDesktopUserProfile,
    LegacyDesktopVideoDelete,
    LegacyDesktopVideoProgress,
    LegacyDeveloperStorageCron,
    LegacyDeveloperMultipartAbort,
    LegacyDeveloperMultipartComplete,
    LegacyDeveloperMultipartInitiate,
    LegacyDeveloperMultipartPresign,
    LegacyDeveloperVideoCreate,
    LegacyDeveloperUsage,
    LegacyDeveloperVideos,
    LegacyDeveloperVideo { video_id: String },
    LegacyDeveloperVideoStatus { video_id: String },
    Discovery,
    Capabilities,
    ApiHealth,
    PublicShare { share_id: String },
    PublicMedia { share_id: String },
    PublicCollaborationGrant { share_id: String },
    PublicComments { share_id: String },
    PublicTranscript { share_id: String },
    PublicAnalyticsConsent { share_id: String },
    PublicAnalyticsEvents { share_id: String },
    BrowserAuthLogin,
    BrowserAuthSignup,
    BrowserAuthRecovery,
    BrowserAuthVerify,
    BrowserAuthLogout,
    AuthenticatedWebWorkspace { surface: String },
    AuthenticatedWebAction { action: String },
    AuthenticatedWebCompatibilityAction { operation_id: String },
    StorageGrantCreate,
    StorageGrantRevoke { grant_id: String },
    StorageGrantRead { tenant_id: String, grant_id: String },
    VideoCreate,
    VideoPrivacy { video_id: String },
    VideoTranscript { video_id: String },
    UploadIntent,
    UploadStatus { upload_id: String },
    UploadContent { upload_id: String },
    UploadFinalize { upload_id: String },
    UploadMultipart { upload_id: String },
    UploadMultipartPart { upload_id: String, part_number: u16 },
    UploadMultipartComplete { upload_id: String },
    InstantFinalize { session_id: String },
    MediaJobCreate,
    MediaJobStatus { job_id: String },
    MediaJobCancel { job_id: String },
    WorkerMediaJobClaim,
    WorkerMediaJobSource { job_id: String },
    WorkerMediaJobOutput { job_id: String },
    WorkerMediaJobSourceOrdinal { job_id: String, ordinal: u16 },
    WorkerMediaJobOutputOrdinal { job_id: String, ordinal: u16 },
    WorkerMediaJobHeartbeat { job_id: String },
    WorkerMediaJobProgress { job_id: String },
    WorkerMediaJobComplete { job_id: String },
    WorkerMediaJobFail { job_id: String },
    AuthorityStatus,
    CutoverStatus { tenant_id: String, domain: String },
    CutoverTransition { tenant_id: String, domain: String },
    CutoverReplayPause { tenant_id: String, domain: String },
    CutoverReplayResume { tenant_id: String, domain: String },
    CutoverSignal { tenant_id: String, domain: String },
    CutoverShadowObservation { tenant_id: String, domain: String },
    LocalRepositoryConformance,
    LocalAuthRepositoryConformance,
    LocalOrganizationRepositoryConformance,
    LocalR2StorageConformance,
    InvalidApiPath,
    UnknownApi,
    NotApi,
}

pub fn classify_raw_path(path: &str) -> Route {
    if path == "/__frame/local/r2-storage-conformance" {
        return Route::LocalR2StorageConformance;
    }
    if path == "/__frame/local/organization-repository-conformance" {
        return Route::LocalOrganizationRepositoryConformance;
    }
    if path == "/__frame/local/auth-repository-conformance" {
        return Route::LocalAuthRepositoryConformance;
    }
    if path == "/__frame/local/repository-conformance" {
        return Route::LocalRepositoryConformance;
    }
    if path == "/" {
        return Route::LegacyRoot;
    }
    if path == "/health" {
        return Route::LegacyHealth;
    }
    if path == "/media-server" {
        return Route::LegacyMediaServerRoot;
    }
    if path.starts_with("/media-server/") {
        return if !invalid_api_path(path) && protected_media_route_shape(path) {
            Route::LegacyProtectedMedia
        } else {
            Route::NotApi
        };
    }
    if !invalid_api_path(path) && protected_billing_auth_route_shape(path) {
        return Route::LegacyProtectedBillingAuth;
    }
    if path.starts_with("/api\\") {
        return Route::InvalidApiPath;
    }
    if path != "/api" && !path.starts_with("/api/") {
        return Route::NotApi;
    }
    if invalid_api_path(path) {
        return Route::InvalidApiPath;
    }
    if let Some(operation_id) = protected_integration_route_operation(path) {
        return Route::LegacyProtectedIntegration { operation_id };
    }
    if protected_media_route_shape(path) {
        return Route::LegacyProtectedMedia;
    }
    match path {
        "/api" | "/api/" => Route::Discovery,
        "/api/status" => Route::LegacyApiStatus,
        "/api/changelog" => Route::LegacyChangelog,
        "/api/changelog/status" => Route::LegacyChangelogStatus,
        "/api/download" => Route::LegacyDownload,
        "/api/playlist" => Route::LegacyPlaylist,
        "/api/storage/object" => Route::LegacyStorageObject,
        "/api/upload/multipart/abort" => Route::LegacyMultipartAbort,
        "/api/upload/multipart/complete" => Route::LegacyMultipartComplete,
        "/api/upload/multipart/initiate" => Route::LegacyMultipartInitiate,
        "/api/upload/multipart/presign-part" => Route::LegacyMultipartPresignPart,
        "/api/upload/recording-complete" => Route::LegacyRecordingComplete,
        "/api/upload/signed" => Route::LegacySignedUpload,
        "/api/upload/signed/batch" => Route::LegacySignedUploadBatch,
        "/api/mobile/session/config" => Route::LegacyMobileSessionConfig,
        "/api/mobile/session/email/request" => Route::LegacyMobileEmailSessionRequest,
        "/api/mobile/session/email/verify" => Route::LegacyMobileEmailSessionVerify,
        "/api/mobile/session/request" => Route::LegacyMobileSessionRequest,
        "/api/mobile/session/revoke" => Route::LegacyMobileSessionRevoke,
        "/api/mobile/uploads" => Route::LegacyMobileUploadCreate,
        "/api/mobile/bootstrap" => Route::LegacyMobileBootstrap,
        "/api/mobile/caps" => Route::LegacyMobileCaps,
        "/api/mobile/folders" => Route::LegacyMobileFolders,
        "/api/analytics" => Route::LegacyAnalytics,
        "/api/analytics/track" => Route::LegacyAnalyticsTrack,
        "/api/dashboard/analytics" => Route::LegacyDashboardAnalytics,
        "/api/video/comment/delete" => Route::LegacyWebCommentDelete,
        "/api/video/metadata" => Route::LegacyVideoMetadata,
        "/api/video/analytics" => Route::LegacyVideoAnalytics,
        "/api/video/domain-info" => Route::LegacyVideoDomainInfo,
        "/api/video/delete" => Route::LegacyVideoDelete,
        "/api/video/og" => Route::LegacyVideoOg,
        "/api/erpc" => Route::LegacyEffectRpc,
        "/api/settings/user/name" => Route::LegacyUserName,
        "/api/invite/accept" => Route::LegacyInviteAccept,
        "/api/invite/decline" => Route::LegacyInviteDecline,
        "/api/extension/auth/start" => Route::LegacyExtensionAuthStart,
        "/api/extension/auth/approve" => Route::LegacyExtensionAuthApprove,
        "/api/extension/auth/revoke" => Route::LegacyExtensionAuthRevoke,
        "/api/extension/bootstrap" => Route::LegacyExtensionBootstrap,
        "/api/extension/instant-recordings" => Route::LegacyExtensionInstantCreate,
        "/api/extension/instant-recordings/progress" => Route::LegacyExtensionInstantProgress,
        "/api/notifications" => Route::LegacyNotifications,
        "/api/notifications/preferences" => Route::LegacyNotificationPreferences,
        "/api/desktop/session/request" => Route::LegacyDesktopSessionRequest,
        "/api/desktop/org-custom-domain" => Route::LegacyDesktopOrgCustomDomain,
        "/api/desktop/organizations" => Route::LegacyDesktopOrganizations,
        "/api/desktop/storage/set-active" => Route::LegacyDesktopStorageSetActive,
        "/api/desktop/user/profile" => Route::LegacyDesktopUserProfile,
        "/api/desktop/video/delete" => Route::LegacyDesktopVideoDelete,
        "/api/desktop/video/progress" => Route::LegacyDesktopVideoProgress,
        "/api/cron/developer-storage" => Route::LegacyDeveloperStorageCron,
        "/api/developer/sdk/v1/upload/multipart/abort" => Route::LegacyDeveloperMultipartAbort,
        "/api/developer/sdk/v1/upload/multipart/complete" => {
            Route::LegacyDeveloperMultipartComplete
        }
        "/api/developer/sdk/v1/upload/multipart/initiate" => {
            Route::LegacyDeveloperMultipartInitiate
        }
        "/api/developer/sdk/v1/upload/multipart/presign-part" => {
            Route::LegacyDeveloperMultipartPresign
        }
        "/api/developer/sdk/v1/videos/create" => Route::LegacyDeveloperVideoCreate,
        "/api/developer/v1/usage" => Route::LegacyDeveloperUsage,
        "/api/developer/v1/videos" => Route::LegacyDeveloperVideos,
        "/api/v1" | "/api/v1/" => Route::Capabilities,
        "/api/v1/health" => Route::ApiHealth,
        "/api/v1/videos" => Route::VideoCreate,
        "/api/v1/uploads/intents" => Route::UploadIntent,
        "/api/v1/media-jobs" => Route::MediaJobCreate,
        "/api/v1/storage/grants" => Route::StorageGrantCreate,
        "/api/v1/worker/media-jobs/claim" => Route::WorkerMediaJobClaim,
        "/api/v1/operations/authority" => Route::AuthorityStatus,
        "/api/v1/web/auth/login" => Route::BrowserAuthLogin,
        "/api/v1/web/auth/signup" => Route::BrowserAuthSignup,
        "/api/v1/web/auth/recovery" => Route::BrowserAuthRecovery,
        "/api/v1/web/auth/verify" => Route::BrowserAuthVerify,
        "/api/v1/web/auth/logout" => Route::BrowserAuthLogout,
        _ => dynamic_route(path),
    }
}

/// Resolve only the 20 source-pinned provider-backed integration routes.
///
/// The two parameterized Cap routes are matched segment-by-segment. Keeping
/// them here avoids admitting suffixes or lookalike prefixes into a protected
/// carrier before its method and authentication checks run.
fn protected_integration_route_operation(path: &str) -> Option<&'static str> {
    let operation_id = match path {
        "/api/desktop/feedback" => "cap-v1-30b7af7323aa2c37",
        "/api/desktop/logs" => "cap-v1-dfbbc4c0b56179d1",
        "/api/desktop/plan" => "cap-v1-10180c4650ffde88",
        "/api/desktop/s3/config" => "cap-v1-9d91d42d52472a83",
        "/api/desktop/s3/config/delete" => "cap-v1-58ec99a456d61373",
        "/api/desktop/s3/config/get" => "cap-v1-c6214b213eaa2360",
        "/api/desktop/s3/config/test" => "cap-v1-2d1396c2f68299f9",
        "/api/desktop/storage/google-drive/callback" => "cap-v1-49531a09fd9433e7",
        "/api/desktop/storage/google-drive/connect" => "cap-v1-679e4241ef5e7383",
        "/api/desktop/storage/google-drive/disconnect" => "cap-v1-5ef3570390b8c80c",
        "/api/desktop/storage/google-drive/test" => "cap-v1-8d5930c717418665",
        "/api/desktop/storage/integrations" => "cap-v1-0b36c9acda9bd6a2",
        "/api/desktop/user/profile/image" => "cap-v1-2e4ee222efc29606",
        "/api/desktop/video/create" => "cap-v1-60f863b2cb19353f",
        "/api/loom/video" => "cap-v1-f0a00e93ab606a52",
        "/api/mobile/user/active-organization" => "cap-v1-05776c542380771e",
        "/api/tools/loom-download" => "cap-v1-221a713f60d7528f",
        "/api/webhooks/media-server/progress" => "cap-v1-17d69edf5d3b06bb",
        _ => {
            return match path.split('/').collect::<Vec<_>>().as_slice() {
                ["", "api", "releases", "tauri", version, target, arch]
                    if [version, target, arch]
                        .iter()
                        .all(|segment| !segment.is_empty()) =>
                {
                    Some("cap-v1-8a1e6c87b4426f93")
                }
                ["", "api", "webhooks", "media-server", "multipart", action]
                    if !action.is_empty() =>
                {
                    Some("cap-v1-5af545d5d20508bd")
                }
                _ => None,
            };
        }
    };
    Some(operation_id)
}

fn protected_media_route_shape(path: &str) -> bool {
    if matches!(
        path,
        "/api/cron/finalize-stale-desktop-segments"
            | "/api/thumbnail"
            | "/api/video/ai"
            | "/api/video/preview"
            | "/api/video/transcribe/status"
            | "/media-server/audio/check"
            | "/media-server/audio/convert"
            | "/media-server/audio/extract"
            | "/media-server/audio/status"
            | "/media-server/health"
            | "/media-server/video/cleanup"
            | "/media-server/video/convert"
            | "/media-server/video/edit"
            | "/media-server/video/force-cleanup"
            | "/media-server/video/mux-segments"
            | "/media-server/video/probe"
            | "/media-server/video/process"
            | "/media-server/video/status"
            | "/media-server/video/thumbnail"
    ) {
        return true;
    }
    matches!(
        path.split('/').collect::<Vec<_>>().as_slice(),
        ["", "api", "videos", id, "retry-ai"]
            | ["", "media-server", "video", "process", id, "cancel"]
            | ["", "media-server", "video", "process", id, "status"]
            if !id.is_empty()
    )
}

/// Resolve the nine exact authentication/billing route shapes. NextAuth owns
/// a bounded wildcard below `/api/auth/`, but only for the source-pinned
/// NextAuth entrypoint names; lookalike API prefixes remain closed.
fn protected_billing_auth_route_shape(path: &str) -> bool {
    if matches!(
        path,
        "/api/desktop/subscribe"
            | "/api/developer/credits/checkout"
            | "/api/settings/billing/guest-checkout"
            | "/api/settings/billing/manage"
            | "/api/settings/billing/subscribe"
            | "/api/settings/billing/usage"
            | "/api/webhooks/stripe"
            | "/api/commercial/checkout"
    ) {
        return true;
    }
    path.strip_prefix("/api/auth/").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 256
            && !suffix.contains("..")
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_'))
            && matches!(
                suffix.split('/').next().unwrap_or_default(),
                "callback"
                    | "csrf"
                    | "error"
                    | "providers"
                    | "session"
                    | "signin"
                    | "signout"
                    | "verify-request"
            )
    })
}

pub fn valid_repository_conformance_target(target: &RawRequestTarget) -> bool {
    target.scheme == "http"
        && target
            .authority
            .strip_prefix("127.0.0.1:")
            .and_then(|port| port.parse::<u16>().ok())
            .is_some_and(|port| port != 0)
}

fn dynamic_route(path: &str) -> Route {
    let segments = path.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        ["", "api", "developer", "v1", "videos", video_id, "status"] if !video_id.is_empty() => {
            Route::LegacyDeveloperVideoStatus {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "developer", "v1", "videos", video_id] if !video_id.is_empty() => {
            Route::LegacyDeveloperVideo {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "videos", video_id, "retry-transcription"] if !video_id.is_empty() => {
            Route::LegacyRetryTranscription {
                video_id: (*video_id).to_owned(),
            }
        }
        [
            "",
            "api",
            "desktop",
            "organizations",
            organization_id,
            "branding",
        ] if !organization_id.is_empty() => Route::LegacyDesktopOrganizationBranding {
            organization_id: (*organization_id).to_owned(),
        },
        ["", "api", "extension", "instant-recordings", video_id] if !video_id.is_empty() => {
            Route::LegacyExtensionInstantDelete {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "uploads", video_id, "complete"] if !video_id.is_empty() => {
            Route::LegacyMobileUploadComplete {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "uploads", video_id, "progress"] if !video_id.is_empty() => {
            Route::LegacyMobileUploadProgress {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id, "download"] if !video_id.is_empty() => {
            Route::LegacyMobileCapDownload {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id, "playback"] if !video_id.is_empty() => {
            Route::LegacyMobileCapPlayback {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id, "password"] if !video_id.is_empty() => {
            Route::LegacyMobileCapPassword {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id, "sharing"] if !video_id.is_empty() => {
            Route::LegacyMobileCapSharing {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id, "title"] if !video_id.is_empty() => {
            Route::LegacyMobileCapTitle {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id, "comments"] if !video_id.is_empty() => {
            Route::LegacyMobileCapComments {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id, "reactions"] if !video_id.is_empty() => {
            Route::LegacyMobileCapReactions {
                video_id: (*video_id).to_owned(),
            }
        }
        ["", "api", "mobile", "comments", comment_id] if !comment_id.is_empty() => {
            Route::LegacyMobileComment {
                comment_id: (*comment_id).to_owned(),
            }
        }
        ["", "api", "mobile", "caps", video_id] if !video_id.is_empty() => Route::LegacyMobileCap {
            video_id: (*video_id).to_owned(),
        },
        ["", "api", "v1", "public", "shares", share_id] => Route::PublicShare {
            share_id: (*share_id).to_owned(),
        },
        ["", "api", "v1", "public", "shares", share_id, "media"] => Route::PublicMedia {
            share_id: (*share_id).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "public",
            "shares",
            share_id,
            "collaboration-grants",
        ] => Route::PublicCollaborationGrant {
            share_id: (*share_id).to_owned(),
        },
        ["", "api", "v1", "public", "shares", share_id, "comments"] => Route::PublicComments {
            share_id: (*share_id).to_owned(),
        },
        ["", "api", "v1", "public", "shares", share_id, "transcript"] => Route::PublicTranscript {
            share_id: (*share_id).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "public",
            "shares",
            share_id,
            "analytics",
            "consent",
        ] => Route::PublicAnalyticsConsent {
            share_id: (*share_id).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "public",
            "shares",
            share_id,
            "analytics",
            "events",
        ] => Route::PublicAnalyticsEvents {
            share_id: (*share_id).to_owned(),
        },
        ["", "api", "v1", "web", "workspace", surface] => Route::AuthenticatedWebWorkspace {
            surface: (*surface).to_owned(),
        },
        ["", "api", "v1", "web", "actions", action] => Route::AuthenticatedWebAction {
            action: (*action).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "web",
            "compatibility-actions",
            operation_id,
        ] if !operation_id.is_empty() => Route::AuthenticatedWebCompatibilityAction {
            operation_id: (*operation_id).to_owned(),
        },
        ["", "api", "v1", "storage", "grants", grant_id] => Route::StorageGrantRevoke {
            grant_id: (*grant_id).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "storage",
            "tenants",
            tenant_id,
            "grants",
            grant_id,
        ] => Route::StorageGrantRead {
            tenant_id: (*tenant_id).to_owned(),
            grant_id: (*grant_id).to_owned(),
        },
        ["", "api", "v1", "videos", video_id, "privacy"] => Route::VideoPrivacy {
            video_id: (*video_id).to_owned(),
        },
        ["", "api", "v1", "videos", video_id, "transcript"] => Route::VideoTranscript {
            video_id: (*video_id).to_owned(),
        },
        ["", "api", "v1", "uploads", upload_id] => Route::UploadStatus {
            upload_id: (*upload_id).to_owned(),
        },
        ["", "api", "v1", "uploads", upload_id, "content"] => Route::UploadContent {
            upload_id: (*upload_id).to_owned(),
        },
        ["", "api", "v1", "uploads", upload_id, "finalize"] => Route::UploadFinalize {
            upload_id: (*upload_id).to_owned(),
        },
        ["", "api", "v1", "uploads", upload_id, "multipart"] => Route::UploadMultipart {
            upload_id: (*upload_id).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "uploads",
            upload_id,
            "multipart",
            "parts",
            part_number,
        ] => match part_number.parse::<u16>() {
            Ok(parsed_part_number)
                if (1..=10_000).contains(&parsed_part_number)
                    && parsed_part_number.to_string() == *part_number =>
            {
                Route::UploadMultipartPart {
                    upload_id: (*upload_id).to_owned(),
                    part_number: parsed_part_number,
                }
            }
            _ => Route::InvalidApiPath,
        },
        [
            "",
            "api",
            "v1",
            "uploads",
            upload_id,
            "multipart",
            "complete",
        ] => Route::UploadMultipartComplete {
            upload_id: (*upload_id).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "instant-recordings",
            session_id,
            "finalize",
        ] => Route::InstantFinalize {
            session_id: (*session_id).to_owned(),
        },
        ["", "api", "v1", "media-jobs", job_id] => Route::MediaJobStatus {
            job_id: (*job_id).to_owned(),
        },
        ["", "api", "v1", "media-jobs", job_id, "cancel"] => Route::MediaJobCancel {
            job_id: (*job_id).to_owned(),
        },
        ["", "api", "v1", "worker", "media-jobs", job_id, "source"] => {
            Route::WorkerMediaJobSource {
                job_id: (*job_id).to_owned(),
            }
        }
        ["", "api", "v1", "worker", "media-jobs", job_id, "output"] => {
            Route::WorkerMediaJobOutput {
                job_id: (*job_id).to_owned(),
            }
        }
        [
            "",
            "api",
            "v1",
            "worker",
            "media-jobs",
            job_id,
            "sources",
            ordinal,
        ] => canonical_worker_ordinal(ordinal).map_or(Route::UnknownApi, |ordinal| {
            Route::WorkerMediaJobSourceOrdinal {
                job_id: (*job_id).to_owned(),
                ordinal,
            }
        }),
        [
            "",
            "api",
            "v1",
            "worker",
            "media-jobs",
            job_id,
            "outputs",
            ordinal,
        ] => canonical_worker_ordinal(ordinal).map_or(Route::UnknownApi, |ordinal| {
            Route::WorkerMediaJobOutputOrdinal {
                job_id: (*job_id).to_owned(),
                ordinal,
            }
        }),
        ["", "api", "v1", "worker", "media-jobs", job_id, "heartbeat"] => {
            Route::WorkerMediaJobHeartbeat {
                job_id: (*job_id).to_owned(),
            }
        }
        ["", "api", "v1", "worker", "media-jobs", job_id, "progress"] => {
            Route::WorkerMediaJobProgress {
                job_id: (*job_id).to_owned(),
            }
        }
        ["", "api", "v1", "worker", "media-jobs", job_id, "complete"] => {
            Route::WorkerMediaJobComplete {
                job_id: (*job_id).to_owned(),
            }
        }
        ["", "api", "v1", "worker", "media-jobs", job_id, "fail"] => Route::WorkerMediaJobFail {
            job_id: (*job_id).to_owned(),
        },
        ["", "api", "v1", "operations", "cutover", tenant_id, domain] => Route::CutoverStatus {
            tenant_id: (*tenant_id).to_owned(),
            domain: (*domain).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "operations",
            "cutover",
            tenant_id,
            domain,
            "transition",
        ] => Route::CutoverTransition {
            tenant_id: (*tenant_id).to_owned(),
            domain: (*domain).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "operations",
            "cutover",
            tenant_id,
            domain,
            "replay",
            "pause",
        ] => Route::CutoverReplayPause {
            tenant_id: (*tenant_id).to_owned(),
            domain: (*domain).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "operations",
            "cutover",
            tenant_id,
            domain,
            "replay",
            "resume",
        ] => Route::CutoverReplayResume {
            tenant_id: (*tenant_id).to_owned(),
            domain: (*domain).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "operations",
            "cutover",
            tenant_id,
            domain,
            "signals",
        ] => Route::CutoverSignal {
            tenant_id: (*tenant_id).to_owned(),
            domain: (*domain).to_owned(),
        },
        [
            "",
            "api",
            "v1",
            "operations",
            "cutover",
            tenant_id,
            domain,
            "shadow-observations",
        ] => Route::CutoverShadowObservation {
            tenant_id: (*tenant_id).to_owned(),
            domain: (*domain).to_owned(),
        },
        _ => Route::UnknownApi,
    }
}

fn canonical_worker_ordinal(value: &str) -> Option<u16> {
    let ordinal = value.parse::<u16>().ok().filter(|ordinal| *ordinal < 64)?;
    (ordinal.to_string() == value).then_some(ordinal)
}

fn invalid_api_path(path: &str) -> bool {
    path.len() > 2_048
        || !path.is_ascii()
        || path.bytes().any(|byte| byte.is_ascii_control())
        || path.contains('%')
        || path.contains(';')
        || path.contains('\\')
        || path.contains("//")
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
}

fn valid_dns_name(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn valid_local_authority(authority: &str, configured_host: &str) -> bool {
    let (host, port) = if authority.starts_with('[') {
        let Some(close) = authority.find(']') else {
            return false;
        };
        let host = &authority[..=close];
        let suffix = &authority[close + 1..];
        let port = suffix.strip_prefix(':');
        if !suffix.is_empty() && port.is_none() {
            return false;
        }
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port))
                if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
            {
                (host, Some(port))
            }
            _ => (authority, None),
        }
    };
    if port.is_some_and(|value| value.parse::<u16>().is_err()) {
        return false;
    }
    host == configured_host || matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn production() -> HostPolicy {
        HostPolicy::new(Deployment::Production, "frame.engmanager.xyz").expect("policy")
    }

    #[test]
    fn raw_target_keeps_query_out_of_path_and_rejects_ambiguous_authorities() {
        let target = parse_raw_request_target("https://frame.engmanager.xyz/api?next=/apix")
            .expect("target");
        assert_eq!(target.path, "/api");
        assert_eq!(target.authority, "frame.engmanager.xyz");
        assert!(parse_raw_request_target("https://evil@frame.engmanager.xyz/api").is_err());
        assert!(parse_raw_request_target("https://frame.engmanager.xyz/api#fragment").is_err());
        assert!(parse_raw_request_target("javascript://frame.engmanager.xyz/api").is_err());
    }

    #[test]
    fn production_host_is_exact_https_and_matches_the_request_header() {
        let target = parse_raw_request_target("https://frame.engmanager.xyz/api").expect("target");
        assert_eq!(
            validate_host(&target, Some("frame.engmanager.xyz"), &production()),
            Ok(())
        );
        assert_eq!(
            validate_host(&target, Some("evil.example"), &production()),
            Err(HostRejection::HostHeaderMismatch)
        );
        let port =
            parse_raw_request_target("https://frame.engmanager.xyz:443/api").expect("target");
        assert_eq!(
            validate_host(&port, Some("frame.engmanager.xyz:443"), &production()),
            Err(HostRejection::UnexpectedHost)
        );
        let insecure = parse_raw_request_target("http://frame.engmanager.xyz/api").expect("target");
        assert_eq!(
            validate_host(&insecure, Some("frame.engmanager.xyz"), &production()),
            Err(HostRejection::InsecureScheme)
        );
    }

    #[test]
    fn local_policy_preserves_wranger_health_with_loopback_ports() {
        let policy = HostPolicy::new(Deployment::Local, "localhost").expect("policy");
        for url in [
            "http://localhost:8787/health",
            "http://127.0.0.1:8787/health",
            "http://[::1]:8787/health",
        ] {
            let target = parse_raw_request_target(url).expect("target");
            assert_eq!(
                validate_host(&target, Some(&target.authority), &policy),
                Ok(())
            );
        }
    }

    #[test]
    fn repository_conformance_route_is_exact_and_requires_ipv4_loopback() {
        assert_eq!(
            classify_raw_path("/__frame/local/repository-conformance"),
            Route::LocalRepositoryConformance
        );
        for path in [
            "/__frame/local/repository-conformance/",
            "/__frame/local/repository-conformance%2f",
            "/__frame/local/repository-conformance/reads",
        ] {
            assert_eq!(classify_raw_path(path), Route::NotApi);
        }
        let allowed =
            parse_raw_request_target("http://127.0.0.1:8787/__frame/local/repository-conformance")
                .expect("target");
        assert!(valid_repository_conformance_target(&allowed));
        assert_eq!(
            classify_raw_path("/__frame/local/auth-repository-conformance"),
            Route::LocalAuthRepositoryConformance
        );
        for path in [
            "/__frame/local/auth-repository-conformance/",
            "/__frame/local/auth-repository-conformance%2f",
            "/__frame/local/auth-repository-conformance/session",
        ] {
            assert_eq!(classify_raw_path(path), Route::NotApi);
        }
        assert_eq!(
            classify_raw_path("/__frame/local/organization-repository-conformance"),
            Route::LocalOrganizationRepositoryConformance
        );
        for path in [
            "/__frame/local/organization-repository-conformance/",
            "/__frame/local/organization-repository-conformance%2f",
            "/__frame/local/organization-repository-conformance/invite",
        ] {
            assert_eq!(classify_raw_path(path), Route::NotApi);
        }
        assert_eq!(
            classify_raw_path("/__frame/local/r2-storage-conformance"),
            Route::LocalR2StorageConformance
        );
        for path in [
            "/__frame/local/r2-storage-conformance/",
            "/__frame/local/r2-storage-conformance%2f",
            "/__frame/local/r2-storage-conformance/objects",
        ] {
            assert_eq!(classify_raw_path(path), Route::NotApi);
        }
        let r2_allowed =
            parse_raw_request_target("http://127.0.0.1:8787/__frame/local/r2-storage-conformance")
                .expect("target");
        assert!(valid_repository_conformance_target(&r2_allowed));
        for denied in [
            "http://localhost:8787/__frame/local/repository-conformance",
            "https://127.0.0.1:8787/__frame/local/repository-conformance",
            "http://127.0.0.1/__frame/local/repository-conformance",
        ] {
            let target = parse_raw_request_target(denied).expect("target");
            assert!(!valid_repository_conformance_target(&target));
        }
    }

    #[test]
    fn broad_route_lookalikes_never_enter_api_handlers() {
        for path in ["/apix", "/apiary", "/%61pi", "/api%2fv1", "/API/v1"] {
            assert_eq!(classify_raw_path(path), Route::NotApi, "{path}");
        }
        for path in [
            "/api//v1",
            "/api/./v1",
            "/api/../v1",
            "/api/v1;admin",
            "/api/v1/%2e%2e/private",
            "/api\\v1",
        ] {
            assert_eq!(classify_raw_path(path), Route::InvalidApiPath, "{path}");
        }
    }

    #[test]
    fn versioned_routes_are_matched_without_router_decoding() {
        assert_eq!(classify_raw_path("/api"), Route::Discovery);
        assert_eq!(classify_raw_path("/api/status"), Route::LegacyApiStatus);
        assert_eq!(classify_raw_path("/api/changelog"), Route::LegacyChangelog);
        assert_eq!(classify_raw_path("/api/changelog/"), Route::UnknownApi);
        for (path, expected) in [
            ("/api/download", Route::LegacyDownload),
            ("/api/playlist", Route::LegacyPlaylist),
            ("/api/storage/object", Route::LegacyStorageObject),
            ("/api/upload/multipart/abort", Route::LegacyMultipartAbort),
            (
                "/api/upload/multipart/complete",
                Route::LegacyMultipartComplete,
            ),
            (
                "/api/upload/multipart/initiate",
                Route::LegacyMultipartInitiate,
            ),
            (
                "/api/upload/multipart/presign-part",
                Route::LegacyMultipartPresignPart,
            ),
            (
                "/api/upload/recording-complete",
                Route::LegacyRecordingComplete,
            ),
            ("/api/upload/signed", Route::LegacySignedUpload),
            ("/api/upload/signed/batch", Route::LegacySignedUploadBatch),
        ] {
            assert_eq!(classify_raw_path(path), expected, "{path}");
        }
        for path in [
            "/api/download/",
            "/api/playlist/",
            "/api/storage/object/",
            "/api/upload/multipart//abort",
            "/api/upload/multipart/complete/",
            "/api/upload/signed/%62atch",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "core-storage lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/notifications"),
            Route::LegacyNotifications
        );
        assert_eq!(
            classify_raw_path("/api/notifications/preferences"),
            Route::LegacyNotificationPreferences
        );
        assert_eq!(
            classify_raw_path("/api/video/domain-info"),
            Route::LegacyVideoDomainInfo
        );
        assert_eq!(
            classify_raw_path("/api/video/delete"),
            Route::LegacyVideoDelete
        );
        assert_eq!(classify_raw_path("/api/video/og"), Route::LegacyVideoOg);
        for path in [
            "/api/video/delete/",
            "/api/video//delete",
            "/api/video/%64elete",
            "/api/video/og/",
            "/api/video//og",
            "/api/video/%6fg",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "video lifecycle lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/videos/0123456789abcde/retry-transcription"),
            Route::LegacyRetryTranscription {
                video_id: "0123456789abcde".into(),
            }
        );
        for path in [
            "/api/videos//retry-transcription",
            "/api/videos/0123456789abcde/retry-transcription/",
            "/api/videos/%30/retry-transcription",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "transcript retry lookalike must fail closed: {path}"
            );
        }
        for (path, expected) in [
            ("/api/extension/auth/start", Route::LegacyExtensionAuthStart),
            (
                "/api/extension/auth/approve",
                Route::LegacyExtensionAuthApprove,
            ),
            (
                "/api/extension/auth/revoke",
                Route::LegacyExtensionAuthRevoke,
            ),
            ("/api/extension/bootstrap", Route::LegacyExtensionBootstrap),
            (
                "/api/extension/instant-recordings",
                Route::LegacyExtensionInstantCreate,
            ),
            (
                "/api/extension/instant-recordings/progress",
                Route::LegacyExtensionInstantProgress,
            ),
        ] {
            assert_eq!(classify_raw_path(path), expected, "{path}");
        }
        assert_eq!(
            classify_raw_path("/api/extension/instant-recordings/0123456789abcde"),
            Route::LegacyExtensionInstantDelete {
                video_id: "0123456789abcde".into(),
            }
        );
        for path in [
            "/api/extension/auth/start/",
            "/api/extension/auth//start",
            "/api/extension/auth/%73tart",
            "/api/extension/auth/approve/",
            "/api/extension/auth/revoke;admin",
            "/api/extension/bootstrap/",
            "/api/extension/instant-recordings/",
            "/api/extension/instant-recordings//",
            "/api/extension/instant-recordings/progress/",
            "/api/extension/instant-recordings/0123456789abcde/extra",
            "/api/extension/instant-recordings/%30",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "extension auth lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/desktop/org-custom-domain"),
            Route::LegacyDesktopOrgCustomDomain
        );
        assert_eq!(
            classify_raw_path("/api/desktop/session/request"),
            Route::LegacyDesktopSessionRequest
        );
        for (path, expected) in [
            (
                "/api/desktop/organizations",
                Route::LegacyDesktopOrganizations,
            ),
            (
                "/api/desktop/storage/set-active",
                Route::LegacyDesktopStorageSetActive,
            ),
            ("/api/desktop/user/profile", Route::LegacyDesktopUserProfile),
            ("/api/desktop/video/delete", Route::LegacyDesktopVideoDelete),
            (
                "/api/desktop/video/progress",
                Route::LegacyDesktopVideoProgress,
            ),
        ] {
            assert_eq!(classify_raw_path(path), expected, "{path}");
        }
        assert_eq!(
            classify_raw_path("/api/desktop/organizations/0123456789abcde/branding"),
            Route::LegacyDesktopOrganizationBranding {
                organization_id: "0123456789abcde".into(),
            }
        );
        for path in [
            "/api/desktop/organizations/",
            "/api/desktop/organizations//branding",
            "/api/desktop/organizations/0123456789abcde/branding/",
            "/api/desktop/storage/set-active/",
            "/api/desktop/user/profile/",
            "/api/desktop/video/delete/",
            "/api/desktop/video/progress/",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "desktop compatibility lookalike must fail closed: {path}"
            );
        }
        for path in [
            "/api/notifications/",
            "/api//notifications",
            "/api/%6eotifications",
            "/api/notifications;admin",
            "/api/notifications/preferences/",
            "/api/notifications//preferences",
            "/api/notifications/%70references",
            "/api/notifications/preferences;admin",
            "/api/desktop/org-custom-domain/",
            "/api/desktop//org-custom-domain",
            "/api/desktop/%6frg-custom-domain",
            "/api/desktop/org-custom-domain;admin",
            "/api/desktop/session/request/",
            "/api/desktop//session/request",
            "/api/desktop/session/%72equest",
            "/api/desktop/session/request;admin",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "notification preferences lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/changelog/status"),
            Route::LegacyChangelogStatus
        );
        assert_eq!(
            classify_raw_path("/api/changelog/status/"),
            Route::UnknownApi
        );
        assert_eq!(
            classify_raw_path("/api/mobile/session/config"),
            Route::LegacyMobileSessionConfig
        );
        for path in [
            "/api/mobile/session/config/",
            "/api/mobile/session/configuration",
            "/api/mobile//session/config",
            "/api/mobile/session/%63onfig",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "mobile config lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/mobile/bootstrap"),
            Route::LegacyMobileBootstrap
        );
        assert_eq!(
            classify_raw_path("/api/mobile/uploads"),
            Route::LegacyMobileUploadCreate
        );
        assert_eq!(
            classify_raw_path("/api/mobile/uploads/0123456789abcde/complete"),
            Route::LegacyMobileUploadComplete {
                video_id: "0123456789abcde".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/mobile/uploads/0123456789abcde/progress"),
            Route::LegacyMobileUploadProgress {
                video_id: "0123456789abcde".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/mobile/caps"),
            Route::LegacyMobileCaps
        );
        assert_eq!(
            classify_raw_path("/api/mobile/caps/0123456789abcde"),
            Route::LegacyMobileCap {
                video_id: "0123456789abcde".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/mobile/caps/0123456789abcde/download"),
            Route::LegacyMobileCapDownload {
                video_id: "0123456789abcde".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/mobile/caps/0123456789abcde/playback"),
            Route::LegacyMobileCapPlayback {
                video_id: "0123456789abcde".into(),
            }
        );
        for path in [
            "/api/mobile/bootstrap/",
            "/api/mobile//bootstrap",
            "/api/mobile/%62ootstrap",
            "/api/mobile/uploads/",
            "/api/mobile//uploads",
            "/api/mobile/%75ploads",
            "/api/mobile/uploads//complete",
            "/api/mobile/uploads/0123456789abcde/complete/",
            "/api/mobile/uploads/0123456789abcde/progress/",
            "/api/mobile/uploads/0123456789abcde/complete/extra",
            "/api/mobile/uploads/%30/complete",
            "/api/mobile/caps/",
            "/api/mobile/caps//download",
            "/api/mobile/caps/0123456789abcde/download/",
            "/api/mobile/caps/0123456789abcde/playback/",
            "/api/mobile/caps/%30/playback",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "mobile bootstrap/caps lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/mobile/folders"),
            Route::LegacyMobileFolders
        );
        for path in [
            "/api/mobile/folders/",
            "/api/mobile//folders",
            "/api/mobile/%66olders",
            "/api/mobile/folders;admin",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "mobile folder lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/mobile/caps/video-1/comments"),
            Route::LegacyMobileCapComments {
                video_id: "video-1".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/mobile/caps/video-1/reactions"),
            Route::LegacyMobileCapReactions {
                video_id: "video-1".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/mobile/comments/comment-1"),
            Route::LegacyMobileComment {
                comment_id: "comment-1".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/video/comment/delete"),
            Route::LegacyWebCommentDelete
        );
        assert_eq!(classify_raw_path("/api/analytics"), Route::LegacyAnalytics);
        assert_eq!(
            classify_raw_path("/api/analytics/track"),
            Route::LegacyAnalyticsTrack
        );
        assert_eq!(
            classify_raw_path("/api/dashboard/analytics"),
            Route::LegacyDashboardAnalytics
        );
        assert_eq!(
            classify_raw_path("/api/video/analytics"),
            Route::LegacyVideoAnalytics
        );
        for path in [
            "/api/analytics/",
            "/api/analytics//track",
            "/api/dashboard/analytics/",
            "/api/video/analytics/",
            "/api/%61nalytics",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "analytics lookalike must fail closed: {path}"
            );
        }
        for path in [
            "/api/mobile/caps//comments",
            "/api/mobile/caps/video-1/comments/",
            "/api/mobile/caps/video-1/comments/extra",
            "/api/mobile/caps/%76ideo-1/comments",
            "/api/mobile/caps//reactions",
            "/api/mobile/caps/video-1/reactions/",
            "/api/mobile/comments/",
            "/api/mobile/comments/comment-1/extra",
            "/api/video/comment/delete/",
            "/api/video//comment/delete",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "collaboration lookalike must fail closed: {path}"
            );
        }
        assert_eq!(classify_raw_path("/api/erpc"), Route::LegacyEffectRpc);
        for path in [
            "/api/erpc/",
            "/api//erpc",
            "/api/%65rpc",
            "/api/erpc;FolderCreate",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "Effect RPC lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/api/settings/user/name"),
            Route::LegacyUserName
        );
        for path in [
            "/api/settings/user/name/",
            "/api/settings//user/name",
            "/api/settings/user/Name",
            "/api/settings/%75ser/name",
        ] {
            assert!(
                matches!(
                    classify_raw_path(path),
                    Route::UnknownApi | Route::InvalidApiPath
                ),
                "user name lookalike must fail closed: {path}"
            );
        }
        assert_eq!(
            classify_raw_path("/media-server"),
            Route::LegacyMediaServerRoot
        );
        assert_eq!(classify_raw_path("/api/v1"), Route::Capabilities);
        assert_eq!(classify_raw_path("/api/v1/health"), Route::ApiHealth);
        assert_eq!(
            classify_raw_path("/api/v1/public/shares/018f47a6-7b1c-7f55-8f39-8f8a8690f123"),
            Route::PublicShare {
                share_id: "018f47a6-7b1c-7f55-8f39-8f8a8690f123".into()
            }
        );
        assert_eq!(
            classify_raw_path("/api/v1/public/shares/018f47a6-7b1c-7f55-8f39-8f8a8690f123/media"),
            Route::PublicMedia {
                share_id: "018f47a6-7b1c-7f55-8f39-8f8a8690f123".into()
            }
        );
        assert_eq!(classify_raw_path("/api/v2/health"), Route::UnknownApi);
        assert_eq!(classify_raw_path("/api/v1/videos"), Route::VideoCreate);
        for (path, expected) in [
            ("/api/v1/web/auth/login", Route::BrowserAuthLogin),
            ("/api/v1/web/auth/signup", Route::BrowserAuthSignup),
            ("/api/v1/web/auth/recovery", Route::BrowserAuthRecovery),
            ("/api/v1/web/auth/verify", Route::BrowserAuthVerify),
            ("/api/v1/web/auth/logout", Route::BrowserAuthLogout),
        ] {
            assert_eq!(classify_raw_path(path), expected, "{path}");
        }
        for path in [
            "/api/v1/web/auth/login/",
            "/api/v1/web/auth/verify/extra",
            "/api/v1/web/auth/Login",
            "/api/v1/web/auth",
        ] {
            assert_eq!(classify_raw_path(path), Route::UnknownApi, "{path}");
        }
        assert_eq!(
            classify_raw_path("/api/v1/web/actions/organization.spaces.create.v1"),
            Route::AuthenticatedWebAction {
                action: "organization.spaces.create.v1".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/v1/web/compatibility-actions/cap-v1-7773d3e70d1d5919"),
            Route::AuthenticatedWebCompatibilityAction {
                operation_id: "cap-v1-7773d3e70d1d5919".into(),
            }
        );
        for path in [
            "/api/v1/web/compatibility-actions",
            "/api/v1/web/compatibility-actions/",
            "/api/v1/web/compatibility-actions/cap-v1-7773d3e70d1d5919/extra",
        ] {
            assert_eq!(classify_raw_path(path), Route::UnknownApi, "{path}");
        }
        assert_eq!(
            classify_raw_path("/api/v1/storage/grants"),
            Route::StorageGrantCreate
        );
        assert_eq!(
            classify_raw_path(
                "/api/v1/storage/tenants/018f47a6-7b1c-7f55-8f39-8f8a86900001/grants/018f47a6-7b1c-7f55-8f39-8f8a86900002"
            ),
            Route::StorageGrantRead {
                tenant_id: "018f47a6-7b1c-7f55-8f39-8f8a86900001".into(),
                grant_id: "018f47a6-7b1c-7f55-8f39-8f8a86900002".into(),
            }
        );
        assert_eq!(
            classify_raw_path("/api/v1/videos/018f47a6-7b1c-7f55-8f39-8f8a86900111/privacy"),
            Route::VideoPrivacy {
                video_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into()
            }
        );
        assert_eq!(
            classify_raw_path("/api/v1/uploads/018f47a6-7b1c-7f55-8f39-8f8a86900111/content"),
            Route::UploadContent {
                upload_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into()
            }
        );
        assert_eq!(
            classify_raw_path("/api/v1/uploads/018f47a6-7b1c-7f55-8f39-8f8a86900111/finalize"),
            Route::UploadFinalize {
                upload_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into()
            }
        );
        assert_eq!(
            classify_raw_path("/api/v1/uploads/018f47a6-7b1c-7f55-8f39-8f8a86900111/multipart"),
            Route::UploadMultipart {
                upload_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into()
            }
        );
        assert_eq!(
            classify_raw_path(
                "/api/v1/uploads/018f47a6-7b1c-7f55-8f39-8f8a86900111/multipart/parts/17"
            ),
            Route::UploadMultipartPart {
                upload_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into(),
                part_number: 17,
            }
        );
        assert_eq!(
            classify_raw_path(
                "/api/v1/uploads/018f47a6-7b1c-7f55-8f39-8f8a86900111/multipart/complete"
            ),
            Route::UploadMultipartComplete {
                upload_id: "018f47a6-7b1c-7f55-8f39-8f8a86900111".into()
            }
        );
        for part_number in ["0", "01", "+1", "10001", "65536", "not-a-number"] {
            assert_eq!(
                classify_raw_path(&format!(
                    "/api/v1/uploads/018f47a6-7b1c-7f55-8f39-8f8a86900111/multipart/parts/{part_number}"
                )),
                Route::InvalidApiPath,
                "{part_number}"
            );
        }
        assert_eq!(
            classify_raw_path(
                "/api/v1/instant-recordings/018f47a6-7b1c-7f55-8f39-8f8a86900112/finalize"
            ),
            Route::InstantFinalize {
                session_id: "018f47a6-7b1c-7f55-8f39-8f8a86900112".into()
            }
        );
        assert_eq!(
            classify_raw_path(
                "/api/v1/uploads/018f47a6-7b1c-7f55-8f39-8f8a86900111/finalize/extra"
            ),
            Route::UnknownApi
        );
        let tenant_id = "018f47a6-7b1c-7f55-8f39-8f8a86900001";
        assert_eq!(
            classify_raw_path(&format!("/api/v1/operations/cutover/{tenant_id}/metadata")),
            Route::CutoverStatus {
                tenant_id: tenant_id.into(),
                domain: "metadata".into(),
            }
        );
        assert_eq!(
            classify_raw_path(&format!(
                "/api/v1/operations/cutover/{tenant_id}/metadata/transition"
            )),
            Route::CutoverTransition {
                tenant_id: tenant_id.into(),
                domain: "metadata".into(),
            }
        );
        assert_eq!(
            classify_raw_path(&format!(
                "/api/v1/operations/cutover/{tenant_id}/metadata/replay/pause"
            )),
            Route::CutoverReplayPause {
                tenant_id: tenant_id.into(),
                domain: "metadata".into(),
            }
        );
        assert_eq!(
            classify_raw_path(&format!(
                "/api/v1/operations/cutover/{tenant_id}/metadata/replay/resume"
            )),
            Route::CutoverReplayResume {
                tenant_id: tenant_id.into(),
                domain: "metadata".into(),
            }
        );
        assert_eq!(
            classify_raw_path(&format!(
                "/api/v1/operations/cutover/{tenant_id}/metadata/signals"
            )),
            Route::CutoverSignal {
                tenant_id: tenant_id.into(),
                domain: "metadata".into(),
            }
        );
        assert_eq!(
            classify_raw_path(&format!(
                "/api/v1/operations/cutover/{tenant_id}/metadata/shadow-observations"
            )),
            Route::CutoverShadowObservation {
                tenant_id: tenant_id.into(),
                domain: "metadata".into(),
            }
        );
    }

    #[test]
    fn worker_protocol_routes_are_explicit_and_segment_bounded() {
        let job_id = "018f47a6-7b1c-7f55-8f39-8f8a86900112";
        assert_eq!(
            classify_raw_path("/api/v1/worker/media-jobs/claim"),
            Route::WorkerMediaJobClaim
        );
        assert_eq!(
            classify_raw_path(&format!("/api/v1/worker/media-jobs/{job_id}/source")),
            Route::WorkerMediaJobSource {
                job_id: job_id.into()
            }
        );
        assert_eq!(
            classify_raw_path(&format!("/api/v1/worker/media-jobs/{job_id}/complete")),
            Route::WorkerMediaJobComplete {
                job_id: job_id.into()
            }
        );
        assert_eq!(
            classify_raw_path(&format!("/api/v1/worker/media-jobs/{job_id}/sources/0")),
            Route::WorkerMediaJobSourceOrdinal {
                job_id: job_id.into(),
                ordinal: 0,
            }
        );
        assert_eq!(
            classify_raw_path(&format!("/api/v1/worker/media-jobs/{job_id}/sources/17")),
            Route::WorkerMediaJobSourceOrdinal {
                job_id: job_id.into(),
                ordinal: 17,
            }
        );
        assert_eq!(
            classify_raw_path(&format!("/api/v1/worker/media-jobs/{job_id}/outputs/0")),
            Route::WorkerMediaJobOutputOrdinal {
                job_id: job_id.into(),
                ordinal: 0,
            }
        );
        for ordinal in ["00", "+0", "64", "65536"] {
            assert_eq!(
                classify_raw_path(&format!(
                    "/api/v1/worker/media-jobs/{job_id}/sources/{ordinal}"
                )),
                Route::UnknownApi
            );
        }
        assert_eq!(
            classify_raw_path(&format!("/api/v1/worker/media-jobs/{job_id}/secret")),
            Route::UnknownApi
        );
        assert_eq!(
            classify_raw_path("/api/v1/worker/media-jobs/claim/source"),
            Route::WorkerMediaJobSource {
                job_id: "claim".into()
            }
        );
    }

    #[test]
    fn protected_media_paths_are_exact_and_lookalikes_stay_closed() {
        for path in [
            "/api/cron/finalize-stale-desktop-segments",
            "/api/thumbnail",
            "/api/video/ai",
            "/api/video/preview",
            "/api/video/transcribe/status",
            "/api/videos/video-1/retry-ai",
            "/media-server/audio/check",
            "/media-server/audio/convert",
            "/media-server/audio/extract",
            "/media-server/audio/status",
            "/media-server/health",
            "/media-server/video/cleanup",
            "/media-server/video/convert",
            "/media-server/video/edit",
            "/media-server/video/force-cleanup",
            "/media-server/video/mux-segments",
            "/media-server/video/probe",
            "/media-server/video/process",
            "/media-server/video/process/job-1/cancel",
            "/media-server/video/process/job-1/status",
            "/media-server/video/status",
            "/media-server/video/thumbnail",
        ] {
            assert_eq!(
                classify_raw_path(path),
                Route::LegacyProtectedMedia,
                "{path}"
            );
        }
        for path in [
            "/api/thumbnail/",
            "/api/videos//retry-ai",
            "/api/videos/video-1/retry-ai/extra",
            "/media-server/audio/check/",
            "/media-server/video/process//status",
            "/media-server/video/process/job-1/status/extra",
            "/media-server/video/%70robe",
        ] {
            assert_ne!(
                classify_raw_path(path),
                Route::LegacyProtectedMedia,
                "{path}"
            );
        }
    }

    #[test]
    fn protected_integration_paths_resolve_exact_operation_ids() {
        let routes = [
            ("/api/desktop/feedback", "cap-v1-30b7af7323aa2c37"),
            ("/api/desktop/logs", "cap-v1-dfbbc4c0b56179d1"),
            ("/api/desktop/plan", "cap-v1-10180c4650ffde88"),
            ("/api/desktop/s3/config", "cap-v1-9d91d42d52472a83"),
            ("/api/desktop/s3/config/delete", "cap-v1-58ec99a456d61373"),
            ("/api/desktop/s3/config/get", "cap-v1-c6214b213eaa2360"),
            ("/api/desktop/s3/config/test", "cap-v1-2d1396c2f68299f9"),
            (
                "/api/desktop/storage/google-drive/callback",
                "cap-v1-49531a09fd9433e7",
            ),
            (
                "/api/desktop/storage/google-drive/connect",
                "cap-v1-679e4241ef5e7383",
            ),
            (
                "/api/desktop/storage/google-drive/disconnect",
                "cap-v1-5ef3570390b8c80c",
            ),
            (
                "/api/desktop/storage/google-drive/test",
                "cap-v1-8d5930c717418665",
            ),
            (
                "/api/desktop/storage/integrations",
                "cap-v1-0b36c9acda9bd6a2",
            ),
            ("/api/desktop/user/profile/image", "cap-v1-2e4ee222efc29606"),
            ("/api/desktop/video/create", "cap-v1-60f863b2cb19353f"),
            ("/api/loom/video", "cap-v1-f0a00e93ab606a52"),
            (
                "/api/mobile/user/active-organization",
                "cap-v1-05776c542380771e",
            ),
            (
                "/api/releases/tauri/1.2.3/universal-apple-darwin/aarch64",
                "cap-v1-8a1e6c87b4426f93",
            ),
            ("/api/tools/loom-download", "cap-v1-221a713f60d7528f"),
            (
                "/api/webhooks/media-server/multipart/complete",
                "cap-v1-5af545d5d20508bd",
            ),
            (
                "/api/webhooks/media-server/progress",
                "cap-v1-17d69edf5d3b06bb",
            ),
        ];
        for (path, operation_id) in routes {
            assert_eq!(
                classify_raw_path(path),
                Route::LegacyProtectedIntegration { operation_id },
                "{path}"
            );
        }
    }

    #[test]
    fn protected_integration_parameter_routes_reject_lookalikes() {
        for path in [
            "/api/releases/tauri/1.2.3/universal-apple-darwin",
            "/api/releases/tauri/1.2.3/universal-apple-darwin/aarch64/extra",
            "/api/releases/tauri//universal-apple-darwin/aarch64",
            "/api/webhooks/media-server/multipart",
            "/api/webhooks/media-server/multipart/complete/extra",
            "/api/webhooks/media-server/multipart/",
            "/api/webhooks/media-server/progress/extra",
        ] {
            assert!(
                !matches!(
                    classify_raw_path(path),
                    Route::LegacyProtectedIntegration { .. }
                ),
                "{path}"
            );
        }
    }

    #[test]
    fn protected_billing_auth_paths_are_exact() {
        for path in [
            "/api/auth/session",
            "/api/auth/callback/google",
            "/api/auth/verify-request",
            "/api/desktop/subscribe",
            "/api/developer/credits/checkout",
            "/api/settings/billing/guest-checkout",
            "/api/settings/billing/manage",
            "/api/settings/billing/subscribe",
            "/api/settings/billing/usage",
            "/api/webhooks/stripe",
            "/api/commercial/checkout",
        ] {
            assert_eq!(
                classify_raw_path(path),
                Route::LegacyProtectedBillingAuth,
                "{path}"
            );
        }
    }

    #[test]
    fn protected_billing_auth_lookalikes_stay_closed() {
        for path in [
            "/api/auth",
            "/api/auth/",
            "/api/auth/unknown",
            "/api/auth//session",
            "/api/auth/callback/foo..bar",
            "/api/auth/callback/foo.bar",
            "/api/auth/callback/$provider",
            "/api/desktop/subscribe/",
            "/api/developer/credits/checkout/extra",
            "/api/settings/billing/usage/extra",
            "/api/webhooks/stripe/extra",
            "/api/commercial/checkout/extra",
        ] {
            assert_ne!(
                classify_raw_path(path),
                Route::LegacyProtectedBillingAuth,
                "{path}"
            );
        }
    }
}
