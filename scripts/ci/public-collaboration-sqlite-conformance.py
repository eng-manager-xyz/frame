#!/usr/bin/env python3
"""Adversarial SQLite conformance for public collaboration D1 state.

This is deliberately an offline database test. It proves that the checked-in
migration rejects cross-share/cross-tenant bindings, enforces atomic rate
limits, and keeps the collaboration audit append-only. Worker and browser
delivery are covered by their respective integration gates.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import sqlite3
import sys
from collections.abc import Callable


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
NOW = 1_700_400_000_000

USER_A = "018f47a6-7b1c-7f55-8f39-8f8a8690a501"
USER_B = "018f47a6-7b1c-7f55-8f39-8f8a8690a502"
ORG_A = "018f47a6-7b1c-7f55-8f39-8f8a8690b501"
ORG_B = "018f47a6-7b1c-7f55-8f39-8f8a8690b502"
SHARE_A = "018f47a6-7b1c-7f55-8f39-8f8a8690c501"
SHARE_B = "018f47a6-7b1c-7f55-8f39-8f8a8690c502"
COMMENT_A = "018f47a6-7b1c-7f55-8f39-8f8a8690d501"
COMMENT_B = "018f47a6-7b1c-7f55-8f39-8f8a8690d502"
TOKEN_A = hashlib.sha256(b"public-grant-a").hexdigest()
TOKEN_A_ROTATED = hashlib.sha256(b"public-grant-a-rotated").hexdigest()
TOKEN_B = hashlib.sha256(b"public-grant-b").hexdigest()
ANON_A = hashlib.sha256(b"anonymous-a").hexdigest()
ANON_B = hashlib.sha256(b"anonymous-b").hexdigest()


class ConformanceFailure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConformanceFailure(message)


def operation(number: int) -> str:
    return f"018f47a6-7b1c-7f55-8f39-{number:012d}"


def migrate(database: sqlite3.Connection) -> None:
    database.execute("PRAGMA foreign_keys = ON")
    files = sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
    selected = [path for path in files if int(path.name[:4]) <= 22]
    require(
        [int(path.name[:4]) for path in selected] == list(range(1, 23)),
        "migration sequence through 0022 is not contiguous",
    )
    for path in selected:
        database.executescript(path.read_text(encoding="utf-8"))


def expect_integrity(operation: Callable[[], object], fragment: str) -> None:
    try:
        operation()
    except sqlite3.IntegrityError as error:
        require(fragment in str(error), f"wrong integrity failure: {error}")
    else:
        raise ConformanceFailure(f"expected integrity failure containing {fragment!r}")


def add_user(database: sqlite3.Connection, user_id: str, label: str) -> None:
    database.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) "
        "VALUES (?,?,?,?,?)",
        (user_id, f"{label}@sqlite.invalid", label, NOW - 10_000, NOW - 10_000),
    )


def add_organization(
    database: sqlite3.Connection, organization_id: str, owner_id: str, label: str
) -> None:
    database.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,"
        "updated_at_ms,tombstoned_at_ms,revision,authority_version,retention_until_ms,"
        "recovered_at_ms,last_operation_id) "
        "VALUES (?,?,?,'active','{}',?,?,NULL,0,0,NULL,NULL,?)",
        (organization_id, owner_id, label, NOW - 9_000, NOW - 9_000, operation(1)),
    )
    database.execute(
        "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,"
        "created_at_ms,updated_at_ms,revision,authority_version,last_operation_id) "
        "VALUES (?,?,'owner','active',0,?,?,0,0,?)",
        (organization_id, owner_id, NOW - 8_000, NOW - 8_000, operation(2)),
    )


def add_video(
    database: sqlite3.Connection,
    share_id: str,
    organization_id: str,
    owner_id: str,
) -> None:
    document = json.dumps(
        {"schema_version": 1, "title": share_id[-4:]},
        sort_keys=True,
        separators=(",", ":"),
    )
    database.execute(
        "INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,"
        "organization_id,privacy,metadata_json,revision,metadata_schema_version,"
        "metadata_checksum,comments_enabled,last_operation_id,duration_ms) "
        "VALUES (?,?,?,'ready',?,?,?,?,?,1,1,?,1,?,60000)",
        (
            share_id,
            owner_id,
            f"Share {share_id[-4:]}",
            NOW - 7_000,
            NOW - 7_000,
            organization_id,
            "public",
            document,
            hashlib.sha256(document.encode()).hexdigest(),
            operation(3),
        ),
    )


def add_comment(
    database: sqlite3.Connection,
    comment_id: str,
    share_id: str,
    organization_id: str,
    anonymous_digest: str,
) -> None:
    database.execute(
        "INSERT INTO comments(id,video_id,parent_comment_id,author_user_id,"
        "anonymous_author_digest,body,created_at_ms,updated_at_ms,deleted_at_ms,revision,"
        "organization_id,last_operation_id,comment_kind,timeline_micros) "
        "VALUES (?,?,NULL,NULL,?,'hello',?,?,NULL,1,?,?, 'text',NULL)",
        (
            comment_id,
            share_id,
            anonymous_digest,
            NOW - 4_000,
            NOW - 4_000,
            organization_id,
            operation(4),
        ),
    )


def seed(database: sqlite3.Connection) -> None:
    add_user(database, USER_A, "owner-a")
    add_user(database, USER_B, "owner-b")
    add_organization(database, ORG_A, USER_A, "Organization A")
    add_organization(database, ORG_B, USER_B, "Organization B")
    add_video(database, SHARE_A, ORG_A, USER_A)
    add_video(database, SHARE_B, ORG_B, USER_B)
    for organization_id in (ORG_A, ORG_B):
        database.execute(
            "INSERT INTO public_collaboration_policies_v1(organization_id,"
            "anonymous_comments_enabled,comment_moderation,comment_maximum_per_minute,"
            "analytics_enabled,analytics_consent_required,analytics_policy_version,"
            "analytics_retention_days,analytics_maximum_per_minute,revision,updated_at_ms) "
            "VALUES (?,1,'publish',1,1,1,'policy-v1',30,1,1,?)",
            (organization_id, NOW),
        )
    for token, share_id, organization_id in (
        (TOKEN_A, SHARE_A, ORG_A),
        (TOKEN_A_ROTATED, SHARE_A, ORG_A),
        (TOKEN_B, SHARE_B, ORG_B),
    ):
        database.execute(
            "INSERT INTO public_collaboration_grants_v1(token_digest,share_id,"
            "organization_id,comments_enabled,analytics_enabled,analytics_policy_version,"
            "issued_at_ms,expires_at_ms,revoked_at_ms) VALUES (?,?,?,1,1,'policy-v1',?,?,NULL)",
            (token, share_id, organization_id, NOW, NOW + 3_600_000),
        )
    add_comment(database, COMMENT_A, SHARE_A, ORG_A, ANON_A)
    add_comment(database, COMMENT_B, SHARE_B, ORG_B, ANON_B)


def cross_tenant_bindings(database: sqlite3.Connection) -> int:
    rejected = 0

    def reject(
        sql: str,
        bindings: tuple[object, ...],
        fragment: str = "FOREIGN KEY constraint failed",
    ) -> None:
        nonlocal rejected
        expect_integrity(lambda: database.execute(sql, bindings), fragment)
        rejected += 1

    reject(
        "INSERT INTO public_collaboration_grants_v1(token_digest,share_id,organization_id,"
        "comments_enabled,analytics_enabled,analytics_policy_version,issued_at_ms,expires_at_ms) "
        "VALUES (?,?,?,1,1,'policy-v1',?,?)",
        (hashlib.sha256(b"bad-grant").hexdigest(), SHARE_A, ORG_B, NOW, NOW + 1_000),
    )
    reject(
        "INSERT INTO public_comment_moderation_v1(comment_id,share_id,state,decided_at_ms,revision) "
        "VALUES (?,?,'published',?,1)",
        (COMMENT_A, SHARE_B, NOW),
    )
    reject(
        "INSERT INTO public_comment_operations_v1(operation_id,token_digest,share_id,"
        "payload_digest,comment_id,response_json,created_at_ms,expires_at_ms) "
        "VALUES (?,?,?,?,?,'{}',?,?)",
        (operation(10), TOKEN_B, SHARE_A, TOKEN_A, COMMENT_A, NOW, NOW + 1_000),
    )
    reject(
        "INSERT INTO public_comment_operations_v1(operation_id,token_digest,share_id,"
        "payload_digest,comment_id,response_json,created_at_ms,expires_at_ms) "
        "VALUES (?,?,?,?,?,'{}',?,?)",
        (operation(11), TOKEN_A, SHARE_A, TOKEN_A, COMMENT_B, NOW, NOW + 1_000),
    )
    reject(
        "INSERT INTO public_analytics_consents_v1(token_digest,share_id,policy_version,state,"
        "granted_at_ms,expires_at_ms,revision,last_operation_id) "
        "VALUES (?,?,'policy-v1','granted',?,?,1,?)",
        (TOKEN_B, SHARE_A, NOW, NOW + 1_000, operation(12)),
    )
    reject(
        "INSERT INTO public_analytics_consent_operations_v1(operation_id,token_digest,"
        "share_id,payload_digest,response_json,created_at_ms,expires_at_ms) "
        "VALUES (?,?,?,?,'{}',?,?)",
        (operation(13), TOKEN_B, SHARE_A, TOKEN_A, NOW, NOW + 1_000),
    )
    reject(
        "INSERT INTO public_analytics_events_v1(operation_id,token_digest,share_id,"
        "policy_version,payload_digest,sequence,kind,position_ms,occurred_at_ms,"
        "recorded_at_ms,expires_at_ms) VALUES (?,?,?,'policy-v1',?,1,"
        "'playback_started',0,?,?,?)",
        (operation(14), TOKEN_B, SHARE_A, TOKEN_A, NOW, NOW, NOW + 1_000),
    )
    reject(
        "INSERT INTO public_collaboration_rate_events_v1(id,token_digest,share_id,action,"
        "accepted_at_ms,expires_at_ms) VALUES (?,?,?,'comment',?,?)",
        (operation(15), TOKEN_B, SHARE_A, NOW, NOW + 1_000),
    )
    reject(
        "INSERT INTO public_transcripts_v1(share_id,organization_id,revision,language,"
        "duration_ms,document_json,document_checksum,is_current,published_at_ms,"
        "published_by_user_id) VALUES (?,?,1,'en',60000,'{}',?,1,?,?)",
        (SHARE_A, ORG_B, TOKEN_A, NOW, USER_A),
    )
    return rejected


def atomic_limits_and_immutability(database: sqlite3.Connection) -> dict[str, int]:
    database.execute(
        "INSERT INTO public_collaboration_rate_events_v1(id,token_digest,share_id,action,"
        "accepted_at_ms,expires_at_ms) VALUES (?,?,?,'comment',?,?)",
        (operation(20), TOKEN_A, SHARE_A, NOW, NOW + 60_000),
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO public_collaboration_rate_events_v1(id,token_digest,share_id,action,"
            "accepted_at_ms,expires_at_ms) VALUES (?,?,?,'comment',?,?)",
            (operation(21), TOKEN_A_ROTATED, SHARE_A, NOW + 1, NOW + 60_001),
        ),
        "frame_public_collaboration_rate_limited_v1",
    )
    # A distinct share has an independent global bucket.
    database.execute(
        "INSERT INTO public_collaboration_rate_events_v1(id,token_digest,share_id,action,"
        "accepted_at_ms,expires_at_ms) VALUES (?,?,?,'comment',?,?)",
        (operation(22), TOKEN_B, SHARE_B, NOW + 1, NOW + 60_001),
    )

    for index in range(120):
        database.execute(
            "INSERT INTO public_collaboration_grant_rate_v1(id,share_id,accepted_at_ms,expires_at_ms) "
            "VALUES (?,?,?,?)",
            (operation(100 + index), SHARE_A, NOW + index, NOW + 120_000 + index),
        )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO public_collaboration_grant_rate_v1(id,share_id,accepted_at_ms,expires_at_ms) "
            "VALUES (?,?,?,?)",
            (operation(220), SHARE_A, NOW + 120, NOW + 120_120),
        ),
        "frame_public_collaboration_grant_rate_limited_v1",
    )
    database.execute(
        "INSERT INTO public_collaboration_grant_rate_v1(id,share_id,accepted_at_ms,expires_at_ms) "
        "VALUES (?,?,?,?)",
        (operation(221), SHARE_B, NOW + 120, NOW + 120_120),
    )

    # A globally unique operation ID is replayable only inside the original
    # token+share authority scope. The same payload cannot cross that scope.
    payload = hashlib.sha256(b"same-comment-payload").hexdigest()
    database.execute(
        "INSERT INTO public_comment_operations_v1(operation_id,token_digest,share_id,"
        "payload_digest,comment_id,response_json,created_at_ms,expires_at_ms) "
        "VALUES (?,?,?,?,?,'{}',?,?)",
        (operation(230), TOKEN_A, SHARE_A, payload, COMMENT_A, NOW, NOW + 60_000),
    )
    scoped = database.execute(
        "SELECT payload_digest,response_json FROM public_comment_operations_v1 "
        "WHERE operation_id=? AND token_digest=? AND share_id=? LIMIT 1",
        (operation(230), TOKEN_B, SHARE_B),
    ).fetchone()
    require(scoped is None, "cross-share operation was replayable")
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO public_comment_operations_v1(operation_id,token_digest,share_id,"
            "payload_digest,comment_id,response_json,created_at_ms,expires_at_ms) "
            "VALUES (?,?,?,?,?,'{}',?,?)",
            (operation(230), TOKEN_B, SHARE_B, payload, COMMENT_B, NOW, NOW + 60_000),
        ),
        "UNIQUE constraint failed",
    )
    checksum = hashlib.sha256(b"{}").hexdigest()
    database.execute(
        "INSERT INTO public_transcripts_v1(share_id,organization_id,revision,language,"
        "duration_ms,document_json,document_checksum,is_current,published_at_ms,"
        "published_by_user_id) VALUES (?,?,1,'en',60000,'{}',?,1,?,?)",
        (SHARE_A, ORG_A, checksum, NOW, USER_A),
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO public_transcripts_v1(share_id,organization_id,revision,language,"
            "duration_ms,document_json,document_checksum,is_current,published_at_ms,"
            "published_by_user_id) VALUES (?,?,2,'en',60000,'{}',?,1,?,?)",
            (SHARE_A, ORG_A, checksum, NOW + 1, USER_A),
        ),
        "UNIQUE constraint failed",
    )
    database.execute(
        "INSERT INTO public_collaboration_audit_v1(id,share_id,token_digest,action,outcome,"
        "correlation_id,occurred_at_ms) VALUES (?,?,?,'grant_issued','applied','test',?)",
        (operation(30), SHARE_A, TOKEN_A, NOW),
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE public_collaboration_audit_v1 SET outcome='ignored' WHERE id=?",
            (operation(30),),
        ),
        "public collaboration audit is immutable",
    )
    expect_integrity(
        lambda: database.execute(
            "DELETE FROM public_collaboration_audit_v1 WHERE id=?", (operation(30),)
        ),
        "public collaboration audit is immutable",
    )
    require(database.execute("PRAGMA foreign_key_check").fetchall() == [], "foreign key drift")
    return {
        "atomic_rate_caps": 2,
        "cross_share_rate_isolation": 2,
        "scope_bound_replay": 1,
        "current_transcript_guards": 1,
        "immutable_audit": 2,
    }


def main() -> int:
    database = sqlite3.connect(":memory:")
    try:
        migrate(database)
        seed(database)
        result = {
            "migration": "0022_public_collaboration_runtime.sql",
            "cross_tenant_bindings_rejected": cross_tenant_bindings(database),
            **atomic_limits_and_immutability(database),
        }
        print(json.dumps(result, sort_keys=True))
        return 0
    except (ConformanceFailure, sqlite3.Error) as error:
        print(f"public collaboration conformance failed: {error}", file=sys.stderr)
        return 1
    finally:
        database.close()


if __name__ == "__main__":
    raise SystemExit(main())
