//! Application services that coordinate domain state machines and infrastructure ports.
//!
//! The services in this crate are deliberately runtime-neutral: they compile for native
//! binaries and `wasm32-unknown-unknown`, while adapters remain in the executable crates.

mod api_workflow;
mod backfill;
mod business;
mod cutover;
mod idempotency;
mod identity;
mod media_router;
mod multipart;
mod organization;
mod storage;
mod storage_governance;
mod upload;

pub use api_workflow::*;
pub use backfill::{BackfillProcessOutcomeV1, ObjectBackfillCoordinatorV1, ObjectBackfillErrorV1};
pub use business::{BusinessDataService, BusinessServiceError};
pub use cutover::CutoverCoordinator;
pub use idempotency::{CommandClaim, CommandFingerprint, CommandLedger, CommandStatus};
pub use identity::{
    AbuseContext, AuthFailure, AuthHashKey, AuthHashKeyRing, AuthPolicy, AuthService,
    AuthenticatedIdentity, BrowserMutationRequest, IssuedApiKey, IssuedSession, LogoutAllReceipt,
    OAuthCompletionOutcome, OAuthProviderPolicy, OAuthStart, PkceChallengeMethod,
    PrincipalAssurance, ValidatedBrowserMutationProof, VerificationConsumeOutcome,
    VerificationIssueReceipt, VerifiedIdentityProvisioning, VerifiedPrincipal,
};
pub use media_router::{MediaRoute, MediaRouter, MediaRoutingPolicy, RoutedTransform};
pub use multipart::*;
pub use organization::*;
pub use storage::{ImmutableStorageService, ImmutableWriteOutcome};
pub use storage_governance::*;
pub use upload::{BeginUpload, MultipartUploadCoordinator, UploadPartReceipt, UploadSession};

use std::fmt;

use frame_ports::PortError;
use thiserror::Error;

/// Stable application failures. Adapter details are intentionally redacted from `Debug`.
#[derive(Clone, Error, PartialEq, Eq)]
pub enum ApplicationError {
    #[error("resource not found")]
    NotFound,
    #[error("request conflicts with current state")]
    Conflict,
    #[error("request is invalid")]
    Invalid,
    #[error("capability is unsupported")]
    Unsupported,
    #[error("service is temporarily unavailable")]
    Unavailable,
    #[error("internal application failure")]
    Internal,
}

impl fmt::Debug for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFound => "NotFound",
            Self::Conflict => "Conflict",
            Self::Invalid => "Invalid",
            Self::Unsupported => "Unsupported",
            Self::Unavailable => "Unavailable",
            Self::Internal => "Internal",
        })
    }
}

impl From<PortError> for ApplicationError {
    fn from(error: PortError) -> Self {
        match error {
            PortError::NotFound => Self::NotFound,
            PortError::Conflict => Self::Conflict,
            PortError::InvalidRequest(_) => Self::Invalid,
            PortError::Unsupported(_) => Self::Unsupported,
            PortError::Adapter(_) => Self::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_details_are_never_formatted() {
        let error = ApplicationError::from(PortError::Adapter(
            "https://signed.example/object?token=secret".into(),
        ));
        assert_eq!(error, ApplicationError::Unavailable);
        assert_eq!(format!("{error:?}"), "Unavailable");
        assert!(!format!("{error}").contains("secret"));
    }
}
