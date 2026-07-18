#!/usr/bin/env python3
"""Prove fail-closed local contracts for protected auth/billing operations."""

from __future__ import annotations

import hashlib
import json
import re
import sqlite3
import sys
from pathlib import Path, PurePosixPath


ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures/api-parity/v1/protected-billing-auth.json"
MIGRATION = ROOT / "apps/control-plane/migrations/0063_legacy_protected_billing_auth_expand.sql"
APPLICATION = ROOT / "crates/application/src/legacy_protected_billing_auth.rs"
RUNTIME = ROOT / "apps/control-plane/src/legacy_protected_billing_auth_runtime.rs"
WEB = ROOT / "apps/control-plane/src/legacy_protected_billing_auth_web_runtime.rs"
QUERIES = ROOT / "apps/control-plane/queries/legacy_protected_billing_auth"
AUTH_QUERIES = ROOT / "apps/control-plane/queries/auth"
USER_SESSION_ID = "10000000-0000-4000-8000-000000000001"
ADMIN_SESSION_ID = "10000000-0000-4000-8000-000000000002"
USER_SESSION_DIGEST = hashlib.sha256(b"user-session-token").hexdigest()
ADMIN_SESSION_DIGEST = hashlib.sha256(b"admin-session-token").hexdigest()


def digest(value: str | bytes) -> str:
    if isinstance(value, str):
        value = value.encode()
    return hashlib.sha256(value).hexdigest()


def expect_integrity(connection: sqlite3.Connection, sql: str, marker: str) -> None:
    try:
        connection.execute(sql)
    except sqlite3.IntegrityError as error:
        if marker not in str(error):
            raise AssertionError(f"unexpected integrity error: {error}") from error
    else:
        raise AssertionError(f"expected integrity failure containing {marker!r}")


def redacted_request(
    operation_id: str,
    payload: dict,
    transport_body_digest: str | None = None,
    sealed_request_ref: str | None = None,
    sealed_request_digest: str | None = None,
) -> str:
    provider_required = operation_id != "cap-v1-572763e7b4977abd"
    human_required = operation_id not in {
        "cap-v1-46bda1c18ffba076",
        "cap-v1-82a39c991fae1050",
        "cap-v1-572763e7b4977abd",
    }
    request = {
        "schema_version": "frame.legacy-protected-billing-auth-request.v1",
        "source_operation_id": operation_id,
        "payload": payload,
        "transport_body_digest": transport_body_digest,
        "sealed_request_digest": sealed_request_digest,
        "required_evidence": {
            "human_approval": human_required,
            "provider_execution": provider_required,
        },
    }
    if sealed_request_ref is not None:
        request["sealed_request_ref"] = sealed_request_ref
    return json.dumps(request, separators=(",", ":"), sort_keys=True)


def canonical_request_digest(request_json: str) -> str:
    request = json.loads(request_json)
    request.pop("sealed_request_ref", None)
    return digest(json.dumps(request, separators=(",", ":"), sort_keys=True))


def insert_stage(
    connection: sqlite3.Connection,
    *,
    receipt_id: str,
    operation_id: str,
    replay_key: str,
    request_json: str,
    human_required: bool,
    actor_id: str | None = "user-1",
    authority_class: str | None = None,
    target_id: str | None = None,
    credential_digest: str | None = None,
    credential_kind: str | None = None,
    credential_subject_id: str | None = None,
    credential_key_version: int | None = None,
    auth_class: str | None = None,
    operation_kind: str = "route",
    method: str = "POST",
    provider_kind: str = "stripe_checkout",
    replay_origin: str = "caller",
    idempotency_mode: str = "required",
    created_at_ms: int = 1000,
    request_digest_override: str | None = None,
) -> None:
    request_digest = request_digest_override or digest(request_json)
    request_object = json.loads(request_json)
    resolved_auth_class = auth_class or (
        "admin_session" if actor_id else "public_or_flow_token"
    )
    if credential_kind is None:
        if resolved_auth_class == "signed_webhook":
            credential_kind = "signed_endpoint"
            credential_subject_id = "stripe-webhook.endpoint.v1"
            credential_digest = credential_digest or digest("stripe-webhook-provider-v1")
        elif actor_id is None:
            credential_kind = "public_flow"
            credential_digest = credential_digest or digest("public-flow-principal")
        elif resolved_auth_class == "session_or_api_key" and credential_digest is not None:
            credential_kind = "api_key"
            credential_subject_id = "key-1"
            credential_key_version = None
        else:
            credential_kind = "session_token"
            credential_subject_id = (
                ADMIN_SESSION_ID if actor_id == "admin-1" else USER_SESSION_ID
            )
            credential_key_version = 1
            credential_digest = (
                ADMIN_SESSION_DIGEST if actor_id == "admin-1" else USER_SESSION_DIGEST
            )
    payload = json.dumps(
        {
            "schema_version": "frame.legacy-protected-billing-auth-outbox.v1",
            "receipt_id": receipt_id,
            "source_operation_id": operation_id,
            "request_digest": request_digest,
            "redacted_request": json.loads(request_json),
            "required_evidence": {
                "human_approval": human_required,
                "provider_execution": True,
            },
        },
        separators=(",", ":"),
        sort_keys=True,
    )
    connection.execute(
        """
        INSERT INTO legacy_protected_billing_auth_receipts_v1 (
          receipt_id,source_operation_id,operation_kind,method,surface_path,
          auth_class,authority_class,provider_kind,human_approval_required,
          provider_execution_required,principal_digest,actor_id,
          credential_kind,credential_subject_id,credential_key_version,credential_digest,
          sealed_request_ref,sealed_request_digest,target_id,
          replay_key_digest,replay_origin,idempotency_mode,request_digest,
          redacted_request_json,state,created_at_ms,completed_at_ms
        ) VALUES (
          ?,?,?,?,'/test',
          ?,?,?,?,
          1,?,?,?,?,?,?,?,?,?,
          ?,?,?,?,
          ?,?,?,NULL
        )
        """,
        (
            receipt_id,
            operation_id,
            operation_kind,
            method,
            resolved_auth_class,
            authority_class or ("active_session" if actor_id else "public_flow"),
            provider_kind,
            int(human_required),
            digest("principal"),
            actor_id,
            credential_kind,
            credential_subject_id,
            credential_key_version,
            credential_digest,
            request_object.get("sealed_request_ref"),
            request_object.get("sealed_request_digest"),
            target_id,
            digest(replay_key),
            replay_origin,
            idempotency_mode,
            request_digest,
            request_json,
            "awaiting_human_approval"
            if human_required
            else "awaiting_provider_evidence",
            created_at_ms,
        ),
    )
    connection.execute(
        """
        INSERT INTO legacy_protected_billing_auth_outbox_v1 (
          receipt_id,provider_kind,payload_json,payload_digest,state,
          attempt_count,created_at_ms,completed_at_ms
        ) VALUES (?,?,?, ?,?,0,?,NULL)
        """,
        (
            receipt_id,
            provider_kind,
            payload,
            digest(payload),
            "blocked_human_approval"
            if human_required
            else "pending_provider_evidence",
            created_at_ms,
        ),
    )
    if human_required:
        connection.execute(
            """
            INSERT INTO legacy_protected_billing_auth_approval_requests_v1(
              receipt_id,approval_scope,request_digest,state,created_at_ms,resolved_at_ms
            ) VALUES(?,'billing_admin.v1',?,'pending',?,NULL)
            """,
            (receipt_id, request_digest, created_at_ms),
        )


def admit_human_approval(
    connection: sqlite3.Connection,
    receipt_id: str,
    resolved_at_ms: int,
) -> None:
    request_digest = connection.execute(
        "SELECT request_digest FROM legacy_protected_billing_auth_receipts_v1 WHERE receipt_id=?",
        (receipt_id,),
    ).fetchone()[0]
    connection.execute(
        """
        INSERT INTO legacy_protected_billing_auth_human_evidence_v1(
          receipt_id,request_digest,decision,approver_subject_digest,
          approval_evidence_digest,change_ticket,verifier_class,verified_at_ms
        ) VALUES(?,?,'approved',?,?,?,'independent_human_approver',?)
        """,
        (
            receipt_id,
            request_digest,
            digest(f"approver:{receipt_id}"),
            digest(f"approval:{receipt_id}"),
            f"FRAME-{receipt_id[-4:]}",
            resolved_at_ms,
        ),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_approval_requests_v1 SET state='approved',resolved_at_ms=? WHERE receipt_id=?",
        (resolved_at_ms, receipt_id),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_outbox_v1 SET state='pending_provider_evidence' WHERE receipt_id=?",
        (receipt_id,),
    )


def insert_sealed_provider_evidence(
    connection: sqlite3.Connection,
    receipt_id: str,
    verified_at_ms: int,
) -> None:
    request_digest = connection.execute(
        "SELECT request_digest FROM legacy_protected_billing_auth_receipts_v1 WHERE receipt_id=?",
        (receipt_id,),
    ).fetchone()[0]
    connection.execute(
        """
        INSERT INTO legacy_protected_billing_auth_provider_evidence_v1(
          receipt_id,request_digest,provider_evidence_digest,sealed_response_ref,
          sealed_response_digest,verifier_class,verified_at_ms
        ) VALUES(?,?,?,?,?,'independent_provider_executor',?)
        """,
        (
            receipt_id,
            request_digest,
            digest(f"provider-evidence:{receipt_id}"),
            f"frame-pba-http-v1:{digest(f'sealed-response:{receipt_id}')}",
            digest(f"typed-http-envelope:{receipt_id}"),
            verified_at_ms,
        ),
    )


def expect_execution_authority_stale(
    connection: sqlite3.Connection,
    receipt_id: str,
    verified_at_ms: int,
) -> None:
    try:
        insert_sealed_provider_evidence(connection, receipt_id, verified_at_ms)
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_execution_authority_stale_v1" in str(error)
    else:
        raise AssertionError(f"stale execution authority admitted evidence for {receipt_id}")


def validate_fixture() -> dict:
    fixture = json.loads(FIXTURE.read_text())
    operations = fixture["operations"]
    assert fixture["reference"]["commit"] == "6ba69561ac86b8efdb17616d6727f9638015546b"
    assert fixture["summary"] == {
        "operation_count": 17,
        "human_and_provider": 14,
        "provider_only": 2,
        "local_exact": 1,
        "local_terminal_behavior": "sixteen_fail_closed_plus_credentialed_cors_preflight",
    }
    assert len(operations) == 17
    assert len({operation["id"] for operation in operations}) == 17
    assert sum(operation["protected_gates"] == ["provider_execution"] for operation in operations) == 2
    assert sum(operation["protected_gates"] == ["human_approval", "provider_execution"] for operation in operations) == 14
    assert sum(operation["protected_gates"] == [] for operation in operations) == 1
    assert sum(operation["rate_limit_bucket"] == "auth_session.v1" for operation in operations) == 3
    assert sum(operation["rate_limit_bucket"] == "billing_admin.v1" for operation in operations) == 13
    assert sum(operation["rate_limit_bucket"] == "stripe_webhook_ingress.v1" for operation in operations) == 1
    preflight = next(
        operation
        for operation in operations
        if operation["id"] == "cap-v1-572763e7b4977abd"
    )
    assert preflight["auth"] == "anonymous"
    assert preflight["authority"] == "public_flow"
    assert preflight["provider"] == "local_credentialed_cors_preflight"
    assert len(preflight["source_manifest"]) == 2
    assert {operation["method"] for operation in operations} <= {
        "GET",
        "POST",
        "OPTIONS",
        "ACTION",
        "WORKFLOW",
    }
    application = APPLICATION.read_text()
    for operation in operations:
        assert operation["id"] in application
        assert operation["idempotency"] in {"required", "optional", "forbidden"}
        for source in operation["source_manifest"]:
            source_path = PurePosixPath(source["path"])
            assert source_path.parts and not source_path.is_absolute()
            assert ".." not in source_path.parts
            assert source["symbol"]
            assert re.fullmatch(r"[0-9a-f]{64}", source["sha256"])
    assert "checkout_success" in fixture["local_contract"]["never_implies"]
    assert "video_reprocessing" in fixture["local_contract"]["never_implies"]
    return fixture


def validate_source_contracts() -> None:
    application = APPLICATION.read_text()
    runtime = RUNTIME.read_text()
    web = WEB.read_text()
    assert "LEGACY_PROTECTED_BILLING_AUTH_OPERATION_COUNT: usize = 17" in application
    assert "canonical_stripe_event" in application
    assert "redact_value" in application
    assert "batch(statements)" in runtime
    assert "stage_with_browser_proof" in runtime
    assert "grant_assertion_statement" in runtime
    assert "receipt_replay_assert.sql" in runtime
    assert "stage_workflow_from_parent" in runtime
    assert "workflow_parent_read.sql" in runtime
    assert "transport_credential_digest" in application
    assert "DELIVERY_AUDIT_INSERT_SQL" in runtime
    assert "APPROVAL_REQUEST_INSERT_SQL" in runtime
    assert "EvidenceRequired" in runtime
    assert 'profile.operation_id == "cap-v1-572763e7b4977abd"' in runtime
    assert "PROTECTED_EXECUTION_EVIDENCE_REQUIRED" in web
    assert "STRIPE_WEBHOOK_SECRET" in web
    assert "Hmac::<Sha256>::new_from_slice" in web
    assert "verify_slice" in web
    assert "STRIPE_SIGNATURE_TOLERANCE_SECONDS" in web
    assert "idempotency-key" in web
    assert "CompatibilityRateLimitBucketV1::AuthSession" in web
    assert "CompatibilityRateLimitBucketV1::BillingAdmin" in web
    assert "CompatibilityRateLimitBucketV1::StripeWebhookIngress" in web
    assert "local_credentialed_cors_preflight" in application
    assert "developer_checkout_origin_allowed" in web
    assert "access-control-allow-credentials" in web
    assert "verified-stripe-webhook.endpoint.v1" in web
    assert "ProtectedRequestVaultV1" in web
    assert "ProtectedTerminalHttpResponseResolverV1" in web
    assert "response_json" not in MIGRATION.read_text()
    assert "server_action_response" in web
    assert "workflow_response" in web
    for query in (
        "authority_read.sql",
        "receipt_replay.sql",
        "receipt_replay_assert.sql",
        "receipt_insert.sql",
        "outbox_insert.sql",
        "approval_request_insert.sql",
        "workflow_parent_read.sql",
        "delivery_audit_insert.sql",
        "generated_claim_upsert.sql",
        "generated_receipt_replay.sql",
    ):
        assert (QUERIES / query).is_file(), query


def validate_database() -> None:
    connection = sqlite3.connect(":memory:")
    connection.execute("PRAGMA foreign_keys = ON")
    connection.executescript(MIGRATION.read_text())

    # Exercise the checked authority query against the minimum retained D1
    # schema. Provider credentials are digests; user/app/video authorities are
    # tenant-local database facts.
    connection.executescript(
        """
        CREATE TABLE users(
          id TEXT PRIMARY KEY,email TEXT,status TEXT,deleted_at_ms INTEGER
        );
        CREATE TABLE videos(
          id TEXT PRIMARY KEY,state TEXT,deleted_at_ms INTEGER
        );
        CREATE TABLE developer_apps(
          id TEXT PRIMARY KEY,owner_user_id TEXT,status TEXT,deleted_at_ms INTEGER
        );
        CREATE TABLE developer_credit_accounts(
          id TEXT PRIMARY KEY,app_id TEXT UNIQUE
        );
        CREATE TABLE authenticated_web_action_assertions_v1(
          operation_id TEXT NOT NULL,
          assertion_kind TEXT NOT NULL,
          expected_count INTEGER NOT NULL,
          actual_count INTEGER NOT NULL,
          PRIMARY KEY(operation_id,assertion_kind),
          CHECK(expected_count = actual_count)
        );
        CREATE TABLE auth_identities_v2(
          user_id TEXT PRIMARY KEY,session_version INTEGER NOT NULL
        );
        CREATE TABLE auth_api_keys(
          id TEXT PRIMARY KEY,user_id TEXT NOT NULL,key_digest TEXT NOT NULL UNIQUE,
          expires_at_ms INTEGER,revoked_at_ms INTEGER
        );
        CREATE TABLE auth_sessions_v2(
          id TEXT PRIMARY KEY,user_id TEXT NOT NULL,state TEXT NOT NULL,
          revoked_at_ms INTEGER,
          generation INTEGER NOT NULL,token_key_version INTEGER NOT NULL,
          token_digest TEXT NOT NULL,session_version INTEGER NOT NULL,
          idle_expires_at_ms INTEGER NOT NULL,
          absolute_expires_at_ms INTEGER NOT NULL
        );
        CREATE TABLE auth_session_mutation_grants_v2(
          id TEXT PRIMARY KEY,session_id TEXT NOT NULL,user_id TEXT NOT NULL,
          generation INTEGER NOT NULL,token_key_version INTEGER NOT NULL,
          token_digest TEXT NOT NULL
        );
        INSERT INTO users VALUES
          ('user-1','person@example.test','active',NULL),
          ('admin-1','richie@cap.so','active',NULL),
          ('suspended-1','suspended@example.test','suspended',NULL);
        INSERT INTO videos VALUES('video-1','ready',NULL);
        INSERT INTO developer_apps VALUES('app-1','user-1','active',NULL);
        INSERT INTO developer_credit_accounts VALUES('account-1','app-1');
        INSERT INTO auth_identities_v2 VALUES('user-1',7),('admin-1',3);
        INSERT INTO auth_api_keys VALUES(
          'key-1','user-1','API_KEY_DIGEST',5000,NULL
        );
        INSERT INTO auth_sessions_v2 VALUES
          ('session-1','user-1','active',NULL,3,1,'token-digest',7,999999,999999),
          ('USER_SESSION','user-1','active',NULL,3,1,'USER_SESSION_DIGEST',7,9999999,9999999),
          ('ADMIN_SESSION','admin-1','active',NULL,2,1,'ADMIN_SESSION_DIGEST',3,9999999,9999999);
        """
        .replace("API_KEY_DIGEST", digest("api-key-1"))
        .replace("USER_SESSION_DIGEST", USER_SESSION_DIGEST)
        .replace("ADMIN_SESSION_DIGEST", ADMIN_SESSION_DIGEST)
        .replace("USER_SESSION", USER_SESSION_ID)
        .replace("ADMIN_SESSION", ADMIN_SESSION_ID)
    )
    authority_sql = (QUERIES / "authority_read.sql").read_text()
    def authority(actor, role, target, credential, now=1000):
        if role == "signed_stripe_webhook":
            kind = "signed_endpoint"
            subject = "stripe-webhook.endpoint.v1"
            key_version = None
        elif credential is not None:
            kind = "api_key"
            subject = "key-1"
            key_version = None
        elif actor is not None:
            kind = "session_token"
            subject = ADMIN_SESSION_ID if actor == "admin-1" else USER_SESSION_ID
            key_version = 1
            credential = (
                ADMIN_SESSION_DIGEST if actor == "admin-1" else USER_SESSION_DIGEST
            )
        else:
            kind = "public_flow"
            subject = None
            key_version = None
            credential = digest("public-flow-principal")
        return connection.execute(
            authority_sql,
            (actor, role, target, kind, subject, key_version, credential, now),
        ).fetchone()[0]
    assert authority("user-1", "active_session", None, None) == 1
    assert authority("suspended-1", "active_session", None, None) == 0
    assert authority("user-1", "developer_app_owner", "app-1", None) == 1
    assert authority("admin-1", "developer_app_owner", "app-1", None) == 0
    assert authority("admin-1", "messenger_admin_video", "video-1", None) == 1
    assert authority("user-1", "messenger_admin_video", "video-1", None) == 0
    assert authority(None, "signed_stripe_webhook", None, "a" * 64) == 1
    assert authority(None, "signed_stripe_webhook", None, "raw-secret") == 0
    assert authority("user-1", "active_session", None, digest("api-key-1")) == 1
    assert authority(
        "user-1", "active_session", None, digest("api-key-1"), now=5000
    ) == 0
    assert authority(
        "user-1", "developer_app_owner", "app-1", digest("api-key-1")
    ) == 1

    webhook_receipt = "00000000-0000-4000-8000-000000000089"
    webhook_body_digest = digest(b'{"id":"evt_1"}')
    webhook_request = redacted_request(
        "cap-v1-1e5f228815a2a8b7",
        {"id": "evt_1", "type": "checkout.session.completed"},
        webhook_body_digest,
    )
    delivery_audit_sql = (QUERIES / "delivery_audit_insert.sql").read_text()
    with connection:
        insert_stage(
            connection,
            receipt_id=webhook_receipt,
            operation_id="cap-v1-1e5f228815a2a8b7",
            replay_key="evt_1",
            request_json=webhook_request,
            human_required=True,
            actor_id=None,
            authority_class="signed_stripe_webhook",
            target_id="evt_1",
            credential_digest=digest("stripe-webhook-provider-v1"),
            auth_class="signed_webhook",
            provider_kind="stripe_webhook_reconciliation",
        )
        connection.execute(
            delivery_audit_sql,
            (
                webhook_receipt,
                digest("stripe-signature-delivery-1"),
                webhook_body_digest,
                digest(webhook_request),
                1000,
            ),
        )
    connection.execute(
        delivery_audit_sql,
        (
            webhook_receipt,
            digest("stripe-signature-delivery-2"),
            webhook_body_digest,
            digest(webhook_request),
            2000,
        ),
    )
    connection.execute(
        delivery_audit_sql,
        (
            webhook_receipt,
            digest("stripe-signature-delivery-2"),
            webhook_body_digest,
            digest(webhook_request),
            2000,
        ),
    )
    assert connection.execute(
        "SELECT COUNT(*) FROM legacy_protected_billing_auth_delivery_audit_v1 WHERE receipt_id=?",
        (webhook_receipt,),
    ).fetchone() == (2,)
    try:
        connection.execute(
            delivery_audit_sql,
            (
                webhook_receipt,
                digest("stripe-signature-wrong-body"),
                digest("wrong-body"),
                digest(webhook_request),
                3000,
            ),
        )
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_delivery_audit_invalid_v1" in str(error)
    else:
        raise AssertionError("delivery audit accepted a body not bound to its receipt")

    # Each early-read authority can go stale. The receipt trigger repeats the
    # decision inside the staging transaction, so revocation, ownership
    # transfer, and target deletion all abort without leaving an intent.
    assert authority("user-1", "active_session", None, None) == 1
    connection.execute("UPDATE users SET status='suspended' WHERE id='user-1'")
    try:
        insert_stage(
            connection,
            receipt_id="00000000-0000-4000-8000-000000000090",
            operation_id="cap-v1-78537fb518df75ec",
            replay_key="stale-session",
            request_json=redacted_request(
                "cap-v1-78537fb518df75ec", {"priceId": "price_1"}
            ),
            human_required=True,
        )
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_authority_stale_v1" in str(error)
    else:
        raise AssertionError("revoked session authority staged a receipt")
    connection.execute("UPDATE users SET status='active' WHERE id='user-1'")

    # Session-or-API-key routes carry the authenticated key digest into the
    # transaction. Revoking that key after the early read must reject staging,
    # and revoking it after staging must make the immutable receipt ineligible
    # for replay.
    api_key_digest = digest("api-key-1")
    assert authority("user-1", "active_session", None, api_key_digest) == 1
    connection.execute(
        "UPDATE auth_api_keys SET revoked_at_ms=900 WHERE key_digest=?",
        (api_key_digest,),
    )
    try:
        insert_stage(
            connection,
            receipt_id="00000000-0000-4000-8000-000000000094",
            operation_id="cap-v1-78537fb518df75ec",
            replay_key="revoked-api-key-stage",
            request_json=redacted_request(
                "cap-v1-78537fb518df75ec", {"priceId": "price_api_key"}
            ),
            human_required=True,
            credential_digest=api_key_digest,
            auth_class="session_or_api_key",
        )
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_authority_stale_v1" in str(error)
    else:
        raise AssertionError("revoked API-key authority staged a receipt")
    connection.execute(
        "UPDATE auth_api_keys SET revoked_at_ms=NULL WHERE key_digest=?",
        (api_key_digest,),
    )

    api_key_receipt = "00000000-0000-4000-8000-000000000095"
    api_key_replay_key = "live-api-key-replay"
    api_key_request = redacted_request(
        "cap-v1-78537fb518df75ec", {"priceId": "price_api_key"}
    )
    with connection:
        insert_stage(
            connection,
            receipt_id=api_key_receipt,
            operation_id="cap-v1-78537fb518df75ec",
            replay_key=api_key_replay_key,
            request_json=api_key_request,
            human_required=True,
            credential_digest=api_key_digest,
            auth_class="session_or_api_key",
        )
    replay_sql = (QUERIES / "receipt_replay.sql").read_text()
    api_key_replay_bindings = (
        "cap-v1-78537fb518df75ec",
        digest("principal"),
        digest(api_key_replay_key),
        1000,
    )
    assert connection.execute(replay_sql, api_key_replay_bindings).fetchone() is not None
    connection.execute(
        "UPDATE auth_api_keys SET revoked_at_ms=1100 WHERE key_digest=?",
        (api_key_digest,),
    )
    assert connection.execute(replay_sql, api_key_replay_bindings).fetchone() is None
    connection.execute(
        "UPDATE auth_api_keys SET revoked_at_ms=NULL WHERE key_digest=?",
        (api_key_digest,),
    )

    assert authority("user-1", "developer_app_owner", "app-1", None) == 1
    connection.execute(
        "UPDATE developer_apps SET owner_user_id='admin-1' WHERE id='app-1'"
    )
    try:
        insert_stage(
            connection,
            receipt_id="00000000-0000-4000-8000-000000000091",
            operation_id="cap-v1-60b06cc5ab45f187",
            replay_key="stale-app-owner",
            request_json=redacted_request(
                "cap-v1-60b06cc5ab45f187", {"appId": "app-1", "amountCents": 500}
            ),
            human_required=True,
            authority_class="developer_app_owner",
            target_id="app-1",
        )
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_authority_stale_v1" in str(error)
    else:
        raise AssertionError("transferred developer app authority staged a receipt")
    connection.execute(
        "UPDATE developer_apps SET owner_user_id='user-1' WHERE id='app-1'"
    )

    assert authority("admin-1", "messenger_admin_video", "video-1", None) == 1
    connection.execute(
        "UPDATE videos SET state='deleted',deleted_at_ms=1500 WHERE id='video-1'"
    )
    try:
        insert_stage(
            connection,
            receipt_id="00000000-0000-4000-8000-000000000092",
            operation_id="cap-v1-14ea978608dcf07e",
            replay_key="stale-video",
            request_json=redacted_request(
                "cap-v1-14ea978608dcf07e", {"videoId": "video-1"}
            ),
            human_required=True,
            actor_id="admin-1",
            authority_class="messenger_admin_video",
            target_id="video-1",
        )
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_authority_stale_v1" in str(error)
    else:
        raise AssertionError("deleted video authority staged a receipt")
    connection.execute(
        "UPDATE videos SET state='ready',deleted_at_ms=NULL WHERE id='video-1'"
    )

    # The browser mutation grant and protected intent are one transaction.
    # A failed stage restores the grant; an exact replay consumes a fresh grant
    # only after the immutable receipt assertion succeeds.
    grant_assert_sql = (AUTH_QUERIES / "browser_mutation_grant_assert.sql").read_text()
    grant_delete_sql = (
        AUTH_QUERIES / "browser_mutation_grant_delete_by_proof.sql"
    ).read_text()
    change_assert_sql = (
        AUTH_QUERIES / "browser_mutation_change_assert.sql"
    ).read_text()
    replay_assert_sql = (QUERIES / "receipt_replay_assert.sql").read_text()

    def mint_grant(grant_id: str) -> None:
        connection.execute(
            "INSERT INTO auth_session_mutation_grants_v2 VALUES(?,?,?,?,?,?)",
            (grant_id, "session-1", "user-1", 3, 1, "token-digest"),
        )
        connection.commit()

    # Attempted browser mutations consume only the exact proof tuple even when
    # the session/user becomes ineligible before durable staging. Revocation
    # must not strand the attempted grant or delete an unrelated one.
    mint_grant("grant-revoked-exact")
    mint_grant("grant-revoked-unrelated")
    connection.execute("UPDATE users SET status='suspended' WHERE id='user-1'")
    with connection:
        connection.execute(
            grant_delete_sql,
            ("grant-revoked-exact", "session-1", "user-1"),
        )
    assert connection.execute(
        "SELECT id FROM auth_session_mutation_grants_v2 WHERE id LIKE 'grant-revoked-%' ORDER BY id"
    ).fetchall() == [("grant-revoked-unrelated",)]
    connection.execute("UPDATE users SET status='active' WHERE id='user-1'")
    connection.execute(
        "DELETE FROM auth_session_mutation_grants_v2 WHERE id='grant-revoked-unrelated'"
    )

    atomic_operation = "cap-v1-e596f65c43ee2a82"
    atomic_request = redacted_request(atomic_operation, {})
    atomic_receipt = "00000000-0000-4000-8000-000000000080"
    mint_grant("grant-atomic-success")
    with connection:
        connection.execute(
            grant_assert_sql,
            (
                atomic_receipt,
                "grant-atomic-success",
                "session-1",
                "user-1",
                1000,
            ),
        )
        insert_stage(
            connection,
            receipt_id=atomic_receipt,
            operation_id=atomic_operation,
            replay_key="atomic-browser-stage",
            request_json=atomic_request,
            human_required=True,
        )
        connection.execute(
            grant_delete_sql,
            ("grant-atomic-success", "session-1", "user-1"),
        )
        connection.execute(change_assert_sql, (atomic_receipt, "grant_consumed"))
        connection.execute(
            "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?",
            (atomic_receipt,),
        )
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id='grant-atomic-success'"
    ).fetchone() == (0,)

    failed_receipt = "00000000-0000-4000-8000-000000000081"
    mint_grant("grant-atomic-failure")
    try:
        with connection:
            connection.execute(
                grant_assert_sql,
                (
                    failed_receipt,
                    "grant-atomic-failure",
                    "session-1",
                    "user-1",
                    1000,
                ),
            )
            insert_stage(
                connection,
                receipt_id=failed_receipt,
                operation_id=atomic_operation,
                replay_key="failed-browser-stage",
                request_json=atomic_request,
                human_required=True,
            )
            connection.execute(
                grant_delete_sql,
                ("grant-atomic-failure", "session-1", "user-1"),
            )
            connection.execute(change_assert_sql, (failed_receipt, "grant_consumed"))
            connection.execute(
                "INSERT INTO legacy_protected_billing_auth_approval_requests_v1(receipt_id,approval_scope,request_digest,state,created_at_ms) VALUES(?,'billing_admin.v1',?,'pending',1000)",
                (failed_receipt, digest("wrong")),
            )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("failed browser stage unexpectedly committed")
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id='grant-atomic-failure'"
    ).fetchone() == (1,)
    assert connection.execute(
        "SELECT COUNT(*) FROM legacy_protected_billing_auth_receipts_v1 WHERE receipt_id=?",
        (failed_receipt,),
    ).fetchone() == (0,)

    mint_grant("grant-atomic-replay")
    replay_assertion = "00000000-0000-4000-8000-000000000082"
    with connection:
        connection.execute(
            replay_assert_sql,
            (
                replay_assertion,
                atomic_operation,
                digest("principal"),
                digest("atomic-browser-stage"),
                digest(atomic_request),
                1000,
            ),
        )
        connection.execute(
            grant_assert_sql,
            (
                replay_assertion,
                "grant-atomic-replay",
                "session-1",
                "user-1",
                1000,
            ),
        )
        connection.execute(
            grant_delete_sql,
            ("grant-atomic-replay", "session-1", "user-1"),
        )
        connection.execute(change_assert_sql, (replay_assertion, "grant_consumed"))
        connection.execute(
            "DELETE FROM authenticated_web_action_assertions_v1 WHERE operation_id=?",
            (replay_assertion,),
        )
    assert connection.execute(
        "SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id='grant-atomic-replay'"
    ).fetchone() == (0,)

    parent_receipt = "00000000-0000-4000-8000-000000000093"
    parent_request = redacted_request(
        "cap-v1-14ea978608dcf07e", {"videoId": "video-1"}
    )
    with connection:
        insert_stage(
            connection,
            receipt_id=parent_receipt,
            operation_id="cap-v1-14ea978608dcf07e",
            replay_key="admin-action-parent",
            request_json=parent_request,
            human_required=True,
            actor_id="admin-1",
            authority_class="messenger_admin_video",
            target_id="video-1",
            operation_kind="server_action",
            method="ACTION",
            provider_kind="media_reprocess_workflow_dispatch",
        )
    workflow_parent_sql = (QUERIES / "workflow_parent_read.sql").read_text()
    assert connection.execute(
        workflow_parent_sql, (parent_receipt, digest(parent_request), 1000)
    ).fetchone() == (
        "admin-1",
        "video-1",
        "session_token",
        ADMIN_SESSION_ID,
        1,
        ADMIN_SESSION_DIGEST,
    )
    assert connection.execute(
        workflow_parent_sql, (parent_receipt, digest("wrong-parent"), 1000)
    ).fetchone() is None
    assert connection.execute(
        workflow_parent_sql,
        (
            "00000000-0000-4000-8000-000000000001",
            digest(
                redacted_request(
                    "cap-v1-78537fb518df75ec", {"priceId": "price_1"}
                )
            ),
            1000,
        ),
    ).fetchone() is None

    workflow_request = redacted_request(
        "cap-v1-5a990f470c701cec",
        {
            "videoId": "video-1",
            "_frameParentReceiptId": parent_receipt,
            "_frameParentRequestDigest": digest(parent_request),
        },
    )
    with connection:
        insert_stage(
            connection,
            receipt_id="00000000-0000-4000-8000-000000000094",
            operation_id="cap-v1-5a990f470c701cec",
            replay_key=f"parent-receipt:{parent_receipt}",
            request_json=workflow_request,
            human_required=True,
            actor_id="admin-1",
            authority_class="messenger_admin_video",
            target_id="video-1",
            operation_kind="workflow",
            method="WORKFLOW",
            provider_kind="storage_media_server_and_cloudfront",
        )
    forged_workflow_request = redacted_request(
        "cap-v1-5a990f470c701cec",
        {
            "videoId": "video-1",
            "_frameParentReceiptId": "00000000-0000-4000-8000-999999999999",
            "_frameParentRequestDigest": digest(parent_request),
        },
    )
    try:
        insert_stage(
            connection,
            receipt_id="00000000-0000-4000-8000-000000000095",
            operation_id="cap-v1-5a990f470c701cec",
            replay_key="forged-workflow-parent",
            request_json=forged_workflow_request,
            human_required=True,
            actor_id="admin-1",
            authority_class="messenger_admin_video",
            target_id="video-1",
            operation_kind="workflow",
            method="WORKFLOW",
            provider_kind="storage_media_server_and_cloudfront",
        )
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_workflow_parent_invalid_v1" in str(error)
    else:
        raise AssertionError("forged workflow parent staged a receipt")

    human_receipt = "00000000-0000-4000-8000-000000000001"
    operation_id = "cap-v1-78537fb518df75ec"
    request_json = redacted_request(operation_id, {"priceId": "price_1"})
    with connection:
        insert_stage(
            connection,
            receipt_id=human_receipt,
            operation_id=operation_id,
            replay_key="checkout-1",
            request_json=request_json,
            human_required=True,
        )
    assert connection.execute(
        """
        SELECT receipt.state,outbox.state,approval.state
        FROM legacy_protected_billing_auth_receipts_v1 receipt
        JOIN legacy_protected_billing_auth_outbox_v1 outbox USING(receipt_id)
        JOIN legacy_protected_billing_auth_approval_requests_v1 approval USING(receipt_id)
        WHERE receipt.receipt_id=?
        """,
        (human_receipt,),
    ).fetchone() == ("awaiting_human_approval", "blocked_human_approval", "pending")
    replay_sql = (QUERIES / "receipt_replay.sql").read_text()
    replay_bindings = (
        operation_id,
        digest("principal"),
        digest("checkout-1"),
        1000,
    )
    assert connection.execute(replay_sql, replay_bindings).fetchone() is not None
    connection.execute("UPDATE users SET status='suspended' WHERE id='user-1'")
    assert connection.execute(replay_sql, replay_bindings).fetchone() is None
    connection.execute("UPDATE users SET status='active' WHERE id='user-1'")

    # A replay key is unique per operation/principal, and changing the body
    # cannot smuggle a different checkout into the old receipt.
    try:
        with connection:
            insert_stage(
                connection,
                receipt_id="00000000-0000-4000-8000-000000000002",
                operation_id=operation_id,
                replay_key="checkout-1",
                request_json=redacted_request(operation_id, {"priceId": "price_other"}),
                human_required=True,
            )
    except sqlite3.IntegrityError as error:
        assert "UNIQUE constraint failed" in str(error)
    else:
        raise AssertionError("duplicate protected billing replay key was accepted")

    request_digest = digest(request_json)
    sealed_response_ref = f"frame-pba-http-v1:{digest('sealed-checkout-response')}"
    sealed_response_digest = digest("typed-checkout-http-envelope")
    # Provider evidence cannot bypass human approval, and receipt state cannot
    # fabricate a terminal checkout URL.
    expect_integrity(
        connection,
        """
        INSERT INTO legacy_protected_billing_auth_provider_evidence_v1(
          receipt_id,request_digest,provider_evidence_digest,sealed_response_ref,
          sealed_response_digest,verifier_class,verified_at_ms
        ) VALUES(
          '00000000-0000-4000-8000-000000000001','REQUEST','PROVIDER',
          'SEALED_REF','RESPONSE',
          'independent_provider_executor',2000
        )
        """
        .replace("REQUEST", request_digest)
        .replace("PROVIDER", digest("provider-evidence"))
        .replace("SEALED_REF", sealed_response_ref)
        .replace("RESPONSE", sealed_response_digest),
        "frame_protected_billing_auth_provider_evidence_invalid_v1",
    )
    expect_integrity(
        connection,
        f"UPDATE legacy_protected_billing_auth_receipts_v1 SET state='verified',completed_at_ms=2000 WHERE receipt_id='{human_receipt}'",
        "frame_protected_billing_auth_evidence_required_v1",
    )

    connection.execute(
        """
        INSERT INTO legacy_protected_billing_auth_human_evidence_v1(
          receipt_id,request_digest,decision,approver_subject_digest,
          approval_evidence_digest,change_ticket,verifier_class,verified_at_ms
        ) VALUES(?,?, 'approved',?,?,?,'independent_human_approver',2000)
        """,
        (
            human_receipt,
            request_digest,
            digest("approver"),
            digest("approved-change"),
            "FRAME-1234",
        ),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_approval_requests_v1 SET state='approved',resolved_at_ms=2000 WHERE receipt_id=?",
        (human_receipt,),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_outbox_v1 SET state='pending_provider_evidence' WHERE receipt_id=?",
        (human_receipt,),
    )
    connection.execute(
        """
        INSERT INTO legacy_protected_billing_auth_provider_evidence_v1(
          receipt_id,request_digest,provider_evidence_digest,sealed_response_ref,
          sealed_response_digest,verifier_class,verified_at_ms
        ) VALUES(?,?,?,?,?,'independent_provider_executor',3000)
        """,
        (
            human_receipt,
            request_digest,
            digest("provider-evidence"),
            sealed_response_ref,
            sealed_response_digest,
        ),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_outbox_v1 SET state='verified',attempt_count=1,completed_at_ms=3000 WHERE receipt_id=?",
        (human_receipt,),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_receipts_v1 SET state='verified',completed_at_ms=3000 WHERE receipt_id=?",
        (human_receipt,),
    )
    expect_integrity(
        connection,
        f"UPDATE legacy_protected_billing_auth_receipts_v1 SET redacted_request_json='{{}}' WHERE receipt_id='{human_receipt}'",
        "frame_protected_billing_auth_receipt_immutable_v1",
    )
    expect_integrity(
        connection,
        f"DELETE FROM legacy_protected_billing_auth_provider_evidence_v1 WHERE receipt_id='{human_receipt}'",
        "frame_protected_billing_auth_provider_evidence_immutable_v1",
    )

    # Provider-only NextAuth staging cannot be forced through the human path.
    provider_receipt = "00000000-0000-4000-8000-000000000003"
    provider_operation = "cap-v1-46bda1c18ffba076"
    provider_request_ref = f"frame-pba-request-v1:{digest('nextauth-provider-request')}"
    provider_request = redacted_request(
        provider_operation,
        {"nextauthPath": "/api/auth/providers"},
        sealed_request_ref=provider_request_ref,
        sealed_request_digest=digest("nextauth-provider-request-plaintext"),
    )
    with connection:
        insert_stage(
            connection,
            receipt_id=provider_receipt,
            operation_id=provider_operation,
            replay_key="nextauth-read-1",
            request_json=provider_request,
            request_digest_override=canonical_request_digest(provider_request),
            human_required=False,
            actor_id=None,
        )
    assert connection.execute(
        "SELECT state FROM legacy_protected_billing_auth_outbox_v1 WHERE receipt_id=?",
        (provider_receipt,),
    ).fetchone() == ("pending_provider_evidence",)
    assert connection.execute(
        "SELECT COUNT(*) FROM legacy_protected_billing_auth_approval_requests_v1 WHERE receipt_id=?",
        (provider_receipt,),
    ).fetchone() == (0,)

    # A failed atomic staging transaction leaves no orphan receipt.
    atomic_receipt = "00000000-0000-4000-8000-000000000004"
    try:
        with connection:
            insert_stage(
                connection,
                receipt_id=atomic_receipt,
                operation_id="cap-v1-96230bf1f2da3d00",
                replay_key="atomic-1",
                request_json=redacted_request(
                    "cap-v1-96230bf1f2da3d00", {"priceId": "price_2"}
                ),
                human_required=True,
            )
            connection.execute(
                "INSERT INTO legacy_protected_billing_auth_approval_requests_v1(receipt_id,approval_scope,request_digest,state,created_at_ms) VALUES(?,'billing_admin.v1',?,'pending',1000)",
                (atomic_receipt, digest("wrong")),
            )
    except sqlite3.IntegrityError:
        pass
    else:
        raise AssertionError("failed staging transaction unexpectedly committed")
    assert connection.execute(
        "SELECT COUNT(*) FROM legacy_protected_billing_auth_receipts_v1 WHERE receipt_id=?",
        (atomic_receipt,),
    ).fetchone() == (0,)

    # Provider execution reasserts the exact credential tuple, actor, and
    # resource authority at evidence time rather than trusting request-time
    # admission. Each mutation below occurs after a valid staged/approved
    # intent and must reject provider evidence with the dedicated marker.
    def stage_approved(
        receipt_id: str,
        operation_id: str,
        payload: dict,
        **kwargs,
    ) -> None:
        with connection:
            insert_stage(
                connection,
                receipt_id=receipt_id,
                operation_id=operation_id,
                replay_key=f"authority:{receipt_id}",
                request_json=redacted_request(operation_id, payload),
                human_required=True,
                created_at_ms=1000,
                **kwargs,
            )
            admit_human_approval(connection, receipt_id, 1100)

    session_revoke_receipt = "00000000-0000-4000-8000-000000000301"
    stage_approved(
        session_revoke_receipt,
        "cap-v1-e596f65c43ee2a82",
        {},
    )
    connection.execute(
        "UPDATE auth_sessions_v2 SET revoked_at_ms=1200 WHERE id=?",
        (USER_SESSION_ID,),
    )
    expect_execution_authority_stale(connection, session_revoke_receipt, 1300)
    connection.execute(
        "UPDATE auth_sessions_v2 SET revoked_at_ms=NULL WHERE id=?",
        (USER_SESSION_ID,),
    )

    session_expiry_receipt = "00000000-0000-4000-8000-000000000302"
    stage_approved(
        session_expiry_receipt,
        "cap-v1-96230bf1f2da3d00",
        {"priceId": "price_session_expiry"},
    )
    connection.execute(
        "UPDATE auth_sessions_v2 SET idle_expires_at_ms=1200 WHERE id=?",
        (USER_SESSION_ID,),
    )
    expect_execution_authority_stale(connection, session_expiry_receipt, 1200)
    connection.execute(
        "UPDATE auth_sessions_v2 SET idle_expires_at_ms=9999999 WHERE id=?",
        (USER_SESSION_ID,),
    )

    user_suspend_receipt = "00000000-0000-4000-8000-000000000303"
    stage_approved(
        user_suspend_receipt,
        "cap-v1-856dfea22b9d979c",
        {},
        method="GET",
    )
    connection.execute("UPDATE users SET status='suspended' WHERE id='user-1'")
    expect_execution_authority_stale(connection, user_suspend_receipt, 1300)
    connection.execute("UPDATE users SET status='active' WHERE id='user-1'")

    api_key_revoke_receipt = "00000000-0000-4000-8000-000000000304"
    stage_approved(
        api_key_revoke_receipt,
        "cap-v1-78537fb518df75ec",
        {"priceId": "price_api_revoke"},
        auth_class="session_or_api_key",
        credential_kind="api_key",
        credential_subject_id="key-1",
        credential_digest=api_key_digest,
    )
    connection.execute(
        "UPDATE auth_api_keys SET revoked_at_ms=1200 WHERE id='key-1'"
    )
    expect_execution_authority_stale(connection, api_key_revoke_receipt, 1300)
    connection.execute("UPDATE auth_api_keys SET revoked_at_ms=NULL WHERE id='key-1'")

    api_key_expiry_receipt = "00000000-0000-4000-8000-000000000305"
    stage_approved(
        api_key_expiry_receipt,
        "cap-v1-78537fb518df75ec",
        {"priceId": "price_api_expiry"},
        auth_class="session_or_api_key",
        credential_kind="api_key",
        credential_subject_id="key-1",
        credential_digest=api_key_digest,
    )
    expect_execution_authority_stale(connection, api_key_expiry_receipt, 5000)

    app_transfer_receipt = "00000000-0000-4000-8000-000000000306"
    stage_approved(
        app_transfer_receipt,
        "cap-v1-60b06cc5ab45f187",
        {"appId": "app-1", "amountCents": 500},
        auth_class="session_or_api_key",
        authority_class="developer_app_owner",
        target_id="app-1",
    )
    connection.execute(
        "UPDATE developer_apps SET owner_user_id='admin-1' WHERE id='app-1'"
    )
    expect_execution_authority_stale(connection, app_transfer_receipt, 1300)
    connection.execute(
        "UPDATE developer_apps SET owner_user_id='user-1' WHERE id='app-1'"
    )

    app_delete_receipt = "00000000-0000-4000-8000-000000000307"
    stage_approved(
        app_delete_receipt,
        "cap-v1-60b06cc5ab45f187",
        {"appId": "app-1", "amountCents": 500},
        auth_class="session_or_api_key",
        authority_class="developer_app_owner",
        target_id="app-1",
    )
    connection.execute(
        "UPDATE developer_apps SET status='deleted',deleted_at_ms=1200 WHERE id='app-1'"
    )
    expect_execution_authority_stale(connection, app_delete_receipt, 1300)
    connection.execute(
        "UPDATE developer_apps SET status='active',deleted_at_ms=NULL WHERE id='app-1'"
    )

    video_delete_receipt = "00000000-0000-4000-8000-000000000308"
    stage_approved(
        video_delete_receipt,
        "cap-v1-e488991f97723847",
        {"videoId": "video-1"},
        actor_id="admin-1",
        auth_class="admin_session",
        authority_class="messenger_admin_video",
        target_id="video-1",
        operation_kind="server_action",
        method="ACTION",
        provider_kind="cloudfront_cache_invalidation",
    )
    connection.execute(
        "UPDATE videos SET state='deleted',deleted_at_ms=1200 WHERE id='video-1'"
    )
    expect_execution_authority_stale(connection, video_delete_receipt, 1300)
    connection.execute(
        "UPDATE videos SET state='ready',deleted_at_ms=NULL WHERE id='video-1'"
    )

    def stage_workflow_pair(
        parent_receipt_id: str,
        workflow_receipt_id: str,
        suffix: str,
    ) -> tuple[str, str]:
        parent_payload = {"videoId": "video-1"}
        parent_request_json = redacted_request(
            "cap-v1-14ea978608dcf07e", parent_payload
        )
        with connection:
            insert_stage(
                connection,
                receipt_id=parent_receipt_id,
                operation_id="cap-v1-14ea978608dcf07e",
                replay_key=f"workflow-parent:{suffix}",
                request_json=parent_request_json,
                human_required=True,
                actor_id="admin-1",
                auth_class="admin_session",
                authority_class="messenger_admin_video",
                target_id="video-1",
                operation_kind="server_action",
                method="ACTION",
                provider_kind="media_reprocess_workflow_dispatch",
            )
            workflow_request_json = redacted_request(
                "cap-v1-5a990f470c701cec",
                {
                    "videoId": "video-1",
                    "_frameParentReceiptId": parent_receipt_id,
                    "_frameParentRequestDigest": digest(parent_request_json),
                },
            )
            insert_stage(
                connection,
                receipt_id=workflow_receipt_id,
                operation_id="cap-v1-5a990f470c701cec",
                replay_key=f"workflow-child:{suffix}",
                request_json=workflow_request_json,
                human_required=True,
                actor_id="admin-1",
                auth_class="admin_session",
                authority_class="messenger_admin_video",
                target_id="video-1",
                operation_kind="workflow",
                method="WORKFLOW",
                provider_kind="storage_media_server_and_cloudfront",
            )
            admit_human_approval(connection, workflow_receipt_id, 1100)
        return parent_request_json, workflow_request_json

    dead_parent = "00000000-0000-4000-8000-000000000309"
    dead_workflow = "00000000-0000-4000-8000-000000000310"
    stage_workflow_pair(dead_parent, dead_workflow, "dead-letter")
    admit_human_approval(connection, dead_parent, 1150)
    connection.execute(
        "UPDATE legacy_protected_billing_auth_receipts_v1 SET state='dead_letter',completed_at_ms=1200 WHERE receipt_id=?",
        (dead_parent,),
    )
    expect_execution_authority_stale(connection, dead_workflow, 1300)

    rejected_parent = "00000000-0000-4000-8000-000000000311"
    rejected_workflow = "00000000-0000-4000-8000-000000000312"
    rejected_parent_request, _ = stage_workflow_pair(
        rejected_parent, rejected_workflow, "rejected"
    )
    connection.execute(
        """
        INSERT INTO legacy_protected_billing_auth_human_evidence_v1(
          receipt_id,request_digest,decision,approver_subject_digest,
          approval_evidence_digest,change_ticket,verifier_class,verified_at_ms
        ) VALUES(?,?,'rejected',?,?,?,'independent_human_approver',1150)
        """,
        (
            rejected_parent,
            digest(rejected_parent_request),
            digest("rejecting-approver"),
            digest("rejected-workflow-parent"),
            "FRAME-REJECT-PARENT",
        ),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_approval_requests_v1 SET state='rejected',resolved_at_ms=1150 WHERE receipt_id=?",
        (rejected_parent,),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_receipts_v1 SET state='rejected',completed_at_ms=1200 WHERE receipt_id=?",
        (rejected_parent,),
    )
    expect_execution_authority_stale(connection, rejected_workflow, 1300)

    # No-key route traffic claims its deterministic request identity in the
    # same transaction as the receipt. A concurrent request with a different
    # five-minute key cannot create a second pending intent; pending work is
    # discoverable indefinitely, terminal work for 15 minutes, and only then
    # can an intentional later attempt replace the claim.
    generated_operation = "cap-v1-46bda1c18ffba076"
    generated_receipt = "00000000-0000-4000-8000-000000000201"
    generated_ref = f"frame-pba-request-v1:{digest('generated-request-ciphertext-1')}"
    generated_request = redacted_request(
        generated_operation,
        {"nextauthPath": "/api/auth/providers"},
        sealed_request_ref=generated_ref,
        sealed_request_digest=digest("generated-request-plaintext"),
    )
    generated_digest = canonical_request_digest(generated_request)
    generated_principal = digest("principal")
    claim_upsert_sql = (QUERIES / "generated_claim_upsert.sql").read_text()
    generated_replay_sql = (QUERIES / "generated_receipt_replay.sql").read_text()
    with connection:
        insert_stage(
            connection,
            receipt_id=generated_receipt,
            operation_id=generated_operation,
            replay_key="generated-window-1",
            request_json=generated_request,
            request_digest_override=generated_digest,
            human_required=False,
            actor_id=None,
            replay_origin="generated",
            idempotency_mode="forbidden",
        )
        connection.execute(
            claim_upsert_sql,
            (
                generated_operation,
                generated_principal,
                generated_digest,
                generated_receipt,
                1000,
                0,
            ),
        )
    concurrent_receipt = "00000000-0000-4000-8000-000000000202"
    randomized_retry = redacted_request(
        generated_operation,
        {"nextauthPath": "/api/auth/providers"},
        sealed_request_ref=f"frame-pba-request-v1:{digest('generated-request-ciphertext-2')}",
        sealed_request_digest=digest("generated-request-plaintext"),
    )
    assert canonical_request_digest(randomized_retry) == generated_digest
    try:
        with connection:
            insert_stage(
                connection,
                receipt_id=concurrent_receipt,
                operation_id=generated_operation,
                replay_key="generated-window-2",
                request_json=randomized_retry,
                request_digest_override=generated_digest,
                human_required=False,
                actor_id=None,
                replay_origin="generated",
                idempotency_mode="forbidden",
                created_at_ms=301000,
            )
    except sqlite3.IntegrityError as error:
        assert "frame_protected_billing_auth_generated_replay_claimed_v1" in str(error)
    else:
        raise AssertionError("concurrent generated request created a second receipt")
    assert connection.execute(
        "SELECT COUNT(*) FROM legacy_protected_billing_auth_receipts_v1 WHERE receipt_id=?",
        (concurrent_receipt,),
    ).fetchone() == (0,)
    assert connection.execute(
        generated_replay_sql,
        (generated_operation, generated_principal, generated_digest, 9_000_000, 9_000_000),
    ).fetchone()[0] == generated_receipt

    insert_sealed_provider_evidence(connection, generated_receipt, 2000)
    connection.execute(
        "UPDATE legacy_protected_billing_auth_outbox_v1 SET state='verified',attempt_count=1,completed_at_ms=2000 WHERE receipt_id=?",
        (generated_receipt,),
    )
    connection.execute(
        "UPDATE legacy_protected_billing_auth_receipts_v1 SET state='verified',completed_at_ms=2000 WHERE receipt_id=?",
        (generated_receipt,),
    )
    assert connection.execute(
        generated_replay_sql,
        (generated_operation, generated_principal, generated_digest, 1999, 901999),
    ).fetchone()[0] == generated_receipt
    assert connection.execute(
        generated_replay_sql,
        (generated_operation, generated_principal, generated_digest, 2000, 902000),
    ).fetchone() is None

    replacement_receipt = "00000000-0000-4000-8000-000000000203"
    with connection:
        insert_stage(
            connection,
            receipt_id=replacement_receipt,
            operation_id=generated_operation,
            replay_key="generated-window-3",
            request_json=randomized_retry,
            request_digest_override=generated_digest,
            human_required=False,
            actor_id=None,
            replay_origin="generated",
            idempotency_mode="forbidden",
            created_at_ms=902001,
        )
        connection.execute(
            claim_upsert_sql,
            (
                generated_operation,
                generated_principal,
                generated_digest,
                replacement_receipt,
                902001,
                2001,
            ),
        )
    assert connection.execute(
        "SELECT receipt_id FROM legacy_protected_billing_auth_generated_replay_claims_v1 WHERE source_operation_id=? AND principal_digest=? AND request_digest=?",
        (generated_operation, generated_principal, generated_digest),
    ).fetchone() == (replacement_receipt,)

    # Capability-bearing terminal material is structurally impossible in D1.
    # The durable rows contain only opaque references and digests.
    durable_text = []
    for table in (
        "legacy_protected_billing_auth_receipts_v1",
        "legacy_protected_billing_auth_outbox_v1",
        "legacy_protected_billing_auth_provider_evidence_v1",
    ):
        for row in connection.execute(f"SELECT * FROM {table}").fetchall():
            durable_text.extend(str(value) for value in row if isinstance(value, str))
    durable_blob = "\n".join(durable_text)
    for secret in (
        "oauth-code-secret",
        "csrf-cookie-secret",
        "session-cookie-secret",
        "checkout-url-secret",
        "billing-portal-secret",
        "X-Amz-Signature=presigned-secret",
        "provider-token-secret",
    ):
        assert secret not in durable_blob
    assert connection.execute("PRAGMA foreign_key_check").fetchall() == []


def main() -> int:
    validate_fixture()
    validate_source_contracts()
    validate_database()
    print("legacy protected billing/auth SQLite conformance passed (17 operations)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, KeyError, OSError, sqlite3.Error) as error:
        print(
            f"legacy protected billing/auth SQLite conformance failed: {error}",
            file=sys.stderr,
        )
        raise SystemExit(1) from error
