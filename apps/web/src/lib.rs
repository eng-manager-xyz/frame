pub mod hydration;

#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod config;
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod pages;
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod product;

#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod server {

    use std::sync::Arc;

    use axum::{
        Json, Router,
        body::Body,
        extract::{Form, Path, Query, State},
        http::{
            HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, Uri,
            header::{self, HOST},
        },
        middleware::{self, Next},
        response::{IntoResponse, Redirect, Response},
        routing::get,
    };
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};

    pub use crate::config::{ConfigError, ConfigValues, Deployment, RuntimeConfig};
    use crate::pages::{self, Page, SignInState};
    use crate::product::{AuthenticatedRoute, local_authenticated_fixture, local_share_fixture};

    const HYDRATION_HEAD_MARKER: &str = "<!--FRAME_HYDRATION_HEAD-->";
    const HYDRATION_SCRIPT_MARKER: &str = "<!--FRAME_HYDRATION_SCRIPT-->";
    const HYDRATION_MANIFEST_SCHEMA: &str = "frame.web-hydration-manifest.v1";

    #[derive(Deserialize)]
    struct HydrationManifest {
        schema: String,
        bootstrap: ManifestAsset,
        javascript: ManifestAsset,
        wasm: ManifestAsset,
    }

    #[derive(Deserialize)]
    struct ManifestAsset {
        file: String,
        sha256: String,
    }

    #[derive(Clone)]
    struct HydrationAsset {
        file: String,
        bytes: Arc<[u8]>,
        content_type: &'static str,
    }

    #[derive(Clone, Default)]
    struct HydrationAssets {
        bootstrap: Option<HydrationAsset>,
        javascript: Option<HydrationAsset>,
        wasm: Option<HydrationAsset>,
    }

    impl HydrationAssets {
        fn load(deployment: Deployment) -> Self {
            let directories = hydration_asset_directories(
                deployment,
                std::env::var_os("FRAME_WEB_ASSET_DIR").map(std::path::PathBuf::from),
                std::env::current_exe().ok(),
            );
            directories
                .into_iter()
                .find_map(|directory| Self::load_from(&directory))
                .unwrap_or_default()
        }

        fn load_from(directory: &std::path::Path) -> Option<Self> {
            if std::fs::symlink_metadata(directory)
                .ok()?
                .file_type()
                .is_symlink()
            {
                return None;
            }
            let manifest_path = directory.join("manifest.json");
            if std::fs::symlink_metadata(&manifest_path)
                .ok()?
                .file_type()
                .is_symlink()
            {
                return None;
            }
            let manifest_bytes = std::fs::read(manifest_path).ok()?;
            if manifest_bytes.len() > 16 * 1024 {
                return None;
            }
            let manifest: HydrationManifest = serde_json::from_slice(&manifest_bytes).ok()?;
            if manifest.schema != HYDRATION_MANIFEST_SCHEMA {
                return None;
            }
            let bootstrap = load_hydration_asset(
                directory,
                &manifest.bootstrap,
                "frame-web-bootstrap",
                "js",
                "text/javascript; charset=utf-8",
                16 * 1024,
            )?;
            let javascript = load_hydration_asset(
                directory,
                &manifest.javascript,
                "frame-web-hydrate",
                "js",
                "text/javascript; charset=utf-8",
                500_000,
            )?;
            let wasm = load_hydration_asset(
                directory,
                &manifest.wasm,
                "frame-web-hydrate_bg",
                "wasm",
                "application/wasm",
                2_000_000,
            )?;
            if std::str::from_utf8(&bootstrap.bytes).is_err()
                || std::str::from_utf8(&javascript.bytes).is_err()
                || !wasm.bytes.starts_with(b"\0asm")
            {
                return None;
            }
            Some(Self {
                bootstrap: Some(bootstrap),
                javascript: Some(javascript),
                wasm: Some(wasm),
            })
        }

        fn inject(&self, mut document: String) -> String {
            let (Some(bootstrap), Some(javascript), Some(_wasm)) =
                (&self.bootstrap, &self.javascript, &self.wasm)
            else {
                return document
                    .replace(HYDRATION_HEAD_MARKER, "")
                    .replace(HYDRATION_SCRIPT_MARKER, "");
            };
            let head = format!(
                "<link rel=\"modulepreload\" href=\"/assets/{}\">",
                javascript.file
            );
            let script = format!(
                "<script type=\"module\" src=\"/assets/{}\"></script>",
                bootstrap.file
            );
            document = document.replace(HYDRATION_HEAD_MARKER, &head);
            document.replace(HYDRATION_SCRIPT_MARKER, &script)
        }

        fn get(&self, file: &str) -> Option<HydrationAsset> {
            [&self.bootstrap, &self.javascript, &self.wasm]
                .into_iter()
                .flatten()
                .find(|asset| asset.file == file)
                .cloned()
        }

        fn is_complete(&self) -> bool {
            self.bootstrap.is_some() && self.javascript.is_some() && self.wasm.is_some()
        }
    }

    fn hydration_asset_directories(
        deployment: Deployment,
        explicit: Option<std::path::PathBuf>,
        executable: Option<std::path::PathBuf>,
    ) -> Vec<std::path::PathBuf> {
        if let Some(explicit) = explicit {
            if deployment == Deployment::Local {
                return vec![explicit];
            }
            return Vec::new();
        }

        let mut directories = executable
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(|parent| vec![parent.join("web-dist")])
            .unwrap_or_default();
        if deployment == Deployment::Local {
            directories.push(std::path::PathBuf::from("apps/web/dist"));
        }
        directories
    }

    fn load_hydration_asset(
        directory: &std::path::Path,
        manifest: &ManifestAsset,
        prefix: &str,
        extension: &str,
        content_type: &'static str,
        maximum_bytes: usize,
    ) -> Option<HydrationAsset> {
        if manifest.sha256.len() != 64
            || !manifest
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || manifest.file != format!("{prefix}-{}.{extension}", manifest.sha256)
        {
            return None;
        }
        let asset_path = directory.join(&manifest.file);
        if std::fs::symlink_metadata(&asset_path)
            .ok()?
            .file_type()
            .is_symlink()
        {
            return None;
        }
        let bytes = std::fs::read(asset_path).ok()?;
        if bytes.len() > maximum_bytes || format!("{:x}", Sha256::digest(&bytes)) != manifest.sha256
        {
            return None;
        }
        Some(HydrationAsset {
            file: manifest.file.clone(),
            bytes: bytes.into(),
            content_type,
        })
    }

    #[derive(Clone)]
    struct AppState {
        config: Arc<RuntimeConfig>,
        hydration_assets: HydrationAssets,
    }

    pub fn build_app(config: RuntimeConfig) -> Router {
        let hydration_assets = HydrationAssets::load(config.deployment());
        let state = AppState {
            config: Arc::new(config),
            hydration_assets,
        };
        Router::new()
            .route("/", get(landing))
            .route("/login", get(login).post(login_submit))
            .route("/dashboard", get(dashboard))
            .route("/library", get(library))
            .route("/spaces", get(spaces))
            .route("/folders", get(folders))
            .route("/imports", get(imports))
            .route("/settings", get(settings))
            .route("/developer", get(developer))
            .route("/billing", get(billing))
            .route("/admin", get(admin))
            .route("/s/{video_id}", get(share))
            .route("/embed/{video_id}", get(embed))
            .route("/robots.txt", get(robots))
            .route("/sitemap.xml", get(sitemap))
            .route("/assets/{asset}", get(hydration_asset))
            .route("/health", get(legacy_health_redirect))
            .route("/health/live", get(liveness))
            .route("/health/ready", get(readiness))
            .route("/health/dependencies", get(dependency_health))
            .fallback(not_found)
            .layer(middleware::from_fn_with_state(
                state.clone(),
                request_policy,
            ))
            .with_state(state)
    }

    async fn hydration_asset(State(state): State<AppState>, Path(asset): Path<String>) -> Response {
        let Some(asset) = state.hydration_assets.get(&asset) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, asset.content_type),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            asset.bytes.as_ref().to_vec(),
        )
            .into_response()
    }

    pub async fn shutdown_signal() {
        let ctrl_c = async {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
        };

        #[cfg(unix)]
        let terminate = async {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut signal) => {
                    signal.recv().await;
                }
                Err(_) => std::future::pending::<()>().await,
            }
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            () = ctrl_c => {}
            () = terminate => {}
        }
    }

    async fn landing(State(state): State<AppState>) -> Response {
        page_response(pages::landing(&state.config), &state.hydration_assets)
    }

    async fn login(State(state): State<AppState>) -> Response {
        page_response(
            pages::login(&state.config, SignInState::Ready),
            &state.hydration_assets,
        )
    }

    #[derive(Deserialize)]
    struct SignInForm {
        email: String,
    }

    async fn login_submit(State(state): State<AppState>, Form(form): Form<SignInForm>) -> Response {
        // The session service is not connected yet. Consume only the bounded form
        // shape, never log or reflect the identity, and fail without creating a
        // partial client-side session.
        let _valid_shape = form.email.len() <= 254
            && form.email.is_ascii()
            && form.email.contains('@')
            && !form.email.chars().any(char::is_whitespace);
        let mut page = pages::login(&state.config, SignInState::Failed);
        page.status = StatusCode::SERVICE_UNAVAILABLE;
        page_response(page, &state.hydration_assets)
    }

    #[derive(Default, Deserialize)]
    struct AuthenticatedQuery {
        fixture: Option<String>,
    }

    async fn dashboard(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Dashboard, &query)
    }

    async fn library(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Library, &query)
    }

    async fn spaces(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Spaces, &query)
    }

    async fn folders(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Folders, &query)
    }

    async fn imports(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Imports, &query)
    }

    async fn settings(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Settings, &query)
    }

    async fn developer(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Developer, &query)
    }

    async fn billing(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Billing, &query)
    }

    async fn admin(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Admin, &query)
    }

    fn authenticated_response(
        state: &AppState,
        route: AuthenticatedRoute,
        query: &AuthenticatedQuery,
    ) -> Response {
        let session = local_authenticated_fixture(&state.config, query.fixture.as_deref());
        page_response(
            pages::authenticated(&state.config, route, session),
            &state.hydration_assets,
        )
    }

    async fn share(State(state): State<AppState>, Path(video_id): Path<String>) -> Response {
        // Until the privacy-authorizing Worker endpoint is connected, production
        // always renders the indistinguishable unavailable state. Deterministic
        // local fixture IDs exercise public UI states without creating a
        // production authorization bypass.
        let share = local_share_fixture(&state.config, &video_id);
        page_response(
            pages::share(&state.config, &video_id, share),
            &state.hydration_assets,
        )
    }

    async fn embed(State(state): State<AppState>, Path(video_id): Path<String>) -> Response {
        let share = local_share_fixture(&state.config, &video_id);
        page_response(
            pages::embed(&state.config, &video_id, share),
            &state.hydration_assets,
        )
    }

    async fn not_found(State(state): State<AppState>) -> Response {
        page_response(pages::not_found(&state.config), &state.hydration_assets)
    }

    async fn robots(State(state): State<AppState>) -> Response {
        let (body, robots) = if state.config.deployment() == Deployment::Production {
            (
                format!(
                    "User-agent: *\nAllow: /\nDisallow: /login\nDisallow: /dashboard\nDisallow: /library\nDisallow: /spaces\nDisallow: /folders\nDisallow: /imports\nDisallow: /settings\nDisallow: /developer\nDisallow: /billing\nDisallow: /admin\nDisallow: /embed/\nSitemap: {}/sitemap.xml\n",
                    state.config.public_origin().as_str()
                ),
                "index,follow",
            )
        } else {
            ("User-agent: *\nDisallow: /\n".into(), "noindex,nofollow")
        };
        response_with_headers(
            StatusCode::OK,
            "text/plain; charset=utf-8",
            body,
            pages::NO_STORE,
            robots,
        )
    }

    async fn sitemap(State(state): State<AppState>) -> Response {
        if state.config.deployment() != Deployment::Production {
            return response_with_headers(
                StatusCode::NOT_FOUND,
                "text/plain; charset=utf-8",
                "not found".into(),
                pages::NO_STORE,
                "noindex,nofollow",
            );
        }
        let origin = state.config.public_origin().as_str();
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>{origin}/</loc></url></urlset>"
        );
        response_with_headers(
            StatusCode::OK,
            "application/xml; charset=utf-8",
            body,
            "public, max-age=300, s-maxage=3600",
            "index,follow",
        )
    }

    async fn legacy_health_redirect() -> Redirect {
        Redirect::permanent("/health/live")
    }

    #[derive(Serialize)]
    struct Health {
        service: &'static str,
        status: &'static str,
        release: String,
    }

    async fn liveness(State(state): State<AppState>) -> Json<Health> {
        Json(Health {
            service: "frame-web",
            status: "ok",
            release: state.config.release_id().to_owned(),
        })
    }

    #[derive(Serialize)]
    struct ReadyHealth {
        service: &'static str,
        status: &'static str,
        release: String,
        deployment: &'static str,
        router: bool,
        configuration: bool,
        hydration_assets: bool,
    }

    async fn readiness(State(state): State<AppState>) -> Response {
        let hydration_assets = state.hydration_assets.is_complete();
        let ready = state.config.deployment() == Deployment::Local || hydration_assets;
        let health = ReadyHealth {
            service: "frame-web",
            status: if ready { "ready" } else { "degraded" },
            release: state.config.release_id().to_owned(),
            deployment: state.config.deployment().as_str(),
            router: true,
            configuration: true,
            hydration_assets,
        };
        (
            if ready {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            Json(health),
        )
            .into_response()
    }

    #[derive(Serialize)]
    struct DependencyHealth {
        service: &'static str,
        status: &'static str,
        release: String,
        api_origin: &'static str,
        checked: bool,
    }

    async fn dependency_health(State(state): State<AppState>, headers: HeaderMap) -> Response {
        let Some(expected) = state.config.diagnostic_token() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let authorized = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes()));
        if !authorized {
            return StatusCode::NOT_FOUND.into_response();
        }

        // External health is intentionally not part of Render readiness. A
        // bounded active probe will be attached with the issue-36 transport; this
        // protected shape does not expose provider, binding, or account details.
        Json(DependencyHealth {
            service: "frame-web",
            status: "not_probed",
            release: state.config.release_id().to_owned(),
            api_origin: "configured",
            checked: false,
        })
        .into_response()
    }

    async fn request_policy(
        State(state): State<AppState>,
        request: Request<Body>,
        next: Next,
    ) -> Response {
        let path = request.uri().path().to_owned();
        let host = request
            .headers()
            .get(HOST)
            .and_then(|value| value.to_str().ok());
        let Some(host) = host else {
            return secured_error(
                StatusCode::MISDIRECTED_REQUEST,
                "invalid request authority",
                &state.config,
                &path,
            );
        };
        if !state.config.host_is_allowed(host) {
            return secured_error(
                StatusCode::MISDIRECTED_REQUEST,
                "invalid request authority",
                &state.config,
                &path,
            );
        }

        let is_health = path == "/health" || path.starts_with("/health/");
        if state.config.deployment() == Deployment::Production
            && !is_health
            && !state.config.host_is_canonical(host)
        {
            if matches!(*request.method(), Method::GET | Method::HEAD) {
                let location = canonical_location(&state.config, request.uri());
                let mut response = Redirect::permanent(&location).into_response();
                apply_response_policy(&state.config, &path, &mut response);
                return response;
            }
            return secured_error(
                StatusCode::MISDIRECTED_REQUEST,
                "non-canonical authority",
                &state.config,
                &path,
            );
        }

        let mut response = next.run(request).await;
        apply_response_policy(&state.config, &path, &mut response);
        response
    }

    fn page_response(page: Page, hydration_assets: &HydrationAssets) -> Response {
        response_with_headers(
            page.status,
            "text/html; charset=utf-8",
            hydration_assets.inject(page.body),
            page.cache_control,
            page.robots,
        )
    }

    fn response_with_headers(
        status: StatusCode,
        content_type: &'static str,
        body: String,
        cache_control: &'static str,
        robots: &'static str,
    ) -> Response {
        (
            status,
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, cache_control),
                (HeaderName::from_static("x-robots-tag"), robots),
            ],
            body,
        )
            .into_response()
    }

    fn secured_error(
        status: StatusCode,
        message: &'static str,
        config: &RuntimeConfig,
        path: &str,
    ) -> Response {
        let mut response =
            (status, [(header::CACHE_CONTROL, pages::NO_STORE)], message).into_response();
        apply_response_policy(config, path, &mut response);
        response
    }

    fn apply_response_policy(config: &RuntimeConfig, path: &str, response: &mut Response) {
        let headers = response.headers_mut();
        headers
            .entry(header::CACHE_CONTROL)
            .or_insert(HeaderValue::from_static(pages::NO_STORE));
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        headers.insert(
            HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        );
        headers.insert(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        );
        headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(
            "accelerometer=(), autoplay=(self), camera=(), display-capture=(), fullscreen=(self), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), publickey-credentials-get=(), usb=(), xr-spatial-tracking=()",
        ),
    );

        let embed_allowed = path.starts_with("/embed/") && config.embed_policy().enabled();
        let frame_ancestors = if embed_allowed {
            config
                .embed_policy()
                .ancestors()
                .iter()
                .map(|origin| origin.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            "'none'".into()
        };
        if !embed_allowed {
            headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        }
        let connect_source = if config.api_origin() == config.public_origin() {
            "'self'".to_owned()
        } else {
            format!("'self' {}", config.api_origin().as_str())
        };
        let csp = format!(
            "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors {frame_ancestors}; form-action 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self'; connect-src {connect_source}; font-src 'self'; worker-src 'self'; manifest-src 'self'"
        );
        if let Ok(value) = HeaderValue::from_str(&csp) {
            headers.insert(header::CONTENT_SECURITY_POLICY, value);
        }
    }

    fn canonical_location(config: &RuntimeConfig, uri: &Uri) -> String {
        let path_and_query = uri
            .path_and_query()
            .map_or("/", axum::http::uri::PathAndQuery::as_str);
        format!("{}{path_and_query}", config.public_origin().as_str())
    }

    fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
        let mut difference = left.len() ^ right.len();
        let length = left.len().max(right.len());
        for index in 0..length {
            let left = left.get(index).copied().unwrap_or_default();
            let right = right.get(index).copied().unwrap_or_default();
            difference |= usize::from(left ^ right);
        }
        difference == 0
    }

    #[cfg(test)]
    mod tests {
        use axum::body::to_bytes;
        use axum::response::Html;

        use super::*;

        fn local_config() -> RuntimeConfig {
            RuntimeConfig::from_values(ConfigValues::default()).expect("local config")
        }

        fn production_config() -> RuntimeConfig {
            RuntimeConfig::from_values(ConfigValues {
                deployment: Some("production".into()),
                public_origin: Some("https://frame.engmanager.xyz".into()),
                api_origin: Some("https://frame.engmanager.xyz".into()),
                ..ConfigValues::default()
            })
            .expect("production config")
        }

        fn write_hydration_fixture() -> (std::path::PathBuf, HydrationAssets) {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "frame-web-hydration-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).expect("create isolated hydration fixture");
            let javascript = b"export default async function init() {}";
            let wasm = b"\0asmfixture";
            let javascript_sha = format!("{:x}", Sha256::digest(javascript));
            let wasm_sha = format!("{:x}", Sha256::digest(wasm));
            let javascript_file = format!("frame-web-hydrate-{javascript_sha}.js");
            let wasm_file = format!("frame-web-hydrate_bg-{wasm_sha}.wasm");
            let bootstrap = format!(
                "import init from './{javascript_file}'; await init({{module_or_path:new URL('./{wasm_file}', import.meta.url)}});"
            );
            let bootstrap_sha = format!("{:x}", Sha256::digest(bootstrap.as_bytes()));
            let bootstrap_file = format!("frame-web-bootstrap-{bootstrap_sha}.js");
            std::fs::write(directory.join(&javascript_file), javascript)
                .expect("write fixture JavaScript");
            std::fs::write(directory.join(&wasm_file), wasm).expect("write fixture Wasm");
            std::fs::write(directory.join(&bootstrap_file), bootstrap)
                .expect("write fixture bootstrap");
            let manifest = serde_json::json!({
                "schema": HYDRATION_MANIFEST_SCHEMA,
                "bootstrap": {"file": bootstrap_file, "sha256": bootstrap_sha},
                "javascript": {"file": javascript_file, "sha256": javascript_sha},
                "wasm": {"file": wasm_file, "sha256": wasm_sha},
            });
            std::fs::write(
                directory.join("manifest.json"),
                serde_json::to_vec(&manifest).expect("serialize fixture manifest"),
            )
            .expect("write fixture manifest");
            let assets = HydrationAssets::load_from(&directory).expect("load verified fixture");
            (directory, assets)
        }

        #[test]
        fn hydration_bundle_is_all_or_nothing_and_content_addressed() {
            let page = pages::landing(&local_config());
            let no_assets = HydrationAssets::default().inject(page.body.clone());
            assert!(!no_assets.contains("FRAME_HYDRATION_"));
            assert!(!no_assets.contains("<script type=\"module\""));

            let (directory, assets) = write_hydration_fixture();
            assert!(assets.is_complete());
            let hydrated = assets.inject(page.body);
            assert!(!hydrated.contains("FRAME_HYDRATION_"));
            assert!(hydrated.contains("<script type=\"module\""));
            assert!(hydrated.contains("frame-web-bootstrap-"));
            assert!(hydrated.contains("frame-web-hydrate-"));

            let javascript = assets.javascript.as_ref().expect("fixture JavaScript");
            std::fs::write(directory.join(&javascript.file), b"tampered")
                .expect("tamper isolated fixture");
            assert!(HydrationAssets::load_from(&directory).is_none());
            std::fs::remove_dir_all(directory).expect("remove isolated hydration fixture");
        }

        #[tokio::test]
        async fn hydration_assets_are_immutable_and_unknown_names_fail_closed() {
            let (directory, assets) = write_hydration_fixture();
            let javascript = assets.javascript.as_ref().expect("fixture JavaScript");
            let state = AppState {
                config: Arc::new(local_config()),
                hydration_assets: assets.clone(),
            };
            let response =
                hydration_asset(State(state.clone()), Path(javascript.file.clone())).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "public, max-age=31536000, immutable"
            );
            let unknown = hydration_asset(State(state), Path("manifest.json".into())).await;
            assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
            std::fs::remove_dir_all(directory).expect("remove isolated hydration fixture");
        }

        #[test]
        fn nonlocal_asset_search_never_trusts_the_working_directory() {
            let executable = std::path::PathBuf::from("/opt/frame/bin/frame-web");
            assert_eq!(
                hydration_asset_directories(Deployment::Production, None, Some(executable.clone())),
                vec![std::path::PathBuf::from("/opt/frame/bin/web-dist")]
            );
            assert_eq!(
                hydration_asset_directories(Deployment::Preview, None, Some(executable.clone())),
                vec![std::path::PathBuf::from("/opt/frame/bin/web-dist")]
            );
            assert!(
                hydration_asset_directories(
                    Deployment::Production,
                    Some("apps/web/dist".into()),
                    Some(executable.clone())
                )
                .is_empty()
            );
            assert!(
                hydration_asset_directories(
                    Deployment::Production,
                    Some("/srv/frame/web-dist".into()),
                    Some(executable)
                )
                .is_empty()
            );
            assert!(
                hydration_asset_directories(
                    Deployment::Local,
                    None,
                    Some(std::path::PathBuf::from("target/debug/frame-web"))
                )
                .contains(&std::path::PathBuf::from("apps/web/dist"))
            );
        }

        #[tokio::test]
        async fn readiness_blocks_nonlocal_promotion_without_verified_assets() {
            let production = AppState {
                config: Arc::new(production_config()),
                hydration_assets: HydrationAssets::default(),
            };
            let degraded = readiness(State(production)).await;
            assert_eq!(degraded.status(), StatusCode::SERVICE_UNAVAILABLE);
            let body = to_bytes(degraded.into_body(), 16 * 1024)
                .await
                .expect("bounded degraded health body");
            let body = String::from_utf8(body.to_vec()).expect("health JSON is UTF-8");
            assert!(body.contains("\"status\":\"degraded\""));
            assert!(body.contains("\"hydration_assets\":false"));

            let local = AppState {
                config: Arc::new(local_config()),
                hydration_assets: HydrationAssets::default(),
            };
            assert_eq!(readiness(State(local)).await.status(), StatusCode::OK);
        }

        #[test]
        fn canonical_redirect_never_uses_request_host() {
            let config = RuntimeConfig::from_values(ConfigValues {
                deployment: Some("production".into()),
                public_origin: Some("https://frame.engmanager.xyz".into()),
                api_origin: Some("https://frame.engmanager.xyz".into()),
                render_external_url: Some("https://frame-web.onrender.com".into()),
                ..ConfigValues::default()
            })
            .expect("production config");
            let uri = Uri::from_static("/login?return=dashboard");
            assert_eq!(
                canonical_location(&config, &uri),
                "https://frame.engmanager.xyz/login?return=dashboard"
            );
        }

        #[test]
        fn headers_deny_framing_and_capture_by_default() {
            let config = local_config();
            let mut response = Html("ok").into_response();
            apply_response_policy(&config, "/dashboard", &mut response);
            assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
            let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .expect("valid CSP");
            assert!(csp.contains("frame-ancestors 'none'"));
            assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval'"));
            assert!(
                !csp.replace("'wasm-unsafe-eval'", "")
                    .contains("'unsafe-eval'")
            );
            let permissions = response.headers()["permissions-policy"]
                .to_str()
                .expect("valid permissions policy");
            assert!(permissions.contains("camera=()"));
            assert!(permissions.contains("microphone=()"));
            assert!(permissions.contains("display-capture=()"));
        }

        #[test]
        fn production_never_enables_local_share_fixtures() {
            let config = RuntimeConfig::from_values(ConfigValues {
                deployment: Some("production".into()),
                public_origin: Some("https://frame.engmanager.xyz".into()),
                api_origin: Some("https://frame.engmanager.xyz".into()),
                ..ConfigValues::default()
            })
            .expect("production config");
            assert_eq!(
                local_share_fixture(&config, "fixture-public").availability(),
                frame_client::ShareAvailability::Unavailable
            );
        }

        #[test]
        fn diagnostic_token_comparison_handles_different_lengths() {
            assert!(constant_time_eq(b"same", b"same"));
            assert!(!constant_time_eq(b"same", b"different"));
            assert!(!constant_time_eq(b"same", b"samf"));
        }

        #[tokio::test]
        async fn failed_sign_in_never_reflects_identity_or_creates_redirect() {
            let state = AppState {
                config: Arc::new(local_config()),
                hydration_assets: HydrationAssets::default(),
            };
            let response = login_submit(
                State(state),
                Form(SignInForm {
                    email: "private-person@example.test".into(),
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert!(response.headers().get(header::LOCATION).is_none());
            assert_eq!(response.headers()[header::CACHE_CONTROL], pages::NO_STORE);
            let body = to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("bounded response body");
            let body = String::from_utf8(body.to_vec()).expect("HTML is UTF-8");
            assert!(!body.contains("private-person"));
            assert!(body.contains("No session was created"));
        }

        #[tokio::test]
        async fn production_ignores_authenticated_fixture_query() {
            let config = RuntimeConfig::from_values(ConfigValues {
                deployment: Some("production".into()),
                public_origin: Some("https://frame.engmanager.xyz".into()),
                api_origin: Some("https://frame.engmanager.xyz".into()),
                ..ConfigValues::default()
            })
            .expect("production config");
            let state = AppState {
                config: Arc::new(config),
                hydration_assets: HydrationAssets::default(),
            };
            let response = authenticated_response(
                &state,
                AuthenticatedRoute::Dashboard,
                &AuthenticatedQuery {
                    fixture: Some("owner".into()),
                },
            );
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let body = to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("bounded response body");
            let body = String::from_utf8(body.to_vec()).expect("HTML is UTF-8");
            assert!(!body.contains("Local Frame workspace"));
            assert!(!body.contains("Product walkthrough"));
        }
    }
}

#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
pub use server::*;
