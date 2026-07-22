//! Security and workflow core for the Frame Tauri desktop shell.
//!
//! The crate is intentionally independent of `tauri` itself. It provides the
//! serializable trust-boundary contracts and deterministic UI models that a
//! thin Tauri command layer can call after workspace integration.

pub mod accessibility;
#[cfg(all(target_os = "macos", feature = "macos-native"))]
pub mod av_settings;
#[cfg(any(
    all(target_os = "macos", feature = "macos-native"),
    all(target_os = "windows", feature = "windows-native")
))]
pub mod gstreamer_bootstrap;
#[cfg(not(target_arch = "wasm32"))]
pub mod instant_finalize;
pub mod instant_finalize_service;
pub mod ipc;
#[cfg(all(target_os = "macos", feature = "macos-native"))]
pub mod macos_native_backend;
pub mod migration;
pub mod native_backend;
#[cfg(any(
    all(target_os = "macos", feature = "macos-native"),
    all(target_os = "windows", feature = "windows-native")
))]
#[path = "macos_native_backend/normalized_worker.rs"]
mod native_screen_worker;
#[cfg(all(target_os = "macos", feature = "macos-native"))]
pub mod rooted_io;
pub mod runtime;
pub mod surface;
#[cfg(all(target_os = "windows", feature = "windows-native"))]
pub mod windows_native_backend;
pub mod workflow;

pub use accessibility::*;
#[cfg(all(target_os = "macos", feature = "macos-native"))]
pub use av_settings::*;
#[cfg(any(
    all(target_os = "macos", feature = "macos-native"),
    all(target_os = "windows", feature = "windows-native")
))]
pub use gstreamer_bootstrap::*;
#[cfg(not(target_arch = "wasm32"))]
pub use instant_finalize::*;
pub use instant_finalize_service::*;
pub use ipc::*;
#[cfg(all(target_os = "macos", feature = "macos-native"))]
pub use macos_native_backend::*;
pub use migration::*;
pub use native_backend::*;
#[cfg(all(target_os = "macos", feature = "macos-native"))]
pub use rooted_io::*;
pub use runtime::*;
pub use surface::*;
#[cfg(all(target_os = "windows", feature = "windows-native"))]
pub use windows_native_backend::*;
pub use workflow::*;
