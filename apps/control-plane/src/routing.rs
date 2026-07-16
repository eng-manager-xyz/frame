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
    AuthenticatedWebWorkspace { surface: String },
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
    if path.starts_with("/api\\") {
        return Route::InvalidApiPath;
    }
    if path != "/api" && !path.starts_with("/api/") {
        return Route::NotApi;
    }
    if invalid_api_path(path) {
        return Route::InvalidApiPath;
    }
    match path {
        "/api" | "/api/" => Route::Discovery,
        "/api/v1" | "/api/v1/" => Route::Capabilities,
        "/api/v1/health" => Route::ApiHealth,
        "/api/v1/videos" => Route::VideoCreate,
        "/api/v1/uploads/intents" => Route::UploadIntent,
        "/api/v1/media-jobs" => Route::MediaJobCreate,
        "/api/v1/storage/grants" => Route::StorageGrantCreate,
        "/api/v1/worker/media-jobs/claim" => Route::WorkerMediaJobClaim,
        "/api/v1/operations/authority" => Route::AuthorityStatus,
        _ => dynamic_route(path),
    }
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
}
