//! Source-pinned compatibility contract for Cap's Chrome-extension auth handoff.
//!
//! The pinned flow has four deliberately separate endpoints. `start` only
//! validates the Chromium redirect and renders consent (or sends an expired
//! session through login); only the same-origin `approve` form POST may mint a
//! credential. `revoke` and `bootstrap` share Cap's API-key-or-session actor
//! middleware. Frame stores the one-time returned UUID as a SHA-256 digest,
//! while preserving the externally observable UUID and ownership semantics.
//! Frame intentionally tightens the source's active-pointer lookup to require
//! that the active organization is owned by the actor before the owned/member
//! fallbacks; this closes cross-tenant pointer corruption without changing any
//! valid source result.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::{Url, form_urlencoded};

pub const LEGACY_EXTENSION_AUTH_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_EXTENSION_AUTH_START_OPERATION_ID: &str = "cap-v1-249fbd2f77ee7209";
pub const LEGACY_EXTENSION_AUTH_APPROVE_OPERATION_ID: &str = "cap-v1-96499b6c8e845b35";
pub const LEGACY_EXTENSION_AUTH_REVOKE_OPERATION_ID: &str = "cap-v1-ed715d4d23e82181";
pub const LEGACY_EXTENSION_BOOTSTRAP_OPERATION_ID: &str = "cap-v1-12159b1acbaeba7a";
pub const LEGACY_EXTENSION_AUTH_START_PATH: &str = "/api/extension/auth/start";
pub const LEGACY_EXTENSION_AUTH_APPROVE_PATH: &str = "/api/extension/auth/approve";
pub const LEGACY_EXTENSION_AUTH_REVOKE_PATH: &str = "/api/extension/auth/revoke";
pub const LEGACY_EXTENSION_BOOTSTRAP_PATH: &str = "/api/extension/bootstrap";
pub const LEGACY_EXTENSION_AUTH_MAX_FORM_BYTES: usize = 8 * 1024;
pub const LEGACY_EXTENSION_AUTH_MAX_VALUE_BYTES: usize = 4 * 1024;
pub const LEGACY_EXTENSION_AUTH_KEY_MINT_WINDOW_MS: i64 = 60 * 60 * 1_000;
pub const LEGACY_EXTENSION_AUTH_KEY_MINT_LIMIT: usize = 10;
pub const LEGACY_EXTENSION_FREE_PLAN_MAX_RECORDING_SECONDS: u32 = 300;
pub const LEGACY_EXTENSION_AUTH_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_EXTENSION_AUTH_SOURCE_MANIFEST_SHA256: &str =
    "6ec986a350a8f882e8f7150460a24f77f7a0fb5573f3b0b7f634d5578d79aca5";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyExtensionAuthSourceRoleV1 {
    Contract,
    Handler,
    Service,
    Authentication,
    Persistence,
    Entitlement,
    Mount,
    Environment,
    Client,
    DependencyLock,
}

impl LegacyExtensionAuthSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Contract => "contract",
            Self::Handler => "handler",
            Self::Service => "service",
            Self::Authentication => "authentication",
            Self::Persistence => "persistence",
            Self::Entitlement => "entitlement",
            Self::Mount => "mount",
            Self::Environment => "environment",
            Self::Client => "client",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyExtensionAuthSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyExtensionAuthSourceRoleV1,
}

pub const LEGACY_EXTENSION_AUTH_SOURCES: &[LegacyExtensionAuthSourcePinV1] = &[
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-domain/src/Extension.ts",
        symbol: "ExtensionHttpApi+ExtensionApiPaths+ExtensionBootstrapSuccess",
        sha256: "d1bc68b7e302bc098d16c17bd991fe942a7361ffa88675574ce45980395582ba",
        role: LegacyExtensionAuthSourceRoleV1::Contract,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-backend/src/Extension/Http.ts",
        symbol: "startAuth+approveAuth+revokeAuth+bootstrap",
        sha256: "8bcaeadc626ec4b0bd43ca6f6e2bba643c7386f16f033b7f5d3c103e2173c602",
        role: LegacyExtensionAuthSourceRoleV1::Handler,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-backend/src/Extension/Extensions.ts",
        symbol: "mintAuthKey+revokeAuthKey+resolveBootstrapOrganization",
        sha256: "097542ee0ccf8de79f310ebf8da90d982b6c58af169c1d74a4721a68adc48542",
        role: LegacyExtensionAuthSourceRoleV1::Service,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-backend/src/Auth.ts",
        symbol: "getCurrentUser+HttpAuthMiddlewareLive",
        sha256: "aea054db2b84a8c4bd6684fefe8d0e971a094a9faa9653105b0c33ab52ab824d",
        role: LegacyExtensionAuthSourceRoleV1::Authentication,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-domain/src/Authentication.ts",
        symbol: "HttpAuthMiddleware+CurrentUser",
        sha256: "165c9f652c39d7f1cf3b43a5c66c5a4418bbe97338279ca01d00c19f2026167b",
        role: LegacyExtensionAuthSourceRoleV1::Authentication,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/database/schema.ts",
        symbol: "users+organizations+organizationMembers+authApiKeys",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyExtensionAuthSourceRoleV1::Persistence,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-backend/src/Database.ts",
        symbol: "Database",
        sha256: "24500254943ace60c5ea3a7943f40c85ab2c9a8caba36073ff54100ab9488837",
        role: LegacyExtensionAuthSourceRoleV1::Persistence,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/utils/src/constants/plans.ts",
        symbol: "userIsPro",
        sha256: "e047a50e6f72e3fe33985fde475b25ea4d5f9701fbe15adda0c1cb3aaaa21385",
        role: LegacyExtensionAuthSourceRoleV1::Entitlement,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        symbol: "FREE_PLAN_MAX_RECORDING_SECONDS",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
        role: LegacyExtensionAuthSourceRoleV1::Entitlement,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-domain/src/Http/Api.ts",
        symbol: "ApiContract /api/extension mount",
        sha256: "33f37588b210fe8be6584a51cd786b1347f5f7733fb2ac6b9111002e61437a25",
        role: LegacyExtensionAuthSourceRoleV1::Mount,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/web-backend/src/Http/Live.ts",
        symbol: "ExtensionHttpLive layer",
        sha256: "fa73f7797f44f11271e0e59fe14817144733ea06fb954c30e8f2f4720fa7216c",
        role: LegacyExtensionAuthSourceRoleV1::Mount,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/env/server.ts",
        symbol: "serverEnv WEB_URL+CAP_CHROME_EXTENSION_ID+NODE_ENV",
        sha256: "235c2ea66843b610aee61c82cbcafe05086d00193545bc290650d3aa15a2a0a4",
        role: LegacyExtensionAuthSourceRoleV1::Environment,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "packages/env/build.ts",
        symbol: "buildEnv NEXT_PUBLIC_IS_CAP",
        sha256: "454bc82ebd9ca83bae656336b67287d13bc351d357c2143444d226d84f2707bd",
        role: LegacyExtensionAuthSourceRoleV1::Environment,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "apps/chrome-extension/src/shared/api.ts",
        symbol: "createAuthStart+parseAuthResponse+revokeAuth+fetchBootstrap",
        sha256: "7439a031accac54fcd727c8b643a40f1fca885fbaa15d769c8a6c1e99bf28df7",
        role: LegacyExtensionAuthSourceRoleV1::Client,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "apps/chrome-extension/src/shared/types.ts",
        symbol: "ExtensionAuth+BootstrapData",
        sha256: "fdd5da209e33f6a28158b4a33e52e147fb03de44c8aa6cb39b6d9cc20b52ead1",
        role: LegacyExtensionAuthSourceRoleV1::Client,
    },
    LegacyExtensionAuthSourcePinV1 {
        path: "pnpm-lock.yaml",
        symbol: "Effect+Drizzle+extension dependency resolutions",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyExtensionAuthSourceRoleV1::DependencyLock,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyExtensionAuthRouteV1 {
    Start,
    Approve,
    Revoke,
    Bootstrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyExtensionAuthProfileV1 {
    pub route: LegacyExtensionAuthRouteV1,
    pub operation_id: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency_retry: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_EXTENSION_AUTH_PROFILES: &[LegacyExtensionAuthProfileV1] = &[
    LegacyExtensionAuthProfileV1 {
        route: LegacyExtensionAuthRouteV1::Start,
        operation_id: LEGACY_EXTENSION_AUTH_START_OPERATION_ID,
        method: "GET",
        path: LEGACY_EXTENSION_AUTH_START_PATH,
        success: "escaped_consent_html_or_302_login_restart_without_key_mint",
        validation: "https_nonempty_chromiumapp_host_and_pinned_extension_id",
        authorization: "optional_host_only_browser_session",
        idempotency_retry: "side_effect_free_retry_safe_and_client_key_forbidden",
        failure: "bad_redirect_400_persistence_500",
    },
    LegacyExtensionAuthProfileV1 {
        route: LegacyExtensionAuthRouteV1::Approve,
        operation_id: LEGACY_EXTENSION_AUTH_APPROVE_OPERATION_ID,
        method: "POST",
        path: LEGACY_EXTENSION_AUTH_APPROVE_PATH,
        success: "302_chromium_fragment_with_uuid_key_user_id_and_optional_state",
        validation: "bounded_urlencoded_form_redirect_and_fetch_metadata_origin_gate",
        authorization: "same_origin_consent_post_plus_optional_host_only_session",
        idempotency_retry: "non_idempotent_random_key_at_most_ten_per_user_per_hour",
        failure: "bad_request_for_csrf_redirect_or_rate_limit_and_500_persistence",
    },
    LegacyExtensionAuthProfileV1 {
        route: LegacyExtensionAuthRouteV1::Revoke,
        operation_id: LEGACY_EXTENSION_AUTH_REVOKE_OPERATION_ID,
        method: "POST",
        path: LEGACY_EXTENSION_AUTH_REVOKE_PATH,
        success: "success_false_without_bearer_else_delete_actor_owned_digest_and_true",
        validation: "authorization_second_space_segment_compatibility",
        authorization: "36_character_api_key_takes_precedence_else_browser_session",
        idempotency_retry: "physical_delete_retry_safe_success_true_after_authenticated_bearer",
        failure: "401_missing_actor_500_persistence",
    },
    LegacyExtensionAuthProfileV1 {
        route: LegacyExtensionAuthRouteV1::Bootstrap,
        operation_id: LEGACY_EXTENSION_BOOTSTRAP_OPERATION_ID,
        method: "GET",
        path: LEGACY_EXTENSION_BOOTSTRAP_PATH,
        success: "actor_org_plan_json_and_dangling_active_pointer_repair",
        validation: "bounded_live_user_org_and_subscription_projection",
        authorization: "36_character_api_key_takes_precedence_else_browser_session",
        idempotency_retry: "deterministic_active_owned_owned_oldest_member_selection",
        failure: "401_missing_actor_500_missing_org_or_persistence",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyExtensionAuthParamsV1 {
    pub redirect_uri: String,
    pub state: Option<String>,
}

impl LegacyExtensionAuthParamsV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        valid_value(&self.redirect_uri) && self.state.as_deref().is_none_or(valid_optional_value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyExtensionRedirectErrorV1 {
    Invalid,
    ExtensionIdMismatch,
    UnpinnedDeployment,
}

pub fn validate_legacy_extension_redirect_uri(
    redirect_uri: &str,
    configured_extension_id: Option<&str>,
    local_development: bool,
) -> Result<Url, LegacyExtensionRedirectErrorV1> {
    if !valid_value(redirect_uri) {
        return Err(LegacyExtensionRedirectErrorV1::Invalid);
    }
    let url = Url::parse(redirect_uri).map_err(|_| LegacyExtensionRedirectErrorV1::Invalid)?;
    let hostname = url
        .host_str()
        .ok_or(LegacyExtensionRedirectErrorV1::Invalid)?;
    let extension_id = hostname
        .strip_suffix(".chromiumapp.org")
        .filter(|value| !value.is_empty())
        .ok_or(LegacyExtensionRedirectErrorV1::Invalid)?;
    if url.scheme() != "https" {
        return Err(LegacyExtensionRedirectErrorV1::Invalid);
    }
    if let Some(configured) = configured_extension_id.filter(|value| !value.is_empty()) {
        if extension_id != configured {
            return Err(LegacyExtensionRedirectErrorV1::ExtensionIdMismatch);
        }
    } else if !local_development {
        return Err(LegacyExtensionRedirectErrorV1::UnpinnedDeployment);
    }
    Ok(url)
}

pub fn legacy_extension_consent_url(
    current_request_url: &Url,
    params: &LegacyExtensionAuthParamsV1,
) -> Result<Url, LegacyExtensionRedirectErrorV1> {
    if !params.valid() {
        return Err(LegacyExtensionRedirectErrorV1::Invalid);
    }
    let mut url = current_request_url
        .join("start")
        .map_err(|_| LegacyExtensionRedirectErrorV1::Invalid)?;
    url.set_query(None);
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("redirectUri", &params.redirect_uri);
        if let Some(state) = params.state.as_deref() {
            query.append_pair("state", state);
        }
    }
    Ok(url)
}

pub fn legacy_extension_login_url(
    web_origin: &Url,
    consent_url: &Url,
) -> Result<Url, LegacyExtensionRedirectErrorV1> {
    let mut url = web_origin
        .join("/login")
        .map_err(|_| LegacyExtensionRedirectErrorV1::Invalid)?;
    url.query_pairs_mut()
        .append_pair("next", consent_url.as_str());
    Ok(url)
}

pub fn legacy_extension_cancel_url(redirect_uri: &Url, state: Option<&str>) -> Url {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("error", "access_denied");
    if let Some(state) = state {
        serializer.append_pair("state", state);
    }
    let mut url = redirect_uri.clone();
    url.set_fragment(Some(&serializer.finish()));
    url
}

pub fn legacy_extension_approved_url(
    redirect_uri: &Url,
    auth_api_key: &str,
    user_id: &str,
    state: Option<&str>,
) -> Url {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("authApiKey", auth_api_key);
    serializer.append_pair("userId", user_id);
    if let Some(state) = state {
        serializer.append_pair("state", state);
    }
    let mut url = redirect_uri.clone();
    url.set_fragment(Some(&serializer.finish()));
    url
}

#[must_use]
pub fn escape_legacy_extension_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[must_use]
pub fn render_legacy_extension_consent_page(
    email: &str,
    redirect_uri: &Url,
    state: Option<&str>,
) -> String {
    let state_field = state.map_or_else(String::new, |state| {
        format!(
            "<input type=\"hidden\" name=\"state\" value=\"{}\" />",
            escape_legacy_extension_html(state)
        )
    });
    let cancel_url = legacy_extension_cancel_url(redirect_uri, state);
    format!(
        r#"<!doctype html>
<html lang="en">
	<head>
		<meta charset="utf-8" />
		<meta name="viewport" content="width=device-width, initial-scale=1" />
		<meta name="robots" content="noindex" />
		<title>Connect Cap</title>
		<style>
			body {{ margin: 0; min-height: 100vh; display: flex; align-items: center; justify-content: center; font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; background: #f4f4f5; color: #18181b; }}
			.card {{ background: #fff; border: 1px solid #e4e4e7; border-radius: 16px; padding: 32px; max-width: 400px; width: 100%; margin: 16px; box-shadow: 0 1px 3px rgba(0, 0, 0, 0.06); }}
			h1 {{ font-size: 18px; margin: 0 0 12px; }}
			p {{ font-size: 14px; line-height: 1.5; color: #52525b; margin: 0 0 12px; }}
			.email {{ font-weight: 600; color: #18181b; }}
			.actions {{ display: flex; gap: 12px; margin: 24px 0 0; }}
			.actions > * {{ flex: 1; display: flex; align-items: center; justify-content: center; height: 40px; border-radius: 10px; font-size: 14px; font-weight: 500; cursor: pointer; text-decoration: none; box-sizing: border-box; }}
			button {{ background: #18181b; color: #fff; border: none; }}
			a.cancel {{ background: #fff; color: #18181b; border: 1px solid #d4d4d8; }}
		</style>
	</head>
	<body>
		<main class="card">
			<h1>Connect the Cap Chrome extension</h1>
			<p>The Cap extension is asking for access to your Cap account <span class="email">{}</span> to create and upload recordings on your behalf.</p>
			<p>Only continue if you opened this page from the Cap extension.</p>
			<form method="post" action="approve" class="actions">
				<input type="hidden" name="redirectUri" value="{}" />
				{}
				<a class="cancel" href="{}">Cancel</a>
				<button type="submit">Allow access</button>
			</form>
		</main>
	</body>
</html>"#,
        escape_legacy_extension_html(email),
        escape_legacy_extension_html(redirect_uri.as_str()),
        state_field,
        escape_legacy_extension_html(cancel_url.as_str()),
    )
}

#[must_use]
pub fn legacy_extension_bearer_segment(authorization: Option<&str>) -> Option<&str> {
    authorization
        .and_then(|value| value.split(' ').nth(1))
        .filter(|value| !value.is_empty())
}

#[must_use]
pub fn legacy_extension_header_selects_api_key(authorization: Option<&str>) -> bool {
    legacy_extension_bearer_segment(authorization).is_some_and(|value| value.len() == 36)
}

#[must_use]
pub fn legacy_extension_user_is_pro(
    is_cap_hosted: bool,
    stripe_subscription_status: Option<&str>,
    third_party_stripe_subscription_id: Option<&str>,
) -> bool {
    !is_cap_hosted
        || third_party_stripe_subscription_id.is_some_and(|value| !value.is_empty())
        || matches!(
            stripe_subscription_status,
            Some("active" | "trialing" | "complete" | "paid")
        )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyExtensionBootstrapPlanV1 {
    pub is_pro: bool,
    pub max_recording_seconds: Option<u32>,
}

impl LegacyExtensionBootstrapPlanV1 {
    #[must_use]
    pub const fn from_pro(is_pro: bool) -> Self {
        Self {
            is_pro,
            max_recording_seconds: if is_pro {
                None
            } else {
                Some(LEGACY_EXTENSION_FREE_PLAN_MAX_RECORDING_SECONDS)
            },
        }
    }
}

#[must_use]
pub fn legacy_extension_auth_source_manifest() -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-extension-auth-source-manifest-v1\0");
    for source in LEGACY_EXTENSION_AUTH_SOURCES {
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

fn valid_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= LEGACY_EXTENSION_AUTH_MAX_VALUE_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_optional_value(value: &str) -> bool {
    value.len() <= LEGACY_EXTENSION_AUTH_MAX_VALUE_BYTES && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_and_five_axis_profiles_are_complete_and_provider_free() {
        assert_eq!(LEGACY_EXTENSION_AUTH_PROFILES.len(), 4);
        assert_eq!(LEGACY_EXTENSION_AUTH_SOURCES.len(), 16);
        assert_eq!(LEGACY_EXTENSION_AUTH_NO_PROTECTED_GATES, &[] as &[&str]);
        assert_eq!(
            legacy_extension_auth_source_manifest(),
            LEGACY_EXTENSION_AUTH_SOURCE_MANIFEST_SHA256
        );
        assert!(LEGACY_EXTENSION_AUTH_PROFILES.iter().all(|profile| {
            !profile.success.is_empty()
                && !profile.validation.is_empty()
                && !profile.authorization.is_empty()
                && !profile.idempotency_retry.is_empty()
                && !profile.failure.is_empty()
        }));
    }

    #[test]
    fn redirect_validation_requires_https_chromium_and_a_production_pin() {
        let valid = "https://abcdefghijklmnop.chromiumapp.org/callback";
        assert!(
            validate_legacy_extension_redirect_uri(valid, Some("abcdefghijklmnop"), false).is_ok()
        );
        assert_eq!(
            validate_legacy_extension_redirect_uri(valid, Some("other"), false),
            Err(LegacyExtensionRedirectErrorV1::ExtensionIdMismatch)
        );
        assert_eq!(
            validate_legacy_extension_redirect_uri(valid, None, false),
            Err(LegacyExtensionRedirectErrorV1::UnpinnedDeployment)
        );
        assert!(validate_legacy_extension_redirect_uri(valid, None, true).is_ok());
        for rejected in [
            "http://abcdefghijklmnop.chromiumapp.org/callback",
            "https://abcdefghijklmnop.chromiumapp.org.evil.example/callback",
            "https://.chromiumapp.org/callback",
        ] {
            assert_eq!(
                validate_legacy_extension_redirect_uri(rejected, None, true),
                Err(LegacyExtensionRedirectErrorV1::Invalid)
            );
        }
    }

    #[test]
    fn consent_success_escapes_every_reflected_value_and_cancel_uses_fragment() {
        let redirect = Url::parse("https://abcdefghijklmnop.chromiumapp.org/cb?x=1").expect("url");
        let page = render_legacy_extension_consent_page(
            "a<&\"'@example.com",
            &redirect,
            Some("<script>&\"'"),
        );
        assert!(page.contains("a&lt;&amp;&quot;&#39;@example.com"));
        assert!(page.contains("value=\"&lt;script&gt;&amp;&quot;&#39;\""));
        assert!(!page.contains("<script>"));
        assert!(page.contains("#error=access_denied&amp;state=%3Cscript%3E%26%22%27"));
        assert!(page.contains("method=\"post\" action=\"approve\""));
    }

    #[test]
    fn expired_session_restart_and_approval_fragments_preserve_state() {
        let current = Url::parse("https://frame.example/api/extension/auth/approve").expect("url");
        let params = LegacyExtensionAuthParamsV1 {
            redirect_uri: "https://abcdefghijklmnop.chromiumapp.org/cb".into(),
            state: Some("a b&c".into()),
        };
        let consent = legacy_extension_consent_url(&current, &params).expect("consent");
        assert_eq!(consent.path(), LEGACY_EXTENSION_AUTH_START_PATH);
        assert_eq!(
            consent.query(),
            Some("redirectUri=https%3A%2F%2Fabcdefghijklmnop.chromiumapp.org%2Fcb&state=a+b%26c")
        );
        let login = legacy_extension_login_url(
            &Url::parse("https://frame.example/").expect("origin"),
            &consent,
        )
        .expect("login");
        assert_eq!(login.path(), "/login");
        assert_eq!(
            login
                .query_pairs()
                .find(|(key, _)| key == "next")
                .map(|(_, value)| value.into_owned()),
            Some(consent.to_string())
        );
        let approved = legacy_extension_approved_url(
            &Url::parse(&params.redirect_uri).expect("redirect"),
            "00000000-0000-4000-8000-000000000001",
            "actor",
            params.state.as_deref(),
        );
        assert_eq!(
            approved.fragment(),
            Some("authApiKey=00000000-0000-4000-8000-000000000001&userId=actor&state=a+b%26c")
        );
    }

    #[test]
    fn auth_precedence_entitlements_and_retry_contract_match_the_source() {
        assert_eq!(
            legacy_extension_bearer_segment(Some("Bearer key")),
            Some("key")
        );
        assert_eq!(legacy_extension_bearer_segment(Some("Bearer")), None);
        assert!(legacy_extension_header_selects_api_key(Some(
            "anything 00000000-0000-4000-8000-000000000001"
        )));
        assert!(!legacy_extension_header_selects_api_key(Some(
            "Bearer short"
        )));
        for status in ["active", "trialing", "complete", "paid"] {
            assert!(legacy_extension_user_is_pro(true, Some(status), None));
        }
        assert!(legacy_extension_user_is_pro(
            true,
            None,
            Some("third-party")
        ));
        assert!(!legacy_extension_user_is_pro(true, Some("canceled"), None));
        assert!(legacy_extension_user_is_pro(false, Some("canceled"), None));
        assert_eq!(
            LegacyExtensionBootstrapPlanV1::from_pro(false).max_recording_seconds,
            Some(300)
        );
        assert_eq!(
            LegacyExtensionBootstrapPlanV1::from_pro(true).max_recording_seconds,
            None
        );
        assert_eq!(LEGACY_EXTENSION_AUTH_KEY_MINT_LIMIT, 10);
        assert_eq!(LEGACY_EXTENSION_AUTH_KEY_MINT_WINDOW_MS, 3_600_000);
    }
}
