use anyhow::{Context, Result};
use frame_web::{RuntimeConfig, build_app, shutdown_signal};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = RuntimeConfig::from_env().context("invalid Frame web configuration")?;
    let address = config.bind_address();
    let deployment = config.deployment().as_str();
    let release = config.release_id().to_owned();
    let app = build_app(config);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind Frame web listener at {address}"))?;

    info!(%address, deployment, release, "Frame web service listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve Frame web application")
}
