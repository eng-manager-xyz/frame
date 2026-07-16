#!/usr/bin/env python3
"""Validate the deny-by-default sibling-origin and browser capability boundary."""

from __future__ import annotations

import json
import pathlib
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
MATRIX = ROOT / "fixtures/browser-security/v1/capability-matrix.json"


def main() -> int:
    errors: list[str] = []
    try:
        matrix = json.loads(MATRIX.read_text(encoding="utf-8"))
        assert matrix["schema_version"] == 1
        assert matrix["origins"]["relationship"] == "mutually_untrusted_siblings"
        capabilities = {item["name"]: item for item in matrix["capabilities"]}
        assert capabilities["top_level_navigation"]["production"] == "enabled"
        for disabled in (
            "authenticated_ssr",
            "portfolio_login_handoff",
            "portfolio_browser_cors",
            "public_player_embed",
        ):
            assert capabilities[disabled]["production"] == "disabled"
        assert capabilities["recorder_embed"]["production"] == "forbidden"
        assert matrix["cors"]["wildcard_credentials"] is False
        assert matrix["csp_reports"]["maximum_body_bytes"] == 16_384
        assert matrix["csp_reports"]["maximum_reports_per_body"] == 8
    except (AssertionError, KeyError, OSError, json.JSONDecodeError, TypeError) as error:
        print(f"browser capability matrix invalid ({type(error).__name__})", file=sys.stderr)
        return 1

    domain = (ROOT / "crates/domain/src/identity.rs").read_text(encoding="utf-8")
    application = (ROOT / "crates/application/src/identity.rs").read_text(encoding="utf-8")
    authenticated = (ROOT / "apps/web/src/authenticated.rs").read_text(encoding="utf-8")
    share = (ROOT / "apps/web/src/share_player.rs").read_text(encoding="utf-8")
    web = (ROOT / "apps/web/src/lib.rs").read_text(encoding="utf-8")
    browser = (ROOT / "apps/web/src/browser_security.rs").read_text(encoding="utf-8")
    render = (ROOT / "render.yaml").read_text(encoding="utf-8")

    required_markers = {
        "host-only session cookie": (
            domain,
            'name: "__Host-frame_session".into()',
        ),
        "same-site sibling rejection": (application, "FetchSite::SameSite"),
        "allowlisted return paths": (authenticated, "pub struct SafeReturnPath"),
        "return query rejection": (authenticated, "value.contains(['\\\\', '?', '#'])"),
        "exact embed origin": (share, "EmbedRejection::Origin"),
        "parent window validation": (share, "source_is_parent"),
        "message replay fence": (share, "envelope.sequence <= self.last_sequence"),
        "unknown message field rejection": (share, "deny_unknown_fields"),
        "default frame denial": (web, '"frame-ancestors \'none\'"'),
        "capture permissions denial": (web, "camera=(), display-capture=()"),
        "bounded body layer": (web, "DefaultBodyLimit::max(16 * 1024)"),
        "CSP report endpoint": (web, '"/__frame/csp-report"'),
        "CSP reporting directive": (web, "report-uri /__frame/csp-report"),
        "privacy-safe CSP normalization": (browser, "sanitize_csp_reports"),
        "production embed kill switch": (render, 'FRAME_ENABLE_PUBLIC_EMBED'),
        "production embed disabled": (render, 'value: "false"'),
    }
    for name, (source, marker) in required_markers.items():
        if marker not in source:
            errors.append(f"missing {name}")
    if "AuthenticatedSsr" in web or '"x-frame-tenant-id"' in web:
        errors.append("authenticated Render SSR or tenant-header forwarding is enabled")
    if "AuthenticatedSsr" in authenticated or "reqwest" in authenticated:
        errors.append("authenticated contract module contains an SSR credential transport")
    for artifact in (
        "docs/security/browser-integration-boundaries.md",
        "docs/operations/browser-security-rollout.md",
        "docs/evidence/browser-security-local.md",
    ):
        if not (ROOT / artifact).is_file():
            errors.append(f"missing {artifact}")

    if errors:
        print("browser security contract failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print(
        f"validated {len(capabilities)} browser capabilities, host-only auth, exact embed, and sanitized CSP reporting"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
