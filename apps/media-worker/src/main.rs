use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use frame_media::{probe_runtime, record_synthetic_webm};
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    match env::args().nth(1).as_deref().unwrap_or("probe") {
        "probe" => {
            let runtime = probe_runtime().context("GStreamer probe failed")?;
            info!(version = %runtime.version, plugins = ?runtime.required_factories, "media runtime ready");
        }
        "smoke" => {
            let output = env::args()
                .nth(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("target/frame-smoke.webm"));
            record_synthetic_webm(&output).context("synthetic GStreamer pipeline failed")?;
            info!(path = %output.display(), "synthetic media artifact created");
        }
        command => {
            bail!("unknown command '{command}'; expected 'probe' or 'smoke [output.webm]'")
        }
    }

    Ok(())
}
