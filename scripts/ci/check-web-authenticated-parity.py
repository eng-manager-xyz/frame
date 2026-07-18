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
    mutation_grant_assert_sql = (
        ROOT / "apps/control-plane/queries/auth/browser_mutation_grant_assert.sql"
    ).read_text(encoding="utf-8")
    mutation_grant_delete_sql = (
        ROOT
        / "apps/control-plane/queries/auth/browser_mutation_grant_delete_by_proof.sql"
    ).read_text(encoding="utf-8")
    mutation_grant_presence_sql = (
        ROOT / "apps/control-plane/queries/auth/browser_mutation_grant_presence.sql"
    ).read_text(encoding="utf-8")
    mutation_change_assert_sql = (
        ROOT / "apps/control-plane/queries/auth/browser_mutation_change_assert.sql"
    ).read_text(encoding="utf-8")
    worker_auth_source = (
        ROOT / "apps/control-plane/src/worker_auth_runtime.rs"
    ).read_text(encoding="utf-8")
    authenticated_runtime_source = (
        ROOT / "apps/control-plane/src/authenticated_web_runtime.rs"
    ).read_text(encoding="utf-8")
    control_lib_source = (ROOT / "apps/control-plane/src/lib.rs").read_text(
        encoding="utf-8"
    )
    notification_application_source = (
        ROOT / "crates/application/src/legacy_notification_actions.rs"
    ).read_text(encoding="utf-8")
    notification_runtime_source = (
        ROOT / "apps/control-plane/src/legacy_notification_actions_runtime.rs"
    ).read_text(encoding="utf-8")
    notification_ingress_source = (
        ROOT / "apps/control-plane/src/legacy_notification_web_runtime.rs"
    ).read_text(encoding="utf-8")
    notification_migration = (
        ROOT / "apps/control-plane/migrations/0038_legacy_notification_actions_expand.sql"
    ).read_text(encoding="utf-8")
    developer_application_source = (
        ROOT / "crates/application/src/legacy_developer_actions.rs"
    ).read_text(encoding="utf-8")
    developer_runtime_source = (
        ROOT / "apps/control-plane/src/legacy_developer_actions_runtime.rs"
    ).read_text(encoding="utf-8")
    developer_ingress_source = (
        ROOT / "apps/control-plane/src/legacy_developer_web_runtime.rs"
    ).read_text(encoding="utf-8")
    developer_migration = (
        ROOT / "apps/control-plane/migrations/0039_legacy_developer_actions_expand.sql"
    ).read_text(encoding="utf-8")
    membership_application_source = (
        ROOT / "crates/application/src/legacy_membership_actions.rs"
    ).read_text(encoding="utf-8")
    membership_runtime_source = (
        ROOT / "apps/control-plane/src/legacy_membership_actions_runtime.rs"
    ).read_text(encoding="utf-8")
    membership_ingress_source = (
        ROOT / "apps/control-plane/src/legacy_membership_web_runtime.rs"
    ).read_text(encoding="utf-8")
    membership_migration = (
        ROOT / "apps/control-plane/migrations/0040_legacy_membership_actions_expand.sql"
    ).read_text(encoding="utf-8")
    user_account_application_source = (
        ROOT / "crates/application/src/legacy_user_account.rs"
    ).read_text(encoding="utf-8")
    user_account_runtime_source = (
        ROOT / "apps/control-plane/src/legacy_user_account_runtime.rs"
    ).read_text(encoding="utf-8")
    user_account_ingress_source = (
        ROOT / "apps/control-plane/src/legacy_user_account_web_runtime.rs"
    ).read_text(encoding="utf-8")
    user_account_migration = (
        ROOT / "apps/control-plane/migrations/0042_legacy_user_account_expand.sql"
    ).read_text(encoding="utf-8")
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
        "legacy-folder-assignment-sqlite-conformance.py",
        "legacy-library-placement-sqlite-conformance.py",
        "legacy-notification-actions-sqlite-conformance.py",
        "legacy-developer-actions-sqlite-conformance.py",
        "legacy-membership-actions-sqlite-conformance.py",
        "legacy-user-account-sqlite-conformance.py",
        "legacy-video-properties-sqlite-conformance.py",
        "web-authenticated-parity.py",
        "web-authenticated-browser.py",
        "web-hydration-smoke.py",
        "cargo clippy --locked -p frame-web --all-targets -- -D warnings",
        "cargo test --locked -p frame-web",
        "cargo test --locked -p frame-control-plane browser_web_runtime",
        "cargo test --locked -p frame-control-plane legacy_web_action_runtime",
        "cargo test --locked -p frame-control-plane worker_auth_runtime",
        "cargo test --locked -p frame-application --lib legacy_folder_assignment",
        "cargo test --locked -p frame-control-plane --lib legacy_folder",
        "cargo test --locked -p frame-application --lib legacy_library_placement",
        "cargo test --locked -p frame-control-plane --lib legacy_library",
        "cargo test --locked -p frame-application --lib legacy_notification_actions",
        "cargo test --locked -p frame-control-plane --lib legacy_notification",
        "cargo test --locked -p frame-application --lib legacy_developer_actions",
        "cargo test --locked -p frame-control-plane --lib legacy_developer",
        "cargo test --locked -p frame-application --lib legacy_membership_actions",
        "cargo test --locked -p frame-control-plane --lib legacy_membership",
        "cargo test --locked -p frame-application --lib legacy_user_account",
        "cargo test --locked -p frame-control-plane --lib legacy_user_account",
        "cargo test --locked -p frame-application --lib legacy_video_properties",
        "cargo test --locked -p frame-control-plane --lib legacy_video_properties",
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
        "WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1",
        "LEGACY_MARK_NOTIFICATIONS_READ_ACTION_ID",
        "LEGACY_UPDATE_NOTIFICATION_PREFERENCES_ACTION_ID",
        "mark_notifications_read",
        "update_notification_preferences",
        "WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1",
        "LEGACY_CREATE_DEVELOPER_APP_ACTION_ID",
        "LEGACY_UPDATE_DEVELOPER_APP_ACTION_ID",
        "LEGACY_DELETE_DEVELOPER_APP_ACTION_ID",
        "LEGACY_ADD_DEVELOPER_DOMAIN_ACTION_ID",
        "LEGACY_REMOVE_DEVELOPER_DOMAIN_ACTION_ID",
        "LEGACY_REGENERATE_DEVELOPER_KEYS_ACTION_ID",
        "LEGACY_DELETE_DEVELOPER_VIDEO_ACTION_ID",
        "LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_ACTION_ID",
        "create_developer_app",
        "update_developer_app",
        "regenerate_developer_keys",
        "BrowserDeveloperKeyPairV1([redacted])",
        "WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1",
        "LEGACY_REMOVE_ORGANIZATION_INVITE_ACTION_ID",
        "LEGACY_ADD_SPACE_MEMBER_ACTION_ID",
        "LEGACY_SET_SPACE_MEMBERS_ACTION_ID",
        "LEGACY_ADD_SPACE_MEMBERS_ACTION_ID",
        "LEGACY_BATCH_REMOVE_SPACE_MEMBERS_ACTION_ID",
        "LEGACY_REMOVE_SPACE_MEMBER_ACTION_ID",
        "remove_organization_invite",
        "add_space_member",
        "set_space_members",
        "add_space_members",
        "batch_remove_space_members",
        "remove_space_member",
        '"invalid_compatibility_action"',
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
        "authenticated_web_action_operations_v1",
        "database.batch(statements)",
        "organization_choices",
        "consume_selection_grant",
        "CompatibilityRateLimitBucketV1::OrganizationLibrary",
    ):
        require(marker in control_source, f"Worker browser boundary omits {marker}")
    for marker in (
        "BROWSER_MUTATION_GRANT_ASSERT_SQL",
        "browser_mutation_grant_assert.sql",
        "BROWSER_MUTATION_GRANT_DELETE_BY_PROOF_SQL",
        "browser_mutation_grant_delete_by_proof.sql",
        "BROWSER_MUTATION_GRANT_PRESENCE_SQL",
        "browser_mutation_grant_presence.sql",
        "consume_session_grant_or_confirm_absent",
        "BROWSER_MUTATION_CHANGE_ASSERT_SQL",
        "browser_mutation_change_assert.sql",
        "grant_assertion_statement",
        "grant_delete_statement",
        "change_assertion_statement",
    ):
        require(marker in control_source, f"Worker browser grant boundary omits {marker}")
    for marker in (
        "FROM auth_session_mutation_grants_v2",
        "id = ?1",
        "session_id = ?2",
        "user_id = ?3",
        "LIMIT 1",
    ):
        require(
            marker in mutation_grant_presence_sql,
            f"browser mutation grant presence SQL omits {marker}",
        )
    for marker in (
        "auth_session_mutation_grants_v2",
        "auth_sessions_v2",
        "auth_identities_v2",
        "g.session_id = ?3",
        "g.user_id = ?4",
        "s.state = 'active'",
        "s.generation = g.generation",
        "s.token_digest = g.token_digest",
        "s.session_version = i.session_version",
        "s.idle_expires_at_ms > ?5",
        "s.absolute_expires_at_ms > ?5",
    ):
        require(
            marker in mutation_grant_assert_sql,
            f"browser mutation grant assertion SQL omits {marker}",
        )
    for marker in (
        "DELETE FROM auth_session_mutation_grants_v2",
        "id = ?1",
        "session_id = ?2",
        "user_id = ?3",
    ):
        require(
            marker in mutation_grant_delete_sql,
            f"browser mutation grant deletion SQL omits {marker}",
        )
    for marker in (
        "authenticated_web_action_assertions_v1",
        "VALUES (?1, ?2, 1, changes())",
    ):
        require(
            marker in mutation_change_assert_sql,
            f"browser mutation change assertion SQL omits {marker}",
        )
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
    for marker in (
        "LEGACY_MARK_NOTIFICATIONS_READ_OPERATION_ID",
        "LEGACY_UPDATE_NOTIFICATION_PREFERENCES_OPERATION_ID",
        "LegacyNotificationAtomicPortV1",
        "LEGACY_NOTIFICATION_ACTION_PROTECTED_GATES",
    ):
        require(marker in notification_application_source, f"notification app omits {marker}")
    for marker in (
        "D1LegacyNotificationAtomicPortV1",
        "LegacyNotificationAtomicOutcomeV1::Applied",
        "LegacyNotificationAtomicOutcomeV1::Replay",
        "browser_grant_delete_returning.sql",
        "preferences_postcondition_assert.sql",
    ):
        require(marker in notification_runtime_source, f"notification D1 runtime omits {marker}")
    for marker in (
        "WEB_NOTIFICATION_ACTION_REQUEST_SCHEMA_V1",
        "OptionalJsonFieldV1::Missing",
        "OptionalJsonFieldV1::Present(serde_json::Value::Null)",
        'request.headers().get("idempotency-key")',
        "trusted_active_organization_id",
        "consume_session_grant_or_confirm_absent",
    ):
        require(marker in notification_ingress_source, f"notification ingress omits {marker}")
    for marker in (
        "legacy_notification_action_operations_v1",
        "legacy_notification_action_receipts_v1",
        "legacy_notification_action_proof_consumptions_v1",
        "frame_legacy_notification_operation_immutable_v1",
    ):
        require(marker in notification_migration, f"notification migration omits {marker}")
    for marker in (
        "legacy_notification_web_runtime::is_action",
        "legacy_notification_action_response",
        "Response::empty()?.with_status(204)",
    ):
        require(marker in control_lib_source, f"notification route omits {marker}")
    for marker in (
        "LEGACY_CREATE_DEVELOPER_APP_OPERATION_ID",
        "LEGACY_UPDATE_DEVELOPER_APP_OPERATION_ID",
        "LEGACY_DELETE_DEVELOPER_APP_OPERATION_ID",
        "LEGACY_ADD_DEVELOPER_DOMAIN_OPERATION_ID",
        "LEGACY_REMOVE_DEVELOPER_DOMAIN_OPERATION_ID",
        "LEGACY_REGENERATE_DEVELOPER_KEYS_OPERATION_ID",
        "LEGACY_DELETE_DEVELOPER_VIDEO_OPERATION_ID",
        "LEGACY_UPDATE_DEVELOPER_AUTO_TOP_UP_OPERATION_ID",
        "LegacyDeveloperAtomicPortV1",
        "LegacyDeveloperSecretAuthorityV1",
        "LEGACY_DEVELOPER_PROTECTED_GATES",
    ):
        require(marker in developer_application_source, f"developer app omits {marker}")
    for marker in (
        "D1LegacyDeveloperAtomicPortV1",
        "LocalLegacyDeveloperSecretAuthorityV1",
        "LegacyDeveloperAtomicOutcomeV1::Applied",
        "LegacyDeveloperAtomicOutcomeV1::Replay",
        "browser_grant_delete_returning.sql",
        "Zeroizing::new(Vec::with_capacity",
    ):
        require(marker in developer_runtime_source, f"developer D1 runtime omits {marker}")
    for marker in (
        "WEB_DEVELOPER_ACTION_REQUEST_SCHEMA_V1",
        "OptionalJsonFieldV1::Missing",
        "OptionalJsonFieldV1::Present(serde_json::Value::Null)",
        'request.headers().get("idempotency-key")',
        "LegacyAuthenticatedContextV1::principal_only",
        "D1LegacyDeveloperAtomicPortV1::new",
        "LocalLegacyDeveloperSecretAuthorityV1::from_hex",
        "consume_session_grant_or_confirm_absent",
    ):
        require(marker in developer_ingress_source, f"developer ingress omits {marker}")
    for marker in (
        "legacy_developer_action_operations_v1",
        "legacy_developer_action_receipts_v1",
        "legacy_developer_action_proof_consumptions_v1",
        "frame_legacy_developer_operation_immutable_v1",
    ):
        require(marker in developer_migration, f"developer migration omits {marker}")
    for marker in (
        "legacy_developer_web_runtime::is_action",
        "legacy_developer_action_response",
        "effect.app_created()",
        "effect.regenerated_keys()",
        "LegacyDeveloperAppCreatedResponseV1",
        "LegacyDeveloperKeysResponseV1",
    ):
        require(marker in control_lib_source, f"developer route omits {marker}")
    for marker in (
        "LEGACY_REMOVE_ORGANIZATION_INVITE_OPERATION_ID",
        "LEGACY_ADD_SPACE_MEMBER_OPERATION_ID",
        "LEGACY_SET_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_ADD_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_BATCH_REMOVE_SPACE_MEMBERS_OPERATION_ID",
        "LEGACY_REMOVE_SPACE_MEMBER_OPERATION_ID",
        "LegacyMembershipAtomicPortV1",
        "LEGACY_MEMBERSHIP_PROTECTED_GATES",
        "LEGACY_MEMBERSHIP_NO_PROTECTED_GATES",
    ):
        require(marker in membership_application_source, f"membership app omits {marker}")
    for marker in (
        "D1LegacyMembershipAtomicPortV1",
        "LegacyMembershipAtomicOutcomeV1::Applied",
        "LegacyMembershipAtomicOutcomeV1::Replay",
        "browser_grant_delete_returning.sql",
        "authority_generation_postcondition_assert.sql",
        "revoked_grant_postcondition_assert.sql",
    ):
        require(marker in membership_runtime_source, f"membership D1 runtime omits {marker}")
    for marker in (
        "WEB_MEMBERSHIP_ACTION_REQUEST_SCHEMA_V1",
        "OptionalJsonFieldV1::Missing",
        "OptionalJsonFieldV1::Present(_)",
        'request.headers().get("idempotency-key")',
        "trusted_active_organization_id",
        "D1LegacyMembershipAtomicPortV1::new",
        "consume_session_grant_or_confirm_absent",
    ):
        require(marker in membership_ingress_source, f"membership ingress omits {marker}")
    for marker in (
        "legacy_membership_action_operations_v1",
        "legacy_membership_action_receipts_v1",
        "legacy_membership_action_authority_subjects_v1",
        "legacy_membership_action_revoked_grants_v1",
        "frame_legacy_membership_operation_immutable_v1",
    ):
        require(marker in membership_migration, f"membership migration omits {marker}")
    for marker in (
        "legacy_membership_web_runtime::is_action",
        "legacy_membership_action_response",
        "WebMembershipActionEffectV1::SuccessObject",
        "WebMembershipActionEffectV1::SpaceMembersSet",
        "WebMembershipActionEffectV1::SpaceMembersAdded",
        "WebMembershipActionEffectV1::SpaceMembersRemoved",
    ):
        require(marker in control_lib_source, f"membership route omits {marker}")
    for marker in (
        "LEGACY_PATCH_ACCOUNT_OPERATION_ID",
        "LEGACY_SIGN_OUT_ALL_OPERATION_ID",
        "LEGACY_DEMOTE_FROM_PRO_OPERATION_ID",
        "LEGACY_PROMOTE_TO_PRO_OPERATION_ID",
        "LEGACY_RESTART_ONBOARDING_OPERATION_ID",
        "LegacyUserAccountAtomicPortV1",
        "execute_web_action",
    ):
        require(marker in user_account_application_source, f"user/account app omits {marker}")
    for marker in (
        "D1LegacyUserAccountAtomicPortV1",
        "BROWSER_GRANT_ASSERT_SQL",
        "BROWSER_GRANT_DELETE_SQL",
        "BROWSER_ASSERTION_CLEANUP_SQL",
        "consume_browser_fence",
        ".batch(statements)",
    ):
        require(marker in user_account_runtime_source, f"user/account D1 runtime omits {marker}")
    for marker in (
        "WEB_USER_ACCOUNT_ACTION_REQUEST_SCHEMA_V1",
        'request.headers().get("idempotency-key")',
        "authenticate_compatibility_mutation",
        "LegacyAuthenticatedContextV1::principal_only",
        "D1LegacyUserAccountAtomicPortV1::new",
        "consume_session_grant_or_confirm_absent",
    ):
        require(marker in user_account_ingress_source, f"user/account ingress omits {marker}")
    for marker in (
        "legacy_user_account_operations_v1",
        "legacy_user_account_receipts_v1",
        "legacy_user_account_effects_v1",
        "legacy_user_account_audit_events_v1",
        "frame_legacy_user_account_evidence_immutable_v1",
    ):
        require(marker in user_account_migration, f"user/account migration omits {marker}")
    for marker in (
        "legacy_user_account_web_runtime::is_action",
        "legacy_user_account_action_response",
        "WebUserAccountActionVoidV1",
    ):
        require(marker in control_lib_source, f"user/account route omits {marker}")
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
