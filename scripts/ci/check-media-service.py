#!/usr/bin/env python3
"""Validate the media-service catalog, implementation matrix, and fixtures."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
FIXTURE_DIR = ROOT / "fixtures" / "media-jobs" / "v1"
EXPECTED_JOBS = (
    "optimized_clip",
    "frame",
    "spritesheet",
    "audio_extract",
    "probe",
    "audio_presence",
    "distribution_master",
    "animated_preview",
    "audio_normalize",
    "remux_repair",
    "segment_mux",
    "waveform",
    "composition",
    "normalize",
    "transcription",
    "ai_cleanup",
)
EXPECTED_HYBRID = {
    "optimized_clip",
    "frame",
    "spritesheet",
    "audio_extract",
}
EXPECTED_EXTERNAL = {"transcription", "ai_cleanup"}
EXPECTED_FIXTURES = (
    "frame-synthetic-h264-aac-v1",
    "synthetic_segment_set_v1",
    "synthetic_edit_timeline_v1",
    "synthetic_transcription_input_v1",
    "synthetic_caption_document_v1",
)
EXPECTED_LIMIT_PROFILES = {
    "optimized_clip": "managed_video_plus_light_native",
    "frame": "managed_frame_plus_light_native",
    "spritesheet": "managed_spritesheet_plus_light_native",
    "audio_extract": "managed_audio_plus_light_native",
    "probe": "light_native",
    "audio_presence": "light_native",
    "distribution_master": "heavy_native",
    "animated_preview": "light_native",
    "audio_normalize": "light_native",
    "remux_repair": "heavy_native",
    "segment_mux": "heavy_native",
    "waveform": "light_native",
    "composition": "heavy_native",
    "normalize": "heavy_native",
    "transcription": "external_adapter",
    "ai_cleanup": "external_adapter",
}
EXPECTED_IMPLEMENTATIONS = {
    "optimized_clip": (
        "implemented_primary_with_documented_fallback_exception",
        "cloudflare_video_bounded_v1",
        "optimized_clip_h264_aac_mp4_v1",
        "native_h264_aac_codec_approval",
    ),
    "frame": (
        "implemented_primary_with_documented_fallback_exception",
        "cloudflare_frame_bounded_v1",
        "thumbnail_png_v1",
        "native_jpeg_variant_not_audited",
    ),
    "spritesheet": (
        "implemented_primary_with_documented_fallback_exception",
        "cloudflare_spritesheet_bounded_v1",
        "sampled_contact_sheet_jpeg_v1",
        "native_sampling_contract_not_audited",
    ),
    "audio_extract": (
        "implemented_primary_with_documented_fallback_exception",
        "cloudflare_audio_bounded_v1",
        "audio_extract_aac_m4a_v1",
        "native_h264_aac_codec_approval",
    ),
    "probe": ("implemented", "probe_manifest_v1", "none", "none"),
    "audio_presence": ("implemented", "audio_presence_manifest_v1", "none", "none"),
    "distribution_master": (
        "documented_exception",
        "distribution_master_h264_aac_mp4_v1",
        "none",
        "h264_aac_codec_approval",
    ),
    "animated_preview": (
        "documented_exception",
        "sampled_animated_preview_gif_v1",
        "none",
        "animated_preview_sampling_contract",
    ),
    "audio_normalize": (
        "documented_exception",
        "two_pass_audio_normalize_v1",
        "none",
        "loudness_algorithm_approval",
    ),
    "remux_repair": (
        "documented_exception",
        "allowlisted_remux_repair_mp4_v1",
        "none",
        "container_demux_allowlist",
    ),
    "segment_mux": (
        "documented_exception",
        "ordered_segment_mux_mp4_v1",
        "none",
        "multisource_control_plane_protocol",
    ),
    "waveform": ("implemented", "bounded_waveform_manifest_v1", "none", "none"),
    "composition": (
        "documented_exception",
        "timeline_composition_h264_aac_mp4_v1",
        "none",
        "composition_timeline_protocol",
    ),
    "normalize": (
        "documented_exception",
        "normalized_h264_aac_mp4_v1",
        "none",
        "h264_aac_codec_approval",
    ),
    "transcription": (
        "declared_external_adapter_exception",
        "external_transcription_adapter_v1",
        "none",
        "provider_account_and_caption_tolerance",
    ),
    "ai_cleanup": (
        "declared_external_adapter_exception",
        "external_ai_cleanup_adapter_v1",
        "none",
        "provider_account_and_product_review",
    ),
}
EXPECTED_DELEGATED = {
    (
        "apps/media-server/src/routes/video.ts#/process/:jobId/status",
        "durable_media_job_protocol_issue_07",
    ),
    (
        "apps/media-server/src/routes/video.ts#/process/:jobId/cancel",
        "durable_media_job_protocol_issue_07",
    ),
    (
        "apps/web/app/api/webhooks/media-server/progress/route.ts",
        "lease_fenced_progress_and_completion_issue_07",
    ),
    (
        "apps/web/app/api/webhooks/media-server/multipart/[action]/route.ts",
        "multipart_protocol_issue_19",
    ),
}
SAFE_ID = re.compile(r"^[a-z][a-z0-9_]{0,63}$")
SAFE_PARITY_VALUE = re.compile(r"^[a-z][a-z0-9_-]{0,127}$")
SAFE_SOURCE = re.compile(r"^[A-Za-z0-9_./:#@-]{1,256}$")
SAFE_FIXTURE_FILE = re.compile(r"^[a-z0-9][a-z0-9-]{0,127}\.json$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
MAX_JSON_BYTES = 1_000_000


def fail(message: str) -> "None":
    raise SystemExit(f"media service contract invalid: {message}")


def load_object(path: Path) -> tuple[dict[str, object], bytes]:
    if path.is_symlink():
        fail(f"refusing symlinked input: {path.relative_to(ROOT)}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        fail(f"cannot read {path.relative_to(ROOT)}: {error}")
    if not raw or len(raw) > MAX_JSON_BYTES or b"\x00" in raw:
        fail(f"unsafe JSON size or encoding: {path.relative_to(ROOT)}")
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON in {path.relative_to(ROOT)}: {error}")
    if not isinstance(value, dict):
        fail(f"top level must be an object: {path.relative_to(ROOT)}")
    return value, raw


def exact_keys(value: dict[str, object], expected: set[str], label: str) -> None:
    found = set(value)
    if found != expected:
        fail(
            f"{label} keys differ: missing={sorted(expected - found)} "
            f"extra={sorted(found - expected)}"
        )


def validate_catalog() -> tuple[str, str]:
    path = FIXTURE_DIR / "catalog.json"
    catalog, raw = load_object(path)
    exact_keys(
        catalog,
        {
            "schema_version",
            "catalog_version",
            "cap_reference_commit",
            "vendor_contract_revision",
            "inventory_method",
            "jobs",
            "delegated_surfaces",
        },
        "catalog",
    )
    if catalog["schema_version"] != 1 or catalog["catalog_version"] != 1:
        fail("unsupported catalog version")
    if catalog["cap_reference_commit"] != "6ba69561ac86b8efdb17616d6727f9638015546b":
        fail("Cap reference commit drifted")
    if catalog["vendor_contract_revision"] != "cloudflare-media-binding-2026-06-10":
        fail("managed contract revision drifted")
    if not isinstance(catalog["inventory_method"], str) or not catalog["inventory_method"].strip():
        fail("inventory method is missing")

    jobs = catalog["jobs"]
    if not isinstance(jobs, list) or len(jobs) != len(EXPECTED_JOBS):
        fail("job catalog does not contain exactly 16 records")
    ids: list[str] = []
    for index, item in enumerate(jobs):
        if not isinstance(item, dict):
            fail(f"job {index} is not an object")
        exact_keys(
            item,
            {
                "id",
                "input_role",
                "output_role",
                "profile",
                "disposition",
                "preferred",
                "fallback",
                "cap_sources",
            },
            f"job {index}",
        )
        job_id = item["id"]
        if not isinstance(job_id, str) or SAFE_ID.fullmatch(job_id) is None:
            fail(f"job {index} has unsafe ID")
        ids.append(job_id)
        for field in ("input_role", "output_role", "profile", "disposition", "preferred"):
            value = item[field]
            if not isinstance(value, str) or SAFE_ID.fullmatch(value) is None:
                fail(f"job {job_id} has unsafe {field}")
        sources = item["cap_sources"]
        if not isinstance(sources, list) or not sources:
            fail(f"job {job_id} has no Cap sources")
        if any(not isinstance(source, str) or SAFE_SOURCE.fullmatch(source) is None for source in sources):
            fail(f"job {job_id} has an unsafe Cap source")

        if job_id in EXPECTED_HYBRID:
            expected = ("hybrid_managed_native", "cloudflare_media", "native_gstreamer")
        elif job_id in EXPECTED_EXTERNAL:
            expected = ("external_provider_adapter", "external_provider", None)
        else:
            expected = ("native_only", "native_gstreamer", None)
        actual = (item["disposition"], item["preferred"], item["fallback"])
        if actual != expected:
            fail(f"job {job_id} executor disposition drifted: {actual!r}")

    if tuple(ids) != EXPECTED_JOBS or len(ids) != len(set(ids)):
        fail("job identity/order differs from the executable v1 contract")

    delegated = catalog["delegated_surfaces"]
    if not isinstance(delegated, list):
        fail("delegated surfaces must be a list")
    normalized: set[tuple[str, str]] = set()
    for index, item in enumerate(delegated):
        if not isinstance(item, dict):
            fail(f"delegated surface {index} is not an object")
        exact_keys(item, {"cap_source", "disposition"}, f"delegated surface {index}")
        source, disposition = item["cap_source"], item["disposition"]
        if not isinstance(source, str) or not isinstance(disposition, str):
            fail(f"delegated surface {index} fields must be strings")
        normalized.add((source, disposition))
    if normalized != EXPECTED_DELEGATED or len(delegated) != len(EXPECTED_DELEGATED):
        fail("delegated protocol surface ownership drifted")
    return hashlib.sha256(raw).hexdigest(), str(catalog["vendor_contract_revision"])


def validate_fixture() -> tuple[str, int]:
    manifest_path = FIXTURE_DIR / "synthetic-h264-aac.json"
    manifest, _ = load_object(manifest_path)
    exact_keys(
        manifest,
        {
            "schema_version",
            "fixture_id",
            "media_file",
            "sha256",
            "bytes",
            "license",
            "generator",
            "probe",
            "lanes",
        },
        "fixture manifest",
    )
    if manifest["schema_version"] != 1 or manifest["fixture_id"] != "frame-synthetic-h264-aac-v1":
        fail("unsupported fixture identity")
    name = manifest["media_file"]
    if name != "synthetic-h264-aac.mp4":
        fail("fixture filename drifted")
    expected_hash = manifest["sha256"]
    expected_bytes = manifest["bytes"]
    if not isinstance(expected_hash, str) or SHA256.fullmatch(expected_hash) is None:
        fail("fixture SHA-256 is invalid")
    if not isinstance(expected_bytes, int) or isinstance(expected_bytes, bool) or expected_bytes <= 0:
        fail("fixture byte length is invalid")
    media_path = FIXTURE_DIR / name
    if media_path.is_symlink():
        fail("refusing symlinked media fixture")
    media = media_path.read_bytes()
    if len(media) != expected_bytes or hashlib.sha256(media).hexdigest() != expected_hash:
        fail("media bytes do not match the immutable fixture manifest")
    if b"ftyp" not in media[:64] or b"moov" not in media or b"avc1" not in media or b"mp4a" not in media:
        fail("fixture lacks expected MP4/H.264/AAC structural markers")

    license_value = manifest["license"]
    if not isinstance(license_value, dict) or license_value.get("spdx") != "CC0-1.0":
        fail("fixture license is not CC0-1.0")
    lanes = manifest["lanes"]
    if not isinstance(lanes, dict) or lanes.get("local_native_probe") != "required":
        fail("local native probe lane is not required")
    if lanes.get("cloudflare_media_remote") != "protected_not_run_without_named_account_and_cost_approval":
        fail("remote evidence gate must remain explicit")
    if lanes.get("cross_executor_golden") != "protected_pending_remote_output":
        fail("cross-executor evidence gate must remain explicit")
    return expected_hash, expected_bytes


def validate_fixture_registry() -> dict[str, set[str]]:
    registry, _ = load_object(FIXTURE_DIR / "fixture-registry.json")
    exact_keys(
        registry,
        {"schema_version", "fixture_revision", "fixtures"},
        "fixture registry",
    )
    if registry["schema_version"] != 1:
        fail("unsupported fixture registry version")
    if registry["fixture_revision"] != "media-input-fixtures-v1":
        fail("fixture registry revision drifted")
    fixtures = registry["fixtures"]
    if not isinstance(fixtures, list) or len(fixtures) != len(EXPECTED_FIXTURES):
        fail("fixture registry does not contain the exact fixture set")

    fixture_jobs: dict[str, set[str]] = {}
    artifact_files: set[str] = set()
    owned_jobs: set[str] = set()
    for index, fixture in enumerate(fixtures):
        if not isinstance(fixture, dict):
            fail(f"fixture registry row {index} is not an object")
        exact_keys(
            fixture,
            {"fixture_id", "kind", "artifact_file", "artifact_sha256", "jobs"},
            f"fixture registry row {index}",
        )
        fixture_id = fixture["fixture_id"]
        kind = fixture["kind"]
        artifact_file = fixture["artifact_file"]
        artifact_sha256 = fixture["artifact_sha256"]
        jobs = fixture["jobs"]
        if not isinstance(fixture_id, str) or SAFE_PARITY_VALUE.fullmatch(fixture_id) is None:
            fail(f"fixture registry row {index} has an unsafe fixture ID")
        if not isinstance(kind, str) or SAFE_PARITY_VALUE.fullmatch(kind) is None:
            fail(f"fixture registry row {index} has an unsafe kind")
        if not isinstance(artifact_file, str) or SAFE_FIXTURE_FILE.fullmatch(artifact_file) is None:
            fail(f"fixture registry row {index} has an unsafe artifact filename")
        if not isinstance(artifact_sha256, str) or SHA256.fullmatch(artifact_sha256) is None:
            fail(f"fixture registry row {index} has an invalid artifact digest")
        if not isinstance(jobs, list) or not jobs:
            fail(f"fixture registry row {index} owns no parity jobs")
        if any(not isinstance(job, str) or job not in EXPECTED_JOBS for job in jobs):
            fail(f"fixture registry row {index} owns an unknown parity job")
        if len(jobs) != len(set(jobs)) or owned_jobs.intersection(jobs):
            fail(f"fixture registry row {index} duplicates parity job ownership")

        artifact_path = FIXTURE_DIR / artifact_file
        artifact, raw = load_object(artifact_path)
        if artifact.get("schema_version") != 1 or artifact.get("fixture_id") != fixture_id:
            fail(f"fixture artifact identity differs for {fixture_id}")
        license_value = artifact.get("license")
        if not isinstance(license_value, dict) or license_value.get("spdx") != "CC0-1.0":
            fail(f"fixture artifact is not CC0-1.0: {fixture_id}")
        if license_value.get("customer_media") not in (None, False):
            fail(f"fixture artifact includes customer media: {fixture_id}")
        if hashlib.sha256(raw).hexdigest() != artifact_sha256:
            fail(f"fixture artifact digest differs for {fixture_id}")
        if artifact_file in artifact_files or fixture_id in fixture_jobs:
            fail(f"fixture registry identity is duplicated: {fixture_id}")
        artifact_files.add(artifact_file)
        owned_jobs.update(jobs)
        fixture_jobs[fixture_id] = set(jobs)

    if tuple(fixture_jobs) != EXPECTED_FIXTURES:
        fail("fixture identity/order differs from the executable contract")
    if owned_jobs != set(EXPECTED_JOBS):
        fail("fixture registry does not own every retained job exactly once")
    return fixture_jobs


def validate_parity_matrix(fixture_jobs: dict[str, set[str]]) -> None:
    matrix, _ = load_object(FIXTURE_DIR / "parity-matrix.json")
    exact_keys(
        matrix,
        {"schema_version", "catalog_version", "fixture_revision", "jobs"},
        "parity matrix",
    )
    if matrix["schema_version"] != 2 or matrix["catalog_version"] != 1:
        fail("unsupported parity matrix version")
    if matrix["fixture_revision"] != "media-parity-v2":
        fail("parity matrix revision drifted")
    jobs = matrix["jobs"]
    if not isinstance(jobs, list) or len(jobs) != len(EXPECTED_JOBS):
        fail("parity matrix does not contain exactly 16 rows")
    ids: list[str] = []
    required = {
        "id",
        "input_fixture",
        "implementation_state",
        "primary_executor",
        "primary_implementation",
        "fallback_executor",
        "fallback_implementation",
        "limit_profile",
        "fallback_disposition",
        "exception",
        "evidence",
    }
    for index, row in enumerate(jobs):
        if not isinstance(row, dict):
            fail(f"parity row {index} is not an object")
        exact_keys(row, required, f"parity row {index}")
        if not isinstance(row["id"], str) or SAFE_ID.fullmatch(row["id"]) is None:
            fail(f"parity row {index} has an unsafe job ID")
        if any(
            not isinstance(row[field], str) or SAFE_PARITY_VALUE.fullmatch(row[field]) is None
            for field in required - {"id"}
        ):
            fail(f"parity row {index} contains an unsafe identifier")
        job_id = row["id"]
        ids.append(job_id)

        fixture_id = row["input_fixture"]
        if fixture_id not in fixture_jobs or job_id not in fixture_jobs[fixture_id]:
            fail(f"parity row {job_id} is not owned by its concrete fixture")
        if row["limit_profile"] != EXPECTED_LIMIT_PROFILES[job_id]:
            fail(f"parity row {job_id} has a drifted limit profile")
        expected_implementation = EXPECTED_IMPLEMENTATIONS[job_id]
        actual_implementation = (
            row["implementation_state"],
            row["primary_implementation"],
            row["fallback_implementation"],
            row["exception"],
        )
        if actual_implementation != expected_implementation:
            fail(f"parity row {job_id} implementation or exception drifted")

        if job_id in EXPECTED_HYBRID:
            executors = ("cloudflare_media", "native_gstreamer")
            fallback_disposition = "native_on_approved_managed_failure"
        elif job_id in EXPECTED_EXTERNAL:
            executors = ("external_provider", "none")
            fallback_disposition = "none"
        else:
            executors = ("native_gstreamer", "none")
            fallback_disposition = "none"
        if (row["primary_executor"], row["fallback_executor"]) != executors:
            fail(f"parity row {job_id} executor declaration drifted")
        if row["fallback_disposition"] != fallback_disposition:
            fail(f"parity row {job_id} fallback disposition drifted")
    if tuple(ids) != EXPECTED_JOBS or len(ids) != len(set(ids)):
        fail("parity matrix identity/order differs from the catalog")


def validate_repository_contract() -> None:
    source = (ROOT / "crates" / "media" / "src" / "jobs" / "service.rs").read_text(
        encoding="utf-8"
    )
    for required in (
        "MEDIA_SERVICE_CATALOG_VERSION: u16 = 1",
        "cloudflare-media-binding-2026-06-10",
        "pub trait MediaDerivativeExecutorPort",
        "pub struct OfflineMediaDerivativeExecutor",
        "pub struct DurableMediaJob",
        "AwaitExecutorFence",
        "executor_fenced",
    ):
        if required not in source:
            fail(f"Rust contract marker is missing: {required}")
    for path in (
        ROOT / "docs" / "architecture" / "media-service-v1.md",
        ROOT / "docs" / "operations" / "media-service.md",
        ROOT / "docs" / "evidence" / "media-service-local.md",
    ):
        if not path.is_file() or path.is_symlink() or path.stat().st_size < 500:
            fail(f"required documentation is absent or unsafe: {path.relative_to(ROOT)}")

    binding = (ROOT / "apps" / "control-plane" / "src" / "cloudflare_media.rs").read_text(
        encoding="utf-8"
    )
    native = (ROOT / "apps" / "media-worker" / "src" / "native.rs").read_text(
        encoding="utf-8"
    )
    runtime = (
        ROOT / "apps" / "control-plane" / "src" / "media_service_runtime.rs"
    ).read_text(encoding="utf-8")
    control_plane = (ROOT / "apps" / "control-plane" / "src" / "lib.rs").read_text(
        encoding="utf-8"
    )
    migration = (
        ROOT
        / "apps"
        / "control-plane"
        / "migrations"
        / "0014_media_service_runtime.sql"
    ).read_text(encoding="utf-8")
    wrangler = (ROOT / "apps" / "control-plane" / "wrangler.toml").read_text(
        encoding="utf-8"
    )
    for required in (
        'const MEDIA_BINDING: &str = "MEDIA"',
        "ReadableStream::from_stream",
        "execute_to_staging",
        "publish_staged",
        "cancel_and_confirm_absent",
        'etag_does_not_match: Some("*".into())',
        'cache_control: Some("private, no-store".into())',
    ):
        if required not in binding:
            fail(f"Cloudflare binding marker is missing: {required}")
    for implementation_id in (
        "cloudflare_video_bounded_v1",
        "cloudflare_frame_bounded_v1",
        "cloudflare_spritesheet_bounded_v1",
        "cloudflare_audio_bounded_v1",
    ):
        if f'"{implementation_id}"' not in binding:
            fail(f"bounded Cloudflare implementation is missing: {implementation_id}")
    native_implementation_ids = {
        implementation
        for job_id, (_, primary, fallback, _) in EXPECTED_IMPLEMENTATIONS.items()
        if job_id not in EXPECTED_EXTERNAL
        for implementation in (primary, fallback)
        if implementation != "none" and not implementation.startswith("cloudflare_")
    }
    for implementation_id in native_implementation_ids:
        if f'graph_id: "{implementation_id}"' not in native:
            fail(f"native implementation/exception entry is missing: {implementation_id}")
    for required in (
        "NativeImplementationStateV1::Executable",
        "NativeImplementationStateV1::ExecutableWithVariantException",
        "NativeImplementationStateV1::DocumentedException",
        "every_native_profile_has_an_audited_graph_or_typed_exception",
    ):
        if required not in native:
            fail(f"native implementation catalog marker is missing: {required}")
    if re.search(r'"https?://', re.sub(r"//!.*|//.*", "", binding)):
        fail("Cloudflare adapter source contains a runtime URL literal")
    for required in (
        "verified_native_probe",
        "media_job_execution_v1",
        "lease_epoch = lease_epoch + 1",
        "fallback_queued",
        "media_output_manifests_v1",
        "recover_one",
    ):
        if required not in runtime:
            fail(f"managed runtime marker is missing: {required}")
    for required in (
        "CREATE TABLE media_profile_policies_v1",
        "CREATE TABLE media_source_probes_v1",
        "CREATE TABLE media_job_execution_v1",
        "CREATE TABLE media_output_manifests_v1",
        "CREATE TABLE media_execution_events_v1",
        "CREATE TABLE media_native_output_staging_v1",
        "media_output_manifest_immutable",
        "media_native_staging_authority_mismatch",
        "media_native_staging_transition_invalid",
    ):
        if required not in migration:
            fail(f"media migration marker is missing: {required}")
    if '[triggers]\ncrons = ["* * * * *"]' not in wrangler:
        fail("managed recovery cron is not configured")
    for required in (
        "recover_native_staging_one",
        "r2_absent_twice",
        "native_output_candidate_key",
        "media_native_output_staging_v1",
        '"{}.attempt-{}.{}.partial"',
    ):
        if required not in control_plane:
            fail(f"native staging/recovery marker is missing: {required}")

    claim_start = control_plane.find("async fn native_job_claim_response")
    claim_end = control_plane.find("async fn native_job_progress_response", claim_start)
    if claim_start < 0 or claim_end < 0:
        fail("native claim implementation is missing")
    if "'segment_mux_v1'" in control_plane[claim_start:claim_end]:
        fail("segment mux must not be claimed before the multi-source protocol exists")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    catalog_sha256, vendor_revision = validate_catalog()
    fixture_sha256, fixture_bytes = validate_fixture()
    fixture_jobs = validate_fixture_registry()
    validate_parity_matrix(fixture_jobs)
    validate_repository_contract()
    evidence = {
        "schema_version": 1,
        "catalog_sha256": catalog_sha256,
        "catalog_jobs": len(EXPECTED_JOBS),
        "vendor_contract_revision": vendor_revision,
        "fixture_sha256": fixture_sha256,
        "fixture_bytes": fixture_bytes,
        "fixture_artifacts": len(fixture_jobs),
        "parity_jobs": len(EXPECTED_JOBS),
        "customer_media_included": False,
        "remote_execution_claimed": False,
        "result": "pass",
    }
    rendered = json.dumps(evidence, sort_keys=True, separators=(",", ":")) + "\n"
    if args.evidence is not None:
        if not args.evidence.parent.is_dir():
            fail("evidence parent directory does not exist")
        args.evidence.write_text(rendered, encoding="utf-8")
    sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
