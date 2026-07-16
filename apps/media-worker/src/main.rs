use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use axum::{Json, Router, http::StatusCode, routing::get};
use frame_media::{
    FactoryRequirement, diagnose_runtime, media_job_catalog, probe_runtime, record_synthetic_webm,
};
use serde::Serialize;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

const HEALTH_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Serialize)]
struct HealthResponse {
    schema_version: u16,
    service: &'static str,
    status: &'static str,
    catalog_version: u16,
    required_factories_unavailable: usize,
}

fn health_response() -> (StatusCode, Json<HealthResponse>) {
    let diagnostics = diagnose_runtime();
    let missing = diagnostics
        .factories
        .iter()
        .filter(|factory| factory.requirement == FactoryRequirement::Required && !factory.available)
        .count();
    let ready = diagnostics.is_ready();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(HealthResponse {
            schema_version: HEALTH_SCHEMA_VERSION,
            service: "frame-media-worker",
            status: if ready { "ready" } else { "degraded" },
            catalog_version: media_job_catalog().version,
            required_factories_unavailable: missing,
        }),
    )
}

async fn live() -> Json<HealthResponse> {
    Json(HealthResponse {
        schema_version: HEALTH_SCHEMA_VERSION,
        service: "frame-media-worker",
        status: "live",
        catalog_version: media_job_catalog().version,
        required_factories_unavailable: 0,
    })
}

async fn ready() -> (StatusCode, Json<HealthResponse>) {
    health_response()
}

async fn serve() -> Result<()> {
    let address = env::var("FRAME_MEDIA_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8790".into())
        .parse::<SocketAddr>()
        .context("FRAME_MEDIA_ADDR must be an IP socket address")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("failed to bind native media health service at {address}"))?;
    info!(%address, "native media worker health service ready");
    let app = Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready));
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                warn!(%error, "failed to install shutdown signal handler");
            }
        })
        .await
        .context("native media health service failed")
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match env::args().nth(1).as_deref().unwrap_or("probe") {
        "doctor" => {
            let diagnostics = diagnose_runtime();
            info!(
                manifest_version = diagnostics.manifest_version,
                runtime_version = diagnostics
                    .runtime_version
                    .as_deref()
                    .unwrap_or("unavailable"),
                "media runtime diagnostics"
            );
            for factory in &diagnostics.factories {
                if factory.available || factory.requirement == FactoryRequirement::Optional {
                    info!(
                        factory = factory.factory,
                        capability = %factory.capability,
                        requirement = ?factory.requirement,
                        platform = ?factory.platform,
                        available = factory.available,
                        "GStreamer factory"
                    );
                } else {
                    warn!(
                        factory = factory.factory,
                        capability = %factory.capability,
                        requirement = ?factory.requirement,
                        platform = ?factory.platform,
                        "required GStreamer factory unavailable"
                    );
                }
            }
            for issue in &diagnostics.issues {
                warn!(issue = ?issue, "media runtime issue");
            }
            if !diagnostics.is_ready() {
                bail!("media runtime is not ready; inspect the privacy-safe diagnostics above");
            }
        }
        "probe" => {
            let runtime = probe_runtime().context("GStreamer probe failed")?;
            info!(
                version = %runtime.version,
                manifest_version = runtime.manifest_version,
                required_plugins = ?runtime.required_factories,
                optional_plugins = ?runtime.optional_factories_available,
                "media runtime ready"
            );
        }
        "smoke" => {
            let output = env::args()
                .nth(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("target/frame-smoke.webm"));
            record_synthetic_webm(&output).context("synthetic GStreamer pipeline failed")?;
            info!(path = %output.display(), "synthetic media artifact created");
        }
        "catalog" => {
            let catalog = media_job_catalog();
            info!(
                version = catalog.version,
                jobs = catalog.jobs.len(),
                "media job catalog"
            );
            for job in catalog.jobs {
                info!(
                    kind = ?job.kind,
                    preferred = ?job.preferred,
                    managed = job.managed_supported,
                    native = job.native_supported,
                    progress = ?job.progress,
                    cancellation = ?job.cancellation,
                    timeout_ms = job.timeout_ms,
                    fallback_to_native = job.fallback_to_native,
                    "media job capability"
                );
            }
        }
        "serve" => serve().await?,
        command => {
            bail!(
                "unknown command '{command}'; expected 'doctor', 'probe', 'catalog', 'serve', or 'smoke [output.webm]'"
            )
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_contract_is_versioned_and_value_free() {
        let response = HealthResponse {
            schema_version: HEALTH_SCHEMA_VERSION,
            service: "frame-media-worker",
            status: "live",
            catalog_version: media_job_catalog().version,
            required_factories_unavailable: 0,
        };
        assert_eq!(response.schema_version, 1);
        assert_eq!(response.service, "frame-media-worker");
        assert_eq!(response.status, "live");
        assert!(response.catalog_version > 0);
    }

    #[test]
    fn readiness_exposes_only_counts_and_stable_labels() {
        let (_, Json(response)) = health_response();
        assert_eq!(response.service, "frame-media-worker");
        assert!(matches!(response.status, "ready" | "degraded"));
        assert!(response.required_factories_unavailable <= 32);
    }
}
