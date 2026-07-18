#!/usr/bin/env python3
"""Credential-free conformance gate for Issue 39 same-origin routing."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import tomllib
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
MATRIX_PATH = ROOT / "fixtures" / "same-origin-routing" / "v1" / "route-owner-matrix.json"
INVENTORY_PATH = ROOT / "fixtures" / "same-origin-routing" / "v1" / "ownership-inventory.json"
WRANGLER_PATH = ROOT / "apps" / "control-plane" / "wrangler.toml"
RENDER_PATH = ROOT / "render.yaml"
ZONE_PATH = ROOT / "infra" / "cloudflare-zone" / "frame-contract.json"
ROUTING_PATH = ROOT / "apps" / "control-plane" / "src" / "routing.rs"
CONTROL_PATH = ROOT / "apps" / "control-plane" / "src" / "lib.rs"
RUNBOOK_PATH = ROOT / "docs" / "operations" / "same-origin-routing.md"
EVIDENCE_PATH = ROOT / "docs" / "evidence" / "same-origin-routing-local.md"
WORKFLOW_PATH = ROOT / ".github" / "workflows" / "same-origin-routing.yml"
LIVE_RUNNER_PATH = ROOT / "scripts" / "ci" / "same-origin-live-conformance.py"
SMOKE_PATH = ROOT / "scripts" / "ci" / "smoke-canonical-domain.sh"
WRANGLER_PREP_PATH = ROOT / "scripts" / "ci" / "prepare-wrangler-config.py"

CANONICAL_HOST = "frame.engmanager.xyz"
STAGING_HOST = "frame-staging.engmanager.xyz"
ROUTE_PATTERN = f"{CANONICAL_HOST}/api*"
COMPATIBILITY_ROUTE_PATTERN = f"{CANONICAL_HOST}/media-server*"


class ContractError(RuntimeError):
    """A same-origin contract invariant drifted."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def read_json(path: pathlib.Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"{path}: expected a JSON object")
    return value


def literal_worker_route(url: str) -> str | None:
    for scheme in ("https://", "http://"):
        prefix = f"{scheme}{CANONICAL_HOST}"
        if url.startswith(prefix):
            suffix = url.removeprefix(prefix)
            if suffix.startswith("/api"):
                return "api"
            if suffix.startswith("/media-server"):
                return "media-server"
    return None


def validate_matrix(matrix: dict[str, Any]) -> dict[str, int]:
    require(matrix.get("schema") == "frame.same-origin-route-owner-matrix.v1", "matrix schema drifted")
    require(matrix.get("canonical_host") == CANONICAL_HOST, "matrix canonical host drifted")
    require(matrix.get("worker_route_pattern") == ROUTE_PATTERN, "matrix route pattern drifted")
    require(
        matrix.get("compatibility_worker_route_pattern") == COMPATIBILITY_ROUTE_PATTERN,
        "matrix compatibility route pattern drifted",
    )
    require(matrix.get("lookalike_policy") == "non_cacheable_404", "lookalike policy drifted")

    cases = matrix.get("cases")
    require(isinstance(cases, list) and len(cases) >= 25, "route matrix is not exhaustive")
    ids = [case.get("id") for case in cases if isinstance(case, dict)]
    require(len(ids) == len(cases) == len(set(ids)), "route matrix IDs must be unique strings")
    allowed_owners = {
        "render",
        "worker_api",
        "worker_compat",
        "worker_reject",
        "render_or_edge_reject",
    }
    owners: dict[str, int] = {owner: 0 for owner in allowed_owners}
    for case in cases:
        require(isinstance(case, dict), "every route case must be an object")
        owner = case.get("edge_owner")
        require(owner in allowed_owners, f"{case.get('id')}: unsupported edge owner")
        owners[owner] += 1
        url = case.get("url")
        raw_path = case.get("raw_path")
        require(isinstance(url, str) and isinstance(raw_path, str), f"{case.get('id')}: invalid target")
        require(url.startswith(f"https://{CANONICAL_HOST}/"), f"{case.get('id')}: host is not canonical")
        require("#" not in url and raw_path.startswith("/"), f"{case.get('id')}: invalid HTTP target")
        intercepted = literal_worker_route(url)
        if owner == "render":
            require(intercepted is None, f"{case.get('id')}: Render path matches a Worker route")
        elif owner == "worker_api":
            require(intercepted == "api", f"{case.get('id')}: API path misses broad route")
        elif owner == "worker_compat":
            require(
                intercepted == "media-server",
                f"{case.get('id')}: compatibility path misses narrow route",
            )
        elif owner == "worker_reject":
            require(intercepted is not None, f"{case.get('id')}: closed Worker path fell through")

    for required_id in {
        "api-discovery-query",
        "api-repeated-slash",
        "api-dot-segment",
        "api-parent-segment",
        "api-semicolon",
        "api-encoded-dot",
        "lookalike-apix",
        "lookalike-apiary",
        "lookalike-encoded-slash",
        "lookalike-double-encoded-slash",
        "lookalike-encoded-prefix",
        "lookalike-uppercase",
        "media-server-root",
        "media-server-root-query",
        "media-server-trailing-slash",
        "media-server-unpromoted-child",
        "media-server-lookalike",
        "render-media-server-uppercase",
    }:
        require(required_id in ids, f"route matrix is missing {required_id}")

    host_cases = matrix.get("host_cases")
    require(isinstance(host_cases, list) and len(host_cases) >= 8, "negative host matrix is incomplete")
    require(
        {case.get("expected") for case in host_cases if isinstance(case, dict)}
        == {"accepted", "insecure_scheme", "unexpected_host", "host_header_mismatch", "malformed_target"},
        "host matrix result set drifted",
    )

    transport = matrix.get("transport_cases")
    require(isinstance(transport, list), "transport matrix must be a list")
    transport_kinds = {case.get("kind") for case in transport if isinstance(case, dict)}
    require(
        transport_kinds
        == {"methods", "query", "request_body", "response_stream", "range", "redirect", "upgrade", "error"},
        "transport matrix coverage drifted",
    )
    return {"route_cases": len(cases), "host_cases": len(host_cases), "transport_cases": len(transport), **owners}


def validate_inventory(inventory: dict[str, Any]) -> None:
    require(inventory.get("schema") == "frame.same-origin-ownership-inventory.v1", "inventory schema drifted")
    environments = inventory.get("environments")
    require(isinstance(environments, list) and len(environments) == 2, "inventory must contain production and staging")
    by_name = {entry.get("name"): entry for entry in environments if isinstance(entry, dict)}
    require(set(by_name) == {"production", "staging"}, "environment inventory drifted")
    require(by_name["production"].get("hostname") == CANONICAL_HOST, "production hostname drifted")
    require(by_name["staging"].get("hostname") == STAGING_HOST, "staging hostname drifted")
    require(
        by_name["production"].get("status") == "repository_contract_complete_provider_evidence_pending",
        "production inventory overclaims protected provider evidence",
    )
    require(
        by_name["staging"].get("status") == "protected_provider_configuration_pending",
        "staging inventory must remain explicitly protected until provisioned",
    )
    for name, expected_host in (("production", CANONICAL_HOST), ("staging", STAGING_HOST)):
        entry = by_name[name]
        owners = entry.get("owners")
        require(isinstance(owners, dict) and len(owners) == 8, f"{name}: incomplete owner inventory")
        require(all(isinstance(value, str) and value for value in owners.values()), f"{name}: empty owner")
        dns = entry.get("dns")
        require(dns.get("type") == "CNAME" and dns.get("initial_proxy") is False, f"{name}: DNS staging drifted")
        require(dns.get("wildcard") is False, f"{name}: wildcard must remain forbidden")
        route = entry.get("worker_route")
        require(route.get("pattern") == f"{expected_host}/api*", f"{name}: Worker pattern drifted")
        require(
            route.get("compatibility_patterns") == [f"{expected_host}/media-server*"],
            f"{name}: compatibility Worker pattern drifted",
        )
        require(route.get("workers_dev") is False, f"{name}: workers.dev exposure drifted")
    protected = set(inventory.get("protected_evidence_required", []))
    normalization = inventory.get("url_normalization", {})
    require(normalization.get("status") == "protected_raw_and_normalized_trace_pending", "normalization evidence was overclaimed")
    require(normalization.get("edge_raw_trace_field") == "raw.http.request.uri.path", "raw edge trace field drifted")
    require(normalization.get("encoded_prefix_policy") == "never_enter_api_handler", "encoded-prefix policy drifted")
    require(normalization.get("global_zone_setting_change_for_frame_allowed") is False, "Frame must not alter global normalization")
    require(
        {
            "authoritative_dns_history",
            "edge_and_origin_certificate_chains",
            "raw_and_normalized_path_trace",
            "single_hop_request_id_trace",
            "worker_route_removal_rehearsal",
            "dns_only_rehearsal",
            "render_default_hostname_disable_rehearsal",
            "unrelated_host_non_regression",
        }
        <= protected,
        "protected evidence inventory is incomplete",
    )


def validate_declarations() -> None:
    wrangler = tomllib.loads(WRANGLER_PATH.read_text(encoding="utf-8"))
    require(wrangler.get("workers_dev") is False, "production workers.dev must be disabled")
    require(wrangler.get("name") == "frame-control-plane", "Worker script identity drifted")
    routes = wrangler.get("routes")
    require(isinstance(routes, list) and len(routes) == 2, "Wrangler must declare two production routes")
    require(
        routes
        == [
            {"pattern": ROUTE_PATTERN, "zone_name": "engmanager.xyz"},
            {"pattern": COMPATIBILITY_ROUTE_PATTERN, "zone_name": "engmanager.xyz"},
        ],
        "Wrangler routes drifted",
    )
    variables = wrangler.get("vars", {})
    require(variables.get("FRAME_DEPLOYMENT") == "production", "Worker deployment mode drifted")
    require(variables.get("FRAME_PUBLIC_HOST") == CANONICAL_HOST, "Worker host policy drifted")

    render = RENDER_PATH.read_text(encoding="utf-8")
    require(render.count(f"- {CANONICAL_HOST}") == 1, "Render must declare the canonical domain exactly once")
    require("- frame-staging.engmanager.xyz" not in render, "unprovisioned staging domain leaked into production Blueprint")
    require("renderSubdomainPolicy:" in render, "Render default-hostname rollout control is missing")

    zone = read_json(ZONE_PATH)
    require(zone.get("ownership", {}).get("frame_repository_is_read_only_consumer") is True, "Frame must not own shared zone state")
    require(zone.get("dns", {}).get("hostname") == CANONICAL_HOST, "zone DNS host drifted")
    require(zone.get("dns", {}).get("initial_proxy") is False, "DNS must begin DNS-only")
    require(zone.get("worker_route", {}).get("pattern") == ROUTE_PATTERN, "zone handoff route drifted")
    require(
        zone.get("worker_route", {}).get("compatibility_patterns")
        == [COMPATIBILITY_ROUTE_PATTERN],
        "zone handoff compatibility route drifted",
    )
    require(zone.get("worker_route", {}).get("lookalike_policy") == "non_cacheable_404", "zone lookalike policy drifted")


def validate_runtime_and_docs() -> None:
    routing = ROUTING_PATH.read_text(encoding="utf-8")
    control = CONTROL_PATH.read_text(encoding="utf-8")
    for marker in (
        'if path.starts_with("/api\\\\")',
        'path != "/api" && !path.starts_with("/api/")',
        "path.contains('%')",
        "path.contains(';')",
        'path.contains("//")',
        'matches!(segment, "." | "..")',
    ):
        require(marker in routing, f"raw routing guard missing: {marker}")
    for marker in (
        "parse_raw_request_target(&request.inner().url())",
        "let route = classify_raw_path(&target.path);",
        "validate_host(&target, host.as_deref(), &config.host_policy)",
        "Route::NotApi => failure_response(",
        'headers.set("cache-control", "no-store, max-age=0")?;',
        'headers.set("x-request-id", request_id)?;',
        '"strict-transport-security"',
        "normalize_cf_ray(",
    ):
        require(marker in control, f"control-plane same-origin marker missing: {marker}")
    require("WebSocketPair" not in control and "with_status(101)" not in control, "unexpected protocol upgrade surface")

    runbook = RUNBOOK_PATH.read_text(encoding="utf-8")
    evidence = EVIDENCE_PATH.read_text(encoding="utf-8")
    normalized_evidence = " ".join(evidence.lower().split())
    workflow = WORKFLOW_PATH.read_text(encoding="utf-8")
    live_runner = LIVE_RUNNER_PATH.read_text(encoding="utf-8")
    smoke = SMOKE_PATH.read_text(encoding="utf-8")
    wrangler_prep = WRANGLER_PREP_PATH.read_text(encoding="utf-8")
    for marker in (
        "DNS-only",
        "Full (strict)",
        "CAA",
        "default Render hostname",
        "raw.http.request.uri.path",
        "Worker Route rollback",
        "CNAME rollback",
        "unrelated-host",
        "media-server*",
    ):
        require(marker in runbook, f"same-origin runbook is missing {marker}")
    for marker in (
        "locally proved",
        "protected evidence still required",
        "provider state was not changed",
        "url normalization",
    ):
        require(marker in normalized_evidence, f"local evidence is missing {marker}")
    require("python3 -I scripts/ci/check-same-origin-routing.py" in workflow, "workflow omits static checker")
    require("same_origin_routing_v1" in workflow, "workflow omits Rust route matrix")
    require("same-origin-live-conformance.py --self-test" in workflow, "workflow omits live-runner self-test")
    require("bash -n scripts/ci/smoke-canonical-domain.sh" in workflow, "workflow omits smoke syntax validation")
    for marker in (
        'request media_server "/media-server"',
        'request media_server_query "/media-server?smoke=one&smoke=two"',
        "expected_media_server=",
        "assert_not_shared_cache media_server",
    ):
        require(marker in smoke, f"canonical-domain smoke is missing {marker}")
    require(
        f'pattern = "{COMPATIBILITY_ROUTE_PATTERN}"' in wrangler_prep,
        "artifact-only Wrangler preparation does not enforce the compatibility route",
    )
    for marker in (
        "ALLOWED_ORIGINS",
        '"https://frame.engmanager.xyz"',
        '"https://frame-staging.engmanager.xyz"',
        "--require-full",
        "request_id_spoof_rejected",
        "provider_state_changed",
    ):
        require(marker in live_runner, f"live conformance runner is missing {marker}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    arguments = parser.parse_args()
    try:
        matrix_counts = validate_matrix(read_json(MATRIX_PATH))
        validate_inventory(read_json(INVENTORY_PATH))
        validate_declarations()
        validate_runtime_and_docs()
    except (ContractError, OSError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"same-origin routing validation failed: {error}", file=sys.stderr)
        return 1

    report = {
        "schema": "frame.same-origin-local-conformance.v1",
        "result": "passed",
        "provider_state_changed": False,
        "protected_provider_evidence": "not_collected",
        "counts": matrix_counts,
    }
    if arguments.evidence is not None:
        arguments.evidence.parent.mkdir(parents=True, exist_ok=True)
        arguments.evidence.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        "Same-origin routing contract passed "
        f"({matrix_counts['route_cases']} routes, {matrix_counts['host_cases']} hosts, "
        f"{matrix_counts['transport_cases']} transport classes; provider state untouched)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
