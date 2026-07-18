#!/usr/bin/env python3
"""Adversarial SQLite proof for dense native media-job source authority."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import pathlib
import sqlite3
import tempfile
from collections.abc import Callable


ROOT = pathlib.Path(__file__).resolve().parents[2]
DIRECT = ROOT / "scripts" / "ci" / "direct-upload-sqlite-conformance.py"
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
CONTRACT_MIGRATIONS = ROOT / "apps" / "control-plane" / "contract-migrations"
NOW = 1_700_700_000_000
VIDEO_B = "018f47a6-7b1c-7f55-8f39-8f8a8690c602"
JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d701"
PARTIAL_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d702"
SINGLE_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d703"
WRONG_PROFILE_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d704"
REPEATED_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d705"
DUPLICATE_SEGMENT_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d706"
AUTHORITY_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d707"
RACE_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d708"
LEGACY_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d709"
LEGACY_EXPLICIT_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d710"
LEGACY_REMOTE_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d711"
LEGACY_REJECTED_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d712"
LEGACY_LEASED_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d713"
LEGACY_RUNNING_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d714"
EXPAND_WINDOW_JOB = "018f47a6-7b1c-7f55-8f39-8f8a8690d715"
CHECKSUM_A = "ab" * 32
CHECKSUM_B = "cd" * 32
LEGACY_LEASE_DIGEST = "34" * 32


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
    specification = importlib.util.spec_from_file_location("frame_direct_upload", DIRECT)
    require(
        specification is not None and specification.loader is not None,
        "direct-upload fixture helper is unavailable",
    )
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def migrate(database: sqlite3.Connection, through: int = 27) -> None:
    files = [
        path
        for path in sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
        if int(path.name[:4]) <= through
    ]
    require(
        [int(path.name[:4]) for path in files] == list(range(1, through + 1)),
        f"migration sequence through {through:04d} is not contiguous",
    )
    database.execute("PRAGMA foreign_keys = ON")
    for path in files:
        database.executescript(path.read_text(encoding="utf-8"))


def apply_migration_0027(database: sqlite3.Connection) -> None:
    path = MIGRATIONS / "0027_media_job_inputs.sql"
    database.executescript(path.read_text(encoding="utf-8"))


def apply_contract_0032(database: sqlite3.Connection) -> None:
    path = CONTRACT_MIGRATIONS / "0032_media_job_inputs_enforce.sql"
    database.executescript(path.read_text(encoding="utf-8"))


def payload(profile: str, inputs: list[tuple[str, int]], primary: tuple[str, int]) -> str:
    return json.dumps(
        {
            "schema_version": 1,
            "tenant_id": direct.ORG,
            "video_id": primary[0],
            "source_version": primary[1],
            "source_inputs": [
                {"video_id": video_id, "source_version": version}
                for video_id, version in inputs
            ],
            "profile": profile,
        },
        sort_keys=True,
        separators=(",", ":"),
    )


def legacy_payload(*, explicit_singleton: bool) -> str:
    document: dict[str, object] = {
        "schema_version": 1,
        "tenant_id": direct.ORG,
        "video_id": direct.VIDEO,
        "source_version": 1,
        "profile": "normalize_v1",
    }
    if explicit_singleton:
        document["source_inputs"] = [
            {"video_id": direct.VIDEO, "source_version": 1}
        ]
    return json.dumps(document, separators=(",", ":"))


def add_pre_migration_job(
    database: sqlite3.Connection,
    job_id: str,
    *,
    executor: str,
    explicit_singleton: bool,
    state: str = "queued",
    attempt: int = 0,
    revision: int = 0,
) -> None:
    digest = hashlib.sha256(f"legacy:{job_id}".encode()).hexdigest()
    database.execute(
        "INSERT INTO media_jobs(id,video_id,kind,state,idempotency_key,attempt,payload_json,"
        "created_at_ms,updated_at_ms,organization_id,selected_executor,source_version,"
        "profile_version,output_object_key,cancel_requested,revision,worker_id,"
        "lease_token_digest,lease_expires_at_ms,heartbeat_at_ms,progress_basis_points) "
        "VALUES (?1,?2,'normalize',?3,?4,?5,?6,?7,?8,?9,?10,1,1,?11,0,?12,"
        "?13,?14,?15,?16,?17)",
        (
            job_id,
            direct.VIDEO,
            state,
            hashlib.sha256(job_id.encode()).hexdigest(),
            attempt,
            legacy_payload(explicit_singleton=explicit_singleton),
            NOW - 100,
            NOW - 100,
            direct.ORG,
            executor,
            f"tenants/{direct.ORG}/videos/{direct.VIDEO}/derivatives/normalize_v1/{digest}",
            revision,
            direct.USER if state in {"leased", "running"} else None,
            LEGACY_LEASE_DIGEST if state in {"leased", "running"} else None,
            NOW + 60_000 if state in {"leased", "running"} else None,
            NOW - 50 if state in {"leased", "running"} else None,
            2_500 if state == "running" else 0 if state == "leased" else None,
        ),
    )
    if state in {"leased", "running"}:
        database.execute(
            "INSERT INTO media_job_attempts(job_id,attempt,executor,worker_id,started_at_ms) "
            "VALUES (?,?,'native_gstreamer',?,?)",
            (job_id, attempt, direct.USER, NOW - 75),
        )


def add_pre_migration_execution(
    database: sqlite3.Connection, job_id: str, *, state: str = "queued"
) -> None:
    object_key = (
        f"tenants/{direct.ORG}/videos/{direct.VIDEO}/source/v1/payload.webm"
    )
    database.execute(
        "INSERT OR IGNORE INTO media_source_probes_v1(organization_id,video_id,source_version,"
        "source_object_key,source_checksum_sha256,source_bytes,source_content_type,container,"
        "video_codec,audio_codec,duration_ms,width,height,frame_rate_numerator,"
        "frame_rate_denominator,decoded_bytes_upper_bound,frame_count_upper_bound,track_count,"
        "probe_contract_version,probe_digest,trust,state,verified_at_ms,updated_at_ms) "
        "VALUES (?,?,1,?,?,1024,'video/webm','webm','vp9','opus',2000,640,360,30,1,"
        "1000000,60,2,1,?,'verified_native_probe','verified',?,?)",
        (direct.ORG, direct.VIDEO, object_key, CHECKSUM_A, "ef" * 32, NOW, NOW),
    )
    output_key = database.execute(
        "SELECT output_object_key FROM media_jobs WHERE id=?", (job_id,)
    ).fetchone()[0]
    database.execute(
        "INSERT INTO media_job_execution_v1(job_id,organization_id,video_id,source_version,"
        "catalog_version,profile_id,profile_version,normalized_profile_sha256,route_reason,"
        "selected_executor,fallback_executor,state,attempt,lease_epoch,lease_token_digest,"
        "lease_expires_at_ms,final_object_key,"
        "output_content_type,max_output_bytes,created_at_ms,updated_at_ms) "
        "VALUES (?1,?2,?3,1,1,'normalize_v1',1,?4,'native_only','native_gstreamer',"
        "NULL,?5,?6,?7,?8,?9,?10,'video/mp4',33554432,?11,?12)",
        (
            job_id,
            direct.ORG,
            direct.VIDEO,
            "12" * 32,
            state,
            1 if state in {"leased", "transforming"} else 0,
            1 if state in {"leased", "transforming"} else 0,
            LEGACY_LEASE_DIGEST if state in {"leased", "transforming"} else None,
            NOW + 60_000 if state in {"leased", "transforming"} else None,
            output_key,
            NOW - 100,
            NOW - 100,
        ),
    )


def add_job(
    database: sqlite3.Connection,
    job_id: str,
    profile: str,
    inputs: list[tuple[str, int]],
    primary: tuple[str, int],
) -> None:
    digest = hashlib.sha256(f"{job_id}:{profile}".encode()).hexdigest()
    database.execute(
        "INSERT INTO media_jobs(id,video_id,kind,state,idempotency_key,attempt,payload_json,"
        "created_at_ms,updated_at_ms,organization_id,selected_executor,source_version,"
        "profile_version,output_object_key,cancel_requested,revision,input_contract_version) "
        "VALUES (?,?,?,'queued',?,0,?,?,?,?, 'native_gstreamer',?,1,?,0,0,1)",
        (
            job_id,
            primary[0],
            "segment_mux" if profile == "segment_mux_v1" else "composition",
            hashlib.sha256(job_id.encode()).hexdigest(),
            payload(profile, inputs, primary),
            NOW,
            NOW,
            direct.ORG,
            primary[1],
            f"tenants/{direct.ORG}/videos/{primary[0]}/derivatives/{profile}/{digest}",
        ),
    )


def add_input(
    database: sqlite3.Connection,
    job_id: str,
    ordinal: int,
    video_id: str,
    version: int,
    checksum: str,
) -> None:
    database.execute(
        "INSERT INTO media_job_inputs_v1(job_id,organization_id,ordinal,video_id,"
        "source_version,object_key,bytes,checksum_sha256,content_type,created_at_ms) "
        "VALUES (?,?,?,?,?,?,?,?,?,?)",
        (
            job_id,
            direct.ORG,
            ordinal,
            video_id,
            version,
            f"tenants/{direct.ORG}/videos/{video_id}/source/v{version}/payload.webm",
            1_024,
            checksum,
            "video/webm",
            NOW,
        ),
    )


def ready(database: sqlite3.Connection, job_id: str) -> bool:
    return bool(
        database.execute(
            "SELECT json_type(j.payload_json,'$.source_inputs')='array' AND "
            "(SELECT COUNT(*) FROM media_job_current_inputs_v1 i WHERE i.job_id=j.id) = "
            "json_array_length(j.payload_json,'$.source_inputs') AND "
            "(SELECT MIN(i.ordinal) FROM media_job_current_inputs_v1 i WHERE i.job_id=j.id)=0 AND "
            "(SELECT MAX(i.ordinal) FROM media_job_current_inputs_v1 i WHERE i.job_id=j.id)="
            "json_array_length(j.payload_json,'$.source_inputs')-1 "
            "FROM media_jobs j WHERE j.id=?",
            (job_id,),
        ).fetchone()[0]
    )


def claim_current(database: sqlite3.Connection, job_id: str) -> bool:
    result = database.execute(
        "UPDATE media_jobs SET state='leased',attempt=attempt+1,revision=revision+1 "
        "WHERE id=? AND state='queued' AND json_type(payload_json,'$.source_inputs')='array' "
        "AND (SELECT COUNT(*) FROM media_job_current_inputs_v1 i WHERE i.job_id=media_jobs.id)="
        "json_array_length(payload_json,'$.source_inputs') "
        "AND (SELECT MIN(i.ordinal) FROM media_job_current_inputs_v1 i "
        "WHERE i.job_id=media_jobs.id)=0 "
        "AND (SELECT MAX(i.ordinal) FROM media_job_current_inputs_v1 i "
        "WHERE i.job_id=media_jobs.id)=json_array_length(payload_json,'$.source_inputs')-1 "
        "AND CASE json_extract(payload_json,'$.profile') "
        "WHEN 'segment_mux_v1' THEN json_array_length(payload_json,'$.source_inputs') BETWEEN 2 AND 64 "
        "WHEN 'composition_v1' THEN json_array_length(payload_json,'$.source_inputs') BETWEEN 1 AND 64 "
        "ELSE json_array_length(payload_json,'$.source_inputs')=1 END",
        (job_id,),
    )
    return result.rowcount == 1


def reap_invalid_queued(database: sqlite3.Connection) -> str | None:
    candidate = database.execute(
        "SELECT j.id,j.revision,j.attempt FROM media_jobs j "
        "WHERE j.organization_id=? AND j.selected_executor='native_gstreamer' "
        "AND j.state='queued' AND (COALESCE(j.input_contract_version,0)!=1 OR "
        "(SELECT COUNT(*) FROM media_job_inputs_v1 bound "
        "WHERE bound.job_id=j.id AND bound.organization_id=j.organization_id) NOT BETWEEN 1 AND 64 OR "
        "(SELECT COUNT(*) FROM media_job_current_inputs_v1 i "
        "WHERE i.job_id=j.id AND i.organization_id=j.organization_id)!="
        "(SELECT COUNT(*) FROM media_job_inputs_v1 bound "
        "WHERE bound.job_id=j.id AND bound.organization_id=j.organization_id)) "
        "ORDER BY j.updated_at_ms,j.id LIMIT 1",
        (direct.ORG,),
    ).fetchone()
    if candidate is None:
        return None
    job_id, revision, attempt = candidate
    next_revision = revision + 1
    transition = database.execute(
        "UPDATE media_jobs SET state='failed',error_code='media_input_authority_missing',"
        "error_class='input_authority_missing',lease_expires_at_ms=NULL,worker_id=NULL,"
        "lease_token_digest=NULL,heartbeat_at_ms=NULL,updated_at_ms=?,revision=revision+1 "
        "WHERE id=? AND organization_id=? AND revision=? "
        "AND selected_executor='native_gstreamer' AND state='queued' AND ("
        "COALESCE(input_contract_version,0)!=1 OR "
        "(SELECT COUNT(*) FROM media_job_inputs_v1 bound "
        "WHERE bound.job_id=media_jobs.id AND bound.organization_id=media_jobs.organization_id) "
        "NOT BETWEEN 1 AND 64 OR "
        "(SELECT COUNT(*) FROM media_job_current_inputs_v1 i "
        "WHERE i.job_id=media_jobs.id AND i.organization_id=media_jobs.organization_id)!="
        "(SELECT COUNT(*) FROM media_job_inputs_v1 bound "
        "WHERE bound.job_id=media_jobs.id AND bound.organization_id=media_jobs.organization_id))",
        (NOW + 1, job_id, direct.ORG, revision),
    )
    require(transition.rowcount == 1, "revoked source authority raced without a disposition")
    database.execute(
        "UPDATE media_job_execution_v1 SET state='failed',failure_class='invalid_input',"
        "lease_token_digest=NULL,lease_expires_at_ms=NULL,updated_at_ms=? "
        "WHERE job_id=? AND organization_id=? AND selected_executor='native_gstreamer' "
        "AND state NOT IN ('succeeded','failed','cancelled','dead_letter') "
        "AND EXISTS (SELECT 1 FROM media_jobs j WHERE j.id=? "
        "AND j.organization_id=? AND j.state='failed' AND j.revision=?)",
        (NOW + 1, job_id, direct.ORG, job_id, direct.ORG, next_revision),
    )
    database.execute(
        "INSERT INTO media_job_dead_letters(job_id,attempt,error_class,diagnostic_code,created_at_ms) "
        "SELECT ?,?,'input_authority_missing','native_input_authority_revoked',? "
        "FROM media_jobs WHERE ? > 0 AND id=? AND organization_id=? "
        "AND state='failed' AND revision=? ON CONFLICT(job_id) DO NOTHING",
        (job_id, attempt, NOW + 1, attempt, job_id, direct.ORG, next_revision),
    )
    outbox_payload = json.dumps(
        {
            "schema_version": 1,
            "job_id": job_id,
            "attempt": attempt,
            "state": "failed",
            "error_class": "input_authority_missing",
        },
        separators=(",", ":"),
    )
    database.execute(
        "INSERT INTO outbox_events(id,organization_id,aggregate_type,aggregate_id,event_type,"
        "deduplication_key,payload_json,state,attempt,available_at_ms,created_at_ms,"
        "event_sequence,event_fingerprint,payload_schema_version,payload_checksum,revision) "
        "SELECT ?,?,'media_job',?,'media.job.failed',?,?,'pending',0,?,?,0,?,1,?,0 "
        "FROM media_jobs WHERE id=? AND organization_id=? AND state='failed' AND revision=?",
        (
            f"outbox-{job_id}",
            direct.ORG,
            job_id,
            f"media-input-invalid:{job_id}:{revision}",
            outbox_payload,
            NOW + 1,
            NOW + 1,
            hashlib.sha256(b"frame-business-ordered-lifecycle-initial-v1").hexdigest(),
            hashlib.sha256(outbox_payload.encode()).hexdigest(),
            job_id,
            direct.ORG,
            next_revision,
        ),
    )
    return job_id


def drain_active_pre_migration_job(
    database: sqlite3.Connection, job_id: str
) -> None:
    """Model the protected gate waiting out an old lease before contract."""
    row = database.execute(
        "SELECT revision,attempt FROM media_jobs WHERE id=? "
        "AND state IN ('leased','running')",
        (job_id,),
    ).fetchone()
    require(row is not None, f"active rollout drain fixture is missing: {job_id}")
    revision, attempt = row
    database.execute(
        "UPDATE media_jobs SET state='failed',error_code='deployment_drain',"
        "error_class='lease_lost',worker_id=NULL,lease_token_digest=NULL,"
        "lease_expires_at_ms=NULL,heartbeat_at_ms=NULL,updated_at_ms=?,revision=revision+1 "
        "WHERE id=? AND revision=? AND state IN ('leased','running')",
        (NOW + 2, job_id, revision),
    )
    database.execute(
        "UPDATE media_job_attempts SET finished_at_ms=?,outcome='lost_lease',"
        "error_class='lease_lost' WHERE job_id=? AND attempt=? AND outcome IS NULL",
        (NOW + 2, job_id, attempt),
    )
    database.execute(
        "UPDATE media_job_execution_v1 SET state='failed',failure_class='lease_lost',"
        "lease_token_digest=NULL,lease_expires_at_ms=NULL,updated_at_ms=? "
        "WHERE job_id=? AND state NOT IN ('succeeded','failed','cancelled','dead_letter')",
        (NOW + 2, job_id),
    )


def contract_refusal_proof() -> None:
    database = sqlite3.connect(":memory:")
    migrate(database, through=26)
    seed(database)
    add_pre_migration_job(
        database,
        LEGACY_REJECTED_JOB,
        executor="native_gstreamer",
        explicit_singleton=False,
    )
    apply_migration_0027(database)
    expect_integrity(
        lambda: apply_contract_0032(database),
        "frame_media_job_input_contract_not_drained_v1",
    )
    require(
        database.execute(
            "SELECT state FROM media_jobs WHERE id=?", (LEGACY_REJECTED_JOB,)
        ).fetchone()
        == ("queued",),
        "refused contract migration mutated undrained legacy work",
    )
    database.close()


def media_create_wire_document(*, explicit_singleton: bool, version: int = 1) -> str:
    document: dict[str, object] = {
        "schema_version": 1,
        "tenant_id": direct.ORG,
        "video_id": direct.VIDEO,
        "source_version": version,
    }
    if explicit_singleton:
        document["source_inputs"] = [
            {"video_id": direct.VIDEO, "source_version": version}
        ]
    document["profile"] = "normalize_v1"
    return json.dumps(document, separators=(",", ":"))


def media_create_digest(*, explicit_singleton: bool, version: int = 1) -> str:
    wire = media_create_wire_document(
        explicit_singleton=explicit_singleton, version=version
    )
    return hashlib.sha256(b"media_job_create\0" + wire.encode()).hexdigest()


def singleton_idempotency_alias_proof(database: sqlite3.Connection) -> int:
    canonical = media_create_digest(explicit_singleton=False)
    explicit = media_create_digest(explicit_singleton=True)
    require(canonical != explicit, "serializer variants unexpectedly have one wire digest")
    key = "media-rollout-replay-v1"
    response = json.dumps(
        {
            "schema_version": 1,
            "job_id": LEGACY_JOB,
            "state": "queued",
            "profile": "normalize_v1",
            "executor": "native_gstreamer",
        },
        separators=(",", ":"),
    )

    replay_count = 0
    for stored_digest in (canonical, explicit):
        database.execute(
            "INSERT INTO command_idempotency(organization_id,idempotency_key,command_type,"
            "request_digest,response_status,response_json,created_at_ms,expires_at_ms) "
            "VALUES (?,?,'media_job_create',?,202,?,?,?)",
            (direct.ORG, key, stored_digest, response, NOW, NOW + 86_400_000),
        )
        row = database.execute(
            "SELECT response_status,response_json FROM command_idempotency "
            "WHERE organization_id=? AND idempotency_key=? "
            "AND command_type='media_job_create' AND request_digest IN (?,?)",
            (direct.ORG, key, canonical, explicit),
        ).fetchone()
        require(row == (202, response), "singleton serializer alias did not replay")
        different = media_create_digest(explicit_singleton=False, version=2)
        require(
            database.execute(
                "SELECT COUNT(*) FROM command_idempotency WHERE organization_id=? "
                "AND idempotency_key=? AND request_digest=?",
                (direct.ORG, key, different),
            ).fetchone()[0]
            == 0,
            "different source version was treated as a serializer alias",
        )
        database.execute(
            "DELETE FROM command_idempotency WHERE organization_id=? AND idempotency_key=?",
            (direct.ORG, key),
        )
        replay_count += 1
    return replay_count


def seed(database: sqlite3.Connection) -> None:
    direct.seed(database)
    document = json.dumps(
        {"schema_version": 1, "title": "Second native input"},
        sort_keys=True,
        separators=(",", ":"),
    )
    database.execute(
        "INSERT INTO videos(id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id,"
        "privacy,metadata_json,revision,metadata_schema_version,metadata_checksum,comments_enabled,"
        "last_operation_id,duration_ms) VALUES (?,?,?,'ready',?,?,?,?,?,1,1,?,1,?,1000)",
        (
            VIDEO_B,
            direct.USER,
            "Second native input",
            NOW - 1_000,
            NOW - 1_000,
            direct.ORG,
            "private",
            document,
            hashlib.sha256(document.encode()).hexdigest(),
            direct.operation(31),
        ),
    )
    for video_id, version, checksum in (
        (direct.VIDEO, 1, CHECKSUM_A),
        (VIDEO_B, 2, CHECKSUM_B),
    ):
        database.execute(
            "INSERT INTO object_manifests(object_key,video_id,role,bytes,checksum_sha256,"
            "content_type,created_at_ms,organization_id,object_version,provider_etag,state,"
            "updated_at_ms) VALUES (?,?, 'source',1024,?,'video/webm',?,?,?,'etag','available',?)",
            (
                f"tenants/{direct.ORG}/videos/{video_id}/source/v{version}/payload.webm",
                video_id,
                checksum,
                NOW,
                direct.ORG,
                version,
                NOW,
            ),
        )
        database.execute(
            "INSERT INTO storage_governed_objects_v1(organization_id,object_key,role,visibility,"
            "state,malware_disposition,immutable_revision,cache_generation,checksum_sha256,bytes,"
            "content_type,retention_until_ms,created_at_ms,updated_at_ms) "
            "VALUES (?,?,'source','private','active','clean',?,1,?,1024,'video/webm',NULL,?,?)",
            (
                direct.ORG,
                f"tenants/{direct.ORG}/videos/{video_id}/source/v{version}/payload.webm",
                version,
                checksum,
                NOW,
                NOW,
            ),
        )


def concurrency_proof() -> None:
    with tempfile.TemporaryDirectory(prefix="frame-media-input-race-") as directory:
        path = pathlib.Path(directory) / "authority.sqlite3"
        setup = sqlite3.connect(path)
        migrate(setup)
        seed(setup)
        pair = [(direct.VIDEO, 1), (VIDEO_B, 2)]
        add_job(setup, RACE_JOB, "segment_mux_v1", pair, pair[0])
        add_input(setup, RACE_JOB, 0, direct.VIDEO, 1, CHECKSUM_A)
        add_input(setup, RACE_JOB, 1, VIDEO_B, 2, CHECKSUM_B)
        setup.commit()
        setup.close()

        claimer = sqlite3.connect(path, timeout=0)
        authority = sqlite3.connect(path, timeout=0)
        try:
            for connection in (claimer, authority):
                connection.execute("PRAGMA foreign_keys = ON")
            claimer.execute("BEGIN IMMEDIATE")
            require(ready(claimer, RACE_JOB), "race fixture is not initially claimable")
            try:
                authority.execute(
                    "UPDATE object_manifests SET state='missing' WHERE organization_id=? "
                    "AND video_id=? AND object_version=2",
                    (direct.ORG, VIDEO_B),
                )
            except sqlite3.OperationalError as error:
                require("locked" in str(error).lower(), f"unexpected race failure: {error}")
                authority.rollback()
            else:
                raise ConformanceFailure("authority mutation bypassed the claim write lock")
            require(claim_current(claimer, RACE_JOB), "atomic current-authority claim was rejected")
            claimer.commit()

            authority.execute(
                "UPDATE object_manifests SET state='missing' WHERE organization_id=? "
                "AND video_id=? AND object_version=2",
                (direct.ORG, VIDEO_B),
            )
            authority.commit()
            require(
                not ready(authority, RACE_JOB),
                "post-claim authority loss remained source-deliverable",
            )
        finally:
            claimer.close()
            authority.close()


def main() -> int:
    contract_refusal_proof()
    database = sqlite3.connect(":memory:")
    migrate(database, through=26)
    seed(database)
    add_pre_migration_job(
        database,
        LEGACY_JOB,
        executor="native_gstreamer",
        explicit_singleton=False,
        revision=4,
    )
    add_pre_migration_execution(database, LEGACY_JOB)
    add_pre_migration_job(
        database,
        LEGACY_EXPLICIT_JOB,
        executor="native_gstreamer",
        explicit_singleton=True,
    )
    add_pre_migration_job(
        database,
        LEGACY_REMOTE_JOB,
        executor="cloudflare_media",
        explicit_singleton=False,
    )
    add_pre_migration_job(
        database,
        LEGACY_LEASED_JOB,
        executor="native_gstreamer",
        explicit_singleton=False,
        state="leased",
        attempt=1,
        revision=7,
    )
    add_pre_migration_execution(database, LEGACY_LEASED_JOB, state="leased")
    add_pre_migration_job(
        database,
        LEGACY_RUNNING_JOB,
        executor="native_gstreamer",
        explicit_singleton=False,
        state="running",
        attempt=1,
        revision=9,
    )
    add_pre_migration_execution(database, LEGACY_RUNNING_JOB, state="transforming")
    apply_migration_0027(database)

    require(
        database.execute(
            "SELECT state FROM media_jobs WHERE id=?", (LEGACY_JOB,)
        ).fetchone()
        == ("queued",),
        "expand migration terminalized an N-1 queued job before Worker deploy",
    )
    require(
        database.execute(
            "SELECT state FROM media_jobs WHERE id=?", (LEGACY_LEASED_JOB,)
        ).fetchone()
        == ("leased",),
        "expand migration terminalized an N-1 leased provider operation",
    )
    add_pre_migration_job(
        database,
        EXPAND_WINDOW_JOB,
        executor="native_gstreamer",
        explicit_singleton=False,
    )
    require(
        database.execute(
            "SELECT state FROM media_jobs WHERE id=?", (EXPAND_WINDOW_JOB,)
        ).fetchone()
        == ("queued",),
        "expand phase rejected the previous Worker before claim-aware deploy",
    )
    reaped_legacy_jobs: list[str] = []
    while (reaped := reap_invalid_queued(database)) is not None:
        reaped_legacy_jobs.append(reaped)
    require(
        set(reaped_legacy_jobs)
        == {LEGACY_JOB, LEGACY_EXPLICIT_JOB, EXPAND_WINDOW_JOB},
        f"normal queued-authority reaper did not drain legacy work: {reaped_legacy_jobs}",
    )
    drain_active_pre_migration_job(database, LEGACY_LEASED_JOB)
    drain_active_pre_migration_job(database, LEGACY_RUNNING_JOB)
    require(
        database.execute(
            "SELECT COUNT(*) FROM media_jobs j WHERE j.selected_executor='native_gstreamer' "
            "AND j.state IN ('queued','leased','running') AND NOT EXISTS "
            "(SELECT 1 FROM media_job_inputs_v1 i WHERE i.job_id=j.id)"
        ).fetchone()[0]
        == 0,
        "protected rollout drain left legacy nonterminal native work",
    )
    apply_contract_0032(database)
    require(
        database.execute(
            "SELECT assertion FROM media_job_input_contract_assertions_v1 WHERE singleton=1"
        ).fetchone()
        == ("legacy_native_work_drained",),
        "contract phase did not persist its immutable drain assertion",
    )
    require(
        database.execute(
            "SELECT phase FROM media_job_input_rollout_v1 WHERE singleton=1"
        ).fetchone()
        == ("enforced",),
        "contract phase did not publish enforced rollout state",
    )

    legacy = database.execute(
        "SELECT state,attempt,error_code,error_class,revision FROM media_jobs WHERE id=?",
        (LEGACY_JOB,),
    ).fetchone()
    require(
        legacy
        == (
            "failed",
            0,
            "media_input_authority_missing",
            "input_authority_missing",
            5,
        ),
        f"pre-migration native job was not terminally dispositioned: {legacy}",
    )
    require(
        database.execute(
            "SELECT state,failure_class,attempt FROM media_job_execution_v1 WHERE job_id=?",
            (LEGACY_JOB,),
        ).fetchone()
        == ("failed", "invalid_input", 0),
        "managed execution journal did not follow rollout disposition",
    )
    require(
        database.execute(
            "SELECT state,attempt,error_code FROM media_jobs WHERE id=?",
            (EXPAND_WINDOW_JOB,),
        ).fetchone()
        == ("failed", 0, "media_input_authority_missing"),
        "normal queued-authority reaper did not reconcile the expand-window job",
    )
    require(
        database.execute(
            "SELECT state,attempt FROM media_jobs WHERE id=?", (LEGACY_EXPLICIT_JOB,)
        ).fetchone()
        == ("failed", 0),
        "pre-migration explicit payload was trusted without an input authority row",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM media_job_inputs_v1 WHERE job_id IN (?,?,?,?)",
            (
                LEGACY_JOB,
                LEGACY_EXPLICIT_JOB,
                LEGACY_LEASED_JOB,
                LEGACY_RUNNING_JOB,
            ),
        ).fetchone()[0]
        == 0,
        "rollout migration fabricated legacy source inputs",
    )
    require(
        database.execute(
            "SELECT state FROM media_jobs WHERE id=?", (LEGACY_REMOTE_JOB,)
        ).fetchone()
        == ("queued",),
        "non-native queued work was changed by the native rollout",
    )
    active = database.execute(
        "SELECT id,state,attempt,worker_id,lease_token_digest,lease_expires_at_ms,"
        "heartbeat_at_ms,error_code,error_class,revision FROM media_jobs "
        "WHERE id IN (?,?) ORDER BY id",
        (LEGACY_LEASED_JOB, LEGACY_RUNNING_JOB),
    ).fetchall()
    require(
        active
        == [
            (
                LEGACY_LEASED_JOB,
                "failed",
                1,
                None,
                None,
                None,
                None,
                "deployment_drain",
                "lease_lost",
                8,
            ),
            (
                LEGACY_RUNNING_JOB,
                "failed",
                1,
                None,
                None,
                None,
                None,
                "deployment_drain",
                "lease_lost",
                10,
            ),
        ],
        f"active pre-migration jobs did not drain before contract: {active}",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM media_job_attempts WHERE job_id IN (?,?) AND attempt=1 "
            "AND outcome='lost_lease' AND error_class='lease_lost' "
            "AND finished_at_ms IS NOT NULL",
            (LEGACY_LEASED_JOB, LEGACY_RUNNING_JOB),
        ).fetchone()[0]
        == 2,
        "active pre-migration attempts were not drained exactly once",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM media_job_execution_v1 WHERE job_id IN (?,?) "
            "AND state='failed' AND failure_class='lease_lost' AND attempt=1 "
            "AND lease_token_digest IS NULL AND lease_expires_at_ms IS NULL",
            (LEGACY_LEASED_JOB, LEGACY_RUNNING_JOB),
        ).fetchone()[0]
        == 2,
        "active managed execution rows were not drained before contract",
    )
    late_completion = database.execute(
        "UPDATE media_jobs SET state='succeeded',revision=revision+1 WHERE id=? "
        "AND state IN ('leased','running') AND worker_id=? AND lease_token_digest=?",
        (LEGACY_LEASED_JOB, direct.USER, LEGACY_LEASE_DIGEST),
    )
    require(late_completion.rowcount == 0, "late leased worker completed terminal work")
    expect_integrity(
        lambda: database.execute(
            "UPDATE media_jobs SET state='running' WHERE id=?", (LEGACY_RUNNING_JOB,)
        ),
        "terminal media job state is immutable",
    )
    expect_integrity(
        lambda: add_pre_migration_job(
            database,
            LEGACY_REJECTED_JOB,
            executor="native_gstreamer",
            explicit_singleton=False,
        ),
        "frame_media_job_input_rollout_v1",
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE media_jobs SET selected_executor='native_gstreamer' WHERE id=?",
            (LEGACY_REMOTE_JOB,),
        ),
        "frame_media_job_input_rollout_v1",
    )
    serializer_alias_replays = singleton_idempotency_alias_proof(database)
    pair = [(direct.VIDEO, 1), (VIDEO_B, 2)]

    add_job(database, JOB, "segment_mux_v1", pair, pair[0])
    add_input(database, JOB, 0, direct.VIDEO, 1, CHECKSUM_A)
    add_input(database, JOB, 1, VIDEO_B, 2, CHECKSUM_B)
    require(ready(database, JOB), "complete dense segment input set is not claimable")
    require(
        [row[0] for row in database.execute(
            "SELECT ordinal FROM media_job_inputs_v1 WHERE job_id=? ORDER BY ordinal", (JOB,)
        )] == [0, 1],
        "segment inputs are not retained in canonical order",
    )
    expect_integrity(
        lambda: database.execute(
            "UPDATE media_job_inputs_v1 SET bytes=2048 WHERE job_id=? AND ordinal=0", (JOB,)
        ),
        "frame_media_job_input_immutable_v1",
    )
    expect_integrity(
        lambda: database.execute(
            "DELETE FROM media_job_inputs_v1 WHERE job_id=? AND ordinal=0", (JOB,)
        ),
        "frame_media_job_input_immutable_v1",
    )

    add_job(database, PARTIAL_JOB, "composition_v1", pair, pair[0])
    add_input(database, PARTIAL_JOB, 1, VIDEO_B, 2, CHECKSUM_B)
    require(not ready(database, PARTIAL_JOB), "sparse partial input set became claimable")
    add_input(database, PARTIAL_JOB, 0, direct.VIDEO, 1, CHECKSUM_A)
    require(ready(database, PARTIAL_JOB), "completed composition input set is not claimable")

    repeated = [pair[0], pair[0]]
    add_job(database, REPEATED_JOB, "composition_v1", repeated, repeated[0])
    add_input(database, REPEATED_JOB, 0, direct.VIDEO, 1, CHECKSUM_A)
    add_input(database, REPEATED_JOB, 1, direct.VIDEO, 1, CHECKSUM_A)
    require(
        ready(database, REPEATED_JOB),
        "ordinal-distinct repeated composition source was rejected",
    )

    add_job(database, DUPLICATE_SEGMENT_JOB, "segment_mux_v1", repeated, repeated[0])
    add_input(database, DUPLICATE_SEGMENT_JOB, 0, direct.VIDEO, 1, CHECKSUM_A)
    expect_integrity(
        lambda: add_input(database, DUPLICATE_SEGMENT_JOB, 1, direct.VIDEO, 1, CHECKSUM_A),
        "frame_media_job_input_authority_v1",
    )

    add_job(database, AUTHORITY_JOB, "segment_mux_v1", pair, pair[0])
    add_input(database, AUTHORITY_JOB, 0, direct.VIDEO, 1, CHECKSUM_A)
    add_input(database, AUTHORITY_JOB, 1, VIDEO_B, 2, CHECKSUM_B)
    database.execute(
        "UPDATE storage_governed_objects_v1 SET state='quarantined' "
        "WHERE organization_id=? AND object_key=?",
        (
            direct.ORG,
            f"tenants/{direct.ORG}/videos/{VIDEO_B}/source/v2/payload.webm",
        ),
    )
    require(not ready(database, AUTHORITY_JOB), "quarantined authority remained claimable")
    require(
        not claim_current(database, AUTHORITY_JOB),
        "claim update ignored current source authority",
    )
    database.execute(
        "UPDATE storage_governed_objects_v1 SET state='active' "
        "WHERE organization_id=? AND object_key=?",
        (
            direct.ORG,
            f"tenants/{direct.ORG}/videos/{VIDEO_B}/source/v2/payload.webm",
        ),
    )
    require(claim_current(database, AUTHORITY_JOB), "restored dense authority was not claimable")

    database.execute(
        "UPDATE storage_governed_objects_v1 SET state='quarantined' "
        "WHERE organization_id=? AND object_key=?",
        (
            direct.ORG,
            f"tenants/{direct.ORG}/videos/{VIDEO_B}/source/v2/payload.webm",
        ),
    )
    require(
        reap_invalid_queued(database) == JOB,
        "the oldest queued job with revoked source authority was not reconciled",
    )
    require(
        database.execute(
            "SELECT state='failed' AND attempt=0 AND revision=1 "
            "AND error_code='media_input_authority_missing' "
            "AND error_class='input_authority_missing' FROM media_jobs WHERE id=?",
            (JOB,),
        ).fetchone()[0]
        == 1,
        "revoked attempt-zero source authority did not become a stable terminal failure",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM media_job_dead_letters WHERE job_id=?", (JOB,)
        ).fetchone()[0]
        == 0,
        "attempt-zero authority reconciliation fabricated a worker attempt",
    )
    require(
        database.execute(
            "SELECT COUNT(*) FROM outbox_events WHERE aggregate_id=? "
            "AND event_type='media.job.failed' AND state='pending'",
            (JOB,),
        ).fetchone()[0]
        == 1,
        "revoked source authority failure was not published through the outbox",
    )
    database.execute(
        "UPDATE storage_governed_objects_v1 SET state='active' "
        "WHERE organization_id=? AND object_key=?",
        (
            direct.ORG,
            f"tenants/{direct.ORG}/videos/{VIDEO_B}/source/v2/payload.webm",
        ),
    )
    require(
        not claim_current(database, JOB),
        "restoring source authority resurrected a terminally reconciled job",
    )

    expect_integrity(
        lambda: add_job(database, SINGLE_JOB, "segment_mux_v1", [pair[0]], pair[0]),
        "frame_media_job_input_rollout_v1",
    )
    expect_integrity(
        lambda: add_job(database, WRONG_PROFILE_JOB, "normalize_v1", pair, pair[0]),
        "frame_media_job_input_rollout_v1",
    )
    expect_integrity(
        lambda: add_input(database, PARTIAL_JOB, 0, direct.VIDEO, 1, CHECKSUM_A),
        "UNIQUE constraint failed",
    )

    database.commit()
    concurrency_proof()

    print(
        json.dumps(
            {
                "expand_migration": "0027_media_job_inputs.sql",
                "contract_migration": "0032_media_job_inputs_enforce.sql",
                "dense_claim": 4,
                "immutable_rejections": 2,
                "authority_rejections": 7,
                "partial_claim_rejections": 2,
                "queued_authority_revocations_terminalized": 1,
                "repeated_composition_sources": 2,
                "pre_migration_queued_reaper_dispositions": len(reaped_legacy_jobs),
                "expand_phase_n_minus_one_writes_preserved": 3,
                "active_attempts_drained_without_increment": 2,
                "contract_refusals_before_drain": 1,
                "contract_drain_assertions": 1,
                "late_worker_writes_rejected": 2,
                "legacy_inputs_inferred": 0,
                "singleton_idempotency_alias_replays": serializer_alias_replays,
                "serialized_claim_races": 1,
            },
            sort_keys=True,
            separators=(",", ":"),
        )
    )
    return 0


direct = load_direct_module()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except ConformanceFailure as error:
        print(f"media job input conformance failed: {error}")
        raise SystemExit(1) from error
