//! Rust-owned desktop shell operation contracts.
//!
//! The WebView can request these operations only through the versioned
//! [`crate::IpcCommand`] envelope. Platform adapters execute the resulting
//! command outside the runtime lock and return one of these bounded outcomes;
//! they never expose a plugin command, native window handle, updater URL, or
//! update signature to browser code.

use crate::{LifecycleAction, LifecycleSnapshot, PublicErrorCode, UpdateAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopShellCommand {
    Lifecycle {
        action: LifecycleAction,
    },
    Update {
        action: UpdateAction,
        expected_revision: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopShellOutcome {
    LifecycleApplied { snapshot: LifecycleSnapshot },
    UpdateChecked { available: bool },
    UpdateInstalled,
    RelaunchRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopShellFailure {
    pub code: PublicErrorCode,
    pub retryable: bool,
    pub announcement: &'static str,
}

impl DesktopShellFailure {
    #[must_use]
    pub const fn unavailable() -> Self {
        Self {
            code: PublicErrorCode::Unavailable,
            retryable: true,
            announcement: "The desktop shell integration is unavailable. No operation was started.",
        }
    }

    #[must_use]
    pub const fn busy() -> Self {
        Self {
            code: PublicErrorCode::Busy,
            retryable: true,
            announcement: "Another desktop shell operation is still active.",
        }
    }

    #[must_use]
    pub const fn conflict() -> Self {
        Self {
            code: PublicErrorCode::Conflict,
            retryable: true,
            announcement: "Desktop shell state changed before the operation completed.",
        }
    }

    #[must_use]
    pub const fn internal() -> Self {
        Self {
            code: PublicErrorCode::Internal,
            retryable: false,
            announcement: "The desktop shell operation could not be completed.",
        }
    }
}
