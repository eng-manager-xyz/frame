#!/usr/bin/env python3
"""Exercise the capability-safe auth repository through a compiled local Worker."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import http.client
import json
import os
import pathlib
import re
import secrets
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
from collections import Counter
from collections.abc import Sequence
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
CONTROL = ROOT / "apps" / "control-plane"
MIGRATIONS = CONTROL / "migrations"
QUERIES = CONTROL / "queries" / "auth"
CONFIG = CONTROL / "wrangler.local.toml"
SOURCE = CONTROL / "src" / "auth_repository.rs"
SURFACE = CONTROL / "src" / "auth_repository_conformance.rs"
ROUTING = CONTROL / "src" / "routing.rs"
LIB = CONTROL / "src" / "lib.rs"
DATABASE = "frame-local"
WRANGLER_VERSION = "4.111.0"
CONFORMANCE_PATH = "/__frame/local/auth-repository-conformance"
TOKEN_HEADER = "x-frame-auth-repository-conformance-token"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
PLACEHOLDER = re.compile(r"\?([1-9][0-9]*)")
NOW_MS = 1_700_100_000_000

TELEMETRY_OPERATIONS = frozenset(
    {
        "session_issue",
        "session_authenticate",
        "session_rotate",
        "session_revoke",
        "session_logout_all",
        "verification_issue",
        "verification_materialize",
        "verification_attempt",
        "identity_provision",
        "api_key_issue",
        "api_key_authenticate",
        "api_key_revoke",
        "delivery_claim",
        "delivery_acknowledge",
        "delivery_retry",
    }
)
EXERCISED_TELEMETRY_COUNTS = {
    "session_issue": 1,
    "session_authenticate": 8,
    "session_rotate": 2,
    "session_revoke": 1,
    "session_logout_all": 1,
    "verification_issue": 8,
    "verification_materialize": 10,
    "verification_attempt": 6,
    "identity_provision": 2,
    "api_key_authenticate": 8,
    "api_key_issue": 1,
    "api_key_revoke": 1,
    "delivery_claim": 7,
    "delivery_acknowledge": 3,
    "delivery_retry": 4,
}

USER_FOUND = "018f47a6-7b1c-7f55-8f39-8f8a8690a101"
USER_SINGLE_LOGOUT = "018f47a6-7b1c-7f55-8f39-8f8a8690a102"
USER_EXPIRED = "018f47a6-7b1c-7f55-8f39-8f8a8690a201"
USER_REVOKED = "018f47a6-7b1c-7f55-8f39-8f8a8690a301"
USER_REPLAY = "018f47a6-7b1c-7f55-8f39-8f8a8690a401"
USER_LOGOUT = "018f47a6-7b1c-7f55-8f39-8f8a8690a501"
USER_API = "018f47a6-7b1c-7f55-8f39-8f8a8690a601"
USER_VERIFY = "018f47a6-7b1c-7f55-8f39-8f8a8690a801"
USER_PROVISION = "018f47a6-7b1c-7f55-8f39-8f8a8690a901"
USER_ROLLBACK = "018f47a6-7b1c-7f55-8f39-8f8a8690aa01"
USER_FENCE = "018f47a6-7b1c-7f55-8f39-8f8a8690a701"
USER_VERIFY_TWO = "018f47a6-7b1c-7f55-8f39-8f8a8690a802"
USER_VERIFY_THREE = "018f47a6-7b1c-7f55-8f39-8f8a8690a803"

SESSION_FOUND = "018f47a6-7b1c-7f55-8f39-8f8a8690b101"
SESSION_SINGLE_LOGOUT = "018f47a6-7b1c-7f55-8f39-8f8a8690b102"
SESSION_EXPIRED = "018f47a6-7b1c-7f55-8f39-8f8a8690b201"
SESSION_REVOKED = "018f47a6-7b1c-7f55-8f39-8f8a8690b301"
SESSION_REPLAY = "018f47a6-7b1c-7f55-8f39-8f8a8690b401"
SESSION_LOGOUT = "018f47a6-7b1c-7f55-8f39-8f8a8690b501"
SESSION_LOGOUT_TWO = "018f47a6-7b1c-7f55-8f39-8f8a8690b502"
SESSION_ROLLBACK = "018f47a6-7b1c-7f55-8f39-8f8a8690ba01"
SESSION_FENCE = "018f47a6-7b1c-7f55-8f39-8f8a8690b701"

ROTATE_GRANT = "018f47a6-7b1c-7f55-8f39-8f8a8690d101"
SINGLE_LOGOUT_GRANT = "018f47a6-7b1c-7f55-8f39-8f8a8690d102"
LOGOUT_GRANT = "018f47a6-7b1c-7f55-8f39-8f8a8690d501"
PROVISION_GRANT = "018f47a6-7b1c-7f55-8f39-8f8a8690d901"
ROLLBACK_GRANT = "018f47a6-7b1c-7f55-8f39-8f8a8690da01"
FENCE_GRANT = "018f47a6-7b1c-7f55-8f39-8f8a8690d701"
TENANT_API = "018f47a6-7b1c-7f55-8f39-8f8a8690e601"
TENANT_FENCE = "018f47a6-7b1c-7f55-8f39-8f8a8690e701"
DELIVERY_ID = "018f47a6-7b1c-7f55-8f39-8f8a8690f101"
DELIVERY_EXHAUST = "018f47a6-7b1c-7f55-8f39-8f8a8690f102"
DELIVERY_RACE = "018f47a6-7b1c-7f55-8f39-8f8a8690f103"
VERIFICATION_ID = "018f47a6-7b1c-7f55-8f39-8f8a8690f801"
OPERATION_ID = "018f47a6-7b1c-7f55-8f39-8f8a8690ff01"


class ConformanceFailure(RuntimeError):
    """Stable failure that never includes SQL, bindings, credentials, or provider text."""


def digest(seed: int) -> str:
    return f"{seed:064x}"


def sql_literal(value: object) -> str:
    if value is None:
        return "NULL"
    if isinstance(value, int):
        return str(value)
    if not isinstance(value, str) or "\x00" in value:
        raise ValueError("unsupported controlled fixture value")
    return "'" + value.replace("'", "''") + "'"


def user_statements(user_id: str, label: str) -> list[str]:
    return [
        "INSERT INTO users(id,email,display_name,created_at_ms,updated_at_ms) VALUES "
        f"({sql_literal(user_id)},{sql_literal(label + '@auth.invalid')},NULL,{NOW_MS - 10_000},{NOW_MS - 10_000})",
        "INSERT INTO auth_identities_v2(user_id,identity_revision,session_version,created_at_ms,updated_at_ms,revision,last_operation_id) VALUES "
        f"({sql_literal(user_id)},1,0,{NOW_MS - 10_000},{NOW_MS - 10_000},0,{sql_literal(OPERATION_ID)})",
    ]


def session_statements(
    session_id: str,
    family_id: str,
    user_id: str,
    seed: int,
    *,
    idle_expires_at: int,
    state: str = "active",
    credential_state: str = "current",
    revoked_at: int | None = None,
    reason: str | None = None,
) -> list[str]:
    return [
        "INSERT INTO auth_sessions_v2(id,family_id,user_id,client_kind,token_key_version,token_digest,csrf_key_version,csrf_digest,browser_origin,issued_at_ms,rotated_at_ms,idle_expires_at_ms,absolute_expires_at_ms,session_version,generation,state,revoked_at_ms,revocation_reason,revision,last_operation_id) VALUES "
        f"({sql_literal(session_id)},{sql_literal(family_id)},{sql_literal(user_id)},'api',1,{sql_literal(digest(seed))},NULL,NULL,NULL,{NOW_MS - 1_000},{NOW_MS - 1_000},{idle_expires_at},{max(idle_expires_at, NOW_MS + 10_000)},0,0,{sql_literal(state)},{sql_literal(revoked_at)},{sql_literal(reason)},0,{sql_literal(OPERATION_ID)})",
        "INSERT INTO auth_session_credentials_v2(key_version,digest,session_id,family_id,state,revision,last_operation_id) VALUES "
        f"(1,{sql_literal(digest(seed))},{sql_literal(session_id)},{sql_literal(family_id)},{sql_literal(credential_state)},0,{sql_literal(OPERATION_ID)})",
    ]


def fixture_statements() -> list[str]:
    values: list[str] = ["PRAGMA foreign_keys = ON"]
    users = [
        (USER_FOUND, "found"),
        (USER_SINGLE_LOGOUT, "single-logout"),
        (USER_EXPIRED, "expired"),
        (USER_REVOKED, "revoked"),
        (USER_REPLAY, "replay"),
        (USER_LOGOUT, "logout"),
        (USER_API, "api"),
        (USER_VERIFY, "verify"),
        (USER_ROLLBACK, "rollback"),
        (USER_FENCE, "fence"),
        (USER_VERIFY_TWO, "verify-two"),
        (USER_VERIFY_THREE, "verify-three"),
    ]
    for user_id, label in users:
        values.extend(user_statements(user_id, label))

    values.extend(
        session_statements(
            SESSION_FOUND,
            "018f47a6-7b1c-7f55-8f39-8f8a8690c101",
            USER_FOUND,
            11,
            idle_expires_at=NOW_MS + 5_000,
        )
    )
    values.extend(
        session_statements(
            SESSION_FENCE,
            "018f47a6-7b1c-7f55-8f39-8f8a8690c701",
            USER_FENCE,
            72,
            idle_expires_at=NOW_MS + 10_000,
        )
    )
    values.append(
        "INSERT INTO auth_session_mutation_grants_v2(id,session_id,user_id,generation,token_key_version,token_digest,created_at_ms,last_operation_id) VALUES "
        f"({sql_literal(FENCE_GRANT)},{sql_literal(SESSION_FENCE)},{sql_literal(USER_FENCE)},0,1,{sql_literal(digest(72))},{NOW_MS - 1},{sql_literal(OPERATION_ID)})"
    )
    values.append(
        "INSERT INTO auth_session_mutation_grants_v2(id,session_id,user_id,generation,token_key_version,token_digest,created_at_ms,last_operation_id) VALUES "
        f"({sql_literal(ROTATE_GRANT)},{sql_literal(SESSION_FOUND)},{sql_literal(USER_FOUND)},0,1,{sql_literal(digest(11))},{NOW_MS - 1},{sql_literal(OPERATION_ID)})"
    )
    values.extend(
        session_statements(
            SESSION_SINGLE_LOGOUT,
            "018f47a6-7b1c-7f55-8f39-8f8a8690c102",
            USER_SINGLE_LOGOUT,
            16,
            idle_expires_at=NOW_MS + 5_000,
        )
    )
    values.append(
        "INSERT INTO auth_session_mutation_grants_v2(id,session_id,user_id,generation,token_key_version,token_digest,created_at_ms,last_operation_id) VALUES "
        f"({sql_literal(SINGLE_LOGOUT_GRANT)},{sql_literal(SESSION_SINGLE_LOGOUT)},{sql_literal(USER_SINGLE_LOGOUT)},0,1,{sql_literal(digest(16))},{NOW_MS - 1},{sql_literal(OPERATION_ID)})"
    )
    values.extend(
        session_statements(
            SESSION_EXPIRED,
            "018f47a6-7b1c-7f55-8f39-8f8a8690c201",
            USER_EXPIRED,
            21,
            idle_expires_at=NOW_MS,
        )
    )
    values.extend(
        session_statements(
            SESSION_REVOKED,
            "018f47a6-7b1c-7f55-8f39-8f8a8690c301",
            USER_REVOKED,
            31,
            idle_expires_at=NOW_MS + 5_000,
            state="revoked",
            credential_state="revoked",
            revoked_at=NOW_MS - 10,
            reason="operator",
        )
    )
    replay_family = "018f47a6-7b1c-7f55-8f39-8f8a8690c401"
    values.extend(
        session_statements(
            SESSION_REPLAY,
            replay_family,
            USER_REPLAY,
            42,
            idle_expires_at=NOW_MS + 5_000,
        )
    )
    values.append(
        "INSERT INTO auth_session_credentials_v2(key_version,digest,session_id,family_id,state,revision,last_operation_id) VALUES "
        f"(1,{sql_literal(digest(41))},{sql_literal(SESSION_REPLAY)},{sql_literal(replay_family)},'rotated',0,{sql_literal(OPERATION_ID)})"
    )
    logout_family = "018f47a6-7b1c-7f55-8f39-8f8a8690c501"
    values.extend(
        session_statements(
            SESSION_LOGOUT,
            logout_family,
            USER_LOGOUT,
            51,
            idle_expires_at=NOW_MS + 5_000,
        )
    )
    values.extend(
        session_statements(
            SESSION_LOGOUT_TWO,
            "018f47a6-7b1c-7f55-8f39-8f8a8690c502",
            USER_LOGOUT,
            52,
            idle_expires_at=NOW_MS + 5_000,
        )
    )
    values.append(
        "INSERT INTO auth_session_mutation_grants_v2(id,session_id,user_id,generation,token_key_version,token_digest,created_at_ms,last_operation_id) VALUES "
        f"({sql_literal(LOGOUT_GRANT)},{sql_literal(SESSION_LOGOUT)},{sql_literal(USER_LOGOUT)},0,1,{sql_literal(digest(51))},{NOW_MS - 1},{sql_literal(OPERATION_ID)})"
    )

    values.extend(
        [
            "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,revision) VALUES "
            f"({sql_literal(TENANT_API)},{sql_literal(USER_API)},'Auth Tenant','active','{{}}',{NOW_MS - 10_000},{NOW_MS - 10_000},0)",
            "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms,revision) VALUES "
            f"({sql_literal(TENANT_API)},{sql_literal(USER_API)},'owner','active',1,{NOW_MS - 10_000},{NOW_MS - 10_000},0)",
            "INSERT INTO auth_api_keys_v2(id,owner_id,tenant_id,key_version,key_digest,scopes_json,created_at_ms,expires_at_ms,revoked_at_ms,revision,last_operation_id) VALUES "
            f"('018f47a6-7b1c-7f55-8f39-8f8a8690f601',{sql_literal(USER_API)},{sql_literal(TENANT_API)},1,{sql_literal(digest(61))},'[\"videos_read\"]',{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_api_keys_v2(id,owner_id,tenant_id,key_version,key_digest,scopes_json,created_at_ms,expires_at_ms,revoked_at_ms,revision,last_operation_id) VALUES "
            f"('018f47a6-7b1c-7f55-8f39-8f8a8690f701',{sql_literal(USER_API)},{sql_literal(TENANT_API)},1,{sql_literal(digest(71))},'[\"videos_read\",\"videos_read\"]',{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO organizations(id,owner_id,name,status,settings_json,created_at_ms,updated_at_ms,revision) VALUES "
            f"({sql_literal(TENANT_FENCE)},{sql_literal(USER_FENCE)},'Fence Tenant','active','{{}}',{NOW_MS - 10_000},{NOW_MS - 10_000},0)",
            "INSERT INTO organization_members(organization_id,user_id,role,state,has_pro_seat,created_at_ms,updated_at_ms,revision) VALUES "
            f"({sql_literal(TENANT_FENCE)},{sql_literal(USER_FENCE)},'owner','active',1,{NOW_MS - 10_000},{NOW_MS - 10_000},0)",
            "INSERT INTO auth_api_keys_v2(id,owner_id,tenant_id,key_version,key_digest,scopes_json,created_at_ms,expires_at_ms,revoked_at_ms,revision,last_operation_id) VALUES "
            f"('018f47a6-7b1c-7f55-8f39-8f8a8690f702',{sql_literal(USER_FENCE)},{sql_literal(TENANT_FENCE)},1,{sql_literal(digest(73))},'[\"videos_read\"]',{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,0,{sql_literal(OPERATION_ID)})",
        ]
    )

    values.extend(
        [
            "INSERT INTO auth_verification_challenges_v2(id,user_id,initiator_session_id,initiator_user_id,initiator_generation,provisioning_revision,identifier_key_version,identifier_digest,secret_key_version,secret_digest,purpose,channel,attempt_count,max_attempts,created_at_ms,expires_at_ms,consumed_at_ms,state,revision,last_operation_id) VALUES "
            f"({sql_literal(VERIFICATION_ID)},{sql_literal(USER_VERIFY)},NULL,NULL,NULL,NULL,1,{sql_literal(digest(81))},1,{sql_literal(digest(83))},'sign_in','one_time_code',0,5,{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,'pending',0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_verification_challenges_v2(id,user_id,initiator_session_id,initiator_user_id,initiator_generation,provisioning_revision,identifier_key_version,identifier_digest,secret_key_version,secret_digest,purpose,channel,attempt_count,max_attempts,created_at_ms,expires_at_ms,consumed_at_ms,state,revision,last_operation_id) VALUES "
            f"('018f47a6-7b1c-7f55-8f39-8f8a8690f802',{sql_literal(USER_VERIFY_TWO)},NULL,NULL,NULL,NULL,1,{sql_literal(digest(85))},1,{sql_literal(digest(87))},'sign_in','one_time_code',0,5,{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,'pending',0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_verification_challenges_v2(id,user_id,initiator_session_id,initiator_user_id,initiator_generation,provisioning_revision,identifier_key_version,identifier_digest,secret_key_version,secret_digest,purpose,channel,attempt_count,max_attempts,created_at_ms,expires_at_ms,consumed_at_ms,state,revision,last_operation_id) VALUES "
            f"('018f47a6-7b1c-7f55-8f39-8f8a8690f803',{sql_literal(USER_VERIFY_THREE)},NULL,NULL,NULL,NULL,1,{sql_literal(digest(86))},1,{sql_literal(digest(88))},'sign_in','one_time_code',0,5,{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,'pending',0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_verification_challenges_v2(id,user_id,initiator_session_id,initiator_user_id,initiator_generation,provisioning_revision,identifier_key_version,identifier_digest,secret_key_version,secret_digest,purpose,channel,attempt_count,max_attempts,created_at_ms,expires_at_ms,consumed_at_ms,state,revision,last_operation_id) VALUES "
            f"('018f47a6-7b1c-7f55-8f39-8f8a8690f804',{sql_literal(USER_FENCE)},{sql_literal(SESSION_FENCE)},{sql_literal(USER_FENCE)},0,NULL,1,{sql_literal(digest(76))},1,{sql_literal(digest(77))},'account_link','one_time_code',0,5,{NOW_MS - 1_000},{NOW_MS + 10_000},NULL,'pending',0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_identity_provisioning_grants_v2(id,user_id,identity_revision,identifier_key_version,identifier_digest,expires_at_ms,created_at_ms,last_operation_id) VALUES "
            f"({sql_literal(PROVISION_GRANT)},{sql_literal(USER_PROVISION)},1,1,{sql_literal(digest(91))},{NOW_MS + 10_000},{NOW_MS - 1},{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_principal_issuance_grants_v2(id,user_id,identity_revision,expires_at_ms,created_at_ms,last_operation_id) VALUES "
            f"({sql_literal(ROLLBACK_GRANT)},{sql_literal(USER_ROLLBACK)},1,{NOW_MS + 10_000},{NOW_MS - 1},{sql_literal(OPERATION_ID)})",
        ]
    )

    identifier_json = json.dumps(
        [{"key_version": 1, "digest": digest(111)}], separators=(",", ":")
    )
    values.extend(
        [
            "INSERT INTO auth_identifier_digests_v2(key_version,digest,user_id,created_at_ms,last_operation_id) VALUES "
            f"(1,{sql_literal(digest(111))},{sql_literal(USER_FOUND)},{NOW_MS - 1_000},{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_pending_verifications_v2(delivery_id,identifier_candidates_json,active_identifier_key_version,active_identifier_digest,secret_key_version,secret_digest,purpose,channel,initiator_session_id,initiator_user_id,initiator_generation,provisioning_user_id,provisioning_revision,max_attempts,created_at_ms,expires_at_ms,sealed_payload_hex,revision,last_operation_id) VALUES "
            f"({sql_literal(DELIVERY_ID)},{sql_literal(identifier_json)},1,{sql_literal(digest(111))},1,{sql_literal(digest(112))},'sign_in','one_time_code',NULL,NULL,NULL,NULL,NULL,5,{NOW_MS},{NOW_MS + 10_000},{sql_literal('ab' * 32)},0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_delivery_outbox_v2(delivery_id,sealed_payload_hex,suppress,created_at_ms,expires_at_ms,next_attempt_at_ms,attempt,lease_id,lease_expires_at_ms,initiator_session_id,revision,last_operation_id) VALUES "
            f"({sql_literal(DELIVERY_EXHAUST)},{sql_literal('cd' * 32)},0,{NOW_MS},{NOW_MS + 10_000},{NOW_MS + 1_000},11,NULL,NULL,NULL,0,{sql_literal(OPERATION_ID)})",
            "INSERT INTO auth_delivery_outbox_v2(delivery_id,sealed_payload_hex,suppress,created_at_ms,expires_at_ms,next_attempt_at_ms,attempt,lease_id,lease_expires_at_ms,initiator_session_id,revision,last_operation_id) VALUES "
            f"({sql_literal(DELIVERY_RACE)},{sql_literal('ef' * 32)},0,{NOW_MS},{NOW_MS + 10_000},{NOW_MS + 5_000},0,NULL,NULL,NULL,0,{sql_literal(OPERATION_ID)})",
            "CREATE TRIGGER conformance_reject_auth_audit BEFORE INSERT ON auth_audit_events_v2 "
            f"WHEN NEW.user_id={sql_literal(USER_ROLLBACK)} AND NEW.action='session_issue' "
            "BEGIN SELECT RAISE(ABORT,'provider frame_auth_cas_conflict_v1 spoof'); END",
        ]
    )
    for action, dimension, seed in [
        ("sign_in_issue", "identifier", 980),
        ("sign_in_issue", "source", 981),
        ("sign_in_issue", "device", 982),
        ("recover_issue", "identifier", 990),
        ("recover_issue", "source", 991),
        ("recover_issue", "device", 992),
    ]:
        values.append(
            "INSERT INTO auth_rate_limit_buckets_v2(action,dimension,key_version,digest,window_started_at_ms,attempt_count,blocked_until_ms,updated_at_ms,gc_at_ms,revision,last_operation_id) VALUES "
            f"({sql_literal(action)},{sql_literal(dimension)},1,{sql_literal(digest(seed))},{NOW_MS - 1_000},0,NULL,{NOW_MS - 1_000},{NOW_MS + 60_000},0,{sql_literal(OPERATION_ID)})"
        )
    values.append(
        "WITH digits(value) AS (VALUES (0),(1),(2),(3),(4),(5),(6),(7),(8),(9)), "
        "numbers(value) AS ("
        "SELECT ones.value + 10*tens.value + 100*hundreds.value + 1000*thousands.value "
        "FROM digits ones CROSS JOIN digits tens CROSS JOIN digits hundreds CROSS JOIN digits thousands"
        ") "
        "INSERT INTO auth_rate_limit_buckets_v2(action,dimension,key_version,digest,window_started_at_ms,attempt_count,blocked_until_ms,updated_at_ms,gc_at_ms,revision,last_operation_id) "
        f"SELECT 'oauth_begin','identifier',1,printf('%064x',100000 + value),{NOW_MS - 1_000},0,NULL,{NOW_MS - 1_000},{NOW_MS + 60_000},0,{sql_literal(OPERATION_ID)} "
        "FROM numbers WHERE value BETWEEN 0 AND 4088"
    )
    return values


def migration_files() -> list[pathlib.Path]:
    files = sorted(MIGRATIONS.glob("[0-9][0-9][0-9][0-9]_*.sql"))
    if [int(path.name[:4]) for path in files] != list(range(1, len(files) + 1)):
        raise ConformanceFailure("migration sequence is not contiguous")
    return files


def compile_checked_in_sql() -> None:
    database = sqlite3.connect(":memory:")
    try:
        database.execute("PRAGMA foreign_keys = ON")
        for path in migration_files():
            database.executescript(path.read_text(encoding="utf-8"))
        queries = sorted(QUERIES.glob("*.sql"))
        if len(queries) < 60:
            raise ConformanceFailure("auth query inventory is incomplete")
        for path in queries:
            sql = path.read_text(encoding="utf-8").strip()
            indexes = [int(match) for match in PLACEHOLDER.findall(sql)]
            if not indexes:
                raise ConformanceFailure("auth query has no bound contract")
            database.execute("EXPLAIN " + sql, [None] * max(indexes)).fetchall()
        columns = {
            str(row[1]).lower()
            for table in database.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'auth_%_v2'"
            ).fetchall()
            for row in database.execute(f"PRAGMA table_info({table[0]})").fetchall()
        }
        forbidden = {
            "raw_token",
            "raw_otp",
            "api_key_plaintext",
            "oauth_code",
            "cookie",
            "destination",
        }
        if columns & forbidden:
            raise ConformanceFailure("plaintext authentication column detected")
    except sqlite3.Error as error:
        raise ConformanceFailure("checked-in auth SQL did not compile against migrations") from error
    finally:
        database.close()


def verify_compiled_surface() -> None:
    source = SOURCE.read_text(encoding="utf-8")
    surface = "\n".join(
        path.read_text(encoding="utf-8") for path in (SURFACE, ROUTING, LIB)
    )
    if source.count("Err(unsupported_oauth())") != 3:
        raise ConformanceFailure("protected OAuth boundary drifted")
    for forbidden in (
        "d1_verification_repository_not_initialized",
        "d1_delivery_repository_not_initialized",
        "todo!",
        "unimplemented!",
    ):
        if forbidden in source.lower():
            raise ConformanceFailure("auth repository contains an unfinished path")
    for marker in (
        CONFORMANCE_PATH,
        "FRAME_AUTH_REPOSITORY_CONFORMANCE_TOKEN",
        "config.production() || !valid_repository_conformance_target",
        "Route::LocalAuthRepositoryConformance",
    ):
        if marker not in surface:
            raise ConformanceFailure("compiled auth conformance surface drifted")
    for operation in TELEMETRY_OPERATIONS:
        marker = f'AuthRepositoryTelemetry::span("{operation}")'
        direct_marker = f'AuthRepositoryTelemetry::emit("{operation}"'
        if marker not in source and direct_marker not in source:
            raise ConformanceFailure("auth repository telemetry coverage drifted")
    batch_start = source.find("    async fn batch(&self,")
    batch_end = source.find("    async fn settled_rows", batch_start)
    if batch_start < 0 or batch_end < 0:
        raise ConformanceFailure("auth mutation settlement boundary drifted")
    mutation_batch = source[batch_start:batch_end]
    if ".settle_d1(self.database.batch(statements).into_send())" not in mutation_batch:
        raise ConformanceFailure("auth mutations no longer settle their D1 promise")
    if "await_d1" in mutation_batch or "Delay::from" in mutation_batch:
        raise ConformanceFailure("auth mutation batch acquired a local deadline")


def parse_wrangler_json(output: str) -> Any:
    try:
        return json.loads(ANSI.sub("", output).strip())
    except json.JSONDecodeError as error:
        raise ConformanceFailure("Wrangler did not return valid JSON") from error


def detect_wrangler(explicit: str | None) -> list[str]:
    command = (
        (["node", explicit] if explicit and explicit.endswith(".js") else [explicit])
        if explicit
        else ["npx", "--yes", f"wrangler@{WRANGLER_VERSION}"]
    )
    environment = os.environ.copy()
    environment.update({"NO_COLOR": "1", "WRANGLER_LOG_PATH": "/tmp/frame-auth-wrangler-version.log"})
    result = subprocess.run(
        [*command, "--version"],
        cwd=ROOT,
        env=environment,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    if result.returncode != 0 or ANSI.sub("", result.stdout).strip() != WRANGLER_VERSION:
        raise ConformanceFailure(f"Wrangler {WRANGLER_VERSION} is required")
    return command


def refuse_external_authority() -> None:
    forbidden = [
        name
        for name in ("CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID", "DATABASE_URL")
        if os.environ.get(name)
    ]
    if os.environ.get("FRAME_DEPLOYMENT") == "production":
        forbidden.append("FRAME_DEPLOYMENT")
    if forbidden:
        raise ConformanceFailure("local auth conformance refused external authority variables")


class WranglerD1:
    def __init__(self, command: list[str], state: pathlib.Path) -> None:
        self.command = command
        self.state = state
        self.environment = os.environ.copy()
        self.environment.update(
            {
                "CI": "true",
                "NO_COLOR": "1",
                "WRANGLER_LOG_PATH": str(state / "wrangler-cli.log"),
            }
        )

    def run(
        self,
        arguments: Sequence[str],
        *,
        timeout: float = 90,
        command_class: str = "wrangler_command",
    ) -> subprocess.CompletedProcess[str]:
        result = subprocess.run(
            [*self.command, *arguments],
            cwd=ROOT,
            env=self.environment,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        if result.returncode != 0:
            if not re.fullmatch(r"[a-z_]+", command_class):
                command_class = "invalid_command_class"
            raise ConformanceFailure(
                f"local Wrangler auth command failed: {command_class}:rc={result.returncode}"
            )
        return result

    def migrate(self) -> None:
        self.run(
            [
                "d1",
                "migrations",
                "apply",
                DATABASE,
                "--local",
                "--persist-to",
                str(self.state),
                "--config",
                str(CONFIG),
            ],
            command_class="migration_apply",
        )

    def execute_file(self, path: pathlib.Path) -> None:
        result = self.run(
            [
                "d1",
                "execute",
                DATABASE,
                "--local",
                "--persist-to",
                str(self.state),
                "--config",
                str(CONFIG),
                "--file",
                str(path),
                "--json",
            ],
            command_class="fixture_load",
        )
        payload = parse_wrangler_json(result.stdout)
        if not isinstance(payload, list) or not all(
            isinstance(item, dict) and item.get("success") is True for item in payload
        ):
            raise ConformanceFailure("local auth fixture load failed")

    def query(self, sql: str, *, command_class: str) -> list[dict[str, Any]]:
        arguments = [
            "d1",
            "execute",
            DATABASE,
            "--local",
            "--persist-to",
            str(self.state),
            "--config",
            str(CONFIG),
            "--json",
            "--command",
            sql,
        ]
        for attempt in range(4):
            try:
                result = self.run(arguments, command_class=command_class)
                break
            except ConformanceFailure:
                if attempt == 3:
                    raise
                # The terminated local Worker may briefly retain the isolated
                # SQLite file. The command and SQL remain identical.
                time.sleep(0.25 * (attempt + 1))
        payload = parse_wrangler_json(result.stdout)
        if (
            not isinstance(payload, list)
            or len(payload) != 1
            or payload[0].get("success") is not True
            or not isinstance(payload[0].get("results"), list)
        ):
            raise ConformanceFailure("local auth query result shape changed")
        return json.loads(json.dumps(payload[0]["results"], sort_keys=True))


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


class WorkerServer:
    def __init__(self, d1: WranglerD1, token: str, root: pathlib.Path) -> None:
        self.d1 = d1
        self.token = token
        self.port = reserve_port()
        self.log_path = root / "worker.log"
        self.process: subprocess.Popen[str] | None = None
        self.log_file: Any = None

    def start(self) -> None:
        self.log_file = self.log_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [
                *self.d1.command,
                "dev",
                "--local",
                "--persist-to",
                str(self.d1.state),
                "--config",
                str(CONFIG),
                "--ip",
                "127.0.0.1",
                "--port",
                str(self.port),
                "--var",
                f"FRAME_AUTH_REPOSITORY_CONFORMANCE_TOKEN:{self.token}",
            ],
            cwd=ROOT,
            env=self.d1.environment,
            stdin=subprocess.DEVNULL,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
            text=True,
        )
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise ConformanceFailure("local auth Worker exited before becoming ready")
            try:
                connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=1)
                connection.request("GET", "/health")
                response = connection.getresponse()
                response.read()
                connection.close()
                return
            except OSError:
                time.sleep(0.2)
        raise ConformanceFailure("local auth Worker did not become ready")

    def stop(self) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        if self.log_file is not None:
            self.log_file.close()

    def request(
        self,
        scenario: str,
        *,
        token: str | None = None,
        path: str = CONFORMANCE_PATH,
        host: str | None = None,
        timeout: float = 30,
    ) -> tuple[int, dict[str, Any]]:
        body = json.dumps({"schema_version": 1, "scenario": scenario}, separators=(",", ":"))
        headers = {
            "content-type": "application/json",
            TOKEN_HEADER: self.token if token is None else token,
        }
        if host is not None:
            headers["host"] = host
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=timeout)
        connection.request("POST", path, body=body, headers=headers)
        response = connection.getresponse()
        raw = response.read()
        status = response.status
        connection.close()
        try:
            payload = json.loads(raw)
        except (json.JSONDecodeError, UnicodeDecodeError) as error:
            raise ConformanceFailure("local auth Worker returned invalid JSON") from error
        if not isinstance(payload, dict):
            raise ConformanceFailure("local auth Worker response shape changed")
        return status, payload


def expect_scenario(server: WorkerServer, scenario: str) -> dict[str, Any]:
    status, payload = server.request(scenario)
    if status != 200 or payload.get("outcome") != "ok":
        outcome = payload.get("outcome")
        safe_outcome = outcome if isinstance(outcome, str) and re.fullmatch(r"[a-z_]+", outcome) else "invalid"
        raise ConformanceFailure(
            f"compiled auth scenario returned an unexpected outcome: {scenario}:{status}:{safe_outcome}"
        )
    details = payload.get("details")
    if not isinstance(details, dict) or details.get("scenario") != scenario:
        raise ConformanceFailure("compiled auth scenario response was incomplete")
    values = details.get("values")
    if not isinstance(values, dict):
        raise ConformanceFailure("compiled auth scenario values were incomplete")
    return values


def exercise_worker(server: WorkerServer) -> None:
    status, _ = server.request("session_matrix", token=secrets.token_hex(32))
    if status != 404:
        raise ConformanceFailure("auth conformance token did not fail closed")
    status, _ = server.request("session_matrix", path=CONFORMANCE_PATH + "/")
    if status != 404:
        raise ConformanceFailure("auth conformance path was not exact")
    status, _ = server.request("session_matrix", host=f"localhost:{server.port}")
    if status != 404:
        raise ConformanceFailure("auth conformance accepted a non-exact loopback host")

    expected = {
        "verification_issue_near_cap": {
            "accepted": 1,
            "rate_limited": 1,
            "replay_exact": True,
            "challenge_count": 1,
            "delivery_count": 1,
        },
        "verification_issue_existing_bucket": {
            "accepted": 2,
            "replay_exact": True,
            "channels": ["one_time_code", "magic_link"],
            "challenge_count": 2,
            "delivery_count": 2,
        },
        "session_matrix": {
            "found": "authenticated",
            "found_retry": "authenticated",
            "semantic_mismatch": "invalid_request",
            "not_found": "unknown",
            "expired": "expired_and_revoked",
            "revoked": "revoked",
            "replay": "family_revoked",
            "rotation": "rotated_and_reconstructed",
            "logout": "revoked",
        },
        "logout_all": {"session_version": 1, "revoked_sessions": 2},
        "api_key_rotation_and_corrupt": {
            "fallback": "migrated",
            "retry": "migrated",
            "active": "persisted",
            "corrupt": "fail_closed",
        },
        "verification_replay": {
            "first": "verified",
            "retry": "verified",
            "stable_grant": True,
            "second": "replay_detected",
        },
        "delivery_lifecycle": {
            "materialized": 1,
            "attempts": 2,
            "stale_lease": True,
            "retry_idempotent": True,
            "ack_idempotent": True,
        },
        "atomic_rollback": {
            "result": "unavailable",
            "grant_restored": True,
            "session_absent": True,
        },
        "authority_fences": {
            "membership_suspension": "denied",
            "user_suspension": "session_revoked",
            "organization_tombstone": "denied",
            "key_unchanged": True,
            "membership_removal": "denied",
            "downgraded_issue": "forbidden_without_grant_spend",
            "downgraded_revoke": "forbidden_without_grant_spend",
            "suspended_link": "denied_without_identifier",
        },
        "delivery_lease_cleanup": {
            "active_attempt_twelve_lease_preserved": True,
            "exhausted": True,
            "retry_idempotent": True,
            "tombstone": True,
        },
        "contention_retries": {
            "verification": ["verified", "verified"],
            "api_key": ["authenticated", "authenticated"],
        },
    }
    for scenario, values in expected.items():
        observed = expect_scenario(server, scenario)
        if observed != values:
            safe_observed = json.dumps(observed, sort_keys=True, separators=(",", ":"))
            raise ConformanceFailure(
                f"compiled auth scenario values changed: {scenario}:{safe_observed}"
            )

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        futures = [executor.submit(server.request, "provision_race", timeout=30) for _ in range(2)]
        outcomes = [future.result(timeout=35) for future in futures]
    if sorted(status for status, _ in outcomes) != [200, 200]:
        safe = [
            {
                "status": status,
                "outcome": str(payload.get("outcome", "invalid")),
            }
            for status, payload in outcomes
        ]
        raise ConformanceFailure(
            "auth provisioning race did not return two decisions: "
            + json.dumps(safe, sort_keys=True, separators=(",", ":"))
        )
    results = sorted(
        str(payload.get("details", {}).get("values", {}).get("result"))
        for _, payload in outcomes
    )
    if results != ["created", "replay_detected"]:
        raise ConformanceFailure("auth provisioning race did not yield one winner")

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        futures = [executor.submit(server.request, "claim_race", timeout=30) for _ in range(2)]
        claim_outcomes = [future.result(timeout=35) for future in futures]
    if sorted(status for status, _ in claim_outcomes) != [200, 200]:
        raise ConformanceFailure("auth delivery claim race did not return two decisions")
    claim_results = sorted(
        str(payload.get("details", {}).get("values", {}).get("result"))
        for _, payload in claim_outcomes
    )
    if claim_results != ["claimed", "empty"]:
        raise ConformanceFailure("auth delivery claim race did not yield one lease owner")


def assert_final_state(d1: WranglerD1) -> None:
    if d1.query("SELECT 1 AS ready", command_class="final_probe") != [{"ready": 1}]:
        raise ConformanceFailure("auth final inspection probe changed")
    session_rows = d1.query(
        "SELECT "
        f"(SELECT state FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_EXPIRED)}) AS expired_state,"
        f"(SELECT revocation_reason FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_EXPIRED)}) AS expired_reason,"
        f"(SELECT state FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_REPLAY)}) AS replay_state,"
        f"(SELECT state FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_FOUND)}) AS rotated_state,"
        f"(SELECT generation FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_FOUND)}) AS rotated_generation,"
        f"(SELECT token_key_version FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_FOUND)}) AS rotated_key_version,"
        f"(SELECT token_digest FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_FOUND)}) AS rotated_digest,"
        f"(SELECT csrf_digest FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_FOUND)}) AS rotated_csrf_digest,"
        f"(SELECT COUNT(*) FROM auth_sessions_v2 WHERE family_id=(SELECT family_id FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_FOUND)}) AND state='active') AS rotated_family_active,"
        f"(SELECT state FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_SINGLE_LOGOUT)}) AS single_logout_state,"
        f"(SELECT session_version FROM auth_identities_v2 WHERE user_id={sql_literal(USER_LOGOUT)}) AS logout_version,"
        f"(SELECT COUNT(*) FROM auth_sessions_v2 WHERE user_id={sql_literal(USER_LOGOUT)} AND state='active') AS logout_active",
        command_class="final_sessions",
    )
    api_rows = d1.query(
        "SELECT "
        f"(SELECT key_version FROM auth_api_keys_v2 WHERE owner_id={sql_literal(USER_API)} AND id LIKE '%f601') AS api_version,"
        f"(SELECT key_digest FROM auth_api_keys_v2 WHERE owner_id={sql_literal(USER_API)} AND id LIKE '%f601') AS api_digest,"
        f"(SELECT key_version FROM auth_api_keys_v2 WHERE owner_id={sql_literal(USER_FENCE)}) AS fence_key_version,"
        f"(SELECT key_digest FROM auth_api_keys_v2 WHERE owner_id={sql_literal(USER_FENCE)}) AS fence_key_digest,"
        f"(SELECT status FROM users WHERE id={sql_literal(USER_FENCE)}) AS fence_user_status,"
        f"(SELECT status FROM organizations WHERE id={sql_literal(TENANT_FENCE)}) AS fence_org_status",
        command_class="final_api",
    )
    continuation_rows = d1.query(
        "SELECT "
        f"(SELECT state FROM auth_verification_challenges_v2 WHERE id={sql_literal(VERIFICATION_ID)}) AS verification_state,"
        f"(SELECT COUNT(*) FROM auth_verification_challenges_v2 WHERE id IN ('018f47a6-7b1c-7f55-8f39-8f8a8690f802','018f47a6-7b1c-7f55-8f39-8f8a8690f803') AND state='consumed') AS contention_verifications,"
        f"(SELECT COUNT(*) FROM users WHERE id={sql_literal(USER_PROVISION)}) AS provisioned_user,"
        f"(SELECT COUNT(*) FROM auth_principal_issuance_grants_v2 WHERE id={sql_literal(ROLLBACK_GRANT)}) AS rollback_grant,"
        f"(SELECT COUNT(*) FROM auth_sessions_v2 WHERE id={sql_literal(SESSION_ROLLBACK)}) AS rollback_session,"
        f"(SELECT COUNT(*) FROM auth_session_mutation_grants_v2 WHERE id={sql_literal(FENCE_GRANT)}) AS fence_grant,"
        f"(SELECT COUNT(*) FROM auth_identifier_digests_v2 WHERE key_version=1 AND digest={sql_literal(digest(76))}) AS suspended_link_identifiers,"
        f"(SELECT COUNT(*) FROM auth_delivery_outbox_v2 WHERE delivery_id={sql_literal(DELIVERY_ID)}) AS delivery_count,"
        f"(SELECT COUNT(*) FROM auth_delivery_outbox_v2 WHERE delivery_id={sql_literal(DELIVERY_EXHAUST)}) AS exhausted_delivery_count,"
        f"(SELECT COUNT(*) FROM auth_delivery_ack_tombstones_v2 WHERE delivery_id={sql_literal(DELIVERY_EXHAUST)}) AS exhausted_tombstone_count,"
        f"(SELECT COUNT(*) FROM auth_delivery_outbox_v2 WHERE delivery_id={sql_literal(DELIVERY_RACE)} AND attempt=1 AND lease_id IS NOT NULL) AS claim_race_owner_count,"
        "(SELECT COUNT(*) FROM auth_repository_assertions_v2) AS assertion_count",
        command_class="final_continuations",
    )
    if len(session_rows) != 1 or len(api_rows) != 1 or len(continuation_rows) != 1:
        raise ConformanceFailure("auth repository final state shape changed")
    invariants = {**session_rows[0], **api_rows[0], **continuation_rows[0]}
    expected = {
        "expired_state": "revoked",
        "expired_reason": "expired",
        "replay_state": "revoked",
        "rotated_state": "active",
        "rotated_generation": 1,
        "rotated_key_version": 2,
        "rotated_digest": digest(12),
        "rotated_csrf_digest": None,
        "rotated_family_active": 1,
        "single_logout_state": "revoked",
        "logout_version": 1,
        "logout_active": 0,
        "api_version": 2,
        "api_digest": digest(62),
        "fence_key_version": 1,
        "fence_key_digest": digest(73),
        "fence_user_status": "active",
        "fence_org_status": "tombstoned",
        "verification_state": "consumed",
        "contention_verifications": 2,
        "provisioned_user": 1,
        "rollback_grant": 1,
        "rollback_session": 0,
        "fence_grant": 1,
        "suspended_link_identifiers": 0,
        "delivery_count": 0,
        "exhausted_delivery_count": 0,
        "exhausted_tombstone_count": 1,
        "claim_race_owner_count": 1,
        "assertion_count": 0,
    }
    if invariants != expected:
        raise ConformanceFailure("auth repository final state invariant changed")
    audit = d1.query(
        "SELECT action,outcome,reason,COUNT(*) AS count FROM auth_audit_events_v2 "
        "GROUP BY action,outcome,reason ORDER BY action,outcome,reason",
        command_class="final_audit",
    )
    required = {
        ("api_key_authenticate", "allow", "key_version_migrated"),
        ("api_key_issue", "deny", "insufficient_role"),
        ("api_key_revoke", "deny", "insufficient_role"),
        ("account_link", "deny", "invalid_credential"),
        ("identity_provision", "allow", "issued"),
        ("identity_provision", "deny", "replay_detected"),
        ("logout_all", "allow", "logged_out_all"),
        ("logout", "allow", "logged_out"),
        ("session_rotate", "allow", "rotated"),
        ("session_authenticate", "deny", "expired"),
        ("session_authenticate", "deny", "replay_detected"),
        ("session_authenticate", "deny", "revoked"),
        ("verification_issue", "allow", "verification_accepted"),
        ("verification_issue", "deny", "rate_limited"),
        ("verification_consume", "allow", "verification_completed"),
        ("verification_consume", "deny", "replay_detected"),
    }
    observed = {(row["action"], row["outcome"], row["reason"]) for row in audit}
    if not required.issubset(observed):
        raise ConformanceFailure("privacy-safe auth audit coverage changed")


def parse_telemetry(log_path: pathlib.Path, token: str) -> list[dict[str, Any]]:
    clean = ANSI.sub("", log_path.read_text(encoding="utf-8"))
    if token in clean or digest(11) in clean or digest(61) in clean:
        raise ConformanceFailure("auth Worker log exposed a capability or digest")
    records: list[dict[str, Any]] = []
    decoder = json.JSONDecoder()
    for line in clean.splitlines():
        marker = '"event":"d1_auth_repository"'
        cursor = 0
        while (marker_at := line.find(marker, cursor)) >= 0:
            start = line.rfind("{", cursor, marker_at + 1)
            if start < 0:
                cursor = marker_at + len(marker)
                continue
            try:
                record, consumed = decoder.raw_decode(line[start:])
            except json.JSONDecodeError:
                cursor = marker_at + len(marker)
                continue
            if isinstance(record, dict) and record.get("event") == "d1_auth_repository":
                records.append(record)
            cursor = start + consumed
    expected_fields = {"event", "operation", "outcome", "duration_ms", "rows"}
    if not records:
        raise ConformanceFailure("auth Worker emitted no repository telemetry")
    for record in records:
        if (
            set(record) != expected_fields
            or record.get("operation") not in TELEMETRY_OPERATIONS
            or not isinstance(record.get("outcome"), str)
            or re.fullmatch(r"[a-z_]{1,32}", str(record["outcome"])) is None
            or not isinstance(record.get("duration_ms"), int)
            or int(record["duration_ms"]) < 0
            or not isinstance(record.get("rows"), int)
            or int(record["rows"]) < 0
        ):
            raise ConformanceFailure("auth telemetry fields or bounds changed")
    observed_counts = Counter(str(record["operation"]) for record in records)
    if observed_counts != Counter(EXERCISED_TELEMETRY_COUNTS):
        raise ConformanceFailure("auth telemetry operation coverage changed")
    return records


def artifact_digest(files: Sequence[pathlib.Path]) -> str:
    value = hashlib.sha256()
    for path in files:
        value.update(path.name.encode())
        value.update(b"\0")
        value.update(path.read_bytes())
        value.update(b"\0")
    return value.hexdigest()


def write_evidence(path: pathlib.Path, telemetry: Sequence[dict[str, Any]]) -> None:
    report = {
        "schema_version": 1,
        "suite": "frame-d1-auth-repository-conformance",
        "runtime_boundary": "compiled_rust_wasm_worker_over_loopback_http",
        "database": "isolated_local_wrangler_d1",
        "wrangler_version": WRANGLER_VERSION,
        "migration_count": len(migration_files()),
        "migration_digest_sha256": artifact_digest(migration_files()),
        "query_count": len(list(QUERIES.glob("*.sql"))),
        "query_digest_sha256": artifact_digest(sorted(QUERIES.glob("*.sql"))),
        "scenarios": [
            "session_found_not_found_expired_revoked_replay_rotate_logout",
            "verification_issue_existing_bucket_fresh_plan_cas_retry",
            "verification_issue_near_cap_one_accept_one_semantic_limit",
            "verification_issue_exact_receipt_replay_after_materialization_no_duplicates",
            "logout_all_session_version_invalidation",
            "api_key_fallback_rotation_and_corrupt_row",
            "verification_success_and_replay",
            "delivery_materialize_lease_retry_stale_ack",
            "concurrent_identity_provisioning_double_spend",
            "audit_constraint_atomic_rollback",
            "active_user_membership_organization_authority_fences",
            "authority_first_downgraded_removed_key_issue_revoke_and_suspended_link",
            "attempt_twelve_active_lease_cleanup_and_retry_tombstone",
            "two_dispatcher_single_delivery_claim_race",
            "same_bucket_api_and_verification_fresh_plan_cas_retry",
            "exact_operation_receipt_replay_reconstruction",
            "structured_cas_sentinel_spoof_fails_closed",
            "mutation_promise_settlement_without_local_deadline_source_gate",
            "checked_in_sql_compilation",
            "privacy_safe_telemetry",
        ],
        "telemetry_record_count": len(telemetry),
        "result": "pass",
        "protected_boundaries": {
            "oauth_begin": "unsupported_pending_protected_provider_evidence",
            "oauth_exchange_preflight": "unsupported_pending_protected_provider_evidence",
            "oauth_exchange_finalize": "unsupported_pending_protected_provider_evidence",
        },
        "not_claimed": [
            "remote_d1_contention_or_replication",
            "provider_induced_delayed_d1_commit_timing",
            "transport_level_postcommit_response_loss",
            "provider_email_delivery",
            "provider_oauth_exchange",
        ],
        "cas_error_envelope": "wrangler_4.111.0_exact_d1error_trigger_constraint_fail_closed_on_drift",
        "ambiguous_commit_model": "discard_success_then_exact_receipt_replay_not_transport_fault_injection",
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wrangler-bin", help="direct path to pinned Wrangler 4.111.0")
    parser.add_argument(
        "--evidence",
        type=pathlib.Path,
        default=ROOT / "target" / "evidence" / "auth-d1-conformance.json",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    arguments = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        refuse_external_authority()
        compile_checked_in_sql()
        verify_compiled_surface()
        wrangler = detect_wrangler(arguments.wrangler_bin)
        with tempfile.TemporaryDirectory(prefix="frame-auth-d1-conformance-") as directory:
            root = pathlib.Path(directory)
            state = root / "state"
            state.mkdir(mode=0o700)
            d1 = WranglerD1(wrangler, state)
            d1.migrate()
            fixture = root / "fixture.sql"
            fixture.write_text(";\n".join(fixture_statements()) + ";\n", encoding="utf-8")
            d1.execute_file(fixture)
            token = secrets.token_hex(32)
            server = WorkerServer(d1, token, root)
            try:
                server.start()
                exercise_worker(server)
            finally:
                server.stop()
            assert_final_state(d1)
            telemetry = parse_telemetry(server.log_path, token)
        write_evidence(arguments.evidence.resolve(), telemetry)
    except (
        ConformanceFailure,
        OSError,
        sqlite3.Error,
        subprocess.SubprocessError,
        ValueError,
    ) as error:
        print(f"D1 auth repository conformance failed: {error}", file=sys.stderr)
        return 1
    print(
        "D1 auth repository conformance passed through compiled Worker "
        f"({len(migration_files())} migrations; Wrangler {WRANGLER_VERSION})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
