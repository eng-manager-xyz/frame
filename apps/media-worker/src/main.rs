use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result, bail};
use axum::{Json, Router, http::StatusCode, routing::get};
use frame_media::{
    FactoryRequirement, diagnose_runtime, media_job_catalog, probe_runtime, record_synthetic_webm,
};
use serde::Serialize;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod consumer;
mod native;
mod protocol;
mod thumbnail;

use consumer::{WorkOutcome, run_consumer_loop, work_once};
use native::{NativeProfile, ensure_worker_runtime, run_native_child};
use protocol::{WorkerClient, WorkerConfig};
use thumbnail::{
    ReadyGstreamerRuntime, thumbnail_factory_count, thumbnail_runtime_capability,
    thumbnail_runtime_missing_factories,
};

const HEALTH_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Serialize)]
struct HealthResponse {
    schema_version: u16,
    service: &'static str,
    status: &'static str,
    catalog_version: u16,
    required_factories_unavailable: usize,
    native_profiles_declared: usize,
    native_profiles_implemented: usize,
    native_profiles_blocked: usize,
}

fn health_response() -> (StatusCode, Json<HealthResponse>) {
    let diagnostics = diagnose_runtime();
    health_response_with(diagnostics, thumbnail_runtime_missing_factories)
}

fn health_response_with(
    diagnostics: frame_media::RuntimeDiagnostics,
    thumbnail_probe: impl FnOnce(&ReadyGstreamerRuntime) -> usize,
) -> (StatusCode, Json<HealthResponse>) {
    let missing = diagnostics
        .factories
        .iter()
        .filter(|factory| factory.requirement == FactoryRequirement::Required && !factory.available)
        .count();
    let runtime = thumbnail_runtime_capability(&diagnostics);
    let thumbnail_missing = runtime
        .as_ref()
        .map_or_else(thumbnail_factory_count, thumbnail_probe);
    let ready = diagnostics.is_ready() && runtime.is_some() && thumbnail_missing == 0;
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
            required_factories_unavailable: missing.max(thumbnail_missing),
            native_profiles_declared: NativeProfile::ALL.len(),
            native_profiles_implemented: NativeProfile::ALL
                .iter()
                .filter(|profile| profile.has_implemented_graph())
                .count(),
            native_profiles_blocked: NativeProfile::ALL
                .iter()
                .filter(|profile| !profile.has_implemented_graph())
                .count(),
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
        native_profiles_declared: NativeProfile::ALL.len(),
        native_profiles_implemented: NativeProfile::ALL
            .iter()
            .filter(|profile| profile.has_implemented_graph())
            .count(),
        native_profiles_blocked: NativeProfile::ALL
            .iter()
            .filter(|profile| !profile.has_implemented_graph())
            .count(),
    })
}

async fn ready() -> (StatusCode, Json<HealthResponse>) {
    health_response()
}

async fn serve() -> Result<()> {
    let worker_client = WorkerConfig::from_env_optional()?
        .map(WorkerClient::new)
        .transpose()?;
    if worker_client.is_some() {
        ensure_worker_runtime()?;
    }
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
    let (shutdown_sender, shutdown_receiver) = tokio::sync::watch::channel(false);
    let mut consumer = worker_client.map(|client| {
        info!("native media protocol consumer enabled");
        tokio::spawn(run_consumer_loop(client, shutdown_receiver))
    });
    let graceful_sender = shutdown_sender.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            if let Err(error) = tokio::signal::ctrl_c().await {
                warn!(%error, "failed to install shutdown signal handler");
            }
            let _ = graceful_sender.send(true);
        })
        .await
        .context("native media health service failed")?;
    let _ = shutdown_sender.send(true);
    if let Some(handle) = consumer.as_mut()
        && tokio::time::timeout(std::time::Duration::from_secs(50), &mut *handle)
            .await
            .is_err()
    {
        handle.abort();
        let _ = handle.await;
    }
    Ok(())
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
        "work-once" => {
            let client = WorkerClient::new(WorkerConfig::from_env_required()?)?;
            match work_once(&client).await? {
                WorkOutcome::Idle => info!("no eligible native media job was available"),
                WorkOutcome::Completed { job_id } => {
                    info!(%job_id, "one native media job completed")
                }
            }
        }
        "native-child" => {
            let mut arguments = env::args_os().skip(2);
            let plan = arguments
                .next()
                .map(PathBuf::from)
                .filter(|_| arguments.next().is_none())
                .ok_or_else(|| anyhow::anyhow!("native child arguments are invalid"))?;
            if let Err(error) = run_native_child(&plan) {
                std::process::exit(error.child_exit_code());
            }
        }
        "serve" => serve().await?,
        command => {
            bail!(
                "unknown command '{command}'; expected 'doctor', 'probe', 'catalog', 'work-once', 'serve', or 'smoke [output.webm]'"
            )
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use frame_media::{DiagnosticIssue, RuntimeDiagnostics};

    #[test]
    fn live_contract_is_versioned_and_value_free() {
        let response = HealthResponse {
            schema_version: HEALTH_SCHEMA_VERSION,
            service: "frame-media-worker",
            status: "live",
            catalog_version: media_job_catalog().version,
            required_factories_unavailable: 0,
            native_profiles_declared: NativeProfile::ALL.len(),
            native_profiles_implemented: 4,
            native_profiles_blocked: 10,
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

    #[test]
    fn hostile_plugin_environment_short_circuits_before_thumbnail_probe() {
        let diagnostics = RuntimeDiagnostics {
            manifest_version: frame_media::RUNTIME_MANIFEST_VERSION,
            runtime_version: None,
            factories: Vec::new(),
            issues: vec![DiagnosticIssue::PluginSearchPathOverride("GST_PLUGIN_PATH")],
        };
        let (status, Json(response)) = health_response_with(diagnostics, |_| {
            panic!("thumbnail probing must not run after runtime trust rejection")
        });
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.status, "degraded");
        assert_eq!(
            response.required_factories_unavailable,
            thumbnail_factory_count()
        );
    }
}
