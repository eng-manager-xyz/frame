use std::{collections::BTreeSet, fs, path::Path};

use frame_media::{
    CAP_MEDIA_REFERENCE_COMMIT, MEDIA_SERVICE_CATALOG_VERSION, ManagedMediaContract,
    MediaExecutionDisposition, MediaExecutorKind, MediaJobKind, media_service_catalog,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

const CATALOG_FIXTURE: &str = include_str!("../../../fixtures/media-jobs/v1/catalog.json");

fn fixture() -> Value {
    serde_json::from_str(CATALOG_FIXTURE).expect("media catalog fixture must be strict JSON")
}

fn executor_id(executor: MediaExecutorKind) -> &'static str {
    match executor {
        MediaExecutorKind::CloudflareMedia => "cloudflare_media",
        MediaExecutorKind::NativeGstreamer => "native_gstreamer",
        MediaExecutorKind::ExternalProvider => "external_provider",
        MediaExecutorKind::ControlPlane => "control_plane",
    }
}

fn disposition_id(disposition: MediaExecutionDisposition) -> &'static str {
    match disposition {
        MediaExecutionDisposition::HybridManagedNative => "hybrid_managed_native",
        MediaExecutionDisposition::NativeOnly => "native_only",
        MediaExecutionDisposition::ExternalProviderAdapter => "external_provider_adapter",
    }
}

#[test]
fn pinned_cap_inventory_exactly_matches_the_executable_catalog() {
    let fixture = fixture();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["catalog_version"],
        u64::from(MEDIA_SERVICE_CATALOG_VERSION)
    );
    assert_eq!(fixture["cap_reference_commit"], CAP_MEDIA_REFERENCE_COMMIT);
    let rows = fixture["jobs"].as_array().expect("jobs array");
    let catalog = media_service_catalog().validate().expect("catalog");
    assert_eq!(rows.len(), catalog.jobs.len());

    let mut fixture_ids = BTreeSet::new();
    for row in rows {
        let id = row["id"].as_str().expect("job id");
        assert!(fixture_ids.insert(id), "duplicate fixture job {id}");
        let kind = MediaJobKind::ALL
            .into_iter()
            .find(|kind| kind.id() == id)
            .expect("fixture job must be executable");
        let spec = catalog.get(kind).expect("one spec");
        assert_eq!(row["profile"], spec.profile_id);
        assert_eq!(row["disposition"], disposition_id(spec.disposition));
        assert_eq!(row["preferred"], executor_id(spec.preferred));
        match spec.fallback {
            Some(executor) => assert_eq!(row["fallback"], executor_id(executor)),
            None => assert!(row["fallback"].is_null()),
        }
        let sources: Vec<_> = row["cap_sources"]
            .as_array()
            .expect("sources")
            .iter()
            .map(|source| source.as_str().expect("source"))
            .collect();
        assert_eq!(sources, spec.cap_sources);
        assert!(
            sources
                .iter()
                .all(|source| { source.starts_with("apps/") || source.starts_with("target:") })
        );
    }
    assert_eq!(
        fixture_ids,
        MediaJobKind::ALL
            .into_iter()
            .map(MediaJobKind::id)
            .collect()
    );
}

#[test]
fn non_transform_cap_surfaces_have_explicit_owners() {
    let fixture = fixture();
    let delegated = fixture["delegated_surfaces"]
        .as_array()
        .expect("delegated surfaces");
    assert_eq!(delegated.len(), 4);
    let mut sources = BTreeSet::new();
    for surface in delegated {
        let source = surface["cap_source"].as_str().expect("Cap source");
        let disposition = surface["disposition"].as_str().expect("disposition");
        assert!(sources.insert(source));
        assert!(source.starts_with("apps/"));
        assert!(
            disposition.ends_with("issue_07") || disposition.ends_with("issue_19"),
            "delegated behavior needs a numbered implementation owner"
        );
    }
}

#[test]
fn cloudflare_limits_are_one_revisioned_remote_contract() {
    let contract = ManagedMediaContract::cloudflare_2026_06_10();
    contract.validate().expect("managed contract");
    assert_eq!(contract.revision, "cloudflare-media-binding-2026-06-10");
    assert_eq!(contract.documentation_date, "2026-06-10");
    assert_eq!(contract.max_input_bytes_exclusive, 100_000_000);
    assert_eq!(contract.max_input_duration_ms, 600_000);
    assert_eq!(contract.min_output_duration_ms, 1_000);
    assert_eq!(contract.max_output_duration_ms, 60_000);
    assert_eq!(contract.min_output_dimension, 10);
    assert_eq!(contract.max_output_dimension, 2_000);
}

#[test]
fn licensed_synthetic_remote_lane_fixture_is_immutable_and_probeable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media-jobs/v1");
    let metadata: Value = serde_json::from_str(
        &fs::read_to_string(root.join("synthetic-h264-aac.json")).expect("fixture metadata"),
    )
    .expect("fixture metadata JSON");
    let bytes = fs::read(root.join("synthetic-h264-aac.mp4")).expect("fixture media");
    assert_eq!(
        bytes.len() as u64,
        metadata["bytes"].as_u64().expect("bytes")
    );
    let digest = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(digest, metadata["sha256"].as_str().expect("sha256"));
    assert_eq!(metadata["license"]["spdx"], "CC0-1.0");
    assert_eq!(
        metadata["lanes"]["cloudflare_media_remote"],
        "protected_not_run_without_named_account_and_cost_approval"
    );
    assert!(bytes.windows(4).any(|window| window == b"avc1"));
    assert!(bytes.windows(4).any(|window| window == b"mp4a"));
}
