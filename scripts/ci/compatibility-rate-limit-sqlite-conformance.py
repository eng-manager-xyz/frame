#!/usr/bin/env python3
"""Exercise the exact compatibility fixed-window D1 SQL against SQLite."""

from __future__ import annotations

import json
import pathlib
import sqlite3


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATION = (
    ROOT
    / "apps"
    / "control-plane"
    / "migrations"
    / "0034_compatibility_rate_limits.sql"
)
QUERY_ROOT = ROOT / "apps" / "control-plane" / "queries" / "api_workflow"
ADMIT = (QUERY_ROOT / "compatibility_rate_limit_admit.sql").read_text(encoding="utf-8")
CLEANUP = (QUERY_ROOT / "compatibility_rate_limit_cleanup.sql").read_text(encoding="utf-8")
CONTROL_PLANE = (ROOT / "apps" / "control-plane" / "src" / "lib.rs").read_text(
    encoding="utf-8"
)
BROWSER_RUNTIME = (
    ROOT / "apps" / "control-plane" / "src" / "browser_web_runtime.rs"
).read_text(encoding="utf-8")
WINDOW_MS = 60_000


def admit(
    database: sqlite3.Connection,
    *,
    bucket: str,
    dimension: str,
    key_version: int,
    digest: str,
    now_ms: int,
    limit: int,
) -> int | None:
    database.execute(CLEANUP, (now_ms,))
    row = database.execute(
        ADMIT,
        (
            bucket,
            dimension,
            key_version,
            digest,
            now_ms,
            now_ms + 2 * WINDOW_MS,
            now_ms - WINDOW_MS,
            limit,
        ),
    ).fetchone()
    return None if row is None else int(row[0])


def main() -> int:
    required_runtime_markers = (
        "compatibility_rate_limit::admit_edge_request(",
        "CompatibilityRateLimitBucketV1::ClientCompatibility",
        "CompatibilityRateLimitBucketV1::ServiceMisc",
        "CompatibilityRateLimitBucketV1::CollaborationNotifications",
        "compatibility_rate_limit::admit_principal(",
        "rate_limit,",
        '.set("retry-after", &seconds.to_string())',
    )
    for marker in required_runtime_markers:
        assert marker in CONTROL_PLANE, f"missing production limiter wiring: {marker}"
    assert "rate_limit: RateLimitDecisionV1::Allowed" not in CONTROL_PLANE
    assert CONTROL_PLANE.count(
        ".with_retry_after_seconds(compatibility_rate_limit::RETRY_AFTER_SECONDS)"
    ) == 3
    action_start = BROWSER_RUNTIME.index("if action == WebAction::SetActiveOrganization")
    action_end = BROWSER_RUNTIME.index("execute_action(", action_start)
    action_ingress = BROWSER_RUNTIME[action_start:action_end]
    for marker in (
        "compatibility_rate_limit::admit_principal(",
        "CompatibilityRateLimitBucketV1::OrganizationLibrary",
        "BrowserWebFailure::RateLimited",
    ):
        assert marker in action_ingress, f"missing active-organization limiter wiring: {marker}"
    assert action_ingress.index("admit_principal(") < action_ingress.index(
        "authenticate_mutation("
    ), "active-organization limiter must run before one-use mutation grant issuance"

    missing = sqlite3.connect(":memory:")
    try:
        admit(
            missing,
            bucket="client_compatibility.v1",
            dimension="source",
            key_version=1,
            digest="0" * 64,
            now_ms=1,
            limit=12,
        )
    except sqlite3.OperationalError as error:
        if "no such table: compatibility_rate_limit_buckets_v1" not in str(error):
            raise
    else:
        raise AssertionError("missing limiter authority did not fail closed")

    database = sqlite3.connect(":memory:")
    database.executescript(MIGRATION.read_text(encoding="utf-8"))
    subject_a = "a" * 64
    subject_b = "b" * 64
    counts = [
        admit(
            database,
            bucket="client_compatibility.v1",
            dimension="source",
            key_version=7,
            digest=subject_a,
            now_ms=1_000,
            limit=12,
        )
        for _ in range(13)
    ]
    assert counts == [*range(1, 13), None], counts
    stored = database.execute(
        """SELECT request_count FROM compatibility_rate_limit_buckets_v1
           WHERE bucket='client_compatibility.v1' AND dimension='source'
             AND key_version=7 AND subject_digest=?""",
        (subject_a,),
    ).fetchone()
    assert stored == (12,), stored

    assert (
        admit(
            database,
            bucket="client_compatibility.v1",
            dimension="source",
            key_version=7,
            digest=subject_b,
            now_ms=1_000,
            limit=12,
        )
        == 1
    )
    assert (
        admit(
            database,
            bucket="service_misc.v1",
            dimension="source",
            key_version=7,
            digest=subject_a,
            now_ms=1_000,
            limit=120,
        )
        == 1
    )
    assert (
        admit(
            database,
            bucket="client_compatibility.v1",
            dimension="source",
            key_version=7,
            digest=subject_a,
            now_ms=61_000,
            limit=12,
        )
        == 1
    )
    # A regressed clock cannot reset or increment an existing bucket.
    assert (
        admit(
            database,
            bucket="client_compatibility.v1",
            dimension="source",
            key_version=7,
            digest=subject_a,
            now_ms=60_999,
            limit=12,
        )
        is None
    )

    for index in range(20):
        digest = f"{index:064x}"
        database.execute(
            """INSERT INTO compatibility_rate_limit_buckets_v1(
                 bucket,dimension,key_version,subject_digest,window_started_at_ms,
                 request_count,updated_at_ms,gc_at_ms
               ) VALUES ('organization_library.v1','principal',1,?,1,1,1,2)""",
            (digest,),
        )
    database.execute(CLEANUP, (2,))
    expired_remaining = database.execute(
        "SELECT COUNT(*) FROM compatibility_rate_limit_buckets_v1 WHERE gc_at_ms <= 2"
    ).fetchone()[0]
    assert expired_remaining == 4, expired_remaining

    print(
        json.dumps(
            {
                "bounded_cleanup": True,
                "fail_closed_without_authority": True,
                "fixed_window_limit": 12,
                "independent_bucket_and_subject_keys": True,
                "production_wiring_static": True,
                "regressed_clock_rejected": True,
                "status": "ok",
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
