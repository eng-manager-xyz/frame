#![recursion_limit = "256"]

pub mod browser_authenticated;
pub mod hydration;
pub mod share_player;

#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
pub mod authenticated;
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod browser_security;
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod config;
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod pages;
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod product;
#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod public_ssr;

#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
mod server {

    use std::{
        future::{Future, IntoFuture},
        sync::Arc,
        time::Duration,
    };

    use axum::{
        Json, Router,
        body::{Body, Bytes},
        extract::{DefaultBodyLimit, Path, Query, State},
        http::{
            HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, Uri,
            header::{self, HOST},
        },
        middleware::{self, Next},
        response::{IntoResponse, Redirect, Response},
        routing::{get, post},
    };
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};

    use crate::authenticated::RouteViewQuery;
    use crate::browser_security::sanitize_csp_reports;
    pub use crate::config::{ConfigError, ConfigValues, Deployment, ProxyTrust, RuntimeConfig};
    use crate::pages::{self, Page, SignInState};
    use crate::product::{
        AuthenticatedRoute, ShareView, local_authenticated_fixture, local_share_fixture,
    };
    use crate::public_ssr::PublicSsr;

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
        public_ssr: Option<PublicSsr>,
    }

    pub fn build_app(config: RuntimeConfig) -> Router {
        let hydration_assets = HydrationAssets::load(config.deployment());
        let public_ssr = if config.deployment() == Deployment::Local {
            None
        } else {
            PublicSsr::from_config(&config).ok()
        };
        let runtime_test_mode = config.runtime_test_mode();
        let state = AppState {
            config: Arc::new(config),
            hydration_assets,
            public_ssr,
        };
        let router = Router::new()
            .route("/", get(landing))
            .route("/login", get(login))
            .route("/signup", get(signup))
            .route("/recovery", get(recovery))
            .route("/verify", get(verify))
            .route("/dashboard", get(dashboard))
            .route("/library", get(library))
            .route("/spaces", get(spaces))
            .route("/spaces/{resource_id}", get(space))
            .route("/folders", get(folders))
            .route("/folders/{resource_id}", get(folder))
            .route("/onboarding", get(onboarding))
            .route("/imports", get(imports))
            .route("/settings", get(settings))
            .route("/settings/account", get(account_settings))
            .route("/settings/organization", get(organization_settings))
            .route("/settings/members", get(member_settings))
            .route("/settings/storage", get(storage_settings))
            .route("/developer", get(developer))
            .route("/billing", get(billing))
            .route("/analytics", get(analytics))
            .route("/admin", get(admin))
            .route("/dashboard/settings", get(legacy_settings))
            .route("/dashboard/settings/account", get(legacy_account_settings))
            .route(
                "/dashboard/settings/organization",
                get(legacy_organization_settings),
            )
            .route("/dashboard/settings/members", get(legacy_member_settings))
            .route("/dashboard/settings/storage", get(legacy_storage_settings))
            .route("/dashboard/import", get(legacy_imports))
            .route("/dashboard/developers", get(legacy_developer))
            .route("/dashboard/billing", get(legacy_billing))
            .route("/dashboard/analytics", get(legacy_analytics))
            .route("/dashboard/spaces", get(legacy_spaces))
            .route("/dashboard/folders", get(legacy_folders))
            .route("/dashboard/spaces/{resource_id}", get(legacy_space))
            .route("/dashboard/folders/{resource_id}", get(legacy_folder))
            .route("/s/{video_id}", get(share))
            .route("/share/{video_id}", get(legacy_share))
            .route("/embed/{video_id}", get(embed))
            .route("/robots.txt", get(robots))
            .route("/sitemap.xml", get(sitemap))
            .route("/assets/{asset}", get(hydration_asset))
            .route("/health", get(legacy_health_redirect))
            .route("/health/live", get(liveness))
            .route("/health/ready", get(readiness))
            .route("/health/dependencies", get(dependency_health))
            .route("/health/release", get(release_health))
            .route("/__frame/csp-report", post(csp_report))
            .fallback(not_found);
        let router = if runtime_test_mode {
            router.route("/_internal/runtime/drain", get(runtime_drain_probe))
        } else {
            router
        };
        router
            .layer(middleware::from_fn_with_state(
                state.clone(),
                request_policy,
            ))
            .layer(DefaultBodyLimit::max(16 * 1024))
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

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DrainOutcome {
        StoppedWithoutSignal,
        Drained,
        DeadlineExceeded,
    }

    /// Serve until the supplied shutdown signal, then bound Axum's graceful
    /// drain. The web process owns no durable mutation, so dropping a request
    /// at the deadline cannot duplicate or lose durable work.
    pub async fn serve_with_shutdown<F>(
        listener: tokio::net::TcpListener,
        app: Router,
        shutdown: F,
        drain_budget: Duration,
    ) -> Result<DrainOutcome, std::io::Error>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let (notice_tx, notice_rx) = tokio::sync::oneshot::channel();
        let shutdown = async move {
            shutdown.await;
            let _ = notice_tx.send(());
        };
        let server = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .into_future();
        tokio::pin!(server);
        tokio::pin!(notice_rx);

        tokio::select! {
            result = &mut server => result.map(|()| DrainOutcome::StoppedWithoutSignal),
            _ = &mut notice_rx => {
                match tokio::time::timeout(drain_budget, &mut server).await {
                    Ok(result) => result.map(|()| DrainOutcome::Drained),
                    Err(_) => Ok(DrainOutcome::DeadlineExceeded),
                }
            }
        }
    }

    async fn landing(State(state): State<AppState>) -> Response {
        page_response(pages::landing(&state.config), &state.hydration_assets)
    }

    #[derive(Default, Deserialize)]
    struct AuthPageQuery {
        auth_error: Option<String>,
    }

    fn auth_page_state(query: &AuthPageQuery) -> SignInState {
        match query.auth_error.as_deref() {
            Some("invalid") => SignInState::Invalid,
            Some("failed") => SignInState::Failed,
            _ => SignInState::Ready,
        }
    }

    async fn login(State(state): State<AppState>, Query(query): Query<AuthPageQuery>) -> Response {
        page_response(
            pages::login(&state.config, auth_page_state(&query)),
            &state.hydration_assets,
        )
    }

    async fn signup(State(state): State<AppState>, Query(query): Query<AuthPageQuery>) -> Response {
        page_response(
            pages::signup(&state.config, auth_page_state(&query)),
            &state.hydration_assets,
        )
    }

    async fn recovery(
        State(state): State<AppState>,
        Query(query): Query<AuthPageQuery>,
    ) -> Response {
        page_response(
            pages::recovery(&state.config, auth_page_state(&query)),
            &state.hydration_assets,
        )
    }

    async fn verify(State(state): State<AppState>, Query(query): Query<AuthPageQuery>) -> Response {
        page_response(
            pages::verify(&state.config, auth_page_state(&query)),
            &state.hydration_assets,
        )
    }

    #[derive(Default, Deserialize)]
    struct AuthenticatedQuery {
        fixture: Option<String>,
        q: Option<String>,
        filter: Option<String>,
        page: Option<String>,
        theme: Option<String>,
    }

    async fn dashboard(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Dashboard, &query).await
    }

    async fn library(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Library, &query).await
    }

    async fn spaces(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Spaces, &query).await
    }

    async fn space(
        State(state): State<AppState>,
        Path(resource_id): Path<String>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_resource_response(&state, AuthenticatedRoute::Space, &resource_id, &query)
            .await
    }

    async fn folders(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Folders, &query).await
    }

    async fn folder(
        State(state): State<AppState>,
        Path(resource_id): Path<String>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_resource_response(&state, AuthenticatedRoute::Folder, &resource_id, &query)
            .await
    }

    async fn onboarding(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Onboarding, &query).await
    }

    async fn imports(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Imports, &query).await
    }

    async fn settings(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Settings, &query).await
    }

    async fn account_settings(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::AccountSettings, &query).await
    }

    async fn organization_settings(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::OrganizationSettings, &query).await
    }

    async fn member_settings(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::MemberSettings, &query).await
    }

    async fn storage_settings(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::StorageSettings, &query).await
    }

    async fn developer(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Developer, &query).await
    }

    async fn billing(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Billing, &query).await
    }

    async fn analytics(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Analytics, &query).await
    }

    async fn admin(
        State(state): State<AppState>,
        Query(query): Query<AuthenticatedQuery>,
    ) -> Response {
        authenticated_response(&state, AuthenticatedRoute::Admin, &query).await
    }

    async fn authenticated_response(
        state: &AppState,
        route: AuthenticatedRoute,
        query: &AuthenticatedQuery,
    ) -> Response {
        authenticated_response_at(state, route, route.path(), None, query).await
    }

    async fn authenticated_resource_response(
        state: &AppState,
        route: AuthenticatedRoute,
        resource_id: &str,
        query: &AuthenticatedQuery,
    ) -> Response {
        if !safe_resource_id(resource_id) {
            return secured_error(
                StatusCode::NOT_FOUND,
                "resource unavailable",
                &state.config,
                route.path(),
            );
        }
        let prefix = route.dynamic_prefix().expect("resource route prefix");
        authenticated_response_at(
            state,
            route,
            &format!("{prefix}{resource_id}"),
            Some(resource_id),
            query,
        )
        .await
    }

    async fn authenticated_response_at(
        state: &AppState,
        route: AuthenticatedRoute,
        canonical_path: &str,
        resource_id: Option<&str>,
        query: &AuthenticatedQuery,
    ) -> Response {
        let Ok(view_query) = RouteViewQuery::parse(
            query.q.as_deref(),
            query.filter.as_deref(),
            query.page.as_deref(),
            query.theme.as_deref(),
        ) else {
            return secured_error(
                StatusCode::BAD_REQUEST,
                "invalid view query",
                &state.config,
                canonical_path,
            );
        };
        let session = if state.config.deployment() == Deployment::Local {
            local_authenticated_fixture(&state.config, query.fixture.as_deref())
        } else {
            // ADR 0004 and the browser capability matrix deliberately keep
            // authenticated SSR disabled. Render must not receive or forward
            // browser credentials. A future browser-side loader needs a
            // separately approved host-only session and CSRF design.
            let _ = resource_id;
            crate::product::AuthenticatedState::Unauthenticated
        };
        page_response(
            pages::authenticated_at(&state.config, route, session, canonical_path, &view_query),
            &state.hydration_assets,
        )
    }

    fn safe_resource_id(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }

    async fn legacy_settings() -> Redirect {
        Redirect::permanent("/settings")
    }

    async fn legacy_account_settings() -> Redirect {
        Redirect::permanent("/settings/account")
    }

    async fn legacy_organization_settings() -> Redirect {
        Redirect::permanent("/settings/organization")
    }

    async fn legacy_member_settings() -> Redirect {
        Redirect::permanent("/settings/members")
    }

    async fn legacy_storage_settings() -> Redirect {
        Redirect::permanent("/settings/storage")
    }

    async fn legacy_imports() -> Redirect {
        Redirect::permanent("/imports")
    }

    async fn legacy_developer() -> Redirect {
        Redirect::permanent("/developer")
    }

    async fn legacy_billing() -> Redirect {
        Redirect::permanent("/billing")
    }

    async fn legacy_analytics() -> Redirect {
        Redirect::permanent("/analytics")
    }

    async fn legacy_spaces() -> Redirect {
        Redirect::permanent("/spaces")
    }

    async fn legacy_folders() -> Redirect {
        Redirect::permanent("/folders")
    }

    async fn legacy_space(Path(resource_id): Path<String>) -> Response {
        legacy_resource_redirect("/spaces/", &resource_id)
    }

    async fn legacy_folder(Path(resource_id): Path<String>) -> Response {
        legacy_resource_redirect("/folders/", &resource_id)
    }

    async fn legacy_share(Path(video_id): Path<String>) -> Response {
        match crate::share_player::resolve_legacy_route(&format!("/share/{video_id}")) {
            crate::share_player::LegacyRoute::PermanentRedirect { location } => {
                Redirect::permanent(&location).into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        }
    }

    fn legacy_resource_redirect(prefix: &str, resource_id: &str) -> Response {
        if !safe_resource_id(resource_id) {
            return StatusCode::NOT_FOUND.into_response();
        }
        Redirect::permanent(&format!("{prefix}{resource_id}")).into_response()
    }

    async fn share(State(state): State<AppState>, Path(video_id): Path<String>) -> Response {
        let share = public_share_view(&state, &video_id).await;
        page_response(
            pages::share(&state.config, &video_id, share),
            &state.hydration_assets,
        )
    }

    async fn embed(State(state): State<AppState>, Path(video_id): Path<String>) -> Response {
        let share = public_share_view(&state, &video_id).await;
        page_response(
            pages::embed(&state.config, &video_id, share),
            &state.hydration_assets,
        )
    }

    async fn public_share_view(state: &AppState, video_id: &str) -> ShareView {
        if state.config.deployment() == Deployment::Local {
            return local_share_fixture(&state.config, video_id);
        }
        let Some(client) = &state.public_ssr else {
            return ShareView::Unavailable;
        };
        client
            .public_share(video_id)
            .await
            .map_or(ShareView::Unavailable, |summary| {
                ShareView::from_summary(&state.config, summary)
            })
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
        public_ssr: bool,
    }

    async fn readiness(State(state): State<AppState>) -> Response {
        let hydration_assets = state.hydration_assets.is_complete();
        let public_ssr =
            state.config.deployment() == Deployment::Local || state.public_ssr.is_some();
        let ready =
            state.config.deployment() == Deployment::Local || hydration_assets && public_ssr;
        let health = ReadyHealth {
            service: "frame-web",
            status: if ready { "ready" } else { "degraded" },
            release: state.config.release_id().to_owned(),
            deployment: state.config.deployment().as_str(),
            router: true,
            configuration: true,
            hydration_assets,
            public_ssr,
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
        circuit: &'static str,
    }

    async fn dependency_health(State(state): State<AppState>, headers: HeaderMap) -> Response {
        if !diagnostic_authorized(&state.config, &headers) {
            return StatusCode::NOT_FOUND.into_response();
        }

        // External health remains outside Render readiness. This fixed health
        // read is anonymous, redirect-free, size bounded, deadline bounded,
        // and protected from cascading failure by the SSR circuit breaker.
        let Some(client) = &state.public_ssr else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(DependencyHealth {
                    service: "frame-web",
                    status: "unavailable",
                    release: state.config.release_id().to_owned(),
                    api_origin: "configured",
                    checked: false,
                    circuit: "unavailable",
                }),
            )
                .into_response();
        };
        let healthy = client.health().await.is_ok();
        (
            if healthy {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            Json(DependencyHealth {
                service: "frame-web",
                status: if healthy { "ok" } else { "degraded" },
                release: state.config.release_id().to_owned(),
                api_origin: "configured",
                checked: true,
                circuit: client.circuit_status(),
            }),
        )
            .into_response()
    }

    #[derive(Serialize)]
    struct ReleaseHealth<'a> {
        service: &'static str,
        status: &'static str,
        source_git_sha: &'a str,
        contract_major: u16,
        worker_release: &'a str,
        render_deploy: &'a str,
        migration_level: &'a str,
        portfolio_consumer: &'a str,
    }

    #[derive(Serialize)]
    struct IncompleteReleaseHealth {
        service: &'static str,
        status: &'static str,
    }

    async fn release_health(State(state): State<AppState>, headers: HeaderMap) -> Response {
        if !diagnostic_authorized(&state.config, &headers) {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(release) = state.config.release_join() else {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CACHE_CONTROL, pages::NO_STORE)],
                Json(IncompleteReleaseHealth {
                    service: "frame-web",
                    status: "incomplete",
                }),
            )
                .into_response();
        };
        (
            StatusCode::OK,
            [(header::CACHE_CONTROL, pages::NO_STORE)],
            Json(ReleaseHealth {
                service: "frame-web",
                status: "joined",
                source_git_sha: release.source_git_sha(),
                contract_major: frame_client::ApiVersion::current().major,
                worker_release: release.worker_release(),
                render_deploy: release.render_deploy(),
                migration_level: release.migration_level(),
                portfolio_consumer: release.portfolio_consumer(),
            }),
        )
            .into_response()
    }

    fn diagnostic_authorized(config: &RuntimeConfig, headers: &HeaderMap) -> bool {
        let Some(expected) = config.diagnostic_token() else {
            return false;
        };
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|provided| constant_time_eq(provided.as_bytes(), expected.as_bytes()))
    }

    async fn csp_report(
        State(state): State<AppState>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if !matches!(
            content_type,
            Some("application/csp-report" | "application/reports+json" | "application/json")
        ) {
            return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
        }
        for report in sanitize_csp_reports(&body, state.config.public_origin().as_str()) {
            tracing::warn!(
                release = state.config.release_id(),
                directive = report.directive.as_str(),
                blocked_resource = report.blocked.as_str(),
                disposition = if report.report_only {
                    "report"
                } else {
                    "enforce"
                },
                "sanitized browser policy violation"
            );
        }
        StatusCode::NO_CONTENT.into_response()
    }

    async fn request_policy(
        State(state): State<AppState>,
        request: Request<Body>,
        next: Next,
    ) -> Response {
        let path = request.uri().path().to_owned();
        if !forwarded_headers_are_valid(state.config.proxy_trust(), request.headers()) {
            return secured_error(
                StatusCode::BAD_REQUEST,
                "invalid proxy metadata",
                &state.config,
                &path,
            );
        }
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

    fn forwarded_headers_are_valid(proxy: ProxyTrust, headers: &HeaderMap) -> bool {
        // RFC Forwarded is never an authority here: accepting multiple proxy
        // grammars creates precedence confusion. X-Forwarded-Host/For and
        // client-IP headers may be present, but are deliberately ignored.
        if headers.contains_key("forwarded") {
            return false;
        }
        match headers.get("x-forwarded-proto") {
            None => true,
            Some(value) if proxy == ProxyTrust::RenderEdge => value.as_bytes() == b"https",
            Some(_) => false,
        }
    }

    #[derive(Deserialize)]
    struct DrainProbeQuery {
        delay_ms: Option<u64>,
    }

    async fn runtime_drain_probe(Query(query): Query<DrainProbeQuery>) -> StatusCode {
        let delay = query.delay_ms.unwrap_or(250).min(2_000);
        tokio::time::sleep(Duration::from_millis(delay)).await;
        StatusCode::NO_CONTENT
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
        let embed_allowed = path.starts_with("/embed/") && config.embed_policy().enabled();
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
            HeaderValue::from_static(if embed_allowed {
                "cross-origin"
            } else {
                "same-origin"
            }),
        );
        headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(
            "accelerometer=(), autoplay=(self), camera=(), display-capture=(), fullscreen=(self), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), picture-in-picture=(self), publickey-credentials-get=(), usb=(), xr-spatial-tracking=()",
        ),
    );
        headers.insert(
            HeaderName::from_static("reporting-endpoints"),
            HeaderValue::from_static("frame-csp=\"/__frame/csp-report\""),
        );

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
            "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors {frame_ancestors}; form-action 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self'; connect-src {connect_source}; font-src 'self'; worker-src 'self'; manifest-src 'self'; report-to frame-csp; report-uri /__frame/csp-report"
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
                proxy_trust: Some("render".into()),
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
                public_ssr: None,
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

        #[tokio::test]
        async fn legacy_share_redirect_is_canonical_and_drops_ambient_query_data() {
            let response = legacy_share(Path("public-demo".into())).await;
            assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);
            assert_eq!(response.headers()[header::LOCATION], "/s/public-demo");
            assert!(
                !response.headers()[header::LOCATION]
                    .to_str()
                    .expect("ASCII location")
                    .contains('?')
            );

            let invalid = legacy_share(Path("..".into())).await;
            assert_eq!(invalid.status(), StatusCode::NOT_FOUND);
            assert!(invalid.headers().get(header::LOCATION).is_none());
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
                public_ssr: None,
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
                public_ssr: None,
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
                proxy_trust: Some("render".into()),
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
            assert_eq!(
                response.headers()["cross-origin-resource-policy"],
                "same-origin"
            );
            let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .expect("valid CSP");
            assert!(csp.contains("frame-ancestors 'none'"));
            assert!(csp.contains("report-uri /__frame/csp-report"));
            assert_eq!(
                response.headers()["reporting-endpoints"],
                "frame-csp=\"/__frame/csp-report\""
            );
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
            assert!(permissions.contains("picture-in-picture=(self)"));
        }

        #[test]
        fn embed_headers_allow_only_configured_exact_ancestors() {
            let config = RuntimeConfig::from_values(ConfigValues {
                public_embed_enabled: Some("true".into()),
                embed_ancestors: Some("https://engmanager.xyz,https://www.engmanager.xyz".into()),
                ..ConfigValues::default()
            })
            .expect("local embed config");
            let mut response = Html("ok").into_response();
            apply_response_policy(&config, "/embed/public-demo", &mut response);
            assert!(response.headers().get(header::X_FRAME_OPTIONS).is_none());
            assert_eq!(
                response.headers()["cross-origin-resource-policy"],
                "cross-origin"
            );
            let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .expect("valid CSP");
            assert!(
                csp.contains("frame-ancestors https://engmanager.xyz https://www.engmanager.xyz")
            );
            assert!(!csp.contains("frame-ancestors https: "));
            assert!(!csp.contains("frame-ancestors *"));
        }

        #[test]
        fn production_never_enables_local_share_fixtures() {
            let config = RuntimeConfig::from_values(ConfigValues {
                deployment: Some("production".into()),
                public_origin: Some("https://frame.engmanager.xyz".into()),
                api_origin: Some("https://frame.engmanager.xyz".into()),
                proxy_trust: Some("render".into()),
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
        async fn release_health_is_hidden_bounded_and_complete_or_degraded() {
            let token = "diagnostic-token-at-least-24-bytes";
            let config = RuntimeConfig::from_values(ConfigValues {
                release_id: Some("1".repeat(40)),
                diagnostic_token: Some(token.into()),
                worker_release: Some("worker-1111111".into()),
                render_deploy: Some("render-deploy-1".into()),
                migration_level: Some("0035_instant_finalize_public_share_index.sql".into()),
                portfolio_consumer: Some("portfolio-aaaaaaa".into()),
                ..ConfigValues::default()
            })
            .expect("release diagnostics config");
            let state = AppState {
                config: Arc::new(config),
                hydration_assets: HydrationAssets::default(),
                public_ssr: None,
            };
            let mut authorized = HeaderMap::new();
            authorized.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))
                    .expect("safe authorization fixture"),
            );
            let response = release_health(State(state.clone()), authorized.clone()).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], pages::NO_STORE);
            let body = to_bytes(response.into_body(), 16 * 1024)
                .await
                .expect("bounded release health body");
            let value: serde_json::Value =
                serde_json::from_slice(&body).expect("release health JSON");
            assert_eq!(
                value.as_object().map(|object| object.len()),
                Some(8),
                "release health field inventory drifted"
            );
            assert_eq!(value["status"], "joined");
            assert_eq!(value["source_git_sha"], "1".repeat(40));
            assert_eq!(value["contract_major"], 1);
            assert_eq!(value["worker_release"], "worker-1111111");
            assert_eq!(value["render_deploy"], "render-deploy-1");
            assert_eq!(
                value["migration_level"],
                "0035_instant_finalize_public_share_index.sql"
            );
            assert_eq!(value["portfolio_consumer"], "portfolio-aaaaaaa");

            let hidden = release_health(State(state), HeaderMap::new()).await;
            assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

            let incomplete = AppState {
                config: Arc::new(
                    RuntimeConfig::from_values(ConfigValues {
                        diagnostic_token: Some(token.into()),
                        ..ConfigValues::default()
                    })
                    .expect("incomplete local release diagnostics"),
                ),
                hydration_assets: HydrationAssets::default(),
                public_ssr: None,
            };
            let degraded = release_health(State(incomplete), authorized).await;
            assert_eq!(degraded.status(), StatusCode::SERVICE_UNAVAILABLE);
            let body = to_bytes(degraded.into_body(), 16 * 1024)
                .await
                .expect("bounded incomplete release body");
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body)
                    .expect("incomplete release JSON"),
                serde_json::json!({"service": "frame-web", "status": "incomplete"})
            );
        }

        #[test]
        fn proxy_metadata_has_one_unambiguous_scheme_policy() {
            let mut headers = HeaderMap::new();
            assert!(forwarded_headers_are_valid(ProxyTrust::Direct, &headers));
            assert!(forwarded_headers_are_valid(
                ProxyTrust::RenderEdge,
                &headers
            ));

            headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
            assert!(!forwarded_headers_are_valid(ProxyTrust::Direct, &headers));
            assert!(forwarded_headers_are_valid(
                ProxyTrust::RenderEdge,
                &headers
            ));

            headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
            assert!(!forwarded_headers_are_valid(
                ProxyTrust::RenderEdge,
                &headers
            ));
            headers.remove("x-forwarded-proto");
            headers.insert(
                "forwarded",
                HeaderValue::from_static("for=unknown;proto=https"),
            );
            assert!(!forwarded_headers_are_valid(
                ProxyTrust::RenderEdge,
                &headers
            ));
        }

        #[test]
        fn auth_error_query_is_closed_and_contains_no_submitted_material() {
            assert_eq!(
                auth_page_state(&AuthPageQuery {
                    auth_error: Some("invalid".into()),
                }),
                SignInState::Invalid
            );
            assert_eq!(
                auth_page_state(&AuthPageQuery {
                    auth_error: Some("failed".into()),
                }),
                SignInState::Failed
            );
            assert_eq!(
                auth_page_state(&AuthPageQuery {
                    auth_error: Some("private-person@example.test".into()),
                }),
                SignInState::Ready
            );
        }

        #[tokio::test]
        async fn production_authenticated_ssr_is_disabled_and_ignores_fixtures() {
            let config = RuntimeConfig::from_values(ConfigValues {
                deployment: Some("production".into()),
                public_origin: Some("https://frame.engmanager.xyz".into()),
                api_origin: Some("https://frame.engmanager.xyz".into()),
                proxy_trust: Some("render".into()),
                ..ConfigValues::default()
            })
            .expect("production config");
            let state = AppState {
                config: Arc::new(config),
                hydration_assets: HydrationAssets::default(),
                public_ssr: None,
            };
            let response = authenticated_response(
                &state,
                AuthenticatedRoute::Dashboard,
                &AuthenticatedQuery {
                    fixture: Some("owner".into()),
                    ..AuthenticatedQuery::default()
                },
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            let body = to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("bounded response body");
            let body = String::from_utf8(body.to_vec()).expect("HTML is UTF-8");
            assert!(body.contains("Sign in required"));
            assert!(!body.contains("Local Frame workspace"));
            assert!(!body.contains("Product walkthrough"));
        }
    }
}

#[cfg(all(feature = "ssr", not(target_arch = "wasm32")))]
pub use server::*;
