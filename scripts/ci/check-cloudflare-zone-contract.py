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
    "legacy_compatibility",
    "mutation_method",
    "private_or_deleted_share",
    "range_media",
    "set_cookie_response",
    "signed_url",
    "upload_or_finalize",
    "websocket_or_sse",
}
PROTECTED_MEDIA_CHILD_ROUTES = (
    ("cap-v1-105318e146fceb4c", "POST", "/media-server/audio/check", "/media-server/audio/check"),
    ("cap-v1-77fe8c9a4b418f53", "POST", "/media-server/audio/convert", "/media-server/audio/convert"),
    ("cap-v1-a2814dde3550e586", "POST", "/media-server/audio/extract", "/media-server/audio/extract"),
    ("cap-v1-fbd3d44a0ca1786f", "GET", "/media-server/audio/status", "/media-server/audio/status"),
    ("cap-v1-0bf20f7e9b1a474c", "GET", "/media-server/health", "/media-server/health"),
    ("cap-v1-ee9797dd352c4e11", "POST", "/media-server/video/cleanup", "/media-server/video/cleanup"),
    ("cap-v1-9ed2e7b3f858eaaa", "POST", "/media-server/video/convert", "/media-server/video/convert"),
    ("cap-v1-2b48f7704d996758", "POST", "/media-server/video/edit", "/media-server/video/edit"),
    (
        "cap-v1-aa975a14fd384a5c",
        "POST",
        "/media-server/video/force-cleanup",
        "/media-server/video/force-cleanup",
    ),
    (
        "cap-v1-bf2eb9302de590a1",
        "POST",
        "/media-server/video/mux-segments",
        "/media-server/video/mux-segments",
    ),
    ("cap-v1-ba986b8c5b07cfd6", "POST", "/media-server/video/probe", "/media-server/video/probe"),
    (
        "cap-v1-320876fa0aec77cb",
        "POST",
        "/media-server/video/process",
        "/media-server/video/process",
    ),
    (
        "cap-v1-fc2e2bd0d28ffbf3",
        "POST",
        "/media-server/video/process/:jobId/cancel",
        "/media-server/video/process/job-42/cancel",
    ),
    (
        "cap-v1-43bc9ae6aa4f44a8",
        "GET",
        "/media-server/video/process/:jobId/status",
        "/media-server/video/process/job-42/status",
    ),
    ("cap-v1-986bf73a0b5cb676", "GET", "/media-server/video/status", "/media-server/video/status"),
    (
        "cap-v1-4165632f8266ae06",
        "POST",
        "/media-server/video/thumbnail",
        "/media-server/video/thumbnail",
    ),
)


def protected_media_child_route_values() -> list[dict[str, str]]:
    return [
        {
            "operation_id": operation_id,
            "method": method,
            "path": path,
            "example_path": example_path,
        }
        for operation_id, method, path, example_path in PROTECTED_MEDIA_CHILD_ROUTES
    ]


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
        assert route["compatibility_patterns"] == ["frame.engmanager.xyz/media-server*"]
        assert route["owned_pathnames"] == ["/api", "/api/", "/media-server"]
        assert route["lookalike_policy"] == "non_cacheable_404"
        protected_media = route["protected_media_children"]
        assert protected_media["edge_owner"] == "worker_compat"
        assert protected_media["source_pinned"] is True
        assert protected_media["exact_count"] == 16
        assert protected_media["route_shapes"] == protected_media_child_route_values()
        assert protected_media["release_state"] == "fail_closed_unavailable"
        assert protected_media["protected_gates"] == ["hardware_execution", "provider_execution"]
        assert protected_media["provider_promotion_claimed"] is False
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
