#!/usr/bin/env python3
"""Apply every D1/SQLite migration and enforce expand-first invariants."""

from __future__ import annotations

import pathlib
import re
import sqlite3
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
MIGRATIONS = ROOT / "apps" / "control-plane" / "migrations"
CONTRACT_MIGRATIONS = ROOT / "apps" / "control-plane" / "contract-migrations"
NAME = re.compile(r"^(\d{4})_[a-z0-9_]+\.sql$")
DESTRUCTIVE = re.compile(
    r"\b(DROP\s+(?:TABLE|COLUMN|INDEX)|TRUNCATE|VACUUM|DELETE\s+FROM)\b",
    re.IGNORECASE,
)
REQUIRED_TABLES = {
    "auth_api_keys",
    "api_webhook_replay_claims_v1",
    "authority_state",
    "comments",
    "command_idempotency",
    "compatibility_rate_limit_buckets_v1",
    "cutover_authority_audit",
    "cutover_authority_scopes",
    "cutover_change_events",
    "cutover_maintenance_windows",
    "cutover_operational_signal_events",
    "cutover_operational_signals",
    "cutover_repository_assertions_v1",
    "cutover_shadow_query_requirements",
    "cutover_shadow_observations",
    "cutover_slo_config",
    "developer_apps",
    "developer_credit_transactions",
    "etl_checkpoints",
    "folders",
    "identity_accounts",
    "media_jobs",
    "media_execution_events_v1",
    "media_job_execution_v1",
    "media_output_manifests_v1",
    "media_profile_policies_v1",
    "media_source_probes_v1",
    "multipart_upload_parts",
    "multipart_uploads",
    "object_deletion_jobs",
    "object_legal_holds",
    "object_manifests",
    "organizations",
    "organization_members",
    "outbox_events",
    "public_analytics_consents_v1",
    "public_analytics_events_v1",
    "public_collaboration_grants_v1",
    "public_collaboration_policies_v1",
    "public_comment_operations_v1",
    "public_transcripts_v1",
    "sessions",
    "shared_videos",
    "spaces",
    "storage_objects",
    "users",
    "video_edits",
    "video_uploads",
    "videos",
}


def discover() -> list[pathlib.Path]:
    files = sorted(MIGRATIONS.glob("*.sql"))
    numbers: list[int] = []
    for path in files:
        match = NAME.fullmatch(path.name)
        if not match:
            raise ValueError(f"invalid migration filename: {path.name}")
        numbers.append(int(match.group(1)))
    if not files:
        raise ValueError("no migrations found")
    if len(numbers) != len(set(numbers)):
        raise ValueError(f"duplicate expand migration number: {numbers}")
    return files


def discover_contract(expand_files: list[pathlib.Path]) -> list[pathlib.Path]:
    files = sorted(CONTRACT_MIGRATIONS.glob("*.sql"))
    if not files:
        raise ValueError("no protected contract migrations found")
    expand_numbers = {int(path.name[:4]) for path in expand_files}
    numbers: list[int] = []
    for path in files:
        match = NAME.fullmatch(path.name)
        if not match:
            raise ValueError(f"invalid contract migration filename: {path.name}")
        number = int(match.group(1))
        if number in expand_numbers:
            raise ValueError(f"duplicate expand/contract migration number: {number:04d}")
        numbers.append(number)
    if len(numbers) != len(set(numbers)):
        raise ValueError(f"duplicate contract migration number: {numbers}")
    combined = sorted(expand_numbers | set(numbers))
    if combined != list(range(combined[0], combined[-1] + 1)):
        raise ValueError(f"combined migration sequence is not contiguous: {combined}")
    return files


def apply(files: list[pathlib.Path]) -> sqlite3.Connection:
    database = sqlite3.connect(":memory:")
    database.execute("PRAGMA foreign_keys = ON")
    for path in files:
        sql = path.read_text(encoding="utf-8")
        if DESTRUCTIVE.search(sql):
            raise ValueError(f"destructive statement in expand migration: {path.name}")
        try:
            database.executescript(sql)
        except sqlite3.Error as error:
            raise ValueError(f"{path.name} failed: {error}") from error
        violations = database.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise ValueError(f"{path.name} introduced foreign-key violations")
    return database


def expect_integrity_error(
    database: sqlite3.Connection, sql: str, parameters: tuple[object, ...]
) -> None:
    try:
        database.execute(sql, parameters)
    except sqlite3.IntegrityError:
        return
    raise ValueError("scoped cutover schema accepted an invalid authority mutation")


def verify_scoped_cutover_invariants(database: sqlite3.Connection) -> None:
    zero = "0" * 64
    database.execute(
        """INSERT INTO cutover_authority_scopes(
             tenant_id, domain, phase, writer, mirror_enabled, replay_paused,
             epoch, audit_head, rollback_ready, phase_started_at_ms, updated_at_ms
           ) VALUES ('migration-check', 'metadata', 'legacy_authoritative',
                     'legacy', 0, 0, 0, ?, 0, 0, 0)""",
        (zero,),
    )
    database.execute(
        """INSERT INTO cutover_authority_scopes(
             tenant_id, domain, phase, writer, mirror_enabled, replay_paused,
             epoch, audit_head, rollback_ready, phase_started_at_ms, updated_at_ms
           ) VALUES ('deletion-check', 'metadata', 'legacy_authoritative',
                     'legacy', 0, 0, 0, ?, 0, 0, 0)""",
        (zero,),
    )
    expect_integrity_error(
        database,
        "DELETE FROM cutover_authority_scopes WHERE tenant_id = 'deletion-check'",
        (),
    )
    expect_integrity_error(
        database,
        """INSERT INTO cutover_authority_scopes(
             tenant_id, domain, phase, writer, mirror_enabled, replay_paused,
             epoch, audit_head, rollback_ready, phase_started_at_ms, updated_at_ms
           ) VALUES ('invalid-two-writer', 'metadata', 'dual_write',
                     'd1', 1, 0, 0, ?, 0, 0, 0)""",
        (zero,),
    )
    expect_integrity_error(
        database,
        """UPDATE cutover_authority_scopes
           SET phase = 'shadow_read', epoch = 1, updated_at_ms = 1
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'""",
        (),
    )
    audit_hash = "1" * 64
    database.execute(
        """INSERT INTO cutover_authority_audit(
             audit_hash, previous_hash, tenant_id, domain, action,
             from_phase, to_phase, from_epoch, to_epoch,
             operator_digest, evidence_digest, occurred_at_ms
           ) VALUES (?, ?, 'migration-check', 'metadata', 'transition',
                     'legacy_authoritative', 'shadow_read', 0, 1, ?, ?, 1)""",
        (audit_hash, zero, "2" * 64, "3" * 64),
    )
    database.execute(
        """UPDATE cutover_authority_scopes
           SET phase = 'shadow_read', epoch = 1, phase_epoch = 1, audit_head = ?,
               phase_started_at_ms = 1, updated_at_ms = 1
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'""",
        (audit_hash,),
    )
    expect_integrity_error(
        database,
        """UPDATE cutover_authority_scopes
           SET tenant_id = 'moved-scope', epoch = 2
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'""",
        (),
    )
    database.execute(
        """INSERT INTO cutover_slo_config(
             tenant_id, domain, shadow_window_ms, minimum_shadow_observations,
             max_pending_lag_ms, max_shadow_mismatches, max_dead_letter_events,
             max_contention_events, approved_by_digest, updated_at_ms
           ) VALUES ('migration-check', 'metadata', 1000, 1, 1000, 0, 0, 0, ?, 1)""",
        ("2" * 64,),
    )
    database.execute(
        """INSERT INTO cutover_shadow_query_requirements(
             tenant_id, domain, query_class, normalization_digest,
             approved_by_digest, created_at_ms
           ) VALUES ('migration-check', 'metadata', 'video_list', ?, ?, 1)""",
        ("4" * 64, "2" * 64),
    )
    expect_integrity_error(
        database,
        """INSERT INTO cutover_shadow_query_requirements(
             tenant_id, domain, query_class, normalization_digest,
             approved_by_digest, created_at_ms
           ) VALUES ('migration-check', 'metadata', 'unsafe_query', ?, ?, 1)""",
        ("G" * 64, "2" * 64),
    )
    shadow_insert = """INSERT INTO cutover_shadow_observations(
         observation_digest, tenant_id, domain, phase_epoch, query_class, normalization_digest,
         legacy_result_digest, d1_result_digest, classification, observed_at_ms
       ) VALUES (?, 'migration-check', 'metadata', ?, 'video_list', ?, ?, ?, 'match', ?)"""
    expect_integrity_error(
        database,
        shadow_insert,
        ("5" * 64, 1, "4" * 64, "6" * 64, "6" * 64, 0),
    )
    database.execute(
        shadow_insert,
        ("5" * 64, 1, "4" * 64, "6" * 64, "6" * 64, 1),
    )
    database.execute(
        shadow_insert.replace("INSERT INTO", "INSERT OR IGNORE INTO", 1),
        ("5" * 64, 1, "4" * 64, "6" * 64, "6" * 64, 1),
    )
    expect_integrity_error(
        database,
        shadow_insert.replace("INSERT INTO", "INSERT OR IGNORE INTO", 1),
        ("5" * 64, 1, "4" * 64, "7" * 64, "6" * 64, 1),
    )
    expect_integrity_error(
        database,
        shadow_insert,
        ("a" * 64, 0, "4" * 64, "6" * 64, "6" * 64, 1),
    )
    expect_integrity_error(
        database,
        "UPDATE cutover_shadow_observations SET classification = 'error'",
        (),
    )
    expect_integrity_error(
        database,
        "DELETE FROM cutover_shadow_query_requirements",
        (),
    )
    signal_insert = """INSERT INTO cutover_operational_signal_events(
         tenant_id, domain, phase_epoch, kind, occurred_at_ms
       ) VALUES ('migration-check', 'metadata', ?, 'authority_contention', ?)"""
    expect_integrity_error(database, signal_insert, (1, 0))
    expect_integrity_error(database, signal_insert, (0, 1))
    database.execute(signal_insert, (1, 1))
    if database.execute(
        """SELECT count, last_at_ms FROM cutover_operational_signals
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'
             AND kind = 'authority_contention'"""
    ).fetchone() != (1, 1):
        raise ValueError("scoped cutover signal rollup did not follow its event")
    expect_integrity_error(
        database,
        """UPDATE cutover_operational_signals SET count = 2
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'
             AND kind = 'authority_contention'""",
        (),
    )
    expect_integrity_error(
        database,
        "UPDATE cutover_operational_signal_events SET occurred_at_ms = 2",
        (),
    )
    expect_integrity_error(
        database,
        "DELETE FROM authority_state WHERE singleton = 1",
        (),
    )
    expect_integrity_error(
        database,
        """UPDATE authority_state
           SET phase = 'd1_authoritative', authority = 'legacy', epoch = 1,
               updated_at_ms = 1 WHERE singleton = 1""",
        (),
    )
    expect_integrity_error(
        database,
        """INSERT INTO cutover_authority_scopes(
             tenant_id, domain, phase, writer, mirror_enabled, replay_paused,
             epoch, audit_head, rollback_ready, phase_started_at_ms, updated_at_ms
           ) VALUES ('invalid-initial-phase', 'metadata', 'd1_authoritative',
                     'd1', 1, 0, 0, ?, 1, 0, 0)""",
        (zero,),
    )
    dual_audit_hash = "7" * 64
    database.execute(
        """INSERT INTO cutover_authority_audit(
             audit_hash, previous_hash, tenant_id, domain, action,
             from_phase, to_phase, from_epoch, to_epoch,
             operator_digest, evidence_digest, occurred_at_ms
           ) VALUES (?, ?, 'migration-check', 'metadata', 'transition',
                     'shadow_read', 'dual_write', 1, 2, ?, ?, 2)""",
        (dual_audit_hash, audit_hash, "2" * 64, "3" * 64),
    )
    database.execute(
        """UPDATE cutover_authority_scopes
           SET phase = 'dual_write', mirror_enabled = 1, epoch = 2, phase_epoch = 2,
               audit_head = ?,
               phase_started_at_ms = 2, updated_at_ms = 2
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'""",
        (dual_audit_hash,),
    )
    change_insert = """INSERT INTO cutover_change_events(
         event_id, tenant_id, domain, sequence, authority_epoch, source_authority,
         event_digest, payload_ciphertext, state, occurred_at_ms, captured_at_ms
       ) VALUES (?, 'migration-check', 'metadata', ?, 2, ?, ?, 'fixture-ciphertext',
                 'pending', 2, 2)"""
    expect_integrity_error(
        database,
        change_insert,
        ("wrong-writer", 1, "d1", "8" * 64),
    )
    database.execute(change_insert, ("event-one", 1, "legacy", "8" * 64))
    database.execute(
        change_insert.replace("INSERT INTO", "INSERT OR IGNORE INTO", 1),
        ("event-one", 1, "legacy", "8" * 64),
    )
    expect_integrity_error(
        database,
        change_insert.replace("fixture-ciphertext", "changed-ciphertext").replace(
            "INSERT INTO", "INSERT OR IGNORE INTO", 1
        ),
        ("event-one", 1, "legacy", "8" * 64),
    )
    expect_integrity_error(
        database,
        change_insert,
        ("event-gap", 3, "legacy", "9" * 64),
    )
    database.execute(change_insert, ("event-two", 2, "legacy", "9" * 64))
    expect_integrity_error(
        database,
        """UPDATE cutover_change_events SET event_digest = ?
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'
             AND event_id = 'event-one'""",
        ("a" * 64,),
    )
    database.execute(
        """UPDATE cutover_change_events
           SET state = 'applied', applied_at_ms = 3
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'
             AND event_id = 'event-one'"""
    )
    expect_integrity_error(
        database,
        """UPDATE cutover_change_events SET state = 'pending', applied_at_ms = NULL
           WHERE tenant_id = 'migration-check' AND domain = 'metadata'
             AND event_id = 'event-one'""",
        (),
    )
    expect_integrity_error(
        database,
        "UPDATE cutover_authority_audit SET evidence_digest = ? WHERE audit_hash = ?",
        ("4" * 64, audit_hash),
    )
    expect_integrity_error(
        database,
        """INSERT INTO cutover_authority_audit(
             audit_hash, previous_hash, tenant_id, domain, action,
             from_phase, to_phase, from_epoch, to_epoch,
             operator_digest, evidence_digest, occurred_at_ms
           ) VALUES (?, ?, 'migration-check', 'metadata', 'transition',
                     'shadow_read', 'finalized', 1, 2, ?, ?, 2)""",
        ("5" * 64, audit_hash, "2" * 64, "3" * 64),
    )
    expect_integrity_error(
        database,
        """INSERT INTO cutover_authority_audit(
             audit_hash, previous_hash, tenant_id, domain, action,
             from_phase, to_phase, from_epoch, to_epoch,
             operator_digest, evidence_digest, occurred_at_ms
           ) VALUES (?, ?, 'migration-check', 'metadata', 'transition',
                     'shadow_read', 'dual_write', 1, 2, ?, ?, 2)""",
        ("6" * 64, zero, "2" * 64, "3" * 64),
    )
    database.execute(
        """INSERT INTO cutover_maintenance_windows(
             tenant_id, domain, starts_at_ms, ends_at_ms, approved_by_digest
           ) VALUES ('migration-check', 'metadata', 10, 20, ?)""",
        ("2" * 64,),
    )
    expect_integrity_error(
        database,
        """INSERT INTO cutover_maintenance_windows(
             tenant_id, domain, starts_at_ms, ends_at_ms, approved_by_digest
           ) VALUES ('migration-check', 'metadata', 20, 30, ?)""",
        ("2" * 64,),
    )
    database.rollback()


def verify_media_service_invariants(database: sqlite3.Connection) -> None:
    if database.execute("SELECT count(*) FROM media_profile_policies_v1").fetchone() != (16,):
        raise ValueError("media profile policy catalog does not contain exactly 16 rows")
    user = "018f47a6-7b1c-7f55-8f39-8f8a8690a001"
    tenant = "018f47a6-7b1c-7f55-8f39-8f8a8690a002"
    video = "018f47a6-7b1c-7f55-8f39-8f8a8690a003"
    job = "018f47a6-7b1c-7f55-8f39-8f8a8690a004"
    zero = "0" * 64
    one = "1" * 64
    two = "2" * 64
    source_key = f"tenants/{tenant}/videos/{video}/source/v1/payload"
    output_key = f"tenants/{tenant}/videos/{video}/derivatives/thumbnail_v1/{two}"
    database.execute(
        "INSERT INTO users(id,email,created_at_ms,updated_at_ms) VALUES (?,?,0,0)",
        (user, "media-migration@example.invalid"),
    )
    database.execute(
        "INSERT INTO organizations(id,owner_id,name,created_at_ms,updated_at_ms) VALUES (?,?,?,0,0)",
        (tenant, user, "Media migration check"),
    )
    database.execute(
        """INSERT INTO videos(
             id,owner_id,title,state,created_at_ms,updated_at_ms,organization_id
           ) VALUES (?,?,?,'ready',0,0,?)""",
        (video, user, "Synthetic media migration", tenant),
    )
    database.execute(
        """INSERT INTO object_manifests(
             object_key,video_id,role,bytes,checksum_sha256,content_type,created_at_ms,
             organization_id,object_version,state,updated_at_ms
           ) VALUES (?,?,'source',1024,?,'video/mp4',0,?,1,'available',0)""",
        (source_key, video, zero, tenant),
    )
    probe_insert = """INSERT INTO media_source_probes_v1(
         organization_id,video_id,source_version,source_object_key,source_checksum_sha256,
         source_bytes,source_content_type,container,video_codec,audio_codec,duration_ms,
         width,height,frame_rate_numerator,frame_rate_denominator,decoded_bytes_upper_bound,
         frame_count_upper_bound,track_count,probe_contract_version,probe_digest,trust,state,
         verified_at_ms,updated_at_ms
       ) VALUES (?,?,1,?,?,1024,'video/mp4','mp4','h264','aac',2000,640,360,30,1,
                 1000000,60,2,1,?,'verified_native_probe','verified',0,0)"""
    expect_integrity_error(
        database,
        probe_insert,
        (tenant, video, source_key, one, two),
    )
    database.execute(probe_insert, (tenant, video, source_key, zero, one))
    payload = (
        '{"schema_version":1,"tenant_id":"'
        + tenant
        + '","video_id":"'
        + video
        + '","source_version":1,"profile":"thumbnail_v1",'
          '"transform":{"schema_version":1,"profile_version":1,"mode":"frame",'
          '"start_ms":0,"duration_ms":null,"width":640,"height":360,"fit":"contain",'
          '"image_count":null,"include_audio":false,"format":"jpeg",'
          '"max_output_bytes":8000000}}'
    )
    database.execute(
        """INSERT INTO media_jobs(
             id,video_id,kind,state,idempotency_key,attempt,payload_json,created_at_ms,
             updated_at_ms,organization_id,selected_executor,source_version,
             profile_version,output_object_key,cancel_requested,revision
           ) VALUES (?,?,'frame','queued','media-migration-job',0,?,0,0,?,
                     'cloudflare_media',1,1,?,0,0)""",
        (job, video, payload, tenant, output_key),
    )
    execution_insert = """INSERT INTO media_job_execution_v1(
         job_id,organization_id,video_id,source_version,catalog_version,profile_id,
         profile_version,normalized_profile_sha256,route_reason,selected_executor,
         fallback_executor,state,attempt,lease_epoch,final_object_key,output_content_type,
         max_output_bytes,created_at_ms,updated_at_ms
       ) VALUES (?,?,?,1,1,'thumbnail_v1',1,?,'managed_preferred','cloudflare_media',
                 'native_gstreamer','queued',0,0,?,'image/jpeg',8000000,0,0)"""
    expect_integrity_error(
        database,
        execution_insert,
        (job, tenant, video, two, output_key + "-mismatch"),
    )
    database.execute(execution_insert, (job, tenant, video, two, output_key))
    manifest_json = (
        '{"schema_version":1,"job_id":"'
        + job
        + '","executor":"cloudflare_media","source_checksum_sha256":"'
        + zero
        + '","normalized_profile_sha256":"'
        + two
        + '","object_key":"'
        + output_key
        + '","object_checksum_sha256":"'
        + one
        + '","bytes":10,"content_type":"image/jpeg"}'
    )
    expect_integrity_error(
        database,
        """INSERT INTO media_output_manifests_v1(
             manifest_digest,job_id,organization_id,video_id,executor,
             source_checksum_sha256,normalized_profile_sha256,object_key,
             object_checksum_sha256,bytes,content_type,manifest_json,created_at_ms
           ) VALUES (?,?,?,?,'cloudflare_media',?,?,?,?,10,'image/jpeg',?,0)""",
        ("3" * 64, job, tenant, video, zero, two, output_key, one, manifest_json),
    )
    database.rollback()


def verify_instant_finalize_public_share_query_plan(database: sqlite3.Connection) -> None:
    index_name = "instant_finalize_requests_v1_public_share_latest_idx"
    index_columns = [
        (row[2], row[3])
        for row in database.execute(f"PRAGMA index_xinfo('{index_name}')")
        if row[5] == 1
    ]
    expected_columns = [
        ("organization_id", 0),
        ("video_id", 0),
        ("updated_at_ms", 1),
        ("session_id", 1),
        ("state", 0),
        ("last_failure_class", 0),
    ]
    if index_columns != expected_columns:
        raise ValueError(
            "instant-finalize public-share index shape drifted: "
            f"expected {expected_columns}, found {index_columns}"
        )

    query_plan = database.execute(
        """EXPLAIN QUERY PLAN
           SELECT
             (SELECT f.state FROM instant_finalize_requests_v1 f
               WHERE f.video_id=v.id AND f.organization_id=v.organization_id
               ORDER BY f.updated_at_ms DESC,f.session_id DESC LIMIT 1),
             (SELECT f.last_failure_class FROM instant_finalize_requests_v1 f
               WHERE f.video_id=v.id AND f.organization_id=v.organization_id
               ORDER BY f.updated_at_ms DESC,f.session_id DESC LIMIT 1)
           FROM videos v
           WHERE v.id = ?1 AND v.deleted_at_ms IS NULL LIMIT 1""",
        ("018f47a6-7b1c-7f55-8f39-8f8a8690a003",),
    ).fetchall()
    details = [str(row[3]) for row in query_plan]
    covering_lookups = [
        detail
        for detail in details
        if index_name in detail and "COVERING INDEX" in detail.upper()
    ]
    if len(covering_lookups) != 2:
        raise ValueError(
            "instant-finalize public-share projection must use the covering index twice: "
            + " | ".join(details)
        )
    if any("USE TEMP B-TREE" in detail.upper() for detail in details):
        raise ValueError(
            "instant-finalize public-share projection requires a temporary sort: "
            + " | ".join(details)
        )


def main() -> int:
    try:
        expand_files = discover()
        contract_files = discover_contract(expand_files)
        # Production applies the complete expand directory before the separately
        # approved contract directory. Validate that phase order here rather
        # than re-sorting both directories into numeric filename order, which
        # would silently model contract migrations running before later expands.
        files = expand_files + contract_files
        database = apply(files)
        tables = {
            row[0]
            for row in database.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
            )
        }
        missing = sorted(REQUIRED_TABLES - tables)
        if missing:
            raise ValueError(f"required tables missing after migration: {', '.join(missing)}")
        verify_scoped_cutover_invariants(database)
        verify_media_service_invariants(database)
        verify_instant_finalize_public_share_query_plan(database)

        # The only released baseline before this change is migration 0001. Apply
        # it independently, then prove the complete ordered upgrade path.
        baseline = apply(files[:1])
        for path in files[1:]:
            baseline.executescript(path.read_text(encoding="utf-8"))
        violations = baseline.execute("PRAGMA foreign_key_check").fetchall()
        if violations:
            raise ValueError("0001 upgrade path has foreign-key violations")
        verify_instant_finalize_public_share_query_plan(baseline)
    except (OSError, ValueError, sqlite3.Error) as error:
        print(f"migration validation failed: {error}", file=sys.stderr)
        return 1

    print(
        f"validated {len(expand_files)} expand migrations, "
        f"{len(contract_files)} protected contract migrations, and 0001 upgrade path"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
