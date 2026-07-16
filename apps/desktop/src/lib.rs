//! Security and workflow core for the Frame Tauri desktop shell.
//!
//! The crate is intentionally independent of `tauri` itself. It provides the
//! serializable trust-boundary contracts and deterministic UI models that a
//! thin Tauri command layer can call after workspace integration.

pub mod accessibility;
pub mod ipc;
pub mod migration;
pub mod runtime;
pub mod surface;
pub mod workflow;

pub use accessibility::*;
pub use ipc::*;
pub use migration::*;
pub use runtime::*;
pub use surface::*;
pub use workflow::*;
