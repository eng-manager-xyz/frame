#!/usr/bin/env python3
"""Build a bounded, allowlisted, privacy-safe operational support bundle."""

from __future__ import annotations

import argparse
import collections
import json
import pathlib
import re
import sys
import tempfile
import uuid
from typing import Any, Iterable


ROOT = pathlib.Path(__file__).resolve().parents[2]
CATALOG = ROOT / "fixtures/operational-hardening/v1/service-catalog.json"
POLICY = ROOT / "fixtures/operational-hardening/v1/operational-policy.json"
HEX = frozenset("0123456789abcdef")
SAFE_ATOM = re.compile(r"^[a-z][a-z0-9_]{0,63}$")
PROFILE = re.compile(r"^[a-z][a-z0-9_]{0,47}_v[1-9][0-9]{0,5}$")
FORBIDDEN_TEXT = [
    re.compile(r"(?i)bearer\s+\S+"),
    re.compile(r"(?i)(token|password|secret|cookie|authorization)\s*[:=]\s*\S+"),
    re.compile(r"(?i)(x-amz-signature|x-amz-credential|signed[_-]?url)"),
    re.compile(r"[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
    re.compile(r"-----BEGIN [A-Z ]+PRIVATE KEY-----"),
    re.compile(r"(?:^|\s)(?:/Users/|/home/|[A-Za-z]:\\Users\\)"),
]


class BundleError(RuntimeError):
    pass


def load_json(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise BundleError(f"cannot read policy {path}: {error}") from error
    if not isinstance(value, dict):
        raise BundleError(f"policy {path} must be a JSON object")
    return value


def load_contract() -> tuple[dict[str, set[str]], dict[str, Any]]:
    catalog = load_json(CATALOG)
    policy = load_json(POLICY)
    services: dict[str, set[str]] = {}
    for service in catalog.get("services", []):
        if isinstance(service, dict) and isinstance(service.get("id"), str):
            operations = service.get("safe_operations", [])
            services[service["id"]] = {item for item in operations if isinstance(item, str)}
    if not services:
        raise BundleError("service catalog contains no services")
    return services, policy["telemetry"]["event_schema"]


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def validate_text(value: Any, label: str, pattern: re.Pattern[str] = SAFE_ATOM) -> str:
    if not isinstance(value, str) or not pattern.fullmatch(value):
        raise BundleError(f"{label} is not an allowlisted stable identifier")
    for forbidden in FORBIDDEN_TEXT:
        if forbidden.search(value):
            raise BundleError(f"{label} contains forbidden diagnostic data")
    return value


def validate_integer(value: Any, label: str, minimum: int, maximum: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or not minimum <= value <= maximum:
        raise BundleError(f"{label} is outside its safe bound")
    return value


def validate_release(value: Any) -> str:
    if not isinstance(value, str) or len(value) != 40 or not set(value) <= HEX:
        raise BundleError("release_id must be a full lowercase Git SHA")
    return value


def validate_correlation(value: Any) -> str:
    if not isinstance(value, str):
        raise BundleError("correlation_id must be a UUIDv4 string")
    try:
        parsed = uuid.UUID(value)
    except ValueError as error:
        raise BundleError("correlation_id must be a UUIDv4 string") from error
    if parsed.version != 4 or parsed.variant != uuid.RFC_4122 or str(parsed) != value:
        raise BundleError("correlation_id must be canonical random UUIDv4")
    return value


def validate_event(
    event: Any,
    services: dict[str, set[str]],
    schema: dict[str, Any],
) -> dict[str, Any]:
    if not isinstance(event, dict):
        raise BundleError("each diagnostic event must be a JSON object")
    required = set(schema["required"])
    optional = set(schema["optional"])
    keys = set(event)
    if keys - required - optional:
        raise BundleError(f"diagnostic event has forbidden fields: {sorted(keys - required - optional)}")
    if required - keys:
        raise BundleError(f"diagnostic event is missing fields: {sorted(required - keys)}")
    if event["schema_version"] != 1:
        raise BundleError("event schema_version must equal 1")
    service = validate_text(event["service"], "service")
    if service not in services:
        raise BundleError("service is not in the operational catalog")
    operation = validate_text(event["operation"], "operation")
    if operation not in services[service]:
        raise BundleError("operation is not allowlisted for the service")
    environment = validate_text(event["environment"], "environment")
    if environment not in schema["environment_values"]:
        raise BundleError("environment is not allowlisted")
    normalized: dict[str, Any] = {
        "schema_version": 1,
        "timestamp_ms": validate_integer(
            event["timestamp_ms"], "timestamp_ms", 0, 9_999_999_999_999
        ),
        "service": service,
        "release_id": validate_release(event["release_id"]),
        "environment": environment,
        "operation": operation,
        "result_class": validate_text(event["result_class"], "result_class"),
        "duration_ms": validate_integer(event["duration_ms"], "duration_ms", 0, 86_400_000),
        "correlation_id": validate_correlation(event["correlation_id"]),
    }
    if "attempt" in event:
        normalized["attempt"] = validate_integer(event["attempt"], "attempt", 1, 32)
    if "queue_age_ms" in event:
        normalized["queue_age_ms"] = validate_integer(
            event["queue_age_ms"], "queue_age_ms", 0, 604_800_000
        )
    if "bytes_bucket" in event:
        bucket = validate_text(event["bytes_bucket"], "bytes_bucket")
        if bucket not in {"zero", "lt_1k", "lt_1m", "lt_100m", "gte_100m"}:
            raise BundleError("bytes_bucket is not allowlisted")
        normalized["bytes_bucket"] = bucket
    if "executor" in event:
        executor = validate_text(event["executor"], "executor")
        if executor not in {"control_plane", "managed_media", "native_gstreamer", "client"}:
            raise BundleError("executor is not allowlisted")
        normalized["executor"] = executor
    if "profile_revision" in event:
        normalized["profile_revision"] = validate_text(
            event["profile_revision"], "profile_revision", PROFILE
        )
    return normalized


def parse_events(
    lines: Iterable[str], services: dict[str, set[str]], schema: dict[str, Any]
) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    maximum = schema["maximum_events_per_bundle"]
    total_bytes = 0
    for line_number, line in enumerate(lines, 1):
        total_bytes += len(line.encode("utf-8"))
        if total_bytes > schema["maximum_serialized_bytes"]:
            raise BundleError("diagnostic input exceeds its byte bound")
        if not line.strip():
            continue
        if len(events) >= maximum:
            raise BundleError("diagnostic input exceeds its event bound")
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise BundleError(f"diagnostic line {line_number} is invalid JSON") from error
        events.append(validate_event(value, services, schema))
    if not events:
        raise BundleError("diagnostic bundle must contain at least one event")
    return events


def build_bundle(events: list[dict[str, Any]]) -> dict[str, Any]:
    ordered = sorted(
        events,
        key=lambda event: (event["timestamp_ms"], event["service"], event["correlation_id"]),
    )
    counts = collections.Counter(
        (event["service"], event["operation"], event["result_class"]) for event in ordered
    )
    return {
        "schema_version": 1,
        "evidence_scope": "privacy_safe_operational_events_no_payloads_or_identifiers",
        "started_at_ms": ordered[0]["timestamp_ms"],
        "ended_at_ms": ordered[-1]["timestamp_ms"],
        "event_count": len(ordered),
        "summary": [
            {
                "service": service,
                "operation": operation,
                "result_class": result_class,
                "count": count,
            }
            for (service, operation, result_class), count in sorted(counts.items())
        ],
        "events": ordered,
    }


def privacy_scan(payload: bytes) -> None:
    if len(payload) > 1_048_576:
        raise BundleError("diagnostic bundle exceeds its serialized byte bound")
    text = payload.decode("utf-8")
    for forbidden in FORBIDDEN_TEXT:
        if forbidden.search(text):
            raise BundleError("diagnostic bundle contains a forbidden data pattern")


def write_atomic(path: pathlib.Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    try:
        with temporary.open("xb") as handle:
            handle.write(payload)
        temporary.replace(path)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise BundleError(f"cannot write bundle {path}: {error}") from error


def safe_fixture() -> dict[str, Any]:
    return {
        "schema_version": 1,
        "timestamp_ms": 1_750_000_000_000,
        "service": "control_plane_worker",
        "release_id": "a" * 40,
        "environment": "ci",
        "operation": "request",
        "result_class": "dependency_unavailable",
        "duration_ms": 42,
        "correlation_id": "8b1454e8-5d5a-4c44-9c18-8e9bb2d0b9f1",
    }


def self_test() -> None:
    services, schema = load_contract()
    event = safe_fixture()
    validated = validate_event(event, services, schema)
    bundle = build_bundle([validated])
    payload = canonical(bundle)
    privacy_scan(payload)
    if b"dependency_unavailable" not in payload:
        raise BundleError("safe diagnostic fixture was not retained")

    unsafe: list[dict[str, Any]] = []
    for field, value in (
        ("raw_email", "person@example.invalid"),
        ("token", "generated-token-material"),
        ("signed_url", "https://example.invalid/o?x-amz-signature=generated"),
        ("captions", "synthetic spoken words"),
        ("media_bytes", "generated-media-payload"),
    ):
        candidate = dict(event)
        candidate[field] = value
        unsafe.append(candidate)
    candidate = dict(event)
    candidate["result_class"] = "provider said person@example.invalid"
    unsafe.append(candidate)
    candidate = dict(event)
    candidate["correlation_id"] = "tenant-a-object-b"
    unsafe.append(candidate)
    for candidate in unsafe:
        try:
            validate_event(candidate, services, schema)
        except BundleError:
            continue
        raise BundleError("unsafe diagnostic fixture unexpectedly passed")

    with tempfile.TemporaryDirectory(prefix="frame-support-") as temporary:
        output = pathlib.Path(temporary) / "bundle.json"
        write_atomic(output, payload)
        if output.read_bytes() != payload:
            raise BundleError("support bundle atomic write was not reproducible")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--events", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        if args.self_test:
            if args.events is not None or args.output is not None:
                raise BundleError("--self-test cannot be combined with input or output")
            self_test()
            print("privacy-safe diagnostic bundle self-test passed")
            return 0
        if args.events is None or args.output is None:
            raise BundleError("--events and --output are required outside --self-test")
        services, schema = load_contract()
        try:
            with args.events.open(encoding="utf-8") as handle:
                events = parse_events(handle, services, schema)
        except OSError as error:
            raise BundleError(f"cannot read diagnostic events: {error}") from error
        payload = canonical(build_bundle(events))
        privacy_scan(payload)
        write_atomic(args.output, payload)
        print(f"wrote privacy-safe support bundle with {len(events)} events: {args.output}")
        return 0
    except BundleError as error:
        print(f"support bundle failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
