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
mod legacy_analytics;
mod legacy_collaboration;
mod legacy_compatibility;
mod legacy_core_storage;
mod legacy_desktop_compatibility;
mod legacy_desktop_session;
mod legacy_developer_actions;
mod legacy_developer_api;
mod legacy_extension_auth;
mod legacy_extension_instant_recordings;
mod legacy_folder_assignment;
mod legacy_folder_crud;
mod legacy_invite_lifecycle;
mod legacy_library_detail_reads;
mod legacy_library_id_reads;
mod legacy_library_placement;
mod legacy_membership_actions;
mod legacy_mobile_bootstrap_caps;
mod legacy_mobile_session;
mod legacy_mobile_uploads;
mod legacy_notification_actions;
mod legacy_notification_preferences;
mod legacy_notification_read;
mod legacy_org_custom_domain;
mod legacy_organization_library;
mod legacy_organization_selection;
mod legacy_protected_billing_auth;
mod legacy_protected_integrations;
mod legacy_protected_media;
mod legacy_space_authorization;
mod legacy_theme;
mod legacy_transcripts;
mod legacy_upload_storage;
mod legacy_user_account;
mod legacy_video_domain_info;
mod legacy_video_lifecycle;
mod legacy_video_properties;
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
pub use legacy_analytics::*;
pub use legacy_collaboration::*;
pub use legacy_compatibility::*;
pub use legacy_core_storage::*;
pub use legacy_desktop_compatibility::*;
pub use legacy_desktop_session::*;
pub use legacy_developer_actions::*;
pub use legacy_developer_api::*;
pub use legacy_extension_auth::*;
pub use legacy_extension_instant_recordings::*;
pub use legacy_folder_assignment::*;
pub use legacy_folder_crud::*;
pub use legacy_invite_lifecycle::*;
pub use legacy_library_detail_reads::*;
pub use legacy_library_id_reads::*;
pub use legacy_library_placement::*;
pub use legacy_membership_actions::*;
pub use legacy_mobile_bootstrap_caps::*;
pub use legacy_mobile_session::*;
pub use legacy_mobile_uploads::*;
pub use legacy_notification_actions::*;
pub use legacy_notification_preferences::*;
pub use legacy_notification_read::*;
pub use legacy_org_custom_domain::*;
pub use legacy_organization_library::*;
pub use legacy_organization_selection::*;
pub use legacy_protected_billing_auth::*;
pub use legacy_protected_integrations::*;
pub use legacy_protected_media::*;
pub use legacy_space_authorization::*;
pub use legacy_theme::*;
pub use legacy_transcripts::*;
pub use legacy_upload_storage::*;
pub use legacy_user_account::*;
pub use legacy_video_domain_info::*;
pub use legacy_video_lifecycle::*;
pub use legacy_video_properties::*;
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
