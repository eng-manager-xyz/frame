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
BOUNDARY_PATH = ROOT / "fixtures/web-authenticated/v1/browser-direct-boundary.json"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"web authenticated parity: {message}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=pathlib.Path)
    args = parser.parse_args()

    encoded = MATRIX_PATH.read_bytes()
    matrix = json.loads(encoded)
    boundary_encoded = BOUNDARY_PATH.read_bytes()
    boundary = json.loads(boundary_encoded)
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
    browser_source = (ROOT / "apps/web/src/browser_authenticated.rs").read_text(
        encoding="utf-8"
    )
    hydration_source = (ROOT / "apps/web/src/hydration.rs").read_text(
        encoding="utf-8"
    )
    control_source = (ROOT / "apps/control-plane/src/browser_web_runtime.rs").read_text(
        encoding="utf-8"
    )
    worker_auth_source = (
        ROOT / "apps/control-plane/src/worker_auth_runtime.rs"
    ).read_text(encoding="utf-8")
    authenticated_runtime_source = (
        ROOT / "apps/control-plane/src/authenticated_web_runtime.rs"
    ).read_text(encoding="utf-8")
    control_lib_source = (ROOT / "apps/control-plane/src/lib.rs").read_text(
        encoding="utf-8"
    )
    routing_source = (ROOT / "apps/control-plane/src/routing.rs").read_text(
        encoding="utf-8"
    )
    action_migration = (
        ROOT / "apps/control-plane/migrations/0025_authenticated_web_actions.sql"
    ).read_text(encoding="utf-8")
    auth_handoff_migration = (
        ROOT / "apps/control-plane/migrations/0029_worker_auth_delivery_handoff.sql"
    ).read_text(encoding="utf-8")
    identity_source = (ROOT / "crates/application/src/identity.rs").read_text(
        encoding="utf-8"
    )
    workflow = (
        ROOT / ".github/workflows/leptos-authenticated-web.yml"
    ).read_text(encoding="utf-8")
    for command in (
        "check-web-authenticated-parity.py",
        "web-authenticated-action-sqlite-conformance.py",
        "worker-auth-sqlite-conformance.py",
        "web-authenticated-parity.py",
        "web-authenticated-browser.py",
        "web-hydration-smoke.py",
        "cargo clippy --locked -p frame-web --all-targets -- -D warnings",
        "cargo test --locked -p frame-web",
        "cargo test --locked -p frame-control-plane browser_web_runtime",
        "cargo test --locked -p frame-control-plane worker_auth_runtime",
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
        [route.get("path") for route in auth_routes]
        == ["/login", "/signup", "/recovery", "/verify"],
        "auth route set drifted",
    )
    for route in auth_routes:
        path = route["path"]
        require(
            re.search(rf'\.route\(\s*"{re.escape(path)}",\s*get\(', lib_source)
            is not None,
            f"auth route is absent: {path}",
        )
        endpoint = route.get("post_endpoint")
        require(
            endpoint
            in {
                "/api/v1/web/auth/login",
                "/api/v1/web/auth/signup",
                "/api/v1/web/auth/recovery",
                "/api/v1/web/auth/verify",
            },
            f"{path} Worker auth endpoint drifted",
        )
        require(
            f'action="{endpoint}"' in pages_source,
            f"{path} form does not post directly to the Worker",
        )
        require(
            re.search(rf'\.route\(\s*"{re.escape(path)}",\s*get\([^)]*\)\)', lib_source)
            is not None,
            f"{path} Render route admits a POST handler",
        )
        require(
            route.get("post_state")
            == "worker_adapter_local_provider_execution_protected",
            f"{path} auth evidence boundary drifted",
        )

    authentication = boundary.get("authentication", {})
    require(
        authentication.get("paths")
        == [
            "/api/v1/web/auth/login",
            "/api/v1/web/auth/signup",
            "/api/v1/web/auth/recovery",
            "/api/v1/web/auth/verify",
            "/api/v1/web/auth/logout",
        ],
        "Worker auth route set drifted",
    )
    require(
        authentication.get("secret_source") == "worker-csprng"
        and authentication.get("delivery_handoff")
        == "d1-ciphertext-only-deduplicated"
        and authentication.get("provider_execution") == "protected-not-executed",
        "Worker auth local/protected evidence boundary drifted",
    )
    for marker in (
        "getrandom::fill",
        "Aes256Gcm",
        "DELIVERY_PLAINTEXT_BYTES",
        "__Host-frame_auth_pending",
        "issue_identity_provisioning_verification",
        "issue_verification",
        "consume_verification",
        "issue_session",
        "auth_delivery_provider_handoffs_v1",
        "acknowledge_auth_delivery",
        "dispatch_delivery_batch",
        "AUTH_DELIVERY_DISPATCH_BUDGET_PER_TICK",
        "AUTH_DELIVERY_ADMISSION_PER_MINUTE",
        "BROWSER_AUTH_DELIVERY_ACTION_CLASSES",
    ):
        require(marker in worker_auth_source, f"Worker auth adapter omits {marker}")
    for marker in (
        "browser_auth_page_failure_response",
        "browser_auth_verify_failure_response",
        "/login?auth_error=invalid",
        "/signup?auth_error=failed",
        "/recovery?auth_error=failed",
        "/verify?auth_error=invalid",
    ):
        require(marker in control_lib_source, f"browser auth failure UI omits {marker}")
    for marker in (
        "payload_hex",
        "payload_sha256",
        "insert_guard",
        "payload_immutable",
        "protected deployment gate",
    ):
        require(marker in auth_handoff_migration, f"auth handoff migration omits {marker}")

    require("DefaultBodyLimit::max(16 * 1024)" in lib_source, "form body limit is absent")
    require(
        "pub trait AuthenticatedApiPort" in authenticated_source,
        "future typed API boundary is absent",
    )
    require(
        "pub struct BrowserAuthenticatedClient" in browser_source
        and "impl<T> BrowserAuthenticatedClient<T>" in browser_source,
        "production typed browser client is absent",
    )
    for marker in (
        "/api/v1/web/workspace/",
        "/api/v1/web/actions/",
        "/api/v1/web/auth/logout",
        "RequestCredentials::SameOrigin",
        "csrf_protected",
        "decode_workspace",
        "decode_receipt",
        "self.cache.borrow_mut().clear",
    ):
        require(marker in browser_source, f"typed browser client omits {marker}")
    require(
        '"authorization"' not in browser_source
        and '"x-frame-tenant-id"' not in browser_source,
        "browser client can supply bearer or tenant authority",
    )
    require(
        "AuthenticatedWorkspacePanel" in hydration_source
        and "BrowserMutationInput" in hydration_source
        and "random_operation_id" in hydration_source
        and "uncertain_mutation" in hydration_source
        and "Retry exact request" in hydration_source
        and "client.logout()" in hydration_source
        and "Sign out" in hydration_source
        and "Some(input)" in hydration_source
        and "action.permitted_for(current.role)" in hydration_source
        and "data-frame-browser-loader" in pages_source,
        "authenticated browser island or DTO-authorized action gate is not wired",
    )
    require(
        "AuthenticatedSsr" not in lib_source
        and '"x-frame-tenant-id"' not in lib_source
        and "AuthenticatedSsr" not in authenticated_source,
        "Render authenticated SSR or credential forwarding is activated contrary to ADR 0004",
    )
    require(
        boundary.get("schema") == "frame.web-browser-direct-boundary.v1",
        "browser-direct boundary schema drifted",
    )
    require(
        boundary.get("render_ssr") == "data-free-no-credential-forwarding",
        "browser-direct fixture permits credential-bearing Render SSR",
    )
    require(
        boundary.get("forbidden_browser_headers")
        == ["authorization", "x-frame-tenant-id"],
        "browser authority header deny-list drifted",
    )
    require(
        boundary.get("load", {}).get("tenant_source")
        == "users.active_organization_id+organization_preference_revision; dashboard recovery exposes only current active memberships"
        and boundary.get("load", {}).get("selection_contract")
        == "opaque-context+revision+authorized-frame-uuid-choices"
        and boundary.get("load", {}).get("cache")
        == "invalidate-all-workspace-envelopes-after-mutation",
        "browser organization selection or workspace cache contract drifted",
    )
    require(
        boundary.get("load", {}).get("authority_revalidation")
        == "exact-selection-membership-before-and-after-dto"
        and boundary.get("mutation", {}).get("uncertain_outcome")
        == "retain-exact-request-and-key+invalidate-all+force-refresh",
        "browser load or uncertain-mutation recovery contract drifted",
    )
    expected_actions = [
        route["mutation"] for route in routes if route.get("mutation") is not None
    ]
    require(
        boundary.get("actions") == expected_actions,
        "browser action inventory does not match the route matrix",
    )
    expected_applied_actions = [
        "organization.active-selection.update.v1",
        "organization.spaces.create.v1",
        "organization.folders.create.v1",
        "business.imports.start.v1",
        "identity.account.update.v1",
        "organization.settings.update.v1",
    ]
    expected_pending_actions = [
        "organization.onboarding.complete.v1",
        "organization.members.manage.v1",
        "business.storage.configure.v1",
        "business.developer.credentials.manage.v1",
        "business.billing.manage.v1",
        "business.admin.execute.v1",
    ]
    require(
        boundary.get("applied_actions") == expected_applied_actions
        and boundary.get("pending_protected_execution_actions")
        == expected_pending_actions
        and set(expected_applied_actions).isdisjoint(expected_pending_actions)
        and set(expected_applied_actions + expected_pending_actions)
        == set(expected_actions),
        "browser action applied/pending execution disposition drifted",
    )
    require(
        boundary.get("mutation", {}).get("atomic_authority_assertions")
        == [
            "action-specific organization-or-selection revision",
            "current active-organization selection",
            "current source-or-target membership",
            "one_use_mutation_grant",
            "replay_current_authority",
        ],
        "browser action atomic authority assertions drifted",
    )
    require(
        boundary.get("mutation", {}).get("active_organization_selector")
        == {
            "input": "authorized-frame-uuid-choice",
            "recovery": "dashboard-membership-only-selector-when-current-selection-invalid",
            "grant_disposition": "atomic-with-selection-write-and-idempotency-receipt",
            "idempotency_scope": "user+action+key-across-target-organizations",
            "replay_authority": "receipt-target-selection-revision+target-membership+one-use-grant",
            "rate_limit_bucket": "organization_library.v1",
            "legacy_cap_server_action_status": "local-contract-ingress-pending",
        },
        "active-organization selector boundary drifted",
    )
    selector_replay = re.search(
        r"async fn consume_selection_grant.*?\n}\n\n#\[allow",
        control_source,
        re.DOTALL,
    )
    require(
        selector_replay is not None
        and "active_organization_id=?3" in selector_replay.group(0)
        and "organization_preference_revision=?4" in selector_replay.group(0)
        and "target_membership_assertion_statement" in selector_replay.group(0)
        and "grant_delete_statement" in selector_replay.group(0)
        and "existing_selection_operation" in control_source
        and "existing.organization_id != target_organization_id" in control_source
        and "receipt.revision" in control_source,
        "active-organization replay is not bound to global key scope and current authority",
    )
    require(
        boundary.get("mutation", {}).get("applied_http_status") == 200
        and boundary.get("mutation", {}).get(
            "pending_protected_execution_http_status"
        )
        == 202,
        "browser action applied/pending HTTP status contract drifted",
    )
    for action in expected_actions:
        require(action in browser_source, f"web client omits action {action}")
        require(action in control_source, f"Worker boundary omits action {action}")
    for marker in (
        "D1AuthStateRepository::new",
        "AuthService::new",
        "__Host-frame_session",
        "__Host-frame_csrf",
        'get("origin")',
        'get("sec-fetch-site")',
        'get("x-frame-csrf")',
        'get("authorization")',
        'get("x-frame-tenant-id")',
        "active_membership",
        "active_organization_id",
        "organization_preference_revision",
        "selection_context",
        "selection_authority",
        "membership_authority",
        "m.revision=?5",
        "supported_browser_role",
        "PendingProtectedExecution",
        "auth_session_mutation_grants_v2",
        "authenticated_web_action_operations_v1",
        "database.batch(statements)",
        "organization_choices",
        "consume_selection_grant",
        "CompatibilityRateLimitBucketV1::OrganizationLibrary",
    ):
        require(marker in control_source, f"Worker browser boundary omits {marker}")
    for marker in (
        "authenticated-organization-choice",
        "current.organizations",
        "BrowserAction::SetActiveOrganization",
    ):
        require(marker in hydration_source, f"active-organization selector omits {marker}")
    for marker in (
        "u.active_organization_id=?1",
        "u.organization_preference_revision=?3",
        "m.role=?4",
        "m.revision=?5",
        "m.role IN ('owner','admin','member')",
    ):
        require(
            marker in authenticated_runtime_source,
            f"authenticated load boundary omits {marker}",
        )
    role_helper = re.search(
        r"fn role_permits_surface.*?fn valid_surface",
        authenticated_runtime_source,
        re.DOTALL,
    )
    require(
        role_helper is not None and '"viewer"' not in role_helper.group(0),
        "authenticated load role helper still admits viewer",
    )
    require(
        "current_membership = active_membership" in control_source
        and "load_authority_is_current" in control_source
        and "current == Some(expected)" in control_source
        and "workspace_role == expected.role" in control_source,
        "workspace DTO does not revalidate final active selection/membership authority",
    )
    mutate_client = re.search(
        r"pub async fn mutate\(.*?\n    }\n\n    #\[cfg\(test\)\]",
        browser_source,
        re.DOTALL,
    )
    require(
        mutate_client is not None
        and mutate_client.group(0).find("self.cache.borrow_mut().clear")
        < mutate_client.group(0).find(".transport"),
        "mutation cache eviction does not precede the uncertain transport boundary",
    )
    require(
        "mutation_grant_id" in identity_source
        and "session_id" in identity_source
        and "user_id" in identity_source,
        "one-use browser mutation proof cannot bind the D1 write",
    )
    require(
        "AuthenticatedWebAction" in routing_source
        and '"web", "actions", action' in routing_source
        and "browser_web_runtime::load" in control_lib_source
        and "browser_web_runtime::mutate" in control_lib_source,
        "Worker browser routes are not dispatched",
    )
    workspace_arm = re.search(
        r"Route::AuthenticatedWebWorkspace.*?Route::AuthenticatedWebAction",
        control_lib_source,
        re.DOTALL,
    )
    require(workspace_arm is not None, "Worker workspace route arm is absent")
    require(
        "authenticated_command_preflight" not in workspace_arm.group(0)
        and "authorized_tenant" not in workspace_arm.group(0),
        "browser workspace route still requires bearer or browser tenant authority",
    )
    for marker in (
        "authenticated_web_action_operations_v1",
        "authenticated_web_action_effects_v1",
        "authenticated_web_action_assertions_v1",
        "CHECK (expected_count = actual_count)",
        "membership_authority",
        "selection_authority",
        "pending_protected_execution",
        "product_effect",
        "action_effect",
        "organization_update",
        "operation_complete",
        "grant_consumed",
    ):
        require(marker in action_migration, f"browser action migration omits {marker}")
    require(
        "PendingProtectedExecution" in browser_source
        and '"pending_protected_execution"' in browser_source
        and "No provider change is claimed yet" in hydration_source,
        "pending protected action receipts can be presented as product success",
    )
    require("data-form-contract=\"revision-fenced-v1\"" in pages_source, "form state contract marker is absent")
    require("data-unsaved-guard=\"required\"" in pages_source, "unsaved-change marker is absent")
    require("noindex,nofollow" in pages_source, "private metadata policy is absent")
    require(
        len(matrix.get("protected_evidence_pending", [])) >= 5,
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
        "browser_boundary_sha256": hashlib.sha256(boundary_encoded).hexdigest(),
        "route_count": len(routes),
        "role_route_cases": len(routes) * len(matrix["roles"]),
        "auth_route_count": len(auth_routes),
        "legacy_alias_count": len(all_aliases),
        "states": len(matrix["states"]),
        "themes": len(matrix["themes"]),
        "breakpoints": len(matrix["breakpoints"]),
        "protected_evidence_pending": len(matrix["protected_evidence_pending"]),
        "typed_action_count": len(expected_actions),
        "browser_direct_boundary": "complete",
        "status": "local_browser_journeys_complete_protected_evidence_pending",
    }
    rendered = json.dumps(evidence, indent=2, sort_keys=True) + "\n"
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
