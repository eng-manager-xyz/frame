#!/usr/bin/env python3
"""Validate the closed Issue 31 authenticated-web contract without providers."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re


ROOT = pathlib.Path(__file__).resolve().parents[2]
MATRIX_PATH = ROOT / "fixtures/web-authenticated/v1/route-matrix.json"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"web authenticated parity: {message}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()

    encoded = MATRIX_PATH.read_bytes()
    matrix = json.loads(encoded)
    require(
        matrix.get("schema") == "frame.web-authenticated-route-matrix.v1",
        "unexpected matrix schema",
    )
    require(
        matrix.get("reference")
        == "CapSoftware/Cap@6ba69561ac86b8efdb17616d6727f9638015546b",
        "reference revision is not pinned",
    )
    require(matrix.get("roles") == ["owner", "admin", "member"], "role set drifted")
    require(
        matrix.get("states")
        == ["loading", "ready", "empty", "denied", "failed", "unauthenticated"],
        "state set drifted",
    )
    require(matrix.get("themes") == ["system", "dark", "light"], "theme set drifted")
    require(
        [item.get("name") for item in matrix.get("breakpoints", [])]
        == ["mobile", "tablet", "desktop"],
        "breakpoint set drifted",
    )
    budgets = matrix.get("budgets", {})
    require(budgets.get("server_render_p95_ms") == 750, "SSR charter budget drifted")
    require(budgets.get("api_p95_ms") == 500, "API charter budget drifted")
    require(0 < budgets.get("html_bytes", 0) <= 131_072, "HTML budget is invalid")
    require(
        budgets.get("hydration_wasm_bytes") == 2_000_000
        and budgets.get("hydration_javascript_bytes") == 500_000,
        "hydration loader budgets drifted",
    )

    expected = {
        "dashboard": ("Dashboard", "/dashboard", "/dashboard", {"owner", "admin", "member"}),
        "library": ("Library", "/library", "/library", {"owner", "admin", "member"}),
        "spaces": ("Spaces", "/spaces", "/spaces", {"owner", "admin", "member"}),
        "space": ("Space", "/spaces/{resource_id}", "/spaces/fixture-space", {"owner", "admin", "member"}),
        "folders": ("Folders", "/folders", "/folders", {"owner", "admin", "member"}),
        "folder": ("Folder", "/folders/{resource_id}", "/folders/fixture-folder", {"owner", "admin", "member"}),
        "onboarding": ("Onboarding", "/onboarding", "/onboarding", {"owner", "admin", "member"}),
        "imports": ("Imports", "/imports", "/imports", {"owner", "admin"}),
        "settings": ("Settings", "/settings", "/settings", {"owner", "admin", "member"}),
        "account_settings": ("AccountSettings", "/settings/account", "/settings/account", {"owner", "admin", "member"}),
        "organization_settings": ("OrganizationSettings", "/settings/organization", "/settings/organization", {"owner", "admin"}),
        "member_settings": ("MemberSettings", "/settings/members", "/settings/members", {"owner", "admin"}),
        "storage_settings": ("StorageSettings", "/settings/storage", "/settings/storage", {"owner", "admin"}),
        "developer": ("Developer", "/developer", "/developer", {"owner", "admin"}),
        "billing": ("Billing", "/billing", "/billing", {"owner"}),
        "analytics": ("Analytics", "/analytics", "/analytics", {"owner", "admin"}),
        "admin": ("Admin", "/admin", "/admin", {"owner", "admin"}),
    }
    routes = matrix.get("routes", [])
    require(len(routes) == len(expected), "route count drifted")
    by_name = {route.get("name"): route for route in routes}
    require(len(by_name) == len(routes), "route names are not unique")
    require(set(by_name) == set(expected), "route name set drifted")

    lib_source = (ROOT / "apps/web/src/lib.rs").read_text(encoding="utf-8")
    authenticated_source = (ROOT / "apps/web/src/authenticated.rs").read_text(
        encoding="utf-8"
    )
    product_source = (ROOT / "apps/web/src/product.rs").read_text(encoding="utf-8")
    pages_source = (ROOT / "apps/web/src/pages.rs").read_text(encoding="utf-8")
    workflow = (
        ROOT / ".github/workflows/leptos-authenticated-web.yml"
    ).read_text(encoding="utf-8")
    for command in (
        "check-web-authenticated-parity.py",
        "web-authenticated-parity.py",
        "web-authenticated-browser.py",
        "web-hydration-smoke.py",
        "cargo clippy --locked -p frame-web --all-targets -- -D warnings",
        "cargo test --locked -p frame-web",
    ):
        require(command in workflow, f"authenticated-web workflow omits {command}")
    all_aliases: set[str] = set()
    for name, (variant, router_path, fixture_path, roles) in expected.items():
        row = by_name[name]
        expected_pattern = router_path.replace("{resource_id}", "{space_id}" if name == "space" else "{folder_id}")
        require(row.get("pattern") == expected_pattern, f"{name} pattern drifted")
        require(row.get("fixture_path") == fixture_path, f"{name} fixture path drifted")
        require(set(row.get("allowed_roles", [])) == roles, f"{name} roles drifted")
        require(row.get("component") and row["component"] in product_source, f"{name} component is not bound to Rust")
        require(row.get("journey") and row["journey"] in product_source, f"{name} journey is not bound to Rust")
        require(re.search(rf"\b{re.escape(variant)}\b", product_source), f"{name} enum variant is absent")
        require(
            re.search(rf'\.route\(\s*"{re.escape(router_path)}"', lib_source) is not None,
            f"{name} Axum route is absent",
        )
        require(row.get("api_operation", "").endswith(".v1"), f"{name} API operation is unversioned")
        require(row.get("rollout_flag", "").startswith("web."), f"{name} rollout flag is absent")
        for alias in row.get("legacy_aliases", []):
            require(alias not in all_aliases, f"duplicate legacy alias {alias}")
            all_aliases.add(alias)
            source_alias = alias.replace("fixture-space", "{resource_id}").replace(
                "fixture-folder", "{resource_id}"
            )
            require(
                re.search(rf'\.route\(\s*"{re.escape(source_alias)}"', lib_source)
                is not None,
                f"legacy alias is not routed: {alias}",
            )

    auth_routes = matrix.get("auth_routes", [])
    require(
        [route.get("path") for route in auth_routes] == ["/login", "/signup", "/verify"],
        "auth route set drifted",
    )
    for route in auth_routes:
        path = route["path"]
        require(
            re.search(rf'\.route\(\s*"{re.escape(path)}",\s*get\(', lib_source)
            is not None,
            f"auth route is absent: {path}",
        )
        require(route.get("post_state") == "adapter_pending_fail_closed", f"{path} overclaims auth authority")

    require("DefaultBodyLimit::max(16 * 1024)" in lib_source, "form body limit is absent")
    require(
        "pub trait AuthenticatedApiPort" in authenticated_source,
        "future typed API boundary is absent",
    )
    require(
        "AuthenticatedSsr" not in lib_source
        and '"x-frame-tenant-id"' not in lib_source
        and "AuthenticatedSsr" not in authenticated_source,
        "Render authenticated SSR or credential forwarding is activated contrary to ADR 0004",
    )
    require("data-form-contract=\"revision-fenced-v1\"" in pages_source, "form state contract marker is absent")
    require("data-unsaved-guard=\"required\"" in pages_source, "unsaved-change marker is absent")
    require("noindex,nofollow" in pages_source, "private metadata policy is absent")
    require(
        len(matrix.get("protected_evidence_pending", [])) >= 7,
        "protected evidence is not explicitly pending",
    )
    for document in (
        ROOT / "docs/architecture/leptos-authenticated-web-v1.md",
        ROOT / "docs/evidence/leptos-authenticated-web-local.md",
        ROOT / "docs/operations/leptos-route-cutover.md",
    ):
        require(document.is_file(), f"required documentation is absent: {document.relative_to(ROOT)}")

    evidence = {
        "schema": "frame.web-authenticated-contract-check.v1",
        "matrix_sha256": hashlib.sha256(encoded).hexdigest(),
        "route_count": len(routes),
        "role_route_cases": len(routes) * len(matrix["roles"]),
        "auth_route_count": len(auth_routes),
        "legacy_alias_count": len(all_aliases),
        "states": len(matrix["states"]),
        "themes": len(matrix["themes"]),
        "breakpoints": len(matrix["breakpoints"]),
        "protected_evidence_pending": len(matrix["protected_evidence_pending"]),
        "status": "local_contract_complete_production_adapters_pending",
    }
    rendered = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
