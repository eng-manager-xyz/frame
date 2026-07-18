#!/usr/bin/env python3
"""SQLite proof for six source-pinned Cap desktop compatibility routes."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_desktop_compatibility"
SQL = {path.stem: path.read_text(encoding="utf-8") for path in QUERIES.glob("*.sql")}
NOW = 1_700_000_000_000
ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz"


def uid(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


def cap_id(number: int) -> str:
    output = []
    for _ in range(15):
        output.append(ALPHABET[number & 31])
        number >>= 5
    return "".join(reversed(output))


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:", isolation_level=None)
    connection.row_factory = sqlite3.Row
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    return connection


def add_user(connection: sqlite3.Connection, number: int, name: str, last: str) -> str:
    user_id = uid(number)
    connection.execute(
        """INSERT INTO users(
             id,email,display_name,legacy_last_name,created_at_ms,updated_at_ms
           ) VALUES(?,?,?,?,1,1)""",
        (user_id, f"user-{number}@example.test", name, last),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_user_aliases_v1(
             legacy_user_id,mapped_user_id,image_url,provenance,created_at_ms,refreshed_at_ms
           ) VALUES(?,?,?,'cap_backfill',1,1)""",
        (cap_id(10_000 + number), user_id, f"https://images.example/{number}.png"),
    )
    return user_id


def add_organization(
    connection: sqlite3.Connection,
    number: int,
    owner_id: str,
    name: str,
) -> tuple[str, str]:
    organization_id = uid(100 + number)
    legacy_id = cap_id(20_000 + number)
    connection.execute(
        """INSERT INTO organizations(
             id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,
             legacy_desktop_metadata_json
           ) VALUES(?,?,?,'active','{}',?,?,?)""",
        (
            organization_id,
            owner_id,
            name,
            number,
            number,
            json.dumps(
                {
                    "unknown": {"preserved": True},
                    "branding": {"note": "keep", "colors": {"primary": "#aabbcc"}},
                },
                separators=(",", ":"),
            ),
        ),
    )
    connection.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'owner','active',0,1,1)""",
        (organization_id, owner_id),
    )
    connection.execute(
        """INSERT INTO legacy_user_account_organization_ids_v1(
             organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
           ) VALUES(?,?,1,?)""",
        (organization_id, legacy_id, uid(800 + number)),
    )
    return organization_id, legacy_id


def claim(
    connection: sqlite3.Connection,
    operation_id: str,
    source_operation_id: str,
    kind: str,
    actor_id: str,
    organization_id: str | None,
    target_id: str | None,
    key: str,
    request: str,
) -> None:
    values = (
        operation_id,
        source_operation_id,
        kind,
        actor_id,
        organization_id,
        target_id,
        digest(key),
        digest(request),
    )
    connection.execute(SQL["operation_claim"], (*values, NOW))
    connection.execute(SQL["claim_assert"], values)


def complete(
    connection: sqlite3.Connection,
    operation_id: str,
    source_operation_id: str,
    actor_id: str,
    target: str,
    request: str,
    kind: str,
    body: str,
) -> None:
    result_digest = digest(f"result:{kind}:{body}")
    connection.execute(
        SQL["receipt_insert"],
        (operation_id, 200, kind, body, result_digest, NOW),
    )
    connection.execute(
        SQL["audit_insert"],
        (
            uid(9_000 + complete.serial),
            operation_id,
            source_operation_id,
            digest(f"actor:{actor_id}"),
            digest(f"target:{target}"),
            digest(request),
            result_digest,
            NOW,
        ),
    )
    complete.serial += 1
    connection.execute(SQL["operation_complete"], (operation_id, NOW))
    connection.execute(SQL["durable_assert"], (operation_id, NOW, result_digest))
    connection.execute(SQL["assertion_cleanup"], (operation_id,))


complete.serial = 1


def main() -> None:
    db = database()
    owner = add_user(db, 1, "  Ada", "Lovelace\u00a0")
    admin = add_user(db, 2, "Grace", "Hopper")
    member = add_user(db, 3, "Member", "User")
    outsider = add_user(db, 4, "Outside", "User")
    organization, legacy_organization = add_organization(db, 1, owner, "Compiler Group")
    second_org, _ = add_organization(db, 2, outsider, "Other Tenant")
    db.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?,?,'admin','active',0,2,2),(?,?,'member','active',0,2,2)""",
        (organization, admin, organization, member),
    )

    # Organization reads are live, actor-scoped, role-normalized, source-ID
    # projected, and normalize valid brand colors to uppercase.
    owner_rows = db.execute(SQL["organizations_read"], (owner,)).fetchall()
    assert len(owner_rows) == 1
    assert owner_rows[0]["legacy_organization_id"] == legacy_organization
    assert owner_rows[0]["effective_role"] == "owner"
    metadata = json.loads(owner_rows[0]["metadata_json"])
    assert metadata["branding"]["colors"]["primary"] == "#aabbcc"
    admin_rows = db.execute(SQL["organizations_read"], (admin,)).fetchall()
    member_rows = db.execute(SQL["organizations_read"], (member,)).fetchall()
    assert admin_rows[0]["effective_role"] == "admin"
    assert member_rows[0]["effective_role"] == "member"
    assert db.execute(SQL["organizations_read"], (outsider,)).fetchall()[0]["name"] == "Other Tenant"

    # Profile projection preserves nullability and source name joining inputs.
    profile = db.execute(SQL["profile_read"], (owner,)).fetchone()
    assert profile["first_name"] == "  Ada" and profile["last_name"] == "Lovelace\u00a0"
    assert profile["image_url"] == "https://images.example/1.png"

    # Admin branding is atomic, preserves unknown metadata, replaces colors,
    # records an immediately renderable data URL, and creates durable evidence.
    branding = db.execute(SQL["branding_snapshot"], (admin, legacy_organization)).fetchone()
    assert branding is not None and branding["effective_role"] == "admin"
    branding_operation = uid(1_001)
    next_metadata = json.loads(branding["metadata_json"])
    next_metadata["branding"]["colors"] = {
        "primary": "#AABBCC",
        "secondary": None,
        "accent": "#123456",
        "background": None,
    }
    encoded_metadata = json.dumps(next_metadata, separators=(",", ":"))
    logo = "data:image/png;base64,iVBORw0KGgo="
    db.execute("BEGIN")
    claim(
        db,
        branding_operation,
        "cap-v1-cdfdf7db0f5cb243",
        "organization_branding",
        admin,
        organization,
        legacy_organization,
        "branding-key",
        "branding-request",
    )
    db.execute(
        SQL["branding_update"],
        (
            organization,
            branding["revision"],
            branding["branding_revision"],
            encoded_metadata,
            1,
            logo,
            NOW,
            branding_operation,
            admin,
        ),
    )
    db.execute(
        SQL["branding_assert"],
        (
            branding_operation,
            organization,
            encoded_metadata,
            logo,
            branding["revision"] + 1,
            branding["branding_revision"] + 1,
        ),
    )
    organization_body = json.dumps(
        {
            "id": legacy_organization,
            "name": "Compiler Group",
            "ownerId": cap_id(10_001),
            "role": "admin",
            "canEditBrand": True,
            "iconUrl": logo,
            "brandColors": next_metadata["branding"]["colors"],
        },
        separators=(",", ":"),
    )
    complete(
        db,
        branding_operation,
        "cap-v1-cdfdf7db0f5cb243",
        admin,
        legacy_organization,
        "branding-request",
        "organization",
        organization_body,
    )
    db.execute("COMMIT")
    stored_metadata = json.loads(
        db.execute(
            "SELECT legacy_desktop_metadata_json FROM organizations WHERE id=?",
            (organization,),
        ).fetchone()[0]
    )
    assert stored_metadata["unknown"]["preserved"] is True
    assert stored_metadata["branding"]["note"] == "keep"
    assert stored_metadata["branding"]["colors"]["accent"] == "#123456"
    forbidden = db.execute(SQL["branding_snapshot"], (member, legacy_organization)).fetchone()
    assert forbidden is not None and forbidden["effective_role"] == "member"
    assert db.execute(SQL["branding_snapshot"], (admin, cap_id(999_999))).fetchone() is None

    # Personal storage selection never crosses actor boundaries. Google Drive
    # chooses active-first/updated-first; S3 clears every personal active row.
    integration_one, integration_two = uid(2_001), uid(2_002)
    for integration_id, updated in ((integration_one, 10), (integration_two, 20)):
        db.execute(
            """INSERT INTO storage_integrations(
                 id,organization_id,owner_user_id,provider,state,capabilities_json,
                 credential_ciphertext,created_at_ms,updated_at_ms,capabilities_checksum
               ) VALUES(?,?,?,'google_drive','active','{"schema_version":1}',
                        'encrypted-test-value',1,?,?)""",
            (integration_id, organization, admin, updated, "a" * 64),
        )
        db.execute(
            """INSERT INTO legacy_desktop_personal_storage_integrations_v1(
                 integration_id,owner_user_id,provider,status,active,updated_at_ms
               ) VALUES(?,?,'googleDrive','active',0,?)""",
            (integration_id, admin, updated),
        )
    selected = db.execute(SQL["storage_snapshot"], (admin,)).fetchone()
    assert selected["integration_id"] == integration_two
    storage_operation = uid(2_100)
    db.execute("BEGIN")
    claim(
        db,
        storage_operation,
        "cap-v1-a77171e54b2ba955",
        "storage_set_active",
        admin,
        None,
        "googleDrive",
        "storage-key",
        "storage-request",
    )
    db.execute(SQL["storage_deactivate"], (admin, NOW, storage_operation))
    db.execute(SQL["storage_activate"], (integration_two, admin, NOW, storage_operation))
    db.execute(SQL["storage_assert"], (storage_operation, admin, integration_two))
    complete(
        db,
        storage_operation,
        "cap-v1-a77171e54b2ba955",
        admin,
        "googleDrive",
        "storage-request",
        "storage_success",
        '{"success":true}',
    )
    db.execute("COMMIT")
    assert db.execute(
        "SELECT integration_id FROM legacy_desktop_personal_storage_integrations_v1 WHERE owner_user_id=? AND active=1",
        (admin,),
    ).fetchone()[0] == integration_two
    assert db.execute(SQL["storage_snapshot"], (member,)).fetchone() is None

    # Progress inserts a missing row, ignores stale source timestamps, updates
    # fresh values, and removes completed non-multipart rows only for the owner.
    video_id, legacy_video = uid(3_001), cap_id(30_001)
    db.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id
           ) VALUES(?,?,'Desktop recording','ready',1,1,?)""",
        (video_id, admin, organization),
    )
    db.execute(
        """INSERT INTO legacy_collaboration_video_aliases_v1(
             legacy_video_id,mapped_video_id,provenance,created_at_ms
           ) VALUES(?,?,'cap_backfill',1)""",
        (legacy_video, video_id),
    )
    assert db.execute(
        SQL["video_snapshot"], (admin, uid(99_999), legacy_video)
    ).fetchone()["video_id"] == video_id
    assert db.execute(
        SQL["video_snapshot"], (outsider, uid(99_999), legacy_video)
    ).fetchone() is None
    db.execute(SQL["progress_insert"], (video_id, 10.0, 100.0, 200, uid(3_100)))
    row = db.execute(SQL["progress_snapshot"], (video_id,)).fetchone()
    db.execute(SQL["progress_update"], (video_id, 20.0, 100.0, 199, uid(3_101), row["revision"]))
    assert db.execute(SQL["progress_snapshot"], (video_id,)).fetchone()["uploaded"] == 10.0
    db.execute(SQL["progress_update"], (video_id, 50.0, 100.0, 201, uid(3_102), row["revision"]))
    fresh = db.execute(SQL["progress_snapshot"], (video_id,)).fetchone()
    assert fresh["uploaded"] == 50.0 and fresh["updated_at_ms"] == 201
    db.execute(SQL["progress_delete"], (video_id, fresh["revision"]))
    assert db.execute(SQL["progress_snapshot"], (video_id,)).fetchone() is None

    # Video delete records object inventory before tombstoning, enters an
    # effect-pending state, then finalizes only after the simulated R2 delete.
    db.execute(
        """INSERT INTO storage_objects(
             id,organization_id,integration_id,video_id,object_key,role,
             object_version,state,bytes,content_type,checksum_sha256,created_at_ms
           ) VALUES(?,?,?,?,?,'source',1,'available',10,'video/mp4',?,1)""",
        (
            uid(3_200),
            organization,
            integration_one,
            video_id,
            f"{cap_id(10_002)}/{legacy_video}/source.mp4",
            "b" * 64,
        ),
    )
    db.execute(
        """INSERT INTO object_legal_holds(
             id,storage_object_id,reason_code,placed_by_user_id,placed_at_ms
           ) VALUES(?,?,'retention',?,1)""",
        (uid(3_201), uid(3_200), admin),
    )
    delete_operation = uid(3_300)
    video = db.execute(SQL["video_snapshot"], (admin, video_id, legacy_video)).fetchone()
    db.execute("BEGIN")
    claim(
        db,
        delete_operation,
        "cap-v1-acc98d2d5e8ff345",
        "video_delete",
        admin,
        organization,
        video_id,
        "delete-key",
        "delete-request",
    )
    db.execute(SQL["video_delete_inventory"], (delete_operation, video_id))
    db.execute(SQL["video_delete_jobs"], (delete_operation, video_id, NOW))
    db.execute(SQL["video_delete_uploads"], (video_id, organization))
    db.execute(SQL["video_delete_imports"], (video_id,))
    db.execute(SQL["video_delete_progress"], (video_id,))
    db.execute(SQL["video_delete_mark_objects"], (delete_operation, video_id, NOW))
    db.execute(SQL["video_delete_mark_manifests"], (delete_operation, video_id, NOW))
    db.execute(
        SQL["video_delete_apply"],
        (video_id, admin, organization, NOW, delete_operation, video["revision"]),
    )
    db.execute(
        SQL["video_delete_assert"],
        (delete_operation, video_id, admin, organization, NOW, video["revision"] + 1),
    )
    db.execute(SQL["video_delete_effect_pending"], (delete_operation,))
    db.execute(SQL["assertion_cleanup"], (delete_operation,))
    db.execute("COMMIT")
    assert db.execute(
        SQL["video_delete_legal_hold"], (delete_operation,)
    ).fetchone()["has_legal_hold"] == 1
    pending = db.execute(SQL["video_delete_pending_objects"], (delete_operation,)).fetchall()
    assert len(pending) == 1 and pending[0]["has_legal_hold"] == 1
    db.execute(
        "UPDATE object_legal_holds SET released_at_ms=? WHERE id=?",
        (NOW, uid(3_201)),
    )
    assert db.execute(
        SQL["video_delete_legal_hold"], (delete_operation,)
    ).fetchone()["has_legal_hold"] == 0
    pending = db.execute(SQL["video_delete_pending_objects"], (delete_operation,)).fetchall()
    assert len(pending) == 1 and pending[0]["has_legal_hold"] == 0
    db.execute(
        SQL["video_delete_object_complete"],
        (delete_operation, pending[0]["object_key"], NOW),
    )
    db.execute("BEGIN")
    db.execute(SQL["video_delete_provider_finalize_objects"], (delete_operation, NOW))
    db.execute(SQL["video_delete_provider_finalize_manifests"], (delete_operation, NOW))
    db.execute(SQL["video_delete_provider_finalize_jobs"], (delete_operation, NOW))
    complete(
        db,
        delete_operation,
        "cap-v1-acc98d2d5e8ff345",
        admin,
        legacy_video,
        "delete-request",
        "json_true",
        "true",
    )
    db.execute("COMMIT")
    assert db.execute(
        "SELECT state FROM videos WHERE id=?", (video_id,)
    ).fetchone()[0] == "deleted"
    assert db.execute(
        "SELECT state FROM storage_objects WHERE video_id=?", (video_id,)
    ).fetchone()[0] == "deleted"
    replay = db.execute(
        SQL["operation_lookup"],
        ("cap-v1-acc98d2d5e8ff345", admin, digest("delete-key")),
    ).fetchone()
    assert replay["state"] == "complete" and replay["result_json"] == "true"
    try:
        db.execute(
            SQL["operation_claim"],
            (
                uid(3_301),
                "cap-v1-acc98d2d5e8ff345",
                "video_delete",
                admin,
                organization,
                video_id,
                digest("delete-key"),
                digest("different-request"),
                NOW,
            ),
        )
        raise AssertionError("same desktop idempotency key accepted a different request")
    except sqlite3.IntegrityError:
        pass

    assert db.execute("PRAGMA foreign_key_check").fetchall() == []
    print(
        "legacy desktop compatibility SQLite conformance passed: actor-scoped "
        "organization/profile reads, admin branding, personal storage selection, "
        "timestamp-arbitrated progress, legal-hold gating, and resumable D1/R2 video deletion"
    )


if __name__ == "__main__":
    main()
