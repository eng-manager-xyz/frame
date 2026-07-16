use anyhow::{Context, Result};
use std::time::Duration;

use frame_web::{DrainOutcome, RuntimeConfig, build_app, serve_with_shutdown, shutdown_signal};
use tracing::{info, warn};
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
    let outcome = serve_with_shutdown(listener, app, shutdown_signal(), Duration::from_secs(55))
        .await
        .context("serve Frame web application")?;
    match outcome {
        DrainOutcome::DeadlineExceeded => {
            warn!(deployment, release, "Frame web drain deadline reached");
        }
        DrainOutcome::Drained | DrainOutcome::StoppedWithoutSignal => {
            info!(deployment, release, ?outcome, "Frame web service stopped");
        }
    }
    Ok(())
}
