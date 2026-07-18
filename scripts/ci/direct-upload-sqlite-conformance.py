#!/usr/bin/env python3
"""Offline D1 contract tests for browser-direct R2 upload state.

Hosted R2 behavior remains a protected provider gate. This suite proves the
checked-in relational boundary: brokered/direct separation, immutable staging
identity, unique capabilities, ordered finalization, and one-shot expiry work.
"""

from __future__ import annotations

import hashlib
import json
import pathlib
import sqlite3
from collections.abc import Callable


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
NOW = 1_700_500_000_000
USER = "018f47a6-7b1c-7f55-8f39-8f8a8690a601"
ORG = "018f47a6-7b1c-7f55-8f39-8f8a8690b601"
VIDEO = "018f47a6-7b1c-7f55-8f39-8f8a8690c601"
UPLOAD = "018f47a6-7b1c-7f55-8f39-8f8a8690d601"
EXPIRED_UPLOAD = "018f47a6-7b1c-7f55-8f39-8f8a8690d602"
CHECKSUM = "ab" * 32
TENANT_SCOPE = hashlib.sha256(
    b"frame.direct-upload.tenant.v1\0" + ORG.encode()
).hexdigest()


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
    selected = [path for path in files if int(path.name[:4]) <= 23]
    require(
        [int(path.name[:4]) for path in selected] == list(range(1, 24)),
        "migration sequence through 0023 is not contiguous",
    )
    for path in selected:
        database.executescript(path.read_text(encoding="utf-8"))


def expect_integrity(operation_fn: Callable[[], object], fragment: str) -> None:
    try:
        operation_fn()
    except sqlite3.IntegrityError as error:
        require(fragment in str(error), f"wrong integrity failure: {error}")
    else:
        raise ConformanceFailure(f"expected integrity failure containing {fragment!r}")


def seed(database: sqlite3.Connection) -> None:
    database.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) "
        "VALUES (?,?,?,?,?)",
        (USER, "direct-owner@sqlite.invalid", "Direct Owner", NOW - 10_000, NOW - 10_000),
    )
    database.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,"
        "updated_at_ms,tombstoned_at_ms,revision,authority_version,retention_until_ms,"
        "recovered_at_ms,last_operation_id) "
        "VALUES (?,?,?,'active','{}',?,?,NULL,0,0,NULL,NULL,?)",
        (ORG, USER, "Direct Upload", NOW - 9_000, NOW - 9_000, operation(1)),
    )
    database.execute(
        "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,"
        "created_at_ms,updated_at_ms,revision,authority_version,last_operation_id) "
        "VALUES (?,?,'owner','active',0,?,?,0,0,?)",
        (ORG, USER, NOW - 8_000, NOW - 8_000, operation(2)),
    )
    document = json.dumps(
        {"schema_version": 1, "title": "Direct Upload"},
        sort_keys=True,
        separators=(",", ":"),
    )
    database.execute(
        "INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,"
        "organization_id,privacy,metadata_json,revision,metadata_schema_version,"
        "metadata_checksum,comments_enabled,last_operation_id,duration_ms) "
        "VALUES (?,?,?,'pending',?,?,?,?,?,1,1,?,1,?,NULL)",
        (
            VIDEO,
            USER,
            "Direct Upload",
            NOW - 7_000,
            NOW - 7_000,
            ORG,
            "private",
            document,
            hashlib.sha256(document.encode()).hexdigest(),
            operation(3),
        ),
    )


def staging_key(upload_id: str) -> str:
    return f"uploads/{TENANT_SCOPE}/staging/{upload_id}.webm"


def insert_direct(
    database: sqlite3.Connection,
    upload_id: str,
    *,
    key: str | None = None,
    checksum: str | None = CHECKSUM,
    expiry: int | None = NOW + 300_000,
    idempotency: str | None = None,
    version: int = 1,
) -> None:
    database.execute(
        "INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,"
        "received_bytes,idempotency_key,source_object_key,source_version,content_type,"
        "checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,event_fingerprint,"
        "transfer_mode,direct_staging_key,direct_checksum_sha256,direct_expires_at_ms) "
        "VALUES (?,?,?,'initiated',1024,0,?,?,?,'video/webm',NULL,?,?,0,0,NULL,'direct',?,?,?)",
        (
            upload_id,
            ORG,
            VIDEO,
            idempotency or f"direct-{upload_id}",
            f"tenants/{ORG}/videos/{VIDEO}/source/v{version}/payload",
            version,
            NOW,
            NOW,
            staging_key(upload_id) if key is None else key,
            checksum,
            expiry,
        ),
    )


def run() -> dict[str, object]:
    database = sqlite3.connect(":memory:")
    migrate(database)
    seed(database)
    insert_direct(database, UPLOAD)

    expect_integrity(
        lambda: database.execute(
            "INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,"
            "received_bytes,idempotency_key,source_object_key,source_version,content_type,"
            "created_at_ms,updated_at_ms,revision,transfer_mode,direct_staging_key,"
            "direct_checksum_sha256,direct_expires_at_ms) "
            "VALUES (?,?,?,'initiated',1,0,'bad-brokered',?,8,'video/webm',?,?,0,'brokered',?,?,?)",
            (
                operation(10),
                ORG,
                VIDEO,
                f"tenants/{ORG}/videos/{VIDEO}/source/v8/payload",
                NOW,
                NOW,
                staging_key(operation(10)),
                CHECKSUM,
                NOW + 300_000,
            ),
        ),
        "frame_direct_upload_contract_v1",
    )
    expect_integrity(
        lambda: insert_direct(database, operation(11), checksum=None, version=9),
        "frame_direct_upload_contract_v1",
    )
    expect_integrity(
        lambda: insert_direct(
            database,
            operation(12),
            key="uploads/scope/staging/../escape",
            version=10,
        ),
        "frame_direct_upload_contract_v1",
    )
    expect_integrity(
        lambda: insert_direct(
            database,
            operation(13),
            key=staging_key(UPLOAD),
            version=11,
        ),
        "UNIQUE constraint failed: video_uploads.direct_staging_key",
    )
    expect_integrity(
        lambda: insert_direct(
            database,
            operation(14),
            key=f"uploads/{'a' * 63}/staging/{operation(14)}.webm",
            version=12,
        ),
        "frame_direct_upload_contract_v1",
    )
    expect_integrity(
        lambda: insert_direct(
            database,
            operation(15),
            key=f"uploads/{TENANT_SCOPE}/staging/{operation(15)}.mp4",
            version=13,
        ),
        "frame_direct_upload_contract_v1",
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE video_uploads SET direct_checksum_sha256=? WHERE id=?",
            ("cd" * 32, UPLOAD),
        ),
        "frame_direct_upload_contract_v1",
    )

    database.execute(
        "UPDATE video_uploads SET state='uploading',updated_at_ms=?,revision=revision+1,"
        "event_sequence=event_sequence+1,event_fingerprint=? WHERE id=?",
        (NOW + 1, hashlib.sha256(b"uploading").hexdigest(), UPLOAD),
    )
    database.execute(
        "UPDATE video_uploads SET state='finalizing',updated_at_ms=?,revision=revision+1,"
        "event_sequence=event_sequence+1,event_fingerprint=? WHERE id=?",
        (NOW + 2, hashlib.sha256(b"finalizing").hexdigest(), UPLOAD),
    )
    database.execute(
        "UPDATE video_uploads SET state='complete',received_bytes=expected_bytes,"
        "checksum_sha256=direct_checksum_sha256,updated_at_ms=?,revision=revision+1,"
        "event_sequence=event_sequence+1,event_fingerprint=? WHERE id=?",
        (NOW + 3, hashlib.sha256(b"complete").hexdigest(), UPLOAD),
    )
    complete = database.execute(
        "SELECT state,received_bytes,expected_bytes,checksum_sha256 FROM video_uploads WHERE id=?",
        (UPLOAD,),
    ).fetchone()
    require(complete == ("complete", 1024, 1024, CHECKSUM), "completion state drifted")

    insert_direct(
        database,
        EXPIRED_UPLOAD,
        expiry=NOW + 1,
        version=2,
    )
    candidate = database.execute(
        "SELECT u.id,u.direct_staging_key FROM video_uploads u "
        "LEFT JOIN direct_upload_staging_cleanup_v1 c ON c.upload_id=u.id "
        "WHERE u.transfer_mode='direct' AND u.direct_expires_at_ms<=? "
        "AND u.direct_staging_key IS NOT NULL AND c.upload_id IS NULL "
        "ORDER BY u.direct_expires_at_ms,u.id LIMIT 1",
        (NOW + 2,),
    ).fetchone()
    require(candidate == (EXPIRED_UPLOAD, staging_key(EXPIRED_UPLOAD)), "wrong expiry candidate")
    database.execute(
        "INSERT INTO direct_upload_staging_cleanup_v1(upload_id,cleaned_at_ms) VALUES (?,?)",
        (EXPIRED_UPLOAD, NOW + 2),
    )
    remaining = database.execute(
        "SELECT COUNT(*) FROM video_uploads u LEFT JOIN direct_upload_staging_cleanup_v1 c "
        "ON c.upload_id=u.id WHERE u.id=? AND c.upload_id IS NULL",
        (EXPIRED_UPLOAD,),
    ).fetchone()[0]
    require(remaining == 0, "cleanup receipt did not suppress replay")

    return {
        "schema_version": 1,
        "status": "passed",
        "migration": 23,
        "checks": {
            "direct_contract_rejections": 7,
            "ordered_completion": True,
            "one_shot_expiry_cleanup": True,
        },
        "protected_gates": ["hosted_r2_sigv4_put", "hosted_r2_head_checksum"],
    }


if __name__ == "__main__":
    print(json.dumps(run(), sort_keys=True, separators=(",", ":")))
