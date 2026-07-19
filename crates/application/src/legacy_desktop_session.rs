//! Source-pinned compatibility contract for Cap's desktop sign-in handoff.
//!
//! The source route exports either the already-authenticated browser session
//! token or a newly minted desktop API key, then hands it to the native client
//! through a loopback callback or `cap-desktop://` deep link. Frame preserves
//! those wire shapes. It deliberately narrows the source's unconstrained
//! `port` string to a decimal TCP port in `1..=65535`; that closes the source
//! open-redirect/HTML-injection edge without changing a valid desktop client.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::{Url, form_urlencoded};

pub const LEGACY_DESKTOP_SESSION_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_DESKTOP_SESSION_OPERATION_ID: &str = "cap-v1-768895bc99380850";
pub const LEGACY_DESKTOP_SESSION_PATH: &str = "/api/desktop/session/request";
pub const LEGACY_DESKTOP_SESSION_POLICY: &str = "auth_session.v1";
pub const LEGACY_DESKTOP_SESSION_NO_PROTECTED_GATES: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDesktopSessionSourceRoleV1 {
    Handler,
    Mount,
    DesktopClient,
    DesktopRouting,
    Authentication,
    ApiMiddlewareExclusion,
    SessionDecoder,
    Persistence,
    Database,
    Environment,
    DependencyLock,
}

impl LegacyDesktopSessionSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Mount => "mount",
            Self::DesktopClient => "desktop_client",
            Self::DesktopRouting => "desktop_routing",
            Self::Authentication => "authentication",
            Self::ApiMiddlewareExclusion => "api_middleware_exclusion",
            Self::SessionDecoder => "session_decoder",
            Self::Persistence => "persistence",
            Self::Database => "database",
            Self::Environment => "environment",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDesktopSessionSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyDesktopSessionSourceRoleV1,
}

pub const LEGACY_DESKTOP_SESSION_SOURCES: &[LegacyDesktopSessionSourcePinV1] = &[
    LegacyDesktopSessionSourcePinV1 {
        path: "apps/web/app/api/desktop/[...route]/session.ts",
        symbol: "GET /request+createDesktopRedirectPage+getDeploymentOrigin",
        sha256: "22c7b789cf901926e6ae4ffe2fcd574e5f6d8474a14c961d94ae3797b3ad45e0",
        role: LegacyDesktopSessionSourceRoleV1::Handler,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "apps/web/app/api/desktop/[...route]/route.ts",
        symbol: "desktop mount+GET+OPTIONS+CORS",
        sha256: "34854ff6fc0839838165990bea1c9ebee86770b1648ec832bbbb786720c9db41",
        role: LegacyDesktopSessionSourceRoleV1::Mount,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "apps/desktop/src/utils/auth.ts",
        symbol: "createSessionRequestUrl+paramsValidator+processAuthData",
        sha256: "ae80288f5caac230ff6390a96b3286f6eb961307cb85d3ca9dcc95f99931f914",
        role: LegacyDesktopSessionSourceRoleV1::DesktopClient,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "apps/desktop/src/utils/server-url-routing.ts",
        symbol: "shouldUseLocalServerSessionForUrl+resolveServerRequestPath",
        sha256: "3826d1163e4a8a558d199f2202290872d158ef0370f94c4841bebb5e614c46ff",
        role: LegacyDesktopSessionSourceRoleV1::DesktopRouting,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "apps/web/app/api/utils.ts",
        symbol: "getAuth+corsMiddleware",
        sha256: "241e5259f690ece17b0c50f78a9dc30c3e783082287040fef0f47e56a937bb30",
        role: LegacyDesktopSessionSourceRoleV1::Authentication,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "apps/web/proxy.ts",
        symbol: "API matcher exclusion",
        sha256: "7da98445a31f6b48d01b56877c47aaa79ba3af93dff8c015ad06a6e94fb42fcb",
        role: LegacyDesktopSessionSourceRoleV1::ApiMiddlewareExclusion,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        symbol: "decodeSessionToken+authOptions",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
        role: LegacyDesktopSessionSourceRoleV1::SessionDecoder,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "packages/database/auth/session.ts",
        symbol: "getCurrentUser",
        sha256: "d526dc9d7a6a1a7cb6a8695c24ab88b843ce09b4444f8e4ade24b7a06cbbc1ee",
        role: LegacyDesktopSessionSourceRoleV1::Authentication,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "packages/database/schema.ts",
        symbol: "users+authApiKeys",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyDesktopSessionSourceRoleV1::Persistence,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "packages/database/index.ts",
        symbol: "db",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyDesktopSessionSourceRoleV1::Database,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "packages/env/server.ts",
        symbol: "WEB_URL+VERCEL_ENV+VERCEL_BRANCH_URL_HOST+NEXTAUTH_SECRET",
        sha256: "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
        role: LegacyDesktopSessionSourceRoleV1::Environment,
    },
    LegacyDesktopSessionSourcePinV1 {
        path: "pnpm-lock.yaml",
        symbol: "Hono+Drizzle+NextAuth dependency resolutions",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyDesktopSessionSourceRoleV1::DependencyLock,
    },
];

pub const LEGACY_DESKTOP_SESSION_SOURCE_MANIFEST_SHA256: &str =
    "5aee0bd2953a3c80db86a8aadd3f367ad0011aa0a903ed27e96ce58e54f263aa";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyDesktopSessionProfileV1 {
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency_retry: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_DESKTOP_SESSION_PROFILE: LegacyDesktopSessionProfileV1 =
    LegacyDesktopSessionProfileV1 {
        operation_id: LEGACY_DESKTOP_SESSION_OPERATION_ID,
        method: "GET",
        path: LEGACY_DESKTOP_SESSION_PATH,
        success: "302_loopback_or_cap_desktop_deep_link_or_200_no_store_hybrid_html",
        validation: "unique_known_query_fields_and_decimal_tcp_port_1_through_65535",
        authorization: "host_only_current_browser_session_before_session_export_or_key_mint",
        idempotency_retry: "session_export_retry_safe_api_key_mint_non_idempotent_random_uuid",
        failure: "302_absolute_login_restart_for_missing_or_invalid_session_else_500",
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LegacyDesktopSessionPlatformV1 {
    #[default]
    Web,
    Desktop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LegacyDesktopSessionCredentialTypeV1 {
    #[default]
    Session,
    ApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LegacyDesktopSessionQueryV1 {
    pub port: Option<u16>,
    pub platform: LegacyDesktopSessionPlatformV1,
    pub credential_type: LegacyDesktopSessionCredentialTypeV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyDesktopSessionQueryErrorV1 {
    UnknownField,
    DuplicateField,
    InvalidPlatform,
    InvalidCredentialType,
    InvalidPort,
}

pub fn parse_legacy_desktop_session_query(
    query: Option<&str>,
) -> Result<LegacyDesktopSessionQueryV1, LegacyDesktopSessionQueryErrorV1> {
    let mut parsed = LegacyDesktopSessionQueryV1::default();
    let mut seen_port = false;
    let mut seen_platform = false;
    let mut seen_type = false;
    for (key, value) in form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "port" if !seen_port => {
                seen_port = true;
                if value.is_empty()
                    || value.len() > 5
                    || !value.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(LegacyDesktopSessionQueryErrorV1::InvalidPort);
                }
                let port = value
                    .parse::<u16>()
                    .map_err(|_| LegacyDesktopSessionQueryErrorV1::InvalidPort)?;
                if port == 0 {
                    return Err(LegacyDesktopSessionQueryErrorV1::InvalidPort);
                }
                parsed.port = Some(port);
            }
            "platform" if !seen_platform => {
                seen_platform = true;
                parsed.platform = match value.as_ref() {
                    "web" => LegacyDesktopSessionPlatformV1::Web,
                    "desktop" => LegacyDesktopSessionPlatformV1::Desktop,
                    _ => return Err(LegacyDesktopSessionQueryErrorV1::InvalidPlatform),
                };
            }
            "type" if !seen_type => {
                seen_type = true;
                parsed.credential_type = match value.as_ref() {
                    "session" => LegacyDesktopSessionCredentialTypeV1::Session,
                    "api_key" => LegacyDesktopSessionCredentialTypeV1::ApiKey,
                    _ => return Err(LegacyDesktopSessionQueryErrorV1::InvalidCredentialType),
                };
            }
            "port" | "platform" | "type" => {
                return Err(LegacyDesktopSessionQueryErrorV1::DuplicateField);
            }
            _ => return Err(LegacyDesktopSessionQueryErrorV1::UnknownField),
        }
    }
    Ok(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum LegacyDesktopSessionPayloadV1 {
    #[serde(rename = "token")]
    Token { token: String, expires: String },
    #[serde(rename = "api_key")]
    ApiKey { api_key: String },
}

impl LegacyDesktopSessionPayloadV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        match self {
            Self::Token { token, expires } => {
                (32..=512).contains(&token.len())
                    && token.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
                    })
                    && expires.parse::<u64>().is_ok_and(|value| value > 0)
            }
            Self::ApiKey { api_key } => valid_uuid_v4(api_key),
        }
    }
}

fn valid_uuid_v4(value: &str) -> bool {
    value.len() == 36
        && value.as_bytes().get(8) == Some(&b'-')
        && value.as_bytes().get(13) == Some(&b'-')
        && value.as_bytes().get(18) == Some(&b'-')
        && value.as_bytes().get(23) == Some(&b'-')
        && value.as_bytes().get(14) == Some(&b'4')
        && value
            .as_bytes()
            .get(19)
            .is_some_and(|byte| matches!(byte, b'8' | b'9' | b'a' | b'b'))
        && value.bytes().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(&byte)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyDesktopSessionDestinationV1 {
    Redirect(Url),
    HybridPage { primary_url: Url, fallback_url: Url },
}

pub fn legacy_desktop_session_destination(
    query: LegacyDesktopSessionQueryV1,
    payload: &LegacyDesktopSessionPayloadV1,
    user_id: &str,
) -> Option<LegacyDesktopSessionDestinationV1> {
    if !payload.valid() || user_id.is_empty() || user_id.len() > 128 || !user_id.is_ascii() {
        return None;
    }
    let params = legacy_desktop_session_params(payload, user_id);
    let deep_link = Url::parse(&format!("cap-desktop://signin?{params}")).ok()?;
    let Some(port) = query.port else {
        return Some(LegacyDesktopSessionDestinationV1::Redirect(deep_link));
    };
    let loopback = Url::parse(&format!("http://127.0.0.1:{port}?{params}")).ok()?;
    Some(match query.platform {
        LegacyDesktopSessionPlatformV1::Web => {
            LegacyDesktopSessionDestinationV1::Redirect(loopback)
        }
        LegacyDesktopSessionPlatformV1::Desktop => LegacyDesktopSessionDestinationV1::HybridPage {
            primary_url: deep_link,
            fallback_url: loopback,
        },
    })
}

#[must_use]
pub fn legacy_desktop_session_params(
    payload: &LegacyDesktopSessionPayloadV1,
    user_id: &str,
) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    match payload {
        LegacyDesktopSessionPayloadV1::Token { token, expires } => {
            serializer.append_pair("type", "token");
            serializer.append_pair("token", token);
            serializer.append_pair("expires", expires);
        }
        LegacyDesktopSessionPayloadV1::ApiKey { api_key } => {
            serializer.append_pair("type", "api_key");
            serializer.append_pair("api_key", api_key);
        }
    }
    serializer.append_pair("user_id", user_id);
    serializer.finish()
}

pub fn legacy_desktop_login_url(
    deployment_origin: &Url,
    request_path_and_query: &str,
) -> Option<Url> {
    if deployment_origin.cannot_be_a_base()
        || !matches!(deployment_origin.scheme(), "http" | "https")
        || deployment_origin.path() != "/"
        || deployment_origin.query().is_some()
        || deployment_origin.fragment().is_some()
        || !request_path_and_query.starts_with('/')
        || request_path_and_query.starts_with("//")
    {
        return None;
    }
    let next = deployment_origin.join(request_path_and_query).ok()?;
    if next.origin() != deployment_origin.origin() {
        return None;
    }
    let mut login = deployment_origin.join("login").ok()?;
    login.query_pairs_mut().append_pair("next", next.as_str());
    Some(login)
}

pub fn legacy_desktop_deployment_origin(
    web_url: &str,
    deployment_environment: Option<&str>,
    preview_branch_host: Option<&str>,
) -> Option<Url> {
    let canonical = normalized_http_origin(web_url)?;
    if deployment_environment != Some("preview") {
        return Some(canonical);
    }
    let Some(branch) = preview_branch_host
        .and_then(|host| host.strip_suffix(".vercel.app"))
        .filter(|value| !value.is_empty())
    else {
        return Some(canonical);
    };
    if !branch
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
    {
        return Some(canonical);
    }
    normalized_http_origin(&format!("https://{branch}.vercel.app")).or(Some(canonical))
}

fn normalized_http_origin(value: &str) -> Option<Url> {
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    url.set_path("/");
    Some(url)
}

#[must_use]
pub fn legacy_desktop_session_source_manifest() -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-desktop-session-source-manifest-v1\0");
    for source in LEGACY_DESKTOP_SESSION_SOURCES {
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

    fn api_key() -> LegacyDesktopSessionPayloadV1 {
        let fixture_value = "018f47a6-7b1c-4f55-8f39-8f8a8690f704";
        LegacyDesktopSessionPayloadV1::ApiKey {
            api_key: fixture_value.into(),
        }
    }

    #[test]
    fn source_closure_and_profile_are_frozen_and_provider_free() {
        assert_eq!(LEGACY_DESKTOP_SESSION_SOURCES.len(), 12);
        assert_eq!(
            legacy_desktop_session_source_manifest(),
            LEGACY_DESKTOP_SESSION_SOURCE_MANIFEST_SHA256
        );
        assert!(LEGACY_DESKTOP_SESSION_NO_PROTECTED_GATES.is_empty());
        assert_eq!(LEGACY_DESKTOP_SESSION_PROFILE.method, "GET");
        assert_eq!(
            LEGACY_DESKTOP_SESSION_PROFILE.operation_id,
            LEGACY_DESKTOP_SESSION_OPERATION_ID
        );
    }

    #[test]
    fn query_defaults_match_cap_and_port_is_tightened_to_tcp_range() {
        assert_eq!(
            parse_legacy_desktop_session_query(None),
            Ok(LegacyDesktopSessionQueryV1::default())
        );
        assert_eq!(
            parse_legacy_desktop_session_query(Some("type=api_key&platform=desktop&port=43117")),
            Ok(LegacyDesktopSessionQueryV1 {
                port: Some(43117),
                platform: LegacyDesktopSessionPlatformV1::Desktop,
                credential_type: LegacyDesktopSessionCredentialTypeV1::ApiKey,
            })
        );
        for invalid in [
            "port=0",
            "port=65536",
            "port=80%2F%2Fevil.test",
            "port=1&port=2",
            "unknown=x",
        ] {
            assert!(parse_legacy_desktop_session_query(Some(invalid)).is_err());
        }
    }

    #[test]
    fn destinations_preserve_cap_parameter_shapes() {
        let web = legacy_desktop_session_destination(
            LegacyDesktopSessionQueryV1 {
                port: Some(43117),
                platform: LegacyDesktopSessionPlatformV1::Web,
                credential_type: LegacyDesktopSessionCredentialTypeV1::ApiKey,
            },
            &api_key(),
            "user-1",
        )
        .expect("destination");
        let LegacyDesktopSessionDestinationV1::Redirect(web) = web else {
            panic!("web loopback must redirect");
        };
        assert_eq!(web.scheme(), "http");
        assert_eq!(web.host_str(), Some("127.0.0.1"));
        assert_eq!(web.port(), Some(43117));
        let web_query = web.query().expect("loopback query");
        assert!(web_query.contains("type=api_key"));
        assert!(web_query.contains("user_id=user-1"));

        let deep_link = legacy_desktop_session_destination(
            LegacyDesktopSessionQueryV1::default(),
            &LegacyDesktopSessionPayloadV1::Token {
                token: "s".repeat(64),
                expires: "1700000000".into(),
            },
            "user-1",
        )
        .expect("destination");
        let LegacyDesktopSessionDestinationV1::Redirect(deep_link) = deep_link else {
            panic!("no-port handoff must deep link");
        };
        assert_eq!(deep_link.scheme(), "cap-desktop");
        let deep_link_query = deep_link.query().expect("deep-link query");
        assert!(deep_link_query.contains("type=token"));
        assert!(deep_link_query.contains("expires=1700000000"));
    }

    #[test]
    fn login_restart_is_absolute_and_preview_host_is_pinned() {
        let canonical = legacy_desktop_deployment_origin(
            "https://frame.engmanager.xyz",
            Some("production"),
            None,
        )
        .expect("canonical origin");
        assert_eq!(canonical.as_str(), "https://frame.engmanager.xyz/");
        let preview = legacy_desktop_deployment_origin(
            "https://frame.engmanager.xyz",
            Some("preview"),
            Some("frame-pr-9.vercel.app"),
        )
        .expect("preview origin");
        assert_eq!(preview.as_str(), "https://frame-pr-9.vercel.app/");
        assert_eq!(
            legacy_desktop_deployment_origin(
                "https://frame.engmanager.xyz",
                Some("preview"),
                Some("attacker.example"),
            ),
            Some(canonical.clone())
        );
        let login = legacy_desktop_login_url(
            &canonical,
            "/api/desktop/session/request?type=api_key&port=43117",
        )
        .expect("login URL");
        assert_eq!(login.path(), "/login");
        assert!(
            login
                .query()
                .expect("login query")
                .contains("next=https%3A%2F%2Fframe.engmanager.xyz")
        );
    }
}
