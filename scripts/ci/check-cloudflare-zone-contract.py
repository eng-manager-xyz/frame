#!/usr/bin/env python3
"""Validate the cross-repository Cloudflare zone handoff without owning zone state."""

from __future__ import annotations

import json
import pathlib
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "infra" / "cloudflare-zone" / "frame-contract.json"
REQUIRED_BYPASS = {
    "api",
    "authorization_header",
    "auth_session_account",
    "cookie",
    "health",
    "mutation_method",
    "private_or_deleted_share",
    "range_media",
    "set_cookie_response",
    "signed_url",
    "upload_or_finalize",
    "websocket_or_sse",
}


def main() -> int:
    try:
        policy = json.loads(CONTRACT.read_text(encoding="utf-8"))
        ownership = policy["ownership"]
        dns = policy["dns"]
        route = policy["worker_route"]
        cache = policy["cache"]
        purge = policy["purge"]
        security = policy["security"]
        assert policy["schema_version"] == 1
        assert ownership["authoritative_repository"] == "eng-manager-xyz/engmanager.xyz"
        assert ownership["frame_repository_is_read_only_consumer"] is True
        assert ownership["requires_complete_phase_import"] is True
        assert ownership["requires_semantic_noop_before_frame_rules"] is True
        assert dns["hostname"] == "frame.engmanager.xyz"
        assert dns["record_type"] == "CNAME"
        assert dns["wildcard_allowed"] is False
        assert dns["initial_proxy"] is False
        assert "AAAA" in dns["assert_absent_record_types"]
        assert route["pattern"] == "frame.engmanager.xyz/api*"
        assert route["lookalike_policy"] == "non_cacheable_404"
        assert route["workers_dev"] is False
        assert cache["default"] == "bypass"
        assert REQUIRED_BYPASS <= set(cache["always_bypass"])
        assert cache["immutable_only"]["requires_content_fingerprint"] is True
        assert cache["immutable_only"]["allowed_methods"] == ["GET", "HEAD"]
        assert cache["public_share_html_enabled"] is False
        assert cache["origin_cache_control_required"] is True
        assert purge["allowed_hostname"] == "frame.engmanager.xyz"
        assert purge["exact_urls_only"] is True
        assert purge["zone_wide_purge_allowed"] is False
        assert security["initial_enforcement"] == "observe"
        assert security["host_scope"] == "frame.engmanager.xyz"
        assert security["user_controlled_identifier_may_be_sole_key"] is False
        assert security["cloudflare_access_public_app"] is False
        assert "remove_worker_route" in policy["rollback"]
        assert "switch_frame_cname_dns_only" in policy["rollback"]
    except (AssertionError, KeyError, OSError, json.JSONDecodeError, TypeError) as error:
        print(
            f"Cloudflare zone contract validation failed ({type(error).__name__})",
            file=sys.stderr,
        )
        return 1
    print("Cloudflare cross-repository DNS, route, cache, purge, and WAF contract verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
