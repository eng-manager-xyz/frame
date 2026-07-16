#!/usr/bin/env python3
"""Validate the audited GStreamer contract against privacy-safe doctor output."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CONTRACT = ROOT / "crates" / "media" / "gstreamer-runtime.json"
MAX_JSON_BYTES = 1_000_000
FACTORY_RE = re.compile(r"^[A-Za-z0-9_.+-]{1,128}$")
CAPABILITY_RE = re.compile(r"^[a-z0-9_]{1,64}$")
PACKAGE_RE = re.compile(r"^[a-z0-9][a-z0-9+.-]{0,127}$")
TARGETS = {
    "linux-x86_64",
    "linux-aarch64",
    "macos-x86_64",
    "macos-aarch64",
    "windows-x86_64",
}
DENIED_ENVIRONMENT_VARIABLES = {
    "GST_PLUGIN_PATH",
    "GST_PLUGIN_PATH_1_0",
    "GST_PLUGIN_SYSTEM_PATH",
    "GST_PLUGIN_SCANNER",
    "GST_PLUGIN_SCANNER_1_0",
    "GST_REGISTRY",
    "GST_REGISTRY_1_0",
    "GST_REGISTRY_DISABLE",
    "GST_REGISTRY_UPDATE",
    "GST_REGISTRY_FORK",
    "GST_REGISTRY_MODE",
    "GST_REGISTRY_REUSE_PLUGIN_SCANNER",
    "GST_PLUGIN_LOADING_WHITELIST",
    "GST_PLUGIN_FEATURE_RANK",
}
DENIED_LOADER_ENVIRONMENT_VARIABLES = {
    "LD_LIBRARY_PATH",
    "LD_PRELOAD",
    "LD_AUDIT",
    "DYLD_LIBRARY_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_FALLBACK_FRAMEWORK_PATH",
    "DYLD_ROOT_PATH",
    "DYLD_IMAGE_SUFFIX",
    "DYLD_SHARED_REGION",
}
TRUSTED_SYSTEM_PATH_VARIABLE = "GST_PLUGIN_SYSTEM_PATH_1_0"
TARGET_POLICY = {
    "linux-x86_64": (
        "hosted_runner_apt_packages",
        "synthetic_ci_only",
        "github-hosted-ubuntu-24.04-apt-origin-unverified",
    ),
    "linux-aarch64": (
        "system_package_plan_unapproved",
        "clean_machine_gate_pending",
        "ubuntu-24.04-origin-and-signature-evidence-pending",
    ),
    "macos-x86_64": (
        "signed_app_bundle_pending",
        "bundle_signing_gate_pending",
        "pinned-gstreamer-runtime-not-yet-approved",
    ),
    "macos-aarch64": (
        "signed_app_bundle_pending",
        "bundle_signing_gate_pending",
        "pinned-gstreamer-runtime-not-yet-approved",
    ),
    "windows-x86_64": (
        "signed_app_bundle_pending",
        "bundle_signing_gate_pending",
        "pinned-gstreamer-runtime-not-yet-approved",
    ),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--contract", type=Path, default=DEFAULT_CONTRACT)
    parser.add_argument("--doctor", type=Path, required=True)
    parser.add_argument("--packages", type=Path)
    parser.add_argument("--evidence", type=Path, required=True)
    return parser.parse_args()


def load_json(path: Path) -> tuple[dict[str, object], bytes]:
    if path.is_symlink():
        fail(f"refusing symlinked JSON input: {path.name}")
    try:
        raw = path.read_bytes()
    except OSError as error:
        fail(f"could not read {path.name}: {error}")
    if not raw or len(raw) > MAX_JSON_BYTES or b"\x00" in raw:
        fail(f"invalid JSON size or encoding marker in {path.name}")
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"invalid JSON in {path.name}: {error}")
    if not isinstance(value, dict):
        fail(f"top-level JSON in {path.name} must be an object")
    return value, raw


def require_keys(value: dict[str, object], expected: set[str], label: str) -> None:
    found = set(value)
    if found != expected:
        fail(f"{label} keys differ: missing={sorted(expected - found)} extra={sorted(found - expected)}")


def validate_factory(item: object, label: str) -> tuple[str, str, str, str]:
    if not isinstance(item, dict):
        fail(f"{label} must be an object")
    required = {"factory", "capability", "requirement", "platform"}
    if "available" in item:
        required.add("available")
        required.add("trusted_provenance")
        required.add("plugin_version")
    require_keys(item, required, label)
    factory = item.get("factory")
    capability = item.get("capability")
    requirement = item.get("requirement")
    platform_name = item.get("platform")
    if not isinstance(factory, str) or FACTORY_RE.fullmatch(factory) is None:
        fail(f"{label} has an unsafe factory name")
    if not isinstance(capability, str) or CAPABILITY_RE.fullmatch(capability) is None:
        fail(f"{label} has an unsafe capability")
    if requirement not in {"required", "optional", "prohibited"}:
        fail(f"{label} has an invalid requirement")
    if platform_name not in {"all", "native_desktop", "linux", "macos", "windows"}:
        fail(f"{label} has an invalid platform")
    if "available" in item and not isinstance(item["available"], bool):
        fail(f"{label} availability must be boolean")
    if "trusted_provenance" in item and not isinstance(item["trusted_provenance"], bool):
        fail(f"{label} provenance must be boolean")
    if "plugin_version" in item:
        plugin_version = item["plugin_version"]
        if plugin_version is not None and (
            not isinstance(plugin_version, str)
            or re.fullmatch(r"[A-Za-z0-9_.+-]{1,64}", plugin_version) is None
        ):
            fail(f"{label} plugin version is invalid")
    return factory, capability, str(requirement), str(platform_name)


def validate_contract(contract: dict[str, object]) -> list[tuple[str, str, str, str]]:
    require_keys(
        contract,
        {
            "schema_version",
            "rust_manifest_version",
            "minimum_gstreamer",
            "plugin_search_policy",
            "factories",
            "targets",
            "codec_decisions",
        },
        "contract",
    )
    if contract["schema_version"] != 3:
        fail("unsupported runtime contract schema")
    if not isinstance(contract["rust_manifest_version"], int) or contract["rust_manifest_version"] <= 0:
        fail("invalid Rust manifest version")
    if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", str(contract["minimum_gstreamer"])) is None:
        fail("minimum GStreamer version must be numeric semver")
    policy = contract["plugin_search_policy"]
    if not isinstance(policy, dict):
        fail("plugin search policy must be an object")
    require_keys(
        policy,
        {
            "denied_environment_variables",
            "denied_loader_environment_variables",
            "trusted_system_path_variable",
            "trusted_system_path_must_match_build_time_pluginsdir",
            "system_plugin_fallback_allowed_for_bundles",
            "undeclared_frame_authored_factories_allowed",
            "trusted_service_launcher",
            "raw_binary_rejects_loader_overrides",
            "signed_launch_chain_gate_pending",
        },
        "plugin search policy",
    )
    if set(policy["denied_environment_variables"]) != DENIED_ENVIRONMENT_VARIABLES:
        fail("plugin search override denylist differs from the runtime")
    if set(policy["denied_loader_environment_variables"]) != DENIED_LOADER_ENVIRONMENT_VARIABLES:
        fail("native loader override denylist differs from the runtime")
    if policy["trusted_system_path_variable"] != TRUSTED_SYSTEM_PATH_VARIABLE:
        fail("trusted plugin system path variable differs from the runtime")
    if policy["trusted_system_path_must_match_build_time_pluginsdir"] is not True:
        fail("trusted plugin system path must match the build-time runtime")
    if policy["system_plugin_fallback_allowed_for_bundles"] is not False:
        fail("signed bundles must not fall back to unversioned system plugins")
    if policy["undeclared_frame_authored_factories_allowed"] is not False:
        fail("undeclared Frame-authored factories must remain forbidden")
    if policy["trusted_service_launcher"] != "scripts/ci/gstreamer-sanitized-exec":
        fail("trusted GStreamer service launcher differs from the audited entrypoint")
    if policy["raw_binary_rejects_loader_overrides"] is not True:
        fail("raw GStreamer binaries must reject loader overrides")
    if policy["signed_launch_chain_gate_pending"] is not True:
        fail("signed launch-chain gate must remain explicitly pending")

    factories = contract["factories"]
    if not isinstance(factories, list) or not factories:
        fail("factory contract is empty")
    normalized = [validate_factory(item, f"contract factory {index}") for index, item in enumerate(factories)]
    names = [item[0] for item in normalized]
    if len(names) != len(set(names)):
        fail("factory contract contains duplicate names")

    targets = contract["targets"]
    if not isinstance(targets, list):
        fail("target policy must be a list")
    found_targets: set[str] = set()
    for index, target in enumerate(targets):
        if not isinstance(target, dict):
            fail(f"target {index} must be an object")
        require_keys(
            target,
            {"target", "delivery_model", "release_status", "source", "ci_top_level_packages"},
            f"target {index}",
        )
        target_name = target["target"]
        if target_name not in TARGETS:
            fail(f"target {index} is unsupported")
        found_targets.add(str(target_name))
        expected_policy = TARGET_POLICY[str(target_name)]
        if (
            target["delivery_model"],
            target["release_status"],
            target["source"],
        ) != expected_policy:
            fail(f"target {index} status/source policy differs from the protected claim")
        if not isinstance(target["ci_top_level_packages"], list) or not all(
            isinstance(item, str) for item in target["ci_top_level_packages"]
        ):
            fail(f"target {index} package list is invalid")
    if found_targets != TARGETS:
        fail(f"target policy is incomplete: {sorted(TARGETS - found_targets)}")
    return normalized


def validate_doctor(
    doctor: dict[str, object],
    contract: dict[str, object],
    expected_factories: list[tuple[str, str, str, str]],
) -> None:
    require_keys(
        doctor,
        {"schema_version", "application_version", "manifest_version", "minimum_gstreamer", "ready", "runtime_version", "issues", "factories"},
        "doctor",
    )
    if doctor["schema_version"] != 2:
        fail("unsupported doctor schema")
    if not isinstance(doctor["application_version"], str) or re.fullmatch(
        r"[0-9]+\.[0-9]+\.[0-9]+", doctor["application_version"]
    ) is None:
        fail("doctor application version is invalid")
    if doctor["manifest_version"] != contract["rust_manifest_version"]:
        fail("doctor and contract manifest versions differ")
    if doctor["minimum_gstreamer"] != contract["minimum_gstreamer"]:
        fail("doctor and contract minimum versions differ")
    if doctor["ready"] is not True or doctor["issues"] != []:
        fail("GStreamer doctor is not ready")
    if not isinstance(doctor["runtime_version"], str) or len(doctor["runtime_version"]) > 128:
        fail("doctor runtime version is invalid")
    doctor_factories = doctor["factories"]
    if not isinstance(doctor_factories, list):
        fail("doctor factory list is invalid")
    normalized = [validate_factory(item, f"doctor factory {index}") for index, item in enumerate(doctor_factories)]
    if normalized != expected_factories:
        fail("Rust runtime factory manifest differs from the audited JSON contract")
    unavailable = [
        item["factory"]
        for item in doctor_factories
        if isinstance(item, dict) and item.get("requirement") == "required" and item.get("available") is not True
    ]
    if unavailable:
        fail(f"required factories are unavailable: {unavailable}")
    untrusted = [
        item["factory"]
        for item in doctor_factories
        if isinstance(item, dict)
        and item.get("available") is True
        and item.get("trusted_provenance") is not True
    ]
    if untrusted:
        fail(f"available factories are outside the trusted plugin root: {untrusted}")
    missing_versions = [
        item["factory"]
        for item in doctor_factories
        if isinstance(item, dict)
        and item.get("available") is True
        and not isinstance(item.get("plugin_version"), str)
    ]
    if missing_versions:
        fail(f"available factories have no bounded plugin version: {missing_versions}")


def validate_package_inventory(
    path: Path | None, contract: dict[str, object]
) -> list[dict[str, str]]:
    if path is None:
        if platform.system() == "Linux" and platform.machine() in {"x86_64", "AMD64"}:
            fail("Linux x86_64 validation requires the installed top-level package inventory")
        return []
    if path.is_symlink():
        fail("refusing symlinked top-level package inventory")
    try:
        raw = path.read_bytes()
    except OSError as error:
        fail(f"could not read top-level package inventory: {error}")
    if not raw or len(raw) > 256_000 or b"\x00" in raw:
        fail("invalid top-level package inventory size or encoding marker")
    try:
        lines = raw.decode("utf-8").splitlines()
    except UnicodeDecodeError as error:
        fail(f"invalid top-level package inventory encoding: {error}")
    packages: dict[str, str] = {}
    for index, line in enumerate(lines):
        fields = line.split("\t")
        if len(fields) != 3:
            fail(f"package inventory row {index} must contain name, version, and status")
        binary_name, version, status = fields
        name_parts = binary_name.rsplit(":", maxsplit=1)
        name = name_parts[0]
        if len(name_parts) == 2 and name_parts[1] != "amd64":
            fail(f"package inventory row {index} has the wrong architecture")
        if PACKAGE_RE.fullmatch(name) is None or not version or len(version) > 128:
            fail(f"package inventory row {index} is invalid")
        if any(character.isspace() or ord(character) < 32 or ord(character) == 127 for character in version):
            fail(f"package inventory version {index} is invalid")
        if status != "install ok installed":
            fail(f"package inventory row {index} is not installed")
        if name in packages:
            fail(f"package inventory contains duplicate package {name}")
        packages[name] = version

    targets = contract["targets"]
    assert isinstance(targets, list)
    target = next(
        (
            item
            for item in targets
            if isinstance(item, dict) and item.get("target") == "linux-x86_64"
        ),
        None,
    )
    if target is None or not isinstance(target.get("ci_top_level_packages"), list):
        fail("Linux x86_64 package policy is absent")
    expected = set(target["ci_top_level_packages"])
    found = set(packages)
    if found != expected:
        fail(
            "installed top-level GStreamer package inventory differs from the CI selection: "
            f"missing={sorted(expected - found)} extra={sorted(found - expected)}"
        )
    return [
        {"name": name, "version": packages[name]}
        for name in sorted(packages)
    ]


def write_evidence(
    path: Path,
    contract_raw: bytes,
    contract: dict[str, object],
    doctor: dict[str, object],
    packages: list[dict[str, str]],
) -> None:
    if path.exists() and path.is_symlink():
        fail("refusing to replace symlinked evidence output")
    active_overrides = sorted(
        name for name in DENIED_ENVIRONMENT_VARIABLES if os.environ.get(name)
    )
    active_loader_overrides = sorted(
        name for name in DENIED_LOADER_ENVIRONMENT_VARIABLES if os.environ.get(name)
    )
    if active_overrides:
        fail(f"untrusted plugin search overrides are active: {active_overrides}")
    if active_loader_overrides:
        fail(f"untrusted native loader overrides are active: {active_loader_overrides}")
    if not os.environ.get(TRUSTED_SYSTEM_PATH_VARIABLE):
        fail("trusted build-time plugin path is not configured")
    payload = {
        "schema_version": 2,
        "contract_sha256": hashlib.sha256(contract_raw).hexdigest(),
        "manifest_version": contract["rust_manifest_version"],
        "minimum_gstreamer": contract["minimum_gstreamer"],
        "runner": {"system": platform.system(), "machine": platform.machine()},
        "runtime_version": doctor["runtime_version"],
        "application_version": doctor["application_version"],
        "required_factories": sorted(
            item["factory"]
            for item in doctor["factories"]
            if isinstance(item, dict) and item.get("requirement") == "required"
        ),
        "optional_factories_available": sorted(
            item["factory"]
            for item in doctor["factories"]
            if isinstance(item, dict)
            and item.get("requirement") == "optional"
            and item.get("available") is True
        ),
        "ci_top_level_packages": packages,
        "factory_plugin_versions": {
            item["factory"]: item["plugin_version"]
            for item in doctor["factories"]
            if isinstance(item, dict) and isinstance(item.get("plugin_version"), str)
        },
        "plugin_search_overrides": active_overrides,
        "native_loader_overrides": active_loader_overrides,
        "trusted_system_path_configured": True,
        "result": "pass",
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def fail(message: str) -> None:
    print(f"GStreamer runtime contract failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def main() -> None:
    args = parse_args()
    contract, contract_raw = load_json(args.contract)
    doctor, _ = load_json(args.doctor)
    factories = validate_contract(contract)
    validate_doctor(doctor, contract, factories)
    packages = validate_package_inventory(args.packages, contract)
    write_evidence(args.evidence, contract_raw, contract, doctor, packages)
    print(f"GStreamer runtime contract passed ({len(factories)} factories)")


if __name__ == "__main__":
    main()
