#!/usr/bin/env python3
"""Offline D1 conformance for multipart verification and Instant publication.

This suite does not emulate R2 or a Worker. It proves the relational crash
fence used after a complete object has been streamed and hashed, immutable
Instant request retention, tenant binding, replay rows, and the all-or-nothing
publication state reached by the control-plane reconciler.
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import sqlite3
import struct
from collections.abc import Callable


ROOT = pathlib.Path(__file__).resolve().parents[2]
DIRECT_CONFORMANCE = ROOT / "scripts" / "ci" / "direct-upload-sqlite-conformance.py"
MIGRATION = ROOT / "apps" / "control-plane" / "migrations" / "0024_instant_finalize_runtime.sql"
NOW = 1_700_600_000_000
ORG = "018f47a6-7b1c-7f55-8f39-8f8a8690b601"
OTHER_ORG = "018f47a6-7b1c-7f55-8f39-8f8a8690b602"
USER = "018f47a6-7b1c-7f55-8f39-8f8a8690a601"
VIDEO = "018f47a6-7b1c-7f55-8f39-8f8a8690c601"
UPLOAD = "018f47a6-7b1c-7f55-8f39-8f8a8690d610"
SESSION = "018f47a6-7b1c-7f55-8f39-8f8a8690e610"
OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8690f610"
JOB_ID = "018f47a6-7b1c-7f55-8f39-8f8a86911610"
PUBLICATION_ID = "018f47a6-7b1c-7f55-8f39-8f8a86912610"
INTEGRATION_ID = "018f47a6-7b1c-7f55-8f39-8f8a86913610"
STORAGE_OBJECT_ID = "018f47a6-7b1c-7f55-8f39-8f8a86914610"
ABORT_UPLOAD = "018f47a6-7b1c-7f55-8f39-8f8a86915610"
AUTH_ABORT_UPLOAD = "018f47a6-7b1c-7f55-8f39-8f8a86915611"
AUTH_ABORT_CLAIM_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8691d610"
AUTH_ABORT_STALE_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8691e610"
AUTH_ABORT_RETRY_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a8691f610"
AUTH_ABORT_FINISH_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a86920610"
PRESERVED_ABORT_UPLOAD = "018f47a6-7b1c-7f55-8f39-8f8a86915612"
PRESERVED_ABORT_CLAIM_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a86921610"
PRESERVED_ABORT_FINISH_OPERATION = "018f47a6-7b1c-7f55-8f39-8f8a86922610"
SCAN_UPLOAD_A = "018f47a6-7b1c-7f55-8f39-8f8a86916610"
SCAN_UPLOAD_B = "018f47a6-7b1c-7f55-8f39-8f8a86917610"
SCAN_SESSION_A = "018f47a6-7b1c-7f55-8f39-8f8a86918610"
SCAN_SESSION_B = "018f47a6-7b1c-7f55-8f39-8f8a86919610"
SCAN_JOB_A = "018f47a6-7b1c-7f55-8f39-8f8a8691a610"
SCAN_JOB_B = "018f47a6-7b1c-7f55-8f39-8f8a8691b610"
SCAN_OPERATION_A = "018f47a6-7b1c-7f55-8f39-8f8a8691c610"
CHECKSUM = "ab" * 32
ORDERED_PARTS = "cd" * 32
PROBE_DIGEST = "de" * 32
PROVIDER_VERSION = "provider-version-v1"
PROVIDER_ETAG = '"provider-etag-v1"'
BYTES = 10 * 1024 * 1024
PART_SIZE = 5 * 1024 * 1024
OBJECT_KEY = f"tenants/{ORG}/videos/{VIDEO}/source/v2/payload.mp4"
INITIAL_EVENT = "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9"


class ConformanceFailure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConformanceFailure(message)


def expect_integrity(operation: Callable[[], object], fragment: str) -> None:
    try:
        operation()
    except sqlite3.IntegrityError as error:
        require(fragment in str(error), f"wrong integrity failure: {error}")
    else:
        raise ConformanceFailure(f"expected integrity failure containing {fragment!r}")


def load_direct_module():
    specification = importlib.util.spec_from_file_location(
        "frame_direct_upload_conformance", DIRECT_CONFORMANCE
    )
    require(
        specification is not None and specification.loader is not None,
        "direct-upload conformance helper is unavailable",
    )
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def append_text(digest: hashlib._Hash, value: str) -> None:
    encoded = value.encode("utf-8")
    digest.update(struct.pack(">I", len(encoded)))
    digest.update(encoded)


def object_version(provider_version: str) -> str:
    digest = hashlib.sha256()
    digest.update(b"frame.instant.r2-object-version.v1\0")
    append_text(digest, provider_version)
    return digest.hexdigest()


def request_digest(object_version_digest: str) -> str:
    digest = hashlib.sha256()
    digest.update(b"frame.instant.finalize-wire.v1\0")
    digest.update(struct.pack(">H", 1))
    for value in (ORG, SESSION, UPLOAD, VIDEO):
        append_text(digest, value)
    for value in (ORDERED_PARTS, object_version_digest, JOB_ID):
        append_text(digest, value)
    digest.update(struct.pack(">Q", 1))
    return digest.hexdigest()


def insert_finalize_request(
    database: sqlite3.Connection,
    *,
    organization_id: str,
    session_id: str,
    request_sha256: str,
    object_version_digest: str,
) -> None:
    database.execute(
        "INSERT INTO instant_finalize_requests_v1("
        "session_id,organization_id,upload_id,video_id,ordered_parts_sha256,object_version,"
        "job_id,job_generation,request_sha256,state,publication_id,playable_object_key,"
        "distribution_eligible,reconcile_attempt_count,next_attempt_at_ms,last_failure_class,"
        "created_at_ms,updated_at_ms,published_at_ms,dead_lettered_at_ms) "
        "VALUES (?,?,?,?,?,?,?,1,?,'pending',NULL,NULL,0,0,?,NULL,?,?,NULL,NULL)",
        (
            session_id,
            organization_id,
            UPLOAD,
            VIDEO,
            ORDERED_PARTS,
            object_version_digest,
            JOB_ID,
            request_sha256,
            NOW,
            NOW,
            NOW,
        ),
    )


def apply_publication_mutations(
    database: sqlite3.Connection,
    *,
    update_job: bool = True,
) -> None:
    for state in ("uploading", "finalizing"):
        database.execute(
            "UPDATE video_uploads SET state=?,updated_at_ms=?,revision=revision+1,"
            "event_sequence=event_sequence+1,event_fingerprint=? "
            "WHERE id=? AND organization_id=?",
            (state, NOW + 10, hashlib.sha256(state.encode()).hexdigest(), UPLOAD, ORG),
        )
    database.execute(
        "UPDATE video_uploads SET state='complete',received_bytes=expected_bytes,"
        "checksum_sha256=?,updated_at_ms=?,revision=revision+1,"
        "event_sequence=event_sequence+1,event_fingerprint=? WHERE id=? AND organization_id=?",
        (CHECKSUM, NOW + 11, hashlib.sha256(b"complete").hexdigest(), UPLOAD, ORG),
    )
    database.execute(
        "INSERT INTO storage_objects(id,organization_id,integration_id,video_id,object_key,"
        "role,object_version,state,bytes,content_type,checksum_sha256,provider_etag,created_at_ms) "
        "VALUES (?,?,?,?,?,'source',2,'available',?,'video/mp4',?,?,?)",
        (
            STORAGE_OBJECT_ID,
            ORG,
            INTEGRATION_ID,
            VIDEO,
            OBJECT_KEY,
            BYTES,
            CHECKSUM,
            PROVIDER_ETAG,
            NOW + 11,
        ),
    )
    database.execute(
        "INSERT INTO storage_governed_objects_v1(organization_id,object_key,role,visibility,"
        "state,malware_disposition,immutable_revision,cache_generation,checksum_sha256,bytes,"
        "content_type,retention_until_ms,created_at_ms,updated_at_ms) "
        "VALUES (?,?,'source','private','active','clean',2,1,?,?,'video/mp4',NULL,?,?)",
        (ORG, OBJECT_KEY, CHECKSUM, BYTES, NOW + 11, NOW + 11),
    )
    database.execute(
        "UPDATE videos SET source_object_key=?,playback_object_key=?,duration_ms=60000,"
        "state='ready',updated_at_ms=?,revision=revision+1 "
        "WHERE id=? AND organization_id=? AND deleted_at_ms IS NULL",
        (OBJECT_KEY, OBJECT_KEY, NOW + 11, VIDEO, ORG),
    )
    database.execute(
        "UPDATE instant_finalize_requests_v1 SET state='published',publication_id=?,"
        "playable_object_key=?,distribution_eligible=1,updated_at_ms=?,published_at_ms=?,"
        "last_failure_class=NULL WHERE session_id=? AND state='pending'",
        (PUBLICATION_ID, OBJECT_KEY, NOW + 11, NOW + 11, SESSION),
    )
    if update_job:
        database.execute(
            "UPDATE instant_finalize_jobs_v1 SET state='published',updated_at_ms=? "
            "WHERE session_id=? AND state='retained'",
            (NOW + 11, SESSION),
        )
    database.execute(
        "UPDATE instant_finalize_operations_v1 SET result_state='published',publication_id=? "
        "WHERE session_id=? AND result_state='pending'",
        (PUBLICATION_ID, SESSION),
    )
    database.execute(
        "INSERT INTO instant_finalize_publication_assertions_v1(session_id,publication_id,"
        "asserted_at_ms) VALUES (?,?,?)",
        (SESSION, PUBLICATION_ID, NOW + 11),
    )


def insert_brokered_upload(
    database: sqlite3.Connection,
    upload_id: str,
    object_key: str,
    idempotency_key: str,
    *,
    created_at_ms: int = NOW,
) -> None:
    database.execute(
        "INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,"
        "received_bytes,idempotency_key,source_object_key,source_version,content_type,"
        "checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,event_fingerprint,"
        "transfer_mode,direct_staging_key,direct_checksum_sha256,direct_expires_at_ms) "
        "VALUES (?,?,?,'initiated',?,0,?,?,2,'video/mp4',NULL,?,?,0,0,?,'brokered',NULL,NULL,NULL)",
        (
            upload_id,
            ORG,
            VIDEO,
            BYTES,
            idempotency_key,
            object_key,
            created_at_ms,
            created_at_ms,
            INITIAL_EVENT,
        ),
    )


def insert_scan_request(
    database: sqlite3.Connection,
    *,
    session_id: str,
    upload_id: str,
    job_id: str,
    request_sha256: str,
    created_at_ms: int,
) -> None:
    database.execute(
        "INSERT INTO instant_finalize_requests_v1(session_id,organization_id,upload_id,video_id,"
        "ordered_parts_sha256,object_version,job_id,job_generation,request_sha256,state,"
        "publication_id,playable_object_key,distribution_eligible,reconcile_attempt_count,"
        "next_attempt_at_ms,last_failure_class,created_at_ms,updated_at_ms,published_at_ms,"
        "dead_lettered_at_ms) VALUES (?,?,?,?,?,?,?,1,?,'pending',NULL,NULL,0,0,?,NULL,?,?,NULL,NULL)",
        (
            session_id,
            ORG,
            upload_id,
            VIDEO,
            hashlib.sha256(f"parts:{session_id}".encode()).hexdigest(),
            hashlib.sha256(f"version:{session_id}".encode()).hexdigest(),
            job_id,
            request_sha256,
            NOW,
            created_at_ms,
            created_at_ms,
        ),
    )


def insert_abort_change_assertion(
    database: sqlite3.Connection,
    *,
    operation_id: str,
    upload_id: str,
    assertion_kind: str,
) -> None:
    database.execute(
        "INSERT INTO r2_multipart_abort_batch_assertions_v1("
        "operation_id,upload_id,assertion_kind,expected_count,actual_count) "
        "VALUES (?,?,?,1,changes())",
        (operation_id, upload_id, assertion_kind),
    )


def claim_authenticated_abort(
    database: sqlite3.Connection,
    *,
    upload_id: str,
    operation_id: str,
    now_ms: int,
    lock_until_ms: int,
) -> None:
    """Apply the durable, row-count-asserted manual-abort claim batch."""
    with database:
        database.execute(
            "INSERT INTO r2_multipart_abort_reconciliation_v1("
            "upload_id,intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class,"
            "started_at_ms,updated_at_ms,terminal_at_ms) "
            "SELECT ?,'authenticated_delete','pending',1,?,NULL,?,?,NULL "
            "WHERE EXISTS (SELECT 1 FROM r2_multipart_sessions_v1 session "
            "JOIN video_uploads upload ON upload.id=session.upload_id "
            "WHERE session.upload_id=? AND upload.organization_id=? "
            "AND session.state IN ('open','completing') "
            "AND upload.state IN ('initiated','uploading','finalizing','failed')) "
            "ON CONFLICT(upload_id) DO UPDATE SET "
            "intent_kind='authenticated_delete',attempt_count=attempt_count+1,"
            "next_attempt_at_ms=excluded.next_attempt_at_ms,last_failure_class=NULL,"
            "updated_at_ms=excluded.updated_at_ms "
            "WHERE state='pending' "
            "AND (intent_kind='expiry_cleanup' OR next_attempt_at_ms<=excluded.updated_at_ms)",
            (upload_id, lock_until_ms, now_ms, now_ms, upload_id, ORG),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="attempt_claim",
        )
        database.execute(
            "DELETE FROM r2_multipart_abort_batch_assertions_v1 WHERE operation_id=?",
            (operation_id,),
        )


def finish_authenticated_abort(
    database: sqlite3.Connection,
    *,
    upload_id: str,
    operation_id: str,
    attempt: int,
    now_ms: int,
    match_video_row: bool = True,
) -> None:
    """Apply the atomic D1 half after provider success or provider NotFound."""
    with database:
        database.execute(
            "UPDATE r2_multipart_sessions_v1 SET state='aborted' "
            "WHERE upload_id=? AND state IN ('open','completing')",
            (upload_id,),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="session_transition",
        )
        database.execute(
            "UPDATE video_uploads SET state='aborted',updated_at_ms=?,revision=revision+1,"
            "event_sequence=event_sequence+1,event_fingerprint=? "
            "WHERE id=? AND organization_id=? "
            "AND state IN ('initiated','uploading','finalizing','failed') AND ?=1",
            (
                now_ms,
                hashlib.sha256(f"authenticated-abort:{attempt}".encode()).hexdigest(),
                upload_id,
                ORG,
                int(match_video_row),
            ),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="video_upload_transition",
        )
        database.execute(
            "UPDATE r2_multipart_abort_reconciliation_v1 SET state='confirmed',"
            "next_attempt_at_ms=?,last_failure_class=NULL,updated_at_ms=?,terminal_at_ms=? "
            "WHERE upload_id=? AND intent_kind='authenticated_delete' "
            "AND state='pending' AND attempt_count=?",
            (now_ms, now_ms, now_ms, upload_id, attempt),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="reconciliation_transition",
        )
        database.execute(
            "INSERT INTO r2_multipart_abort_terminal_assertions_v1("
            "upload_id,outcome,asserted_at_ms) VALUES (?,'confirmed',?)",
            (upload_id, now_ms),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="terminal_assertion",
        )
        database.execute(
            "DELETE FROM r2_multipart_abort_batch_assertions_v1 WHERE operation_id=?",
            (operation_id,),
        )


def finish_preserved_authenticated_abort(
    database: sqlite3.Connection,
    *,
    upload_id: str,
    operation_id: str,
    attempt: int,
    now_ms: int,
) -> None:
    """Record that HEAD found a completed object without aborting its upload row."""
    with database:
        database.execute(
            "UPDATE r2_multipart_sessions_v1 SET state='completing' "
            "WHERE upload_id=? AND state IN ('open','completing')",
            (upload_id,),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="session_transition",
        )
        database.execute(
            "UPDATE r2_multipart_abort_reconciliation_v1 SET state='preserved_object',"
            "next_attempt_at_ms=?,last_failure_class=NULL,updated_at_ms=?,terminal_at_ms=? "
            "WHERE upload_id=? AND intent_kind='authenticated_delete' "
            "AND state='pending' AND attempt_count=?",
            (now_ms, now_ms, now_ms, upload_id, attempt),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="reconciliation_transition",
        )
        database.execute(
            "INSERT INTO r2_multipart_abort_terminal_assertions_v1("
            "upload_id,outcome,asserted_at_ms) VALUES (?,'preserved_object',?)",
            (upload_id, now_ms),
        )
        insert_abort_change_assertion(
            database,
            operation_id=operation_id,
            upload_id=upload_id,
            assertion_kind="terminal_assertion",
        )
        database.execute(
            "DELETE FROM r2_multipart_abort_batch_assertions_v1 WHERE operation_id=?",
            (operation_id,),
        )


def run() -> dict[str, object]:
    direct = load_direct_module()
    database = sqlite3.connect(":memory:")
    direct.migrate(database)
    direct.seed(database)
    database.executescript(MIGRATION.read_text(encoding="utf-8"))
    database.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,"
        "updated_at_ms,tombstoned_at_ms,revision,authority_version,retention_until_ms,"
        "recovered_at_ms,last_operation_id) "
        "VALUES (?,?,?,'active','{}',?,?,NULL,0,0,NULL,NULL,?)",
        (OTHER_ORG, USER, "Other Tenant", NOW - 1_000, NOW - 1_000, SESSION),
    )
    capabilities_json = json.dumps(
        {"multipart": True, "schema_version": 1},
        sort_keys=True,
        separators=(",", ":"),
    )
    database.execute(
        "INSERT INTO storage_integrations(id,organization_id,owner_user_id,provider,state,"
        "capabilities_json,credential_ciphertext,created_at_ms,updated_at_ms,revision,"
        "capabilities_schema_version,capabilities_checksum) "
        "VALUES (?,?,?,'r2','active',?,'fixture-ciphertext',?,?,0,1,?)",
        (
            INTEGRATION_ID,
            ORG,
            USER,
            capabilities_json,
            NOW - 100,
            NOW - 100,
            hashlib.sha256(capabilities_json.encode()).hexdigest(),
        ),
    )
    database.execute(
        "INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,"
        "received_bytes,idempotency_key,source_object_key,source_version,content_type,"
        "checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,event_fingerprint,"
        "transfer_mode,direct_staging_key,direct_checksum_sha256,direct_expires_at_ms) "
        "VALUES (?,?,?,'initiated',?,0,?,?,2,'video/mp4',NULL,?,?,0,0,?,'brokered',NULL,NULL,NULL)",
        (
            UPLOAD,
            ORG,
            VIDEO,
            BYTES,
            "instant-multipart-v1",
            OBJECT_KEY,
            NOW,
            NOW,
            INITIAL_EVENT,
        ),
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO r2_multipart_intents_v1(upload_id,integration_id,checksum_sha256,part_size,"
            "part_count,expires_at_ms,created_at_ms) VALUES (?,?,?,?,?,?,?)",
            (UPLOAD, INTEGRATION_ID, CHECKSUM, PART_SIZE, 3, NOW + 86_400_000, NOW),
        ),
        "frame_r2_multipart_intent_v1",
    )
    database.execute(
        "INSERT INTO r2_multipart_intents_v1(upload_id,integration_id,checksum_sha256,part_size,"
        "part_count,expires_at_ms,created_at_ms) VALUES (?,?,?,?,?,?,?)",
        (UPLOAD, INTEGRATION_ID, CHECKSUM, PART_SIZE, 2, NOW + 86_400_000, NOW),
    )
    database.execute(
        "INSERT INTO r2_multipart_sessions_v1(upload_id,object_key,provider_upload_id,state,"
        "expected_bytes,checksum_sha256,content_type,correlation_id,created_at_ms,"
        "expires_at_ms,completed_at_ms) VALUES (?,?,?,'completing',?,?,'video/mp4',?,?,?,NULL)",
        (UPLOAD, OBJECT_KEY, "opaque-provider-upload", BYTES, CHECKSUM, UPLOAD, NOW, NOW + 86_400_000),
    )
    expect_integrity(
        lambda: database.execute(
            "INSERT INTO r2_multipart_verified_objects_v1(upload_id,provider_version,"
            "provider_etag,bytes,checksum_sha256,content_type,verified_at_ms) "
            "VALUES (?,?,?,?,?,'video/mp4',?)",
            (UPLOAD, PROVIDER_VERSION, PROVIDER_ETAG, BYTES, "ef" * 32, NOW + 1),
        ),
        "frame_r2_multipart_verified_object_v1",
    )
    database.execute(
        "INSERT INTO r2_multipart_verified_objects_v1(upload_id,provider_version,provider_etag,"
        "bytes,checksum_sha256,content_type,verified_at_ms) VALUES (?,?,?,?,?,'video/mp4',?)",
        (UPLOAD, PROVIDER_VERSION, PROVIDER_ETAG, BYTES, CHECKSUM, NOW + 1),
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE r2_multipart_verified_objects_v1 SET provider_etag=? WHERE upload_id=?",
            ('"changed"', UPLOAD),
        ),
        "frame_r2_multipart_verified_object_v1",
    )
    database.execute(
        "INSERT INTO object_manifests(object_key,video_id,role,bytes,checksum_sha256,content_type,"
        "created_at_ms,organization_id,object_version,provider_etag,state,updated_at_ms) "
        "VALUES (?,?,'source',?,?,'video/mp4',?,?,2,?,'available',?)",
        (OBJECT_KEY, VIDEO, BYTES, CHECKSUM, NOW + 2, ORG, PROVIDER_ETAG, NOW + 2),
    )
    database.execute(
        "INSERT INTO media_source_probes_v1(organization_id,video_id,source_version,"
        "source_object_key,source_checksum_sha256,source_bytes,source_content_type,container,"
        "video_codec,audio_codec,duration_ms,width,height,frame_rate_numerator,"
        "frame_rate_denominator,decoded_bytes_upper_bound,frame_count_upper_bound,track_count,"
        "probe_contract_version,probe_digest,trust,state,verified_at_ms,updated_at_ms) "
        "VALUES (?,?,2,?,?,?,'video/mp4','mp4','h264','aac',60000,1920,1080,30,1,"
        "500000000,1800,2,1,?,'verified_native_probe','verified',?,?)",
        (ORG, VIDEO, OBJECT_KEY, CHECKSUM, BYTES, PROBE_DIGEST, NOW + 3, NOW + 3),
    )
    database.execute(
        "INSERT INTO r2_multipart_completions_v1(upload_id,request_parts_sha256,provider_version,"
        "provider_etag,bytes,checksum_sha256,content_type,container,video_codec,audio_codec,width,"
        "height,duration_ms,frame_rate_millihertz,completed_at_ms,correlation_id) "
        "VALUES (?,?,?,?,?,?,'video/mp4','mp4','h264','aac',1920,1080,60000,30000,?,?)",
        (UPLOAD, ORDERED_PARTS, PROVIDER_VERSION, PROVIDER_ETAG, BYTES, CHECKSUM, NOW + 4, UPLOAD),
    )
    database.execute(
        "UPDATE r2_multipart_sessions_v1 SET state='complete',completed_at_ms=? WHERE upload_id=?",
        (NOW + 4, UPLOAD),
    )

    version_digest = object_version(PROVIDER_VERSION)
    finalize_digest = request_digest(version_digest)
    expect_integrity(
        lambda: insert_finalize_request(
            database,
            organization_id=OTHER_ORG,
            session_id="018f47a6-7b1c-7f55-8f39-8f8a8690e611",
            request_sha256="fa" * 32,
            object_version_digest=version_digest,
        ),
        "frame_instant_finalize_scope_v1",
    )
    insert_finalize_request(
        database,
        organization_id=ORG,
        session_id=SESSION,
        request_sha256=finalize_digest,
        object_version_digest=version_digest,
    )
    database.execute(
        "INSERT INTO instant_finalize_jobs_v1(job_id,session_id,generation,request_sha256,state,"
        "created_at_ms,updated_at_ms) VALUES (?,?,1,?,'retained',?,?)",
        (JOB_ID, SESSION, finalize_digest, NOW, NOW),
    )
    database.execute(
        "INSERT INTO instant_finalize_operations_v1(operation_id,session_id,request_sha256,"
        "result_state,publication_id,committed_at_ms) VALUES (?,?,?,'pending',NULL,?)",
        (OPERATION, SESSION, finalize_digest, NOW),
    )
    database.execute(
        "INSERT INTO instant_finalize_http_idempotency_v1(organization_id,idempotency_key,"
        "operation_id,session_id,request_sha256,job_id,created_at_ms) VALUES (?,?,?,?,?,?,?)",
        (ORG, "instant-finalize-001", OPERATION, SESSION, finalize_digest, JOB_ID, NOW),
    )
    database.execute(
        "INSERT INTO instant_finalize_reservation_assertions_v1(operation_id,organization_id,"
        "idempotency_key,asserted_at_ms) VALUES (?,?,?,?)",
        (OPERATION, ORG, "instant-finalize-001", NOW),
    )

    database.commit()

    def conflicting_http_retry() -> None:
        with database:
            retry_operation = "018f47a6-7b1c-7f55-8f39-8f8a8690f611"
            database.execute(
                "INSERT INTO instant_finalize_operations_v1(operation_id,session_id,"
                "request_sha256,result_state,publication_id,committed_at_ms) "
                "VALUES (?,?,?,'pending',NULL,?)",
                (retry_operation, SESSION, finalize_digest, NOW + 1),
            )
            database.execute(
                "INSERT INTO instant_finalize_http_idempotency_v1(organization_id,"
                "idempotency_key,operation_id,session_id,request_sha256,job_id,created_at_ms) "
                "VALUES (?,?,?,?,?,?,?) ON CONFLICT(organization_id,idempotency_key) DO NOTHING",
                (
                    ORG,
                    "instant-finalize-001",
                    retry_operation,
                    SESSION,
                    finalize_digest,
                    JOB_ID,
                    NOW + 1,
                ),
            )
            database.execute(
                "INSERT INTO instant_finalize_reservation_assertions_v1(operation_id,"
                "organization_id,idempotency_key,asserted_at_ms) VALUES (?,?,?,?)",
                (retry_operation, ORG, "instant-finalize-001", NOW + 1),
            )

    expect_integrity(
        conflicting_http_retry,
        "frame_instant_finalize_reservation_v1",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM instant_finalize_operations_v1 WHERE operation_id=?",
            ("018f47a6-7b1c-7f55-8f39-8f8a8690f611",),
        ).fetchone()[0]
        == 0,
        "conflicting HTTP retry escaped its atomic reservation",
    )

    def revoked_writer_batch() -> None:
        with database:
            database.execute(
                "INSERT INTO cutover_repository_assertions_v1(id,satisfied) "
                "VALUES ('instant-finalize-revoked-writer',0)"
            )
            apply_publication_mutations(database)

    expect_integrity(revoked_writer_batch, "frame_cutover_authority_conflict_v1")
    require(
        database.execute(
            "SELECT state FROM instant_finalize_requests_v1 WHERE session_id=?", (SESSION,)
        ).fetchone()[0]
        == "pending",
        "revoked writer mutated publication state",
    )

    def deleted_video_batch() -> None:
        with database:
            database.execute(
                "UPDATE videos SET state='deleted',deleted_at_ms=? WHERE id=?",
                (NOW + 9, VIDEO),
            )
            apply_publication_mutations(database)

    expect_integrity(deleted_video_batch, "frame_instant_finalize_publication_v1")
    require(
        database.execute(
            "SELECT state,deleted_at_ms FROM videos WHERE id=?", (VIDEO,)
        ).fetchone()[1]
        is None,
        "deleted-video publication failure did not roll back",
    )

    expect_integrity(
        lambda: (
            database.execute("BEGIN"),
            apply_publication_mutations(database, update_job=False),
            database.commit(),
        ),
        "frame_instant_finalize_publication_v1",
    )
    database.rollback()
    require(
        database.execute(
            "SELECT state FROM instant_finalize_requests_v1 WHERE session_id=?", (SESSION,)
        ).fetchone()[0]
        == "pending",
        "zero-row job publication escaped the assertion rollback",
    )

    with database:
        apply_publication_mutations(database)

    published = database.execute(
        "SELECT r.state,r.publication_id,r.playable_object_key,r.distribution_eligible,j.state,"
        "o.result_state,o.publication_id,u.state,u.checksum_sha256,v.state,v.playback_object_key "
        "FROM instant_finalize_requests_v1 r JOIN instant_finalize_jobs_v1 j USING(session_id) "
        "JOIN instant_finalize_operations_v1 o USING(session_id) JOIN video_uploads u ON u.id=r.upload_id "
        "JOIN videos v ON v.id=r.video_id WHERE r.session_id=?",
        (SESSION,),
    ).fetchone()
    require(
        published
        == (
            "published",
            PUBLICATION_ID,
            OBJECT_KEY,
            1,
            "published",
            "published",
            PUBLICATION_ID,
            "complete",
            CHECKSUM,
            "ready",
            OBJECT_KEY,
        ),
        "Instant publication postcondition drifted",
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE instant_finalize_requests_v1 SET object_version=? WHERE session_id=?",
            ("ef" * 32, SESSION),
        ),
        "frame_instant_finalize_conflict_v1",
    )

    abort_key = f"tenants/{ORG}/videos/{VIDEO}/source/v2/abort-retry.mp4"
    insert_brokered_upload(
        database,
        ABORT_UPLOAD,
        abort_key,
        "abort-retry-fixture",
        created_at_ms=NOW - 2_000,
    )
    database.execute(
        "INSERT INTO r2_multipart_sessions_v1(upload_id,object_key,provider_upload_id,state,"
        "expected_bytes,checksum_sha256,content_type,correlation_id,created_at_ms,"
        "expires_at_ms,completed_at_ms) VALUES (?,?,?,'open',?,?,'video/mp4',?,?,?,NULL)",
        (
            ABORT_UPLOAD,
            abort_key,
            "provider-abort-retry",
            BYTES,
            CHECKSUM,
            ABORT_UPLOAD,
            NOW - 2_000,
            NOW - 1,
        ),
    )
    database.execute(
        "INSERT INTO r2_multipart_abort_reconciliation_v1(upload_id,state,attempt_count,"
        "next_attempt_at_ms,last_failure_class,started_at_ms,updated_at_ms,terminal_at_ms) "
        "VALUES (?,'pending',1,?,NULL,?,?,NULL)",
        (ABORT_UPLOAD, NOW, NOW, NOW),
    )
    database.execute(
        "UPDATE r2_multipart_abort_reconciliation_v1 SET next_attempt_at_ms=?,"
        "last_failure_class='unavailable',updated_at_ms=? WHERE upload_id=?",
        (NOW + 1_000, NOW + 1, ABORT_UPLOAD),
    )
    require(
        database.execute(
            "SELECT s.state,r.state,r.last_failure_class FROM r2_multipart_sessions_v1 s "
            "JOIN r2_multipart_abort_reconciliation_v1 r USING(upload_id) WHERE s.upload_id=?",
            (ABORT_UPLOAD,),
        ).fetchone()
        == ("open", "pending", "unavailable"),
        "retryable provider failure expired the multipart session",
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE r2_multipart_abort_reconciliation_v1 SET state='confirmed',"
            "next_attempt_at_ms=?,updated_at_ms=?,terminal_at_ms=? WHERE upload_id=?",
            (NOW + 2, NOW + 2, NOW + 2, ABORT_UPLOAD),
        ),
        "frame_r2_multipart_abort_reconciliation_v1",
    )
    with database:
        database.execute(
            "UPDATE r2_multipart_sessions_v1 SET state='expired' WHERE upload_id=? AND state='open'",
            (ABORT_UPLOAD,),
        )
        database.execute(
            "UPDATE r2_multipart_abort_reconciliation_v1 SET state='confirmed',"
            "next_attempt_at_ms=?,last_failure_class=NULL,updated_at_ms=?,terminal_at_ms=? "
            "WHERE upload_id=? AND state='pending'",
            (NOW + 2, NOW + 2, NOW + 2, ABORT_UPLOAD),
        )
        database.execute(
            "INSERT INTO r2_multipart_abort_terminal_assertions_v1(upload_id,outcome,"
            "asserted_at_ms) VALUES (?,'confirmed',?)",
            (ABORT_UPLOAD, NOW + 2),
        )

    # An authenticated DELETE takes ownership of an in-flight expiry intent
    # before touching R2. The provider can then succeed while the atomic D1
    # terminal batch fails or its acknowledgement is lost; the durable pending
    # row must remain claimable, and provider NotFound on retry is terminal.
    auth_abort_key = f"tenants/{ORG}/videos/{VIDEO}/source/v2/auth-abort-retry.mp4"
    insert_brokered_upload(
        database,
        AUTH_ABORT_UPLOAD,
        auth_abort_key,
        "authenticated-abort-retry-fixture",
        created_at_ms=NOW - 3_000,
    )
    database.execute(
        "UPDATE video_uploads SET state='uploading',updated_at_ms=?,revision=revision+1,"
        "event_sequence=event_sequence+1,event_fingerprint=? WHERE id=?",
        (
            NOW - 2_999,
            hashlib.sha256(b"authenticated-abort-uploading").hexdigest(),
            AUTH_ABORT_UPLOAD,
        ),
    )
    database.execute(
        "INSERT INTO r2_multipart_sessions_v1(upload_id,object_key,provider_upload_id,state,"
        "expected_bytes,checksum_sha256,content_type,correlation_id,created_at_ms,"
        "expires_at_ms,completed_at_ms) VALUES (?,?,?,'open',?,?,'video/mp4',?,?,?,NULL)",
        (
            AUTH_ABORT_UPLOAD,
            auth_abort_key,
            "provider-auth-abort-retry",
            BYTES,
            CHECKSUM,
            AUTH_ABORT_UPLOAD,
            NOW - 3_000,
            NOW - 1,
        ),
    )
    database.execute(
        "INSERT INTO r2_multipart_abort_reconciliation_v1(upload_id,intent_kind,state,"
        "attempt_count,next_attempt_at_ms,last_failure_class,started_at_ms,updated_at_ms,"
        "terminal_at_ms) VALUES (?,'expiry_cleanup','pending',1,?,NULL,?,?,NULL)",
        (AUTH_ABORT_UPLOAD, NOW + 300_000, NOW - 10, NOW - 10),
    )
    database.commit()

    first_claim_at = NOW + 10
    first_lock_until = first_claim_at + 60_000
    claim_authenticated_abort(
        database,
        upload_id=AUTH_ABORT_UPLOAD,
        operation_id=AUTH_ABORT_CLAIM_OPERATION,
        now_ms=first_claim_at,
        lock_until_ms=first_lock_until,
    )
    require(
        database.execute(
            "SELECT intent_kind,state,attempt_count,next_attempt_at_ms,last_failure_class "
            "FROM r2_multipart_abort_reconciliation_v1 WHERE upload_id=?",
            (AUTH_ABORT_UPLOAD,),
        ).fetchone()
        == ("authenticated_delete", "pending", 2, first_lock_until, None),
        "authenticated DELETE did not take over the expiry reconciliation atomically",
    )

    # A duplicate request cannot steal the live claim. Its zero-row mutation
    # trips the assertion and leaves the original attempt unchanged.
    expect_integrity(
        lambda: claim_authenticated_abort(
            database,
            upload_id=AUTH_ABORT_UPLOAD,
            operation_id=AUTH_ABORT_STALE_OPERATION,
            now_ms=first_claim_at + 1,
            lock_until_ms=first_lock_until + 1,
        ),
        "expected_count = actual_count",
    )
    require(
        database.execute(
            "SELECT attempt_count,next_attempt_at_ms FROM "
            "r2_multipart_abort_reconciliation_v1 WHERE upload_id=?",
            (AUTH_ABORT_UPLOAD,),
        ).fetchone()
        == (2, first_lock_until),
        "a duplicate authenticated abort stole the live provider attempt",
    )

    # Model provider success followed by a contingent D1 write matching zero
    # rows. The entire terminal batch—including the earlier session update—
    # must roll back so the scheduler can reconcile a lost acknowledgement.
    expect_integrity(
        lambda: finish_authenticated_abort(
            database,
            upload_id=AUTH_ABORT_UPLOAD,
            operation_id=AUTH_ABORT_FINISH_OPERATION,
            attempt=2,
            now_ms=first_claim_at + 2,
            match_video_row=False,
        ),
        "expected_count = actual_count",
    )
    require(
        database.execute(
            "SELECT session.state,upload.state,upload.revision,upload.event_sequence,"
            "reconciliation.state,reconciliation.attempt_count,terminal.upload_id "
            "FROM r2_multipart_sessions_v1 session "
            "JOIN video_uploads upload ON upload.id=session.upload_id "
            "JOIN r2_multipart_abort_reconciliation_v1 reconciliation USING(upload_id) "
            "LEFT JOIN r2_multipart_abort_terminal_assertions_v1 terminal USING(upload_id) "
            "WHERE session.upload_id=?",
            (AUTH_ABORT_UPLOAD,),
        ).fetchone()
        == ("open", "uploading", 1, 1, "pending", 2, None),
        "failed authenticated-abort D1 commit escaped its atomic rollback",
    )

    retry_at = first_lock_until + 1
    retry_lock_until = retry_at + 60_000
    claim_authenticated_abort(
        database,
        upload_id=AUTH_ABORT_UPLOAD,
        operation_id=AUTH_ABORT_RETRY_OPERATION,
        now_ms=retry_at,
        lock_until_ms=retry_lock_until,
    )
    # The retry observes provider NotFound after the first provider success;
    # NotFound is an authoritative terminal confirmation, not a retry failure.
    finish_authenticated_abort(
        database,
        upload_id=AUTH_ABORT_UPLOAD,
        operation_id=AUTH_ABORT_FINISH_OPERATION,
        attempt=3,
        now_ms=retry_at + 1,
    )
    require(
        database.execute(
            "SELECT session.state,upload.state,upload.revision,upload.event_sequence,"
            "reconciliation.intent_kind,reconciliation.state,reconciliation.attempt_count,"
            "terminal.outcome FROM r2_multipart_sessions_v1 session "
            "JOIN video_uploads upload ON upload.id=session.upload_id "
            "JOIN r2_multipart_abort_reconciliation_v1 reconciliation USING(upload_id) "
            "JOIN r2_multipart_abort_terminal_assertions_v1 terminal USING(upload_id) "
            "WHERE session.upload_id=?",
            (AUTH_ABORT_UPLOAD,),
        ).fetchone()
        == (
            "aborted",
            "aborted",
            2,
            2,
            "authenticated_delete",
            "confirmed",
            3,
            "confirmed",
        ),
        "provider NotFound retry did not converge the authenticated abort exactly once",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM r2_multipart_abort_batch_assertions_v1"
        ).fetchone()[0]
        == 0,
        "ephemeral authenticated-abort assertions escaped a successful batch",
    )

    # HEAD-present is the opposite terminal: preserve the completed object,
    # keep the product upload live, and hand the session back to completion
    # reconciliation. Starting in `completing` also proves that an asserted
    # same-state session transition remains a valid one-row D1 mutation.
    preserved_key = f"tenants/{ORG}/videos/{VIDEO}/source/v2/auth-abort-preserved.mp4"
    insert_brokered_upload(
        database,
        PRESERVED_ABORT_UPLOAD,
        preserved_key,
        "authenticated-abort-preserved-fixture",
    )
    database.execute(
        "INSERT INTO r2_multipart_sessions_v1(upload_id,object_key,provider_upload_id,state,"
        "expected_bytes,checksum_sha256,content_type,correlation_id,created_at_ms,"
        "expires_at_ms,completed_at_ms) VALUES (?,?,?,'completing',?,?,'video/mp4',?,?,?,NULL)",
        (
            PRESERVED_ABORT_UPLOAD,
            preserved_key,
            "provider-auth-abort-preserved",
            BYTES,
            CHECKSUM,
            PRESERVED_ABORT_UPLOAD,
            NOW,
            NOW + 86_400_000,
        ),
    )
    database.commit()
    preserved_claim_at = NOW + 70_000
    claim_authenticated_abort(
        database,
        upload_id=PRESERVED_ABORT_UPLOAD,
        operation_id=PRESERVED_ABORT_CLAIM_OPERATION,
        now_ms=preserved_claim_at,
        lock_until_ms=preserved_claim_at + 60_000,
    )
    finish_preserved_authenticated_abort(
        database,
        upload_id=PRESERVED_ABORT_UPLOAD,
        operation_id=PRESERVED_ABORT_FINISH_OPERATION,
        attempt=1,
        now_ms=preserved_claim_at + 1,
    )
    require(
        database.execute(
            "SELECT session.state,upload.state,upload.revision,upload.event_sequence,"
            "reconciliation.state,terminal.outcome FROM r2_multipart_sessions_v1 session "
            "JOIN video_uploads upload ON upload.id=session.upload_id "
            "JOIN r2_multipart_abort_reconciliation_v1 reconciliation USING(upload_id) "
            "JOIN r2_multipart_abort_terminal_assertions_v1 terminal USING(upload_id) "
            "WHERE session.upload_id=?",
            (PRESERVED_ABORT_UPLOAD,),
        ).fetchone()
        == ("completing", "initiated", 0, 0, "preserved_object", "preserved_object"),
        "HEAD-present authenticated abort did not preserve completion and product state",
    )

    scan_key_a = f"tenants/{ORG}/videos/{VIDEO}/source/v2/scan-a.mp4"
    scan_key_b = f"tenants/{ORG}/videos/{VIDEO}/source/v2/scan-b.mp4"
    insert_brokered_upload(database, SCAN_UPLOAD_A, scan_key_a, "scan-upload-a")
    insert_brokered_upload(database, SCAN_UPLOAD_B, scan_key_b, "scan-upload-b")
    scan_digest_a = hashlib.sha256(b"scan-request-a").hexdigest()
    scan_digest_b = hashlib.sha256(b"scan-request-b").hexdigest()
    insert_scan_request(
        database,
        session_id=SCAN_SESSION_A,
        upload_id=SCAN_UPLOAD_A,
        job_id=SCAN_JOB_A,
        request_sha256=scan_digest_a,
        created_at_ms=NOW - 10_000,
    )
    insert_scan_request(
        database,
        session_id=SCAN_SESSION_B,
        upload_id=SCAN_UPLOAD_B,
        job_id=SCAN_JOB_B,
        request_sha256=scan_digest_b,
        created_at_ms=NOW,
    )
    scan_sql = (
        "SELECT r.session_id FROM instant_finalize_requests_v1 r "
        "CROSS JOIN instant_finalize_scheduler_v1 scheduler "
        "WHERE scheduler.singleton=1 AND r.state='pending' AND r.next_attempt_at_ms<=? "
        "ORDER BY CASE WHEN scheduler.cursor_session_id IS NULL "
        "OR r.session_id>scheduler.cursor_session_id THEN 0 ELSE 1 END,r.session_id LIMIT 1"
    )
    database.execute(
        "UPDATE instant_finalize_scheduler_v1 SET cursor_session_id=?,updated_at_ms=? "
        "WHERE singleton=1",
        (SCAN_SESSION_A, NOW),
    )
    require(
        database.execute(scan_sql, (NOW,)).fetchone()[0] == SCAN_SESSION_B,
        "oldest blocked finalize request starved the next ring candidate",
    )
    database.execute(
        "UPDATE instant_finalize_scheduler_v1 SET cursor_session_id=?,updated_at_ms=? "
        "WHERE singleton=1",
        (SCAN_SESSION_B, NOW + 1),
    )
    require(
        database.execute(scan_sql, (NOW,)).fetchone()[0] == SCAN_SESSION_A,
        "finalize scheduler cursor did not wrap fairly",
    )

    database.execute(
        "INSERT INTO instant_finalize_jobs_v1(job_id,session_id,generation,request_sha256,state,"
        "created_at_ms,updated_at_ms) VALUES (?,?,1,?,'retained',?,?)",
        (SCAN_JOB_A, SCAN_SESSION_A, scan_digest_a, NOW, NOW),
    )
    database.execute(
        "INSERT INTO instant_finalize_operations_v1(operation_id,session_id,request_sha256,"
        "result_state,publication_id,committed_at_ms) VALUES (?,?,?,'pending',NULL,?)",
        (SCAN_OPERATION_A, SCAN_SESSION_A, scan_digest_a, NOW),
    )
    database.execute(
        "INSERT INTO instant_finalize_http_idempotency_v1(organization_id,idempotency_key,"
        "operation_id,session_id,request_sha256,job_id,created_at_ms) VALUES (?,?,?,?,?,?,?)",
        (
            ORG,
            "scan-dead-letter-001",
            SCAN_OPERATION_A,
            SCAN_SESSION_A,
            scan_digest_a,
            SCAN_JOB_A,
            NOW,
        ),
    )
    database.execute(
        "INSERT INTO instant_finalize_reservation_assertions_v1(operation_id,organization_id,"
        "idempotency_key,asserted_at_ms) VALUES (?,?,?,?)",
        (SCAN_OPERATION_A, ORG, "scan-dead-letter-001", NOW),
    )
    database.commit()

    def incomplete_dead_letter() -> None:
        with database:
            database.execute(
                "UPDATE instant_finalize_requests_v1 SET state='dead_letter',"
                "reconcile_attempt_count=1,last_failure_class='conflict',updated_at_ms=?,"
                "dead_lettered_at_ms=? WHERE session_id=? AND state='pending'",
                (NOW + 3, NOW + 3, SCAN_SESSION_A),
            )
            database.execute(
                "UPDATE instant_finalize_jobs_v1 SET state='cancelled',updated_at_ms=? "
                "WHERE session_id=?",
                (NOW + 3, SCAN_SESSION_A),
            )
            database.execute(
                "INSERT INTO instant_finalize_dead_letters_v1(session_id,organization_id,"
                "request_sha256,attempt_count,failure_class,created_at_ms) VALUES (?,?,?,1,'conflict',?)",
                (SCAN_SESSION_A, ORG, scan_digest_a, NOW + 3),
            )

    expect_integrity(incomplete_dead_letter, "frame_instant_finalize_dead_letter_v1")
    with database:
        database.execute(
            "UPDATE instant_finalize_requests_v1 SET state='dead_letter',"
            "reconcile_attempt_count=1,last_failure_class='conflict',updated_at_ms=?,"
            "dead_lettered_at_ms=? WHERE session_id=? AND state='pending'",
            (NOW + 3, NOW + 3, SCAN_SESSION_A),
        )
        database.execute(
            "UPDATE instant_finalize_jobs_v1 SET state='cancelled',updated_at_ms=? "
            "WHERE session_id=?",
            (NOW + 3, SCAN_SESSION_A),
        )
        database.execute(
            "UPDATE instant_finalize_operations_v1 SET result_state='dead_letter' "
            "WHERE session_id=? AND result_state='pending'",
            (SCAN_SESSION_A,),
        )
        database.execute(
            "INSERT INTO instant_finalize_dead_letters_v1(session_id,organization_id,"
            "request_sha256,attempt_count,failure_class,created_at_ms) VALUES (?,?,?,1,'conflict',?)",
            (SCAN_SESSION_A, ORG, scan_digest_a, NOW + 3),
        )

    require(not database.execute("PRAGMA foreign_key_check").fetchall(), "foreign-key drift")
    return {
        "schema_version": 1,
        "status": "passed",
        "migration": "0024_instant_finalize_runtime.sql",
        "checks": {
            "multipart_geometry_rejected": True,
            "verified_object_receipt_immutable": True,
            "cross_tenant_finalize_rejected": True,
            "http_idempotency_atomic": True,
            "revoked_authority_rolled_back": True,
            "deleted_video_rolled_back": True,
            "contingent_rows_asserted": True,
            "publication_postcondition": True,
            "multipart_abort_retry_retained": True,
            "authenticated_abort_takeover_atomic": True,
            "authenticated_abort_lost_ack_reconciled": True,
            "authenticated_abort_not_found_terminal": True,
            "authenticated_abort_exactly_once": True,
            "authenticated_abort_completed_object_preserved": True,
            "fair_bounded_scan": True,
            "dead_letter_asserted": True,
        },
        "protected_gates": ["hosted_r2", "hosted_d1_contention", "browser_playback"],
    }


if __name__ == "__main__":
    print(json.dumps(run(), sort_keys=True, separators=(",", ":")))
