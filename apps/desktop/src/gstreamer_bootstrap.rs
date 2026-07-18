//! Pre-thread GStreamer bootstrap for the native macOS desktop binary.

use std::{os::unix::process::CommandExt, process::Command};

use frame_media::{DesktopRuntimeLaunchPlan, MediaError, desktop_runtime_launch_plan};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DesktopGStreamerBootstrapError {
    #[error(transparent)]
    Runtime(#[from] MediaError),
    #[error("could not replace the desktop process with its trusted GStreamer environment: {0}")]
    Reexec(#[source] std::io::Error),
}

/// Makes a raw release executable, or a local bundle still in the canonical
/// Cargo target tree, replace itself before Tauri or GStreamer creates threads.
/// `exec` preserves the process identity and signal/exit behavior; the second
/// image is ready by construction, so no recursion marker or forgeable
/// environment token is needed.
pub fn bootstrap_desktop_gstreamer() -> Result<(), DesktopGStreamerBootstrapError> {
    match desktop_runtime_launch_plan()? {
        DesktopRuntimeLaunchPlan::Ready => Ok(()),
        DesktopRuntimeLaunchPlan::Reexec(plan) => {
            let mut command = Command::new(plan.executable());
            command.args(std::env::args_os().skip(1));
            plan.apply_to(&mut command);
            Err(DesktopGStreamerBootstrapError::Reexec(command.exec()))
        }
    }
}
