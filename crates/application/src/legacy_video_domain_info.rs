//! Source-pinned contract for Cap's public video custom-domain lookup.
//!
//! The concrete route performs no session lookup. It resolves the first shared
//! organization for a video, falls back to the first organization owned by the
//! video's owner, and serializes Drizzle's nullable timestamp as an ISO string
//! (or the boolean `false`). Keeping those details explicit prevents the
//! generated catalog's former `session`/boolean inference from becoming a
//! security or wire-contract rewrite.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

pub const LEGACY_VIDEO_DOMAIN_INFO_CAP_COMMIT: &str = "6ba69561ac86b8efdb17616d6727f9638015546b";
pub const LEGACY_VIDEO_DOMAIN_INFO_OPERATION_ID: &str = "cap-v1-10e17d0e86b49830";
pub const LEGACY_VIDEO_DOMAIN_INFO_PATH: &str = "/api/video/domain-info";
pub const LEGACY_VIDEO_DOMAIN_INFO_POLICY: &str = "video_media.v1";
pub const LEGACY_VIDEO_DOMAIN_INFO_NO_PROTECTED_GATES: &[&str] = &[];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVideoDomainInfoSourceRoleV1 {
    Handler,
    PersistenceSchema,
    VideoIdentifier,
    Database,
    ApiMiddlewareExclusion,
    DependencyDeclaration,
    DependencyLock,
}

impl LegacyVideoDomainInfoSourceRoleV1 {
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::PersistenceSchema => "persistence_schema",
            Self::VideoIdentifier => "video_identifier",
            Self::Database => "database",
            Self::ApiMiddlewareExclusion => "api_middleware_exclusion",
            Self::DependencyDeclaration => "dependency_declaration",
            Self::DependencyLock => "dependency_lock",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyVideoDomainInfoSourcePinV1 {
    pub path: &'static str,
    pub symbol: &'static str,
    pub sha256: &'static str,
    pub role: LegacyVideoDomainInfoSourceRoleV1,
}

pub const LEGACY_VIDEO_DOMAIN_INFO_SOURCES: &[LegacyVideoDomainInfoSourcePinV1] = &[
    LegacyVideoDomainInfoSourcePinV1 {
        path: "apps/web/app/api/video/domain-info/route.ts",
        symbol: "GET",
        sha256: "07e0373bace84adabaf409bc1f3360221d01ed7143e1ab49514730d893b66bc5",
        role: LegacyVideoDomainInfoSourceRoleV1::Handler,
    },
    LegacyVideoDomainInfoSourcePinV1 {
        path: "packages/database/schema.ts",
        symbol: "videos+sharedVideos+organizations",
        sha256: "7fce297f9076be78a9ac6280d9d060bf6e836a62e0f82b5390fa0e42dc7bb9e9",
        role: LegacyVideoDomainInfoSourceRoleV1::PersistenceSchema,
    },
    LegacyVideoDomainInfoSourcePinV1 {
        path: "packages/web-domain/src/Video.ts",
        symbol: "Video.VideoId",
        sha256: "adc3db0eded2670b1ed89969e7bc85993e04021acb303082b3d015f0afb1c9a7",
        role: LegacyVideoDomainInfoSourceRoleV1::VideoIdentifier,
    },
    LegacyVideoDomainInfoSourcePinV1 {
        path: "packages/database/index.ts",
        symbol: "db",
        sha256: "161c1d1fd2a561fd2846aeceb148f24b58afc58bdaa95175240e48dbe61d9bbb",
        role: LegacyVideoDomainInfoSourceRoleV1::Database,
    },
    LegacyVideoDomainInfoSourcePinV1 {
        path: "apps/web/proxy.ts",
        symbol: "API matcher exclusion",
        sha256: "7da98445a31f6b48d01b56877c47aaa79ba3af93dff8c015ad06a6e94fb42fcb",
        role: LegacyVideoDomainInfoSourceRoleV1::ApiMiddlewareExclusion,
    },
    LegacyVideoDomainInfoSourcePinV1 {
        path: "apps/web/package.json",
        symbol: "next+drizzle dependencies",
        sha256: "c1358cd1880ac5dc9d659760c2788cedd5c4f61fec2cb0dd1b60cbc9bb8af920",
        role: LegacyVideoDomainInfoSourceRoleV1::DependencyDeclaration,
    },
    LegacyVideoDomainInfoSourcePinV1 {
        path: "pnpm-lock.yaml",
        symbol: "dependency lock",
        sha256: "fc0fe122ae5fbea4dcaa7e510bd6275635c14071c4e031996431a54fb7e25e3a",
        role: LegacyVideoDomainInfoSourceRoleV1::DependencyLock,
    },
];

pub const LEGACY_VIDEO_DOMAIN_INFO_SOURCE_MANIFEST_SHA256: &str =
    "21128e00791e73825651a961f1896bc10d355e7240cc887bba9c877024f4fe00";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyVideoDomainInfoProfileV1 {
    pub operation_id: &'static str,
    pub path: &'static str,
    pub method: &'static str,
    pub auth: &'static str,
    pub query: &'static str,
    pub precedence: &'static str,
    pub domain_verified: &'static str,
    pub failure: &'static str,
}

pub const LEGACY_VIDEO_DOMAIN_INFO_PROFILE: LegacyVideoDomainInfoProfileV1 =
    LegacyVideoDomainInfoProfileV1 {
        operation_id: LEGACY_VIDEO_DOMAIN_INFO_OPERATION_ID,
        path: LEGACY_VIDEO_DOMAIN_INFO_PATH,
        method: "GET",
        auth: "anonymous_concrete_handler_no_session_lookup",
        query: "first_videoId_search_param_required_non_empty",
        precedence: "first_shared_organization_with_domain_then_first_owner_organization",
        domain_verified: "nullable_drizzle_timestamp_serializes_as_iso_string_else_false",
        failure: "400_missing_video_id_404_missing_video_500_invalid_owner_or_database",
    };

#[must_use]
pub fn legacy_video_domain_info_source_manifest() -> String {
    let mut digest = Sha256::new();
    digest.update(b"frame-cap-video-domain-info-source-manifest-v1\0");
    for source in LEGACY_VIDEO_DOMAIN_INFO_SOURCES {
        digest.update(source.path.as_bytes());
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVideoDomainInfoProjectionV1 {
    pub custom_domain: Option<String>,
    pub domain_verified_iso: Option<String>,
}

impl LegacyVideoDomainInfoProjectionV1 {
    #[must_use]
    pub fn absent() -> Self {
        Self {
            custom_domain: None,
            domain_verified_iso: None,
        }
    }

    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.custom_domain
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    }
}

#[must_use]
pub fn legacy_video_domain_info_select(
    shared: Option<LegacyVideoDomainInfoProjectionV1>,
    owner: Option<LegacyVideoDomainInfoProjectionV1>,
) -> LegacyVideoDomainInfoProjectionV1 {
    match shared {
        Some(projection) if projection.is_usable() => projection,
        _ => owner
            .filter(LegacyVideoDomainInfoProjectionV1::is_usable)
            .unwrap_or_else(LegacyVideoDomainInfoProjectionV1::absent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_closure_is_frozen_and_provider_free() {
        assert_eq!(LEGACY_VIDEO_DOMAIN_INFO_SOURCES.len(), 7);
        assert_eq!(
            legacy_video_domain_info_source_manifest(),
            LEGACY_VIDEO_DOMAIN_INFO_SOURCE_MANIFEST_SHA256
        );
        assert!(LEGACY_VIDEO_DOMAIN_INFO_NO_PROTECTED_GATES.is_empty());
        assert!(LEGACY_VIDEO_DOMAIN_INFO_SOURCES.iter().all(
            |source| !source.path.contains("stripe")
                && !source.path.contains("google-drive")
                && !source.path.contains("tinybird")
        ));
    }

    #[test]
    fn concrete_handler_is_anonymous_and_preserves_timestamp_shape() {
        assert_eq!(
            LEGACY_VIDEO_DOMAIN_INFO_PROFILE.auth,
            "anonymous_concrete_handler_no_session_lookup"
        );
        assert_eq!(
            LEGACY_VIDEO_DOMAIN_INFO_PROFILE.domain_verified,
            "nullable_drizzle_timestamp_serializes_as_iso_string_else_false"
        );
    }

    #[test]
    fn shared_domain_wins_but_empty_shared_domain_falls_back_to_owner() {
        let shared = LegacyVideoDomainInfoProjectionV1 {
            custom_domain: Some("shared.example".into()),
            domain_verified_iso: Some("2026-07-17T19:00:00.000Z".into()),
        };
        let owner = LegacyVideoDomainInfoProjectionV1 {
            custom_domain: Some("owner.example".into()),
            domain_verified_iso: None,
        };
        assert_eq!(
            legacy_video_domain_info_select(Some(shared.clone()), Some(owner.clone())),
            shared
        );
        assert_eq!(
            legacy_video_domain_info_select(
                Some(LegacyVideoDomainInfoProjectionV1::absent()),
                Some(owner.clone())
            ),
            owner
        );
    }
}
