//! Source-pinned contracts for Cap's Effect-RPC carrier and provider-free video lifecycle.
//!
//! The RPC transport is shared by several compatibility families.  This module
//! owns only the carrier rows plus organization-icon update, video delete,
//! video duplicate, instant recording creation, and the public OG image route.
//! R2 is a first-party storage effect for these operations; none of the rows in
//! this family may be promoted by pretending that a third-party provider ran.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const LEGACY_VIDEO_LIFECYCLE_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";

pub const LEGACY_ERPC_GET_OPERATION_ID: &str = "cap-v1-6cee4f8c7f91f9fd";
pub const LEGACY_ERPC_POST_OPERATION_ID: &str = "cap-v1-5d669b34ea762549";
pub const LEGACY_VIDEO_DELETE_ROUTE_OPERATION_ID: &str = "cap-v1-ac0d7aa564f2991c";
pub const LEGACY_VIDEO_OG_OPERATION_ID: &str = "cap-v1-7d6fa824a5356ace";
pub const LEGACY_ORGANISATION_UPDATE_OPERATION_ID: &str = "cap-v1-e32af2138aa62c8d";
pub const LEGACY_VIDEO_DELETE_RPC_OPERATION_ID: &str = "cap-v1-1e909cc023a9c4a7";
pub const LEGACY_VIDEO_DUPLICATE_OPERATION_ID: &str = "cap-v1-e6a882aeeffaa4f6";
pub const LEGACY_VIDEO_INSTANT_CREATE_OPERATION_ID: &str = "cap-v1-7b4e8210491e549d";

pub const LEGACY_ERPC_IDENTITY: &str = "/api/erpc";
pub const LEGACY_VIDEO_DELETE_ROUTE_IDENTITY: &str = "/api/video/delete";
pub const LEGACY_VIDEO_OG_IDENTITY: &str = "/api/video/og";
pub const LEGACY_ORGANISATION_UPDATE_IDENTITY: &str = "/api/erpc#OrganisationUpdate";
pub const LEGACY_VIDEO_DELETE_RPC_IDENTITY: &str = "/api/erpc#VideoDelete";
pub const LEGACY_VIDEO_DUPLICATE_IDENTITY: &str = "/api/erpc#VideoDuplicate";
pub const LEGACY_VIDEO_INSTANT_CREATE_IDENTITY: &str = "/api/erpc#VideoInstantCreate";

pub const LEGACY_VIDEO_LIFECYCLE_MAX_BODY_BYTES: usize = 256 * 1024;
pub const LEGACY_VIDEO_LIFECYCLE_CONTENT_TYPE: &str = "application/json";
pub const LEGACY_VIDEO_LIFECYCLE_NO_PROTECTED_GATES: &[&str] = &[];
pub const LEGACY_VIDEO_LIFECYCLE_RPC_SCHEMA: &str = "effect-rpc-json.v1";
pub const LEGACY_VIDEO_LIFECYCLE_MAX_IMAGE_BYTES: usize = 256 * 1024;
pub const LEGACY_VIDEO_LIFECYCLE_MAX_R2_PAGES: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyVideoLifecycleSurfaceV1 {
    ErpcGet,
    ErpcPost,
    DeleteRoute,
    OgImage,
    OrganisationUpdate,
    VideoDelete,
    VideoDuplicate,
    VideoInstantCreate,
}

impl LegacyVideoLifecycleSurfaceV1 {
    #[must_use]
    pub const fn operation_id(self) -> &'static str {
        match self {
            Self::ErpcGet => LEGACY_ERPC_GET_OPERATION_ID,
            Self::ErpcPost => LEGACY_ERPC_POST_OPERATION_ID,
            Self::DeleteRoute => LEGACY_VIDEO_DELETE_ROUTE_OPERATION_ID,
            Self::OgImage => LEGACY_VIDEO_OG_OPERATION_ID,
            Self::OrganisationUpdate => LEGACY_ORGANISATION_UPDATE_OPERATION_ID,
            Self::VideoDelete => LEGACY_VIDEO_DELETE_RPC_OPERATION_ID,
            Self::VideoDuplicate => LEGACY_VIDEO_DUPLICATE_OPERATION_ID,
            Self::VideoInstantCreate => LEGACY_VIDEO_INSTANT_CREATE_OPERATION_ID,
        }
    }

    #[must_use]
    pub const fn identity(self) -> &'static str {
        match self {
            Self::ErpcGet | Self::ErpcPost => LEGACY_ERPC_IDENTITY,
            Self::DeleteRoute => LEGACY_VIDEO_DELETE_ROUTE_IDENTITY,
            Self::OgImage => LEGACY_VIDEO_OG_IDENTITY,
            Self::OrganisationUpdate => LEGACY_ORGANISATION_UPDATE_IDENTITY,
            Self::VideoDelete => LEGACY_VIDEO_DELETE_RPC_IDENTITY,
            Self::VideoDuplicate => LEGACY_VIDEO_DUPLICATE_IDENTITY,
            Self::VideoInstantCreate => LEGACY_VIDEO_INSTANT_CREATE_IDENTITY,
        }
    }

    #[must_use]
    pub const fn method(self) -> &'static str {
        match self {
            Self::ErpcGet | Self::OgImage => "GET",
            Self::ErpcPost => "POST",
            Self::DeleteRoute => "DELETE",
            Self::OrganisationUpdate
            | Self::VideoDelete
            | Self::VideoDuplicate
            | Self::VideoInstantCreate => "RPC",
        }
    }

    #[must_use]
    pub const fn policy(self) -> &'static str {
        match self {
            Self::ErpcGet | Self::ErpcPost => "service_misc.v1",
            Self::OrganisationUpdate => "organization_library.v1",
            Self::DeleteRoute
            | Self::OgImage
            | Self::VideoDelete
            | Self::VideoDuplicate
            | Self::VideoInstantCreate => "video_media.v1",
        }
    }

    #[must_use]
    pub fn parse(operation_id: &str) -> Option<Self> {
        LEGACY_VIDEO_LIFECYCLE_PROFILES
            .iter()
            .find(|profile| profile.surface.operation_id() == operation_id)
            .map(|profile| profile.surface)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoLifecycleSourceRoleV1 {
    Handler,
    RpcTransport,
    RpcDefinition,
    Service,
    Authorization,
    Authentication,
    Persistence,
    Storage,
    ImageRenderer,
    ApiContract,
}

impl LegacyVideoLifecycleSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::RpcTransport => "rpc_transport",
            Self::RpcDefinition => "rpc_definition",
            Self::Service => "service",
            Self::Authorization => "authorization",
            Self::Authentication => "authentication",
            Self::Persistence => "persistence",
            Self::Storage => "storage",
            Self::ImageRenderer => "image_renderer",
            Self::ApiContract => "api_contract",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyVideoLifecycleSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyVideoLifecycleSourceRoleV1,
}

const ERPC_ROUTE: LegacyVideoLifecycleSourcePinV1 = LegacyVideoLifecycleSourcePinV1 {
    path: "apps/web/app/api/erpc/route.ts",
    symbol: "GET+POST Effect RPC HTTP transport",
    sha256: "01a2dee0518e44fe6137513f117100e6a626b904e4ee4608fc0be6d69e210783",
    role: LegacyVideoLifecycleSourceRoleV1::RpcTransport,
};
const RPCS: LegacyVideoLifecycleSourcePinV1 = LegacyVideoLifecycleSourcePinV1 {
    path: "packages/web-backend/src/Rpcs.ts",
    symbol: "RpcsLive+RpcAuthMiddlewareLive",
    sha256: "cfb2cbee41a0abef4496fa2eb42c43688310cc13590e77c1425dc7f919304f19",
    role: LegacyVideoLifecycleSourceRoleV1::Authentication,
};
const VIDEO_RPCS: LegacyVideoLifecycleSourcePinV1 = LegacyVideoLifecycleSourcePinV1 {
    path: "packages/web-backend/src/Videos/VideosRpcs.ts",
    symbol: "VideoDelete+VideoDuplicate+VideoInstantCreate",
    sha256: "6edf9add90a28c542fb53c9a7bfa858bc89290e2a0fbeec827210bd5af189623",
    role: LegacyVideoLifecycleSourceRoleV1::RpcDefinition,
};
const VIDEOS: LegacyVideoLifecycleSourcePinV1 = LegacyVideoLifecycleSourcePinV1 {
    path: "packages/web-backend/src/Videos/index.ts",
    symbol: "Videos.delete+Videos.duplicate+Videos.createInstantRecording",
    sha256: "43b523a47ed667f70f7f10dde8677740d663811c61f1af278441929184963849",
    role: LegacyVideoLifecycleSourceRoleV1::Service,
};
const VIDEO_DOMAIN: LegacyVideoLifecycleSourcePinV1 = LegacyVideoLifecycleSourcePinV1 {
    path: "packages/web-domain/src/Video.ts",
    symbol: "VideoRpcs+InstantRecordingCreateInput",
    sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
    role: LegacyVideoLifecycleSourceRoleV1::RpcDefinition,
};

pub const LEGACY_ERPC_SOURCES: &[LegacyVideoLifecycleSourcePinV1] = &[ERPC_ROUTE, RPCS];

pub const LEGACY_VIDEO_DELETE_ROUTE_SOURCES: &[LegacyVideoLifecycleSourcePinV1] = &[
    LegacyVideoLifecycleSourcePinV1 {
        path: "apps/web/app/api/video/delete/route.ts",
        symbol: "DELETE",
        sha256: "8bf8a91a8f5ec657e199fec9df28638759ae78d6e9021977dff0e608b008a5cc",
        role: LegacyVideoLifecycleSourceRoleV1::Handler,
    },
    LegacyVideoLifecycleSourcePinV1 {
        path: "packages/web-api-contract-effect/src/index.ts",
        symbol: "delete",
        sha256: "9c2185ebf12be4c9d231d42938c975ea6ad596a0031ed8a0aca2bb1cbec3c7a0",
        role: LegacyVideoLifecycleSourceRoleV1::ApiContract,
    },
    LegacyVideoLifecycleSourcePinV1 {
        path: "packages/web-api-contract/src/index.ts",
        symbol: "DELETE /video/delete",
        sha256: "98bb2529e27eba0ed1569d286a1f5d4069cbbf23cf9e1dde62fdc1f6a9737e3e",
        role: LegacyVideoLifecycleSourceRoleV1::ApiContract,
    },
    VIDEOS,
];

pub const LEGACY_VIDEO_OG_SOURCES: &[LegacyVideoLifecycleSourcePinV1] = &[
    LegacyVideoLifecycleSourcePinV1 {
        path: "apps/web/app/api/video/og/route.tsx",
        symbol: "GET",
        sha256: "675da4bf50b4952801275b179d1b2cc833a3a6cb9d717bd468f4f67026c20758",
        role: LegacyVideoLifecycleSourceRoleV1::Handler,
    },
    LegacyVideoLifecycleSourcePinV1 {
        path: "apps/web/actions/videos/get-og-image.tsx",
        symbol: "generateVideoOgImage+getData",
        sha256: "5ac48c887030592bfeb439f8a900306e6249aad47a170992eda7b27914eed958",
        role: LegacyVideoLifecycleSourceRoleV1::ImageRenderer,
    },
];

pub const LEGACY_ORGANISATION_UPDATE_SOURCES: &[LegacyVideoLifecycleSourcePinV1] = &[
    ERPC_ROUTE,
    LegacyVideoLifecycleSourcePinV1 {
        path: "packages/web-backend/src/ImageUploads/index.ts",
        symbol: "ImageUploads.applyUpdate",
        sha256: "1dc0952ae84d76844128d0fc5cdf2eb63519c26183f932c035638ff0d6463d1c",
        role: LegacyVideoLifecycleSourceRoleV1::Storage,
    },
    LegacyVideoLifecycleSourcePinV1 {
        path: "packages/web-backend/src/Organisations/OrganisationsPolicy.ts",
        symbol: "OrganisationsPolicy.isAdminOrOwner",
        sha256: "a003866c7dd649252bd705f5530b2ac0a2f23387a5a8c433069a0dd9cf532736",
        role: LegacyVideoLifecycleSourceRoleV1::Authorization,
    },
    LegacyVideoLifecycleSourcePinV1 {
        path: "packages/web-backend/src/Organisations/OrganisationsRpcs.ts",
        symbol: "OrganisationUpdate",
        sha256: "b87253931de5e7401fa25e392dbdd111417a207d88aeba52b493e058807243d5",
        role: LegacyVideoLifecycleSourceRoleV1::RpcDefinition,
    },
    LegacyVideoLifecycleSourcePinV1 {
        path: "packages/web-backend/src/Organisations/index.ts",
        symbol: "Organisations.update",
        sha256: "ea3364361f49922d7682697789943718b9f1fdd0f30ca02d11dea08b8f941ce4",
        role: LegacyVideoLifecycleSourceRoleV1::Service,
    },
    RPCS,
    LegacyVideoLifecycleSourcePinV1 {
        path: "packages/web-domain/src/Organisation.ts",
        symbol: "OrganisationUpdate",
        sha256: "14d634ad8910d3921af2ea5b136b9c3d2a8ae26f74b3dcb7a82b9cf19d6a3264",
        role: LegacyVideoLifecycleSourceRoleV1::RpcDefinition,
    },
];

pub const LEGACY_VIDEO_RPC_SOURCES: &[LegacyVideoLifecycleSourcePinV1] =
    &[ERPC_ROUTE, RPCS, VIDEO_RPCS, VIDEOS, VIDEO_DOMAIN];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyVideoLifecycleProfileV1 {
    pub surface: LegacyVideoLifecycleSurfaceV1,
    pub auth: &'static str,
    pub idempotency: &'static str,
    pub success: &'static str,
    pub validation: &'static str,
    pub authorization: &'static str,
    pub idempotency_retry: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_VIDEO_LIFECYCLE_PROFILES: &[LegacyVideoLifecycleProfileV1] = &[
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::ErpcGet,
        auth: "transport_no_effect_without_authenticated_rpc_request",
        idempotency: "forbidden",
        success: "effect_rpc_transport_response",
        validation: "exact_path_and_get_transport_protocol",
        authorization: "individual_rpc_middleware_owns_session_policy",
        idempotency_retry: "read_only_transport_probe",
        failure: "stable_effect_rpc_defect_without_secret_or_tenant_disclosure",
    },
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::ErpcPost,
        auth: "host_only_session_per_authenticated_rpc",
        idempotency: "effect_rpc_request_id_replay_key",
        success: "effect_rpc_json_exit_array",
        validation: "bounded_single_request_effect_rpc_json",
        authorization: "authenticated_rpc_middleware_then_family_policy",
        idempotency_retry: "request_id_digest_replays_durable_mutation_result",
        failure: "typed_fail_die_or_defect_exit",
    },
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::DeleteRoute,
        auth: "session_owner",
        idempotency: "derived_actor_video_cleanup_key",
        success: "empty_success_after_d1_tombstone_and_r2_prefix_delete",
        validation: "video_id_query_only_and_empty_body",
        authorization: "owner_only_with_cross_tenant_non_disclosure",
        idempotency_retry: "pending_cleanup_resumes_and_complete_replays",
        failure: "401_session_404_video_500_d1_or_r2",
    },
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::OgImage,
        auth: "public_video_only",
        idempotency: "forbidden",
        success: "1200x630_png_with_optional_screenshot_and_play_mark",
        validation: "single_video_id_query",
        authorization: "public_bit_or_non_disclosing_fallback_card",
        idempotency_retry: "safe_read_recomputation",
        failure: "not_found_or_private_render_stable_fallback_png",
    },
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::OrganisationUpdate,
        auth: "session_admin_or_owner",
        idempotency: "effect_rpc_request_id_replay_key",
        success: "void_after_r2_put_d1_pointer_swap_and_old_object_delete",
        validation: "effect_option_none_or_bounded_base64_image_payload",
        authorization: "active_organization_admin_or_owner",
        idempotency_retry: "operation_key_replays_exact_icon_binding",
        failure: "typed_org_not_found_policy_denied_or_internal",
    },
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::VideoDelete,
        auth: "session_owner",
        idempotency: "effect_rpc_request_id_replay_key",
        success: "void_after_d1_tombstone_and_bounded_paginated_r2_delete",
        validation: "cap_video_identifier_payload",
        authorization: "owner_only_with_cross_tenant_non_disclosure",
        idempotency_retry: "pending_cleanup_resumes_and_complete_replays",
        failure: "typed_video_not_found_policy_denied_or_internal",
    },
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::VideoDuplicate,
        auth: "session_owner",
        idempotency: "effect_rpc_request_id_replay_key",
        success: "void_after_metadata_clone_and_serial_r2_prefix_copy",
        validation: "cap_video_identifier_payload",
        authorization: "owner_only_source_video_and_tenant_storage",
        idempotency_retry: "durable_destination_binding_and_per_object_copy_receipts",
        failure: "typed_video_not_found_policy_denied_or_internal",
    },
    LegacyVideoLifecycleProfileV1 {
        surface: LegacyVideoLifecycleSurfaceV1::VideoInstantCreate,
        auth: "session_active_organization_member",
        idempotency: "effect_rpc_request_id_replay_key",
        success: "video_alias_share_url_and_presigned_r2_upload_target",
        validation: "finite_optional_media_metadata_and_live_folder",
        authorization: "requested_org_equals_active_org_with_writable_r2",
        idempotency_retry: "durable_ids_and_exact_result_replay",
        failure: "typed_policy_denied_or_internal_without_fabricated_upload",
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyOrganisationImageV1 {
    pub data: String,
    pub content_type: String,
    pub file_name: String,
}

impl LegacyOrganisationImageV1 {
    #[must_use]
    pub fn valid_metadata(&self) -> bool {
        !self.data.is_empty()
            && self.data.len() <= LEGACY_VIDEO_LIFECYCLE_MAX_IMAGE_BYTES.saturating_mul(2)
            && matches!(
                self.content_type.as_str(),
                "image/jpeg" | "image/png" | "image/webp" | "image/gif"
            )
            && !self.file_name.is_empty()
            && self.file_name.len() <= 255
            && !self.file_name.chars().any(char::is_control)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyOrganisationImagePatchV1 {
    Absent,
    Remove,
    Replace(LegacyOrganisationImageV1),
}

#[must_use]
pub fn legacy_video_lifecycle_valid_cap_id(value: &str) -> bool {
    value.len() == 15
        && value
            .bytes()
            .all(|byte| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&byte))
}

#[must_use]
pub fn legacy_video_lifecycle_object_prefix(
    owner_alias: &str,
    video_alias: &str,
) -> Option<String> {
    legacy_video_lifecycle_valid_cap_id(owner_alias)
        .then_some(())
        .filter(|()| legacy_video_lifecycle_valid_cap_id(video_alias))
        .map(|()| format!("{owner_alias}/{video_alias}/"))
}

#[must_use]
pub fn legacy_video_lifecycle_copy_key(
    source_prefix: &str,
    destination_prefix: &str,
    source_key: &str,
) -> Option<String> {
    valid_prefix(source_prefix)
        .then_some(())
        .filter(|()| valid_prefix(destination_prefix))
        .and_then(|()| source_key.strip_prefix(source_prefix))
        .filter(|suffix| !suffix.is_empty())
        .map(|suffix| format!("{destination_prefix}{suffix}"))
}

#[must_use]
pub fn legacy_organisation_icon_key(
    organization_id: &str,
    operation_id: &str,
    file_name: &str,
) -> Option<String> {
    let extension = file_name
        .rsplit('.')
        .next()
        .unwrap_or("jpg")
        .to_ascii_lowercase();
    (!organization_id.is_empty()
        && organization_id.len() <= 128
        && !organization_id.chars().any(char::is_control)
        && valid_uuid(operation_id)
        && matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif"))
    .then(|| format!("organizations/{organization_id}/{operation_id}.{extension}"))
}

#[must_use]
pub fn legacy_video_lifecycle_source_manifest(surface: LegacyVideoLifecycleSurfaceV1) -> String {
    let sources = match surface {
        LegacyVideoLifecycleSurfaceV1::ErpcGet | LegacyVideoLifecycleSurfaceV1::ErpcPost => {
            LEGACY_ERPC_SOURCES
        }
        LegacyVideoLifecycleSurfaceV1::DeleteRoute => LEGACY_VIDEO_DELETE_ROUTE_SOURCES,
        LegacyVideoLifecycleSurfaceV1::OgImage => LEGACY_VIDEO_OG_SOURCES,
        LegacyVideoLifecycleSurfaceV1::OrganisationUpdate => LEGACY_ORGANISATION_UPDATE_SOURCES,
        LegacyVideoLifecycleSurfaceV1::VideoDelete
        | LegacyVideoLifecycleSurfaceV1::VideoDuplicate
        | LegacyVideoLifecycleSurfaceV1::VideoInstantCreate => LEGACY_VIDEO_RPC_SOURCES,
    };
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-video-lifecycle-source-manifest-v1\0");
    digest.update(surface.operation_id().as_bytes());
    digest.update([0]);
    for source in sources {
        digest.update(source.path.as_bytes());
        digest.update([0]);
        digest.update(source.symbol.as_bytes());
        digest.update([0]);
        digest.update(source.sha256.as_bytes());
        digest.update([0]);
        digest.update(source.role.stable_code().as_bytes());
        digest.update(b"\n");
    }
    let mut encoded = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("write digest");
    }
    encoded
}

fn valid_prefix(value: &str) -> bool {
    value.len() >= 3
        && value.len() <= 512
        && value.ends_with('/')
        && !value.starts_with('/')
        && !value.contains("\\")
        && !value.contains("..")
        && !value.contains("//")
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_has_source_and_five_axis_closure_without_protected_gates() {
        assert_eq!(LEGACY_VIDEO_LIFECYCLE_PROFILES.len(), 8);
        assert_eq!(LEGACY_VIDEO_LIFECYCLE_NO_PROTECTED_GATES, &[] as &[&str]);
        for profile in LEGACY_VIDEO_LIFECYCLE_PROFILES {
            assert!(!profile.success.is_empty());
            assert!(!profile.validation.is_empty());
            assert!(!profile.authorization.is_empty());
            assert!(!profile.idempotency_retry.is_empty());
            assert!(!profile.failure.is_empty());
            assert_eq!(
                LegacyVideoLifecycleSurfaceV1::parse(profile.surface.operation_id()),
                Some(profile.surface)
            );
            assert_eq!(
                legacy_video_lifecycle_source_manifest(profile.surface).len(),
                64
            );
        }
    }

    #[test]
    fn copy_keys_never_escape_the_destination_prefix() {
        assert_eq!(
            legacy_video_lifecycle_copy_key(
                "owner/source/",
                "owner/destination/",
                "owner/source/segments/1.m4s"
            ),
            Some("owner/destination/segments/1.m4s".into())
        );
        assert_eq!(
            legacy_video_lifecycle_copy_key("owner/source/", "owner/destination/", "other/key"),
            None
        );
        assert_eq!(
            legacy_video_lifecycle_copy_key("../source/", "owner/destination/", "../source/x"),
            None
        );
    }

    #[test]
    fn icon_keys_are_operation_bound_and_extension_limited() {
        let operation = "018f47ef-f1da-7cc5-9d84-4ebf3c07ad0e";
        assert_eq!(
            legacy_organisation_icon_key("org", operation, "photo.PNG"),
            Some(format!("organizations/org/{operation}.png"))
        );
        assert_eq!(
            legacy_organisation_icon_key("org", operation, "payload.exe"),
            None
        );
    }
}
