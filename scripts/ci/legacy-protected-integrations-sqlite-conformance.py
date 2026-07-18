#!/usr/bin/env python3
"""Adversarial SQLite proof for 45 provider-only integration carriers.

The proof exercises exact credential tuples, aliases, live policy and
entitlement loss, conditional business authority, generated/natural replay,
cross-family parents, executor leases, sealed terminal evidence, secret
exclusion, and immutability. It never contacts or simulates a provider.
"""

from __future__ import annotations

import hashlib
import json
import re
import sqlite3
from pathlib import Path, PurePosixPath


ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps/control-plane/migrations"
QUERIES = ROOT / "apps/control-plane/queries/legacy_protected_integrations"
APPLICATION = ROOT / "crates/application/src/legacy_protected_integrations.rs"
RUNTIME = ROOT / "apps/control-plane/src/legacy_protected_integrations_runtime.rs"
WEB_RUNTIME = ROOT / "apps/control-plane/src/legacy_protected_integrations_web_runtime.rs"
FIXTURE = ROOT / "fixtures/api-parity/v1/protected-integrations.json"
REPORT = ROOT / "fixtures/api-parity/v1/route-workflow-report.json"
CAP_COMMIT = "6ba69561ac86b8efdb17616d6727f9638015546b"

NOW = 1_780_000_000_000
DIGEST = "a" * 64
OTHER_DIGEST = "b" * 64


def identifier(number: int) -> str:
    return f"00000000-0000-7000-8000-{number:012x}"


OWNER = identifier(1)
ADMIN = identifier(2)
MEMBER = identifier(3)
SPACE_MANAGER = identifier(4)
OUTSIDER = identifier(5)
ORG = identifier(10)
OTHER_ORG = identifier(11)
SPACE = identifier(20)
SECOND_SPACE = identifier(21)
PRIVATE_VIDEO = identifier(30)
PUBLIC_VIDEO = identifier(31)
PASSWORD_VIDEO = identifier(32)
MEMBER_VIDEO = identifier(33)
OTHER_VIDEO = identifier(34)
SESSION = identifier(40)
API_KEY = identifier(42)
OPERATION = identifier(43)

LEGACY_ORG = "01h000000000001"
LEGACY_OTHER_ORG = "01h000000000011"
LEGACY_SPACE = "01h000000000002"
LEGACY_SECOND_SPACE = "01h000000000003"
LEGACY_PRIVATE_VIDEO = "01h000000000004"
LEGACY_PUBLIC_VIDEO = "01h000000000005"
LEGACY_PASSWORD_VIDEO = "01h000000000006"
LEGACY_MEMBER_VIDEO = "01h000000000007"
LEGACY_OTHER_VIDEO = "01h000000000008"
LEGACY_OWNER = "01h000000000101"
LEGACY_ADMIN = "01h000000000102"
LEGACY_MEMBER = "01h000000000103"
LEGACY_SPACE_MANAGER = "01h000000000104"
LEGACY_OUTSIDER = "01h000000000105"
LOOM_VIDEO = "loom-video-1"
API_KEY_DIGEST = hashlib.sha256(b"api-key-secret").hexdigest()
HOSTILE_SECRETS = (
    "hostile-aws-access-key-plaintext",
    "oauth-code-hostile-plaintext",
    "https://storage.example/signed?token=hostile-plaintext",
    "hostile-recipient@example.invalid",
    "hostile feedback body that must never reach d1",
    "hostile desktop log line that must never reach d1",
    "session=hostile-cookie-plaintext",
    '{"providerResponse":"hostile-provider-json"}',
)

SQL = {
    path.stem: path.read_text(encoding="utf-8").strip()
    for path in sorted(QUERIES.glob("*.sql"))
}


def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


ACTORS = (OWNER, ADMIN, MEMBER, SPACE_MANAGER, OUTSIDER)


def session_id(actor_id: str) -> str:
    try:
        return identifier(40 + ACTORS.index(actor_id))
    except ValueError:
        return identifier(999)


def session_digest(actor_id: str) -> str:
    return digest(f"session:{actor_id}")


def migrated() -> sqlite3.Connection:
    database = sqlite3.connect(":memory:", isolation_level=None)
    database.row_factory = sqlite3.Row
    database.execute("PRAGMA foreign_keys = ON")
    for migration in sorted(MIGRATIONS.glob("*.sql")):
        database.executescript(migration.read_text(encoding="utf-8"))
        assert not database.execute("PRAGMA foreign_key_check").fetchall(), migration.name
    return database


def seed(database: sqlite3.Connection) -> None:
    for user_id, email, active_org in (
        (OWNER, "owner@example.test", ORG),
        (ADMIN, "admin@cap.so", ORG),
        (MEMBER, "member@example.test", ORG),
        (SPACE_MANAGER, "space@example.test", ORG),
        (OUTSIDER, "outsider@example.test", OTHER_ORG),
    ):
        database.execute(
            """INSERT INTO users(
                 id,email,display_name,created_at_ms,updated_at_ms,status,
                 active_organization_id,default_organization_id
               ) VALUES(?1,?2,?2,1,1,'active',?3,?3)""",
            (user_id, email, active_org),
        )
        database.execute(
            """INSERT INTO auth_identities_v2(
                 user_id,identity_revision,session_version,created_at_ms,
                 updated_at_ms,revision,last_operation_id
               ) VALUES(?1,1,1,1,1,1,?2)""",
            (user_id, OPERATION),
        )
    for index, actor_id in enumerate(ACTORS):
        database.execute(
            """INSERT INTO auth_sessions_v2(
                 id,family_id,user_id,client_kind,token_key_version,token_digest,
                 csrf_key_version,csrf_digest,browser_origin,issued_at_ms,rotated_at_ms,
                 idle_expires_at_ms,absolute_expires_at_ms,session_version,generation,
                 state,revoked_at_ms,revocation_reason,revision,last_operation_id
               ) VALUES(
                 ?1,?2,?3,'desktop',7,?4,NULL,NULL,NULL,?5,?5,?6,?6,1,0,
                 'active',NULL,NULL,1,?7
               )""",
            (
                session_id(actor_id),
                identifier(140 + index),
                actor_id,
                session_digest(actor_id),
                NOW - 1_000,
                NOW + 10_000_000,
                OPERATION,
            ),
        )
    for legacy_user_id, actor_id in (
        (LEGACY_OWNER, OWNER),
        (LEGACY_ADMIN, ADMIN),
        (LEGACY_MEMBER, MEMBER),
        (LEGACY_SPACE_MANAGER, SPACE_MANAGER),
        (LEGACY_OUTSIDER, OUTSIDER),
    ):
        database.execute(
            """INSERT INTO legacy_collaboration_user_aliases_v1(
                 legacy_user_id,mapped_user_id,image_url,provenance,
                 created_at_ms,refreshed_at_ms
               ) VALUES(?1,?2,NULL,'cap_backfill',1,1)""",
            (legacy_user_id, actor_id),
        )
    database.execute(
        """INSERT INTO auth_api_keys(
             id,user_id,key_digest,name,scopes_json,created_at_ms,expires_at_ms,
             last_used_at_ms,revoked_at_ms
           ) VALUES(?1,?2,?3,'conformance','[\"*\"]',?4,?5,NULL,NULL)""",
        (API_KEY, OWNER, API_KEY_DIGEST, NOW - 1_000, NOW + 60_000),
    )
    for organization_id, owner_id, name in (
        (ORG, OWNER, "Frame"),
        (OTHER_ORG, OUTSIDER, "Other"),
    ):
        database.execute(
            """INSERT INTO organizations(
                 id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms
               ) VALUES(?1,?2,?3,'active','{}',1,1)""",
            (organization_id, owner_id, name),
        )
        database.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,has_pro_seat,
                 created_at_ms,updated_at_ms
               ) VALUES(?1,?2,'owner','active',1,1,1)""",
            (organization_id, owner_id),
        )
    for organization_id, legacy_organization_id in (
        (ORG, LEGACY_ORG),
        (OTHER_ORG, LEGACY_OTHER_ORG),
    ):
        database.execute(
            """INSERT INTO legacy_user_account_organization_ids_v1(
                 organization_id,legacy_organization_id,recorded_at_ms,last_operation_id
               ) VALUES(?1,?2,1,?3)""",
            (organization_id, legacy_organization_id, OPERATION),
        )
    for actor_id, role in ((ADMIN, "admin"), (MEMBER, "member"), (SPACE_MANAGER, "viewer")):
        database.execute(
            """INSERT INTO organization_members(
                 organization_id,user_id,role,state,has_pro_seat,
                 created_at_ms,updated_at_ms
               ) VALUES(?1,?2,?3,'active',1,1,1)""",
            (ORG, actor_id, role),
        )
    database.execute(
        """UPDATE users SET
             legacy_stripe_customer_id='cus_owner',
             legacy_stripe_subscription_id='sub_owner',
             legacy_stripe_subscription_status='active'
           WHERE id=?1""",
        (OWNER,),
    )
    # Cap's calculateProSeats counts a Pro owner through the ownerIsPro branch
    # even when their membership row does not carry hasProSeat.
    database.execute(
        """UPDATE organization_members SET has_pro_seat=0
           WHERE organization_id=?1 AND user_id=?2""",
        (ORG, OWNER),
    )
    database.execute(
        """UPDATE users SET legacy_third_party_stripe_subscription_id='third_party_admin'
           WHERE id=?1""",
        (ADMIN,),
    )
    for space_id, name in ((SPACE, "Managed"), (SECOND_SPACE, "Second")):
        database.execute(
            """INSERT INTO spaces(
                 id,organization_id,created_by_user_id,name,is_public,
                 created_at_ms,updated_at_ms
               ) VALUES(?1,?2,?3,?4,0,1,1)""",
            (space_id, ORG, OWNER, name),
        )
    database.execute(
        """INSERT INTO space_members(
             space_id,user_id,role,created_at_ms,updated_at_ms
           ) VALUES(?1,?2,'manager',1,1)""",
        (SPACE, SPACE_MANAGER),
    )
    for video_id, privacy in (
        (PRIVATE_VIDEO, "private"),
        (PUBLIC_VIDEO, "public"),
        (PASSWORD_VIDEO, "public"),
    ):
        database.execute(
            """INSERT INTO videos(
                 id,owner_id,title,state,created_at_ms,updated_at_ms,
                 organization_id,privacy
               ) VALUES(?1,?2,?3,'ready',1,1,?4,?5)""",
            (video_id, OWNER, video_id, ORG, privacy),
        )
    for video_id, owner_id, organization_id in (
        (MEMBER_VIDEO, MEMBER, ORG),
        (OTHER_VIDEO, OUTSIDER, OTHER_ORG),
    ):
        database.execute(
            """INSERT INTO videos(
                 id,owner_id,title,state,created_at_ms,updated_at_ms,
                 organization_id,privacy
               ) VALUES(?1,?2,?1,'ready',1,1,?3,'private')""",
            (video_id, owner_id, organization_id),
        )
    database.execute(
        "UPDATE videos SET legacy_public=CASE WHEN privacy='public' THEN 1 ELSE 0 END"
    )
    database.execute(
        "UPDATE videos SET legacy_password_hash=?2,legacy_property_revision=7 WHERE id=?1",
        (PASSWORD_VIDEO, "A" * 64),
    )
    database.execute(
        """INSERT INTO legacy_mobile_cap_uploads_v1(
             mapped_video_id,uploaded,total,phase,processing_progress,
             processing_message,processing_error,raw_file_key,updated_at_ms
           ) VALUES(?1,1,1,'processing',0,NULL,NULL,?2,1)""",
        (
            PRIVATE_VIDEO,
            f"{LEGACY_OWNER}/{LEGACY_PRIVATE_VIDEO}/raw-upload.webm",
        ),
    )
    for space_id, legacy_space_id in (
        (SPACE, LEGACY_SPACE),
        (SECOND_SPACE, LEGACY_SECOND_SPACE),
    ):
        database.execute(
            """INSERT INTO legacy_library_space_aliases_v1(
                 legacy_space_id,space_id,provenance,created_at_ms
               ) VALUES(?1,?2,'cap_backfill',1)""",
            (legacy_space_id, space_id),
        )
    for video_id, legacy_video_id in (
        (PRIVATE_VIDEO, LEGACY_PRIVATE_VIDEO),
        (PUBLIC_VIDEO, LEGACY_PUBLIC_VIDEO),
        (PASSWORD_VIDEO, LEGACY_PASSWORD_VIDEO),
        (MEMBER_VIDEO, LEGACY_MEMBER_VIDEO),
        (OTHER_VIDEO, LEGACY_OTHER_VIDEO),
    ):
        database.execute(
            """INSERT INTO legacy_collaboration_video_aliases_v1(
                 legacy_video_id,mapped_video_id,provenance,created_at_ms
               ) VALUES(?1,?2,'cap_backfill',1)""",
            (legacy_video_id, video_id),
        )


def fixture() -> dict:
    document = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert document["schema_version"] == "frame.legacy-protected-integrations.v1"
    assert document["reference"]["commit"] == CAP_COMMIT
    assert document["operation_count"] == 45
    assert document["protected_gates"] == ["provider_execution"]
    conditional = document["security"]["conditional_authority"]
    assert conditional["space_create_actor_pro"] == [
        "passwordEnabled",
        "disableSummary",
        "disableChapters",
        "disableTranscript",
    ]
    assert "need not be that owner" in conditional["space_publish_owner_pro"]
    assert "preserves existing" in conditional["space_update_non_pro_settings"]
    assert "hasProSeat is false" in conditional["seat_capacity"]
    operations = document["operations"]
    ids = [operation["id"] for operation in operations]
    assert len(ids) == len(set(ids)) == 45
    assert {operation["carrier"] for operation in operations} == {
        "route",
        "rpc",
        "server_action",
        "workflow",
    }
    assert all(operation["source_count"] >= 1 for operation in operations)
    assert all(operation["provider"] for operation in operations)
    entitlement_ids = [
        operation_id
        for operation_ids in document["local_entitlements"].values()
        for operation_id in operation_ids
    ]
    assert len(entitlement_ids) == len(set(entitlement_ids))
    assert set(entitlement_ids) <= set(ids)
    assert all(
        (ROOT / source).is_file()
        for source in document["implementation"]["local_adapter_sources"]
    )
    return document


def prove_inventory(document: dict) -> None:
    report = json.loads(REPORT.read_text(encoding="utf-8"))
    expected = {
        entry["id"]
        for entry in report["entries"]
        if entry.get("implementation", {}).get("local_status")
        == "rust_exact_protected_integration_provider_staging_local_contract"
    }
    actual = {operation["id"] for operation in document["operations"]}
    assert actual == expected, (sorted(actual - expected), sorted(expected - actual))
    report_by_id = {entry["id"]: entry for entry in report["entries"]}
    assert all(
        report_by_id[operation_id]["completion"]["local_work"] == "complete"
        and report_by_id[operation_id]["completion"]["protected_gates"]
        == ["provider_execution"]
        and report_by_id[operation_id]["completion"]["production_behavior"]
        == "fail_closed_unavailable"
        for operation_id in actual
    )

    application = APPLICATION.read_text(encoding="utf-8")
    profiles: dict[str, tuple[str, str]] = {}
    for block in re.findall(r"profile!\((.*?)\n\s*\),", application, re.DOTALL):
        strings = re.findall(r'\"([^\"]*)\"', block)
        assert len(strings) >= 4, block
        profiles[strings[0]] = (strings[-3], strings[-1])
    assert set(profiles) == actual
    assert "LEGACY_PROTECTED_INTEGRATIONS_OPERATION_COUNT: usize = 45" in application

    for operation_id, (source_path, expected_hash) in profiles.items():
        source = PurePosixPath(source_path)
        assert source.parts and not source.is_absolute(), operation_id
        assert ".." not in source.parts, operation_id
        assert re.fullmatch(r"[0-9a-f]{64}", expected_hash), operation_id

    runtime = RUNTIME.read_text(encoding="utf-8")
    web = WEB_RUNTIME.read_text(encoding="utf-8")
    assert set(SQL) == {
        "authority_read",
        "generated_claim_upsert",
        "generated_receipt_replay",
        "outbox_insert",
        "receipt_insert",
        "receipt_replay",
        "workflow_parent_read",
    }
    for token in (
        "validate_legacy_protected_integration_envelope",
        "AUTHORITY_READ_SQL",
        "WORKFLOW_PARENT_READ_SQL",
        "RECEIPT_INSERT_SQL",
        "GENERATED_RECEIPT_REPLAY_SQL",
        "GENERATED_CLAIM_UPSERT_SQL",
        "OUTBOX_INSERT_SQL",
        "ProviderEvidenceRequired",
    ):
        assert token in runtime
    for token in (
        "route_response",
        "effect_rpc_response_from_bytes",
        "server_action_http_response",
        "workflow_response",
        "ProtectedIntegrationRequestVaultV1",
        "ProtectedIntegrationTerminalResolverV1",
        "google_callback_public_error_response",
        "compatibility_rate_limit::admit_principal",
        "PROVIDER_EXECUTION_REQUIRED",
        "frame-pi-request-v1:",
        "frame-pi-terminal-v1:",
        "constant_time_equal",
    ):
        assert token in web
    lowered_web = web.lower()
    assert "idempotency-key" not in lowered_web
    assert "x-frame-sealed-payload-ref" not in lowered_web
    assert "x-frame-flow-token" not in lowered_web


def authority(
    database: sqlite3.Connection,
    authority_class: str,
    actor_id: str | None = None,
    authenticated_tenant_id: str | None = None,
    legacy_tenant_id: str | None = None,
    legacy_target_id: str | None = None,
    tenant_domain: str = "none",
    target_domain: str = "none",
    credential_kind: str | None = None,
    credential_subject_id: str | None = None,
    credential_key_version: int | None = None,
    credential_digest: str | None = None,
    credential_expires_at_ms: int | None = None,
    policy_proofs: list[dict] | None = None,
    entitlement: str = "none",
    operation_id: str = "cap-v1-30b7af7323aa2c37",
    now_ms: int = NOW,
    legacy_workflow_actor_id: str | None = None,
    legacy_workflow_cap_tenant_id: str | None = None,
    parent_family: str | None = None,
    parent_receipt_id: str | None = None,
    parent_request_digest: str | None = None,
    parent_authority_binding_digest: str | None = None,
    workflow_raw_file_key: str | None = None,
) -> sqlite3.Row:
    if credential_kind is None:
        if actor_id is None:
            credential_kind = "none"
        else:
            credential_kind = "session_token"
            credential_subject_id = session_id(actor_id)
            credential_key_version = 7
            credential_digest = session_digest(actor_id)
    row = database.execute(
        SQL["authority_read"],
        (
            actor_id,
            authority_class,
            authenticated_tenant_id,
            legacy_tenant_id,
            legacy_target_id,
            tenant_domain,
            target_domain,
            credential_kind,
            credential_subject_id,
            credential_key_version,
            credential_digest,
            credential_expires_at_ms,
            json.dumps(policy_proofs or [], separators=(",", ":"), sort_keys=True),
            entitlement,
            operation_id,
            now_ms,
            legacy_workflow_actor_id,
            legacy_workflow_cap_tenant_id,
            parent_family,
            parent_receipt_id,
            parent_request_digest,
            parent_authority_binding_digest,
            workflow_raw_file_key,
        ),
    ).fetchone()
    assert row is not None
    return row


def policy_proof(kind: str, target_id: str, subject_id: str, revision: int) -> dict:
    return {
        "kind": kind,
        "target_id": target_id,
        "subject_id": subject_id,
        "revision": revision,
        "audit_digest": digest(f"{kind}:{target_id}:{subject_id}:{revision}"),
    }


def prove_authority(database: sqlite3.Connection) -> None:
    database.execute(
        """INSERT INTO legacy_protected_integration_signed_authorities_v1(
             credential_subject_id,credential_kind,credential_key_version,
             credential_digest,state,expires_at_ms
           ) VALUES(
             'media-server-webhook.endpoint.v1','signed_endpoint',1,?1,
             'active',?2
           )""",
        (DIGEST, NOW + 60_000),
    )

    assert authority(database, "public")["authorized"] == 1
    assert authority(database, "public", credential_kind="api_key")["authorized"] == 0
    assert authority(database, "session", actor_id=OWNER)["authorized"] == 1
    assert authority(database, "session", actor_id="missing")["authorized"] == 0
    assert authority(
        database,
        "session",
        actor_id=OWNER,
        credential_kind="session_token",
        credential_subject_id=SESSION,
        credential_key_version=8,
        credential_digest=session_digest(OWNER),
    )["authorized"] == 0
    assert authority(
        database,
        "session",
        actor_id=OWNER,
        credential_kind="api_key",
        credential_subject_id=API_KEY,
        credential_digest=API_KEY_DIGEST,
    )["authorized"] == 1
    assert authority(
        database,
        "session",
        actor_id=OWNER,
        credential_kind="api_key",
        credential_subject_id=API_KEY,
        credential_key_version=1,
        credential_digest=API_KEY_DIGEST,
    )["authorized"] == 0
    assert authority(
        database,
        "signed_state_or_organization_owner",
        actor_id=OWNER,
        authenticated_tenant_id=ORG,
        tenant_domain="organization",
        credential_kind="signed_state",
        credential_subject_id="google-drive-oauth-state.v1",
        credential_key_version=1,
        credential_digest=OTHER_DIGEST,
        credential_expires_at_ms=NOW + 1,
        operation_id="cap-v1-49531a09fd9433e7",
    )["authorized"] == 1
    assert authority(
        database,
        "signed_state_or_organization_owner",
        actor_id=OWNER,
        authenticated_tenant_id=ORG,
        tenant_domain="organization",
        credential_kind="signed_state",
        credential_subject_id="google-drive-oauth-state.v1",
        credential_key_version=1,
        credential_digest=OTHER_DIGEST,
        credential_expires_at_ms=NOW,
        operation_id="cap-v1-49531a09fd9433e7",
    )["authorized"] == 0
    assert authority(
        database,
        "signed_webhook",
        credential_kind="signed_endpoint",
        credential_subject_id="media-server-webhook.endpoint.v1",
        credential_key_version=1,
        credential_digest=DIGEST,
    )["authorized"] == 1
    assert authority(
        database,
        "signed_webhook",
        credential_kind="signed_endpoint",
        credential_subject_id="media-server-webhook.endpoint.v1",
        credential_key_version=1,
        credential_digest=OTHER_DIGEST,
    )["authorized"] == 0

    assert authority(
        database,
        "organization_member",
        actor_id=MEMBER,
        authenticated_tenant_id=ORG,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )["authorized"] == 1
    manager_authority = authority(
        database,
        "organization_manager",
        actor_id=ADMIN,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )
    assert manager_authority["authorized"] == 1
    assert manager_authority["owner_id"] == OWNER
    assert manager_authority["owner_revision"] == 0
    assert authority(
        database,
        "organization_manager",
        actor_id=MEMBER,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )["authorized"] == 0
    assert authority(
        database,
        "organization_owner",
        actor_id=OWNER,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )["authorized"] == 1
    assert authority(
        database,
        "organization_owner",
        actor_id=ADMIN,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )["authorized"] == 0
    assert authority(
        database,
        "organization_member",
        actor_id=OUTSIDER,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )["authorized"] == 0
    resolved = authority(database, "organization_member", actor_id=MEMBER)
    assert resolved["authorized"] == 1 and resolved["resolved_tenant_id"] == ORG

    space_manager_authority = authority(
        database,
        "space_manager",
        actor_id=SPACE_MANAGER,
        legacy_target_id=LEGACY_SPACE,
        target_domain="space",
    )
    assert space_manager_authority["authorized"] == 1
    assert space_manager_authority["owner_id"] == OWNER
    assert space_manager_authority["owner_revision"] == 0
    assert authority(
        database,
        "space_manager",
        actor_id=MEMBER,
        legacy_target_id=LEGACY_SPACE,
        target_domain="space",
    )["authorized"] == 0

    owner_proof = policy_proof("owner_bypass", PRIVATE_VIDEO, PRIVATE_VIDEO, 0)
    member_proof = policy_proof(
        "unprotected_video_policy", PRIVATE_VIDEO, PRIVATE_VIDEO, 0
    )
    public_proof = policy_proof(
        "unprotected_video_policy", PUBLIC_VIDEO, PUBLIC_VIDEO, 0
    )
    assert authority(
        database,
        "video_viewer",
        actor_id=OWNER,
        legacy_target_id=LEGACY_PRIVATE_VIDEO,
        target_domain="video",
        policy_proofs=[owner_proof],
        operation_id="cap-v1-d9b654b30f6c362a",
    )["authorized"] == 1
    assert authority(
        database,
        "video_viewer",
        actor_id=MEMBER,
        legacy_target_id=LEGACY_PRIVATE_VIDEO,
        target_domain="video",
        policy_proofs=[member_proof],
        operation_id="cap-v1-d9b654b30f6c362a",
    )["authorized"] == 1
    assert authority(
        database,
        "video_viewer",
        actor_id=OUTSIDER,
        legacy_target_id=LEGACY_PRIVATE_VIDEO,
        target_domain="video",
        policy_proofs=[member_proof],
        operation_id="cap-v1-d9b654b30f6c362a",
    )["authorized"] == 0
    assert authority(
        database,
        "video_viewer",
        actor_id=OUTSIDER,
        legacy_target_id=LEGACY_PUBLIC_VIDEO,
        target_domain="video",
        policy_proofs=[public_proof],
        operation_id="cap-v1-d9b654b30f6c362a",
    )["authorized"] == 1

    assert authority(database, "session", actor_id=ADMIN, entitlement="cap_internal")["authorized"] == 1
    assert authority(database, "session", actor_id=OWNER, entitlement="cap_internal")["authorized"] == 0
    assert authority(database, "session", actor_id=OWNER, entitlement="pro")["authorized"] == 1
    assert authority(database, "session", actor_id=ADMIN, entitlement="pro")["authorized"] == 1
    assert authority(database, "session", actor_id=MEMBER, entitlement="pro")["authorized"] == 0
    assert authority(database, "session", actor_id=OWNER, entitlement="subscription_read")["authorized"] == 1
    assert authority(database, "session", actor_id=ADMIN, entitlement="subscription_read")["authorized"] == 0
    assert authority(database, "session", actor_id=OWNER, entitlement="subscription_manage")["authorized"] == 1
    assert authority(database, "session", actor_id=MEMBER, entitlement="invalid")["authorized"] == 0

    database.execute("UPDATE users SET status='suspended' WHERE id=?1", (ADMIN,))
    assert authority(database, "organization_manager", actor_id=ADMIN)["authorized"] == 0
    database.execute("UPDATE users SET status='active' WHERE id=?1", (ADMIN,))


def prove_credential_revocation(database: sqlite3.Connection) -> None:
    assert authority(database, "session", actor_id=OWNER)["authorized"] == 1
    database.execute(
        """UPDATE auth_sessions_v2
           SET state='revoked',revoked_at_ms=?2,revocation_reason='operator'
           WHERE id=?1""",
        (SESSION, NOW),
    )
    assert authority(database, "session", actor_id=OWNER)["authorized"] == 0
    database.execute(
        """UPDATE auth_sessions_v2
           SET state='active',revoked_at_ms=NULL,revocation_reason=NULL
           WHERE id=?1""",
        (SESSION,),
    )
    database.execute(
        "UPDATE auth_identities_v2 SET session_version=2 WHERE user_id=?1", (OWNER,)
    )
    assert authority(database, "session", actor_id=OWNER)["authorized"] == 0
    database.execute(
        "UPDATE auth_identities_v2 SET session_version=1 WHERE user_id=?1", (OWNER,)
    )
    database.execute(
        "UPDATE auth_sessions_v2 SET idle_expires_at_ms=?2 WHERE id=?1",
        (SESSION, NOW),
    )
    assert authority(database, "session", actor_id=OWNER)["authorized"] == 0
    database.execute(
        "UPDATE auth_sessions_v2 SET idle_expires_at_ms=?2 WHERE id=?1",
        (SESSION, NOW + 10_000_000),
    )

    def api_key_authorized() -> int:
        return authority(
            database,
            "session",
            actor_id=OWNER,
            credential_kind="api_key",
            credential_subject_id=API_KEY,
            credential_digest=API_KEY_DIGEST,
        )["authorized"]

    assert api_key_authorized() == 1
    database.execute(
        "UPDATE auth_api_keys SET revoked_at_ms=?2 WHERE id=?1", (API_KEY, NOW)
    )
    assert api_key_authorized() == 0
    database.execute(
        "UPDATE auth_api_keys SET revoked_at_ms=NULL,expires_at_ms=?2 WHERE id=?1",
        (API_KEY, NOW),
    )
    assert api_key_authorized() == 0
    database.execute(
        "UPDATE auth_api_keys SET expires_at_ms=?2 WHERE id=?1",
        (API_KEY, NOW + 60_000),
    )
    database.execute(
        """UPDATE legacy_protected_integration_signed_authorities_v1
           SET state='disabled'
           WHERE credential_subject_id='media-server-webhook.endpoint.v1'"""
    )
    assert authority(
        database,
        "signed_webhook",
        credential_kind="signed_endpoint",
        credential_subject_id="media-server-webhook.endpoint.v1",
        credential_key_version=1,
        credential_digest=DIGEST,
    )["authorized"] == 0
    database.execute(
        """UPDATE legacy_protected_integration_signed_authorities_v1
           SET state='active'
           WHERE credential_subject_id='media-server-webhook.endpoint.v1'"""
    )


def prove_entitlement_loss(database: sqlite3.Connection) -> None:
    assert authority(database, "session", actor_id=OWNER, entitlement="pro")["authorized"] == 1
    assert authority(
        database, "session", actor_id=OWNER, entitlement="subscription_manage"
    )["authorized"] == 1
    database.execute(
        """UPDATE users SET
             legacy_stripe_subscription_status='canceled',
             legacy_stripe_subscription_id=NULL,
             legacy_stripe_customer_id=NULL
           WHERE id=?1""",
        (OWNER,),
    )
    assert authority(database, "session", actor_id=OWNER, entitlement="pro")["authorized"] == 0
    assert authority(
        database, "session", actor_id=OWNER, entitlement="subscription_manage"
    )["authorized"] == 0
    database.execute(
        """UPDATE users SET
             legacy_stripe_subscription_status='active',
             legacy_stripe_subscription_id='sub_owner',
             legacy_stripe_customer_id='cus_owner'
           WHERE id=?1""",
        (OWNER,),
    )
    assert authority(database, "session", actor_id=ADMIN, entitlement="pro")["authorized"] == 1
    database.execute(
        "UPDATE users SET legacy_third_party_stripe_subscription_id=NULL WHERE id=?1",
        (ADMIN,),
    )
    assert authority(database, "session", actor_id=ADMIN, entitlement="pro")["authorized"] == 0
    database.execute(
        """UPDATE users SET
             legacy_third_party_stripe_subscription_id='third_party_admin'
           WHERE id=?1""",
        (ADMIN,),
    )


def video_authority(
    database: sqlite3.Connection,
    actor_id: str,
    legacy_video_id: str,
    proof: dict,
) -> int:
    return authority(
        database,
        "video_viewer",
        actor_id=actor_id,
        legacy_target_id=legacy_video_id,
        target_domain="video",
        policy_proofs=[proof],
        operation_id="cap-v1-d9b654b30f6c362a",
    )["authorized"]


def prove_alias_and_policy_revocation(database: sqlite3.Connection) -> None:
    member_org = lambda: authority(
        database,
        "organization_member",
        actor_id=MEMBER,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )["authorized"]
    assert member_org() == 1
    database.execute(
        """UPDATE organization_members SET state='removed'
           WHERE organization_id=?1 AND user_id=?2""",
        (ORG, MEMBER),
    )
    assert member_org() == 0
    database.execute(
        """UPDATE organization_members SET state='active'
           WHERE organization_id=?1 AND user_id=?2""",
        (ORG, MEMBER),
    )
    database.execute("UPDATE organizations SET status='tombstoned' WHERE id=?1", (ORG,))
    assert member_org() == 0
    database.execute("UPDATE organizations SET status='active' WHERE id=?1", (ORG,))
    manager = lambda: authority(
        database,
        "organization_manager",
        actor_id=ADMIN,
        legacy_tenant_id=LEGACY_ORG,
        tenant_domain="organization",
    )["authorized"]
    assert manager() == 1
    database.execute(
        """UPDATE organization_members SET role='member'
           WHERE organization_id=?1 AND user_id=?2""",
        (ORG, ADMIN),
    )
    assert manager() == 0
    database.execute(
        """UPDATE organization_members SET role='admin'
           WHERE organization_id=?1 AND user_id=?2""",
        (ORG, ADMIN),
    )

    managed_space = lambda: authority(
        database,
        "space_manager",
        actor_id=SPACE_MANAGER,
        legacy_target_id=LEGACY_SPACE,
        target_domain="space",
    )["authorized"]
    assert managed_space() == 1
    database.execute("UPDATE spaces SET deleted_at_ms=?2 WHERE id=?1", (SPACE, NOW))
    assert managed_space() == 0
    database.execute("UPDATE spaces SET deleted_at_ms=NULL WHERE id=?1", (SPACE,))

    private_proof = policy_proof(
        "unprotected_video_policy", PRIVATE_VIDEO, PRIVATE_VIDEO, 0
    )
    assert video_authority(database, MEMBER, LEGACY_PRIVATE_VIDEO, private_proof) == 1
    database.execute("UPDATE videos SET deleted_at_ms=?2 WHERE id=?1", (PRIVATE_VIDEO, NOW))
    assert video_authority(database, MEMBER, LEGACY_PRIVATE_VIDEO, private_proof) == 0
    database.execute("UPDATE videos SET deleted_at_ms=NULL WHERE id=?1", (PRIVATE_VIDEO,))

    public_proof = policy_proof(
        "unprotected_video_policy", PUBLIC_VIDEO, PUBLIC_VIDEO, 0
    )
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, public_proof) == 1
    database.execute(
        "UPDATE organizations SET legacy_allowed_email_restriction='cap.so' WHERE id=?1",
        (ORG,),
    )
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, public_proof) == 0
    database.execute(
        "UPDATE organizations SET legacy_allowed_email_restriction=NULL WHERE id=?1", (ORG,)
    )
    database.execute(
        "UPDATE videos SET legacy_public=0,privacy='private' WHERE id=?1", (PUBLIC_VIDEO,)
    )
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, public_proof) == 0
    database.execute(
        "UPDATE videos SET legacy_public=1,privacy='public' WHERE id=?1", (PUBLIC_VIDEO,)
    )

    direct_proof = policy_proof("video_password", PASSWORD_VIDEO, PASSWORD_VIDEO, 7)
    assert video_authority(database, OUTSIDER, LEGACY_PASSWORD_VIDEO, direct_proof) == 1
    database.execute(
        "UPDATE videos SET legacy_property_revision=8 WHERE id=?1", (PASSWORD_VIDEO,)
    )
    assert video_authority(database, OUTSIDER, LEGACY_PASSWORD_VIDEO, direct_proof) == 0
    database.execute(
        "UPDATE videos SET legacy_property_revision=7 WHERE id=?1", (PASSWORD_VIDEO,)
    )

    database.execute(
        "UPDATE spaces SET legacy_password_hash=?2,legacy_password_revision=3 WHERE id=?1",
        (SPACE, "B" * 64),
    )
    database.execute(
        "UPDATE spaces SET legacy_password_hash=?2,legacy_password_revision=5 WHERE id=?1",
        (SECOND_SPACE, "C" * 64),
    )
    for space_id, video_id in (
        (SECOND_SPACE, PUBLIC_VIDEO),
        (SPACE, PASSWORD_VIDEO),
    ):
        database.execute(
            """INSERT INTO space_videos(
                 space_id,video_id,folder_id,added_by_user_id,added_at_ms
               ) VALUES(?1,?2,NULL,?3,1)""",
            (space_id, video_id, OWNER),
        )
    first_space = policy_proof("space_password", PUBLIC_VIDEO, SPACE, 3)
    second_space = policy_proof("space_password", PUBLIC_VIDEO, SECOND_SPACE, 5)
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, second_space) == 1
    database.execute(
        """INSERT INTO space_videos(
             space_id,video_id,folder_id,added_by_user_id,added_at_ms
           ) VALUES(?1,?2,NULL,?3,1)""",
        (SPACE, PUBLIC_VIDEO, OWNER),
    )
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, second_space) == 0
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, first_space) == 1
    database.execute(
        "UPDATE spaces SET legacy_password_revision=4 WHERE id=?1", (SPACE,)
    )
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, first_space) == 0
    database.execute(
        "UPDATE spaces SET legacy_password_revision=3 WHERE id=?1", (SPACE,)
    )
    wrong_precedence = policy_proof("space_password", PASSWORD_VIDEO, SPACE, 3)
    assert video_authority(database, OUTSIDER, LEGACY_PASSWORD_VIDEO, wrong_precedence) == 0
    database.execute(
        "DELETE FROM space_videos WHERE space_id=?1 AND video_id=?2",
        (SPACE, PUBLIC_VIDEO),
    )
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, first_space) == 0
    assert video_authority(database, OUTSIDER, LEGACY_PUBLIC_VIDEO, second_space) == 1
    database.execute(
        "DELETE FROM space_videos WHERE video_id IN (?1,?2)",
        (PUBLIC_VIDEO, PASSWORD_VIDEO),
    )
    database.execute(
        "UPDATE spaces SET legacy_password_hash=NULL WHERE id IN (?1,?2)",
        (SPACE, SECOND_SPACE),
    )


def insert_receipt(
    database: sqlite3.Connection,
    receipt_id: str,
    replay_digest: str,
    request_digest: str = DIGEST,
    **overrides: object,
) -> None:
    values: dict[str, object] = {
        "source_operation_id": "cap-v1-9d91d42d52472a83",
        "operation_kind": "route",
        "method": "POST",
        "surface_path": "/api/desktop/s3/config",
        "auth_class": "session_or_api_key",
        "authority_class": "session",
        "provider_kind": "sealed_s3_configuration",
        "principal_digest": digest("principal"),
        "actor_id": OWNER,
        "tenant_id": None,
        "target_id": None,
        "tenant_domain": "none",
        "target_domain": "none",
        "legacy_tenant_id": None,
        "legacy_target_id": None,
        "legacy_workflow_actor_id": None,
        "legacy_workflow_cap_tenant_id": None,
        "workflow_raw_file_key": None,
        "credential_kind": "session_token",
        "credential_subject_id": SESSION,
        "credential_key_version": 7,
        "credential_digest": session_digest(OWNER),
        "credential_expires_at_ms": None,
        "policy_proofs_json": "[]",
        "entitlement_kind": None,
        "entitlement_subject_id": None,
        "entitlement_revision": None,
        "entitlement_expires_at_ms": None,
        "conditional_bindings_json": "[]",
        "authority_binding_digest": digest("authority-binding"),
        "parent_family": None,
        "parent_receipt_id": None,
        "parent_request_digest": None,
        "parent_authority_binding_digest": None,
        "replay_origin": "generated",
        "idempotency_mode": "required",
        "redacted_request_json": None,
        "sealed_request_ref": f"frame-pi-request-v1:{digest('request-ref')}",
        "sealed_request_digest": digest("provider-request"),
        "transport_body_digest": None,
        "terminal_kind": "http",
        "conditional_duration_seconds": None,
        "conditional_password_requested": 0,
        "conditional_pro_settings_requested": 0,
        "conditional_public_requested": 0,
        "seat_quantity": None,
        "created_at_ms": NOW,
    }
    unknown = set(overrides) - set(values)
    assert not unknown, sorted(unknown)
    values.update(overrides)
    if "redacted_request_json" not in overrides:
        values["redacted_request_json"] = json.dumps(
            {
                "schema_version": "frame.legacy-protected-integration-request.v1",
                "source_operation_id": values["source_operation_id"],
                "payload": {
                    "digest_only": True,
                    "sha256": digest("hostile-payload"),
                },
                "sealed_request_digest": values["sealed_request_digest"],
                "transport_body_digest": values["transport_body_digest"],
                "parent_family": values["parent_family"],
                "parent_receipt_id": values["parent_receipt_id"],
                "parent_request_digest": values["parent_request_digest"],
                "parent_authority_binding_digest": values[
                    "parent_authority_binding_digest"
                ],
            },
            separators=(",", ":"),
            sort_keys=True,
        )
    database.execute(
        SQL["receipt_insert"],
        (
            receipt_id,
            values["source_operation_id"],
            values["operation_kind"],
            values["method"],
            values["surface_path"],
            values["auth_class"],
            values["authority_class"],
            values["provider_kind"],
            values["principal_digest"],
            values["actor_id"],
            values["tenant_id"],
            values["target_id"],
            values["tenant_domain"],
            values["target_domain"],
            values["legacy_tenant_id"],
            values["legacy_target_id"],
            values["legacy_workflow_actor_id"],
            values["legacy_workflow_cap_tenant_id"],
            values["workflow_raw_file_key"],
            values["credential_kind"],
            values["credential_subject_id"],
            values["credential_key_version"],
            values["credential_digest"],
            values["credential_expires_at_ms"],
            values["policy_proofs_json"],
            values["entitlement_kind"],
            values["entitlement_subject_id"],
            values["entitlement_revision"],
            values["entitlement_expires_at_ms"],
            values["conditional_bindings_json"],
            values["authority_binding_digest"],
            values["parent_family"],
            values["parent_receipt_id"],
            values["parent_request_digest"],
            values["parent_authority_binding_digest"],
            replay_digest,
            values["replay_origin"],
            values["idempotency_mode"],
            request_digest,
            values["redacted_request_json"],
            values["sealed_request_ref"],
            values["sealed_request_digest"],
            values["transport_body_digest"],
            values["terminal_kind"],
            values["conditional_duration_seconds"],
            values["conditional_password_requested"],
            values["conditional_pro_settings_requested"],
            values["conditional_public_requested"],
            values["seat_quantity"],
            values["created_at_ms"],
        ),
    )


def insert_outbox(
    database: sqlite3.Connection,
    receipt_id: str,
    payload: str,
) -> None:
    database.execute(
        SQL["outbox_insert"],
        (receipt_id, "sealed_s3_configuration", payload, digest(payload), NOW),
    )


def insert_exact_outbox(
    database: sqlite3.Connection, receipt_id: str, created_at_ms: int = NOW
) -> str:
    receipt = database.execute(
        "SELECT * FROM legacy_protected_integration_receipts_v1 WHERE receipt_id=?1",
        (receipt_id,),
    ).fetchone()
    assert receipt is not None
    payload = json.dumps(
        {
            "schema_version": "frame.legacy-protected-integration-outbox.v1",
            "receipt_id": receipt_id,
            "source_operation_id": receipt["source_operation_id"],
            "kind": receipt["operation_kind"],
            "method": receipt["method"],
            "path": receipt["surface_path"],
            "provider": receipt["provider_kind"],
            "principal_digest": receipt["principal_digest"],
            "tenant_id": receipt["tenant_id"],
            "target_id": receipt["target_id"],
            "legacy_tenant_id": receipt["legacy_tenant_id"],
            "legacy_target_id": receipt["legacy_target_id"],
            "request_digest": receipt["request_digest"],
            "authority_binding_digest": receipt["authority_binding_digest"],
            "conditional_bindings": json.loads(receipt["conditional_bindings_json"]),
            "redacted_request": json.loads(receipt["redacted_request_json"]),
            "sealed_request_ref": receipt["sealed_request_ref"],
            "sealed_request_digest": receipt["sealed_request_digest"],
            "release_gate": "independent_provider_executor_evidence",
        },
        separators=(",", ":"),
        sort_keys=True,
    )
    database.execute(
        SQL["outbox_insert"],
        (
            receipt_id,
            receipt["provider_kind"],
            payload,
            digest(payload),
            created_at_ms,
        ),
    )
    return digest(payload)


def prove_generated_and_natural_replay(database: sqlite3.Connection) -> None:
    receipt_id = identifier(100)
    replay_digest = digest("save-s3-1")
    authority_digest = digest("authority-binding")
    insert_receipt(database, receipt_id, replay_digest)
    redacted_request = json.loads(
        database.execute(
            """SELECT redacted_request_json
               FROM legacy_protected_integration_receipts_v1 WHERE receipt_id=?1""",
            (receipt_id,),
        ).fetchone()[0]
    )
    payload = json.dumps(
        {
            "schema_version": "frame.legacy-protected-integration-outbox.v1",
            "receipt_id": receipt_id,
            "source_operation_id": "cap-v1-9d91d42d52472a83",
            "kind": "route",
            "method": "POST",
            "path": "/api/desktop/s3/config",
            "provider": "sealed_s3_configuration",
            "principal_digest": digest("principal"),
            "tenant_id": None,
            "target_id": None,
            "legacy_tenant_id": None,
            "legacy_target_id": None,
            "request_digest": DIGEST,
            "authority_binding_digest": authority_digest,
            "conditional_bindings": [],
            "redacted_request": redacted_request,
            "sealed_request_ref": f"frame-pi-request-v1:{digest('request-ref')}",
            "sealed_request_digest": digest("provider-request"),
            "release_gate": "independent_provider_executor_evidence",
        },
        separators=(",", ":"),
        sort_keys=True,
    )
    insert_outbox(database, receipt_id, payload)
    database.execute(
        SQL["generated_claim_upsert"],
        (
            "cap-v1-9d91d42d52472a83",
            digest("principal"),
            DIGEST,
            receipt_id,
            NOW,
            NOW - 900_000,
        ),
    )

    stored = database.execute(
        """SELECT redacted_request_json,sealed_request_ref,sealed_request_digest
           FROM legacy_protected_integration_receipts_v1 WHERE receipt_id=?1""",
        (receipt_id,),
    ).fetchone()
    assert stored["sealed_request_ref"] == f"frame-pi-request-v1:{digest('request-ref')}"
    assert stored["sealed_request_digest"] == digest("provider-request")
    assert set(json.loads(stored["redacted_request_json"])["payload"]) == {
        "digest_only",
        "sha256",
    }
    outbox = database.execute(
        "SELECT payload_json FROM legacy_protected_integration_outbox_v1 WHERE receipt_id=?1",
        (receipt_id,),
    ).fetchone()[0]

    replay = database.execute(
        SQL["generated_receipt_replay"],
        (
            "cap-v1-9d91d42d52472a83",
            digest("principal"),
            DIGEST,
            authority_digest,
            NOW,
            NOW - 900_000,
        ),
    ).fetchone()
    assert replay["state"] == "pending_provider_evidence"

    try:
        insert_receipt(
            database,
            identifier(101),
            digest("race-replay-key"),
        )
    except sqlite3.IntegrityError as error:
        assert "generated_replay_claimed" in str(error)
    else:
        raise AssertionError("generated pending claim was bypassed")

    database.execute(
        """UPDATE auth_sessions_v2
           SET state='revoked',revoked_at_ms=?2,revocation_reason='operator'
           WHERE id=?1""",
        (SESSION, NOW),
    )
    replay = database.execute(
        SQL["generated_receipt_replay"],
        (
            "cap-v1-9d91d42d52472a83",
            digest("principal"),
            DIGEST,
            authority_digest,
            NOW,
            NOW - 900_000,
        ),
    ).fetchone()
    assert replay is None
    database.execute(
        """UPDATE auth_sessions_v2
           SET state='active',revoked_at_ms=NULL,revocation_reason=NULL
           WHERE id=?1""",
        (SESSION,),
    )

    try:
        insert_receipt(
            database,
            identifier(102),
            digest("bad-sealed-ref"),
            request_digest=OTHER_DIGEST,
            principal_digest=digest("bad-sealed-principal"),
            sealed_request_ref="https://vault.example/object?secret=plaintext",
        )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("non-opaque request reference was accepted")

    atomic_receipt = identifier(103)
    try:
        database.execute("BEGIN")
        insert_receipt(
            database,
            atomic_receipt,
            digest("atomic"),
            request_digest=digest("atomic-request"),
            principal_digest=digest("atomic-principal"),
        )
        database.execute(
            SQL["outbox_insert"],
            (atomic_receipt, "", "{}", digest("{}"), NOW),
        )
        database.execute("COMMIT")
    except sqlite3.IntegrityError:
        database.execute("ROLLBACK")
    assert database.execute(
        "SELECT 1 FROM legacy_protected_integration_receipts_v1 WHERE receipt_id=?1",
        (atomic_receipt,),
    ).fetchone() is None

    state_receipt = identifier(110)
    state_principal = digest("signed-state-principal")
    state_request = digest("signed-state-request")
    state_authority = digest("signed-state-authority")
    insert_receipt(
        database,
        state_receipt,
        digest("signed-state-replay"),
        request_digest=state_request,
        source_operation_id="cap-v1-49531a09fd9433e7",
        method="GET",
        surface_path="/api/desktop/storage/google-drive/callback",
        auth_class="signed_state",
        authority_class="signed_state_or_organization_owner",
        provider_kind="google_drive_oauth_exchange",
        principal_digest=state_principal,
        tenant_id=ORG,
        tenant_domain="organization",
        credential_kind="signed_state",
        credential_subject_id="google-drive-oauth-state.v1",
        credential_key_version=1,
        credential_digest=OTHER_DIGEST,
        credential_expires_at_ms=NOW + 100,
        authority_binding_digest=state_authority,
    )
    database.execute(
        SQL["generated_claim_upsert"],
        (
            "cap-v1-49531a09fd9433e7",
            state_principal,
            state_request,
            state_receipt,
            NOW,
            NOW - 900_000,
        ),
    )
    assert database.execute(
        SQL["generated_receipt_replay"],
        (
            "cap-v1-49531a09fd9433e7",
            state_principal,
            state_request,
            state_authority,
            NOW,
            NOW - 900_000,
        ),
    ).fetchone() is not None
    assert database.execute(
        SQL["generated_receipt_replay"],
        (
            "cap-v1-49531a09fd9433e7",
            state_principal,
            state_request,
            state_authority,
            NOW + 101,
            NOW - 900_000,
        ),
    ).fetchone() is None

    webhook_receipt = identifier(111)
    webhook_principal = digest("webhook-principal")
    webhook_replay = digest("webhook-natural")
    webhook_authority = digest("webhook-authority")
    insert_receipt(
        database,
        webhook_receipt,
        webhook_replay,
        request_digest=digest("webhook-request"),
        source_operation_id="cap-v1-17d69edf5d3b06bb",
        method="POST",
        surface_path="/api/webhooks/media-server/progress",
        auth_class="signed_webhook",
        authority_class="signed_webhook",
        provider_kind="media_server_webhook",
        principal_digest=webhook_principal,
        actor_id=None,
        credential_kind="signed_endpoint",
        credential_subject_id="media-server-webhook.endpoint.v1",
        credential_key_version=1,
        credential_digest=DIGEST,
        replay_origin="natural",
        authority_binding_digest=webhook_authority,
        transport_body_digest=digest("raw-webhook-body"),
    )
    assert database.execute(
        SQL["receipt_replay"],
        (
            "cap-v1-17d69edf5d3b06bb",
            webhook_principal,
            webhook_replay,
            webhook_authority,
            NOW,
        ),
    ).fetchone() is not None
    database.execute(
        """UPDATE legacy_protected_integration_signed_authorities_v1
           SET credential_digest=?1
           WHERE credential_subject_id='media-server-webhook.endpoint.v1'""",
        (OTHER_DIGEST,),
    )
    expect_integrity(
        lambda: insert_receipt(
            database,
            identifier(112),
            webhook_replay,
            request_digest=digest("webhook-rotated-credential-request"),
            source_operation_id="cap-v1-17d69edf5d3b06bb",
            method="POST",
            surface_path="/api/webhooks/media-server/progress",
            auth_class="signed_webhook",
            authority_class="signed_webhook",
            provider_kind="media_server_webhook",
            principal_digest=digest("webhook-rotated-principal"),
            actor_id=None,
            credential_kind="signed_endpoint",
            credential_subject_id="media-server-webhook.endpoint.v1",
            credential_key_version=1,
            credential_digest=OTHER_DIGEST,
            replay_origin="natural",
            authority_binding_digest=digest("webhook-rotated-authority"),
            transport_body_digest=digest("raw-webhook-body"),
        ),
        "UNIQUE constraint failed",
    )
    database.execute(
        """UPDATE legacy_protected_integration_signed_authorities_v1
           SET credential_digest=?1
           WHERE credential_subject_id='media-server-webhook.endpoint.v1'""",
        (DIGEST,),
    )
    database.execute(
        """UPDATE legacy_protected_integration_signed_authorities_v1
           SET state='disabled'
           WHERE credential_subject_id='media-server-webhook.endpoint.v1'"""
    )
    assert database.execute(
        SQL["receipt_replay"],
        (
            "cap-v1-17d69edf5d3b06bb",
            webhook_principal,
            webhook_replay,
            webhook_authority,
            NOW,
        ),
    ).fetchone() is None
    database.execute(
        """UPDATE legacy_protected_integration_signed_authorities_v1
           SET state='active'
           WHERE credential_subject_id='media-server-webhook.endpoint.v1'"""
    )


def conditional_binding(
    kind: str, subject_id: str, revision: int, value: int | None = None
) -> dict:
    return {
        "kind": kind,
        "subject_id": subject_id,
        "revision": revision,
        "value": value,
    }


def prove_conditional_authority(database: sqlite3.Connection) -> None:
    def f60_authority(
        actor_id: str,
        legacy_org_id: str | None = None,
        legacy_video_id: str | None = None,
    ) -> sqlite3.Row:
        return authority(
            database,
            "organization_member",
            actor_id=actor_id,
            authenticated_tenant_id=OTHER_ORG,
            legacy_tenant_id=legacy_org_id,
            legacy_target_id=legacy_video_id,
            tenant_domain="organization",
            target_domain="video",
            operation_id="cap-v1-60f863b2cb19353f",
        )

    # Existing-video reuse derives the video's organization and ignores even
    # an inaccessible org selector. Ownership alone is sufficient on this
    # early-return source branch.
    database.execute(
        "UPDATE organization_members SET state='removed' WHERE organization_id=?1 AND user_id=?2",
        (ORG, MEMBER),
    )
    existing_owned = f60_authority(MEMBER, LEGACY_OTHER_ORG, LEGACY_MEMBER_VIDEO)
    assert existing_owned["authorized"] == 1
    assert existing_owned["resolved_tenant_id"] == ORG
    assert existing_owned["resolved_target_id"] == MEMBER_VIDEO
    assert f60_authority(ADMIN, LEGACY_ORG, LEGACY_MEMBER_VIDEO)["authorized"] == 0
    member_existing_binding = conditional_binding(
        "video_existing_owner", MEMBER_VIDEO, 0
    )
    insert_receipt(
        database,
        identifier(117),
        digest("video-existing-removed-member-replay"),
        request_digest=digest("video-existing-removed-member-request"),
        source_operation_id="cap-v1-60f863b2cb19353f",
        method="GET",
        surface_path="/api/desktop/video/create",
        authority_class="organization_member",
        provider_kind="storage_signing_email_and_short_link",
        principal_digest=digest("video-existing-removed-member-principal"),
        actor_id=MEMBER,
        credential_subject_id=session_id(MEMBER),
        credential_digest=session_digest(MEMBER),
        tenant_id=ORG,
        target_id=MEMBER_VIDEO,
        tenant_domain="organization",
        target_domain="video",
        legacy_target_id=LEGACY_MEMBER_VIDEO,
        conditional_bindings_json=json.dumps(
            [member_existing_binding], separators=(",", ":")
        ),
    )
    database.execute(
        "UPDATE organization_members SET state='active' WHERE organization_id=?1 AND user_id=?2",
        (ORG, MEMBER),
    )

    # Unknown video IDs are lookup hints, not caller-selected IDs. They fall
    # through to target-null creation in the explicit/default/fallback tenant.
    unknown_video = "01h000000000009"
    explicit = f60_authority(MEMBER, LEGACY_ORG, unknown_video)
    assert explicit["authorized"] == 1
    assert explicit["resolved_tenant_id"] == ORG
    assert explicit["resolved_target_id"] is None
    assert f60_authority(MEMBER, LEGACY_OTHER_ORG, unknown_video)["authorized"] == 0

    database.execute(
        "UPDATE users SET active_organization_id=?2,default_organization_id=?3 WHERE id=?1",
        (MEMBER, OTHER_ORG, ORG),
    )
    default_scope = f60_authority(MEMBER)
    assert default_scope["authorized"] == 1
    assert default_scope["resolved_tenant_id"] == ORG
    database.execute(
        "UPDATE users SET default_organization_id=?2 WHERE id=?1", (MEMBER, OTHER_ORG)
    )
    invalid_default_fallback = f60_authority(MEMBER)
    assert invalid_default_fallback["authorized"] == 1
    assert invalid_default_fallback["resolved_tenant_id"] == ORG
    database.execute(
        "UPDATE users SET default_organization_id=NULL WHERE id=?1", (MEMBER,)
    )
    null_default_fallback = f60_authority(MEMBER)
    assert null_default_fallback["authorized"] == 1
    assert null_default_fallback["resolved_tenant_id"] == ORG
    database.execute(
        "UPDATE users SET active_organization_id=?2,default_organization_id=?2 WHERE id=?1",
        (MEMBER, ORG),
    )

    existing = conditional_binding("video_existing_owner", PRIVATE_VIDEO, 0)
    insert_receipt(
        database,
        identifier(120),
        digest("video-existing-replay"),
        request_digest=digest("video-existing-request"),
        source_operation_id="cap-v1-60f863b2cb19353f",
        method="GET",
        surface_path="/api/desktop/video/create",
        authority_class="organization_member",
        provider_kind="storage_signing_email_and_short_link",
        principal_digest=digest("video-existing-principal"),
        tenant_id=ORG,
        target_id=PRIVATE_VIDEO,
        tenant_domain="organization",
        target_domain="video",
        legacy_tenant_id=None,
        legacy_target_id=LEGACY_PRIVATE_VIDEO,
        conditional_bindings_json=json.dumps([existing], separators=(",", ":")),
    )
    try:
        insert_receipt(
            database,
            identifier(121),
            digest("video-not-owner-replay"),
            request_digest=digest("video-not-owner-request"),
            source_operation_id="cap-v1-60f863b2cb19353f",
            method="GET",
            surface_path="/api/desktop/video/create",
            authority_class="organization_member",
            provider_kind="storage_signing_email_and_short_link",
            principal_digest=digest("video-not-owner-principal"),
            actor_id=MEMBER,
            credential_subject_id=session_id(MEMBER),
            credential_digest=session_digest(MEMBER),
            tenant_id=ORG,
            target_id=PRIVATE_VIDEO,
            tenant_domain="organization",
            target_domain="video",
            legacy_tenant_id=None,
            legacy_target_id=LEGACY_PRIVATE_VIDEO,
            conditional_bindings_json=json.dumps([existing], separators=(",", ":")),
        )
    except sqlite3.IntegrityError as error:
        assert "authority_stale" in str(error)
    else:
        raise AssertionError("non-owner desktop video binding was admitted")

    new_org = conditional_binding("video_new_organization_member", ORG, 0)
    insert_receipt(
        database,
        identifier(122),
        digest("video-new-org-replay"),
        request_digest=digest("video-new-org-request"),
        source_operation_id="cap-v1-60f863b2cb19353f",
        method="GET",
        surface_path="/api/desktop/video/create",
        authority_class="organization_member",
        provider_kind="storage_signing_email_and_short_link",
        principal_digest=digest("video-new-org-principal"),
        actor_id=MEMBER,
        credential_subject_id=session_id(MEMBER),
        credential_digest=session_digest(MEMBER),
        tenant_id=ORG,
        tenant_domain="organization",
        legacy_tenant_id=LEGACY_ORG,
        conditional_bindings_json=json.dumps([new_org], separators=(",", ":")),
    )

    duration_pro = conditional_binding("video_duration_pro", OWNER, 0)
    insert_receipt(
        database,
        identifier(123),
        digest("video-duration-replay"),
        request_digest=digest("video-duration-request"),
        source_operation_id="cap-v1-60f863b2cb19353f",
        method="GET",
        surface_path="/api/desktop/video/create",
        authority_class="organization_member",
        provider_kind="storage_signing_email_and_short_link",
        principal_digest=digest("video-duration-principal"),
        tenant_id=ORG,
        target_id=PRIVATE_VIDEO,
        tenant_domain="organization",
        target_domain="video",
        legacy_tenant_id=None,
        legacy_target_id=LEGACY_PRIVATE_VIDEO,
        conditional_bindings_json=json.dumps(
            [existing, duration_pro], separators=(",", ":")
        ),
        conditional_duration_seconds=301,
    )
    database.execute(
        "UPDATE users SET legacy_stripe_subscription_status='canceled' WHERE id=?1",
        (OWNER,),
    )
    assert database.execute(
        """SELECT 1 FROM legacy_protected_integration_live_authority_v1
           WHERE receipt_id=?1""",
        (identifier(123),),
    ).fetchone() is None
    database.execute(
        "UPDATE users SET legacy_stripe_subscription_status='active' WHERE id=?1",
        (OWNER,),
    )

    # Exact operation-to-binding cardinality rejects a recognized but
    # semantically substituted binding and an ignored unknown target hint.
    expect_integrity(
        lambda: insert_receipt(
            database,
            identifier(118),
            digest("video-existing-substituted-binding-replay"),
            request_digest=digest("video-existing-substituted-binding-request"),
            source_operation_id="cap-v1-60f863b2cb19353f",
            method="GET",
            surface_path="/api/desktop/video/create",
            authority_class="organization_member",
            provider_kind="storage_signing_email_and_short_link",
            principal_digest=digest("video-existing-substituted-binding-principal"),
            tenant_id=ORG,
            target_id=PRIVATE_VIDEO,
            tenant_domain="organization",
            target_domain="video",
            legacy_target_id=LEGACY_PRIVATE_VIDEO,
            conditional_bindings_json=json.dumps([new_org], separators=(",", ":")),
        ),
        "authority_stale",
    )
    expect_integrity(
        lambda: insert_receipt(
            database,
            identifier(119),
            digest("video-unknown-retained-target-replay"),
            request_digest=digest("video-unknown-retained-target-request"),
            source_operation_id="cap-v1-60f863b2cb19353f",
            method="GET",
            surface_path="/api/desktop/video/create",
            authority_class="organization_member",
            provider_kind="storage_signing_email_and_short_link",
            principal_digest=digest("video-unknown-retained-target-principal"),
            actor_id=MEMBER,
            credential_subject_id=session_id(MEMBER),
            credential_digest=session_digest(MEMBER),
            tenant_id=ORG,
            tenant_domain="organization",
            target_domain="video",
            legacy_tenant_id=LEGACY_ORG,
            legacy_target_id=unknown_video,
            conditional_bindings_json=json.dumps([new_org], separators=(",", ":")),
        ),
        "authority_stale",
    )

    space_password = conditional_binding("space_password_pro", ADMIN, 0)
    space_settings = conditional_binding("space_settings_pro", ADMIN, 0)
    space_public = conditional_binding("space_publish_owner_pro", OWNER, 0)
    space_receipt = identifier(124)
    insert_receipt(
        database,
        space_receipt,
        digest("space-create-replay"),
        request_digest=digest("space-create-request"),
        source_operation_id="cap-v1-0c233c1115838206",
        operation_kind="server_action",
        method="ACTION",
        surface_path="action://apps/web/actions/organization/create-space.ts#createSpace",
        auth_class="session",
        authority_class="organization_member",
        provider_kind="space_icon_storage",
        principal_digest=digest("space-create-admin-principal"),
        actor_id=ADMIN,
        credential_subject_id=session_id(ADMIN),
        credential_digest=session_digest(ADMIN),
        tenant_id=ORG,
        tenant_domain="none",
        conditional_bindings_json=json.dumps(
            [space_password, space_settings, space_public], separators=(",", ":")
        ),
        terminal_kind="json",
        conditional_password_requested=1,
        conditional_pro_settings_requested=1,
        conditional_public_requested=1,
    )
    database.execute(
        """UPDATE users SET legacy_third_party_stripe_subscription_id=NULL
           WHERE id=?1""",
        (ADMIN,),
    )
    assert database.execute(
        """SELECT 1 FROM legacy_protected_integration_live_authority_v1
           WHERE receipt_id=?1""",
        (space_receipt,),
    ).fetchone() is None
    database.execute(
        """UPDATE users SET
             legacy_third_party_stripe_subscription_id='third_party_admin'
           WHERE id=?1""",
        (ADMIN,),
    )
    database.execute(
        "UPDATE users SET legacy_stripe_subscription_status='canceled' WHERE id=?1",
        (OWNER,),
    )
    assert database.execute(
        """SELECT 1 FROM legacy_protected_integration_live_authority_v1
           WHERE receipt_id=?1""",
        (space_receipt,),
    ).fetchone() is None
    database.execute(
        "UPDATE users SET legacy_stripe_subscription_status='active' WHERE id=?1",
        (OWNER,),
    )

    # The delegated create carrier has the same selector-free member contract
    # and does not require actor-Pro when no gated option was requested.
    insert_receipt(
        database,
        identifier(240),
        digest("delegated-space-create-replay"),
        request_digest=digest("delegated-space-create-request"),
        source_operation_id="cap-v1-5e7e4265d65c8365",
        operation_kind="server_action",
        method="ACTION",
        surface_path=(
            "action://apps/web/app/(org)/dashboard/_components/Navbar/server.ts"
            "#createSpace"
        ),
        auth_class="session",
        authority_class="organization_member",
        provider_kind="space_icon_storage",
        principal_digest=digest("delegated-space-create-member-principal"),
        actor_id=MEMBER,
        credential_subject_id=session_id(MEMBER),
        credential_digest=session_digest(MEMBER),
        tenant_id=ORG,
        tenant_domain="none",
        terminal_kind="json",
    )

    def substituted_create(receipt_number: int, bindings: list[dict]) -> None:
        insert_receipt(
            database,
            identifier(receipt_number),
            digest(f"space-create-substitution-{receipt_number}-replay"),
            request_digest=digest(f"space-create-substitution-{receipt_number}-request"),
            source_operation_id="cap-v1-0c233c1115838206",
            operation_kind="server_action",
            method="ACTION",
            surface_path="action://apps/web/actions/organization/create-space.ts#createSpace",
            auth_class="session",
            authority_class="organization_member",
            provider_kind="space_icon_storage",
            principal_digest=digest(f"space-create-substitution-{receipt_number}-principal"),
            actor_id=ADMIN,
            credential_subject_id=session_id(ADMIN),
            credential_digest=session_digest(ADMIN),
            tenant_id=ORG,
            tenant_domain="none",
            conditional_bindings_json=json.dumps(bindings, separators=(",", ":")),
            terminal_kind="json",
            conditional_password_requested=1 if len(bindings) == 2 else 0,
            conditional_public_requested=1,
        )

    expect_integrity(
        lambda: substituted_create(241, [space_password]), "authority_stale"
    )
    expect_integrity(
        lambda: substituted_create(242, [space_password, space_password]),
        "authority_stale",
    )

    expect_integrity(
        lambda: insert_receipt(
            database,
            identifier(243),
            digest("space-update-substitution-replay"),
            request_digest=digest("space-update-substitution-request"),
            source_operation_id="cap-v1-3a394a2798233b0b",
            operation_kind="server_action",
            method="ACTION",
            surface_path="action://apps/web/actions/organization/update-space.ts#updateSpace",
            auth_class="session",
            authority_class="space_manager",
            provider_kind="space_icon_storage",
            principal_digest=digest("space-update-substitution-principal"),
            tenant_id=ORG,
            target_id=SPACE,
            tenant_domain="organization",
            target_domain="space",
            legacy_tenant_id=LEGACY_ORG,
            legacy_target_id=LEGACY_SPACE,
            conditional_bindings_json=json.dumps(
                [conditional_binding("space_password_pro", OWNER, 0)],
                separators=(",", ":"),
            ),
            terminal_kind="json",
            conditional_public_requested=1,
        ),
        "authority_stale",
    )

    expect_integrity(
        lambda: insert_receipt(
            database,
            identifier(244),
            digest("seat-binding-substitution-replay"),
            request_digest=digest("seat-binding-substitution-request"),
            source_operation_id="cap-v1-17470f7df902263e",
            operation_kind="server_action",
            method="ACTION",
            surface_path=(
                "action://apps/web/actions/organization/update-seat-quantity.ts"
                "#updateSeatQuantity"
            ),
            auth_class="session",
            authority_class="organization_owner",
            provider_kind="stripe_subscription_update",
            principal_digest=digest("seat-binding-substitution-principal"),
            tenant_id=ORG,
            tenant_domain="organization",
            legacy_tenant_id=LEGACY_ORG,
            entitlement_kind="subscription_manage",
            entitlement_subject_id=OWNER,
            entitlement_revision=0,
            conditional_bindings_json=json.dumps(
                [conditional_binding("space_password_pro", OWNER, 0)],
                separators=(",", ":"),
            ),
            terminal_kind="json",
            seat_quantity=4,
        ),
        "authority_stale",
    )

    update_public = conditional_binding("space_publish_owner_pro", OWNER, 0)
    update_receipt = identifier(127)
    insert_receipt(
        database,
        update_receipt,
        digest("space-update-replay"),
        request_digest=digest("space-update-request"),
        source_operation_id="cap-v1-3a394a2798233b0b",
        operation_kind="server_action",
        method="ACTION",
        surface_path="action://apps/web/actions/organization/update-space.ts#updateSpace",
        auth_class="session",
        authority_class="space_manager",
        provider_kind="space_icon_storage",
        principal_digest=digest("space-update-manager-principal"),
        actor_id=SPACE_MANAGER,
        credential_subject_id=session_id(SPACE_MANAGER),
        credential_digest=session_digest(SPACE_MANAGER),
        tenant_id=ORG,
        target_id=SPACE,
        tenant_domain="organization",
        target_domain="space",
        legacy_tenant_id=LEGACY_ORG,
        legacy_target_id=LEGACY_SPACE,
        conditional_bindings_json=json.dumps([update_public], separators=(",", ":")),
        terminal_kind="json",
        conditional_public_requested=1,
    )
    database.execute("UPDATE spaces SET is_public=1 WHERE id=?1", (SPACE,))
    assert database.execute(
        """SELECT 1 FROM legacy_protected_integration_live_authority_v1
           WHERE receipt_id=?1""",
        (update_receipt,),
    ).fetchone() is None
    database.execute("UPDATE spaces SET is_public=0 WHERE id=?1", (SPACE,))

    seat_binding = conditional_binding("seat_capacity", ORG, 0, 4)
    seat_receipt = identifier(125)
    insert_receipt(
        database,
        seat_receipt,
        digest("seat-replay"),
        request_digest=digest("seat-request"),
        source_operation_id="cap-v1-17470f7df902263e",
        operation_kind="server_action",
        method="ACTION",
        surface_path=(
            "action://apps/web/actions/organization/update-seat-quantity.ts"
            "#updateSeatQuantity"
        ),
        auth_class="session",
        authority_class="organization_owner",
        provider_kind="stripe_subscription_update",
        principal_digest=digest("seat-principal"),
        tenant_id=ORG,
        tenant_domain="organization",
        legacy_tenant_id=LEGACY_ORG,
        entitlement_kind="subscription_manage",
        entitlement_subject_id=OWNER,
        entitlement_revision=0,
        conditional_bindings_json=json.dumps([seat_binding], separators=(",", ":")),
        terminal_kind="json",
        seat_quantity=4,
    )
    database.execute(
        """INSERT INTO organization_members(
             organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms
           ) VALUES(?1,?2,'member','active',1,1,1)""",
        (ORG, OUTSIDER),
    )
    assert database.execute(
        """SELECT 1 FROM legacy_protected_integration_live_authority_v1
           WHERE receipt_id=?1""",
        (seat_receipt,),
    ).fetchone() is None
    database.execute(
        """UPDATE organization_members SET state='removed'
           WHERE organization_id=?1 AND user_id=?2""",
        (ORG, OUTSIDER),
    )
    try:
        insert_receipt(
            database,
            identifier(126),
            digest("seat-too-small-replay"),
            request_digest=digest("seat-too-small-request"),
            source_operation_id="cap-v1-17470f7df902263e",
            operation_kind="server_action",
            method="ACTION",
            surface_path=(
                "action://apps/web/actions/organization/update-seat-quantity.ts"
                "#updateSeatQuantity"
            ),
            auth_class="session",
            authority_class="organization_owner",
            provider_kind="stripe_subscription_update",
            principal_digest=digest("seat-too-small-principal"),
            tenant_id=ORG,
            tenant_domain="organization",
            legacy_tenant_id=LEGACY_ORG,
            entitlement_kind="subscription_manage",
            entitlement_subject_id=OWNER,
            entitlement_revision=0,
            conditional_bindings_json=json.dumps(
                [conditional_binding("seat_capacity", ORG, 0, 3)],
                separators=(",", ":"),
            ),
            terminal_kind="json",
            seat_quantity=3,
        )
    except sqlite3.IntegrityError as error:
        assert "authority_stale" in str(error)
    else:
        raise AssertionError("seat quantity below live proSeatsUsed was admitted")


def insert_parent_registry(
    database: sqlite3.Connection,
    family: str,
    receipt_id: str,
    operation_id: str,
    request_digest: str,
    authority_digest: str,
    target_id: str | None,
) -> None:
    database.execute(
        """INSERT INTO legacy_protected_effect_parent_registry_v1(
             parent_family,parent_receipt_id,source_operation_id,request_digest,
             actor_id,tenant_id,target_id,auth_class,authority_class,
             credential_kind,credential_subject_id,credential_key_version,
             credential_digest,credential_expires_at_ms,policy_proofs_json,
             entitlement_kind,entitlement_subject_id,entitlement_revision,
             entitlement_expires_at_ms,authority_binding_digest,state,
             created_at_ms,completed_at_ms
           ) VALUES(
             ?1,?2,?3,?4,?5,?6,?7,'session','video_owner',
             'session_token',?8,7,?9,NULL,'[]',
             NULL,NULL,NULL,NULL,?10,'pending_execution_evidence',?11,NULL
           )""",
        (
            family,
            receipt_id,
            operation_id,
            request_digest,
            OWNER,
            ORG,
            target_id,
            SESSION,
            session_digest(OWNER),
            authority_digest,
            NOW - 1,
        ),
    )


def prove_workflow_parent_registry(database: sqlite3.Connection) -> None:
    mirrored = database.execute(
        """SELECT * FROM legacy_protected_effect_parent_registry_v1
           WHERE parent_family='protected_integrations' AND parent_receipt_id=?1""",
        (identifier(100),),
    ).fetchone()
    assert mirrored is not None
    assert mirrored["credential_kind"] == "session_token"
    assert mirrored["credential_subject_id"] == SESSION
    assert mirrored["credential_expires_at_ms"] is None

    api_key_receipt = identifier(213)
    insert_receipt(
        database,
        api_key_receipt,
        digest("api-key-registry-replay"),
        request_digest=digest("api-key-registry-request"),
        principal_digest=digest("api-key-registry-principal"),
        credential_kind="api_key",
        credential_subject_id=API_KEY,
        credential_key_version=None,
        credential_digest=API_KEY_DIGEST,
    )
    api_key_mirror = database.execute(
        """SELECT credential_kind,credential_subject_id,credential_key_version,
                  credential_digest,credential_expires_at_ms
           FROM legacy_protected_effect_parent_registry_v1
           WHERE parent_family='protected_integrations' AND parent_receipt_id=?1""",
        (api_key_receipt,),
    ).fetchone()
    assert api_key_mirror is not None
    assert tuple(api_key_mirror) == (
        "api_key",
        API_KEY,
        None,
        API_KEY_DIGEST,
        None,
    )

    public_receipt = identifier(214)
    insert_receipt(
        database,
        public_receipt,
        digest("public-registry-replay"),
        request_digest=digest("public-registry-request"),
        source_operation_id="cap-v1-8a1e6c87b4426f93",
        method="GET",
        surface_path="/api/releases/tauri/:version/:target/:arch",
        auth_class="public",
        authority_class="public",
        provider_kind="github_releases",
        principal_digest=digest("public-registry-principal"),
        actor_id=None,
        credential_kind="none",
        credential_subject_id=None,
        credential_key_version=None,
        credential_digest=None,
        credential_expires_at_ms=None,
        idempotency_mode="forbidden",
    )
    public_mirror = database.execute(
        """SELECT auth_class,authority_class,actor_id,credential_kind,
                  credential_subject_id,credential_key_version,credential_digest,
                  credential_expires_at_ms
           FROM legacy_protected_effect_parent_registry_v1
           WHERE parent_family='protected_integrations' AND parent_receipt_id=?1""",
        (public_receipt,),
    ).fetchone()
    assert public_mirror is not None
    assert tuple(public_mirror) == (
        "public",
        "public",
        None,
        "none",
        None,
        None,
        None,
        None,
    )

    signed_state_mirror = database.execute(
        """SELECT credential_kind,credential_subject_id,credential_key_version,
                  credential_digest,credential_expires_at_ms
           FROM legacy_protected_effect_parent_registry_v1
           WHERE parent_family='protected_integrations' AND parent_receipt_id=?1""",
        (identifier(110),),
    ).fetchone()
    assert signed_state_mirror is not None
    assert tuple(signed_state_mirror) == (
        "signed_state",
        "google-drive-oauth-state.v1",
        1,
        OTHER_DIGEST,
        NOW + 100,
    )

    signed_endpoint_mirror = database.execute(
        """SELECT credential_kind,credential_subject_id,credential_key_version,
                  credential_digest,credential_expires_at_ms
           FROM legacy_protected_effect_parent_registry_v1
           WHERE parent_family='protected_integrations' AND parent_receipt_id=?1""",
        (identifier(111),),
    ).fetchone()
    assert signed_endpoint_mirror is not None
    assert tuple(signed_endpoint_mirror) == (
        "signed_endpoint",
        "media-server-webhook.endpoint.v1",
        1,
        DIGEST,
        None,
    )

    cross_edges = database.execute(
        """SELECT child_operation_id,target_binding_rule
           FROM legacy_protected_effect_parent_edges_v1
           WHERE parent_family='protected_integrations'
             AND parent_operation_id='cap-v1-d9b654b30f6c362a'
             AND child_family='protected_media'
           ORDER BY child_operation_id"""
    ).fetchall()
    assert len(cross_edges) == 3
    assert {row["target_binding_rule"] for row in cross_edges} == {"child_derived"}

    parent_id = identifier(200)
    parent_request = digest("media-parent-request")
    parent_authority = digest("media-parent-authority")
    insert_parent_registry(
        database,
        "protected_media",
        parent_id,
        "cap-v1-94a9944ce37fa085",
        parent_request,
        parent_authority,
        None,
    )
    loaded = database.execute(
        SQL["workflow_parent_read"],
        (
            "protected_media",
            parent_id,
            parent_request,
            "cap-v1-b9fcb0fbd25b2234",
        ),
    ).fetchone()
    assert loaded is not None
    assert loaded["target_binding_rule"] == "child_derived"
    assert loaded["authority_binding_digest"] == parent_authority
    child_id = identifier(201)
    child_principal = digest("loom-child-principal")
    child_replay = digest("loom-child-natural")
    child_authority = digest("loom-child-authority")
    insert_receipt(
        database,
        child_id,
        child_replay,
        request_digest=digest("loom-child-request"),
        source_operation_id="cap-v1-b9fcb0fbd25b2234",
        operation_kind="workflow",
        method="WORKFLOW",
        surface_path="workflow://apps/web/workflows/import-loom-video.ts#importLoomVideoWorkflow",
        auth_class="parent_receipt",
        authority_class="parent_receipt",
        provider_kind="loom_storage_and_media_dispatch",
        principal_digest=child_principal,
        tenant_id=ORG,
        target_id=PRIVATE_VIDEO,
        tenant_domain="none",
        target_domain="video",
        legacy_target_id=LEGACY_PRIVATE_VIDEO,
        legacy_workflow_actor_id=LEGACY_OWNER,
        workflow_raw_file_key=(
            f"{LEGACY_OWNER}/{LEGACY_PRIVATE_VIDEO}/raw-upload.webm"
        ),
        credential_kind="session_token",
        credential_subject_id=SESSION,
        credential_key_version=7,
        credential_digest=session_digest(OWNER),
        authority_binding_digest=child_authority,
        parent_family="protected_media",
        parent_receipt_id=parent_id,
        parent_request_digest=parent_request,
        parent_authority_binding_digest=parent_authority,
        replay_origin="natural",
        terminal_kind="workflow",
    )
    assert child_authority != parent_authority
    assert database.execute(
        SQL["receipt_replay"],
        (
            "cap-v1-b9fcb0fbd25b2234",
            child_principal,
            child_replay,
            child_authority,
            NOW,
        ),
    ).fetchone() is not None
    expect_integrity(
        lambda: insert_receipt(
            database,
            identifier(252),
            child_replay,
            request_digest=digest("loom-same-parent-retry-request"),
            source_operation_id="cap-v1-b9fcb0fbd25b2234",
            operation_kind="workflow",
            method="WORKFLOW",
            surface_path=(
                "workflow://apps/web/workflows/import-loom-video.ts"
                "#importLoomVideoWorkflow"
            ),
            auth_class="parent_receipt",
            authority_class="parent_receipt",
            provider_kind="loom_storage_and_media_dispatch",
            principal_digest=digest("loom-same-parent-retry-principal"),
            tenant_id=ORG,
            target_id=PRIVATE_VIDEO,
            tenant_domain="none",
            target_domain="video",
            legacy_target_id=LEGACY_PRIVATE_VIDEO,
            legacy_workflow_actor_id=LEGACY_OWNER,
            workflow_raw_file_key=(
                f"{LEGACY_OWNER}/{LEGACY_PRIVATE_VIDEO}/raw-upload.webm"
            ),
            authority_binding_digest=digest("loom-same-parent-retry-authority"),
            parent_family="protected_media",
            parent_receipt_id=parent_id,
            parent_request_digest=parent_request,
            parent_authority_binding_digest=parent_authority,
            replay_origin="natural",
            terminal_kind="workflow",
        ),
        "UNIQUE constraint failed",
    )

    try:
        insert_receipt(
            database,
            identifier(202),
            digest("loom-bad-parent-replay"),
            request_digest=digest("loom-bad-parent-request"),
            source_operation_id="cap-v1-b9fcb0fbd25b2234",
            operation_kind="workflow",
            method="WORKFLOW",
            surface_path=(
                "workflow://apps/web/workflows/import-loom-video.ts"
                "#importLoomVideoWorkflow"
            ),
            auth_class="parent_receipt",
            authority_class="parent_receipt",
            provider_kind="loom_storage_and_media_dispatch",
            principal_digest=digest("loom-bad-parent-principal"),
            tenant_id=ORG,
            target_id=PRIVATE_VIDEO,
            tenant_domain="none",
            target_domain="video",
            legacy_target_id=LEGACY_PRIVATE_VIDEO,
            legacy_workflow_actor_id=LEGACY_OWNER,
            workflow_raw_file_key=(
                f"{LEGACY_OWNER}/{LEGACY_PRIVATE_VIDEO}/raw-upload.webm"
            ),
            authority_binding_digest=digest("loom-bad-child-authority"),
            parent_family="protected_media",
            parent_receipt_id=parent_id,
            parent_request_digest=parent_request,
            parent_authority_binding_digest=OTHER_DIGEST,
            replay_origin="natural",
            terminal_kind="workflow",
        )
    except sqlite3.IntegrityError as error:
        assert "workflow_parent_invalid" in str(error)
    else:
        raise AssertionError("workflow parent authority digest mismatch was admitted")

    database.execute(
        "UPDATE auth_sessions_v2 SET state='revoked',revoked_at_ms=?2,revocation_reason='operator' WHERE id=?1",
        (SESSION, NOW),
    )
    assert database.execute(
        SQL["receipt_replay"],
        (
            "cap-v1-b9fcb0fbd25b2234",
            child_principal,
            child_replay,
            child_authority,
            NOW,
        ),
    ).fetchone() is None
    database.execute(
        "UPDATE auth_sessions_v2 SET state='active',revoked_at_ms=NULL,revocation_reason=NULL WHERE id=?1",
        (SESSION,),
    )
    database.execute(
        """UPDATE legacy_protected_effect_parent_registry_v1
           SET state='dead_letter',completed_at_ms=?2
           WHERE parent_family='protected_media' AND parent_receipt_id=?1""",
        (parent_id, NOW),
    )
    assert database.execute(
        SQL["receipt_replay"],
        (
            "cap-v1-b9fcb0fbd25b2234",
            child_principal,
            child_replay,
            child_authority,
            NOW,
        ),
    ).fetchone() is None

    retry_parent_id = identifier(253)
    retry_parent_request = digest("media-retry-parent-request")
    retry_parent_authority = digest("media-retry-parent-authority")
    insert_parent_registry(
        database,
        "protected_media",
        retry_parent_id,
        "cap-v1-94a9944ce37fa085",
        retry_parent_request,
        retry_parent_authority,
        None,
    )
    insert_receipt(
        database,
        identifier(254),
        digest("loom-child-natural-new-parent"),
        request_digest=digest("loom-child-request-new-parent"),
        source_operation_id="cap-v1-b9fcb0fbd25b2234",
        operation_kind="workflow",
        method="WORKFLOW",
        surface_path=(
            "workflow://apps/web/workflows/import-loom-video.ts"
            "#importLoomVideoWorkflow"
        ),
        auth_class="parent_receipt",
        authority_class="parent_receipt",
        provider_kind="loom_storage_and_media_dispatch",
        principal_digest=digest("loom-child-principal-new-parent"),
        tenant_id=ORG,
        target_id=PRIVATE_VIDEO,
        tenant_domain="none",
        target_domain="video",
        legacy_target_id=LEGACY_PRIVATE_VIDEO,
        legacy_workflow_actor_id=LEGACY_OWNER,
        workflow_raw_file_key=(
            f"{LEGACY_OWNER}/{LEGACY_PRIVATE_VIDEO}/raw-upload.webm"
        ),
        authority_binding_digest=digest("loom-child-authority-new-parent"),
        parent_family="protected_media",
        parent_receipt_id=retry_parent_id,
        parent_request_digest=retry_parent_request,
        parent_authority_binding_digest=retry_parent_authority,
        replay_origin="natural",
        terminal_kind="workflow",
    )

    # CSV import is launched by an organization manager but may import a
    # different active member's video. Bind the child owner alias, target org,
    # exact mp4 key, and inherited parent tuple independently.
    csv_parent_id = identifier(246)
    csv_parent_request = digest("loom-csv-parent-request")
    csv_parent_authority = digest("loom-csv-parent-authority")
    insert_receipt(
        database,
        csv_parent_id,
        digest("loom-csv-parent-replay"),
        request_digest=csv_parent_request,
        source_operation_id="cap-v1-d062d262b013a0cd",
        operation_kind="server_action",
        method="ACTION",
        surface_path="action://apps/web/actions/loom.ts#importFromLoomCsv",
        auth_class="session",
        authority_class="organization_manager",
        provider_kind="loom_csv_import",
        principal_digest=digest("loom-csv-parent-principal"),
        actor_id=ADMIN,
        credential_subject_id=session_id(ADMIN),
        credential_digest=session_digest(ADMIN),
        tenant_id=ORG,
        tenant_domain="organization",
        legacy_tenant_id=LEGACY_ORG,
        entitlement_kind="pro",
        entitlement_subject_id=ADMIN,
        entitlement_revision=0,
        authority_binding_digest=csv_parent_authority,
        terminal_kind="json",
    )

    def insert_csv_child(
        receipt_number: int,
        replay_label: str,
        target_id: str = MEMBER_VIDEO,
        legacy_target_id: str = LEGACY_MEMBER_VIDEO,
        legacy_owner_id: str = LEGACY_MEMBER,
        raw_file_key: str | None = None,
    ) -> None:
        insert_receipt(
            database,
            identifier(receipt_number),
            digest(replay_label),
            request_digest=digest(f"{replay_label}-request"),
            source_operation_id="cap-v1-b9fcb0fbd25b2234",
            operation_kind="workflow",
            method="WORKFLOW",
            surface_path=(
                "workflow://apps/web/workflows/import-loom-video.ts"
                "#importLoomVideoWorkflow"
            ),
            auth_class="parent_receipt",
            authority_class="parent_receipt",
            provider_kind="loom_storage_and_media_dispatch",
            principal_digest=digest(f"{replay_label}-principal"),
            actor_id=ADMIN,
            credential_subject_id=session_id(ADMIN),
            credential_digest=session_digest(ADMIN),
            tenant_id=ORG,
            target_id=target_id,
            tenant_domain="none",
            target_domain="video",
            legacy_target_id=legacy_target_id,
            legacy_workflow_actor_id=legacy_owner_id,
            workflow_raw_file_key=(
                raw_file_key
                if raw_file_key is not None
                else f"{legacy_owner_id}/{legacy_target_id}/raw-upload.mp4"
            ),
            entitlement_kind="pro",
            entitlement_subject_id=ADMIN,
            entitlement_revision=0,
            authority_binding_digest=digest(f"{replay_label}-authority"),
            parent_family="protected_integrations",
            parent_receipt_id=csv_parent_id,
            parent_request_digest=csv_parent_request,
            parent_authority_binding_digest=csv_parent_authority,
            replay_origin="natural",
            terminal_kind="workflow",
        )

    insert_csv_child(247, "loom-csv-member-child")
    expect_integrity(
        lambda: insert_csv_child(
            248, "loom-csv-wrong-owner", legacy_owner_id=LEGACY_OWNER
        ),
        "authority_stale",
    )
    expect_integrity(
        lambda: insert_csv_child(
            249,
            "loom-csv-wrong-raw",
            raw_file_key=f"{LEGACY_MEMBER}/{LEGACY_MEMBER_VIDEO}/raw-upload.mov",
        ),
        "authority_stale",
    )
    expect_integrity(
        lambda: insert_csv_child(
            250,
            "loom-csv-cross-org",
            target_id=OTHER_VIDEO,
            legacy_target_id=LEGACY_OTHER_VIDEO,
            legacy_owner_id=LEGACY_OUTSIDER,
        ),
        "authority_stale",
    )
    database.execute(
        "UPDATE organization_members SET state='removed' WHERE organization_id=?1 AND user_id=?2",
        (ORG, MEMBER),
    )
    expect_integrity(
        lambda: insert_csv_child(251, "loom-csv-removed-member"),
        "authority_stale",
    )
    database.execute(
        "UPDATE organization_members SET state='active' WHERE organization_id=?1 AND user_id=?2",
        (ORG, MEMBER),
    )

    same_parent_id = identifier(245)
    same_parent_request = digest("same-parent-request")
    same_parent_authority = digest("same-parent-authority")
    f0_authority = authority(
        database,
        "organization_member",
        actor_id=ADMIN,
        authenticated_tenant_id=ORG,
        legacy_tenant_id=LEGACY_ORG,
        legacy_target_id=LOOM_VIDEO,
        tenant_domain="organization",
        target_domain="external",
        entitlement="cap_internal",
        operation_id="cap-v1-f0a00e93ab606a52",
        legacy_workflow_actor_id=LEGACY_ADMIN,
        legacy_workflow_cap_tenant_id=LEGACY_ORG,
    )
    assert f0_authority["authorized"] == 1
    assert authority(
        database,
        "organization_member",
        actor_id=ADMIN,
        authenticated_tenant_id=ORG,
        legacy_tenant_id=LEGACY_ORG,
        legacy_target_id=LOOM_VIDEO,
        tenant_domain="organization",
        target_domain="external",
        entitlement="cap_internal",
        operation_id="cap-v1-f0a00e93ab606a52",
        legacy_workflow_actor_id=LEGACY_OWNER,
        legacy_workflow_cap_tenant_id=LEGACY_ORG,
    )["authorized"] == 0
    assert authority(
        database,
        "organization_member",
        actor_id=ADMIN,
        authenticated_tenant_id=ORG,
        legacy_tenant_id=LEGACY_ORG,
        legacy_target_id=LOOM_VIDEO,
        tenant_domain="organization",
        target_domain="external",
        entitlement="cap_internal",
        operation_id="cap-v1-f0a00e93ab606a52",
        legacy_workflow_actor_id=LEGACY_ADMIN,
        legacy_workflow_cap_tenant_id=LEGACY_OTHER_ORG,
    )["authorized"] == 0
    insert_receipt(
        database,
        same_parent_id,
        digest("loom-http-parent-replay"),
        request_digest=same_parent_request,
        source_operation_id="cap-v1-f0a00e93ab606a52",
        method="POST",
        surface_path="/api/loom/video",
        auth_class="session_or_api_key",
        authority_class="organization_member",
        provider_kind="loom_import",
        principal_digest=digest("loom-http-parent-principal"),
        actor_id=ADMIN,
        credential_subject_id=session_id(ADMIN),
        credential_digest=session_digest(ADMIN),
        tenant_id=ORG,
        target_id=LOOM_VIDEO,
        tenant_domain="organization",
        target_domain="external",
        legacy_tenant_id=LEGACY_ORG,
        legacy_target_id=LOOM_VIDEO,
        legacy_workflow_actor_id=LEGACY_ADMIN,
        legacy_workflow_cap_tenant_id=LEGACY_ORG,
        entitlement_kind="cap_internal",
        entitlement_subject_id=ADMIN,
        entitlement_revision=0,
        authority_binding_digest=same_parent_authority,
    )
    same_loaded = database.execute(
        SQL["workflow_parent_read"],
        (
            "protected_integrations",
            same_parent_id,
            same_parent_request,
            "cap-v1-bd1b9d67380624f7",
        ),
    ).fetchone()
    assert same_loaded is not None and same_loaded["target_binding_rule"] == "same"
    insert_receipt(
        database,
        identifier(211),
        digest("same-child-replay"),
        request_digest=digest("same-child-request"),
        source_operation_id="cap-v1-bd1b9d67380624f7",
        operation_kind="workflow",
        method="WORKFLOW",
        surface_path="workflow://packages/web-domain/src/Loom.ts#LoomImportVideo",
        auth_class="parent_receipt",
        authority_class="parent_receipt",
        provider_kind="loom_import",
        principal_digest=digest("same-child-principal"),
        actor_id=ADMIN,
        credential_subject_id=session_id(ADMIN),
        credential_digest=session_digest(ADMIN),
        tenant_id=ORG,
        target_id=LOOM_VIDEO,
        tenant_domain="organization",
        target_domain="external",
        legacy_tenant_id=LEGACY_ORG,
        legacy_target_id=LOOM_VIDEO,
        legacy_workflow_actor_id=LEGACY_ADMIN,
        legacy_workflow_cap_tenant_id=LEGACY_ORG,
        entitlement_kind="cap_internal",
        entitlement_subject_id=ADMIN,
        entitlement_revision=0,
        authority_binding_digest=digest("same-child-authority"),
        parent_family="protected_integrations",
        parent_receipt_id=same_parent_id,
        parent_request_digest=same_parent_request,
        parent_authority_binding_digest=same_parent_authority,
        replay_origin="natural",
        terminal_kind="workflow",
    )

    def bd1_authority(
        workflow_actor_id: str = LEGACY_ADMIN,
        cap_tenant_id: str = LEGACY_ORG,
        loom_tenant_id: str = LEGACY_ORG,
    ) -> sqlite3.Row:
        return authority(
            database,
            "parent_receipt",
            actor_id=ADMIN,
            authenticated_tenant_id=ORG,
            legacy_tenant_id=loom_tenant_id,
            legacy_target_id=LOOM_VIDEO,
            tenant_domain="organization",
            target_domain="external",
            entitlement="cap_internal",
            operation_id="cap-v1-bd1b9d67380624f7",
            legacy_workflow_actor_id=workflow_actor_id,
            legacy_workflow_cap_tenant_id=cap_tenant_id,
            parent_family="protected_integrations",
            parent_receipt_id=same_parent_id,
            parent_request_digest=same_parent_request,
            parent_authority_binding_digest=same_parent_authority,
        )

    assert bd1_authority()["authorized"] == 1
    assert bd1_authority(workflow_actor_id=LEGACY_OWNER)["authorized"] == 0
    assert bd1_authority(cap_tenant_id=LEGACY_OTHER_ORG)["authorized"] == 0
    assert bd1_authority(loom_tenant_id=LEGACY_OTHER_ORG)["authorized"] == 0
    try:
        insert_receipt(
            database,
            identifier(212),
            digest("wrong-target-replay"),
            request_digest=digest("wrong-target-request"),
            source_operation_id="cap-v1-bd1b9d67380624f7",
            operation_kind="workflow",
            method="WORKFLOW",
            surface_path="workflow://packages/web-domain/src/Loom.ts#LoomImportVideo",
            auth_class="parent_receipt",
            authority_class="parent_receipt",
            provider_kind="loom_import",
            principal_digest=digest("wrong-target-principal"),
            actor_id=ADMIN,
            credential_subject_id=session_id(ADMIN),
            credential_digest=session_digest(ADMIN),
            tenant_id=ORG,
            target_id="other-loom-video",
            tenant_domain="organization",
            target_domain="external",
            legacy_tenant_id=LEGACY_ORG,
            legacy_target_id="other-loom-video",
            legacy_workflow_actor_id=LEGACY_ADMIN,
            legacy_workflow_cap_tenant_id=LEGACY_ORG,
            entitlement_kind="cap_internal",
            entitlement_subject_id=ADMIN,
            entitlement_revision=0,
            authority_binding_digest=digest("wrong-target-authority"),
            parent_family="protected_integrations",
            parent_receipt_id=same_parent_id,
            parent_request_digest=same_parent_request,
            parent_authority_binding_digest=same_parent_authority,
            replay_origin="natural",
            terminal_kind="workflow",
        )
    except sqlite3.IntegrityError as error:
        assert "workflow_parent_invalid" in str(error)
    else:
        raise AssertionError("same-target parent edge accepted a different target")


def expect_integrity(action, token: str | None = None) -> None:
    try:
        action()
    except sqlite3.IntegrityError as error:
        if token is not None:
            assert token in str(error), (token, str(error))
    else:
        raise AssertionError(f"expected SQLite integrity failure: {token or 'constraint'}")


def insert_lease(
    database: sqlite3.Connection,
    lease_id: str,
    receipt_id: str,
    executor_id: str,
    request_digest: str,
    outbox_digest: str,
    authority_digest: str,
    leased_at_ms: int,
    expires_at_ms: int,
) -> None:
    database.execute(
        """INSERT INTO legacy_protected_integration_executor_leases_v1(
             lease_id,receipt_id,executor_id,request_digest,outbox_payload_digest,
             authority_binding_digest,leased_at_ms,lease_expires_at_ms,state
           ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'active')""",
        (
            lease_id,
            receipt_id,
            executor_id,
            request_digest,
            outbox_digest,
            authority_digest,
            leased_at_ms,
            expires_at_ms,
        ),
    )


def insert_evidence(
    database: sqlite3.Connection,
    receipt_id: str,
    lease_id: str,
    executor_id: str,
    request_digest: str,
    outbox_digest: str,
    authority_digest: str,
    terminal_kind: str = "http",
    terminal_ref: str | None = None,
    terminal_digest: str | None = None,
    verified_at_ms: int = NOW + 20,
    terminal_expires_at_ms: int = NOW + 600_020,
) -> None:
    database.execute(
        """INSERT INTO legacy_protected_integration_evidence_v1(
             receipt_id,lease_id,executor_id,request_digest,outbox_payload_digest,
             authority_binding_digest,provider_evidence_digest,terminal_kind,
             sealed_terminal_ref,sealed_terminal_digest,terminal_expires_at_ms,
             verified_at_ms
           ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)""",
        (
            receipt_id,
            lease_id,
            executor_id,
            request_digest,
            outbox_digest,
            authority_digest,
            digest("provider-evidence"),
            terminal_kind,
            terminal_ref or f"frame-pi-terminal-v1:{digest('terminal-ref')}",
            terminal_digest or digest("terminal-plaintext"),
            terminal_expires_at_ms,
            verified_at_ms,
        ),
    )


def prove_executor_lease_and_evidence(database: sqlite3.Connection) -> None:
    receipt_id = identifier(100)
    authority_digest = digest("authority-binding")
    outbox_digest = database.execute(
        """SELECT payload_digest FROM legacy_protected_integration_outbox_v1
           WHERE receipt_id=?1""",
        (receipt_id,),
    ).fetchone()[0]
    evidence_columns = {
        row["name"]
        for row in database.execute(
            "PRAGMA table_info(legacy_protected_integration_evidence_v1)"
        )
    }
    assert "response_json" not in evidence_columns
    assert {
        "lease_id",
        "executor_id",
        "request_digest",
        "outbox_payload_digest",
        "authority_binding_digest",
        "terminal_kind",
        "sealed_terminal_ref",
        "sealed_terminal_digest",
        "terminal_expires_at_ms",
    } <= evidence_columns

    database.execute(
        """INSERT INTO legacy_protected_integration_executors_v1(
             executor_id,provider_kind,identity_digest,state
           ) VALUES('executor.s3.v1','sealed_s3_configuration',?1,'active')""",
        (digest("executor-identity"),),
    )
    database.execute(
        """INSERT INTO legacy_protected_integration_executors_v1(
             executor_id,provider_kind,identity_digest,state
           ) VALUES('executor.wrong.v1','wrong_provider',?1,'active')""",
        (digest("wrong-executor-identity"),),
    )
    expect_integrity(
        lambda: insert_lease(
            database,
            identifier(220),
            receipt_id,
            "executor.wrong.v1",
            DIGEST,
            outbox_digest,
            authority_digest,
            NOW + 10,
            NOW + 100,
        ),
        "lease_invalid",
    )
    expect_integrity(
        lambda: insert_lease(
            database,
            identifier(221),
            receipt_id,
            "executor.s3.v1",
            OTHER_DIGEST,
            outbox_digest,
            authority_digest,
            NOW + 10,
            NOW + 100,
        ),
        "lease_invalid",
    )
    expect_integrity(
        lambda: database.execute(
            """UPDATE legacy_protected_integration_outbox_v1
               SET state='verified',completed_at_ms=?2 WHERE receipt_id=?1""",
            (receipt_id, NOW + 20),
        ),
        "evidence_required",
    )
    expect_integrity(
        lambda: database.execute(
            """UPDATE legacy_protected_integration_receipts_v1
               SET state='verified',completed_at_ms=?2 WHERE receipt_id=?1""",
            (receipt_id, NOW + 20),
        ),
        "evidence_required",
    )

    lease_id = identifier(222)
    insert_lease(
        database,
        lease_id,
        receipt_id,
        "executor.s3.v1",
        DIGEST,
        outbox_digest,
        authority_digest,
        NOW + 10,
        NOW + 100,
    )
    expect_integrity(
        lambda: insert_lease(
            database,
            identifier(223),
            receipt_id,
            "executor.s3.v1",
            DIGEST,
            outbox_digest,
            authority_digest,
            NOW + 11,
            NOW + 101,
        )
    )
    for mismatch in (
        {"request_digest": OTHER_DIGEST},
        {"outbox_digest": OTHER_DIGEST},
        {"authority_digest": OTHER_DIGEST},
        {"terminal_kind": "json"},
        {"verified_at_ms": NOW + 100},
    ):
        arguments = {
            "request_digest": DIGEST,
            "outbox_digest": outbox_digest,
            "authority_digest": authority_digest,
            "terminal_kind": "http",
            "verified_at_ms": NOW + 20,
        }
        arguments.update(mismatch)
        expect_integrity(
            lambda arguments=arguments: insert_evidence(
                database,
                receipt_id,
                lease_id,
                "executor.s3.v1",
                arguments["request_digest"],
                arguments["outbox_digest"],
                arguments["authority_digest"],
                terminal_kind=arguments["terminal_kind"],
                verified_at_ms=arguments["verified_at_ms"],
            ),
            "evidence_invalid",
        )
    expect_integrity(
        lambda: insert_evidence(
            database,
            receipt_id,
            lease_id,
            "executor.s3.v1",
            DIGEST,
            outbox_digest,
            authority_digest,
            terminal_ref="https://provider.example/signed?secret=plaintext",
        )
    )

    database.execute(
        """UPDATE auth_sessions_v2
           SET state='revoked',revoked_at_ms=?2,revocation_reason='operator'
           WHERE id=?1""",
        (SESSION, NOW + 15),
    )
    expect_integrity(
        lambda: insert_evidence(
            database,
            receipt_id,
            lease_id,
            "executor.s3.v1",
            DIGEST,
            outbox_digest,
            authority_digest,
        ),
        "evidence_invalid",
    )
    database.execute(
        """UPDATE auth_sessions_v2
           SET state='active',revoked_at_ms=NULL,revocation_reason=NULL
           WHERE id=?1""",
        (SESSION,),
    )
    insert_evidence(
        database,
        receipt_id,
        lease_id,
        "executor.s3.v1",
        DIGEST,
        outbox_digest,
        authority_digest,
    )
    receipt = database.execute(
        "SELECT state,completed_at_ms FROM legacy_protected_integration_receipts_v1 WHERE receipt_id=?1",
        (receipt_id,),
    ).fetchone()
    outbox = database.execute(
        "SELECT state,completed_at_ms FROM legacy_protected_integration_outbox_v1 WHERE receipt_id=?1",
        (receipt_id,),
    ).fetchone()
    lease = database.execute(
        "SELECT state FROM legacy_protected_integration_executor_leases_v1 WHERE lease_id=?1",
        (lease_id,),
    ).fetchone()
    assert tuple(receipt) == ("verified", NOW + 20)
    assert tuple(outbox) == ("verified", NOW + 20)
    assert lease["state"] == "consumed"
    registry = database.execute(
        """SELECT state,completed_at_ms FROM legacy_protected_effect_parent_registry_v1
           WHERE parent_family='protected_integrations' AND parent_receipt_id=?1""",
        (receipt_id,),
    ).fetchone()
    assert tuple(registry) == ("verified", NOW + 20)

    replay = database.execute(
        SQL["generated_receipt_replay"],
        (
            "cap-v1-9d91d42d52472a83",
            digest("principal"),
            DIGEST,
            authority_digest,
            NOW + 21,
            NOW - 900_000,
        ),
    ).fetchone()
    assert replay is not None and replay["state"] == "verified"
    assert replay["sealed_terminal_ref"].startswith("frame-pi-terminal-v1:")
    assert database.execute(
        SQL["generated_receipt_replay"],
        (
            "cap-v1-9d91d42d52472a83",
            digest("principal"),
            DIGEST,
            authority_digest,
            NOW + 600_020,
            NOW - 900_000,
        ),
    ).fetchone() is None

    for statement, token in (
        (
            "DELETE FROM legacy_protected_integration_receipts_v1 WHERE receipt_id=?1",
            "receipt_immutable",
        ),
        (
            "DELETE FROM legacy_protected_integration_outbox_v1 WHERE receipt_id=?1",
            "outbox_immutable",
        ),
        (
            "DELETE FROM legacy_protected_integration_executor_leases_v1 WHERE lease_id=?1",
            "lease_immutable",
        ),
        (
            "DELETE FROM legacy_protected_integration_evidence_v1 WHERE receipt_id=?1",
            "evidence_immutable",
        ),
    ):
        key = lease_id if "lease_id" in statement else receipt_id
        expect_integrity(lambda statement=statement, key=key: database.execute(statement, (key,)), token)

    replacement_time = NOW + 900_021
    replacement_id = identifier(224)
    insert_receipt(
        database,
        replacement_id,
        digest("rollover-replay"),
        created_at_ms=replacement_time,
    )
    database.execute(
        SQL["generated_claim_upsert"],
        (
            "cap-v1-9d91d42d52472a83",
            digest("principal"),
            DIGEST,
            replacement_id,
            replacement_time,
            replacement_time - 900_000,
        ),
    )
    replacement = database.execute(
        SQL["generated_receipt_replay"],
        (
            "cap-v1-9d91d42d52472a83",
            digest("principal"),
            DIGEST,
            authority_digest,
            replacement_time,
            replacement_time - 900_000,
        ),
    ).fetchone()
    assert replacement is not None
    assert replacement["receipt_id"] == replacement_id
    assert replacement["state"] == "pending_provider_evidence"

    state_receipt = identifier(110)
    state_outbox_digest = insert_exact_outbox(database, state_receipt)
    database.execute(
        """INSERT INTO legacy_protected_integration_executors_v1(
             executor_id,provider_kind,identity_digest,state
           ) VALUES('executor.google.v1','google_drive_oauth_exchange',?1,'active')""",
        (digest("google-executor-identity"),),
    )
    state_lease = identifier(225)
    insert_lease(
        database,
        state_lease,
        state_receipt,
        "executor.google.v1",
        digest("signed-state-request"),
        state_outbox_digest,
        digest("signed-state-authority"),
        NOW + 50,
        NOW + 200,
    )
    expect_integrity(
        lambda: insert_evidence(
            database,
            state_receipt,
            state_lease,
            "executor.google.v1",
            digest("signed-state-request"),
            state_outbox_digest,
            digest("signed-state-authority"),
            verified_at_ms=NOW + 101,
        ),
        "evidence_invalid",
    )
    assert database.execute(
        """SELECT 1 FROM legacy_protected_integration_evidence_v1
           WHERE receipt_id=?1""",
        (state_receipt,),
    ).fetchone() is None


def prove_secret_exclusion(database: sqlite3.Connection) -> None:
    secret = HOSTILE_SECRETS[0]
    bad_receipt = identifier(230)
    descriptor = {
        "schema_version": "frame.legacy-protected-integration-request.v1",
        "source_operation_id": "cap-v1-9d91d42d52472a83",
        "payload": {
            "digest_only": True,
            "sha256": digest(secret),
            "plaintext": secret,
        },
        "sealed_request_digest": digest("provider-request"),
        "transport_body_digest": None,
        "parent_family": None,
        "parent_receipt_id": None,
        "parent_request_digest": None,
        "parent_authority_binding_digest": None,
    }
    expect_integrity(
        lambda: insert_receipt(
            database,
            bad_receipt,
            digest("hostile-replay"),
            request_digest=digest("hostile-request"),
            principal_digest=digest("hostile-principal"),
            redacted_request_json=json.dumps(descriptor, separators=(",", ":")),
        ),
        "request_not_redacted",
    )

    tables = [
        row[0]
        for row in database.execute(
            """SELECT name FROM sqlite_master
               WHERE type='table' AND name NOT LIKE 'sqlite_%'"""
        )
    ]
    for table in tables:
        columns = [
            row["name"]
            for row in database.execute(f'PRAGMA table_info("{table}")')
            if "TEXT" in row["type"].upper()
        ]
        for column in columns:
            values = database.execute(
                f'SELECT "{column}" FROM "{table}" WHERE "{column}" IS NOT NULL'
            )
            for row in values:
                text = str(row[0])
                for hostile in HOSTILE_SECRETS:
                    assert hostile not in text, (table, column, hostile)


def main() -> int:
    document = fixture()
    prove_inventory(document)
    database = migrated()
    seed(database)
    prove_authority(database)
    prove_credential_revocation(database)
    prove_entitlement_loss(database)
    prove_alias_and_policy_revocation(database)
    prove_generated_and_natural_replay(database)
    prove_conditional_authority(database)
    prove_workflow_parent_registry(database)
    prove_executor_lease_and_evidence(database)
    prove_secret_exclusion(database)
    assert not database.execute("PRAGMA foreign_key_check").fetchall()
    print(
        "legacy protected integrations SQLite conformance passed "
        "(45 source-pinned operations; exact D1 authority, aliases, conditional "
        "business rules, replay, cross-family parents, executor leases, sealed "
        "evidence, secret exclusion, and immutability)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
