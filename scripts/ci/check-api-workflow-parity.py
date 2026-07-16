#!/usr/bin/env python3
"""Generate and validate the Cap-to-Frame API/workflow parity inventory.

The ignored `.tmp/cap` checkout is an optional parity oracle in normal CI and
is required by `--generate`/`--require-reference`. The committed report remains
fully checkable without network access or a vendored upstream checkout.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
CAP = ROOT / ".tmp" / "cap"
REPORT = ROOT / "fixtures" / "api-parity" / "v1" / "route-workflow-report.json"
DOC = ROOT / "docs" / "generated" / "api-workflow-parity-v1.md"
REFERENCE_COMMIT = "6ba69561ac86b8efdb17616d6727f9638015546b"
SCHEMA_VERSION = "frame.api-parity.v1"
CONTRACT_VERSION = "frame.api.v1"
HTTP_METHODS = ("GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS")
ALL_METHODS = set(HTTP_METHODS) | {"ACTION", "RPC", "WORKFLOW"}

HMAC_WEBHOOK_PATHS = ("/api/webhooks/",)

HonoMount = tuple[str, str, tuple[str, ...]]
HONO_MOUNTS: tuple[HonoMount, ...] = (
    ("apps/web/app/api/desktop/[...route]/root.ts", "/api/desktop", ("app",)),
    (
        "apps/web/app/api/desktop/[...route]/s3Config.ts",
        "/api/desktop/s3/config",
        ("app",),
    ),
    (
        "apps/web/app/api/desktop/[...route]/session.ts",
        "/api/desktop/session",
        ("app",),
    ),
    (
        "apps/web/app/api/desktop/[...route]/storage.ts",
        "/api/desktop/storage",
        ("app", "protectedApp"),
    ),
    (
        "apps/web/app/api/desktop/[...route]/video.ts",
        "/api/desktop/video",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/credits/checkout/route.ts",
        "/api/developer/credits/checkout",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/sdk/v1/[...route]/upload.ts",
        "/api/developer/sdk/v1/upload/multipart",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/sdk/v1/[...route]/video-create.ts",
        "/api/developer/sdk/v1/videos",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/v1/[...route]/usage.ts",
        "/api/developer/v1/usage",
        ("app",),
    ),
    (
        "apps/web/app/api/developer/v1/[...route]/videos.ts",
        "/api/developer/v1/videos",
        ("app",),
    ),
    (
        "apps/web/app/api/upload/[...route]/multipart.ts",
        "/api/upload/multipart",
        ("app",),
    ),
    (
        "apps/web/app/api/upload/[...route]/recording-complete.ts",
        "/api/upload/recording-complete",
        ("app",),
    ),
    (
        "apps/web/app/api/upload/[...route]/signed.ts",
        "/api/upload/signed",
        ("app",),
    ),
    ("apps/media-server/src/app.ts", "/media-server", ("app",)),
    ("apps/media-server/src/routes/health.ts", "/media-server/health", ("health",)),
    ("apps/media-server/src/routes/audio.ts", "/media-server/audio", ("audio",)),
    ("apps/media-server/src/routes/video.ts", "/media-server/video", ("video",)),
)

EFFECT_SOURCES: tuple[tuple[str, str], ...] = (
    ("packages/web-domain/src/Mobile.ts", "/api/mobile"),
    ("packages/web-domain/src/Extension.ts", "/api/extension"),
    ("packages/web-domain/src/Loom.ts", "/api/loom"),
    ("packages/web-api-contract-effect/src/index.ts", "/api"),
)

TS_REST_CONTRACT_SOURCES: tuple[str, ...] = (
    "packages/web-api-contract/src/index.ts",
    "packages/web-api-contract/src/desktop.ts",
)

EXTENSION_ENDPOINTS: tuple[tuple[str, str, str], ...] = (
    ("GET", "/auth/start", "startAuth"),
    ("POST", "/auth/approve", "approveAuth"),
    ("POST", "/auth/revoke", "revokeAuth"),
    ("GET", "/bootstrap", "bootstrap"),
    ("POST", "/instant-recordings", "createInstantRecording"),
    ("POST", "/instant-recordings/progress", "updateInstantRecordingProgress"),
    ("DELETE", "/instant-recordings/:videoId", "deleteInstantRecording"),
)

CURATED_WORKFLOW_ENTRYPOINTS: tuple[tuple[str, str], ...] = (
    ("apps/web/lib/video-processing.ts", "startVideoProcessingWorkflow"),
    ("apps/web/lib/desktop-segments-finalization.ts", "queueDesktopSegmentsFinalization"),
    (
        "apps/web/lib/desktop-segments-recovery.ts",
        "completeDesktopSegmentsManifestAndQueue",
    ),
    ("apps/web/lib/desktop-segments-recovery.ts", "recoverStaleDesktopSegments"),
    ("apps/web/lib/video-edit-processing.ts", "reconcileStaleEditUpload"),
    ("apps/web/lib/generate-ai.ts", "startAiGeneration"),
)

TRANSPORT_WRAPPERS = (
    "apps/web/app/api/[[...route]]/route.ts",
    "apps/web/app/api/desktop/[...route]/route.ts",
    "apps/web/app/api/developer/sdk/v1/[...route]/route.ts",
    "apps/web/app/api/developer/v1/[...route]/route.ts",
    "apps/web/app/api/mobile/[...route]/route.ts",
    "apps/web/app/api/upload/[...route]/route.ts",
)

EVIDENCE_VALUES = {
    "local_contract",
    "family_contract",
    "retirement_contract_pending_approval",
    "endpoint_adapter_pending",
    "protected_evidence_required",
    "dependency_pending",
}


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def source_ref(path: str, symbol: str) -> dict[str, str]:
    source = CAP / path
    return {
        "path": path,
        "symbol": symbol,
        "sha256": sha256_bytes(source.read_bytes()),
    }


def join_path(prefix: str, suffix: str) -> str:
    if suffix == "/":
        return prefix or "/"
    return f"{prefix.rstrip('/')}/{suffix.lstrip('/')}"


def filesystem_api_path(source: Path) -> str:
    relative = source.relative_to(CAP / "apps" / "web" / "app").parent
    parts: list[str] = []
    for part in relative.parts:
        catch_all = re.fullmatch(r"\[\.\.\.([^]]+)\]", part)
        optional_catch_all = re.fullmatch(r"\[\[\.\.\.([^]]+)\]\]", part)
        dynamic = re.fullmatch(r"\[([^]]+)\]", part)
        if catch_all or optional_catch_all:
            parts.append(f":{(catch_all or optional_catch_all).group(1)}*")
        elif dynamic:
            parts.append(f":{dynamic.group(1)}")
        else:
            parts.append(part)
    return "/" + "/".join(parts)


def collect_raw() -> list[dict[str, Any]]:
    if not CAP.is_dir():
        raise RuntimeError("the pinned .tmp/cap reference checkout is missing")

    rows: list[dict[str, Any]] = []

    # Next route handlers that are not catch-all transport wrappers.
    api_root = CAP / "apps" / "web" / "app" / "api"
    for source in sorted((*api_root.rglob("route.ts"), *api_root.rglob("route.tsx"))):
        relative = source.relative_to(CAP).as_posix()
        if relative in TRANSPORT_WRAPPERS:
            continue
        text = source.read_text(encoding="utf-8")
        methods = set(
            re.findall(
                r"export\s+(?:(?:async\s+)?function|const)\s+"
                r"(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b",
                text,
            )
        )
        for export_block in re.findall(r"export\s*\{([^}]+)\}", text, re.DOTALL):
            methods.update(
                re.findall(
                    r"\bas\s+(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b",
                    export_block,
                )
            )
        for method in sorted(methods):
            rows.append(
                raw_row(
                    "route",
                    method,
                    filesystem_api_path(source),
                    method,
                    source_ref(relative, method),
                )
            )

    # Mounted Hono services, including the legacy media-server control API.
    for relative, prefix, receivers in HONO_MOUNTS:
        source = CAP / relative
        text = source.read_text(encoding="utf-8")
        receiver_pattern = "|".join(re.escape(value) for value in receivers)
        pattern = re.compile(
            rf"(?:\b(?:{receiver_pattern})|new\s+Hono\(\))\s*\.\s*"
            r"(get|post|put|patch|delete|head|options)\s*\(\s*[\"']([^\"']+)[\"']",
            re.DOTALL,
        )
        for match in pattern.finditer(text):
            method = match.group(1).upper()
            path = join_path(prefix, match.group(2))
            rows.append(
                raw_row(
                    "route",
                    method,
                    path,
                    f"{match.group(1)}:{match.group(2)}",
                    source_ref(relative, f"{method} {match.group(2)}"),
                )
            )

    # Effect HttpApi definitions. Prefixes are applied where their host API mounts them.
    endpoint_pattern = re.compile(
        r"HttpApiEndpoint\.(get|post|del|put|patch)\(\s*"
        r"[\"']([^\"']+)[\"']\s*,\s*(?:[\"']([^\"']+)[\"']|`([^`]+)`)",
        re.DOTALL,
    )
    for relative, prefix in EFFECT_SOURCES:
        source = CAP / relative
        text = source.read_text(encoding="utf-8")
        for match in endpoint_pattern.finditer(text):
            method = {"del": "DELETE"}.get(match.group(1), match.group(1).upper())
            operation = match.group(2)
            endpoint = (match.group(3) or match.group(4)).replace(
                "${INSTANT_RECORDINGS_PATH}", "/instant-recordings"
            )
            effective_prefix = "" if endpoint.startswith("/commercial/") else prefix
            rows.append(
                raw_row(
                    "route",
                    method,
                    join_path(effective_prefix, endpoint),
                    operation,
                    source_ref(relative, operation),
                )
            )

    # The older ts-rest snapshot remains a supported-client contract oracle.
    # Merge it into the route identity rather than counting it as another route.
    ts_rest_pattern = re.compile(
        r"method:\s*[\"'](GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)[\"']\s*,\s*"
        r"path:\s*[\"']([^\"']+)[\"']",
        re.DOTALL,
    )
    for relative in TS_REST_CONTRACT_SOURCES:
        text = (CAP / relative).read_text(encoding="utf-8")
        for method, endpoint in ts_rest_pattern.findall(text):
            prefix = "" if endpoint.startswith("/commercial/") else "/api"
            rows.append(
                raw_row(
                    "route",
                    method,
                    join_path(prefix, endpoint),
                    f"ts-rest:{method}:{endpoint}",
                    source_ref(relative, f"{method} {endpoint}"),
                )
            )

    # Extension paths are deliberately centralized as constants in the source,
    # so they cannot all be recovered from a literal-only HttpApi regex.
    extension_relative = "packages/web-domain/src/Extension.ts"
    for method, endpoint, operation in EXTENSION_ENDPOINTS:
        rows.append(
            raw_row(
                "route",
                method,
                join_path("/api/extension", endpoint),
                operation,
                source_ref(extension_relative, operation),
            )
        )

    # Next server actions. Files without `use server` are ordinary helpers.
    web_root = CAP / "apps" / "web"
    action_pattern = re.compile(
        r"export\s+(?:async\s+function\s+([A-Za-z_$][\w$]*)|"
        r"const\s+([A-Za-z_$][\w$]*)\s*=\s*async\b)"
    )
    for source in sorted((*web_root.rglob("*.ts"), *web_root.rglob("*.tsx"))):
        text = source.read_text(encoding="utf-8")
        if not re.search(r"[\"']use server[\"']", text):
            continue
        relative = source.relative_to(CAP).as_posix()
        for match in action_pattern.finditer(text):
            operation = match.group(1) or match.group(2)
            rows.append(
                raw_row(
                    "server_action",
                    "ACTION",
                    f"action://{relative}#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )
        # An inline action is not exported, but Next still turns it into a
        # remotely invocable server action when the function body carries the directive.
        for operation in re.findall(
            r"async\s+function\s+([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*\{\s*"
            r"[\"']use server[\"']",
            text,
            re.DOTALL,
        ):
            rows.append(
                raw_row(
                    "server_action",
                    "ACTION",
                    f"action://{relative}#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )

    # Durable workflow entrypoints in the web workflow directory.
    workflows_root = CAP / "apps" / "web" / "workflows"
    workflow_pattern = re.compile(
        r"export\s+async\s+function\s+([A-Za-z_$][\w$]*Workflow)\b"
    )
    for source in sorted(workflows_root.rglob("*.ts")):
        text = source.read_text(encoding="utf-8")
        relative = source.relative_to(CAP).as_posix()
        for operation in workflow_pattern.findall(text):
            rows.append(
                raw_row(
                    "workflow",
                    "WORKFLOW",
                    f"workflow://{relative}#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )

    for relative, operation in CURATED_WORKFLOW_ENTRYPOINTS:
        text = (CAP / relative).read_text(encoding="utf-8")
        if not re.search(
            rf"export\s+(?:async\s+function|const)\s+{re.escape(operation)}\b",
            text,
        ):
            raise RuntimeError(f"curated workflow entrypoint drifted: {relative}#{operation}")
        rows.append(
            raw_row(
                "workflow",
                "WORKFLOW",
                f"workflow://{relative}#{operation}",
                operation,
                source_ref(relative, operation),
            )
        )

    # Effect RPC operations carried over the /api/erpc transport.
    domain_root = CAP / "packages" / "web-domain" / "src"
    for source in sorted(domain_root.rglob("*.ts")):
        text = source.read_text(encoding="utf-8")
        relative = source.relative_to(CAP).as_posix()
        for operation in re.findall(r"Rpc\.make\(\s*[\"']([^\"']+)[\"']", text):
            rows.append(
                raw_row(
                    "rpc",
                    "RPC",
                    f"/api/erpc#{operation}",
                    operation,
                    source_ref(relative, operation),
                )
            )

    # Effect durable workflows outside apps/web/workflows.
    loom_relative = "packages/web-domain/src/Loom.ts"
    loom_text = (CAP / loom_relative).read_text(encoding="utf-8")
    for operation in re.findall(
        r"Workflow\.make\(\s*\{\s*name:\s*[\"']([^\"']+)[\"']", loom_text, re.DOTALL
    ):
        rows.append(
            raw_row(
                "workflow",
                "WORKFLOW",
                f"workflow://{loom_relative}#{operation}",
                operation,
                source_ref(loom_relative, operation),
            )
        )

    return merge_routes(rows)


def raw_row(
    kind: str,
    method: str,
    legacy_path: str,
    operation: str,
    source: dict[str, str],
) -> dict[str, Any]:
    return {
        "kind": kind,
        "method": method,
        "legacy_path": legacy_path,
        "operations": [operation],
        "sources": [source],
    }


def merge_routes(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    merged: dict[tuple[str, str, str], dict[str, Any]] = {}
    for row in rows:
        key = (row["kind"], row["method"], row["legacy_path"])
        if row["kind"] != "route" or key not in merged:
            if key in merged:
                raise RuntimeError(f"duplicate non-route inventory identity: {key}")
            merged[key] = row
            continue
        current = merged[key]
        current["operations"] = sorted(
            set(current["operations"]) | set(row["operations"])
        )
        known_sources = {
            (item["path"], item["symbol"], item["sha256"]) for item in current["sources"]
        }
        for source in row["sources"]:
            identity = (source["path"], source["symbol"], source["sha256"])
            if identity not in known_sources:
                current["sources"].append(source)
        current["sources"].sort(key=lambda item: (item["path"], item["symbol"]))
    return sorted(
        merged.values(),
        key=lambda row: (row["kind"], row["legacy_path"], row["method"]),
    )


def classify(row: dict[str, Any]) -> dict[str, Any]:
    searchable = " ".join(
        [row["legacy_path"], *row["operations"], *(item["path"] for item in row["sources"])]
    ).lower()
    path = row["legacy_path"].lower()
    method = row["method"]

    if "messenger" in searchable:
        family = "messenger_support"
    # Classify the authority before the transport. A Stripe callback and a
    # media workflow are not made low-risk merely because both use a webhook
    # or workflow wrapper.
    elif any(value in searchable for value in ("billing", "stripe", "checkout", "subscribe")):
        family = "billing_admin"
    elif "admin" in searchable:
        family = "billing_admin"
    elif any(value in searchable for value in ("loom", "import")):
        family = "imports_integrations"
    elif any(
        value in searchable
        for value in ("organization", "organisation", "space", "folder", "collection")
    ):
        family = "organization_library"
    elif any(
        value in searchable
        for value in (
            "/auth",
            "auth/",
            "startauth",
            "approveauth",
            "revokeauth",
            "oauth",
            "session",
            "invite",
        )
    ):
        family = "auth_session"
    elif any(
        value in searchable
        for value in ("upload", "storage", "s3", "google-drive", "download")
    ):
        family = "upload_storage"
    elif any(
        value in searchable
        for value in ("comment", "notification", "transcript", "reaction")
    ):
        family = "collaboration_notifications"
    elif any(value in searchable for value in ("share", "password", "playlist")):
        family = "share_playback"
    elif any(value in searchable for value in ("developer", "commercial", "credit", "usage")):
        family = "developer_api"
    elif "analytics" in searchable:
        family = "analytics_consent"
    elif any(
        value in searchable
        for value in (
            "video",
            "thumbnail",
            "audio",
            "transcrib",
            "recording",
            "media-server",
            "generateai",
            "generate-ai",
            "desktop-segments",
        )
    ):
        family = "video_media"
    elif any(value in searchable for value in ("webhook", "cron/", "workflow://")):
        family = "callbacks_webhooks_workflows"
    elif any(value in searchable for value in ("desktop", "mobile", "extension")):
        family = "client_compatibility"
    else:
        family = "service_misc"

    clients = clients_for(searchable, row["kind"])
    auth = auth_for(path, searchable, family)
    disposition, local_status, deprecation = disposition_for(family, searchable)
    owners, authority = authority_for(family)
    idempotency = (
        "forbidden"
        if method in {"GET", "HEAD", "OPTIONS"}
        else "required"
    )
    body_limit = 0 if method in {"GET", "HEAD", "OPTIONS"} else 256 * 1024
    if family == "upload_storage":
        body_limit = 8 * 1024 * 1024
    if path.startswith(HMAC_WEBHOOK_PATHS):
        body_limit = 1024 * 1024

    evidence_default = (
        "protected_evidence_required"
        if disposition == "protected_parity_required"
        else "dependency_pending"
        if family == "video_media"
        else "retirement_contract_pending_approval"
        if disposition == "retire"
        else "family_contract"
    )
    endpoint_success = (
        "retirement_contract_pending_approval"
        if disposition == "retire"
        else "protected_evidence_required"
        if disposition == "protected_parity_required"
        else "endpoint_adapter_pending"
    )
    retry_evidence = (
        "local_contract"
        if row["kind"] == "workflow" or family == "callbacks_webhooks_workflows"
        else evidence_default
    )

    identity = "\0".join((row["kind"], method, row["legacy_path"])).encode()
    return {
        "id": f"cap-v1-{sha256_bytes(identity)[:16]}",
        "kind": row["kind"],
        "legacy_path": row["legacy_path"],
        "method": method,
        "operations": row["operations"],
        "sources": row["sources"],
        "clients": clients,
        "auth": auth,
        "policy": f"{family}.v1",
        "contract_version": CONTRACT_VERSION,
        "owners": owners,
        "implementation": {
            "rust_authority": authority,
            "local_status": local_status,
        },
        "disposition": disposition,
        "security": {
            "max_body_bytes": body_limit,
            "rate_limit_bucket": f"{family}.v1",
            "idempotency": idempotency,
            "tenant_non_disclosure": family not in {"service_misc", "analytics_consent"},
        },
        "contract_evidence": {
            "success": endpoint_success,
            "validation": evidence_default,
            "authorization": evidence_default,
            "idempotency_retry": retry_evidence,
            "failure": evidence_default,
        },
        "deprecation": deprecation,
    }


def clients_for(searchable: str, kind: str) -> list[str]:
    clients: list[str] = []
    for token, client in (
        ("desktop", "desktop"),
        ("mobile", "mobile"),
        ("extension", "extension"),
        ("developer", "developer"),
        ("sdk", "developer"),
        ("webhook", "provider"),
        ("cron", "scheduler"),
        ("media-server", "internal_worker"),
    ):
        if token in searchable and client not in clients:
            clients.append(client)
    if kind == "workflow" and "scheduler" not in clients:
        clients.append("scheduler")
    if kind == "server_action" and "web" not in clients:
        clients.append("web")
    if not clients:
        clients.append("web")
    return sorted(clients)


def auth_for(path: str, searchable: str, family: str) -> str:
    if path.startswith("/api/webhooks/"):
        return "signed_webhook"
    if "/cron/" in path:
        return "scheduler_secret"
    if path.startswith("/media-server/"):
        return "internal_service"
    if "developer/sdk" in searchable or "developer/v1" in searchable:
        return "developer_api_key"
    if family == "billing_admin" or "admin" in searchable:
        return "admin_session"
    if any(
        marker in path
        for marker in (
            "/status",
            "/changelog",
            "/releases/",
            "/auth/",
            "/commercial/",
        )
    ):
        return "public_or_flow_token"
    if any(marker in path for marker in ("/thumbnail", "/preview", "/playlist", "/download")):
        return "optional_session_or_share_capability"
    return "session"


def disposition_for(family: str, searchable: str) -> tuple[str, str, dict[str, Any]]:
    no_deprecation = {
        "state": "not_deprecated",
        "earliest_removal": None,
        "migration_path": None,
        "approval": None,
    }
    if family == "messenger_support":
        return (
            "retire",
            "retirement_response_and_owner_approval_pending",
            {
                "state": "retirement_proposed",
                "earliest_removal": None,
                "migration_path": "privacy-safe export; product surface remains off by default",
                "approval": "repository_owner_pending",
            },
        )
    if family == "billing_admin":
        return (
            "protected_parity_required",
            "provider_sandbox_and_ledger_reconciliation_pending",
            no_deprecation,
        )
    if family == "imports_integrations" or any(
        token in searchable for token in ("s3", "google-drive")
    ):
        return (
            "migrate",
            "migration_authority_present_provider_adapter_pending",
            no_deprecation,
        )
    if family == "video_media":
        return (
            "replace",
            "rust_authority_present_issue_28_adapter_or_protected_evidence_pending",
            no_deprecation,
        )
    return "replace", "rust_authority_present_endpoint_adapter_pending", no_deprecation


def authority_for(family: str) -> tuple[list[str], str]:
    mapping = {
        "auth_session": (["13", "30"], "frame-application::identity + api_workflow"),
        "organization_library": (
            ["14", "30"],
            "frame-application::organization + business",
        ),
        "upload_storage": (
            ["18", "19", "20", "21", "30"],
            "frame-application storage/multipart/backfill/governance",
        ),
        "collaboration_notifications": (
            ["15", "30"],
            "frame-application::business durable aggregates/outbox",
        ),
        "share_playback": (
            ["15", "21", "30", "32"],
            "frame-application::business + governed storage",
        ),
        "developer_api": (
            ["13", "15", "30", "36"],
            "frame-application::identity + business developer ledger",
        ),
        "billing_admin": (
            ["15", "30"],
            "frame-application::business ledger; provider authority not promoted",
        ),
        "analytics_consent": (
            ["15", "30"],
            "frame-application::business consent/usage aggregates",
        ),
        "imports_integrations": (
            ["15", "20", "30"],
            "frame-application::backfill + business import workflow",
        ),
        "video_media": (
            ["07", "15", "28", "29", "30"],
            "frame-media job contracts + control-plane capability router",
        ),
        "callbacks_webhooks_workflows": (
            ["15", "28", "30"],
            "frame-application::api_workflow + control-plane D1 replay + business outbox",
        ),
        "messenger_support": (
            ["15", "30"],
            "business export authority; retained product adapter intentionally absent",
        ),
        "client_compatibility": (
            ["30", "36"],
            "frame-domain::api_workflow compatibility policy",
        ),
        "service_misc": (["30"], "frame-domain/application API contracts"),
    }
    return mapping[family]


def build_report() -> dict[str, Any]:
    raw = collect_raw()
    entries = [classify(row) for row in raw]
    entries.sort(key=lambda row: (row["kind"], row["legacy_path"], row["method"]))
    by_kind = Counter(row["kind"] for row in entries)
    by_disposition = Counter(row["disposition"] for row in entries)
    by_status = Counter(row["implementation"]["local_status"] for row in entries)
    return {
        "schema_version": SCHEMA_VERSION,
        "reference": {
            "repository": "CapSoftware/Cap",
            "commit": REFERENCE_COMMIT,
        },
        "contract_version": CONTRACT_VERSION,
        "generated_by": "scripts/ci/check-api-workflow-parity.py",
        "scope": {
            "included": [
                "Next API route handlers",
                "mounted Hono routes",
                "Effect HttpApi endpoints",
                "Next server actions",
                "Effect RPC operations",
                "durable workflow entrypoints",
            ],
            "transport_wrappers": list(TRANSPORT_WRAPPERS),
            "excluded": [
                "ordinary helper exports without use server",
                "UI routes and pixel behavior owned by issues 31-33",
                "native desktop IPC owned by issue 33",
            ],
        },
        "summary": {
            "total": len(entries),
            "by_kind": dict(sorted(by_kind.items())),
            "by_disposition": dict(sorted(by_disposition.items())),
            "by_local_status": dict(sorted(by_status.items())),
            "endpoint_success_proven": sum(
                row["contract_evidence"]["success"] == "local_contract" for row in entries
            ),
            "endpoint_success_pending": sum(
                row["contract_evidence"]["success"] != "local_contract" for row in entries
            ),
        },
        "entries": entries,
    }


def render_doc(report: dict[str, Any]) -> str:
    summary = report["summary"]
    entries = report["entries"]
    route_entries = [row for row in entries if row["kind"] == "route"]
    evidence_values = (
        "local_contract",
        "family_contract",
        "dependency_pending",
        "endpoint_adapter_pending",
        "protected_evidence_required",
        "retirement_contract_pending_approval",
    )
    evidence_axes = (
        "success",
        "validation",
        "authorization",
        "idempotency_retry",
        "failure",
    )
    client_counts = Counter(client for row in entries for client in row["clients"])
    client_success = Counter(
        client
        for row in entries
        if row["contract_evidence"]["success"] == "local_contract"
        for client in row["clients"]
    )
    lines = [
        "# Generated API and workflow parity report",
        "",
        "<!-- Generated by scripts/ci/check-api-workflow-parity.py; do not edit by hand. -->",
        "",
        f"Reference: `CapSoftware/Cap@{REFERENCE_COMMIT}`. Target contract: `{CONTRACT_VERSION}`.",
        "",
        "This is an inventory and gap report, not a production-parity attestation. "
        "A mapped Rust authority does not prove that its legacy endpoint adapter or protected "
        "provider path has passed E2E tests.",
        "",
        "## Summary",
        "",
        f"- Total retained/retirement decisions inventoried: **{summary['total']}**",
        f"- Endpoint-level success contracts proven locally: **{summary['endpoint_success_proven']}**",
        f"- Endpoint-level success or retirement approval still pending: **{summary['endpoint_success_pending']}**",
        "- Kinds: " + ", ".join(f"`{key}` {value}" for key, value in summary["by_kind"].items()),
        "- Dispositions: "
        + ", ".join(f"`{key}` {value}" for key, value in summary["by_disposition"].items()),
        "",
        "## Executable coverage boundary",
        "",
        f"The {len(route_entries)} HTTP method rows represent "
        f"{len({row['legacy_path'] for row in route_entries})} unique legacy paths. "
        f"Exactly {sum(row['legacy_path'].startswith('/api/v1') for row in route_entries)} "
        "are already under `/api/v1`; a target-native Frame route is not counted as a legacy "
        "adapter unless its inventory row has endpoint-level evidence.",
        "",
        "| Kind | Inventory rows | Endpoint success proven | Endpoint success pending |",
        "|---|---:|---:|---:|",
    ]
    for kind, count in summary["by_kind"].items():
        proven = sum(
            row["kind"] == kind
            and row["contract_evidence"]["success"] == "local_contract"
            for row in entries
        )
        lines.append(f"| `{kind}` | {count} | {proven} | {count - proven} |")
    lines.extend(
        [
            "",
            "Evidence values below are row counts, not endpoint passes inferred from a shared "
            "family contract.",
            "",
            "| Evidence axis | `local_contract` | `family_contract` | `dependency_pending` | "
            "`endpoint_adapter_pending` | `protected_evidence_required` | "
            "`retirement_contract_pending_approval` |",
            "|---|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for axis in evidence_axes:
        counts = Counter(row["contract_evidence"][axis] for row in entries)
        values = " | ".join(str(counts[value]) for value in evidence_values)
        lines.append(f"| `{axis}` | {values} |")
    lines.extend(
        [
            "",
            "Client counts are associations and can overlap when one operation serves multiple "
            "client families. They do not claim a current or N-1 client journey.",
            "",
            "| Client family | Operation associations | Endpoint success proven | "
            "Endpoint success pending |",
            "|---|---:|---:|---:|",
        ]
    )
    for client, count in sorted(client_counts.items()):
        proven = client_success[client]
        lines.append(f"| `{client}` | {count} | {proven} | {count - proven} |")
    lines.extend(
        [
            "",
            "## Contract inventory",
            "",
            "| Method | Legacy path / operation | Clients | Auth | Policy | Disposition | Local status |",
            "|---|---|---|---|---|---|---|",
        ]
    )
    for row in entries:
        path = row["legacy_path"].replace("|", "\\|")
        clients = ", ".join(row["clients"])
        lines.append(
            f"| `{row['method']}` | `{path}` | {clients} | `{row['auth']}` | "
            f"`{row['policy']}` | `{row['disposition']}` | "
            f"`{row['implementation']['local_status']}` |"
        )
    lines.extend(
        [
            "",
            "## Reading the evidence fields",
            "",
            "Each machine-readable row has independent success, validation, authorization, "
            "idempotency/retry, and failure evidence states. `family_contract` means the shared "
            "Rust authority has focused tests; `endpoint_adapter_pending` means the legacy-shaped "
            "transport has not earned redirect authority. Protected provider and retirement rows "
            "remain explicitly pending until their named reviewer evidence exists.",
            "",
        ]
    )
    return "\n".join(lines)


def validate_report(report: dict[str, Any], *, compare_reference: bool) -> list[str]:
    errors: list[str] = []
    required_top = {
        "schema_version",
        "reference",
        "contract_version",
        "generated_by",
        "scope",
        "summary",
        "entries",
    }
    if set(report) != required_top:
        errors.append("report top-level keys drifted")
        return errors
    if report["schema_version"] != SCHEMA_VERSION:
        errors.append("report schema version drifted")
    if report["reference"] != {
        "repository": "CapSoftware/Cap",
        "commit": REFERENCE_COMMIT,
    }:
        errors.append("reference identity drifted")
    if report["contract_version"] != CONTRACT_VERSION:
        errors.append("target API contract version drifted")
    entries = report.get("entries")
    if not isinstance(entries, list) or not entries:
        errors.append("report entries must be a non-empty array")
        return errors

    entry_keys = {
        "id",
        "kind",
        "legacy_path",
        "method",
        "operations",
        "sources",
        "clients",
        "auth",
        "policy",
        "contract_version",
        "owners",
        "implementation",
        "disposition",
        "security",
        "contract_evidence",
        "deprecation",
    }
    identities: set[tuple[str, str, str]] = set()
    ids: set[str] = set()
    for index, row in enumerate(entries):
        label = f"entry {index}"
        if not isinstance(row, dict) or set(row) != entry_keys:
            errors.append(f"{label}: keys drifted")
            continue
        if not re.fullmatch(r"cap-v1-[0-9a-f]{16}", row["id"]):
            errors.append(f"{label}: invalid ID")
        expected_id = "cap-v1-" + sha256_bytes(
            "\0".join((row["kind"], row["method"], row["legacy_path"])).encode()
        )[:16]
        if row["id"] != expected_id:
            errors.append(f"{label}: ID does not match its stable identity")
        if row["id"] in ids:
            errors.append(f"{label}: duplicate ID")
        ids.add(row["id"])
        identity = (row["kind"], row["method"], row["legacy_path"])
        if identity in identities:
            errors.append(f"{label}: duplicate route/action/workflow identity")
        identities.add(identity)
        if row["method"] not in ALL_METHODS:
            errors.append(f"{label}: unsupported method")
        if row["contract_version"] != CONTRACT_VERSION:
            errors.append(f"{label}: contract version drifted")
        for field in ("operations", "sources", "clients", "owners"):
            if not isinstance(row[field], list) or not row[field]:
                errors.append(f"{label}: {field} must be non-empty")
        if row["disposition"] not in {
            "replace",
            "migrate",
            "retire",
            "protected_parity_required",
        }:
            errors.append(f"{label}: disposition is invalid")
        searchable = " ".join(
            [
                row["legacy_path"],
                *row["operations"],
                *(source["path"] for source in row["sources"]),
            ]
        ).lower()
        if "messenger" not in searchable and any(
            token in searchable
            for token in ("billing", "stripe", "checkout", "subscribe")
        ) and row["disposition"] != "protected_parity_required":
            errors.append(f"{label}: billing/provider authority lost its protected gate")
        if row["legacy_path"].startswith("/api/webhooks/") and row["auth"] != "signed_webhook":
            errors.append(f"{label}: webhook is not bound to signed-webhook auth")
        if set(row["implementation"]) != {"rust_authority", "local_status"}:
            errors.append(f"{label}: implementation keys drifted")
        elif "/api/webhooks/media-server/" in row["legacy_path"] and row[
            "implementation"
        ]["local_status"] != (
            "rust_authority_present_issue_28_adapter_or_protected_evidence_pending"
        ):
            errors.append(f"{label}: media callback lost its issue 28 evidence gate")
        if set(row["security"]) != {
            "max_body_bytes",
            "rate_limit_bucket",
            "idempotency",
            "tenant_non_disclosure",
        }:
            errors.append(f"{label}: security keys drifted")
        if not isinstance(row["security"]["max_body_bytes"], int) or not 0 <= row["security"][
            "max_body_bytes"
        ] <= 8 * 1024 * 1024:
            errors.append(f"{label}: body limit is invalid")
        if row["security"]["idempotency"] not in {"forbidden", "optional", "required"}:
            errors.append(f"{label}: idempotency policy is invalid")
        if set(row["contract_evidence"]) != {
            "success",
            "validation",
            "authorization",
            "idempotency_retry",
            "failure",
        }:
            errors.append(f"{label}: evidence axes drifted")
        elif any(value not in EVIDENCE_VALUES for value in row["contract_evidence"].values()):
            errors.append(f"{label}: evidence value is invalid")
        if set(row["deprecation"]) != {
            "state",
            "earliest_removal",
            "migration_path",
            "approval",
        }:
            errors.append(f"{label}: deprecation keys drifted")
        if row["disposition"] == "retire" and (
            not row["deprecation"]["migration_path"] or not row["deprecation"]["approval"]
        ):
            errors.append(f"{label}: retirement lacks migration/approval state")
        for source in row["sources"]:
            if set(source) != {"path", "symbol", "sha256"}:
                errors.append(f"{label}: source keys drifted")
                continue
            if source["path"].startswith(("/", "../")) or not re.fullmatch(
                r"[0-9a-f]{64}", source["sha256"]
            ):
                errors.append(f"{label}: source identity is unsafe")
            source_path = CAP / source["path"]
            if compare_reference and source_path.is_file():
                if sha256_bytes(source_path.read_bytes()) != source["sha256"]:
                    errors.append(f"{label}: source checksum drifted for {source['path']}")

    expected_summary = build_summary(entries)
    if report["summary"] != expected_summary:
        errors.append("summary does not match entries")
    if entries != sorted(entries, key=lambda row: (row["kind"], row["legacy_path"], row["method"])):
        errors.append("entries are not deterministically sorted")

    if compare_reference:
        try:
            expected = build_report()
        except (OSError, RuntimeError, ValueError) as error:
            errors.append(f"reference extraction failed: {error}")
        else:
            if expected != report:
                errors.append("committed report differs from the pinned Cap extraction")
    return errors


def build_summary(entries: list[dict[str, Any]]) -> dict[str, Any]:
    by_kind = Counter(row["kind"] for row in entries)
    by_disposition = Counter(row["disposition"] for row in entries)
    by_status = Counter(row["implementation"]["local_status"] for row in entries)
    return {
        "total": len(entries),
        "by_kind": dict(sorted(by_kind.items())),
        "by_disposition": dict(sorted(by_disposition.items())),
        "by_local_status": dict(sorted(by_status.items())),
        "endpoint_success_proven": sum(
            row["contract_evidence"]["success"] == "local_contract" for row in entries
        ),
        "endpoint_success_pending": sum(
            row["contract_evidence"]["success"] != "local_contract" for row in entries
        ),
    }


def reference_commit() -> str | None:
    if not (CAP / ".git").exists():
        return None
    result = subprocess.run(
        ["git", "-C", str(CAP), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--generate", action="store_true")
    parser.add_argument("--require-reference", action="store_true")
    args = parser.parse_args()

    commit = reference_commit()
    if (args.generate or args.require_reference) and commit != REFERENCE_COMMIT:
        print(
            f"expected pinned Cap checkout {REFERENCE_COMMIT}, found {commit or 'none'}",
            file=sys.stderr,
        )
        return 1
    compare_reference = commit == REFERENCE_COMMIT

    if args.generate:
        report = build_report()
        REPORT.parent.mkdir(parents=True, exist_ok=True)
        DOC.parent.mkdir(parents=True, exist_ok=True)
        REPORT.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        DOC.write_text(render_doc(report), encoding="utf-8")

    try:
        report = json.loads(REPORT.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"unable to load API parity report: {error}", file=sys.stderr)
        return 1

    errors = validate_report(report, compare_reference=compare_reference)
    expected_doc = render_doc(report)
    try:
        actual_doc = DOC.read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"unable to load generated API documentation: {error}")
    else:
        if actual_doc != expected_doc:
            errors.append("generated API documentation differs from the report")

    if errors:
        print("API/workflow parity validation failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    mode = "with pinned-reference drift comparison" if compare_reference else "offline"
    print(
        f"API/workflow parity validation passed {mode}: "
        f"{report['summary']['total']} entries; "
        f"{report['summary']['endpoint_success_pending']} endpoint/retirement gates remain explicit."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
