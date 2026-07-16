#!/usr/bin/env python3
"""Offline semantic conformance for Issue-15 business metadata.

This suite executes the checked-in migrations and parameterized SQL against
SQLite. It intentionally does not claim Worker, Wrangler, D1-provider, R2,
payment-provider, email-provider, or browser validation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import sqlite3
import sys
from collections.abc import Sequence
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
CONTROL = ROOT / "apps" / "control-plane"
MIGRATIONS = CONTROL / "migrations"
QUERIES = CONTROL / "queries" / "business"
REPOSITORY = CONTROL / "src" / "business_repository.rs"
DOMAIN = ROOT / "crates" / "domain" / "src" / "business.rs"
PORT = ROOT / "crates" / "ports" / "src" / "business.rs"
APPLICATION = ROOT / "crates" / "application" / "src" / "business.rs"
SAMPLE_EXPORT = ROOT / "docs" / "evidence" / "business-data-sample-export-v1.json"
PLACEHOLDER = re.compile(r"\?([1-9][0-9]*)")
NOW = 1_700_300_000_000
BUSINESS_DATA_CLASSES = [
    "video_metadata",
    "video_edit",
    "share",
    "comment",
    "notification",
    "outbox",
    "storage_integration",
    "storage_object",
    "derivative_job",
    "upload",
    "import",
    "developer_app",
    "developer_domain",
    "developer_api_key",
    "developer_video",
    "credit_account",
    "credit_transaction",
    "usage_ledger",
    "daily_storage_snapshot",
    "messenger_legacy",
]

ORG_A = "018f47a6-7b1c-7f55-8f39-8f8a8690e401"
ORG_B = "018f47a6-7b1c-7f55-8f39-8f8a8690e402"
OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a401"
ADMIN = "018f47a6-7b1c-7f55-8f39-8f8a8690a402"
MEMBER = "018f47a6-7b1c-7f55-8f39-8f8a8690a403"
VIEWER = "018f47a6-7b1c-7f55-8f39-8f8a8690a404"
OUTSIDER = "018f47a6-7b1c-7f55-8f39-8f8a8690a405"
VIDEO_PRIVATE = "018f47a6-7b1c-7f55-8f39-8f8a8690b401"
VIDEO_UNLISTED = "018f47a6-7b1c-7f55-8f39-8f8a8690b402"
VIDEO_MEMBER = "018f47a6-7b1c-7f55-8f39-8f8a8690b403"
VIDEO_B = "018f47a6-7b1c-7f55-8f39-8f8a8690b404"
INTEGRATION_A = "018f47a6-7b1c-7f55-8f39-8f8a8690c401"
INTEGRATION_B = "018f47a6-7b1c-7f55-8f39-8f8a8690c402"
OBJECT_A = "018f47a6-7b1c-7f55-8f39-8f8a8690d401"
APP_A = "018f47a6-7b1c-7f55-8f39-8f8a8690e411"
APP_B = "018f47a6-7b1c-7f55-8f39-8f8a8690e412"
ACCOUNT_A = "018f47a6-7b1c-7f55-8f39-8f8a8690f401"
ANON_DIGEST = hashlib.sha256(b"anonymous-a").hexdigest()
OTHER_DIGEST = hashlib.sha256(b"anonymous-b").hexdigest()
INITIAL_EVENT_FINGERPRINT = hashlib.sha256(
    b"frame-business-ordered-lifecycle-initial-v1"
).hexdigest()


class ConformanceFailure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConformanceFailure(message)


def migration_files() -> list[pathlib.Path]:
    files = sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
    require(
        [int(path.name[:4]) for path in files] == list(range(1, len(files) + 1)),
        "migration sequence is not contiguous",
    )
    require(
        any(path.name == "0011_business_authority_expand.sql" for path in files),
        "0011 business migration is missing",
    )
    require(
        any(path.name == "0016_business_authority_enforce.sql" for path in files),
        "0016 business enforcement migration is missing",
    )
    require(
        any(path.name == "0017_business_document_enforce.sql" for path in files),
        "0017 business document enforcement migration is missing",
    )
    require(
        any(path.name == "0018_business_event_enforce.sql" for path in files),
        "0018 business event enforcement migration is missing",
    )
    require(
        any(path.name == "0019_business_lifecycle_enforce.sql" for path in files),
        "0019 business lifecycle enforcement migration is missing",
    )
    require(
        any(path.name == "0020_business_integrity_view.sql" for path in files),
        "0020 business integrity view migration is missing",
    )
    require(
        any(path.name == "0021_repository_business_outbox_compat.sql" for path in files),
        "0021 repository compatibility migration is missing",
    )
    return files


def migrate(database: sqlite3.Connection, *, through: int | None = None) -> None:
    database.execute("PRAGMA foreign_keys = ON")
    files = migration_files()
    if through is not None:
        files = [path for path in files if int(path.name[:4]) <= through]
    for path in files:
        database.executescript(path.read_text(encoding="utf-8"))


def query(name: str) -> str:
    return (QUERIES / name).read_text(encoding="utf-8").strip()


def execute_query(
    database: sqlite3.Connection, name: str, bindings: Sequence[Any]
) -> sqlite3.Cursor:
    return database.execute(query(name), bindings)


def expect_integrity(operation: Any, fragment: str) -> None:
    try:
        operation()
    except sqlite3.IntegrityError as error:
        require(fragment in str(error), f"wrong failure: {error}")
    else:
        raise ConformanceFailure(f"expected integrity error containing {fragment}")


def add_user(database: sqlite3.Connection, user_id: str, label: str) -> None:
    database.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) "
        "VALUES (?,?,?,?,?)",
        (user_id, f"{label}@sqlite.invalid", label, NOW - 10_000, NOW - 10_000),
    )
    database.execute(
        "INSERT INTO auth_identities_v2(user_id,identity_revision,session_version,"
        "created_at_ms,updated_at_ms,revision,last_operation_id) VALUES (?,1,0,?,?,0,?)",
        (user_id, NOW - 10_000, NOW - 10_000, user_id),
    )


def add_org(database: sqlite3.Connection, organization_id: str, owner: str, name: str) -> None:
    database.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,"
        "updated_at_ms,tombstoned_at_ms,revision,authority_version,retention_until_ms,"
        "recovered_at_ms,last_operation_id) VALUES (?,?,?,'active','{}',?,?,NULL,0,0,NULL,NULL,?)",
        (organization_id, owner, name, NOW - 9_000, NOW - 9_000, owner),
    )


def add_member(
    database: sqlite3.Connection,
    organization_id: str,
    user_id: str,
    role: str,
) -> None:
    database.execute(
        "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,"
        "created_at_ms,updated_at_ms,revision,authority_version,last_operation_id) "
        "VALUES (?,?,?,'active',0,?,?,0,0,?)",
        (organization_id, user_id, role, NOW - 8_000, NOW - 8_000, user_id),
    )


def canonical(value: dict[str, Any]) -> tuple[str, str]:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return encoded, hashlib.sha256(encoded.encode()).hexdigest()


def semantic_fingerprint(*components: str) -> str:
    """Mirror `business_semantic_fingerprint`, including its checksum wrapper."""
    digest = hashlib.sha256(b"frame-business-semantic-v1\0")
    for component in components:
        encoded = component.encode()
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
    return hashlib.sha256(digest.digest()).hexdigest()


def tenant_key(organization_id: str, purpose: str, logical_key: str) -> str:
    return semantic_fingerprint(
        "frame-business-tenant-key-v1", organization_id, purpose, logical_key
    )


def add_video(
    database: sqlite3.Connection,
    video_id: str,
    organization_id: str,
    owner_id: str,
    privacy: str,
) -> None:
    metadata, checksum = canonical({"schema_version": 1, "title": video_id[-4:]})
    database.execute(
        "INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,"
        "organization_id,privacy,metadata_json,revision,metadata_schema_version,"
        "metadata_checksum,comments_enabled,last_operation_id) "
        "VALUES (?,?,?,'ready',?,?,?,?,?,1,1,?,1,?)",
        (
            video_id,
            owner_id,
            f"Video {video_id[-4:]}",
            NOW - 7_000,
            NOW - 7_000,
            organization_id,
            privacy,
            metadata,
            checksum,
            owner_id,
        ),
    )


def seed_current(database: sqlite3.Connection) -> None:
    for user_id, label in (
        (OWNER, "owner"),
        (ADMIN, "admin"),
        (MEMBER, "member"),
        (VIEWER, "viewer"),
        (OUTSIDER, "outsider"),
    ):
        add_user(database, user_id, label)
    add_org(database, ORG_A, OWNER, "Organization A")
    add_org(database, ORG_B, OUTSIDER, "Organization B")
    for organization_id, user_id, role in (
        (ORG_A, OWNER, "owner"),
        (ORG_A, ADMIN, "admin"),
        (ORG_A, MEMBER, "member"),
        (ORG_A, VIEWER, "viewer"),
        (ORG_B, OUTSIDER, "owner"),
    ):
        add_member(database, organization_id, user_id, role)
    add_video(database, VIDEO_PRIVATE, ORG_A, OWNER, "private")
    add_video(database, VIDEO_UNLISTED, ORG_A, OWNER, "unlisted")
    add_video(database, VIDEO_MEMBER, ORG_A, MEMBER, "organization")
    add_video(database, VIDEO_B, ORG_B, OUTSIDER, "organization")


def clean_migration() -> dict[str, int]:
    database = sqlite3.connect(":memory:")
    migrate(database)
    require(database.execute("PRAGMA foreign_key_check").fetchall() == [], "foreign key failure")
    mapped = database.execute("SELECT COUNT(*) FROM business_source_table_map_v1").fetchone()[0]
    policies = database.execute(
        "SELECT COUNT(*) FROM business_data_handling_policies_v1"
    ).fetchone()[0]
    derived = database.execute(
        "SELECT COUNT(*) FROM business_derived_aggregate_map_v1"
    ).fetchone()[0]
    require(mapped == 20, "pinned Cap source mapping inventory is incomplete")
    require(derived == 1, "Frame-derived aggregate inventory is incomplete")
    require(policies == 20, "data handling matrix is incomplete")
    return {
        "mapped_source_tables": mapped,
        "derived_aggregates": derived,
        "data_classes": policies,
    }


def compile_queries() -> dict[str, int]:
    database = sqlite3.connect(":memory:")
    migrate(database)
    files = sorted(QUERIES.glob("*.sql"))
    require(len(files) >= 30, "business query inventory is incomplete")
    for path in files:
        sql = path.read_text(encoding="utf-8").strip()
        require(";" not in sql.rstrip(";"), f"multi-statement query: {path.name}")
        indexes = [int(match) for match in PLACEHOLDER.findall(sql)]
        require(indexes, f"query has no numbered bindings: {path.name}")
        database.execute("EXPLAIN " + sql, [None] * max(indexes)).fetchall()
    return {"compiled_queries": len(files)}


def seed_dirty_before_0011(database: sqlite3.Connection) -> None:
    add_user(database, OWNER, "dirty-owner")
    add_org(database, ORG_A, OWNER, "Dirty organization")
    add_member(database, ORG_A, OWNER, "owner")
    database.execute(
        "INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,"
        "organization_id,privacy,metadata_json,revision) "
        "VALUES (?,?,?,'ready',?,?,?,'public','{\"legacy\":true}',0)",
        (VIDEO_PRIVATE, OWNER, "Legacy", NOW - 5_000, NOW - 5_000, ORG_A),
    )
    database.execute(
        "INSERT INTO video_edits(id,video_id,document_version,edit_spec_json,"
        "created_by_user_id,created_at_ms,updated_at_ms,revision) "
        "VALUES (?,?,1,'{\"legacy\":true}',?,?,?,0)",
        (VIDEO_UNLISTED, VIDEO_PRIVATE, OWNER, NOW - 4_000, NOW - 4_000),
    )
    database.execute(
        "INSERT INTO comments(id,video_id,author_user_id,body,created_at_ms,updated_at_ms,revision) "
        "VALUES (?,?,?,'legacy',?,?,0)",
        (VIDEO_MEMBER, VIDEO_PRIVATE, OWNER, NOW - 3_000, NOW - 3_000),
    )
    database.execute(
        "INSERT INTO messenger_conversations(id,user_id,mode,created_at_ms,updated_at_ms,last_message_at_ms) "
        "VALUES (?,?, 'support',?,?,?)",
        (INTEGRATION_A, OWNER, NOW - 3_000, NOW - 3_000, NOW - 3_000),
    )
    database.execute(
        "INSERT INTO messenger_messages(id,conversation_id,role,body,created_at_ms) "
        "VALUES (?,?, 'user','legacy message',?)",
        (INTEGRATION_B, INTEGRATION_A, NOW - 2_000),
    )
    database.execute(
        "INSERT INTO messenger_support_emails(id,conversation_id,user_id,provider_message_id,status,created_at_ms) "
        "VALUES (?,?,?,'legacy-provider-id','sent',?)",
        (OBJECT_A, INTEGRATION_A, OWNER, NOW - 2_000),
    )
    database.execute(
        "INSERT INTO developer_apps(id,owner_user_id,organization_id,name,environment,status,"
        "created_at_ms,updated_at_ms,revision) VALUES (?,?,?,'Dirty app','test','active',?,?,0)",
        (APP_A, OWNER, ORG_A, NOW - 2_000, NOW - 2_000),
    )
    database.execute(
        "INSERT INTO developer_credit_accounts(id,app_id,balance_microcredits,auto_top_up_enabled,"
        "created_at_ms,updated_at_ms,revision) VALUES (?,?,100,0,?,?,0)",
        (ACCOUNT_A, APP_A, NOW - 2_000, NOW - 2_000),
    )
    database.execute(
        "INSERT INTO developer_credit_transactions(id,account_id,transaction_type,"
        "amount_microcredits,balance_after_microcredits,reference_type,reference_id,"
        "idempotency_key,created_at_ms) VALUES (?,?, 'purchase',100,100,'legacy','legacy','legacy:one',?)",
        (VIDEO_B, ACCOUNT_A, NOW - 1_000),
    )
    database.execute(
        "INSERT INTO usage_ledger(id,organization_id,usage_type,quantity,microcredits_charged,"
        "idempotency_key,occurred_at_ms,recorded_at_ms) "
        "VALUES (?,?, 'upload_byte',10,1,'legacy:usage',?,?)",
        (APP_B, ORG_A, NOW - 1_000, NOW - 1_000),
    )


def dirty_upgrade() -> dict[str, int]:
    database = sqlite3.connect(":memory:")
    migrate(database, through=10)
    seed_dirty_before_0011(database)
    for migration in migration_files()[10:]:
        database.executescript(migration.read_text(encoding="utf-8"))
    require(database.execute("PRAGMA foreign_key_check").fetchall() == [], "dirty FK failure")
    findings = dict(database.execute("SELECT finding,finding_count FROM business_source_integrity_v1"))
    require(findings["video_metadata_without_checksum"] == 1, "legacy metadata hidden")
    require(findings["edit_documents_without_checksum"] == 1, "legacy edit hidden")
    require(findings["credit_transactions_without_sequence"] == 1, "legacy credit hidden")
    require(findings["usage_without_operation"] == 1, "legacy usage hidden")
    require(findings["messenger_quarantined"] == 3, "legacy messenger not quarantined")
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO messenger_conversations(id,user_id,mode,created_at_ms,updated_at_ms,last_message_at_ms) "
            "VALUES (?,?, 'support',?,?,?)",
            (VIDEO_B, OWNER, NOW, NOW, NOW),
        ),
        "frame_messenger_excluded_fail_closed_v1",
    )
    require(
        database.execute("SELECT COUNT(*) FROM messenger_conversations").fetchone()[0] == 1,
        "dirty messenger row was destroyed",
    )
    messenger_subject = f"messenger_messages:{INTEGRATION_B}"
    purge_after = database.execute(
        "SELECT purge_after_ms FROM business_messenger_legacy_quarantine_v1 "
        "WHERE source_table='messenger_messages' AND source_id=? AND organization_id=?",
        (INTEGRATION_B, ORG_A),
    ).fetchone()
    require(purge_after is not None, "messenger quarantine was not tenant-classified")
    conversation_subject = f"messenger_conversations:{INTEGRATION_A}"
    conversation_purge_after = database.execute(
        "SELECT purge_after_ms FROM business_messenger_legacy_quarantine_v1 "
        "WHERE source_table='messenger_conversations' AND source_id=? AND organization_id=?",
        (INTEGRATION_A, ORG_A),
    ).fetchone()
    require(conversation_purge_after is not None, "conversation quarantine lacked tenant scope")
    execute_query(
        database,
        "messenger_quarantine_delete_conversation.sql",
        (conversation_subject, ORG_A, conversation_purge_after[0]),
    )
    require(
        database.execute(
            "SELECT 1 FROM messenger_conversations WHERE id=?", (INTEGRATION_A,)
        ).fetchone()
        is not None,
        "conversation purge cascaded into separately quarantined children",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "data_subject_assert.sql",
            ("messenger:cross", ORG_B, "messenger_legacy", messenger_subject),
        ),
        "frame_business_authority_conflict_v1",
    )
    execute_query(
        database,
        "data_subject_assert.sql",
        ("messenger:subject", ORG_A, "messenger_legacy", messenger_subject),
    )
    execute_query(
        database,
        "messenger_quarantine_delete_support_email.sql",
        (messenger_subject, ORG_A, purge_after[0]),
    )
    execute_query(
        database,
        "messenger_quarantine_delete_message.sql",
        (messenger_subject, ORG_A, purge_after[0]),
    )
    execute_query(
        database,
        "messenger_quarantine_delete_conversation.sql",
        (messenger_subject, ORG_A, purge_after[0]),
    )
    execute_query(
        database,
        "messenger_quarantine_purge.sql",
        (messenger_subject, ORG_A, purge_after[0], OWNER),
    )
    execute_query(
        database,
        "data_delete_postcondition.sql",
        ("messenger:post", ORG_A, "messenger_legacy", messenger_subject, purge_after[0], OWNER),
    )
    require(
        database.execute("SELECT 1 FROM messenger_messages WHERE id=?", (INTEGRATION_B,)).fetchone()
        is None,
        "tenant-bound messenger purge left source content",
    )
    return {"dirty_findings": sum(findings.values()), "quarantined_messenger_rows": 3}


def authority_and_privacy(database: sqlite3.Connection) -> int:
    authority = query("read_authority_assert.sql")
    database.execute(
        authority,
        ("auth:owner", ORG_A, "user", OWNER, 1, 0, VIDEO_PRIVATE),
    )
    database.execute("DELETE FROM business_repository_assertions_v1")
    expect_integrity(
        lambda: database.execute(
            authority,
            ("auth:member-private", ORG_A, "user", MEMBER, 1, 0, VIDEO_PRIVATE),
        ),
        "frame_business_authority_conflict_v1",
    )
    expect_integrity(
        lambda: database.execute(
            authority,
            ("auth:anonymous-private", ORG_A, "anonymous", ANON_DIGEST, 0, 0, VIDEO_PRIVATE),
        ),
        "frame_business_authority_conflict_v1",
    )
    database.execute(
        authority,
        ("auth:anonymous-unlisted", ORG_A, "anonymous", ANON_DIGEST, 0, 0, VIDEO_UNLISTED),
    )
    database.execute("DELETE FROM business_repository_assertions_v1")
    expect_integrity(
        lambda: database.execute(
            authority,
            ("auth:cross", ORG_A, "user", OUTSIDER, 1, 0, VIDEO_UNLISTED),
        ),
        "frame_business_authority_conflict_v1",
    )

    database.execute(
        "INSERT INTO comments(id,video_id,anonymous_author_digest,body,created_at_ms,updated_at_ms,"
        "revision,organization_id,last_operation_id,comment_kind,timeline_micros) "
        "VALUES (?,?,?,?,?,?,1,?,?, 'emoji',1000)",
        (INTEGRATION_A, VIDEO_UNLISTED, ANON_DIGEST, "👍", NOW, NOW, ORG_A, OWNER),
    )
    require(
        database.execute(
            "SELECT comment_kind,timeline_micros FROM comments WHERE id=?", (INTEGRATION_A,)
        ).fetchone()
        == ("emoji", 1000),
        "comment kind or timeline position was not persisted",
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO comments(id,video_id,anonymous_author_digest,body,created_at_ms,updated_at_ms,"
            "revision,organization_id,last_operation_id,comment_kind) "
            "VALUES (?,?,?,?,?,?,1,?,?, 'emoji')",
            (APP_A, VIDEO_UNLISTED, ANON_DIGEST, "👍 👍", NOW, NOW, ORG_A, OWNER),
        ),
        "frame_business_authority_conflict_v1",
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO comments(id,video_id,anonymous_author_digest,body,created_at_ms,updated_at_ms,"
            "revision,organization_id,last_operation_id) VALUES (?,?,?,?,?,?,1,?,?)",
            (INTEGRATION_B, VIDEO_PRIVATE, ANON_DIGEST, "private probe", NOW, NOW, ORG_A, OWNER),
        ),
        "frame_business_authority_conflict_v1",
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO comments(id,video_id,author_user_id,body,created_at_ms,updated_at_ms,"
            "revision,organization_id,last_operation_id) VALUES (?,?,?,?,?,?,1,?,?)",
            (OBJECT_A, VIDEO_B, MEMBER, "cross", NOW, NOW, ORG_A, OWNER),
        ),
        "frame_business_authority_conflict_v1",
    )

    database.execute(
        "INSERT INTO shared_videos(id,video_id,organization_id,shared_by_user_id,sharing_mode,"
        "shared_at_ms,revision,last_operation_id) VALUES (?,?,?,?, 'organization',?,1,?)",
        (INTEGRATION_B, VIDEO_MEMBER, ORG_A, MEMBER, NOW, MEMBER),
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO shared_videos(id,video_id,organization_id,shared_by_user_id,sharing_mode,"
            "shared_at_ms,revision,last_operation_id) VALUES (?,?,?,?, 'space',?,1,?)",
            (APP_B, VIDEO_MEMBER, ORG_A, MEMBER, NOW, MEMBER),
        ),
        "frame_business_authority_conflict_v1",
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO shared_videos(id,video_id,organization_id,shared_by_user_id,sharing_mode,"
            "shared_at_ms,revision,last_operation_id) VALUES (?,?,?,?, 'organization',?,1,?)",
            (OBJECT_A, VIDEO_PRIVATE, ORG_A, MEMBER, NOW, MEMBER),
        ),
        "frame_business_authority_conflict_v1",
    )
    return 11


def replay_receipts(database: sqlite3.Connection) -> int:
    fingerprint = hashlib.sha256(b"operation").hexdigest()
    operation = "018f47a6-7b1c-7f55-8f39-8f8a8690aa41"
    bindings = (
        operation,
        ORG_A,
        "user",
        OWNER,
        "business:one",
        "video_manage",
        VIDEO_PRIVATE,
        fingerprint,
        "applied",
        2,
        NOW,
    )
    execute_query(database, "operation_insert.sql", bindings)
    rows = execute_query(
        database,
        "operation_by_idempotency.sql",
        (ORG_A, "user", OWNER, "business:one"),
    ).fetchall()
    require(len(rows) == 1, "current principal cannot retrieve receipt")
    require(
        execute_query(
            database,
            "operation_by_idempotency.sql",
            (ORG_A, "user", ADMIN, "business:one"),
        ).fetchall()
        == [],
        "receipt leaked to another principal",
    )
    expect_integrity(lambda: execute_query(database, "operation_insert.sql", bindings), "UNIQUE")
    expect_integrity(
        lambda: database.execute(
            "UPDATE business_repository_operations_v1 SET result_code='unchanged' WHERE operation_id=?",
            (operation,),
        ),
        "immutable",
    )
    return 4


def ordered_events(database: sqlite3.Connection) -> int:
    payload, checksum = canonical({"schema_version": 1, "video_id": VIDEO_UNLISTED})
    outbox_id = "018f47a6-7b1c-7f55-8f39-8f8a8690ab41"
    logical_key = "outbox:one"
    database.execute(
        "INSERT INTO outbox_events(id,organization_id,aggregate_type,aggregate_id,event_type,"
        "deduplication_key,payload_json,state,attempt,available_at_ms,created_at_ms,event_sequence,"
        "event_fingerprint,payload_schema_version,payload_checksum,revision,last_operation_id) "
        "VALUES (?,?, 'video',?,'comment',?,?,'pending',0,?,?,0,?,1,?,0,?)",
        (
            outbox_id,
            ORG_A,
            VIDEO_UNLISTED,
            tenant_key(ORG_A, "outbox", logical_key),
            payload,
            NOW,
            NOW,
            INITIAL_EVENT_FINGERPRINT,
            checksum,
            OWNER,
        ),
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO outbox_events(id,organization_id,aggregate_type,aggregate_id,event_type,"
            "deduplication_key,payload_json,state,attempt,available_at_ms,created_at_ms,event_sequence,"
            "event_fingerprint,payload_schema_version,payload_checksum,revision,last_operation_id) "
            "VALUES (?,?, 'video',?,'comment',?,?,'leased',0,?,?,0,?,1,?,0,?)",
            (
                "018f47a6-7b1c-7f55-8f39-8f8a8690ab42",
                ORG_A,
                VIDEO_UNLISTED,
                tenant_key(ORG_A, "outbox", "forged-initial"),
                payload,
                NOW,
                NOW,
                INITIAL_EVENT_FINGERPRINT,
                checksum,
                OWNER,
            ),
        ),
        "frame_business_document_invalid_v1",
    )
    sequence_one = hashlib.sha256(b"outbox-1").hexdigest()
    sequence_two = hashlib.sha256(b"outbox-2").hexdigest()
    deferred_operation = "018f47a6-7b1c-7f55-8f39-8f8a8690ab43"
    deferred_key = "outbox:deferred-two"
    deferred_request_fingerprint = hashlib.sha256(b"deferred-command").hexdigest()
    execute_query(
        database,
        "event_inbox_insert.sql",
        (
            ORG_A,
            "outbox",
            outbox_id,
            2,
            sequence_two,
            "delivered",
            0,
            NOW,
            deferred_operation,
        ),
    )
    execute_query(
        database,
        "operation_insert.sql",
        (
            deferred_operation,
            ORG_A,
            "user",
            OWNER,
            deferred_key,
            "notification_manage",
            outbox_id,
            deferred_request_fingerprint,
            "accepted",
            0,
            NOW,
        ),
    )
    require(
        database.execute(
            "SELECT disposition FROM business_event_inbox_v1 WHERE aggregate_id=? AND event_sequence=2",
            (outbox_id,),
        ).fetchone()[0]
        == "deferred",
        "out-of-order event was not deferred",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "event_inbox_insert.sql",
            (ORG_A, "outbox", outbox_id, 2, OTHER_DIGEST, "delivered", 0, NOW, ADMIN),
        ),
        "frame_business_semantic_replay_conflict_v1",
    )
    execute_query(
        database,
        "event_inbox_insert.sql",
        (ORG_A, "outbox", outbox_id, 1, sequence_one, "leased", 0, NOW, OWNER),
    )
    execute_query(
        database,
        "outbox_advance.sql",
        (outbox_id, ORG_A, 1, "leased", sequence_one, NOW + 60_000, NOW, OWNER),
    )
    # Replaying the exact same accepted operation must be allowed to converge
    # its previously deferred event without mutating the immutable receipt.
    execute_query(
        database,
        "operation_match_assert.sql",
        (
            "ordered:accepted-replay",
            ORG_A,
            "user",
            OWNER,
            deferred_key,
            "notification_manage",
            outbox_id,
            deferred_request_fingerprint,
        ),
    )
    execute_query(
        database,
        "event_inbox_insert.sql",
        (
            ORG_A,
            "outbox",
            outbox_id,
            2,
            sequence_two,
            "delivered",
            1,
            NOW,
            deferred_operation,
        ),
    )
    execute_query(
        database,
        "outbox_advance.sql",
        (
            outbox_id,
            ORG_A,
            2,
            "delivered",
            sequence_two,
            None,
            NOW,
            deferred_operation,
        ),
    )
    state = database.execute(
        "SELECT state,event_sequence FROM outbox_events WHERE id=?", (outbox_id,)
    ).fetchone()
    require(state == ("delivered", 2), "deferred outbox event did not converge")
    require(
        database.execute(
            "SELECT disposition FROM business_event_inbox_v1 WHERE aggregate_id=? AND event_sequence=2",
            (outbox_id,),
        ).fetchone()[0]
        == "applied",
        "deferred event audit disposition did not advance",
    )
    require(
        database.execute(
            "SELECT result_code FROM business_repository_operations_v1 WHERE operation_id=?",
            (deferred_operation,),
        ).fetchone()
        == ("accepted",),
        "ordered convergence mutated the immutable accepted receipt",
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id='ordered:accepted-replay'"
    )

    # Failed imports require one bounded redacted class; nonfailed states forbid it.
    import_id = "018f47a6-7b1c-7f55-8f39-8f8a8690ab44"
    execute_query(
        database,
        "import_upsert.sql",
        (
            import_id,
            ORG_A,
            VIDEO_UNLISTED,
            "loom",
            hashlib.sha256(b"legacy-import-id").hexdigest(),
            "queued",
            "import:ordered:one",
            None,
            NOW,
            NOW,
            0,
            INITIAL_EVENT_FINGERPRINT,
            0,
            OWNER,
        ),
    )
    execute_query(
        database,
        "import_immutable_assert.sql",
        (
            "import:immutable",
            import_id,
            ORG_A,
            VIDEO_UNLISTED,
            "loom",
            hashlib.sha256(b"legacy-import-id").hexdigest(),
            "import:ordered:one",
            NOW,
        ),
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id='import:immutable'"
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "import_immutable_assert.sql",
            (
                "import:immutable-drift",
                import_id,
                ORG_A,
                VIDEO_UNLISTED,
                "loom",
                hashlib.sha256(b"legacy-import-id").hexdigest(),
                "import:ordered:one",
                NOW + 99,
            ),
        ),
        "frame_business_authority_conflict_v1",
    )
    import_one = hashlib.sha256(b"import-running").hexdigest()
    execute_query(
        database,
        "import_advance.sql",
        (import_id, ORG_A, 1, "running", import_one, NOW + 1, None, OWNER),
    )
    import_two = hashlib.sha256(b"import-failed").hexdigest()
    expect_integrity(
        lambda: execute_query(
            database,
            "import_advance.sql",
            (import_id, ORG_A, 2, "failed", import_two, NOW + 2, None, OWNER),
        ),
        "frame_business_event_order_conflict_v1",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "import_advance.sql",
            (
                import_id,
                ORG_A,
                2,
                "failed",
                import_two,
                NOW + 2,
                "Provider URL leaked",
                OWNER,
            ),
        ),
        "frame_business_event_order_conflict_v1",
    )
    execute_query(
        database,
        "import_advance.sql",
        (
            import_id,
            ORG_A,
            2,
            "failed",
            import_two,
            NOW + 2,
            "provider_timeout",
            OWNER,
        ),
    )
    require(
        database.execute(
            "SELECT state,error_class,event_sequence FROM imported_videos WHERE id=?",
            (import_id,),
        ).fetchone()
        == ("failed", "provider_timeout", 2),
        "failed import did not persist its redacted class",
    )
    return 13


def storage_and_derivatives(database: sqlite3.Connection) -> int:
    capabilities = '{"schema_version":1}'
    capabilities_checksum = hashlib.sha256(capabilities.encode()).hexdigest()
    for integration, organization in ((INTEGRATION_A, ORG_A), (INTEGRATION_B, ORG_B)):
        execute_query(
            database,
            "storage_integration_upsert.sql",
            (
                integration,
                organization,
                OWNER if organization == ORG_A else OUTSIDER,
                "r2",
                "active",
                capabilities,
                "ciphertext-material-0123456789-abcd",
                NOW,
                NOW,
                1,
                1,
                OWNER,
                1,
                capabilities_checksum,
                0,
                0,
            ),
        )
    checksum = hashlib.sha256(b"source").hexdigest()
    database.execute(
        "INSERT INTO storage_objects(id,organization_id,integration_id,video_id,object_key,role,"
        "object_version,state,bytes,content_type,checksum_sha256,created_at_ms,updated_at_ms,revision,last_operation_id) "
        "VALUES (?,?,?,?,?,'source',1,'available',6,'video/mp4',?,?,?,1,?)",
        (OBJECT_A, ORG_A, INTEGRATION_A, VIDEO_UNLISTED, "tenant/source.mp4", checksum, NOW, NOW, OWNER),
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO storage_objects(id,organization_id,integration_id,video_id,object_key,role,"
            "object_version,state,bytes,content_type,checksum_sha256,created_at_ms,updated_at_ms,revision,last_operation_id) "
            "VALUES (?,?,?,?,?,'source',1,'available',6,'video/mp4',?,?,?,1,?)",
            (APP_A, ORG_A, INTEGRATION_B, VIDEO_UNLISTED, "tenant/cross.mp4", checksum, NOW, NOW, OWNER),
        ),
        "frame_business_authority_conflict_v1",
    )
    job_id = "018f47a6-7b1c-7f55-8f39-8f8a8690ac41"
    database.execute(
        "INSERT INTO media_jobs(id,video_id,kind,state,idempotency_key,attempt,payload_json,"
        "created_at_ms,updated_at_ms,organization_id,source_version,profile_version,revision) "
        "VALUES (?,?,'preview','queued','job:business',0,'{}',?,?,?,1,1,0)",
        (job_id, VIDEO_UNLISTED, NOW, NOW, ORG_A),
    )
    database.execute(
        "INSERT INTO business_derivative_manifests_v1(job_id,organization_id,executor,"
        "source_object_id,source_version,transform_profile,profile_version,output_role,"
        "output_object_key,output_content_type,state,usage_units,cost_microcredits,revision,last_operation_id) "
        "VALUES (?,?,'native_gstreamer',?,1,'web_preview',1,'preview','tenant/preview.mp4',"
        "'video/mp4','queued',0,0,1,?)",
        (job_id, ORG_A, OBJECT_A, OWNER),
    )
    require(
        database.execute(
            "SELECT capabilities_schema_version,capabilities_checksum,revision "
            "FROM storage_integrations WHERE id=?",
            (INTEGRATION_A,),
        ).fetchone()
        == (1, capabilities_checksum, 1),
        "typed storage integration did not persist its capability contract",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "share_exact_postcondition.sql",
            (
                "exact:share",
                INTEGRATION_B,
                VIDEO_MEMBER,
                ORG_A,
                None,
                ADMIN,
                "organization",
                NOW,
                None,
                1,
                MEMBER,
            ),
        ),
        "frame_business_authority_conflict_v1",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "storage_object_exact_postcondition.sql",
            (
                "exact:storage",
                OBJECT_A,
                ORG_A,
                INTEGRATION_A,
                VIDEO_UNLISTED,
                "tenant/source.mp4",
                "preview",
                1,
                "available",
                6,
                "video/mp4",
                checksum,
                NOW,
                None,
                NOW,
                1,
                OWNER,
            ),
        ),
        "frame_business_authority_conflict_v1",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "derivative_exact_postcondition.sql",
            (
                "exact:derivative",
                job_id,
                ORG_A,
                "native_gstreamer",
                OBJECT_A,
                1,
                "web_preview",
                1,
                "thumbnail",
                None,
                "tenant/preview.mp4",
                None,
                "video/mp4",
                "queued",
                0,
                0,
                None,
                1,
                OWNER,
            ),
        ),
        "frame_business_authority_conflict_v1",
    )
    return 7


def developer_and_ledger(database: sqlite3.Connection) -> int:
    execute_query(
        database,
        "developer_app_upsert.sql",
        (APP_A, OWNER, ORG_A, "App A", "test", "active", NOW, NOW, None, 1, 1, OWNER, 0, 0),
    )
    execute_query(
        database,
        "developer_app_upsert.sql",
        (
            APP_B,
            OUTSIDER,
            ORG_B,
            "App B",
            "test",
            "active",
            NOW,
            NOW,
            None,
            1,
            1,
            OUTSIDER,
            0,
            0,
        ),
    )
    execute_query(
        database,
        "developer_domain_upsert.sql",
        (APP_A, "example.invalid", NOW, NOW, 1, 0, ADMIN, ORG_A),
    )
    developer_metadata = '{"schema_version":1,"source":"sdk"}'
    developer_metadata_checksum = hashlib.sha256(developer_metadata.encode()).hexdigest()
    execute_query(
        database,
        "developer_video_upsert.sql",
        (
            OBJECT_A,
            APP_A,
            VIDEO_UNLISTED,
            OTHER_DIGEST,
            developer_metadata,
            NOW,
            NOW,
            None,
            1,
            developer_metadata_checksum,
            1,
            0,
            ADMIN,
            ORG_A,
        ),
    )
    require(
        database.execute(
            "SELECT external_user_id,external_user_digest,metadata_checksum "
            "FROM developer_videos WHERE id=?",
            (OBJECT_A,),
        ).fetchone()
        == (OTHER_DIGEST, OTHER_DIGEST, developer_metadata_checksum),
        "developer video persisted plaintext or lost its metadata checksum",
    )
    key_digest = hashlib.sha256(b"secret-key").hexdigest()
    execute_query(
        database,
        "developer_key_insert.sql",
        (
            INTEGRATION_A,
            APP_A,
            key_digest,
            "secret",
            "sk_test",
            NOW,
            None,
            None,
            "sk_test",
            1,
            OWNER,
            ORG_A,
        ),
    )
    columns = {row[1] for row in database.execute("PRAGMA table_info(developer_api_keys)")}
    require("encryptedKey" not in columns and "encrypted_key" not in columns, "plaintext key column")
    require(
        database.execute("SELECT key_digest FROM developer_api_keys").fetchone()[0] == key_digest,
        "key digest not persisted",
    )

    database.execute(
        "INSERT INTO developer_credit_accounts(id,app_id,balance_microcredits,auto_top_up_enabled,"
        "created_at_ms,updated_at_ms,revision,ledger_sequence,last_operation_id) "
        "VALUES (?,?,0,0,?,?,0,0,?)",
        (ACCOUNT_A, APP_A, NOW, NOW, OWNER),
    )
    reference_one = hashlib.sha256(b"purchase").hexdigest()
    op_one = "018f47a6-7b1c-7f55-8f39-8f8a8690ad41"
    execute_query(
        database,
        "credit_transaction_insert.sql",
        (
            VIDEO_PRIVATE,
            ACCOUNT_A,
            "purchase",
            100,
            100,
            "manual",
            reference_one,
            "credit:one",
            NOW,
            1,
            reference_one,
            op_one,
            reference_one,
        ),
    )
    require(
        database.execute(
            "SELECT balance_microcredits,ledger_sequence FROM developer_credit_accounts WHERE id=?",
            (ACCOUNT_A,),
        ).fetchone()
        == (100, 1),
        "credit account did not advance atomically",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "credit_transaction_insert.sql",
            (
                VIDEO_UNLISTED,
                ACCOUNT_A,
                "usage",
                -10,
                90,
                "job",
                OTHER_DIGEST,
                "credit:gap",
                NOW,
                3,
                OTHER_DIGEST,
                "018f47a6-7b1c-7f55-8f39-8f8a8690ad42",
                OTHER_DIGEST,
            ),
        ),
        "frame_business_accounting_conflict_v1",
    )
    reference_two = hashlib.sha256(b"usage").hexdigest()
    execute_query(
        database,
        "credit_transaction_insert.sql",
        (
            VIDEO_UNLISTED,
            ACCOUNT_A,
            "usage",
            -40,
            60,
            "job",
            reference_two,
            "credit:two",
            NOW + 1,
            2,
            reference_two,
            "018f47a6-7b1c-7f55-8f39-8f8a8690ad43",
            reference_two,
        ),
    )
    require(
        database.execute(
            "SELECT balance_microcredits,ledger_sequence FROM developer_credit_accounts WHERE id=?",
            (ACCOUNT_A,),
        ).fetchone()
        == (60, 2),
        "usage debit did not reconcile",
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE developer_credit_transactions SET amount_microcredits=0 WHERE id=?",
            (VIDEO_UNLISTED,),
        ),
        "append-only",
    )

    usage_op = "018f47a6-7b1c-7f55-8f39-8f8a8690ad44"
    execute_query(
        database,
        "usage_insert.sql",
        (
            INTEGRATION_B,
            ORG_A,
            APP_A,
            VIDEO_UNLISTED,
            None,
            "upload_byte",
            6,
            2,
            "usage:one",
            NOW,
            NOW,
            usage_op,
            reference_two,
        ),
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "usage_insert.sql",
            (
                OBJECT_A,
                ORG_A,
                APP_B,
                VIDEO_UNLISTED,
                None,
                "upload_byte",
                6,
                2,
                "usage:cross",
                NOW,
                NOW,
                "018f47a6-7b1c-7f55-8f39-8f8a8690ad45",
                reference_two,
            ),
        ),
        "frame_business_accounting_conflict_v1",
    )
    return 11


def tenant_scoped_idempotency(database: sqlite3.Connection) -> int:
    payload, checksum = canonical({"schema_version": 1, "kind": "same-key"})
    logical_key = "same-logical-key"
    for index, (organization, video, operation) in enumerate(
        (
            (ORG_A, VIDEO_UNLISTED, OWNER),
            (ORG_B, VIDEO_B, OUTSIDER),
        ),
        start=1,
    ):
        database.execute(
            "INSERT INTO outbox_events(id,organization_id,aggregate_type,aggregate_id,event_type,"
            "deduplication_key,payload_json,state,attempt,available_at_ms,created_at_ms,event_sequence,"
            "event_fingerprint,payload_schema_version,payload_checksum,revision,last_operation_id) "
            "VALUES (?,?,'video',?,'same_key',?,?,'pending',0,?,?,0,?,1,?,0,?)",
            (
                f"018f47a6-7b1c-7f55-8f39-8f8a8690b4{index:02d}",
                organization,
                video,
                tenant_key(organization, "outbox", logical_key),
                payload,
                NOW,
                NOW,
                INITIAL_EVENT_FINGERPRINT,
                checksum,
                operation,
            ),
        )
    require(
        database.execute(
            "SELECT COUNT(DISTINCT deduplication_key) FROM outbox_events "
            "WHERE aggregate_id IN (?,?)",
            (VIDEO_UNLISTED, VIDEO_B),
        ).fetchone()[0]
        >= 2,
        "outbox logical keys were not tenant-scoped",
    )

    for index, (organization, app, video, operation) in enumerate(
        (
            (ORG_A, APP_A, VIDEO_UNLISTED, OWNER),
            (ORG_B, APP_B, VIDEO_B, OUTSIDER),
        ),
        start=1,
    ):
        execute_query(
            database,
            "usage_insert.sql",
            (
                f"018f47a6-7b1c-7f55-8f39-8f8a8690b5{index:02d}",
                organization,
                app,
                video,
                None,
                "upload_byte",
                1,
                0,
                tenant_key(organization, "usage", logical_key),
                NOW,
                NOW,
                operation,
                hashlib.sha256(f"usage-{index}".encode()).hexdigest(),
            ),
        )
    require(
        database.execute(
            "SELECT COUNT(DISTINCT idempotency_key) FROM usage_ledger "
            "WHERE quantity=1 AND microcredits_charged=0"
        ).fetchone()[0]
        == 2,
        "usage logical keys were not tenant-scoped",
    )
    return 4


def export_and_subject_authority(database: sqlite3.Connection) -> int:
    execute_query(
        database,
        "export_authority_assert.sql",
        ("export:owner", ORG_A, "user", OWNER, 1, 0),
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id='export:owner'"
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "export_authority_assert.sql",
            ("export:member", ORG_A, "user", MEMBER, 1, 0),
        ),
        "frame_business_authority_conflict_v1",
    )
    execute_query(
        database,
        "data_subject_assert.sql",
        ("subject:local", ORG_A, "storage_object", OBJECT_A),
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id='subject:local'"
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "data_subject_assert.sql",
            ("subject:cross", ORG_B, "storage_object", OBJECT_A),
        ),
        "frame_business_authority_conflict_v1",
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "data_subject_assert.sql",
            ("subject:missing", ORG_A, "storage_object", ""),
        ),
        "frame_business_authority_conflict_v1",
    )
    return 4


def retention_and_messenger(database: sqlite3.Connection) -> int:
    hold_id = "018f47a6-7b1c-7f55-8f39-8f8a8690ae41"
    database.execute(
        "INSERT INTO business_legal_holds_v1(id,organization_id,data_class,subject_id,"
        "reason_code,placed_by_user_id,placed_at_ms) VALUES (?,?, 'storage_object',?,'litigation',?,?)",
        (hold_id, ORG_A, OBJECT_A, OWNER, NOW),
    )
    expect_integrity(
        lambda: execute_query(
            database,
            "retention_assert.sql",
            ("retention:hold", ORG_A, "storage_object", "delete", OBJECT_A),
        ),
        "frame_business_retention_locked_v1",
    )
    execute_query(
        database,
        "retention_assert.sql",
        ("retention:export", ORG_A, "storage_object", "export", OBJECT_A),
    )
    database.execute("DELETE FROM business_retention_assertions_v1")
    expect_integrity(
        lambda: execute_query(
            database,
            "retention_assert.sql",
            ("retention:key-export", ORG_A, "developer_api_key", "export", INTEGRATION_A),
        ),
        "frame_business_retention_locked_v1",
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO messenger_conversations(id,user_id,mode,created_at_ms,updated_at_ms,last_message_at_ms) "
            "VALUES (?,?, 'support',?,?,?)",
            (VIDEO_B, OWNER, NOW, NOW, NOW),
        ),
        "frame_messenger_excluded_fail_closed_v1",
    )
    return 4


def concrete_business_surfaces(database: sqlite3.Connection) -> int:
    # Read/list and idempotent state-marking surfaces execute their checked-in SQL.
    comments = execute_query(
        database, "comment_list.sql", (ORG_A, VIDEO_UNLISTED)
    ).fetchall()
    require(any(row[0] == INTEGRATION_A for row in comments), "comment list omitted row")

    notification_id = "018f47a6-7b1c-7f55-8f39-8f8a8690ba41"
    notification_payload, notification_checksum = canonical(
        {"schema_version": 1, "video_id": VIDEO_UNLISTED}
    )
    execute_query(
        database,
        "notification_insert.sql",
        (
            notification_id,
            ORG_A,
            MEMBER,
            "comment",
            "notification:read:one",
            notification_payload,
            NOW,
            None,
            1,
            notification_checksum,
            OWNER,
        ),
    )
    require(
        execute_query(database, "notification_list.sql", (ORG_A, MEMBER)).fetchone()[0]
        == notification_id,
        "notification list did not bind the recipient",
    )
    execute_query(
        database,
        "notification_mark_read.sql",
        (notification_id, ORG_A, MEMBER, NOW + 1, OWNER),
    )
    execute_query(
        database,
        "notification_mark_read_postcondition.sql",
        ("notification:post", notification_id, ORG_A, MEMBER, NOW + 1, OWNER),
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id='notification:post'"
    )

    account = execute_query(database, "credit_account.sql", (ACCOUNT_A, ORG_A)).fetchone()
    require(account is not None and account[2:4] == (60, 0), "credit account read drifted")

    # Legal hold placement/list/release is executable and tenant-scoped.
    hold_id = "018f47a6-7b1c-7f55-8f39-8f8a8690ba42"
    execute_query(
        database,
        "legal_hold_insert.sql",
        (hold_id, ORG_A, "video_metadata", VIDEO_UNLISTED, "investigation", OWNER, NOW),
    )
    require(
        any(
            row[0] == hold_id
            for row in execute_query(database, "legal_hold_list.sql", (ORG_A,)).fetchall()
        ),
        "legal hold list omitted placed hold",
    )
    execute_query(database, "legal_hold_release.sql", (hold_id, ORG_A, NOW + 1))
    execute_query(
        database,
        "legal_hold_release_postcondition.sql",
        ("hold:released", hold_id, ORG_A, NOW + 1),
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id='hold:released'"
    )

    # Upload lifecycle starts from the one canonical sequence-zero state.
    upload_id = "018f47a6-7b1c-7f55-8f39-8f8a8690ba43"
    execute_query(
        database,
        "upload_insert.sql",
        (
            upload_id,
            ORG_A,
            VIDEO_UNLISTED,
            100,
            tenant_key(ORG_A, "upload", "upload:surface:one"),
            "tenant/upload.bin",
            1,
            "video/mp4",
            NOW,
            OWNER,
        ),
    )
    upload_one = hashlib.sha256(b"upload-one").hexdigest()
    execute_query(
        database,
        "event_inbox_insert.sql",
        (ORG_A, "upload", upload_id, 1, upload_one, "uploading", 0, NOW, OWNER),
    )
    execute_query(
        database,
        "upload_advance.sql",
        (upload_id, ORG_A, 1, "uploading", 10, None, upload_one, NOW + 1, OWNER),
    )
    execute_query(
        database,
        "upload_exact_postcondition.sql",
        ("upload:post", upload_id, ORG_A, "uploading", 10, None, 1, upload_one, NOW + 1, 1, OWNER),
    )
    database.execute("DELETE FROM business_repository_assertions_v1 WHERE id='upload:post'")

    # A concrete delete executor changes the subject before recording completion.
    execute_query(
        database,
        "data_subject_assert.sql",
        ("delete:subject", ORG_A, "comment", INTEGRATION_A),
    )
    execute_query(
        database,
        "retention_assert.sql",
        ("delete:retention", ORG_A, "comment", "delete", INTEGRATION_A),
    )
    execute_query(
        database,
        "delete_comment_data.sql",
        (INTEGRATION_A, ORG_A, NOW + 2, OWNER),
    )
    execute_query(
        database,
        "data_delete_postcondition.sql",
        ("delete:post", ORG_A, "comment", INTEGRATION_A, NOW + 2, OWNER),
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id IN "
        "('delete:subject','delete:post')"
    )
    database.execute(
        "DELETE FROM business_retention_assertions_v1 WHERE id='delete:retention'"
    )

    # Append-only ledger deletion creates real, balanced compensation rows.
    for index, (data_class, original_id, amount, sequence, balance) in enumerate(
        (
            ("credit_transaction", VIDEO_UNLISTED, 40, 3, 100),
            ("usage_ledger", INTEGRATION_B, 2, 4, 102),
        ),
        start=1,
    ):
        compensation_id = f"018f47a6-7b1c-7f55-8f39-8f8a8690bb4{index}"
        operation_id = f"018f47a6-7b1c-7f55-8f39-8f8a8690bc4{index}"
        reference = semantic_fingerprint(
            "frame-business-deletion-compensation-v1", data_class, original_id
        )
        execute_query(
            database,
            "ledger_compensation_assert.sql",
            (
                f"compensation:{index}",
                ORG_A,
                data_class,
                original_id,
                compensation_id,
                ACCOUNT_A,
                sequence,
                amount,
                balance,
                "adjustment",
                "data_deletion_compensation",
                reference,
            ),
        )
        execute_query(
            database,
            "credit_transaction_insert.sql",
            (
                compensation_id,
                ACCOUNT_A,
                "adjustment",
                amount,
                balance,
                "data_deletion_compensation",
                reference,
                f"compensation:{index}:key",
                NOW + 10 + index,
                sequence,
                reference,
                operation_id,
                reference,
            ),
        )
        database.execute(
            "DELETE FROM business_repository_assertions_v1 WHERE id=?",
            (f"compensation:{index}",),
        )
    require(
        database.execute(
            "SELECT balance_microcredits,ledger_sequence FROM developer_credit_accounts WHERE id=?",
            (ACCOUNT_A,),
        ).fetchone()
        == (102, 4),
        "compensation entries did not reconcile the account",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM developer_credit_transactions WHERE reference_type='data_deletion_compensation'"
        ).fetchone()[0]
        == 2,
        "ledger delete did not append compensation rows",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM developer_credit_transactions WHERE id IN (?,?)",
            (VIDEO_PRIVATE, VIDEO_UNLISTED),
        ).fetchone()[0]
        == 2,
        "append-only originals were mutated or deleted",
    )

    # Organization-only charged usage is valid and can credit a caller-selected
    # account that is authoritatively scoped to the same tenant.
    organization_usage_id = "018f47a6-7b1c-7f55-8f39-8f8a8690bd41"
    execute_query(
        database,
        "usage_insert.sql",
        (
            organization_usage_id,
            ORG_A,
            None,
            None,
            None,
            "upload_byte",
            3,
            3,
            "usage:organization-only",
            NOW + 20,
            NOW + 20,
            "018f47a6-7b1c-7f55-8f39-8f8a8690bd42",
            hashlib.sha256(b"organization-usage").hexdigest(),
        ),
    )
    organization_reference = semantic_fingerprint(
        "frame-business-deletion-compensation-v1", "usage_ledger", organization_usage_id
    )
    execute_query(
        database,
        "ledger_compensation_assert.sql",
        (
            "compensation:organization-usage",
            ORG_A,
            "usage_ledger",
            organization_usage_id,
            "018f47a6-7b1c-7f55-8f39-8f8a8690bd43",
            ACCOUNT_A,
            5,
            3,
            105,
            "adjustment",
            "data_deletion_compensation",
            organization_reference,
        ),
    )
    database.execute(
        "DELETE FROM business_repository_assertions_v1 WHERE id='compensation:organization-usage'"
    )

    export_rows = execute_query(
        database, "export_rows.sql", (ORG_A, "", "", 100_000)
    ).fetchall()
    export_text = "\n".join(row[2] for row in export_rows)
    require(export_rows, "tenant export returned no rows")
    for forbidden in ("credential_ciphertext", "key_digest", ANON_DIGEST):
        require(forbidden not in export_text, f"tenant export leaked {forbidden}")
    first_page = execute_query(database, "export_rows.sql", (ORG_A, "", "", 2)).fetchall()
    require(len(first_page) == 2, "export keyset page was not bounded")
    cursor_class, cursor_subject = first_page[-1][0], first_page[-1][1]
    second_page = execute_query(
        database, "export_rows.sql", (ORG_A, cursor_class, cursor_subject, 2)
    ).fetchall()
    require(
        second_page and (second_page[0][0], second_page[0][1]) > (cursor_class, cursor_subject),
        "export keyset cursor repeated or skipped backward",
    )
    return 23


def fault_rollback(database: sqlite3.Connection) -> int:
    before = database.execute("SELECT COUNT(*) FROM shared_videos").fetchone()[0]
    try:
        database.execute("BEGIN")
        database.execute(
            query("authority_assert.sql"),
            (
                "fault:authority",
                ORG_A,
                "user",
                OWNER,
                1,
                0,
                0,
                0,
                0,
                0,
                "write",
                "share_manage",
                VIDEO_PRIVATE,
            ),
        )
        database.execute(
            "INSERT INTO shared_videos(id,video_id,organization_id,shared_by_user_id,sharing_mode,"
            "shared_at_ms,revision,last_operation_id) VALUES (?,?,?,?, 'organization',?,1,?)",
            (OBJECT_A, VIDEO_PRIVATE, ORG_A, OWNER, NOW, OWNER),
        )
        database.execute(
            query("resource_postcondition.sql"),
            ("fault:post", "share", OBJECT_A, ORG_A, 99, OWNER),
        )
        database.execute("COMMIT")
        raise ConformanceFailure("fault injection unexpectedly committed")
    except sqlite3.IntegrityError:
        database.execute("ROLLBACK")
    after = database.execute("SELECT COUNT(*) FROM shared_videos").fetchone()[0]
    require(before == after, "failed postcondition left a tenant mutation")
    require(
        database.execute("SELECT COUNT(*) FROM business_repository_assertions_v1").fetchone()[0]
        == 0,
        "failed batch left an authority assertion",
    )
    return 2


def static_contracts() -> dict[str, int]:
    repository = REPOSITORY.read_text(encoding="utf-8")
    domain = DOMAIN.read_text(encoding="utf-8")
    port = PORT.read_text(encoding="utf-8")
    application = APPLICATION.read_text(encoding="utf-8")
    require("statements.insert(\n            0," in repository, "mutation authority is not first")
    require(
        "vec![\n            self.read_authority_statement" in repository,
        "read authorization is not first",
    )
    require("operation_receipt" in port and "principal" in port, "principal receipts absent")
    require("EncryptedProviderConfig([redacted])" in domain, "provider config debug is unsafe")
    require("ReadOnlyPreserve" in domain, "forward document policy absent")
    require("constant_time_checksum_eq" in application, "fingerprints are not compared safely")
    for source in (domain, port, application):
        require("JsValue" not in source, "JS binding leaked into provider-neutral contract")
        require("signed_url" not in source.lower(), "private signed URL leaked into contract")
    expected_tables = {
        "videos",
        "video_edits",
        "shared_videos",
        "comments",
        "notifications",
        "messenger_conversations",
        "messenger_messages",
        "messenger_support_emails",
        "s3_buckets",
        "storage_integrations",
        "storage_objects",
        "video_uploads",
        "imported_videos",
        "developer_apps",
        "developer_app_domains",
        "developer_api_keys",
        "developer_videos",
        "developer_credit_accounts",
        "developer_credit_transactions",
        "developer_daily_storage_snapshots",
    }
    migration = (MIGRATIONS / "0011_business_authority_expand.sql").read_text(encoding="utf-8")
    for table in expected_tables:
        require(f"('{table}'," in migration, f"source table not mapped: {table}")
    require(
        "('usage_ledger','frame_derived'" in migration,
        "Frame-derived usage ledger provenance is missing",
    )
    require(
        "tenant_scoped_idempotency_digest" in repository,
        "tenant-local logical keys are stored without a scoped digest",
    )
    require(
        "export_authority_statement(&request" in repository,
        "owner-only export authorization is not wired",
    )
    require(
        "DATA_SUBJECT_ASSERT_SQL" in repository,
        "data-subject tenant binding is not wired",
    )
    for query_name in (
        "video_upsert.sql",
        "edit_upsert.sql",
        "storage_integration_upsert.sql",
        "developer_app_upsert.sql",
        "developer_domain_upsert.sql",
        "developer_video_upsert.sql",
        "daily_snapshot_upsert.sql",
    ):
        require(
            "created_at_ms = excluded.created_at_ms" in query(query_name),
            f"immutable creation time is not checked by {query_name}",
        )
    require(
        "export_page(&request" in repository and "final_counts != class_counts" in repository,
        "complete keyset export and final snapshot fence are not wired",
    )
    sample_export = json.loads(SAMPLE_EXPORT.read_text(encoding="utf-8"))
    require(sample_export["schema_version"] == 1, "sample export schema drifted")
    require(sample_export["rows"] == [], "zero-row sample unexpectedly contains data")
    require(
        list(sample_export["class_counts"]) == BUSINESS_DATA_CLASSES,
        "sample export class order or coverage drifted",
    )
    require(sample_export["excludes_secrets"] is True, "sample export secret fence absent")
    checksum_material = bytearray()
    for data_class in BUSINESS_DATA_CLASSES:
        count = sample_export["class_counts"][data_class]
        require(count == 0, "zero-row sample has a nonzero class count")
        checksum_material.extend(data_class.encode("utf-8"))
        checksum_material.extend(count.to_bytes(8, "big"))
    checksum_material.extend(sample_export["source_revision"].to_bytes(8, "big"))
    require(
        hashlib.sha256(checksum_material).hexdigest()
        == sample_export["content_checksum"],
        "sample export checksum drifted",
    )
    return {
        "statically_mapped_tables": len(expected_tables),
        "derived_aggregates": 1,
        "sample_exports": 1,
    }


def semantic_suite() -> dict[str, int]:
    database = sqlite3.connect(":memory:")
    migrate(database)
    seed_current(database)
    checks = 0
    checks += authority_and_privacy(database)
    checks += replay_receipts(database)
    checks += ordered_events(database)
    checks += storage_and_derivatives(database)
    checks += developer_and_ledger(database)
    checks += tenant_scoped_idempotency(database)
    checks += export_and_subject_authority(database)
    checks += retention_and_messenger(database)
    checks += concrete_business_surfaces(database)
    database.commit()
    checks += fault_rollback(database)
    require(database.execute("PRAGMA foreign_key_check").fetchall() == [], "semantic FK failure")
    database.execute(
        "INSERT INTO business_messenger_legacy_quarantine_v1("
        "source_table,source_id,quarantined_at_ms,purge_after_ms) VALUES ('external',?,?,?)",
        (VIDEO_B, NOW, NOW + 1),
    )
    counts = execute_query(database, "export_counts.sql", (ORG_A,)).fetchall()
    require(len(counts) == 20, "export manifest does not cover all data classes")
    require(dict(counts)["messenger_legacy"] == 0, "global messenger quarantine leaked into tenant export")
    source_revision = execute_query(database, "export_revision.sql", (ORG_A,)).fetchone()[0]
    require(source_revision == 2, "tenant export revision did not track committed operations")
    checks += 4
    return {"semantic_assertions": checks, "exported_data_classes": len(counts)}


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--migration-only", action="store_true", help="run clean and dirty migration checks only")
    parser.add_argument("--json", action="store_true", help="emit one JSON summary")
    return parser.parse_args(argv)


def main(argv: Sequence[str]) -> int:
    args = parse_args(argv)
    result: dict[str, int | str] = {"status": "ok", "mode": "offline_sqlite"}
    result.update(clean_migration())
    result.update(dirty_upgrade())
    if not args.migration_only:
        result.update(compile_queries())
        result.update(static_contracts())
        result.update(semantic_suite())
    if args.json:
        print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    else:
        print("business SQLite semantic conformance: ok")
        for key, value in sorted(result.items()):
            if key not in {"status", "mode"}:
                print(f"  {key}: {value}")
        print("  protected provider claims: not exercised")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (ConformanceFailure, sqlite3.Error) as error:
        print(f"business SQLite semantic conformance: FAILED: {error}", file=sys.stderr)
        raise SystemExit(1) from error
