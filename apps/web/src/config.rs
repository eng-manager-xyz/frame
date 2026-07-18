use std::{
    env, fmt,
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use axum::http::uri::Authority;

const PRODUCTION_ORIGIN: &str = "https://frame.engmanager.xyz";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    Local,
    Preview,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyTrust {
    /// Direct loopback development. Forwarding metadata is rejected so a
    /// local reverse proxy cannot silently become an identity authority.
    Direct,
    /// Render terminates TLS, but Frame still derives neither host nor client
    /// identity from forwarding headers. Only the exact HTTPS scheme marker
    /// is admitted when Render supplies it.
    RenderEdge,
}

impl ProxyTrust {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::RenderEdge => "render",
        }
    }
}

impl Deployment {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Preview => "preview",
            Self::Production => "production",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Origin {
    serialized: String,
    authority: String,
    host: String,
}

impl Origin {
    fn parse(value: &str, allow_loopback_http: bool) -> Result<Self, ConfigError> {
        if value.is_empty()
            || value.trim() != value
            || !value.is_ascii()
            || value.contains(['\\', '@', '?', '#'])
            || value.starts_with("//")
        {
            return Err(ConfigError::InvalidOrigin);
        }

        let (scheme, remainder) = value.split_once("://").ok_or(ConfigError::InvalidOrigin)?;
        if !matches!(scheme, "https" | "http") {
            return Err(ConfigError::InvalidOrigin);
        }
        let authority_text = remainder.strip_suffix('/').unwrap_or(remainder);
        if authority_text.is_empty() || authority_text.contains('/') {
            return Err(ConfigError::InvalidOrigin);
        }

        let authority =
            Authority::from_str(authority_text).map_err(|_| ConfigError::InvalidOrigin)?;
        let host = authority.host().to_ascii_lowercase();
        if host.is_empty()
            || host.split('.').any(|label| label.starts_with("xn--"))
            || authority.port_u16().is_some() && !is_loopback_host(&host)
            || !allow_loopback_http
                && (host.parse::<IpAddr>().is_ok()
                    || !host.contains('.')
                    || host.split('.').any(str::is_empty))
        {
            return Err(ConfigError::InvalidOrigin);
        }
        if scheme == "http" && !(allow_loopback_http && is_loopback_host(&host)) {
            return Err(ConfigError::InsecureOrigin);
        }

        let authority = normalize_authority(&authority);
        Ok(Self {
            serialized: format!("{scheme}://{authority}"),
            authority,
            host,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.serialized
    }

    #[must_use]
    pub fn authority(&self) -> &str {
        &self.authority
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }
}

impl fmt::Debug for Origin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Origin(<validated>)")
    }
}

#[derive(Clone)]
pub struct EmbedPolicy {
    enabled: bool,
    ancestors: Vec<Origin>,
}

impl EmbedPolicy {
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn ancestors(&self) -> &[Origin] {
        &self.ancestors
    }
}

#[derive(Clone)]
pub struct RuntimeConfig {
    bind: SocketAddr,
    deployment: Deployment,
    public_origin: Origin,
    api_origin: Origin,
    allowed_hosts: Vec<String>,
    release_id: String,
    release_join: Option<ReleaseJoin>,
    proxy_trust: ProxyTrust,
    diagnostic_token: Option<String>,
    embed: EmbedPolicy,
    runtime_test_mode: bool,
}

#[derive(Clone)]
pub struct ReleaseJoin {
    source_git_sha: String,
    worker_release: String,
    render_deploy: String,
    migration_level: String,
    portfolio_consumer: String,
}

impl ReleaseJoin {
    #[must_use]
    pub fn source_git_sha(&self) -> &str {
        &self.source_git_sha
    }

    #[must_use]
    pub fn worker_release(&self) -> &str {
        &self.worker_release
    }

    #[must_use]
    pub fn render_deploy(&self) -> &str {
        &self.render_deploy
    }

    #[must_use]
    pub fn migration_level(&self) -> &str {
        &self.migration_level
    }

    #[must_use]
    pub fn portfolio_consumer(&self) -> &str {
        &self.portfolio_consumer
    }
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(ConfigValues {
            frame_addr: env::var("FRAME_ADDR").ok(),
            port: env::var("PORT").ok(),
            deployment: env::var("FRAME_DEPLOYMENT").ok(),
            is_pull_request: env::var("IS_PULL_REQUEST").ok(),
            public_origin: env::var("FRAME_PUBLIC_ORIGIN").ok(),
            api_origin: env::var("FRAME_API_ORIGIN").ok(),
            render_external_url: env::var("RENDER_EXTERNAL_URL").ok(),
            release_id: env::var("FRAME_RELEASE_ID")
                .ok()
                .or_else(|| env::var("RENDER_GIT_COMMIT").ok()),
            proxy_trust: env::var("FRAME_PROXY_TRUST").ok(),
            diagnostic_token: env::var("FRAME_DIAGNOSTIC_TOKEN").ok(),
            worker_release: env::var("FRAME_WORKER_RELEASE").ok(),
            render_deploy: env::var("FRAME_RENDER_DEPLOY").ok(),
            migration_level: env::var("FRAME_MIGRATION_LEVEL").ok(),
            portfolio_consumer: env::var("FRAME_PORTFOLIO_CONSUMER").ok(),
            public_embed_enabled: env::var("FRAME_ENABLE_PUBLIC_EMBED").ok(),
            embed_ancestors: env::var("FRAME_EMBED_ANCESTORS").ok(),
            runtime_test_mode: env::var("FRAME_RUNTIME_TEST_MODE").ok(),
        })
    }

    pub fn from_values(values: ConfigValues) -> Result<Self, ConfigError> {
        let pull_request = parse_bool(values.is_pull_request.as_deref(), "IS_PULL_REQUEST")?;
        let deployment = match values.deployment.as_deref() {
            Some("local") | None if !pull_request => Deployment::Local,
            Some("preview") | None => Deployment::Preview,
            Some("production") if !pull_request => Deployment::Production,
            Some(_) => return Err(ConfigError::InvalidDeployment),
        };

        let bind = resolve_bind(values.frame_addr.as_deref(), values.port.as_deref())?;
        let proxy_trust = match (deployment, values.proxy_trust.as_deref()) {
            (Deployment::Local, None | Some("direct")) => ProxyTrust::Direct,
            (Deployment::Preview | Deployment::Production, Some("render")) => {
                ProxyTrust::RenderEdge
            }
            _ => return Err(ConfigError::InvalidProxyTrust),
        };
        let runtime_test_mode = parse_bool(
            values.runtime_test_mode.as_deref(),
            "FRAME_RUNTIME_TEST_MODE",
        )?;
        if runtime_test_mode && deployment != Deployment::Local {
            return Err(ConfigError::RuntimeTestModeNonLocal);
        }
        let render_origin = values
            .render_external_url
            .as_deref()
            .map(|value| Origin::parse(value, false))
            .transpose()?;

        let (public_origin, api_origin) = match deployment {
            Deployment::Production => {
                let public = Origin::parse(
                    values
                        .public_origin
                        .as_deref()
                        .ok_or(ConfigError::Missing("FRAME_PUBLIC_ORIGIN"))?,
                    false,
                )?;
                if public.as_str() != PRODUCTION_ORIGIN {
                    return Err(ConfigError::WrongProductionOrigin);
                }
                let api = Origin::parse(
                    values
                        .api_origin
                        .as_deref()
                        .ok_or(ConfigError::Missing("FRAME_API_ORIGIN"))?,
                    false,
                )?;
                if api != public {
                    return Err(ConfigError::CrossOriginProductionApi);
                }
                (public, api)
            }
            Deployment::Preview => {
                // Render preview URLs are authoritative. FRAME_PUBLIC_ORIGIN may
                // intentionally contain a fail-closed sentinel in render.yaml.
                let public = render_origin
                    .clone()
                    .ok_or(ConfigError::Missing("RENDER_EXTERNAL_URL"))?;
                if public.as_str() == PRODUCTION_ORIGIN {
                    return Err(ConfigError::PreviewUsesProduction);
                }
                let api = Origin::parse(
                    values
                        .api_origin
                        .as_deref()
                        .ok_or(ConfigError::Missing("FRAME_API_ORIGIN"))?,
                    false,
                )?;
                if api.as_str() == PRODUCTION_ORIGIN {
                    return Err(ConfigError::PreviewUsesProduction);
                }
                (public, api)
            }
            Deployment::Local => {
                let default = format!("http://127.0.0.1:{}", bind.port());
                let public =
                    Origin::parse(values.public_origin.as_deref().unwrap_or(&default), true)?;
                let api = Origin::parse(
                    values.api_origin.as_deref().unwrap_or(public.as_str()),
                    true,
                )?;
                (public, api)
            }
        };

        let release_id = values.release_id.unwrap_or_else(|| "development".into());
        if release_id.is_empty()
            || release_id.len() > 64
            || !release_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ConfigError::InvalidReleaseId);
        }

        let release_join = parse_release_join(
            &release_id,
            values.worker_release,
            values.render_deploy,
            values.migration_level,
            values.portfolio_consumer,
        )?;

        let diagnostic_token = values
            .diagnostic_token
            .filter(|value| !value.trim().is_empty());
        if deployment != Deployment::Local
            && diagnostic_token
                .as_ref()
                .is_some_and(|token| token.len() < 24 || !token.is_ascii())
        {
            return Err(ConfigError::WeakDiagnosticToken);
        }

        let embed_enabled = parse_bool(
            values.public_embed_enabled.as_deref(),
            "FRAME_ENABLE_PUBLIC_EMBED",
        )?;
        let ancestors = values
            .embed_ancestors
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .filter(|value| !value.trim().is_empty())
            .map(|value| Origin::parse(value.trim(), deployment == Deployment::Local))
            .collect::<Result<Vec<_>, _>>()?;
        if embed_enabled && ancestors.is_empty() {
            return Err(ConfigError::EmbedWithoutAncestors);
        }

        let mut allowed_hosts = vec![public_origin.authority().to_owned()];
        if let Some(render_origin) = render_origin {
            push_unique(&mut allowed_hosts, render_origin.authority());
        }
        if deployment == Deployment::Local {
            for host in [
                format!("127.0.0.1:{}", bind.port()),
                format!("localhost:{}", bind.port()),
                format!("[::1]:{}", bind.port()),
            ] {
                push_unique(&mut allowed_hosts, &host);
            }
        }

        Ok(Self {
            bind,
            deployment,
            public_origin,
            api_origin,
            allowed_hosts,
            release_id,
            release_join,
            proxy_trust,
            diagnostic_token,
            embed: EmbedPolicy {
                enabled: embed_enabled,
                ancestors,
            },
            runtime_test_mode,
        })
    }

    #[must_use]
    pub const fn bind_address(&self) -> SocketAddr {
        self.bind
    }

    #[must_use]
    pub const fn deployment(&self) -> Deployment {
        self.deployment
    }

    #[must_use]
    pub fn public_origin(&self) -> &Origin {
        &self.public_origin
    }

    #[must_use]
    pub fn api_origin(&self) -> &Origin {
        &self.api_origin
    }

    #[must_use]
    pub fn release_id(&self) -> &str {
        &self.release_id
    }

    #[must_use]
    pub fn release_join(&self) -> Option<&ReleaseJoin> {
        self.release_join.as_ref()
    }

    #[must_use]
    pub const fn proxy_trust(&self) -> ProxyTrust {
        self.proxy_trust
    }

    #[must_use]
    pub fn diagnostic_token(&self) -> Option<&str> {
        self.diagnostic_token.as_deref()
    }

    #[must_use]
    pub fn embed_policy(&self) -> &EmbedPolicy {
        &self.embed
    }

    #[must_use]
    pub const fn runtime_test_mode(&self) -> bool {
        self.runtime_test_mode
    }

    #[must_use]
    pub fn host_is_allowed(&self, host: &str) -> bool {
        normalize_host_header(host).is_some_and(|host| self.allowed_hosts.contains(&host))
    }

    #[must_use]
    pub fn host_is_canonical(&self, host: &str) -> bool {
        normalize_host_header(host).is_some_and(|host| host == self.public_origin.authority())
    }
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfig")
            .field("bind", &self.bind)
            .field("deployment", &self.deployment)
            .field("public_origin", &self.public_origin)
            .field("api_origin", &self.api_origin)
            .field("release_id", &self.release_id)
            .field("proxy_trust", &self.proxy_trust)
            .field(
                "diagnostic_token",
                &self.diagnostic_token.as_ref().map(|_| "<redacted>"),
            )
            .field("embed_enabled", &self.embed.enabled)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
pub struct ConfigValues {
    pub frame_addr: Option<String>,
    pub port: Option<String>,
    pub deployment: Option<String>,
    pub is_pull_request: Option<String>,
    pub public_origin: Option<String>,
    pub api_origin: Option<String>,
    pub render_external_url: Option<String>,
    pub release_id: Option<String>,
    pub proxy_trust: Option<String>,
    pub diagnostic_token: Option<String>,
    pub worker_release: Option<String>,
    pub render_deploy: Option<String>,
    pub migration_level: Option<String>,
    pub portfolio_consumer: Option<String>,
    pub public_embed_enabled: Option<String>,
    pub embed_ancestors: Option<String>,
    pub runtime_test_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    Missing(&'static str),
    InvalidAddress,
    InvalidPort,
    InvalidDeployment,
    InvalidBoolean(&'static str),
    InvalidOrigin,
    InsecureOrigin,
    WrongProductionOrigin,
    CrossOriginProductionApi,
    PreviewUsesProduction,
    InvalidReleaseId,
    IncompleteReleaseJoin,
    InvalidReleaseJoin,
    InvalidProxyTrust,
    WeakDiagnosticToken,
    EmbedWithoutAncestors,
    RuntimeTestModeNonLocal,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(formatter, "required configuration {name} is missing"),
            Self::InvalidAddress => formatter.write_str("FRAME_ADDR must be a socket address"),
            Self::InvalidPort => formatter.write_str("PORT must be a non-zero TCP port"),
            Self::InvalidDeployment => formatter.write_str(
                "FRAME_DEPLOYMENT must be local, preview, or production and agree with IS_PULL_REQUEST",
            ),
            Self::InvalidBoolean(name) => write!(formatter, "{name} must be true or false"),
            Self::InvalidOrigin => formatter.write_str("an origin is not an absolute unambiguous origin"),
            Self::InsecureOrigin => formatter.write_str("HTTP origins are allowed only for loopback local development"),
            Self::WrongProductionOrigin => formatter.write_str("production public origin must be the canonical Frame origin"),
            Self::CrossOriginProductionApi => formatter.write_str("production web and API origins must be identical"),
            Self::PreviewUsesProduction => formatter.write_str("preview configuration cannot use a production origin"),
            Self::InvalidReleaseId => formatter.write_str("release identifier contains unsupported data"),
            Self::IncompleteReleaseJoin => formatter.write_str(
                "release join requires Worker, Render, migration, and portfolio identifiers together",
            ),
            Self::InvalidReleaseJoin => formatter.write_str(
                "release join identifiers are not bounded safe deployment metadata",
            ),
            Self::InvalidProxyTrust => formatter.write_str(
                "FRAME_PROXY_TRUST must be direct for local or render for preview/production",
            ),
            Self::WeakDiagnosticToken => formatter.write_str("diagnostic token does not meet the production minimum"),
            Self::EmbedWithoutAncestors => formatter.write_str("public embed requires at least one exact ancestor origin"),
            Self::RuntimeTestModeNonLocal => formatter.write_str(
                "runtime test mode is allowed only for loopback local deployment",
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

fn parse_release_join(
    source_git_sha: &str,
    worker_release: Option<String>,
    render_deploy: Option<String>,
    migration_level: Option<String>,
    portfolio_consumer: Option<String>,
) -> Result<Option<ReleaseJoin>, ConfigError> {
    let configured = [
        worker_release.is_some(),
        render_deploy.is_some(),
        migration_level.is_some(),
        portfolio_consumer.is_some(),
    ];
    if configured.iter().all(|present| !present) {
        return Ok(None);
    }
    if !configured.iter().all(|present| *present) {
        return Err(ConfigError::IncompleteReleaseJoin);
    }
    let worker_release = worker_release.ok_or(ConfigError::IncompleteReleaseJoin)?;
    let render_deploy = render_deploy.ok_or(ConfigError::IncompleteReleaseJoin)?;
    let migration_level = migration_level.ok_or(ConfigError::IncompleteReleaseJoin)?;
    let portfolio_consumer = portfolio_consumer.ok_or(ConfigError::IncompleteReleaseJoin)?;
    if !is_git_sha(source_git_sha)
        || !is_safe_release_identifier(&worker_release)
        || !is_safe_release_identifier(&render_deploy)
        || !is_migration_level(&migration_level)
        || !is_safe_release_identifier(&portfolio_consumer)
    {
        return Err(ConfigError::InvalidReleaseJoin);
    }
    Ok(Some(ReleaseJoin {
        source_git_sha: source_git_sha.into(),
        worker_release,
        render_deploy,
        migration_level,
        portfolio_consumer,
    }))
}

fn is_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_safe_release_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_migration_level(value: &str) -> bool {
    let Some((number, name)) = value.split_once('_') else {
        return false;
    };
    number.len() == 4
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && name.ends_with(".sql")
        && name.strip_suffix(".sql").is_some_and(|stem| {
            !stem.is_empty()
                && stem
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
        && value.len() <= 64
}

fn resolve_bind(frame_addr: Option<&str>, port: Option<&str>) -> Result<SocketAddr, ConfigError> {
    if let Some(frame_addr) = frame_addr {
        return frame_addr.parse().map_err(|_| ConfigError::InvalidAddress);
    }
    if let Some(port) = port {
        let port = port.parse::<u16>().map_err(|_| ConfigError::InvalidPort)?;
        if port == 0 {
            return Err(ConfigError::InvalidPort);
        }
        return Ok(SocketAddr::from(([0, 0, 0, 0], port)));
    }
    Ok(SocketAddr::from(([127, 0, 0, 1], 3000)))
}

fn parse_bool(value: Option<&str>, name: &'static str) -> Result<bool, ConfigError> {
    match value {
        None | Some("") | Some("false") | Some("0") => Ok(false),
        Some("true") | Some("1") => Ok(true),
        Some(_) => Err(ConfigError::InvalidBoolean(name)),
    }
}

fn normalize_authority(authority: &Authority) -> String {
    let host = authority.host().to_ascii_lowercase();
    match authority.port_u16() {
        Some(port) if host.contains(':') => format!("[{host}]:{port}"),
        Some(port) => format!("{host}:{port}"),
        None if host.contains(':') => format!("[{host}]"),
        None => host,
    }
}

fn normalize_host_header(value: &str) -> Option<String> {
    if value.trim() != value || !value.is_ascii() || value.contains([',', '@', '\\']) {
        return None;
    }
    Authority::from_str(value)
        .ok()
        .map(|authority| normalize_authority(&authority))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn production_values() -> ConfigValues {
        ConfigValues {
            deployment: Some("production".into()),
            public_origin: Some(PRODUCTION_ORIGIN.into()),
            api_origin: Some(PRODUCTION_ORIGIN.into()),
            release_id: Some("abc123".into()),
            proxy_trust: Some("render".into()),
            ..ConfigValues::default()
        }
    }

    #[test]
    fn frame_addr_takes_precedence_over_port() {
        let config = RuntimeConfig::from_values(ConfigValues {
            frame_addr: Some("127.0.0.1:4100".into()),
            port: Some("10000".into()),
            ..ConfigValues::default()
        })
        .expect("local config");
        assert_eq!(
            config.bind_address(),
            "127.0.0.1:4100".parse().expect("test address")
        );
    }

    #[test]
    fn port_binds_all_interfaces() {
        let config = RuntimeConfig::from_values(ConfigValues {
            port: Some("10000".into()),
            ..ConfigValues::default()
        })
        .expect("local config");
        assert_eq!(
            config.bind_address(),
            "0.0.0.0:10000".parse().expect("test address")
        );
    }

    #[test]
    fn production_requires_canonical_same_origin() {
        assert!(RuntimeConfig::from_values(production_values()).is_ok());
        let mut missing = production_values();
        missing.public_origin = None;
        assert_eq!(
            RuntimeConfig::from_values(missing).expect_err("missing public origin"),
            ConfigError::Missing("FRAME_PUBLIC_ORIGIN")
        );
        let mut split = production_values();
        split.api_origin = Some("https://api.engmanager.xyz".into());
        assert_eq!(
            RuntimeConfig::from_values(split).expect_err("split production origin"),
            ConfigError::CrossOriginProductionApi
        );
    }

    #[test]
    fn release_join_is_all_or_nothing_and_bounded() {
        let mut incomplete = production_values();
        incomplete.release_id = Some("1".repeat(40));
        incomplete.worker_release = Some("worker-release".into());
        assert_eq!(
            RuntimeConfig::from_values(incomplete).expect_err("incomplete release join"),
            ConfigError::IncompleteReleaseJoin
        );

        let mut complete = production_values();
        complete.release_id = Some("1".repeat(40));
        complete.worker_release = Some("worker-1111111".into());
        complete.render_deploy = Some("render-deploy-1".into());
        complete.migration_level = Some("0063_legacy_protected_billing_auth_expand.sql".into());
        complete.portfolio_consumer = Some("portfolio-aaaaaaa".into());
        let config = RuntimeConfig::from_values(complete).expect("complete safe release join");
        let release = config.release_join().expect("configured release join");
        assert_eq!(release.source_git_sha(), "1".repeat(40));
        assert_eq!(release.worker_release(), "worker-1111111");
        assert_eq!(release.render_deploy(), "render-deploy-1");
        assert_eq!(
            release.migration_level(),
            "0063_legacy_protected_billing_auth_expand.sql"
        );
        assert_eq!(release.portfolio_consumer(), "portfolio-aaaaaaa");

        let mut unsafe_value = production_values();
        unsafe_value.release_id = Some("1".repeat(40));
        unsafe_value.worker_release = Some("worker?secret".into());
        unsafe_value.render_deploy = Some("render-deploy-1".into());
        unsafe_value.migration_level = Some("0063_legacy_protected_billing_auth_expand.sql".into());
        unsafe_value.portfolio_consumer = Some("portfolio-aaaaaaa".into());
        assert_eq!(
            RuntimeConfig::from_values(unsafe_value).expect_err("unsafe release join"),
            ConfigError::InvalidReleaseJoin
        );
    }

    #[test]
    fn preview_derives_public_origin_and_rejects_production_api() {
        let preview = RuntimeConfig::from_values(ConfigValues {
            deployment: Some("preview".into()),
            is_pull_request: Some("true".into()),
            public_origin: Some("https://frame-preview.invalid".into()),
            render_external_url: Some("https://frame-pr-7.onrender.com".into()),
            api_origin: Some("https://frame-staging.engmanager.xyz".into()),
            proxy_trust: Some("render".into()),
            ..ConfigValues::default()
        })
        .expect("preview config");
        assert_eq!(
            preview.public_origin().as_str(),
            "https://frame-pr-7.onrender.com"
        );

        let error = RuntimeConfig::from_values(ConfigValues {
            deployment: Some("preview".into()),
            is_pull_request: Some("true".into()),
            render_external_url: Some("https://frame-pr-7.onrender.com".into()),
            api_origin: Some(PRODUCTION_ORIGIN.into()),
            proxy_trust: Some("render".into()),
            ..ConfigValues::default()
        })
        .expect_err("preview production API");
        assert_eq!(error, ConfigError::PreviewUsesProduction);
    }

    #[test]
    fn origins_reject_credentials_paths_unicode_and_public_http() {
        for origin in [
            "https://user@frame.engmanager.xyz",
            "https://frame.engmanager.xyz/path",
            "https://frame.engmanager.xyz?token=secret",
            "https://främe.engmanager.xyz",
            "https://xn--frme-0ra.engmanager.xyz",
            "//frame.engmanager.xyz",
            "http://frame.engmanager.xyz",
            "https://frame.engmanager.xyz:8443",
        ] {
            assert!(Origin::parse(origin, false).is_err(), "accepted {origin}");
        }
        assert!(Origin::parse("http://localhost:3000", true).is_ok());
        assert!(Origin::parse("http://127.0.0.1:3000", true).is_ok());
    }

    #[test]
    fn embed_is_disabled_unless_exact_ancestors_are_configured() {
        let error = RuntimeConfig::from_values(ConfigValues {
            public_embed_enabled: Some("true".into()),
            ..ConfigValues::default()
        })
        .expect_err("embed without ancestors");
        assert_eq!(error, ConfigError::EmbedWithoutAncestors);

        let config = RuntimeConfig::from_values(ConfigValues {
            public_embed_enabled: Some("true".into()),
            embed_ancestors: Some("https://engmanager.xyz".into()),
            ..ConfigValues::default()
        })
        .expect("explicit embed policy");
        assert!(config.embed_policy().enabled());
    }

    #[test]
    fn proxy_trust_and_runtime_test_mode_fail_closed() {
        let mut production = production_values();
        production.proxy_trust = None;
        assert_eq!(
            RuntimeConfig::from_values(production).expect_err("missing production proxy mode"),
            ConfigError::InvalidProxyTrust
        );

        let local_test = RuntimeConfig::from_values(ConfigValues {
            runtime_test_mode: Some("true".into()),
            ..ConfigValues::default()
        })
        .expect("local runtime test mode");
        assert!(local_test.runtime_test_mode());

        let mut production_test = production_values();
        production_test.runtime_test_mode = Some("true".into());
        assert_eq!(
            RuntimeConfig::from_values(production_test).expect_err("nonlocal runtime test mode"),
            ConfigError::RuntimeTestModeNonLocal
        );
    }
}
