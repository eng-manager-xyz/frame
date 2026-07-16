use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use frame_media::{
    FactoryRequirement, diagnose_runtime, media_job_catalog, probe_runtime, record_synthetic_webm,
};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
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
        command => {
            bail!(
                "unknown command '{command}'; expected 'doctor', 'probe', 'catalog', or 'smoke [output.webm]'"
            )
        }
    }

    Ok(())
}
