//! Source-pinned contracts for Cap's four mobile session routes.
//!
//! The compatibility boundary intentionally preserves Cap's unusual details:
//! email and code normalization, destructive one-use verification attempts,
//! replacement (rather than accumulation) of mobile API keys, optional browser
//! sessions on the login redirect route, and the middleware/handler split on
//! revocation. Provider execution remains an explicit protected gate; the
//! local adapter may only commit a ciphertext delivery handoff or Stripe-sync
//! effect intent.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

pub const LEGACY_MOBILE_SESSION_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_MOBILE_EMAIL_REQUEST_OPERATION_ID: &str = "cap-v1-e16563e40f697519";
pub const LEGACY_MOBILE_EMAIL_VERIFY_OPERATION_ID: &str = "cap-v1-139a189f8a00b38c";
pub const LEGACY_MOBILE_SESSION_REQUEST_OPERATION_ID: &str = "cap-v1-ea999fdc5829fbd1";
pub const LEGACY_MOBILE_SESSION_REVOKE_OPERATION_ID: &str = "cap-v1-1eef72e518a37abd";
pub const LEGACY_MOBILE_EMAIL_REQUEST_PATH: &str = "/api/mobile/session/email/request";
pub const LEGACY_MOBILE_EMAIL_VERIFY_PATH: &str = "/api/mobile/session/email/verify";
pub const LEGACY_MOBILE_SESSION_REQUEST_PATH: &str = "/api/mobile/session/request";
pub const LEGACY_MOBILE_SESSION_REVOKE_PATH: &str = "/api/mobile/session/revoke";
pub const LEGACY_MOBILE_SESSION_RATE_LIMIT_BUCKET: &str = "auth_session.v1";
pub const LEGACY_MOBILE_SESSION_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_MOBILE_EMAIL_CODE_TTL_MS: i64 = 10 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyMobileSessionSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
}

const ROUTE: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "apps/web/app/api/mobile/[...route]/route.ts",
    symbol: "mobile session route implementation",
    sha256: "02df2ce92dc6e8ae11748b6e082c1304596ba9e4c370b35069867754218f5f79",
};
const MOBILE_DOMAIN: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "packages/web-domain/src/Mobile.ts",
    symbol: "mobile session schemas and redirect allowlist",
    sha256: "331d76900372d62389d729f8682baca1344f3583e3f41f42ad6e3ef2be7a3d5b",
};
const DATABASE_SCHEMA: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "packages/database/schema.ts",
    symbol: "users, verificationTokens, authApiKeys",
    sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
};
const DATABASE_SERVICE: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "packages/web-backend/src/Database.ts",
    symbol: "Database transaction service",
    sha256: "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
};
const DOMAIN_UTILS: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "packages/database/auth/domain-utils.ts",
    symbol: "isEmailAllowedForSignup",
    sha256: "f2f77ad2ee6106e482ff6cee183f1d49541de2b7705a824e72a4ea1c55c3310a",
};
const ENV: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "packages/env/server.ts",
    symbol: "mobile auth provider and deployment configuration",
    sha256: "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
};
const MOBILE_CLIENT: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "apps/mobile/src/api/mobile.ts",
    symbol: "released mobile session caller",
    sha256: "dc426448ea7197353880ddfb771e7ca9d17b903a539acfa6ba28cd66227c3a08",
};
const LOCKFILE: LegacyMobileSessionSourcePinV1 = LegacyMobileSessionSourcePinV1 {
    path: "pnpm-lock.yaml",
    symbol: "URL, Effect HTTP, email, NextAuth, and Stripe dependency closure",
    sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
};

pub const LEGACY_MOBILE_EMAIL_REQUEST_SOURCES: &[LegacyMobileSessionSourcePinV1] = &[
    ROUTE,
    MOBILE_DOMAIN,
    DATABASE_SCHEMA,
    DATABASE_SERVICE,
    DOMAIN_UTILS,
    LegacyMobileSessionSourcePinV1 {
        path: "packages/database/emails/config.ts",
        symbol: "sendEmail via Resend",
        sha256: "d7f399dcefaeb0dd9c0a048f1a7212b24af11249b85e7c31cb569b6cb0108ead",
    },
    LegacyMobileSessionSourcePinV1 {
        path: "packages/database/emails/otp-email.tsx",
        symbol: "OTPEmail",
        sha256: "66c3c658224bc8bd0f03ed2944127dbb5971bbea489efd7bbec6c3c698ba03cc",
    },
    ENV,
    MOBILE_CLIENT,
    LOCKFILE,
];

pub const LEGACY_MOBILE_EMAIL_VERIFY_SOURCES: &[LegacyMobileSessionSourcePinV1] = &[
    ROUTE,
    MOBILE_DOMAIN,
    DATABASE_SCHEMA,
    DATABASE_SERVICE,
    DOMAIN_UTILS,
    LegacyMobileSessionSourcePinV1 {
        path: "packages/database/auth/auth-options.ts",
        symbol: "authOptions adapter binding",
        sha256: "22b8923e1cab6b5b1b318609abe664e171fb740ae39817c2c962908ca0dc8595",
    },
    LegacyMobileSessionSourcePinV1 {
        path: "packages/database/auth/drizzle-adapter.ts",
        symbol: "getUserByEmail, createUser, updateUser and Stripe provisioning",
        sha256: "c97d95b50851adf4e809ba829d4fafcb1790a56d5e55a5331f4c614f947a5c52",
    },
    LegacyMobileSessionSourcePinV1 {
        path: "packages/database/helpers.ts",
        symbol: "nanoId",
        sha256: "e976df51a8186737a1c4696a20cd52f2c029aa630b2463d1970b8667b0dd20cd",
    },
    LegacyMobileSessionSourcePinV1 {
        path: "packages/utils/src/index.ts",
        symbol: "STRIPE_AVAILABLE and stripe export",
        sha256: "1a9ebe6c9dadb39206ae8cd3bea95dbd7f9ae913426e8f3f7eb7e5acf9461c83",
    },
    LegacyMobileSessionSourcePinV1 {
        path: "packages/utils/src/lib/stripe/stripe.ts",
        symbol: "Stripe singleton",
        sha256: "d2bb7868a33928f06ab543b564bd7365f0b5a48fed619c9ecd66f2a36e244dfc",
    },
    ENV,
    MOBILE_CLIENT,
    LOCKFILE,
];

pub const LEGACY_MOBILE_SESSION_REQUEST_SOURCES: &[LegacyMobileSessionSourcePinV1] = &[
    ROUTE,
    MOBILE_DOMAIN,
    DATABASE_SCHEMA,
    DATABASE_SERVICE,
    LegacyMobileSessionSourcePinV1 {
        path: "packages/web-backend/src/Auth.ts",
        symbol: "getCurrentUser",
        sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    },
    ENV,
    MOBILE_CLIENT,
    LOCKFILE,
];

pub const LEGACY_MOBILE_SESSION_REVOKE_SOURCES: &[LegacyMobileSessionSourcePinV1] = &[
    ROUTE,
    MOBILE_DOMAIN,
    DATABASE_SCHEMA,
    DATABASE_SERVICE,
    LegacyMobileSessionSourcePinV1 {
        path: "packages/web-backend/src/Auth.ts",
        symbol: "HttpAuthMiddlewareLive",
        sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
    },
    LegacyMobileSessionSourcePinV1 {
        path: "packages/web-domain/src/Authentication.ts",
        symbol: "HttpAuthMiddleware",
        sha256: "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
    },
    MOBILE_CLIENT,
    LOCKFILE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileSessionActionV1 {
    EmailRequest,
    EmailVerify,
    SessionRequest,
    SessionRevoke,
}

impl LegacyMobileSessionActionV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::EmailRequest => LEGACY_MOBILE_EMAIL_REQUEST_OPERATION_ID,
            Self::EmailVerify => LEGACY_MOBILE_EMAIL_VERIFY_OPERATION_ID,
            Self::SessionRequest => LEGACY_MOBILE_SESSION_REQUEST_OPERATION_ID,
            Self::SessionRevoke => LEGACY_MOBILE_SESSION_REVOKE_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::SessionRequest => "GET",
            Self::EmailRequest | Self::EmailVerify | Self::SessionRevoke => "POST",
        }
    }

    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::EmailRequest => LEGACY_MOBILE_EMAIL_REQUEST_PATH,
            Self::EmailVerify => LEGACY_MOBILE_EMAIL_VERIFY_PATH,
            Self::SessionRequest => LEGACY_MOBILE_SESSION_REQUEST_PATH,
            Self::SessionRevoke => LEGACY_MOBILE_SESSION_REVOKE_PATH,
        }
    }

    #[must_use]
    pub const fn sources(self) -> &'static [LegacyMobileSessionSourcePinV1] {
        match self {
            Self::EmailRequest => LEGACY_MOBILE_EMAIL_REQUEST_SOURCES,
            Self::EmailVerify => LEGACY_MOBILE_EMAIL_VERIFY_SOURCES,
            Self::SessionRequest => LEGACY_MOBILE_SESSION_REQUEST_SOURCES,
            Self::SessionRevoke => LEGACY_MOBILE_SESSION_REVOKE_SOURCES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileSessionErrorV1 {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Internal,
}

impl LegacyMobileSessionErrorV1 {
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::Internal => 500,
        }
    }

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::BadRequest => "BadRequest",
            Self::Unauthorized => "Unauthorized",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "NotFound",
            Self::Internal => "InternalServerError",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LegacyMobileEmailRequestV1 {
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LegacyMobileEmailVerifyV1 {
    pub email: String,
    pub code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyMobileSuccessV1 {
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobileApiKeyV1 {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub api_key: String,
    pub user_id: String,
}

impl LegacyMobileApiKeyV1 {
    #[must_use]
    pub fn new(api_key: String, user_id: String) -> Self {
        Self {
            kind: "api_key",
            api_key,
            user_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMobileProviderV1 {
    Google,
    Workos,
}

impl LegacyMobileProviderV1 {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "google" => Some(Self::Google),
            "workos" => Some(Self::Workos),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Workos => "workos",
        }
    }
}

/// JavaScript's `String#trim` whitespace set (ECMAScript WhiteSpace plus
/// LineTerminator). Rust's broader Unicode whitespace predicate is not used.
#[must_use]
pub fn legacy_mobile_trim(value: &str) -> &str {
    value.trim_matches(is_ecmascript_whitespace)
}

#[must_use]
pub fn normalize_legacy_mobile_email(value: &str) -> String {
    legacy_mobile_trim(value).to_lowercase()
}

#[must_use]
pub fn legacy_mobile_email_is_valid(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || local.chars().any(is_ecmascript_whitespace)
        || domain.chars().any(is_ecmascript_whitespace)
    {
        return false;
    }
    let Some((before_dot, after_dot)) = domain.rsplit_once('.') else {
        return false;
    };
    !before_dot.is_empty() && !after_dot.is_empty()
}

#[must_use]
pub fn legacy_mobile_email_code_is_valid(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[must_use]
pub fn legacy_mobile_email_identifier_digest(email: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(email.as_bytes());
    format!("{:x}", digest.finalize())
}

#[must_use]
pub fn legacy_mobile_email_identifier(email: &str) -> String {
    format!("mobile:{}", legacy_mobile_email_identifier_digest(email))
}

#[must_use]
pub fn legacy_mobile_email_code_digest(code: &str, nextauth_secret: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(code.as_bytes());
    digest.update(nextauth_secret.as_bytes());
    format!("{:x}", digest.finalize())
}

/// Source-equivalent bearer parser used by the revoke handler. Extra segments
/// are ignored because JavaScript destructuring reads only the first two.
#[must_use]
pub fn legacy_mobile_bearer_token(authorization: Option<&str>) -> Option<&str> {
    let mut segments = authorization?.split(' ');
    let scheme = segments.next()?;
    let token = segments.next()?;
    if scheme.eq_ignore_ascii_case("bearer") && !token.is_empty() {
        Some(token)
    } else {
        None
    }
}

/// Source-equivalent middleware token selection. A 36-byte second segment
/// selects API-key authentication even when the scheme is not `Bearer`.
#[must_use]
pub fn legacy_mobile_middleware_api_key(authorization: Option<&str>) -> Option<&str> {
    let token = authorization?.split(' ').nth(1)?;
    (token.len() == 36).then_some(token)
}

#[must_use]
pub fn is_legacy_mobile_auth_redirect_uri(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    if url.scheme() == "cap" {
        return url.host_str() == Some("auth")
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.path().is_empty()
            && url.query().is_none()
            && url.fragment().is_none();
    }
    if url.scheme() != "exp+cap"
        || url.host_str() != Some("expo-development-client")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    if url.path() == "/--/auth" {
        return url.query().is_none();
    }
    let pairs = url.query_pairs().collect::<Vec<_>>();
    if pairs.len() != 1 || pairs[0].0 != "url" || pairs[0].1.is_empty() {
        return false;
    }
    Url::parse(&pairs[0].1).is_ok_and(|embedded| {
        embedded.path() == "/--/auth" && embedded.query().is_none() && embedded.fragment().is_none()
    })
}

#[must_use]
pub fn legacy_mobile_login_redirect(
    deployment_origin: &Url,
    request_url: &Url,
    provider: Option<LegacyMobileProviderV1>,
    organization_id: Option<&str>,
) -> Option<Url> {
    let mut callback = deployment_origin.clone();
    callback.set_path(request_url.path());
    callback.set_query(request_url.query());
    callback.set_fragment(None);
    let mut login = deployment_origin.join("/login").ok()?;
    login
        .query_pairs_mut()
        .append_pair("next", callback.as_str());
    match provider {
        Some(LegacyMobileProviderV1::Google) => {
            login
                .query_pairs_mut()
                .append_pair("mobileProvider", "google");
        }
        Some(LegacyMobileProviderV1::Workos) => {
            login
                .query_pairs_mut()
                .append_pair("mobileProvider", "workos");
            if let Some(organization_id) = organization_id {
                login
                    .query_pairs_mut()
                    .append_pair("organizationId", organization_id);
            }
        }
        None => {}
    }
    Some(login)
}

#[must_use]
pub fn legacy_mobile_authenticated_redirect(
    redirect_uri: &str,
    api_key: &str,
    legacy_user_id: &str,
) -> Option<Url> {
    if !is_legacy_mobile_auth_redirect_uri(redirect_uri) {
        return None;
    }
    let mut redirect = Url::parse(redirect_uri).ok()?;
    redirect
        .query_pairs_mut()
        .append_pair("api_key", api_key)
        .append_pair("user_id", legacy_user_id);
    Some(redirect)
}

#[must_use]
pub fn legacy_mobile_provisioned_user_name(email: &str) -> String {
    let local = email.split('@').next().unwrap_or_default();
    let mut result = String::with_capacity(local.len());
    let mut separator = false;
    for character in local.chars() {
        if matches!(character, '.' | '_' | '-') {
            if !separator {
                result.push(' ');
                separator = true;
            }
        } else {
            result.push(character);
            separator = false;
        }
    }
    let result = legacy_mobile_trim(&result);
    if result.is_empty() {
        email.to_owned()
    } else {
        result.to_owned()
    }
}

#[must_use]
pub fn legacy_mobile_signup_domain_allowed(email: &str, configured: Option<&str>) -> bool {
    let Some(configured) = configured else {
        return true;
    };
    if configured.trim().is_empty() {
        return true;
    }
    let Some((_, domain)) = email.rsplit_once('@') else {
        return false;
    };
    // Zod's source email validation rejects whitespace and malformed domains.
    if !legacy_mobile_email_is_valid(email) || !valid_signup_domain(domain) {
        return false;
    }
    configured
        .split(',')
        .map(str::trim)
        .filter(|candidate| valid_signup_domain(candidate))
        .any(|candidate| candidate.eq_ignore_ascii_case(domain))
}

fn valid_signup_domain(value: &str) -> bool {
    if value.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if value.is_empty() || value.len() > 253 || !value.contains('.') {
        return false;
    }
    let mut labels = value.split('.').peekable();
    while let Some(label) = labels.next() {
        let terminal = labels.peek().is_none();
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || (terminal && !label.bytes().all(|byte| byte.is_ascii_alphabetic()))
            || (terminal && !(2..=63).contains(&label.len()))
        {
            return false;
        }
    }
    true
}

const fn is_ecmascript_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_and_code_normalization_match_the_source_regexes() {
        assert_eq!(
            normalize_legacy_mobile_email("\u{feff}User.Name@Example.COM\u{00a0}"),
            "user.name@example.com"
        );
        assert!(legacy_mobile_email_is_valid("a@b.co"));
        assert!(legacy_mobile_email_is_valid("a@b.c.d"));
        assert!(!legacy_mobile_email_is_valid("a@@b.co"));
        assert!(!legacy_mobile_email_is_valid("a b@b.co"));
        assert!(legacy_mobile_email_code_is_valid("100000"));
        assert!(!legacy_mobile_email_code_is_valid("１２３４５６"));
        assert!(!legacy_mobile_email_code_is_valid(" 123456"));
    }

    #[test]
    fn code_hash_is_code_then_nextauth_secret_and_identifier_is_prefixed() {
        assert_eq!(
            legacy_mobile_email_code_digest("123456", "secret"),
            "da023f7090dd831097f8a534475b1c4fba2a9a6419968e52be7459e2533ac819"
        );
        assert!(legacy_mobile_email_identifier("user@example.com").starts_with("mobile:"));
        assert_eq!(legacy_mobile_email_identifier("user@example.com").len(), 71);
    }

    #[test]
    fn redirect_allowlist_preserves_cap_and_expo_shapes() {
        assert!(is_legacy_mobile_auth_redirect_uri("cap://auth"));
        assert!(!is_legacy_mobile_auth_redirect_uri("cap://auth/"));
        assert!(!is_legacy_mobile_auth_redirect_uri("cap://auth?x=1"));
        assert!(is_legacy_mobile_auth_redirect_uri(
            "exp+cap://expo-development-client/--/auth"
        ));
        assert!(is_legacy_mobile_auth_redirect_uri(
            "exp+cap://expo-development-client/client?url=https%3A%2F%2Fexample.com%2F--%2Fauth"
        ));
        assert!(!is_legacy_mobile_auth_redirect_uri(
            "exp+cap://expo-development-client/client?url=https%3A%2F%2Fexample.com%2F--%2Fauth%3Fx%3D1"
        ));
        assert!(!is_legacy_mobile_auth_redirect_uri(
            "https://attacker.example/--/auth"
        ));
    }

    #[test]
    fn middleware_and_handler_bearer_parsers_remain_intentionally_distinct() {
        let uuid = "00000000-0000-4000-8000-000000000001";
        assert_eq!(
            legacy_mobile_middleware_api_key(Some(&format!("Basic {uuid}"))),
            Some(uuid)
        );
        assert_eq!(
            legacy_mobile_bearer_token(Some(&format!("Basic {uuid}"))),
            None
        );
        assert_eq!(
            legacy_mobile_bearer_token(Some("Bearer short ignored")),
            Some("short")
        );
        assert_eq!(
            legacy_mobile_middleware_api_key(Some("Bearer short ignored")),
            None
        );
        assert_eq!(legacy_mobile_bearer_token(Some("Bearer  token")), None);
    }

    #[test]
    fn login_redirect_propagates_only_source_provider_parameters() {
        let deployment = Url::parse("https://frame.engmanager.xyz")
            .expect("checked-in deployment URL must parse");
        let request = Url::parse(
            "https://worker.example/api/mobile/session/request?provider=workos&organizationId=org_1",
        )
        .expect("checked-in mobile request URL must parse");
        let Some(redirect) = legacy_mobile_login_redirect(
            &deployment,
            &request,
            Some(LegacyMobileProviderV1::Workos),
            Some("org_1"),
        ) else {
            panic!("valid mobile provider parameters must produce a login redirect");
        };
        assert_eq!(redirect.path(), "/login");
        let pairs = redirect.query_pairs().collect::<Vec<_>>();
        assert!(
            pairs
                .iter()
                .any(|pair| pair.0 == "mobileProvider" && pair.1 == "workos")
        );
        assert!(
            pairs
                .iter()
                .any(|pair| pair.0 == "organizationId" && pair.1 == "org_1")
        );
        assert!(pairs.iter().any(|pair| {
            pair.0 == "next"
                && pair
                    .1
                    .starts_with("https://frame.engmanager.xyz/api/mobile/session/request")
        }));
    }

    #[test]
    fn signup_domain_and_provisioned_name_match_the_source_rules() {
        assert!(legacy_mobile_signup_domain_allowed(
            "user@example.com",
            Some("invalid, Example.com, -bad.example")
        ));
        assert!(!legacy_mobile_signup_domain_allowed(
            "user@elsewhere.com",
            Some("example.com")
        ));
        assert_eq!(
            legacy_mobile_provisioned_user_name("first._-last@example.com"),
            "first last"
        );
        assert_eq!(
            legacy_mobile_provisioned_user_name("---@example.com"),
            "---@example.com"
        );
    }
}
