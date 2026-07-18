#!/usr/bin/env python3
"""Prove immutable local staging for all hardware-gated media contracts."""

from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import sys
from pathlib import Path, PurePosixPath


ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/api-parity/v1/protected-media-contracts.json"
MIGRATIONS = ROOT / "apps/control-plane/migrations"
APPLICATION = ROOT / "crates/application/src/legacy_protected_media.rs"
RUNTIME = ROOT / "apps/control-plane/src/legacy_protected_media_runtime.rs"
WEB = ROOT / "apps/control-plane/src/legacy_protected_media_web_runtime.rs"
QUERIES = ROOT / "apps/control-plane/queries/legacy_protected_media"

OWNER_ID = "11111111-1111-4111-8111-111111111111"
MEMBER_ID = "22222222-2222-4222-8222-222222222222"
PUBLIC_ID = "33333333-3333-4333-8333-333333333333"
DENIED_ID = "44444444-4444-4444-8444-444444444444"
ORGANIZATION_ID = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
LEGACY_ORGANIZATION_ID = "0123456789abcde"
VIDEO_ID = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
LEGACY_VIDEO_ID = "123456789abcdeg"
SPACE_ID = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def expect_integrity(action, marker: str) -> None:
    try:
        action()
    except sqlite3.IntegrityError as error:
        if marker not in str(error):
            raise AssertionError(f"unexpected integrity error: {error}") from error
    else:
        raise AssertionError(f"expected integrity failure containing {marker!r}")


def load_database() -> sqlite3.Connection:
    connection = sqlite3.connect(":memory:")
    connection.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        if int(migration.name[:4]) > 61:
            break
        connection.executescript(migration.read_text())
    return connection


def seed_identity_authority(connection: sqlite3.Connection) -> dict[str, tuple[str, str]]:
    users = (
        (OWNER_ID, "owner@example.com"),
        (MEMBER_ID, "member@example.com"),
        (PUBLIC_ID, "viewer@example.com"),
        (DENIED_ID, "viewer@denied.test"),
    )
    sessions: dict[str, tuple[str, str]] = {}
    for index, (user_id, email) in enumerate(users, 1):
        connection.execute(
            "INSERT INTO users(id,email,created_at_ms,updated_at_ms) VALUES(?,?,1,1)",
            (user_id, email),
        )
        connection.execute(
            """INSERT INTO auth_identities_v2(
              user_id,identity_revision,session_version,created_at_ms,updated_at_ms,
              revision,last_operation_id
            ) VALUES(?,1,0,1,1,0,NULL)""",
            (user_id,),
        )
        session_id = f"{index:08d}-0000-4000-8000-{index:012d}"
        session_digest = digest(f"session-{index}")
        connection.execute(
            """INSERT INTO auth_sessions_v2(
              id,family_id,user_id,client_kind,token_key_version,token_digest,
              csrf_key_version,csrf_digest,browser_origin,issued_at_ms,rotated_at_ms,
              idle_expires_at_ms,absolute_expires_at_ms,session_version,generation,
              state,revoked_at_ms,revocation_reason,revision,last_operation_id
            ) VALUES(?,?,?,?,?,?,?,?,?,1,1,9000000,9000000,0,0,'active',NULL,NULL,0,?)""",
            (
                session_id,
                f"{index:08d}-0000-4000-9000-{index:012d}",
                user_id,
                "browser",
                1,
                session_digest,
                1,
                digest(f"csrf-{index}"),
                "https://frame.engmanager.xyz",
                f"{index:08d}-0000-4000-a000-{index:012d}",
            ),
        )
        sessions[user_id] = (session_id, session_digest)

    connection.execute(
        """INSERT INTO organizations(
          id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,revision
        ) VALUES(?,?,'Frame test','active','{}',1,1,0)""",
        (ORGANIZATION_ID, OWNER_ID),
    )
    for user_id, role in ((OWNER_ID, "owner"), (MEMBER_ID, "member")):
        connection.execute(
            """INSERT INTO organization_members(
              organization_id,user_id,role,state,has_pro_seat,
              created_at_ms,updated_at_ms,revision
            ) VALUES(?,?,?,'active',1,1,1,0)""",
            (ORGANIZATION_ID, user_id, role),
        )
    connection.execute(
        """INSERT INTO legacy_user_account_organization_ids_v1(
          organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
        ) VALUES(?,?,1,'aaaaaaaa-0000-4000-8000-000000000001')""",
        (ORGANIZATION_ID, LEGACY_ORGANIZATION_ID),
    )
    connection.execute(
        """INSERT INTO videos(
          id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,
          privacy,legacy_public,legacy_password_hash,legacy_property_revision
        ) VALUES(?,?,'Protected media','ready',1,1,?,'public',1,?,1)""",
        (VIDEO_ID, OWNER_ID, ORGANIZATION_ID, "A" * 64),
    )
    connection.execute(
        """INSERT INTO legacy_collaboration_video_aliases_v1(
          legacy_video_id,mapped_video_id,provenance,created_at_ms
        ) VALUES(?,?,'cap_backfill',1)""",
        (LEGACY_VIDEO_ID, VIDEO_ID),
    )
    connection.execute(
        """INSERT INTO spaces(
          id,organization_id,created_by_user_id,name,is_primary,is_public,
          settings_json,created_at_ms,updated_at_ms,revision,
          legacy_password_hash,legacy_password_revision
        ) VALUES(?,?,?,'Protected space',0,0,'{}',1,1,0,NULL,0)""",
        (SPACE_ID, ORGANIZATION_ID, OWNER_ID),
    )
    connection.execute(
        """INSERT INTO legacy_protected_media_service_authorities_v1(
          credential_subject_id,credential_kind,credential_key_version,
          credential_digest,state,expires_at_ms
        ) VALUES('MEDIA_SERVER_WEBHOOK_SECRET.v1','service_secret',1,?,'active',9000000)""",
        (digest("media-service"),),
    )
    connection.commit()
    return sessions


def policy_proof(
    target_id: str,
    kind: str,
    subject_id: str,
    revision: int,
    audit: str | None = None,
) -> dict:
    return {
        "target_id": target_id,
        "kind": kind,
        "subject_id": subject_id,
        "revision": revision,
        "audit_digest": audit or digest(f"{target_id}:{kind}:{subject_id}:{revision}"),
    }


def insert_receipt(connection: sqlite3.Connection, **overrides) -> dict:
    receipt_id = overrides["receipt_id"]
    operation_id = overrides["operation_id"]
    created_at_ms = overrides.get("created_at_ms", 1000)
    policy_json = json.dumps(overrides.get("policy_proofs", []), separators=(",", ":"))
    request_digest = overrides.get("request_digest", digest(f"request:{receipt_id}"))
    payload_digest = digest(f"payload:{receipt_id}")
    authority_binding_digest = overrides.get(
        "authority_binding_digest", digest(f"authority:{receipt_id}")
    )
    descriptor = json.dumps(
        {
            "schema_version": "frame.legacy-protected-media-request.v2",
            "source_operation_id": operation_id,
            "payload_digest": payload_digest,
            "payload_descriptor": {},
        },
        separators=(",", ":"),
        sort_keys=True,
    )
    values = {
        "receipt_id": receipt_id,
        "source_operation_id": operation_id,
        "operation_kind": overrides.get("operation_kind", "route"),
        "method": overrides.get("method", "GET"),
        "surface_path": overrides.get("surface_path", "/test"),
        "auth_class": overrides["auth_class"],
        "authority_class": overrides["authority_class"],
        "principal_digest": overrides.get("principal_digest", digest(f"principal:{receipt_id}")),
        "actor_id": overrides.get("actor_id"),
        "tenant_id": overrides.get("tenant_id"),
        "credential_kind": overrides["credential_kind"],
        "credential_subject_id": overrides["credential_subject_id"],
        "credential_key_version": overrides["credential_key_version"],
        "credential_digest": overrides["credential_digest"],
        "policy_proofs_json": policy_json,
        "entitlement_kind": overrides.get("entitlement_kind"),
        "entitlement_subject_id": overrides.get("entitlement_subject_id"),
        "entitlement_revision": overrides.get("entitlement_revision"),
        "entitlement_expires_at_ms": overrides.get("entitlement_expires_at_ms"),
        "target_id": overrides.get("target_id"),
        "authority_binding_digest": authority_binding_digest,
        "parent_family": overrides.get("parent_family"),
        "parent_receipt_id": overrides.get("parent_receipt_id"),
        "parent_request_digest": overrides.get("parent_request_digest"),
        "parent_authority_binding_digest": overrides.get(
            "parent_authority_binding_digest"
        ),
        "execution_key_digest": overrides.get(
            "execution_key_digest", digest(f"execution:{receipt_id}")
        ),
        "replay_origin": overrides.get("replay_origin", "natural"),
        "idempotency_mode": overrides.get("idempotency_mode", "required"),
        "request_digest": request_digest,
        "payload_digest": payload_digest,
        "request_descriptor_json": descriptor,
        "sealed_request_ref": None,
        "sealed_request_digest": None,
        "terminal_kind": overrides.get("terminal_kind", "json"),
        "executor_kind": overrides.get("executor_kind", "provider"),
        "provider_required": overrides.get("provider_required", 1),
        "created_at_ms": created_at_ms,
    }
    columns = ",".join(values)
    bindings = ",".join(f":{column}" for column in values)
    connection.execute(
        f"INSERT INTO legacy_protected_media_receipts_v1 ({columns},state) "
        f"VALUES ({bindings},'pending_execution_evidence')",
        values,
    )
    outbox_descriptor = json.dumps(
        {
            "schema_version": "frame.legacy-protected-media-execution.v2",
            "receipt_id": receipt_id,
            "request_digest": request_digest,
        },
        separators=(",", ":"),
        sort_keys=True,
    )
    connection.execute(
        """INSERT INTO legacy_protected_media_execution_outbox_v1(
          receipt_id,executor_kind,descriptor_json,descriptor_digest,
          state,attempt_count,created_at_ms
        ) VALUES(?,?,?,?,'pending_execution_evidence',0,?)""",
        (
            receipt_id,
            values["executor_kind"],
            outbox_descriptor,
            digest(outbox_descriptor),
            created_at_ms,
        ),
    )
    connection.commit()
    values["outbox_descriptor_digest"] = digest(outbox_descriptor)
    return values


def session_receipt(sessions, actor_id: str, **values) -> dict:
    session_id, session_digest = sessions[actor_id]
    return {
        "actor_id": actor_id,
        "tenant_id": ORGANIZATION_ID if actor_id in (OWNER_ID, MEMBER_ID) else None,
        "credential_kind": "session_token",
        "credential_subject_id": session_id,
        "credential_key_version": 1,
        "credential_digest": session_digest,
        **values,
    }


def validate_fixture() -> dict:
    fixture = json.loads(FIXTURE.read_text())
    operations = fixture["operations"]
    assert fixture["reference"]["commit"] == "6ba69561ac86b8efdb17616d6727f9638015546b"
    assert fixture["summary"] == {
        "hardware_and_provider": 25,
        "hardware_only": 16,
        "local_terminal_behavior": "fail_closed_unavailable",
        "operation_count": 41,
    }
    assert fixture["durable_contract"] == {
        "authority_model": "exact credential plus ordered live policy proofs and optional AI entitlement",
        "caller_idempotency_header": False,
        "execution_evidence": "independent executor lease plus provider evidence when required",
        "request_storage": "redacted v2 descriptor plus optional opaque sealed request reference",
        "terminal_storage": "typed opaque sealed terminal reference with 15-minute retention",
        "workflow_parent_model": "shared protected effect registry, exact parent authority digest, allowlisted target rule",
    }
    assert len(operations) == 41
    ids = {operation["id"] for operation in operations}
    assert len(ids) == 41
    assert all(operation["local_contract"]["atomic_stage"] for operation in operations)
    assert all(operation["local_contract"]["immutable_replay"] for operation in operations)
    assert all("hardware_execution" in operation["protected_gates"] for operation in operations)
    assert sum("provider_execution" in operation["protected_gates"] for operation in operations) == 25
    assert sum(operation["protected_gates"] == ["hardware_execution"] for operation in operations) == 16
    assert all(
        operation["local_auth"] == "parent_derived"
        for operation in operations
        if operation["kind"] == "workflow"
    )
    expected_local_auth = {
        "cap-v1-c471cd8f8f990fcc": "session",
        "cap-v1-fbd3d44a0ca1786f": "public_edge_or_job_capability",
        "cap-v1-0bf20f7e9b1a474c": "public_edge_or_job_capability",
        "cap-v1-43bc9ae6aa4f44a8": "public_edge_or_job_capability",
        "cap-v1-986bf73a0b5cb676": "public_edge_or_job_capability",
        "cap-v1-aa2bd4c3be69ed42": "optional_session_or_share_capability",
    }
    assert all(
        operation["local_auth"] == expected_local_auth.get(
            operation["id"],
            "parent_derived" if operation["kind"] == "workflow" else operation["auth"],
        )
        for operation in operations
    )

    application = APPLICATION.read_text()
    for operation in operations:
        assert operation["id"] in application
        assert operation["method"] in {"GET", "HEAD", "POST", "RPC", "ACTION", "WORKFLOW"}
        assert operation["idempotency"] in {"required", "forbidden"}
        for source in operation["source_manifest"]:
            source_path = PurePosixPath(source["path"])
            assert source_path.parts and not source_path.is_absolute()
            assert ".." not in source_path.parts
            assert source["symbol"]
            assert re.fullmatch(r"[0-9a-f]{64}", source["sha256"])
    return fixture


def validate_source_contracts() -> None:
    runtime = RUNTIME.read_text()
    web = WEB.read_text()
    application = APPLICATION.read_text()
    assert "batch(statements)" in runtime
    assert "pending_execution_evidence" in runtime
    assert "ExecutionEvidenceRequired" in runtime
    assert "EXECUTION_EVIDENCE_REQUIRED" in web
    assert "MEDIA_SERVER_WEBHOOK_SECRET" in web
    assert "CRON_SECRET" in web
    assert 'get("idempotency-key")' not in web
    assert "ProtectedMediaRequestVaultV1" in web
    assert "ProtectedMediaTerminalV1" in web
    assert "parent_authority_binding_digest" in application
    assert "parent_capability" in application
    assert "D1LegacyProtectedMediaRuntimeV1::new" in web
    workflow_carrier = web.split("pub async fn workflow_response(", 1)[1].split(
        "\nfn workflow_principal(", 1
    )[0]
    assert "WORKFLOW_PARENT_READ_SQL" in workflow_carrier
    assert "LegacyProtectedMediaReplayOriginV1::Workflow" in workflow_carrier
    assert "ParentReceiptClaimV1" in workflow_carrier
    assert "_actor_id" not in workflow_carrier
    for query in (
        "receipt_replay.sql",
        "generated_replay.sql",
        "receipt_insert.sql",
        "outbox_insert.sql",
        "share_capability_by_hash.sql",
        "video_policy_base.sql",
        "ai_entitlement.sql",
        "workflow_parent_read.sql",
    ):
        assert (QUERIES / query).is_file()


def validate_database() -> None:
    connection = load_database()
    sessions = seed_identity_authority(connection)

    def reject(marker: str, **receipt) -> None:
        try:
            insert_receipt(connection, **receipt)
        except sqlite3.IntegrityError as error:
            connection.rollback()
            if marker not in str(error):
                raise AssertionError(f"unexpected receipt rejection: {error}") from error
        else:
            raise AssertionError(f"expected receipt rejection containing {marker!r}")

    def is_live(receipt_id: str) -> bool:
        return connection.execute(
            "SELECT 1 FROM legacy_protected_media_live_authority_v1 WHERE receipt_id=?",
            (receipt_id,),
        ).fetchone() == (1,)

    # Independent executor evidence is required, provider evidence is
    # conditional, and one evidence insert atomically closes lease/outbox/receipt.
    service_receipt = "50000000-0000-4000-8000-000000000001"
    service = insert_receipt(
        connection,
        receipt_id=service_receipt,
        operation_id="cap-v1-105318e146fceb4c",
        method="POST",
        auth_class="internal_service",
        authority_class="internal_service",
        credential_kind="service_secret",
        credential_subject_id="MEDIA_SERVER_WEBHOOK_SECRET.v1",
        credential_key_version=1,
        credential_digest=digest("media-service"),
        provider_required=1,
        executor_kind="provider",
    )
    connection.execute(
        """INSERT INTO legacy_protected_media_executors_v1(
          executor_id,executor_kind,identity_digest,state
        ) VALUES('provider-executor.v1','provider',?,'active')""",
        (digest("provider-executor"),),
    )
    lease_id = "50000000-0000-4000-8000-000000000002"
    connection.execute(
        """INSERT INTO legacy_protected_media_executor_leases_v1(
          lease_id,receipt_id,executor_id,request_digest,outbox_descriptor_digest,
          authority_binding_digest,leased_at_ms,lease_expires_at_ms,state
        ) VALUES(?,?,'provider-executor.v1',?,?,?,1000,5000,'active')""",
        (
            lease_id,
            service_receipt,
            service["request_digest"],
            service["outbox_descriptor_digest"],
            service["authority_binding_digest"],
        ),
    )
    connection.commit()
    expect_integrity(
        lambda: connection.execute(
            "UPDATE legacy_protected_media_receipts_v1 "
            "SET state='verified',completed_at_ms=2000 WHERE receipt_id=?",
            (service_receipt,),
        ),
        "frame_protected_media_evidence_required_v1",
    )
    connection.rollback()
    evidence_sql = """INSERT INTO legacy_protected_media_execution_evidence_v1(
      receipt_id,lease_id,executor_id,request_digest,outbox_descriptor_digest,
      authority_binding_digest,execution_evidence_digest,provider_evidence_digest,
      terminal_kind,sealed_terminal_ref,sealed_terminal_digest,
      terminal_expires_at_ms,verified_at_ms
    ) VALUES(?,?,'provider-executor.v1',?,?,?,?,?,'json',?,?,800000,2000)"""
    evidence_bindings = (
        service_receipt,
        lease_id,
        service["request_digest"],
        service["outbox_descriptor_digest"],
        service["authority_binding_digest"],
        digest("executor-evidence"),
        None,
        "frame-pm-terminal-v1:" + digest("terminal-reference"),
        digest("terminal-plaintext"),
    )
    expect_integrity(
        lambda: connection.execute(evidence_sql, evidence_bindings),
        "frame_protected_media_evidence_invalid_v1",
    )
    connection.rollback()
    valid_evidence = list(evidence_bindings)
    valid_evidence[6] = digest("provider-evidence")
    connection.execute(evidence_sql, valid_evidence)
    connection.commit()
    assert connection.execute(
        "SELECT state,completed_at_ms FROM legacy_protected_media_receipts_v1 WHERE receipt_id=?",
        (service_receipt,),
    ).fetchone() == ("verified", 2000)
    assert connection.execute(
        "SELECT state FROM legacy_protected_media_executor_leases_v1 WHERE lease_id=?",
        (lease_id,),
    ).fetchone() == ("consumed",)
    expect_integrity(
        lambda: connection.execute(
            "UPDATE legacy_protected_media_receipts_v1 SET request_digest=? WHERE receipt_id=?",
            (digest("tampered-request"), service_receipt),
        ),
        "frame_protected_media_receipt_immutable_v1",
    )
    connection.rollback()

    owner_proof = policy_proof(
        LEGACY_VIDEO_ID, "owner_bypass", VIDEO_ID, 1
    )
    owner_receipt = "50000000-0000-4000-8000-000000000010"
    insert_receipt(
        connection,
        **session_receipt(
            sessions,
            OWNER_ID,
            receipt_id=owner_receipt,
            operation_id="cap-v1-b3a632bd76471ad5",
            auth_class="optional_session_or_share_capability",
            authority_class="video_view",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[owner_proof],
        ),
    )
    assert is_live(owner_receipt), "owner must bypass every password"

    member_proof = policy_proof(
        LEGACY_VIDEO_ID,
        "video_password",
        VIDEO_ID,
        1,
        digest("A" * 64),
    )
    member_receipt = "50000000-0000-4000-8000-000000000011"
    insert_receipt(
        connection,
        **session_receipt(
            sessions,
            MEMBER_ID,
            receipt_id=member_receipt,
            operation_id="cap-v1-b3a632bd76471ad5",
            auth_class="optional_session_or_share_capability",
            authority_class="video_view",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[member_proof],
        ),
    )
    assert is_live(member_receipt), "member plus current password must pass"
    reject(
        "frame_protected_media_authority_stale_v1",
        **session_receipt(
            sessions,
            MEMBER_ID,
            receipt_id="50000000-0000-4000-8000-000000000012",
            operation_id="cap-v1-b3a632bd76471ad5",
            auth_class="optional_session_or_share_capability",
            authority_class="video_view",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[
                policy_proof(
                    LEGACY_VIDEO_ID, "unprotected_video_policy", VIDEO_ID, 1
                )
            ],
        ),
    )

    # Current email restriction is exact-address/domain CSV logic, and it is
    # evaluated in addition to (not instead of) current password policy.
    connection.execute(
        "UPDATE organizations SET legacy_allowed_email_restriction='example.com' WHERE id=?",
        (ORGANIZATION_ID,),
    )
    public_receipt = "50000000-0000-4000-8000-000000000013"
    insert_receipt(
        connection,
        **session_receipt(
            sessions,
            PUBLIC_ID,
            receipt_id=public_receipt,
            operation_id="cap-v1-b3a632bd76471ad5",
            auth_class="optional_session_or_share_capability",
            authority_class="video_view",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[member_proof],
        ),
    )
    assert is_live(public_receipt)
    reject(
        "frame_protected_media_authority_stale_v1",
        **session_receipt(
            sessions,
            DENIED_ID,
            receipt_id="50000000-0000-4000-8000-000000000014",
            operation_id="cap-v1-b3a632bd76471ad5",
            auth_class="optional_session_or_share_capability",
            authority_class="video_view",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[member_proof],
        ),
    )

    # Password revision drift and membership loss invalidate existing receipts.
    connection.execute(
        "UPDATE videos SET legacy_password_hash=?,legacy_property_revision=2 WHERE id=?",
        ("B" * 64, VIDEO_ID),
    )
    assert not is_live(member_receipt)
    connection.execute(
        "UPDATE organization_members SET state='removed' WHERE organization_id=? AND user_id=?",
        (ORGANIZATION_ID, MEMBER_ID),
    )
    assert not is_live(member_receipt)
    connection.execute(
        "UPDATE organization_members SET state='active' WHERE organization_id=? AND user_id=?",
        (ORGANIZATION_ID, MEMBER_ID),
    )

    # A current space password is also a composite policy proof; deleting the
    # placement removes that proof from the live authority projection.
    connection.execute(
        "UPDATE videos SET legacy_password_hash=NULL,legacy_property_revision=3 WHERE id=?",
        (VIDEO_ID,),
    )
    connection.execute(
        "UPDATE spaces SET legacy_password_hash=?,legacy_password_revision=1 WHERE id=?",
        ("C" * 64, SPACE_ID),
    )
    connection.execute(
        """INSERT INTO space_videos(space_id,video_id,added_by_user_id,added_at_ms)
        VALUES(?,?,?,3000)""",
        (SPACE_ID, VIDEO_ID, OWNER_ID),
    )
    space_receipt = "50000000-0000-4000-8000-000000000015"
    insert_receipt(
        connection,
        **session_receipt(
            sessions,
            MEMBER_ID,
            receipt_id=space_receipt,
            operation_id="cap-v1-b3a632bd76471ad5",
            auth_class="optional_session_or_share_capability",
            authority_class="video_view",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[
                policy_proof(
                    LEGACY_VIDEO_ID,
                    "space_password",
                    SPACE_ID,
                    1,
                    digest("C" * 64),
                )
            ],
        ),
    )
    assert is_live(space_receipt)
    connection.execute(
        "DELETE FROM space_videos WHERE space_id=? AND video_id=?",
        (SPACE_ID, VIDEO_ID),
    )
    assert not is_live(space_receipt)

    # Owner AI routes bind the exact current entitlement; revision/state drift
    # invalidates both the parent and its workflow child.
    connection.execute(
        "UPDATE spaces SET legacy_password_hash=NULL,legacy_password_revision=2 WHERE id=?",
        (SPACE_ID,),
    )
    connection.execute(
        "UPDATE organizations SET legacy_allowed_email_restriction='' WHERE id=?",
        (ORGANIZATION_ID,),
    )
    connection.execute(
        """INSERT INTO legacy_protected_media_ai_entitlements_v1(
          user_id,entitlement_revision,state,expires_at_ms
        ) VALUES(?,1,'active',8000000)""",
        (OWNER_ID,),
    )
    ai_proof = policy_proof(LEGACY_VIDEO_ID, "owner_bypass", VIDEO_ID, 3)
    ai_parent_id = "50000000-0000-4000-8000-000000000020"
    ai_parent = insert_receipt(
        connection,
        **session_receipt(
            sessions,
            OWNER_ID,
            receipt_id=ai_parent_id,
            operation_id="cap-v1-c1ae43fcf8ad7018",
            auth_class="session",
            authority_class="video_view_ai_owner_entitled",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[ai_proof],
            entitlement_kind="ai_owner",
            entitlement_subject_id=OWNER_ID,
            entitlement_revision=1,
            entitlement_expires_at_ms=8000000,
        ),
    )
    child_id = "50000000-0000-4000-8000-000000000021"
    child = insert_receipt(
        connection,
        **session_receipt(
            sessions,
            OWNER_ID,
            receipt_id=child_id,
            operation_id="cap-v1-3e0dec6125f270bf",
            operation_kind="workflow",
            method="WORKFLOW",
            auth_class="parent_derived",
            authority_class="video_owner_ai_entitled",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[ai_proof],
            entitlement_kind="ai_owner",
            entitlement_subject_id=OWNER_ID,
            entitlement_revision=1,
            entitlement_expires_at_ms=8000000,
            parent_family="protected_media",
            parent_receipt_id=ai_parent_id,
            parent_request_digest=ai_parent["request_digest"],
            parent_authority_binding_digest=ai_parent["authority_binding_digest"],
            replay_origin="workflow",
        ),
    )
    assert child["authority_binding_digest"] != ai_parent["authority_binding_digest"]
    assert is_live(child_id)
    workflow_parent_read = (QUERIES / "workflow_parent_read.sql").read_text()
    cursor = connection.execute(
        workflow_parent_read,
        (
            "protected_media",
            ai_parent_id,
            ai_parent["request_digest"],
            "cap-v1-3e0dec6125f270bf",
            4_000,
        ),
    )
    parent_row = dict(zip((column[0] for column in cursor.description), cursor.fetchone()))
    assert parent_row["actor_id"] == OWNER_ID
    assert parent_row["credential_kind"] == "session_token"
    assert parent_row["authority_binding_digest"] == ai_parent["authority_binding_digest"]
    assert parent_row["target_binding_rule"] == "same"
    assert connection.execute(
        workflow_parent_read,
        (
            "protected_media",
            ai_parent_id,
            digest("wrong-parent-request"),
            "cap-v1-3e0dec6125f270bf",
            4_000,
        ),
    ).fetchone() is None
    reject(
        "frame_protected_media_workflow_parent_invalid_v1",
        **session_receipt(
            sessions,
            OWNER_ID,
            receipt_id="50000000-0000-4000-8000-000000000022",
            operation_id="cap-v1-3e0dec6125f270bf",
            operation_kind="workflow",
            method="WORKFLOW",
            auth_class="parent_derived",
            authority_class="video_owner_ai_entitled",
            target_id=LEGACY_VIDEO_ID,
            policy_proofs=[ai_proof],
            entitlement_kind="ai_owner",
            entitlement_subject_id=OWNER_ID,
            entitlement_revision=1,
            entitlement_expires_at_ms=8000000,
            parent_family="protected_media",
            parent_receipt_id=ai_parent_id,
            parent_request_digest=ai_parent["request_digest"],
            parent_authority_binding_digest=digest("wrong-parent-authority"),
            replay_origin="workflow",
        ),
    )
    connection.execute(
        "UPDATE legacy_protected_media_ai_entitlements_v1 SET state='disabled',entitlement_revision=2 WHERE user_id=?",
        (OWNER_ID,),
    )
    assert not is_live(ai_parent_id) and not is_live(child_id)

    # An anonymous integration parent is represented without a fake secret.
    # The media-only parent capability is deterministically bound to that
    # origin; native/legacy target ids may cross only through the exact alias.
    anonymous_parent_id = "50000000-0000-4000-8000-000000000030"
    anonymous_parent_request = digest("anonymous-parent-request")
    anonymous_parent_authority = digest("anonymous-parent-authority")
    child_policy = policy_proof(
        LEGACY_VIDEO_ID, "unprotected_video_policy", VIDEO_ID, 3
    )
    parent_policy = {**child_policy, "target_id": VIDEO_ID}
    connection.execute(
        """INSERT INTO legacy_protected_effect_parent_registry_v1(
          parent_family,parent_receipt_id,source_operation_id,request_digest,
          actor_id,tenant_id,target_id,auth_class,authority_class,
          credential_kind,credential_subject_id,credential_key_version,credential_digest,
          policy_proofs_json,entitlement_kind,entitlement_subject_id,
          entitlement_revision,entitlement_expires_at_ms,authority_binding_digest,
          state,created_at_ms,completed_at_ms
        ) VALUES(
          'protected_integrations',?,'cap-v1-d9b654b30f6c362a',?,
          NULL,NULL,?,'public_or_session','video_viewer',
          'none',NULL,NULL,NULL,?,NULL,NULL,NULL,NULL,?,
          'pending_provider_evidence',4000,NULL
        )""",
        (
            anonymous_parent_id,
            anonymous_parent_request,
            VIDEO_ID,
            json.dumps([parent_policy], separators=(",", ":")),
            anonymous_parent_authority,
        ),
    )
    cursor = connection.execute(
        workflow_parent_read,
        (
            "protected_integrations",
            anonymous_parent_id,
            anonymous_parent_request,
            "cap-v1-39c33826cf514552",
            4_001,
        ),
    )
    anonymous_parent = dict(
        zip((column[0] for column in cursor.description), cursor.fetchone())
    )
    assert anonymous_parent["credential_kind"] == "none"
    assert anonymous_parent["target_binding_rule"] == "child_derived"
    assert anonymous_parent["translated_legacy_target_id"] == LEGACY_VIDEO_ID
    anonymous_child_id = "50000000-0000-4000-8000-000000000031"
    anonymous_child = insert_receipt(
        connection,
        receipt_id=anonymous_child_id,
        operation_id="cap-v1-39c33826cf514552",
        operation_kind="workflow",
        method="WORKFLOW",
        auth_class="parent_derived",
        authority_class="video_share",
        actor_id=None,
        tenant_id=None,
        credential_kind="parent_capability",
        credential_subject_id=f"protected_integrations:{anonymous_parent_id}",
        credential_key_version=4000,
        credential_digest=anonymous_parent_authority,
        target_id=LEGACY_VIDEO_ID,
        policy_proofs=[child_policy],
        parent_family="protected_integrations",
        parent_receipt_id=anonymous_parent_id,
        parent_request_digest=anonymous_parent_request,
        parent_authority_binding_digest=anonymous_parent_authority,
        replay_origin="workflow",
        created_at_ms=4001,
    )
    assert is_live(anonymous_child_id)
    assert anonymous_child["credential_kind"] == "parent_capability"
    connection.execute(
        """UPDATE legacy_protected_effect_parent_registry_v1
        SET state='dead_letter',completed_at_ms=5000
        WHERE parent_family='protected_integrations' AND parent_receipt_id=?""",
        (anonymous_parent_id,),
    )
    assert not is_live(anonymous_child_id)

    receipt_columns = {
        row[1] for row in connection.execute(
            "PRAGMA table_info(legacy_protected_media_receipts_v1)"
        )
    }
    outbox_columns = {
        row[1] for row in connection.execute(
            "PRAGMA table_info(legacy_protected_media_execution_outbox_v1)"
        )
    }
    evidence_columns = {
        row[1] for row in connection.execute(
            "PRAGMA table_info(legacy_protected_media_execution_evidence_v1)"
        )
    }
    assert "request_json" not in receipt_columns
    assert "payload_json" not in outbox_columns
    assert "response_json" not in evidence_columns
    assert connection.execute("PRAGMA foreign_key_check").fetchall() == []


def main() -> int:
    validate_fixture()
    validate_source_contracts()
    validate_database()
    print("legacy protected media SQLite conformance passed (41 operations)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, KeyError, OSError, sqlite3.Error) as error:
        print(f"legacy protected media SQLite conformance failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
