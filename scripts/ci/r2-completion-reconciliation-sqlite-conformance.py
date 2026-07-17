#!/usr/bin/env python3
"""Offline fairness and terminal-state proof for R2 completion reconciliation."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import sqlite3
import sys
from collections.abc import Callable
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
DIRECT_CONFORMANCE = ROOT / "scripts" / "ci" / "direct-upload-sqlite-conformance.py"
RUNTIME = ROOT / "apps" / "control-plane" / "src" / "r2_multipart.rs"
MIGRATION = MIGRATIONS / "0031_r2_completion_reconciliation.sql"
CONTRACT_MIGRATION = (
    ROOT / "apps" / "control-plane" / "contract-migrations"
    / "0033_r2_multipart_claims_enforce.sql"
)
NOW = 1_700_900_000_000
ORG = "018f47a6-7b1c-7f55-8f39-8f8a8690b901"
OWNER = "018f47a6-7b1c-7f55-8f39-8f8a8690a901"
VIDEO = "018f47a6-7b1c-7f55-8f39-8f8a8690c901"
INTEGRATION = "018f47a6-7b1c-7f55-8f39-8f8a8690f901"
POISON = "018f47a6-7b1c-7f55-8f39-8f8a8690d901"
HEALTHY = "018f47a6-7b1c-7f55-8f39-8f8a8690d902"
RECOVERED = "018f47a6-7b1c-7f55-8f39-8f8a8690d903"
LEGACY_AFTER_EXPAND = "018f47a6-7b1c-7f55-8f39-8f8a8690d904"
LEGACY_WITH_ACTIVE_CLAIM = "018f47a6-7b1c-7f55-8f39-8f8a8690d905"
BACKFILL_WITHOUT_CLAIM = "018f47a6-7b1c-7f55-8f39-8f8a8690d906"
BACKFILL_WITH_ACTIVE_CLAIM = "018f47a6-7b1c-7f55-8f39-8f8a8690d907"
BACKFILL_INVALID_COMPLETION = "018f47a6-7b1c-7f55-8f39-8f8a8690d908"
CHECKSUM = "ab" * 32
PARTS_DIGEST = "cd" * 32
EXPECTED_BYTES = 10 * 1024 * 1024
PART_SIZE = 5 * 1024 * 1024
MAX_ATTEMPTS = 12

CANDIDATE_SQL = (
    "SELECT reconciliation.upload_id,reconciliation.attempt_count "
    "FROM r2_multipart_completion_reconciliation_v1 reconciliation "
    "JOIN r2_multipart_sessions_v1 session USING(upload_id) "
    "WHERE reconciliation.state='pending' "
    "AND reconciliation.next_attempt_at_ms<=? "
    "AND session.state='completing' "
    "ORDER BY reconciliation.next_attempt_at_ms,session.created_at_ms,"
    "reconciliation.upload_id LIMIT 1"
)


class ConformanceFailure(RuntimeError):
    """Stable local-contract failure."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ConformanceFailure(message)


def expect_integrity(operation: Callable[[], object], marker: str) -> None:
    try:
        operation()
    except sqlite3.IntegrityError as error:
        require(marker in str(error), "completion journal returned the wrong fence")
    else:
        raise ConformanceFailure("completion journal accepted an invalid mutation")


def load_direct_conformance() -> Any:
    specification = importlib.util.spec_from_file_location(
        "frame_direct_upload_for_completion_reconciliation", DIRECT_CONFORMANCE
    )
    if specification is None or specification.loader is None:
        raise ConformanceFailure("direct-upload SQLite fixture is unavailable")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def migrate(database: sqlite3.Connection, direct: Any, through: int = 31) -> None:
    direct.migrate(database)
    later = [
        path
        for path in sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
        if 24 <= int(path.name[:4]) <= through
    ]
    require(
        [int(path.name[:4]) for path in later] == list(range(24, through + 1)),
        "R2 completion migration sequence is not contiguous",
    )
    for path in later:
        database.executescript(path.read_text(encoding="utf-8"))


def operation(number: int) -> str:
    return f"018f47a6-7b1c-7f55-8f39-{number:012d}"


def seed_authority(database: sqlite3.Connection) -> None:
    database.execute(
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) "
        "VALUES (?,?,'R2 completion owner',?,?)",
        (OWNER, "r2-completion@sqlite.invalid", NOW - 10_000, NOW - 10_000),
    )
    database.execute(
        "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,"
        "updated_at_ms,tombstoned_at_ms,revision,authority_version,retention_until_ms,"
        "recovered_at_ms,last_operation_id) "
        "VALUES (?,?,?,'active','{}',?,?,NULL,0,0,NULL,NULL,?)",
        (ORG, OWNER, "R2 Completion", NOW - 9_000, NOW - 9_000, operation(901)),
    )
    database.execute(
        "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,"
        "created_at_ms,updated_at_ms,revision,authority_version,last_operation_id) "
        "VALUES (?,?,'owner','active',0,?,?,0,0,?)",
        (ORG, OWNER, NOW - 8_000, NOW - 8_000, operation(902)),
    )
    document = json.dumps(
        {"schema_version": 1, "title": "R2 completion"},
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
            OWNER,
            "R2 completion",
            NOW - 7_000,
            NOW - 7_000,
            ORG,
            "private",
            document,
            hashlib.sha256(document.encode()).hexdigest(),
            operation(903),
        ),
    )
    capabilities = json.dumps(
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
            INTEGRATION,
            ORG,
            OWNER,
            capabilities,
            NOW - 6_000,
            NOW - 6_000,
            hashlib.sha256(capabilities.encode()).hexdigest(),
        ),
    )


def seed_completing_session(
    database: sqlite3.Connection,
    upload_id: str,
    ordinal: int,
) -> None:
    created_at = NOW + ordinal
    expires_at = NOW + 3_600_000
    object_key = f"tenants/{ORG}/videos/{VIDEO}/source/v{ordinal}/payload.mp4"
    database.execute(
        "INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,"
        "received_bytes,idempotency_key,source_object_key,source_version,content_type,"
        "checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,"
        "event_fingerprint,transfer_mode,direct_staging_key,direct_checksum_sha256,"
        "direct_expires_at_ms) VALUES (?,?,?,'initiated',?,0,?,?,?,'video/mp4',NULL,"
        "?,?,0,0,?,'brokered',NULL,NULL,NULL)",
        (
            upload_id,
            ORG,
            VIDEO,
            EXPECTED_BYTES,
            f"r2-completion-{ordinal}",
            object_key,
            ordinal,
            created_at,
            created_at,
            "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9",
        ),
    )
    database.execute(
        "INSERT INTO r2_multipart_intents_v1(upload_id,integration_id,checksum_sha256,"
        "part_size,part_count,expires_at_ms,created_at_ms) VALUES (?,?,?,?,2,?,?)",
        (upload_id, INTEGRATION, CHECKSUM, PART_SIZE, expires_at, created_at),
    )
    claim = operation(910 + ordinal)
    database.execute(
        "INSERT INTO r2_multipart_creation_claims_v1(upload_id,organization_id,object_key,"
        "expected_bytes,checksum_sha256,content_type,correlation_id,part_size,part_count,"
        "expires_at_ms,claim_token,state,provider_upload_id,created_at_ms,updated_at_ms) "
        "VALUES (?,?,?,?,?,'video/mp4',?,?,2,?,?,'reserved',NULL,?,?)",
        (
            upload_id,
            ORG,
            object_key,
            EXPECTED_BYTES,
            CHECKSUM,
            upload_id,
            PART_SIZE,
            expires_at,
            claim,
            created_at,
            created_at,
        ),
    )
    database.execute(
        "UPDATE r2_multipart_creation_claims_v1 SET state='provider_bound',"
        "provider_upload_id=?,updated_at_ms=? WHERE upload_id=?",
        (f"provider-{ordinal}", created_at + 1, upload_id),
    )
    database.execute(
        "INSERT INTO r2_multipart_sessions_v1(upload_id,object_key,provider_upload_id,state,"
        "expected_bytes,checksum_sha256,content_type,correlation_id,created_at_ms,"
        "expires_at_ms,completed_at_ms) VALUES (?,?,?,'open',?,?,'video/mp4',?,?,?,NULL)",
        (
            upload_id,
            object_key,
            f"provider-{ordinal}",
            EXPECTED_BYTES,
            CHECKSUM,
            upload_id,
            created_at,
            expires_at,
        ),
    )
    database.execute(
        "UPDATE r2_multipart_creation_claims_v1 SET state='committed',updated_at_ms=? "
        "WHERE upload_id=?",
        (created_at + 2, upload_id),
    )
    database.execute(
        "INSERT INTO r2_multipart_completion_claims_v1(upload_id,request_parts_sha256,"
        "claim_token,state,attempt_count,claimed_at_ms,lease_expires_at_ms,completed_at_ms) "
        "VALUES (?,?,?,'active',1,?,?,NULL)",
        (
            upload_id,
            PARTS_DIGEST,
            operation(930 + ordinal),
            created_at + 10,
            NOW + 60_000,
        ),
    )


def seed_n_minus_one_completing_session(
    database: sqlite3.Connection,
    upload_id: str,
    ordinal: int,
) -> int:
    created_at = NOW + ordinal
    expires_at = NOW + 3_600_000
    object_key = f"tenants/{ORG}/videos/{VIDEO}/source/v{ordinal}/n-minus-one.mp4"
    database.execute(
        "INSERT INTO video_uploads(id,organization_id,video_id,state,expected_bytes,"
        "received_bytes,idempotency_key,source_object_key,source_version,content_type,"
        "checksum_sha256,created_at_ms,updated_at_ms,revision,event_sequence,"
        "event_fingerprint,transfer_mode,direct_staging_key,direct_checksum_sha256,"
        "direct_expires_at_ms) VALUES (?,?,?,'initiated',?,0,?,?,?,'video/mp4',NULL,"
        "?,?,0,0,?,'brokered',NULL,NULL,NULL)",
        (
            upload_id,
            ORG,
            VIDEO,
            EXPECTED_BYTES,
            f"r2-completion-n-minus-one-{ordinal}",
            object_key,
            ordinal,
            created_at,
            created_at,
            "daf2d49bd689dfe48d2c4e168137808de05d76d9766c3cb98ab5da27e7c378b9",
        ),
    )
    database.execute(
        "INSERT INTO r2_multipart_intents_v1(upload_id,integration_id,checksum_sha256,"
        "part_size,part_count,expires_at_ms,created_at_ms) VALUES (?,?,?,?,2,?,?)",
        (
            upload_id,
            INTEGRATION,
            CHECKSUM,
            PART_SIZE,
            expires_at,
            created_at,
        ),
    )
    database.execute(
        "INSERT INTO r2_multipart_sessions_v1(upload_id,object_key,provider_upload_id,state,"
        "expected_bytes,checksum_sha256,content_type,correlation_id,created_at_ms,"
        "expires_at_ms,completed_at_ms) VALUES (?,?,?,'open',?,?,'video/mp4',?,?,?,NULL)",
        (
            upload_id,
            object_key,
            f"provider-n-minus-one-{ordinal}",
            EXPECTED_BYTES,
            CHECKSUM,
            upload_id,
            created_at,
            expires_at,
        ),
    )
    database.execute(
        "UPDATE r2_multipart_sessions_v1 SET state='completing' WHERE upload_id=?",
        (upload_id,),
    )
    return created_at


def finish_n_minus_one_completion(
    database: sqlite3.Connection,
    upload_id: str,
    ordinal: int,
    created_at: int,
    *,
    active_claim: bool,
    completion_checksum: str = CHECKSUM,
) -> str | None:
    claim_token = operation(970 + ordinal) if active_claim else None
    if claim_token is not None:
        database.execute(
            "INSERT INTO r2_multipart_completion_claims_v1(upload_id,request_parts_sha256,"
            "claim_token,state,attempt_count,claimed_at_ms,lease_expires_at_ms,completed_at_ms) "
            "VALUES (?,?,?,'active',1,?,?,NULL)",
            (
                upload_id,
                PARTS_DIGEST,
                claim_token,
                created_at + 10,
                created_at + 60_000,
            ),
        )
    completed_at = created_at + 20
    database.execute(
        "INSERT INTO r2_multipart_completions_v1(upload_id,request_parts_sha256,"
        "provider_version,provider_etag,bytes,checksum_sha256,content_type,container,"
        "video_codec,audio_codec,width,height,duration_ms,frame_rate_millihertz,"
        "completed_at_ms,correlation_id) VALUES (?,?,?, ?,?,?,'video/mp4','mp4','h264',"
        "'aac',1920,1080,60000,30000,?,?)",
        (
            upload_id,
            PARTS_DIGEST,
            f"legacy-provider-version-{ordinal}",
            f"legacy-provider-etag-{ordinal}",
            EXPECTED_BYTES,
            completion_checksum,
            completed_at,
            upload_id,
        ),
    )
    database.execute(
        "UPDATE r2_multipart_sessions_v1 SET state='complete',completed_at_ms=? "
        "WHERE upload_id=? AND state='completing'",
        (completed_at, upload_id),
    )
    return claim_token


def verify_preexisting_completion_backfill(direct: Any) -> None:
    database = sqlite3.connect(":memory:")
    try:
        database.execute("PRAGMA foreign_keys = ON")
        migrate(database, direct, through=30)
        seed_authority(database)

        without_claim_at = seed_n_minus_one_completing_session(
            database, BACKFILL_WITHOUT_CLAIM, 6
        )
        finish_n_minus_one_completion(
            database,
            BACKFILL_WITHOUT_CLAIM,
            6,
            without_claim_at,
            active_claim=False,
        )
        with_claim_at = seed_n_minus_one_completing_session(
            database, BACKFILL_WITH_ACTIVE_CLAIM, 7
        )
        active_claim_token = finish_n_minus_one_completion(
            database,
            BACKFILL_WITH_ACTIVE_CLAIM,
            7,
            with_claim_at,
            active_claim=True,
        )
        invalid_at = seed_n_minus_one_completing_session(
            database, BACKFILL_INVALID_COMPLETION, 8
        )
        finish_n_minus_one_completion(
            database,
            BACKFILL_INVALID_COMPLETION,
            8,
            invalid_at,
            active_claim=False,
            completion_checksum="ef" * 32,
        )
        require(
            database.execute(
                "SELECT state FROM r2_multipart_completion_claims_v1 WHERE upload_id=?",
                (BACKFILL_WITH_ACTIVE_CLAIM,),
            ).fetchone()
            == ("active",),
            "pre-0031 active-claim fixture did not remain ambiguous",
        )

        database.executescript(MIGRATION.read_text(encoding="utf-8"))
        require(
            database.execute(
                "SELECT state FROM r2_multipart_completion_reconciliation_v1 "
                "WHERE upload_id=?",
                (BACKFILL_WITHOUT_CLAIM,),
            ).fetchone()
            == ("complete",),
            "0031 did not backfill an authoritative claim-free N-1 completion",
        )
        require(active_claim_token is not None, "backfill active claim token is missing")
        require(
            database.execute(
                "SELECT reconciliation.state,claim.state,completion.completion_claim_token "
                "FROM r2_multipart_completion_reconciliation_v1 reconciliation "
                "JOIN r2_multipart_completion_claims_v1 claim USING(upload_id) "
                "JOIN r2_multipart_completions_v1 completion USING(upload_id) "
                "WHERE reconciliation.upload_id=?",
                (BACKFILL_WITH_ACTIVE_CLAIM,),
            ).fetchone()
            == ("complete", "complete", active_claim_token),
            "0031 backfill stranded a matching pre-existing active claim",
        )
        require(
            database.execute(
                "SELECT state,attempt_count,last_failure_class,terminal_at_ms IS NOT NULL "
                "FROM r2_multipart_completion_reconciliation_v1 WHERE upload_id=?",
                (BACKFILL_INVALID_COMPLETION,),
            ).fetchone()
            == ("quarantined", 1, "integrity", 1),
            "0031 backfill trusted an invalid N-1 completion",
        )
        database.rollback()
    finally:
        database.close()


def candidate(database: sqlite3.Connection, at_ms: int) -> str | None:
    row = database.execute(CANDIDATE_SQL, (at_ms,)).fetchone()
    return None if row is None else str(row[0])


def acquire(database: sqlite3.Connection, upload_id: str, at_ms: int) -> int:
    row = database.execute(
        "UPDATE r2_multipart_completion_reconciliation_v1 "
        "SET attempt_count=attempt_count+1,next_attempt_at_ms=?,last_failure_class=NULL,"
        "updated_at_ms=? WHERE upload_id=? AND state='pending' "
        "AND next_attempt_at_ms<=? AND attempt_count<? RETURNING attempt_count",
        (at_ms + 900_000, at_ms, upload_id, at_ms, MAX_ATTEMPTS),
    ).fetchone()
    require(row is not None, "due completion reconciliation was not acquired")
    return int(row[0])


def run() -> dict[str, object]:
    source = RUNTIME.read_text(encoding="utf-8")
    migration = MIGRATION.read_text(encoding="utf-8")
    for marker in (
        "r2_multipart_completion_reconciliation_v1",
        "begin_completion_reconciliation_attempt",
        "retain_completion_reconciliation_failure",
        "quarantine_exhausted_completion_reconciliation",
        "completion_failure_is_terminal",
        "MAX_COMPLETION_RECONCILIATION_ATTEMPTS",
    ):
        require(marker in source, "R2 completion runtime surface drifted")
    for marker in (
        "state IN ('pending', 'quarantined', 'complete')",
        "attempt_count BETWEEN 0 AND 12",
        "r2_multipart_completion_reconciliation_v1_due_idx",
        "r2_multipart_completion_reconciliation_v1_session_completing",
        "r2_multipart_completion_reconciliation_v1_session_complete",
        "r2_multipart_completion_reconciliation_v1_legacy_claim_promoted",
        "frame_r2_completion_reconciliation_v1",
    ):
        require(marker in migration, "R2 completion migration surface drifted")

    direct = load_direct_conformance()
    verify_preexisting_completion_backfill(direct)
    database = sqlite3.connect(":memory:")
    try:
        database.execute("PRAGMA foreign_keys = ON")
        migrate(database, direct)
        seed_authority(database)
        legacy_due = seed_n_minus_one_completing_session(
            database, LEGACY_AFTER_EXPAND, 4
        )
        require(
            candidate(database, legacy_due) == LEGACY_AFTER_EXPAND,
            "post-0031 N-1 completing write was invisible to the scheduler",
        )
        require(
            finish_n_minus_one_completion(
                database,
                LEGACY_AFTER_EXPAND,
                4,
                legacy_due,
                active_claim=False,
            )
            is None,
            "claim-free legacy completion unexpectedly created a claim",
        )
        require(
            database.execute(
                "SELECT reconciliation.state,reconciliation.attempt_count,"
                "completion.completion_claim_token "
                "FROM r2_multipart_completion_reconciliation_v1 reconciliation "
                "JOIN r2_multipart_completions_v1 completion USING(upload_id) "
                "WHERE reconciliation.upload_id=?",
                (LEGACY_AFTER_EXPAND,),
            ).fetchone()
            == ("complete", 0, None),
            "claim-free N-1 completion did not terminalize its journal",
        )

        legacy_claim_due = seed_n_minus_one_completing_session(
            database, LEGACY_WITH_ACTIVE_CLAIM, 5
        )
        legacy_claim_token = finish_n_minus_one_completion(
            database,
            LEGACY_WITH_ACTIVE_CLAIM,
            5,
            legacy_claim_due,
            active_claim=True,
        )
        require(legacy_claim_token is not None, "legacy active-claim fixture is missing")
        require(
            database.execute(
                "SELECT reconciliation.state,claim.state,completion.completion_claim_token "
                "FROM r2_multipart_completion_reconciliation_v1 reconciliation "
                "JOIN r2_multipart_completion_claims_v1 claim USING(upload_id) "
                "JOIN r2_multipart_completions_v1 completion USING(upload_id) "
                "WHERE reconciliation.upload_id=?",
                (LEGACY_WITH_ACTIVE_CLAIM,),
            ).fetchone()
            == ("complete", "complete", legacy_claim_token),
            "N-1 completion stranded its matching concurrent active claim",
        )
        database.executescript(CONTRACT_MIGRATION.read_text(encoding="utf-8"))
        seed_completing_session(database, POISON, 1)
        seed_completing_session(database, HEALTHY, 2)
        seed_completing_session(database, RECOVERED, 3)
        due = NOW + 60_001
        require(candidate(database, due) == POISON, "oldest completion was not selected")

        poison_attempt = acquire(database, POISON, due)
        database.execute(
            "UPDATE r2_multipart_completion_reconciliation_v1 "
            "SET state='quarantined',next_attempt_at_ms=?,"
            "last_failure_class='integrity',updated_at_ms=?,terminal_at_ms=? "
            "WHERE upload_id=? AND state='pending' AND attempt_count=? "
            "AND last_failure_class IS NULL",
            (due, due, due, POISON, poison_attempt),
        )
        require(
            database.execute(
                "SELECT state,attempt_count,last_failure_class,terminal_at_ms "
                "FROM r2_multipart_completion_reconciliation_v1 WHERE upload_id=?",
                (POISON,),
            ).fetchone()
            == ("quarantined", 1, "integrity", due),
            "permanent completion failure did not reach stable quarantine",
        )
        expect_integrity(
            lambda: database.execute(
                "UPDATE r2_multipart_completion_reconciliation_v1 "
                "SET state='pending',terminal_at_ms=NULL WHERE upload_id=?",
                (POISON,),
            ),
            "frame_r2_completion_reconciliation_v1",
        )
        expect_integrity(
            lambda: database.execute(
                "DELETE FROM r2_multipart_completion_reconciliation_v1 WHERE upload_id=?",
                (POISON,),
            ),
            "frame_r2_completion_reconciliation_v1",
        )
        require(
            candidate(database, due) == HEALTHY,
            "quarantined oldest completion still starved the later row",
        )

        # A provider completion timestamp can predate the scheduler attempt
        # that recovers a lost acknowledgement. The claim-complete side effect
        # must preserve the newer journal clock instead of rolling it backward.
        recovered_attempt = acquire(database, RECOVERED, due)
        require(recovered_attempt == 1, "recovered completion attempt drifted")
        completed_at = NOW + 30
        completion_claim = database.execute(
            "SELECT claim_token FROM r2_multipart_completion_claims_v1 WHERE upload_id=?",
            (RECOVERED,),
        ).fetchone()
        require(completion_claim is not None, "recovered completion claim is missing")
        database.execute(
            "INSERT INTO r2_multipart_completions_v1(upload_id,request_parts_sha256,"
            "provider_version,provider_etag,bytes,checksum_sha256,content_type,container,"
            "video_codec,audio_codec,width,height,duration_ms,frame_rate_millihertz,"
            "completed_at_ms,correlation_id,completion_claim_token) "
            "VALUES (?,?,'provider-version','provider-etag',?,?,'video/mp4','mp4','h264',"
            "'aac',1920,1080,60000,30000,?,?,?)",
            (
                RECOVERED,
                PARTS_DIGEST,
                EXPECTED_BYTES,
                CHECKSUM,
                completed_at,
                RECOVERED,
                completion_claim[0],
            ),
        )
        database.execute(
            "UPDATE r2_multipart_sessions_v1 SET state='complete',completed_at_ms=? "
            "WHERE upload_id=? AND state='completing'",
            (completed_at, RECOVERED),
        )
        database.execute(
            "UPDATE r2_multipart_completion_claims_v1 SET state='complete',completed_at_ms=? "
            "WHERE upload_id=? AND state='active'",
            (completed_at, RECOVERED),
        )
        require(
            database.execute(
                "SELECT state,attempt_count,updated_at_ms,terminal_at_ms "
                "FROM r2_multipart_completion_reconciliation_v1 WHERE upload_id=?",
                (RECOVERED,),
            ).fetchone()
            == ("complete", 1, due, due),
            "lost-ack completion rolled the reconciliation journal clock backward",
        )

        healthy_attempt = acquire(database, HEALTHY, due)
        retry_at = due + 900_000
        database.execute(
            "UPDATE r2_multipart_completion_reconciliation_v1 "
            "SET next_attempt_at_ms=?,last_failure_class='unavailable',updated_at_ms=? "
            "WHERE upload_id=? AND state='pending' AND attempt_count=? "
            "AND last_failure_class IS NULL",
            (retry_at, due + 1, HEALTHY, healthy_attempt),
        )
        require(candidate(database, retry_at - 1) is None, "retry backoff was bypassed")
        require(candidate(database, retry_at) == HEALTHY, "retry did not become visible when due")

        current = retry_at
        for expected_attempt in range(2, MAX_ATTEMPTS + 1):
            attempt = acquire(database, HEALTHY, current)
            require(attempt == expected_attempt, "completion retry attempt count drifted")
            if attempt == MAX_ATTEMPTS:
                # Model a Worker death after acquiring the final attempt but
                # before it can retain a failure. Once the lease expires the
                # scheduler terminalizes the exhausted row instead of hiding
                # it forever behind the attempt ceiling.
                current += 900_000
                require(
                    candidate(database, current) == HEALTHY,
                    "expired final-attempt lease was not selectable",
                )
                database.execute(
                    "UPDATE r2_multipart_completion_reconciliation_v1 "
                    "SET state='quarantined',next_attempt_at_ms=?,"
                    "last_failure_class='unavailable',updated_at_ms=?,terminal_at_ms=? "
                    "WHERE upload_id=? AND state='pending' AND attempt_count=? "
                    "AND next_attempt_at_ms<=?",
                    (current, current, current, HEALTHY, attempt, current),
                )
            else:
                current += 900_000
                database.execute(
                    "UPDATE r2_multipart_completion_reconciliation_v1 "
                    "SET next_attempt_at_ms=?,last_failure_class='unavailable',updated_at_ms=? "
                    "WHERE upload_id=? AND state='pending' AND attempt_count=? "
                    "AND last_failure_class IS NULL",
                    (current, current - 899_999, HEALTHY, attempt),
                )
        require(
            database.execute(
                "SELECT state,attempt_count,last_failure_class,terminal_at_ms "
                "FROM r2_multipart_completion_reconciliation_v1 WHERE upload_id=?",
                (HEALTHY,),
            ).fetchone()
            == ("quarantined", MAX_ATTEMPTS, "unavailable", current),
            "retryable completion failure did not stop at the bounded attempt ceiling",
        )
        require(candidate(database, current + 1) is None, "terminal rows remained selectable")
        database.rollback()
    finally:
        database.close()
    return {
        "migration": MIGRATION.name,
        "permanent_failure_quarantined": True,
        "later_row_selected_after_quarantine": True,
        "retry_attempt_ceiling": MAX_ATTEMPTS,
        "retry_backoff_enforced": True,
        "expired_final_attempt_terminalized": True,
        "lost_ack_clock_monotonic": True,
        "post_expand_n_minus_one_scheduler_eligible": True,
        "post_expand_n_minus_one_completion_terminalized": True,
        "matching_active_claim_not_stranded": True,
        "preexisting_completion_backfill_validated": True,
    }


def main() -> int:
    try:
        result = run()
    except (ConformanceFailure, OSError, sqlite3.Error, ValueError) as error:
        print(f"R2 completion reconciliation conformance failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
