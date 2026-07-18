//! Source-pinned contracts for Cap's provider-free organization/library actions.
//!
//! The 21 operations in this module have no Stripe, WorkOS, email, analytics,
//! Google API, or arbitrary S3 execution dependency. Their complete effects are
//! D1 state, first-party R2 objects, deterministic local crypto/URL projection,
//! browser-cookie projection, and cache invalidation. Provider-backed siblings
//! remain outside this contract and must continue to fail closed.

use std::fmt;

use async_trait::async_trait;
use frame_domain::{
    IdempotencyKey, LegacyCapNanoId, OrganizationId, SessionId, SessionMutationGrantId, UserId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ValidatedBrowserMutationProof;

pub const LEGACY_ORGANIZATION_LIBRARY_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_ORGANIZATION_LIBRARY_POLICY: &str = "organization_library.v1";
pub const LEGACY_ORGANIZATION_LIBRARY_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_ORGANIZATION_LIBRARY_MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
pub const LEGACY_ORGANIZATION_LIBRARY_REQUEST_SCHEMA_V1: &str =
    "frame.web-organization-library-action.v1";
pub const LEGACY_ORGANIZATION_LIBRARY_OPERATION_COUNT: usize = 21;

pub const LEGACY_SET_COLLECTION_LOGO_OPERATION_ID: &str = "cap-v1-2cbdd906b6b7e371";
pub const LEGACY_VERIFY_COLLECTION_PASSWORD_OPERATION_ID: &str = "cap-v1-61e089033a34d239";
pub const LEGACY_SET_SPACE_COLLECTION_VISIBILITY_OPERATION_ID: &str = "cap-v1-79eeb0016e42f711";
pub const LEGACY_DELETE_SPACE_OPERATION_ID: &str = "cap-v1-120aa129daa79b1e";
pub const LEGACY_GET_ORGANIZATION_SSO_DATA_OPERATION_ID: &str = "cap-v1-9227b0da852f2745";
pub const LEGACY_REMOVE_ORGANIZATION_MEMBER_OPERATION_ID: &str = "cap-v1-575866e31832347a";
pub const LEGACY_UPDATE_ORGANIZATION_SETTINGS_OPERATION_ID: &str = "cap-v1-3a1228254de4338a";
pub const LEGACY_HIDE_SHAREABLE_LINK_CAP_LOGO_OPERATION_ID: &str = "cap-v1-1bed8d446a1553b1";
pub const LEGACY_REMOVE_SHAREABLE_LINK_ICON_OPERATION_ID: &str = "cap-v1-b5f1312195f03a0e";
pub const LEGACY_SELECT_SHAREABLE_LINK_BRANDING_ORGANIZATION_OPERATION_ID: &str =
    "cap-v1-7e1553af9e9427af";
pub const LEGACY_UPDATE_SHAREABLE_LINK_ICON_PREFERENCE_OPERATION_ID: &str =
    "cap-v1-ff1b0a4f37fb9130";
pub const LEGACY_UPLOAD_SHAREABLE_LINK_ICON_OPERATION_ID: &str = "cap-v1-ce276ebd911b73f8";
pub const LEGACY_CONNECT_ORGANIZATION_GOOGLE_DRIVE_OPERATION_ID: &str = "cap-v1-531e69b5e2915e10";
pub const LEGACY_DISCONNECT_ORGANIZATION_GOOGLE_DRIVE_OPERATION_ID: &str =
    "cap-v1-408f009a56471811";
pub const LEGACY_GET_ORGANIZATION_STORAGE_SETTINGS_OPERATION_ID: &str = "cap-v1-dd736ee15a42f26b";
pub const LEGACY_SET_ORGANIZATION_STORAGE_PROVIDER_OPERATION_ID: &str = "cap-v1-0d56f082dce4f861";
pub const LEGACY_TOGGLE_PRO_SEAT_OPERATION_ID: &str = "cap-v1-989b3a5027a3f5c0";
pub const LEGACY_UPDATE_ORGANIZATION_DETAILS_OPERATION_ID: &str = "cap-v1-91184d308c393034";
pub const LEGACY_UPDATE_ORGANIZATION_MEMBER_ROLE_OPERATION_ID: &str = "cap-v1-1ffe1392bb59f2ca";
pub const LEGACY_UPLOAD_SPACE_ICON_OPERATION_ID: &str = "cap-v1-67377a620262de2c";
pub const LEGACY_CREATE_ORGANIZATION_OPERATION_ID: &str = "cap-v1-404c6ea8306ad5a7";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyOrganizationLibraryActionV1 {
    SetCollectionLogo,
    VerifyCollectionPassword,
    SetSpaceCollectionVisibility,
    DeleteSpace,
    GetOrganizationSsoData,
    RemoveOrganizationMember,
    UpdateOrganizationSettings,
    HideShareableLinkCapLogo,
    RemoveShareableLinkIcon,
    SelectShareableLinkBrandingOrganization,
    UpdateShareableLinkIconPreference,
    UploadShareableLinkIcon,
    ConnectOrganizationGoogleDrive,
    DisconnectOrganizationGoogleDrive,
    GetOrganizationStorageSettings,
    SetOrganizationStorageProvider,
    ToggleProSeat,
    UpdateOrganizationDetails,
    UpdateOrganizationMemberRole,
    UploadSpaceIcon,
    CreateOrganization,
}

impl LegacyOrganizationLibraryActionV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::SetCollectionLogo => LEGACY_SET_COLLECTION_LOGO_OPERATION_ID,
            Self::VerifyCollectionPassword => LEGACY_VERIFY_COLLECTION_PASSWORD_OPERATION_ID,
            Self::SetSpaceCollectionVisibility => {
                LEGACY_SET_SPACE_COLLECTION_VISIBILITY_OPERATION_ID
            }
            Self::DeleteSpace => LEGACY_DELETE_SPACE_OPERATION_ID,
            Self::GetOrganizationSsoData => LEGACY_GET_ORGANIZATION_SSO_DATA_OPERATION_ID,
            Self::RemoveOrganizationMember => LEGACY_REMOVE_ORGANIZATION_MEMBER_OPERATION_ID,
            Self::UpdateOrganizationSettings => LEGACY_UPDATE_ORGANIZATION_SETTINGS_OPERATION_ID,
            Self::HideShareableLinkCapLogo => LEGACY_HIDE_SHAREABLE_LINK_CAP_LOGO_OPERATION_ID,
            Self::RemoveShareableLinkIcon => LEGACY_REMOVE_SHAREABLE_LINK_ICON_OPERATION_ID,
            Self::SelectShareableLinkBrandingOrganization => {
                LEGACY_SELECT_SHAREABLE_LINK_BRANDING_ORGANIZATION_OPERATION_ID
            }
            Self::UpdateShareableLinkIconPreference => {
                LEGACY_UPDATE_SHAREABLE_LINK_ICON_PREFERENCE_OPERATION_ID
            }
            Self::UploadShareableLinkIcon => LEGACY_UPLOAD_SHAREABLE_LINK_ICON_OPERATION_ID,
            Self::ConnectOrganizationGoogleDrive => {
                LEGACY_CONNECT_ORGANIZATION_GOOGLE_DRIVE_OPERATION_ID
            }
            Self::DisconnectOrganizationGoogleDrive => {
                LEGACY_DISCONNECT_ORGANIZATION_GOOGLE_DRIVE_OPERATION_ID
            }
            Self::GetOrganizationStorageSettings => {
                LEGACY_GET_ORGANIZATION_STORAGE_SETTINGS_OPERATION_ID
            }
            Self::SetOrganizationStorageProvider => {
                LEGACY_SET_ORGANIZATION_STORAGE_PROVIDER_OPERATION_ID
            }
            Self::ToggleProSeat => LEGACY_TOGGLE_PRO_SEAT_OPERATION_ID,
            Self::UpdateOrganizationDetails => LEGACY_UPDATE_ORGANIZATION_DETAILS_OPERATION_ID,
            Self::UpdateOrganizationMemberRole => {
                LEGACY_UPDATE_ORGANIZATION_MEMBER_ROLE_OPERATION_ID
            }
            Self::UploadSpaceIcon => LEGACY_UPLOAD_SPACE_ICON_OPERATION_ID,
            Self::CreateOrganization => LEGACY_CREATE_ORGANIZATION_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn legacy_identity(self) -> &'static str {
        match self {
            Self::SetCollectionLogo => {
                "action://apps/web/actions/collections/logo.ts#setCollectionLogo"
            }
            Self::VerifyCollectionPassword => {
                "action://apps/web/actions/collections/password.ts#verifyCollectionPassword"
            }
            Self::SetSpaceCollectionVisibility => {
                "action://apps/web/actions/collections/visibility.ts#setSpaceCollectionVisibility"
            }
            Self::DeleteSpace => {
                "action://apps/web/actions/organization/delete-space.ts#deleteSpace"
            }
            Self::GetOrganizationSsoData => {
                "action://apps/web/actions/organization/get-organization-sso-data.ts#getOrganizationSSOData"
            }
            Self::RemoveOrganizationMember => {
                "action://apps/web/actions/organization/remove-member.ts#removeOrganizationMember"
            }
            Self::UpdateOrganizationSettings => {
                "action://apps/web/actions/organization/settings.ts#updateOrganizationSettings"
            }
            Self::HideShareableLinkCapLogo => {
                "action://apps/web/actions/organization/shareable-link-icon.ts#hideShareableLinkCapLogo"
            }
            Self::RemoveShareableLinkIcon => {
                "action://apps/web/actions/organization/shareable-link-icon.ts#removeShareableLinkIcon"
            }
            Self::SelectShareableLinkBrandingOrganization => {
                "action://apps/web/actions/organization/shareable-link-icon.ts#selectShareableLinkBrandingOrganization"
            }
            Self::UpdateShareableLinkIconPreference => {
                "action://apps/web/actions/organization/shareable-link-icon.ts#updateShareableLinkIconPreference"
            }
            Self::UploadShareableLinkIcon => {
                "action://apps/web/actions/organization/shareable-link-icon.ts#uploadShareableLinkIcon"
            }
            Self::ConnectOrganizationGoogleDrive => {
                "action://apps/web/actions/organization/storage.ts#connectOrganizationGoogleDrive"
            }
            Self::DisconnectOrganizationGoogleDrive => {
                "action://apps/web/actions/organization/storage.ts#disconnectOrganizationGoogleDrive"
            }
            Self::GetOrganizationStorageSettings => {
                "action://apps/web/actions/organization/storage.ts#getOrganizationStorageSettings"
            }
            Self::SetOrganizationStorageProvider => {
                "action://apps/web/actions/organization/storage.ts#setOrganizationStorageProvider"
            }
            Self::ToggleProSeat => {
                "action://apps/web/actions/organization/toggle-pro-seat.ts#toggleProSeat"
            }
            Self::UpdateOrganizationDetails => {
                "action://apps/web/actions/organization/update-details.ts#updateOrganizationDetails"
            }
            Self::UpdateOrganizationMemberRole => {
                "action://apps/web/actions/organization/update-member-role.ts#updateOrganizationMemberRole"
            }
            Self::UploadSpaceIcon => {
                "action://apps/web/actions/organization/upload-space-icon.ts#uploadSpaceIcon"
            }
            Self::CreateOrganization => {
                "action://apps/web/components/forms/server.ts#createOrganization"
            }
        }
    }

    #[must_use]
    pub const fn source_path(self) -> &'static str {
        match self {
            Self::SetCollectionLogo => "apps/web/actions/collections/logo.ts",
            Self::VerifyCollectionPassword => "apps/web/actions/collections/password.ts",
            Self::SetSpaceCollectionVisibility => "apps/web/actions/collections/visibility.ts",
            Self::DeleteSpace => "apps/web/actions/organization/delete-space.ts",
            Self::GetOrganizationSsoData => {
                "apps/web/actions/organization/get-organization-sso-data.ts"
            }
            Self::RemoveOrganizationMember => "apps/web/actions/organization/remove-member.ts",
            Self::UpdateOrganizationSettings => "apps/web/actions/organization/settings.ts",
            Self::HideShareableLinkCapLogo
            | Self::RemoveShareableLinkIcon
            | Self::SelectShareableLinkBrandingOrganization
            | Self::UpdateShareableLinkIconPreference
            | Self::UploadShareableLinkIcon => {
                "apps/web/actions/organization/shareable-link-icon.ts"
            }
            Self::ConnectOrganizationGoogleDrive
            | Self::DisconnectOrganizationGoogleDrive
            | Self::GetOrganizationStorageSettings
            | Self::SetOrganizationStorageProvider => "apps/web/actions/organization/storage.ts",
            Self::ToggleProSeat => "apps/web/actions/organization/toggle-pro-seat.ts",
            Self::UpdateOrganizationDetails => "apps/web/actions/organization/update-details.ts",
            Self::UpdateOrganizationMemberRole => {
                "apps/web/actions/organization/update-member-role.ts"
            }
            Self::UploadSpaceIcon => "apps/web/actions/organization/upload-space-icon.ts",
            Self::CreateOrganization => "apps/web/components/forms/server.ts",
        }
    }

    #[must_use]
    pub const fn source_sha256(self) -> &'static str {
        match self {
            Self::SetCollectionLogo => {
                "6da1160c77a218c7d7610f091efac9ab82f4f423734fd4e034590e66b9b3d86a"
            }
            Self::VerifyCollectionPassword => {
                "b9279c5520cfe51eb327f31c93bbcf5a1d9c99308e9ccc54fb9343e2c2fb37ec"
            }
            Self::SetSpaceCollectionVisibility => {
                "37cebf7e6b86a5b81184b29120771680651b3584d18751f6ca6c36994e2b76ba"
            }
            Self::DeleteSpace => "62b0fb5e690021a4fff8f7790da7406c3848e71d068c494fa80464320fdca213",
            Self::GetOrganizationSsoData => {
                "d95337ff755ff0a76e076b5086af79b0087729a32f1b4fbbbea42144092b59ca"
            }
            Self::RemoveOrganizationMember => {
                "f41c5ff6f58aecc8e8462adafabb9cf5749b36b97802827c7304c9ff419b2729"
            }
            Self::UpdateOrganizationSettings => {
                "1395abd9ebf7b71c53fb6395aa71566d0114ee36d7d3ebfef886f163bf2bbd7b"
            }
            Self::HideShareableLinkCapLogo
            | Self::RemoveShareableLinkIcon
            | Self::SelectShareableLinkBrandingOrganization
            | Self::UpdateShareableLinkIconPreference
            | Self::UploadShareableLinkIcon => {
                "a14b88b3ee917aa16c2e4d4c7800c35fb8042cd508cd926f60d2b74883d0334f"
            }
            Self::ConnectOrganizationGoogleDrive
            | Self::DisconnectOrganizationGoogleDrive
            | Self::GetOrganizationStorageSettings
            | Self::SetOrganizationStorageProvider => {
                "25c64e9cacfe2048160d6a8fb37c95b75ec06f07de8f04b94fad939f40a86de5"
            }
            Self::ToggleProSeat => {
                "33744f1aa115a4fb9d4b505a6fa93eb7640a0b029eb9261d0e4f5287e377bb9d"
            }
            Self::UpdateOrganizationDetails => {
                "82b1702a9592e8e6224e447312554eb2afbdcb2b3abc352c1218162d4c2c4946"
            }
            Self::UpdateOrganizationMemberRole => {
                "6561e95488476e58a39a98ead43ae40af9078059347625188c77b0a05da477ee"
            }
            Self::UploadSpaceIcon => {
                "0b66960cfe59616dd4d64390f9db0dc537a9d7ccad4ef5311dc97b5e6a0d2063"
            }
            Self::CreateOrganization => {
                "73cbaead68461609716d42e4b4e4e9d12d286b9b4d866d23550d40a8df7a4d26"
            }
        }
    }

    #[must_use]
    pub const fn requires_session(self) -> bool {
        !matches!(self, Self::VerifyCollectionPassword)
    }

    #[must_use]
    pub const fn requires_active_tenant(self) -> bool {
        !matches!(
            self,
            Self::VerifyCollectionPassword
                | Self::SelectShareableLinkBrandingOrganization
                | Self::CreateOrganization
        )
    }

    #[must_use]
    pub const fn uses_r2(self) -> bool {
        matches!(
            self,
            Self::SetCollectionLogo
                | Self::DeleteSpace
                | Self::RemoveShareableLinkIcon
                | Self::UploadShareableLinkIcon
                | Self::UploadSpaceIcon
                | Self::CreateOrganization
        )
    }

    #[must_use]
    pub const fn all() -> [Self; LEGACY_ORGANIZATION_LIBRARY_OPERATION_COUNT] {
        [
            Self::SetCollectionLogo,
            Self::VerifyCollectionPassword,
            Self::SetSpaceCollectionVisibility,
            Self::DeleteSpace,
            Self::GetOrganizationSsoData,
            Self::RemoveOrganizationMember,
            Self::UpdateOrganizationSettings,
            Self::HideShareableLinkCapLogo,
            Self::RemoveShareableLinkIcon,
            Self::SelectShareableLinkBrandingOrganization,
            Self::UpdateShareableLinkIconPreference,
            Self::UploadShareableLinkIcon,
            Self::ConnectOrganizationGoogleDrive,
            Self::DisconnectOrganizationGoogleDrive,
            Self::GetOrganizationStorageSettings,
            Self::SetOrganizationStorageProvider,
            Self::ToggleProSeat,
            Self::UpdateOrganizationDetails,
            Self::UpdateOrganizationMemberRole,
            Self::UploadSpaceIcon,
            Self::CreateOrganization,
        ]
    }

    #[must_use]
    pub fn from_operation_id(value: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|action| action.operation_id() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrganizationLibraryProfileV1 {
    pub action: LegacyOrganizationLibraryActionV1,
    pub operation_id: &'static str,
    pub legacy_identity: &'static str,
    pub source_path: &'static str,
    pub source_sha256: &'static str,
    pub authentication: &'static str,
    pub idempotency: &'static str,
    pub policy: &'static str,
    pub uses_r2: bool,
    pub protected_gates: &'static [&'static str],
    pub production_promoted: bool,
}

#[must_use]
pub const fn legacy_organization_library_profile(
    action: LegacyOrganizationLibraryActionV1,
) -> LegacyOrganizationLibraryProfileV1 {
    LegacyOrganizationLibraryProfileV1 {
        action,
        operation_id: action.operation_id(),
        legacy_identity: action.legacy_identity(),
        source_path: action.source_path(),
        source_sha256: action.source_sha256(),
        authentication: if action.requires_session() {
            "session"
        } else {
            "anonymous"
        },
        idempotency: if action.requires_session() {
            "required"
        } else {
            "forbidden"
        },
        policy: LEGACY_ORGANIZATION_LIBRARY_POLICY,
        uses_r2: action.uses_r2(),
        protected_gates: &[],
        production_promoted: true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyCollectionKindV1 {
    Folder,
    Space,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyImagePayloadV1 {
    pub file_name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub checksum_sha256: String,
}

impl fmt::Debug for LegacyImagePayloadV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyImagePayloadV1")
            .field("file_name", &"<redacted>")
            .field("content_type", &self.content_type)
            .field("bytes", &self.bytes.len())
            .field("checksum_sha256", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegacyOrganizationStorageProviderV1 {
    S3,
    GoogleDrive,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LegacyOrganizationLibraryInputV1 {
    SetCollectionLogo {
        legacy_collection_id: String,
        kind: LegacyCollectionKindV1,
        remove: bool,
        image: Option<LegacyImagePayloadV1>,
    },
    VerifyCollectionPassword {
        legacy_collection_id: String,
        password: String,
    },
    SetSpaceCollectionVisibility {
        legacy_space_id: String,
        public: Option<bool>,
        settings_patch: Option<serde_json::Value>,
    },
    DeleteSpace {
        legacy_space_id: String,
    },
    GetOrganizationSsoData {
        legacy_organization_id: String,
    },
    RemoveOrganizationMember {
        legacy_member_id: String,
        legacy_organization_id: String,
    },
    UpdateOrganizationSettings {
        settings: serde_json::Value,
    },
    HideShareableLinkCapLogo {
        legacy_organization_id: String,
    },
    RemoveShareableLinkIcon {
        legacy_organization_id: String,
    },
    SelectShareableLinkBrandingOrganization {
        legacy_organization_id: String,
    },
    UpdateShareableLinkIconPreference {
        legacy_organization_id: String,
        use_organization_icon: bool,
    },
    UploadShareableLinkIcon {
        legacy_organization_id: String,
        image: LegacyImagePayloadV1,
    },
    ConnectOrganizationGoogleDrive {
        legacy_organization_id: String,
    },
    DisconnectOrganizationGoogleDrive {
        legacy_organization_id: String,
    },
    GetOrganizationStorageSettings {
        legacy_organization_id: String,
    },
    SetOrganizationStorageProvider {
        legacy_organization_id: String,
        provider: LegacyOrganizationStorageProviderV1,
    },
    ToggleProSeat {
        legacy_member_id: String,
        legacy_organization_id: String,
        enable: bool,
    },
    UpdateOrganizationDetails {
        legacy_organization_id: String,
        organization_name: Option<String>,
        allowed_email_domain: Option<String>,
    },
    UpdateOrganizationMemberRole {
        legacy_member_id: String,
        legacy_organization_id: String,
        role: String,
    },
    UploadSpaceIcon {
        legacy_space_id: String,
        image: LegacyImagePayloadV1,
    },
    CreateOrganization {
        name: String,
        icon: Option<LegacyImagePayloadV1>,
    },
}

impl LegacyOrganizationLibraryInputV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyOrganizationLibraryActionV1 {
        match self {
            Self::SetCollectionLogo { .. } => LegacyOrganizationLibraryActionV1::SetCollectionLogo,
            Self::VerifyCollectionPassword { .. } => {
                LegacyOrganizationLibraryActionV1::VerifyCollectionPassword
            }
            Self::SetSpaceCollectionVisibility { .. } => {
                LegacyOrganizationLibraryActionV1::SetSpaceCollectionVisibility
            }
            Self::DeleteSpace { .. } => LegacyOrganizationLibraryActionV1::DeleteSpace,
            Self::GetOrganizationSsoData { .. } => {
                LegacyOrganizationLibraryActionV1::GetOrganizationSsoData
            }
            Self::RemoveOrganizationMember { .. } => {
                LegacyOrganizationLibraryActionV1::RemoveOrganizationMember
            }
            Self::UpdateOrganizationSettings { .. } => {
                LegacyOrganizationLibraryActionV1::UpdateOrganizationSettings
            }
            Self::HideShareableLinkCapLogo { .. } => {
                LegacyOrganizationLibraryActionV1::HideShareableLinkCapLogo
            }
            Self::RemoveShareableLinkIcon { .. } => {
                LegacyOrganizationLibraryActionV1::RemoveShareableLinkIcon
            }
            Self::SelectShareableLinkBrandingOrganization { .. } => {
                LegacyOrganizationLibraryActionV1::SelectShareableLinkBrandingOrganization
            }
            Self::UpdateShareableLinkIconPreference { .. } => {
                LegacyOrganizationLibraryActionV1::UpdateShareableLinkIconPreference
            }
            Self::UploadShareableLinkIcon { .. } => {
                LegacyOrganizationLibraryActionV1::UploadShareableLinkIcon
            }
            Self::ConnectOrganizationGoogleDrive { .. } => {
                LegacyOrganizationLibraryActionV1::ConnectOrganizationGoogleDrive
            }
            Self::DisconnectOrganizationGoogleDrive { .. } => {
                LegacyOrganizationLibraryActionV1::DisconnectOrganizationGoogleDrive
            }
            Self::GetOrganizationStorageSettings { .. } => {
                LegacyOrganizationLibraryActionV1::GetOrganizationStorageSettings
            }
            Self::SetOrganizationStorageProvider { .. } => {
                LegacyOrganizationLibraryActionV1::SetOrganizationStorageProvider
            }
            Self::ToggleProSeat { .. } => LegacyOrganizationLibraryActionV1::ToggleProSeat,
            Self::UpdateOrganizationDetails { .. } => {
                LegacyOrganizationLibraryActionV1::UpdateOrganizationDetails
            }
            Self::UpdateOrganizationMemberRole { .. } => {
                LegacyOrganizationLibraryActionV1::UpdateOrganizationMemberRole
            }
            Self::UploadSpaceIcon { .. } => LegacyOrganizationLibraryActionV1::UploadSpaceIcon,
            Self::CreateOrganization { .. } => {
                LegacyOrganizationLibraryActionV1::CreateOrganization
            }
        }
    }
}

impl fmt::Debug for LegacyOrganizationLibraryInputV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyOrganizationLibraryInputV1")
            .field("action", &self.action())
            .field("payload", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOrganizationLibraryCredentialV1 {
    Session,
    Public,
}

#[derive(Clone, PartialEq)]
pub struct LegacyOrganizationLibraryRequestV1 {
    pub credential: Option<LegacyOrganizationLibraryCredentialV1>,
    pub actor_id: Option<UserId>,
    pub active_organization_id: Option<OrganizationId>,
    pub idempotency_key: Option<String>,
    pub input: LegacyOrganizationLibraryInputV1,
}

impl fmt::Debug for LegacyOrganizationLibraryRequestV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyOrganizationLibraryRequestV1")
            .field("credential", &self.credential)
            .field("actor", &self.actor_id.map(|_| "<redacted>"))
            .field(
                "active_organization",
                &self.active_organization_id.map(|_| "<redacted>"),
            )
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("input", &self.input)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrganizationLibraryBrowserFenceV1 {
    mutation_grant_id: SessionMutationGrantId,
    session_id: SessionId,
    actor_id: UserId,
}

impl LegacyOrganizationLibraryBrowserFenceV1 {
    #[must_use]
    pub fn from_validated_proof(proof: &ValidatedBrowserMutationProof) -> Self {
        Self {
            mutation_grant_id: proof.mutation_grant_id(),
            session_id: proof.session_id(),
            actor_id: proof.user_id(),
        }
    }

    #[must_use]
    pub const fn mutation_grant_id(self) -> SessionMutationGrantId {
        self.mutation_grant_id
    }

    #[must_use]
    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    #[must_use]
    pub const fn actor_id(self) -> UserId {
        self.actor_id
    }

    #[cfg(test)]
    fn fixture(actor_id: UserId) -> Self {
        Self {
            mutation_grant_id: SessionMutationGrantId::new(),
            session_id: SessionId::new(),
            actor_id,
        }
    }
}

impl fmt::Debug for LegacyOrganizationLibraryBrowserFenceV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LegacyOrganizationLibraryBrowserFenceV1([redacted])")
    }
}

#[derive(Clone, PartialEq)]
pub struct LegacyOrganizationLibraryCommandV1 {
    action: LegacyOrganizationLibraryActionV1,
    actor_id: Option<UserId>,
    active_organization_id: Option<OrganizationId>,
    idempotency_key: Option<IdempotencyKey>,
    input: LegacyOrganizationLibraryInputV1,
    request_digest: [u8; 32],
}

impl LegacyOrganizationLibraryCommandV1 {
    #[must_use]
    pub const fn action(&self) -> LegacyOrganizationLibraryActionV1 {
        self.action
    }

    #[must_use]
    pub const fn actor_id(&self) -> Option<UserId> {
        self.actor_id
    }

    #[must_use]
    pub const fn active_organization_id(&self) -> Option<OrganizationId> {
        self.active_organization_id
    }

    #[must_use]
    pub const fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }

    #[must_use]
    pub const fn input(&self) -> &LegacyOrganizationLibraryInputV1 {
        &self.input
    }

    #[must_use]
    pub const fn request_digest(&self) -> &[u8; 32] {
        &self.request_digest
    }

    #[must_use]
    pub fn request_digest_hex(&self) -> String {
        hex_digest(&self.request_digest)
    }

    #[must_use]
    pub fn deterministic_legacy_organization_id(&self) -> Option<String> {
        matches!(
            self.action,
            LegacyOrganizationLibraryActionV1::CreateOrganization
        )
        .then(|| legacy_id_from_digest(&self.request_digest))
    }
}

impl fmt::Debug for LegacyOrganizationLibraryCommandV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LegacyOrganizationLibraryCommandV1")
            .field("action", &self.action)
            .field("actor", &self.actor_id.map(|_| "<redacted>"))
            .field("tenant", &self.active_organization_id.map(|_| "<redacted>"))
            .field(
                "idempotency",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("input", &self.input)
            .field("request_digest", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyOrganizationLibraryResultV1 {
    Success,
    PasswordVerified {
        password_hash: String,
    },
    PasswordRejected,
    OrganizationSsoData {
        organization_id: String,
        connection_id: String,
        name: String,
    },
    GoogleDriveAuthorization {
        url: String,
    },
    OrganizationStorageSettings {
        settings: serde_json::Value,
    },
    OrganizationCreated {
        legacy_organization_id: String,
    },
    IconUploaded {
        object_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyOrganizationLibraryEffectsV1 {
    pub invalidation_paths: Vec<String>,
    pub set_verified_password_cookie: bool,
    pub r2_keys_written: Vec<String>,
    pub r2_keys_deleted: Vec<String>,
}

impl LegacyOrganizationLibraryEffectsV1 {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.invalidation_paths.len() <= 8
            && self.r2_keys_written.len() <= 2
            && self.r2_keys_deleted.len() <= 10_000
            && self.invalidation_paths.iter().all(|path| {
                path.starts_with('/') && path.len() <= 512 && !path.chars().any(char::is_control)
            })
            && self
                .r2_keys_written
                .iter()
                .chain(self.r2_keys_deleted.iter())
                .all(|key| valid_object_key(key))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyOrganizationLibraryReceiptV1 {
    pub action: LegacyOrganizationLibraryActionV1,
    pub request_digest: [u8; 32],
    pub result: LegacyOrganizationLibraryResultV1,
    pub effects: LegacyOrganizationLibraryEffectsV1,
}

impl LegacyOrganizationLibraryReceiptV1 {
    #[must_use]
    pub fn matches(&self, command: &LegacyOrganizationLibraryCommandV1) -> bool {
        self.action == command.action
            && self.request_digest == command.request_digest
            && self.effects.valid()
            && result_matches_action(&self.result, command.action)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LegacyOrganizationLibraryAtomicOutcomeV1 {
    Applied(LegacyOrganizationLibraryReceiptV1),
    Replay(LegacyOrganizationLibraryReceiptV1),
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum LegacyOrganizationLibraryAtomicErrorV1 {
    #[error("organization or library target was not found")]
    NotFound,
    #[error("organization or library target was not found")]
    AccessDenied,
    #[error("organization or library request conflicts with current state")]
    Conflict,
    #[error("organization or library request is already in flight")]
    InFlight,
    #[error("organization or library authority is unavailable")]
    Unavailable,
    #[error("organization or library authority returned corrupt state")]
    Corrupt,
}

#[async_trait]
pub trait LegacyOrganizationLibraryAtomicPortV1: Send + Sync {
    /// Execute one source-pinned operation. Session operations must assert and
    /// consume `browser_fence` in the same D1 transaction as the business
    /// mutation, replay receipt, audit, and authority-version changes. Public
    /// password verification must receive `None`, never consume a session
    /// grant, and may only disclose the source-shaped valid/invalid result.
    /// R2 operations use deterministic keys and durable operation states so a
    /// retry never reports success before both D1 and R2 postconditions hold.
    async fn execute_atomic(
        &self,
        command: &LegacyOrganizationLibraryCommandV1,
        browser_fence: Option<&LegacyOrganizationLibraryBrowserFenceV1>,
    ) -> Result<LegacyOrganizationLibraryAtomicOutcomeV1, LegacyOrganizationLibraryAtomicErrorV1>;
}

#[derive(Clone, Error, PartialEq, Eq)]
pub enum LegacyOrganizationLibraryErrorV1 {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Invalid request")]
    Invalid,
    #[error("An idempotency key is required")]
    IdempotencyRequired,
    #[error("Organization or library target not found")]
    NotFound,
    #[error("Organization or library request conflicts with current state")]
    Conflict,
    #[error("Organization or library authority is unavailable")]
    Unavailable,
    #[error("Organization or library action failed")]
    Internal,
}

impl fmt::Debug for LegacyOrganizationLibraryErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unauthorized => "Unauthorized",
            Self::Invalid => "Invalid",
            Self::IdempotencyRequired => "IdempotencyRequired",
            Self::NotFound => "NotFound",
            Self::Conflict => "Conflict",
            Self::Unavailable => "Unavailable",
            Self::Internal => "Internal",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyOrganizationLibraryAdapterV1 {
    action: LegacyOrganizationLibraryActionV1,
}

impl LegacyOrganizationLibraryAdapterV1 {
    #[must_use]
    pub const fn new(action: LegacyOrganizationLibraryActionV1) -> Self {
        Self { action }
    }

    #[must_use]
    pub const fn action(self) -> LegacyOrganizationLibraryActionV1 {
        self.action
    }

    pub fn prepare(
        &self,
        request: &LegacyOrganizationLibraryRequestV1,
    ) -> Result<LegacyOrganizationLibraryCommandV1, LegacyOrganizationLibraryErrorV1> {
        if request.input.action() != self.action {
            return Err(LegacyOrganizationLibraryErrorV1::Invalid);
        }
        let (actor_id, active_organization_id, idempotency_key) = if self.action.requires_session()
        {
            if request.credential != Some(LegacyOrganizationLibraryCredentialV1::Session) {
                return Err(LegacyOrganizationLibraryErrorV1::Unauthorized);
            }
            let actor_id = request
                .actor_id
                .ok_or(LegacyOrganizationLibraryErrorV1::Unauthorized)?;
            let active_organization_id = match request.active_organization_id {
                Some(value) => Some(value),
                None if !self.action.requires_active_tenant() => None,
                None => return Err(LegacyOrganizationLibraryErrorV1::Unauthorized),
            };
            let idempotency_key = request
                .idempotency_key
                .as_ref()
                .ok_or(LegacyOrganizationLibraryErrorV1::IdempotencyRequired)
                .and_then(|key| {
                    IdempotencyKey::parse(key.clone())
                        .map_err(|_| LegacyOrganizationLibraryErrorV1::Invalid)
                })?;
            (
                Some(actor_id),
                active_organization_id,
                Some(idempotency_key),
            )
        } else {
            if request.credential != Some(LegacyOrganizationLibraryCredentialV1::Public)
                || request.actor_id.is_some()
                || request.active_organization_id.is_some()
                || request.idempotency_key.is_some()
            {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            (None, None, None)
        };

        validate_input(&request.input, active_organization_id)?;
        let request_digest = fingerprint(
            self.action,
            actor_id,
            active_organization_id,
            idempotency_key.as_ref(),
            &request.input,
        )?;
        Ok(LegacyOrganizationLibraryCommandV1 {
            action: self.action,
            actor_id,
            active_organization_id,
            idempotency_key,
            input: request.input.clone(),
            request_digest,
        })
    }

    pub async fn execute<P>(
        &self,
        port: &P,
        request: &LegacyOrganizationLibraryRequestV1,
        proof: Option<&ValidatedBrowserMutationProof>,
    ) -> Result<LegacyOrganizationLibraryExecutionV1, LegacyOrganizationLibraryErrorV1>
    where
        P: LegacyOrganizationLibraryAtomicPortV1,
    {
        let fence = proof.map(LegacyOrganizationLibraryBrowserFenceV1::from_validated_proof);
        self.execute_with_fence(port, request, fence.as_ref()).await
    }

    async fn execute_with_fence<P>(
        &self,
        port: &P,
        request: &LegacyOrganizationLibraryRequestV1,
        fence: Option<&LegacyOrganizationLibraryBrowserFenceV1>,
    ) -> Result<LegacyOrganizationLibraryExecutionV1, LegacyOrganizationLibraryErrorV1>
    where
        P: LegacyOrganizationLibraryAtomicPortV1,
    {
        if self.action.requires_session() {
            let Some(fence) = fence else {
                return Err(LegacyOrganizationLibraryErrorV1::Unauthorized);
            };
            if request.actor_id != Some(fence.actor_id()) {
                return Err(LegacyOrganizationLibraryErrorV1::Unauthorized);
            }
        } else if fence.is_some() {
            return Err(LegacyOrganizationLibraryErrorV1::Invalid);
        }
        let command = self.prepare(request)?;
        let (receipt, replayed) = match port.execute_atomic(&command, fence).await {
            Ok(LegacyOrganizationLibraryAtomicOutcomeV1::Applied(receipt)) => (receipt, false),
            Ok(LegacyOrganizationLibraryAtomicOutcomeV1::Replay(receipt)) => (receipt, true),
            Err(LegacyOrganizationLibraryAtomicErrorV1::NotFound)
            | Err(LegacyOrganizationLibraryAtomicErrorV1::AccessDenied) => {
                return Err(LegacyOrganizationLibraryErrorV1::NotFound);
            }
            Err(LegacyOrganizationLibraryAtomicErrorV1::Conflict)
            | Err(LegacyOrganizationLibraryAtomicErrorV1::InFlight) => {
                return Err(LegacyOrganizationLibraryErrorV1::Conflict);
            }
            Err(LegacyOrganizationLibraryAtomicErrorV1::Unavailable) => {
                return Err(LegacyOrganizationLibraryErrorV1::Unavailable);
            }
            Err(LegacyOrganizationLibraryAtomicErrorV1::Corrupt) => {
                return Err(LegacyOrganizationLibraryErrorV1::Internal);
            }
        };
        if !receipt.matches(&command) {
            return Err(LegacyOrganizationLibraryErrorV1::Internal);
        }
        Ok(LegacyOrganizationLibraryExecutionV1 {
            result: receipt.result,
            effects: receipt.effects,
            replayed,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyOrganizationLibraryExecutionV1 {
    result: LegacyOrganizationLibraryResultV1,
    effects: LegacyOrganizationLibraryEffectsV1,
    replayed: bool,
}

impl LegacyOrganizationLibraryExecutionV1 {
    #[must_use]
    pub const fn result(&self) -> &LegacyOrganizationLibraryResultV1 {
        &self.result
    }

    #[must_use]
    pub const fn effects(&self) -> &LegacyOrganizationLibraryEffectsV1 {
        &self.effects
    }

    #[must_use]
    pub const fn replayed(&self) -> bool {
        self.replayed
    }
}

fn validate_input(
    input: &LegacyOrganizationLibraryInputV1,
    active_organization_id: Option<OrganizationId>,
) -> Result<(), LegacyOrganizationLibraryErrorV1> {
    use LegacyOrganizationLibraryInputV1 as Input;
    match input {
        Input::SetCollectionLogo {
            legacy_collection_id,
            remove,
            image,
            ..
        } => {
            require_cap_id(legacy_collection_id)?;
            if *remove == image.is_some() {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            if let Some(image) = image {
                validate_image(
                    image,
                    1024 * 1024,
                    &["image/png", "image/jpeg", "image/svg+xml", "image/webp"],
                )?;
            }
        }
        Input::VerifyCollectionPassword {
            legacy_collection_id,
            password,
        } => {
            require_cap_id(legacy_collection_id)?;
            if password.is_empty()
                || password.len() > 4096
                || password.chars().any(char::is_control)
            {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
        }
        Input::SetSpaceCollectionVisibility {
            legacy_space_id,
            public,
            settings_patch,
        } => {
            require_cap_id(legacy_space_id)?;
            if public.is_none() && settings_patch.is_none() {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            if let Some(settings) = settings_patch {
                validate_public_page_settings_patch(settings)?;
            }
        }
        Input::DeleteSpace { legacy_space_id }
        | Input::UploadSpaceIcon {
            legacy_space_id, ..
        } => {
            require_cap_id(legacy_space_id)?;
        }
        Input::SelectShareableLinkBrandingOrganization {
            legacy_organization_id,
        } => {
            require_cap_id(legacy_organization_id)?;
        }
        Input::GetOrganizationSsoData {
            legacy_organization_id,
        }
        | Input::HideShareableLinkCapLogo {
            legacy_organization_id,
        }
        | Input::RemoveShareableLinkIcon {
            legacy_organization_id,
        }
        | Input::UpdateShareableLinkIconPreference {
            legacy_organization_id,
            ..
        }
        | Input::UploadShareableLinkIcon {
            legacy_organization_id,
            ..
        }
        | Input::ConnectOrganizationGoogleDrive {
            legacy_organization_id,
        }
        | Input::DisconnectOrganizationGoogleDrive {
            legacy_organization_id,
        }
        | Input::GetOrganizationStorageSettings {
            legacy_organization_id,
        }
        | Input::SetOrganizationStorageProvider {
            legacy_organization_id,
            ..
        } => require_active_organization(legacy_organization_id, active_organization_id)?,
        Input::RemoveOrganizationMember {
            legacy_member_id,
            legacy_organization_id,
        }
        | Input::ToggleProSeat {
            legacy_member_id,
            legacy_organization_id,
            ..
        }
        | Input::UpdateOrganizationMemberRole {
            legacy_member_id,
            legacy_organization_id,
            ..
        } => {
            require_cap_id(legacy_member_id)?;
            require_active_organization(legacy_organization_id, active_organization_id)?;
        }
        Input::UpdateOrganizationSettings { settings } => {
            validate_organization_settings(settings)?;
        }
        Input::UpdateOrganizationDetails {
            legacy_organization_id,
            organization_name,
            allowed_email_domain,
        } => {
            require_active_organization(legacy_organization_id, active_organization_id)?;
            if organization_name.is_none() && allowed_email_domain.is_none() {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            if organization_name.as_ref().is_some_and(|name| {
                name.is_empty() || name.len() > 255 || name.chars().any(char::is_control)
            }) || allowed_email_domain
                .as_ref()
                .is_some_and(|domain| domain.len() > 255 || domain.chars().any(char::is_control))
            {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
        }
        Input::CreateOrganization { name, icon } => {
            if name.is_empty() || name.len() > 255 || name.chars().any(char::is_control) {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            if let Some(icon) = icon {
                validate_image(icon, 2 * 1024 * 1024, &[])?;
            }
        }
    }
    match input {
        Input::UploadShareableLinkIcon { image, .. } => {
            validate_image(image, 1024 * 1024, &["image/png", "image/jpeg"])?;
        }
        Input::UploadSpaceIcon { image, .. } => {
            validate_image(image, 1024 * 1024, &[])?;
        }
        Input::UpdateOrganizationMemberRole { role, .. }
            if !role.eq_ignore_ascii_case("admin") && !role.eq_ignore_ascii_case("member") =>
        {
            return Err(LegacyOrganizationLibraryErrorV1::Invalid);
        }
        _ => {}
    }
    Ok(())
}

fn validate_organization_settings(
    value: &serde_json::Value,
) -> Result<(), LegacyOrganizationLibraryErrorV1> {
    validate_json_object(value, 64 * 1024)?;
    const ALLOWED: &[&str] = &[
        "disableSummary",
        "disableCaptions",
        "disableChapters",
        "disableReactions",
        "disableTranscript",
        "disableComments",
        "hideShareableLinkCapLogo",
        "shareableLinkUseOrganizationIcon",
        "aiGenerationLanguage",
        "defaultPlaybackSpeed",
    ];
    let object = value
        .as_object()
        .ok_or(LegacyOrganizationLibraryErrorV1::Invalid)?;
    if object.is_empty() || object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(LegacyOrganizationLibraryErrorV1::Invalid);
    }
    for (key, value) in object {
        match key.as_str() {
            "aiGenerationLanguage"
                if value
                    .as_str()
                    .is_none_or(|language| !valid_ai_generation_language(language)) =>
            {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            "defaultPlaybackSpeed"
                if value
                    .as_f64()
                    .is_none_or(|speed| !speed.is_finite() || speed <= 0.0) =>
            {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            "aiGenerationLanguage" | "defaultPlaybackSpeed" => {}
            _ if !value.is_boolean() => {
                return Err(LegacyOrganizationLibraryErrorV1::Invalid);
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_public_page_settings_patch(
    value: &serde_json::Value,
) -> Result<(), LegacyOrganizationLibraryErrorV1> {
    validate_json_object(value, 64 * 1024)?;
    let object = value
        .as_object()
        .ok_or(LegacyOrganizationLibraryErrorV1::Invalid)?;
    const ALLOWED: &[&str] = &[
        "hideTitle",
        "hideCopyLink",
        "logoMode",
        "title",
        "subtitle",
        "ctaLabel",
        "ctaUrl",
        "layout",
        "gridColumns",
    ];
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(LegacyOrganizationLibraryErrorV1::Invalid);
    }
    for (key, value) in object {
        let valid = match key.as_str() {
            "hideTitle" | "hideCopyLink" => value.is_boolean(),
            "logoMode" => value
                .as_str()
                .is_some_and(|value| matches!(value, "cap" | "organization" | "custom" | "none")),
            "title" => bounded_utf16_string(value, 80),
            "subtitle" => bounded_utf16_string(value, 160),
            "ctaLabel" => bounded_utf16_string(value, 40),
            "ctaUrl" => bounded_utf16_string(value, 512),
            "layout" => value
                .as_str()
                .is_some_and(|value| matches!(value, "grid" | "list")),
            "gridColumns" => value.as_u64().is_some_and(|value| matches!(value, 2..=5)),
            _ => false,
        };
        if !valid {
            return Err(LegacyOrganizationLibraryErrorV1::Invalid);
        }
    }
    Ok(())
}

fn bounded_utf16_string(value: &serde_json::Value, max_units: usize) -> bool {
    value
        .as_str()
        .is_some_and(|value| value.encode_utf16().count() <= max_units)
}

fn valid_ai_generation_language(value: &str) -> bool {
    matches!(
        value,
        "auto"
            | "en"
            | "es"
            | "fr"
            | "de"
            | "pt"
            | "it"
            | "nl"
            | "pl"
            | "ro"
            | "sk"
            | "ru"
            | "tr"
            | "ja"
            | "ko"
            | "zh"
            | "ar"
            | "hi"
            | "bn"
            | "ta"
            | "te"
            | "mr"
            | "gu"
            | "ur"
            | "fa"
            | "he"
    )
}

fn validate_json_object(
    value: &serde_json::Value,
    max_bytes: usize,
) -> Result<(), LegacyOrganizationLibraryErrorV1> {
    if !value.is_object()
        || serde_json::to_vec(value)
            .map_err(|_| LegacyOrganizationLibraryErrorV1::Invalid)?
            .len()
            > max_bytes
    {
        return Err(LegacyOrganizationLibraryErrorV1::Invalid);
    }
    Ok(())
}

fn validate_image(
    image: &LegacyImagePayloadV1,
    max_bytes: usize,
    allowed_types: &[&str],
) -> Result<(), LegacyOrganizationLibraryErrorV1> {
    if image.bytes.is_empty()
        || image.bytes.len() > max_bytes
        || image.file_name.is_empty()
        || image.file_name.len() > 255
        || image.file_name.contains(['/', '\\'])
        || image.file_name.chars().any(char::is_control)
        || !image.content_type.starts_with("image/")
        || (!allowed_types.is_empty() && !allowed_types.contains(&image.content_type.as_str()))
        || image.checksum_sha256.len() != 64
        || !image
            .checksum_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(LegacyOrganizationLibraryErrorV1::Invalid);
    }
    let actual = Sha256::digest(&image.bytes);
    if hex_digest(&actual) != image.checksum_sha256 {
        return Err(LegacyOrganizationLibraryErrorV1::Invalid);
    }
    Ok(())
}

fn require_active_organization(
    legacy_id: &str,
    active: Option<OrganizationId>,
) -> Result<(), LegacyOrganizationLibraryErrorV1> {
    let mapped = require_cap_id(legacy_id)?;
    if active.map(|id| id.as_uuid()) != Some(mapped.mapped_uuid()) {
        return Err(LegacyOrganizationLibraryErrorV1::NotFound);
    }
    Ok(())
}

fn require_cap_id(value: &str) -> Result<LegacyCapNanoId, LegacyOrganizationLibraryErrorV1> {
    LegacyCapNanoId::parse(value.to_owned()).map_err(|_| LegacyOrganizationLibraryErrorV1::NotFound)
}

fn fingerprint(
    action: LegacyOrganizationLibraryActionV1,
    actor_id: Option<UserId>,
    active_organization_id: Option<OrganizationId>,
    idempotency_key: Option<&IdempotencyKey>,
    input: &LegacyOrganizationLibraryInputV1,
) -> Result<[u8; 32], LegacyOrganizationLibraryErrorV1> {
    let mut digest = Sha256::new();
    digest.update(b"frame.legacy-organization-library-command.v1\0");
    digest.update(action.operation_id().as_bytes());
    digest.update(b"\0");
    if let Some(actor_id) = actor_id {
        digest.update(actor_id.as_uuid().as_bytes());
    }
    if let Some(organization_id) = active_organization_id {
        digest.update(organization_id.as_uuid().as_bytes());
    }
    if let Some(key) = idempotency_key {
        digest.update(key.expose().as_bytes());
    }
    digest
        .update(serde_json::to_vec(input).map_err(|_| LegacyOrganizationLibraryErrorV1::Invalid)?);
    Ok(digest.finalize().into())
}

fn result_matches_action(
    result: &LegacyOrganizationLibraryResultV1,
    action: LegacyOrganizationLibraryActionV1,
) -> bool {
    match result {
        LegacyOrganizationLibraryResultV1::PasswordVerified { .. } => {
            action == LegacyOrganizationLibraryActionV1::VerifyCollectionPassword
        }
        LegacyOrganizationLibraryResultV1::PasswordRejected => {
            action == LegacyOrganizationLibraryActionV1::VerifyCollectionPassword
        }
        LegacyOrganizationLibraryResultV1::OrganizationSsoData { .. } => {
            action == LegacyOrganizationLibraryActionV1::GetOrganizationSsoData
        }
        LegacyOrganizationLibraryResultV1::GoogleDriveAuthorization { .. } => {
            action == LegacyOrganizationLibraryActionV1::ConnectOrganizationGoogleDrive
        }
        LegacyOrganizationLibraryResultV1::OrganizationStorageSettings { .. } => {
            action == LegacyOrganizationLibraryActionV1::GetOrganizationStorageSettings
        }
        LegacyOrganizationLibraryResultV1::OrganizationCreated { .. } => {
            action == LegacyOrganizationLibraryActionV1::CreateOrganization
        }
        LegacyOrganizationLibraryResultV1::IconUploaded { object_key } => {
            action == LegacyOrganizationLibraryActionV1::UploadSpaceIcon
                && valid_object_key(object_key)
        }
        LegacyOrganizationLibraryResultV1::Success => !matches!(
            action,
            LegacyOrganizationLibraryActionV1::VerifyCollectionPassword
                | LegacyOrganizationLibraryActionV1::GetOrganizationSsoData
                | LegacyOrganizationLibraryActionV1::ConnectOrganizationGoogleDrive
                | LegacyOrganizationLibraryActionV1::GetOrganizationStorageSettings
                | LegacyOrganizationLibraryActionV1::CreateOrganization
                | LegacyOrganizationLibraryActionV1::UploadSpaceIcon
        ),
    }
}

fn valid_object_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
}

fn legacy_id_from_digest(digest: &[u8; 32]) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    digest
        .iter()
        .take(15)
        .map(|byte| char::from(ALPHABET[usize::from(*byte & 31)]))
        .collect()
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoPort;

    #[async_trait]
    impl LegacyOrganizationLibraryAtomicPortV1 for EchoPort {
        async fn execute_atomic(
            &self,
            command: &LegacyOrganizationLibraryCommandV1,
            browser_fence: Option<&LegacyOrganizationLibraryBrowserFenceV1>,
        ) -> Result<LegacyOrganizationLibraryAtomicOutcomeV1, LegacyOrganizationLibraryAtomicErrorV1>
        {
            assert_eq!(command.action.requires_session(), browser_fence.is_some());
            let result = match command.action {
                LegacyOrganizationLibraryActionV1::VerifyCollectionPassword => {
                    LegacyOrganizationLibraryResultV1::PasswordVerified {
                        password_hash: "hash".into(),
                    }
                }
                LegacyOrganizationLibraryActionV1::CreateOrganization => {
                    LegacyOrganizationLibraryResultV1::OrganizationCreated {
                        legacy_organization_id: command
                            .deterministic_legacy_organization_id()
                            .expect("generated id"),
                    }
                }
                _ => LegacyOrganizationLibraryResultV1::Success,
            };
            Ok(LegacyOrganizationLibraryAtomicOutcomeV1::Applied(
                LegacyOrganizationLibraryReceiptV1 {
                    action: command.action,
                    request_digest: command.request_digest,
                    result,
                    effects: LegacyOrganizationLibraryEffectsV1 {
                        invalidation_paths: vec!["/dashboard".into()],
                        set_verified_password_cookie: false,
                        r2_keys_written: vec![],
                        r2_keys_deleted: vec![],
                    },
                },
            ))
        }
    }

    fn user() -> UserId {
        UserId::parse("11111111-1111-4111-8111-111111111111").expect("user")
    }

    fn organization() -> OrganizationId {
        let legacy = LegacyCapNanoId::parse("000000000000000").expect("legacy id");
        OrganizationId::parse(&legacy.mapped_uuid().to_string()).expect("organization")
    }

    fn session_request(
        input: LegacyOrganizationLibraryInputV1,
    ) -> LegacyOrganizationLibraryRequestV1 {
        LegacyOrganizationLibraryRequestV1 {
            credential: Some(LegacyOrganizationLibraryCredentialV1::Session),
            actor_id: Some(user()),
            active_organization_id: Some(organization()),
            idempotency_key: Some("organization-library-key".into()),
            input,
        }
    }

    #[test]
    fn profile_inventory_is_exact_unique_and_provider_free() {
        let actions = LegacyOrganizationLibraryActionV1::all();
        assert_eq!(actions.len(), LEGACY_ORGANIZATION_LIBRARY_OPERATION_COUNT);
        let ids = actions
            .iter()
            .map(|action| action.operation_id())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), actions.len());
        for action in actions {
            let profile = legacy_organization_library_profile(action);
            assert_eq!(profile.operation_id, action.operation_id());
            assert_eq!(profile.legacy_identity, action.legacy_identity());
            assert_eq!(profile.protected_gates, &[] as &[&str]);
            assert!(profile.production_promoted);
            assert_eq!(profile.uses_r2, action.uses_r2());
            assert_eq!(profile.source_sha256.len(), 64);
        }
    }

    #[test]
    fn public_password_contract_rejects_session_and_idempotency() {
        let adapter = LegacyOrganizationLibraryAdapterV1::new(
            LegacyOrganizationLibraryActionV1::VerifyCollectionPassword,
        );
        let input = LegacyOrganizationLibraryInputV1::VerifyCollectionPassword {
            legacy_collection_id: "000000000000000".into(),
            password: "secret".into(),
        };
        let mut request = session_request(input.clone());
        assert_eq!(
            adapter.prepare(&request),
            Err(LegacyOrganizationLibraryErrorV1::Invalid)
        );
        request.credential = Some(LegacyOrganizationLibraryCredentialV1::Public);
        request.actor_id = None;
        request.active_organization_id = None;
        request.idempotency_key = None;
        assert!(adapter.prepare(&request).is_ok());
    }

    #[tokio::test]
    async fn session_actions_bind_actor_tenant_key_and_one_use_proof() {
        let adapter = LegacyOrganizationLibraryAdapterV1::new(
            LegacyOrganizationLibraryActionV1::HideShareableLinkCapLogo,
        );
        let request = session_request(LegacyOrganizationLibraryInputV1::HideShareableLinkCapLogo {
            legacy_organization_id: "000000000000000".into(),
        });
        assert_eq!(
            adapter.execute_with_fence(&EchoPort, &request, None).await,
            Err(LegacyOrganizationLibraryErrorV1::Unauthorized)
        );
        let fence = LegacyOrganizationLibraryBrowserFenceV1::fixture(user());
        let execution = adapter
            .execute_with_fence(&EchoPort, &request, Some(&fence))
            .await
            .expect("execution");
        assert!(!execution.replayed());
        assert_eq!(
            execution.result(),
            &LegacyOrganizationLibraryResultV1::Success
        );
    }

    #[test]
    fn cross_tenant_and_tampered_images_fail_before_the_port() {
        let adapter = LegacyOrganizationLibraryAdapterV1::new(
            LegacyOrganizationLibraryActionV1::UploadShareableLinkIcon,
        );
        let request = session_request(LegacyOrganizationLibraryInputV1::UploadShareableLinkIcon {
            legacy_organization_id: "111111111111111".into(),
            image: LegacyImagePayloadV1 {
                file_name: "icon.png".into(),
                content_type: "image/png".into(),
                bytes: vec![1, 2, 3],
                checksum_sha256: "0".repeat(64),
            },
        });
        assert_eq!(
            adapter.prepare(&request),
            Err(LegacyOrganizationLibraryErrorV1::NotFound)
        );
    }

    #[test]
    fn target_branding_selection_does_not_require_a_current_tenant() {
        let adapter = LegacyOrganizationLibraryAdapterV1::new(
            LegacyOrganizationLibraryActionV1::SelectShareableLinkBrandingOrganization,
        );
        let mut request = session_request(
            LegacyOrganizationLibraryInputV1::SelectShareableLinkBrandingOrganization {
                legacy_organization_id: "111111111111111".into(),
            },
        );
        request.active_organization_id = None;
        assert!(adapter.prepare(&request).is_ok());
    }

    #[test]
    fn public_page_patch_rejects_upload_owned_and_malformed_fields() {
        let adapter = LegacyOrganizationLibraryAdapterV1::new(
            LegacyOrganizationLibraryActionV1::SetSpaceCollectionVisibility,
        );
        for patch in [
            serde_json::json!({"logoUrl": "organizations/forbidden.png"}),
            serde_json::json!({"gridColumns": 6}),
            serde_json::json!({"title": "x".repeat(81)}),
            serde_json::json!({"hideTitle": "yes"}),
        ] {
            let request = session_request(
                LegacyOrganizationLibraryInputV1::SetSpaceCollectionVisibility {
                    legacy_space_id: "000000000000000".into(),
                    public: None,
                    settings_patch: Some(patch),
                },
            );
            assert_eq!(
                adapter.prepare(&request),
                Err(LegacyOrganizationLibraryErrorV1::Invalid)
            );
        }
    }

    #[test]
    fn member_role_validation_matches_the_source_assignable_roles() {
        let adapter = LegacyOrganizationLibraryAdapterV1::new(
            LegacyOrganizationLibraryActionV1::UpdateOrganizationMemberRole,
        );
        for role in ["admin", "member", "ADMIN"] {
            let request = session_request(
                LegacyOrganizationLibraryInputV1::UpdateOrganizationMemberRole {
                    legacy_member_id: "111111111111111".into(),
                    legacy_organization_id: "000000000000000".into(),
                    role: role.into(),
                },
            );
            assert!(adapter.prepare(&request).is_ok(), "role {role}");
        }
        for role in ["owner", "viewer", ""] {
            let request = session_request(
                LegacyOrganizationLibraryInputV1::UpdateOrganizationMemberRole {
                    legacy_member_id: "111111111111111".into(),
                    legacy_organization_id: "000000000000000".into(),
                    role: role.into(),
                },
            );
            assert_eq!(
                adapter.prepare(&request),
                Err(LegacyOrganizationLibraryErrorV1::Invalid),
                "role {role}"
            );
        }
    }
}
