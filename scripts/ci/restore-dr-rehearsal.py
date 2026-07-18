#!/usr/bin/env python3
"""Run an isolated, synthetic backup/restore and disaster-recovery rehearsal.

This proves the repository contract and negative paths without claiming a D1,
R2, regional, encryption-provider, or signing-key-custody restore.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import shutil
import sqlite3
import sys
import tempfile
import time
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
POLICY_PATH = ROOT / "fixtures/operational-hardening/v1/operational-policy.json"
OBJECT_FIXTURE = ROOT / "fixtures/media-jobs/v1/synthetic-h264-aac.mp4"
HEX = frozenset("0123456789abcdef")


class RestoreError(RuntimeError):
    pass


def canonical(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as handle:
            while block := handle.read(1024 * 1024):
                digest.update(block)
    except OSError as error:
        raise RestoreError(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def read_json(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RestoreError(f"cannot read JSON {path}: {error}") from error
    if not isinstance(value, dict):
        raise RestoreError(f"JSON document {path} must be an object")
    return value


def safe_relative(value: Any) -> pathlib.PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value:
        raise RestoreError("backup manifest path is invalid")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise RestoreError("backup manifest path is unsafe")
    return path


def create_source(database: pathlib.Path, object_digest: str, object_size: int) -> None:
    connection = sqlite3.connect(database)
    try:
        connection.execute("PRAGMA foreign_keys = ON")
        connection.executescript(
            """
            CREATE TABLE users (
              id TEXT PRIMARY KEY,
              identifier_digest TEXT NOT NULL UNIQUE
            );
            CREATE TABLE sessions (
              id TEXT PRIMARY KEY,
              user_id TEXT NOT NULL REFERENCES users(id),
              credential_digest TEXT NOT NULL UNIQUE,
              expires_at_ms INTEGER NOT NULL
            );
            CREATE TABLE videos (
              id TEXT PRIMARY KEY,
              owner_id TEXT NOT NULL REFERENCES users(id),
              state TEXT NOT NULL CHECK(state IN ('ready', 'deleted'))
            );
            CREATE TABLE objects (
              logical_id TEXT PRIMARY KEY,
              video_id TEXT NOT NULL REFERENCES videos(id),
              role TEXT NOT NULL,
              relative_path TEXT NOT NULL UNIQUE,
              byte_size INTEGER NOT NULL,
              sha256 TEXT NOT NULL
            );
            CREATE TABLE shares (
              id TEXT PRIMARY KEY,
              video_id TEXT NOT NULL REFERENCES videos(id),
              visibility TEXT NOT NULL CHECK(visibility IN ('private', 'public'))
            );
            """
        )
        connection.execute(
            "INSERT INTO users VALUES (?, ?)",
            ("018f47a6-7b1c-7f55-8f39-8f8a86900001", "11" * 32),
        )
        connection.execute(
            "INSERT INTO sessions VALUES (?, ?, ?, ?)",
            (
                "018f47a6-7b1c-7f55-8f39-8f8a86900002",
                "018f47a6-7b1c-7f55-8f39-8f8a86900001",
                "22" * 32,
                1_800_000_000_000,
            ),
        )
        connection.execute(
            "INSERT INTO videos VALUES (?, ?, ?)",
            (
                "018f47a6-7b1c-7f55-8f39-8f8a86900003",
                "018f47a6-7b1c-7f55-8f39-8f8a86900001",
                "ready",
            ),
        )
        connection.execute(
            "INSERT INTO objects VALUES (?, ?, ?, ?, ?, ?)",
            (
                "018f47a6-7b1c-7f55-8f39-8f8a86900004",
                "018f47a6-7b1c-7f55-8f39-8f8a86900003",
                "source_original",
                "objects/synthetic-h264-aac.mp4",
                object_size,
                object_digest,
            ),
        )
        connection.execute(
            "INSERT INTO shares VALUES (?, ?, ?)",
            (
                "018f47a6-7b1c-7f55-8f39-8f8a86900005",
                "018f47a6-7b1c-7f55-8f39-8f8a86900003",
                "public",
            ),
        )
        connection.commit()
    finally:
        connection.close()


def export_sql(database: pathlib.Path, destination: pathlib.Path) -> None:
    source = sqlite3.connect(f"file:{database}?mode=ro", uri=True)
    try:
        payload = "\n".join(source.iterdump()) + "\n"
    finally:
        source.close()
    destination.write_text(payload, encoding="utf-8")


def write_backup(source_db: pathlib.Path, backup: pathlib.Path) -> dict[str, Any]:
    backup.mkdir()
    (backup / "objects").mkdir()
    (backup / "projects").mkdir()
    export_sql(source_db, backup / "d1-export.sql")
    shutil.copyfile(OBJECT_FIXTURE, backup / "objects/synthetic-h264-aac.mp4")
    (backup / "configuration.json").write_bytes(
        canonical(
            {
                "schema_version": 1,
                "environment": "isolated_restore",
                "bindings": ["DB", "MEDIA_INPUTS", "MEDIA_OUTPUTS", "UPLOADS"],
                "secret_references": ["provider_ref:cloudflare_deploy", "provider_ref:r2_signing"],
                "contains_secret_values": False,
            }
        )
    )
    (backup / "signing-key-catalog.json").write_bytes(
        canonical(
            {
                "schema_version": 1,
                "active_key_id": "release-key-v2",
                "public_key_sha256": "33" * 32,
                "recovery_receipt_sha256": "44" * 32,
                "contains_private_key_material": False,
                "custody_evidence": "protected_not_collected",
            }
        )
    )
    segment = b"generated project segment manifest input\n"
    (backup / "projects/project.json").write_bytes(
        canonical(
            {
                "schema_version": 1,
                "project_id": "018f47a6-7b1c-7f55-8f39-8f8a86900006",
                "open_mode": "read_only",
                "segments": [{"ordinal": 0, "sha256": hashlib.sha256(segment).hexdigest()}],
            }
        )
    )
    entries: list[dict[str, Any]] = []
    for path in sorted(item for item in backup.rglob("*") if item.is_file()):
        relative = path.relative_to(backup).as_posix()
        entries.append(
            {"path": relative, "byte_size": path.stat().st_size, "sha256": sha256_file(path)}
        )
    manifest = {
        "schema_version": 1,
        "backup_id": "synthetic-isolated-restore-v1",
        "snapshot_checkpoint_ms": 1_750_000_000_000,
        "latest_acknowledged_write_ms": 1_749_999_880_000,
        "encryption": "required_in_protected_backup_not_claimed_by_local_fixture",
        "source": "local_synthetic_sqlite_not_d1_or_r2",
        "entries": entries,
    }
    (backup / "backup-manifest.json").write_bytes(canonical(manifest))
    return manifest


def verify_manifest(backup: pathlib.Path, manifest: dict[str, Any]) -> None:
    expected_keys = {
        "schema_version",
        "backup_id",
        "snapshot_checkpoint_ms",
        "latest_acknowledged_write_ms",
        "encryption",
        "source",
        "entries",
    }
    if set(manifest) != expected_keys or manifest["schema_version"] != 1:
        raise RestoreError("backup manifest has an invalid shape")
    entries = manifest["entries"]
    if not isinstance(entries, list) or not entries:
        raise RestoreError("backup manifest must have entries")
    names: set[str] = set()
    for entry in entries:
        if not isinstance(entry, dict) or set(entry) != {"path", "byte_size", "sha256"}:
            raise RestoreError("backup manifest entry has an invalid shape")
        relative = safe_relative(entry["path"])
        name = relative.as_posix()
        if name in names:
            raise RestoreError("backup manifest has duplicate paths")
        names.add(name)
        path = backup.joinpath(*relative.parts)
        if not path.is_file() or path.is_symlink():
            raise RestoreError(f"backup entry is missing or unsafe: {name}")
        if entry["byte_size"] != path.stat().st_size:
            raise RestoreError(f"backup entry size mismatch: {name}")
        digest = entry["sha256"]
        if not isinstance(digest, str) or len(digest) != 64 or not set(digest) <= HEX:
            raise RestoreError(f"backup entry digest is invalid: {name}")
        if sha256_file(path) != digest:
            raise RestoreError(f"backup entry digest mismatch: {name}")
    required = {
        "d1-export.sql",
        "objects/synthetic-h264-aac.mp4",
        "configuration.json",
        "signing-key-catalog.json",
        "projects/project.json",
    }
    if not required <= names:
        raise RestoreError("backup manifest omits a required recovery asset")


def restore_database(sql_path: pathlib.Path, destination: pathlib.Path) -> None:
    connection = sqlite3.connect(destination)
    try:
        # A canonical SQLite dump orders objects lexically, not by the foreign-
        # key graph. Import into the isolated empty target with enforcement
        # deferred, then enable it and run foreign_key_check below.
        connection.execute("PRAGMA foreign_keys = OFF")
        connection.executescript(sql_path.read_text(encoding="utf-8"))
        connection.execute("PRAGMA foreign_keys = ON")
    except (OSError, sqlite3.Error) as error:
        raise RestoreError(f"database restore failed: {error}") from error
    finally:
        connection.close()


def iso_bmff_boxes(path: pathlib.Path) -> set[bytes]:
    data = path.read_bytes()
    boxes: set[bytes] = set()
    offset = 0
    while offset + 8 <= len(data):
        size = int.from_bytes(data[offset : offset + 4], "big")
        kind = data[offset + 4 : offset + 8]
        if size == 0:
            size = len(data) - offset
        elif size == 1:
            if offset + 16 > len(data):
                raise RestoreError("restored media has a truncated extended box")
            size = int.from_bytes(data[offset + 8 : offset + 16], "big")
        if size < 8 or offset + size > len(data):
            raise RestoreError("restored media has an invalid ISO BMFF box")
        boxes.add(kind)
        offset += size
    if offset != len(data):
        raise RestoreError("restored media has trailing invalid bytes")
    return boxes


def validate_restored(restored: pathlib.Path) -> dict[str, Any]:
    database = restored / "frame-restored.sqlite3"
    connection = sqlite3.connect(database)
    try:
        connection.execute("PRAGMA foreign_keys = ON")
        foreign_keys = connection.execute("PRAGMA foreign_key_check").fetchall()
        integrity = connection.execute("PRAGMA integrity_check").fetchone()
        counts = {
            table: connection.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
            for table in ("users", "sessions", "videos", "objects", "shares")
        }
        auth_links = connection.execute(
            "SELECT COUNT(*) FROM sessions JOIN users ON users.id = sessions.user_id"
        ).fetchone()[0]
        object_row = connection.execute(
            """SELECT objects.relative_path, objects.byte_size, objects.sha256
                 FROM objects JOIN videos ON videos.id = objects.video_id
                WHERE videos.state = 'ready'"""
        ).fetchone()
        playback_links = connection.execute(
            """SELECT COUNT(*) FROM shares JOIN videos ON videos.id = shares.video_id
                WHERE shares.visibility = 'public' AND videos.state = 'ready'"""
        ).fetchone()[0]
    finally:
        connection.close()
    if foreign_keys or integrity != ("ok",) or counts != {
        "users": 1,
        "sessions": 1,
        "videos": 1,
        "objects": 1,
        "shares": 1,
    }:
        raise RestoreError("restored database failed integrity or referential checks")
    if auth_links != 1 or playback_links != 1 or object_row is None:
        raise RestoreError("restored auth or playback relationships are incomplete")
    relative, byte_size, digest = object_row
    object_path = restored / relative
    if not object_path.is_file() or object_path.stat().st_size != byte_size:
        raise RestoreError("restored object is missing or has the wrong size")
    if sha256_file(object_path) != digest:
        raise RestoreError("restored object checksum does not match D1 manifest")
    source_bytes = OBJECT_FIXTURE.read_bytes()
    restored_bytes = object_path.read_bytes()
    start = min(32, len(source_bytes))
    end = min(start + 64, len(source_bytes))
    if not source_bytes or restored_bytes[start:end] != source_bytes[start:end]:
        raise RestoreError("restored object failed bounded range verification")
    boxes = iso_bmff_boxes(object_path)
    if not {b"ftyp", b"moov", b"mdat"} <= boxes:
        raise RestoreError("restored object failed local playback-container verification")

    configuration = read_json(restored / "configuration.json")
    if (
        configuration.get("contains_secret_values") is not False
        or set(configuration.get("bindings", [])) != {"DB", "MEDIA_INPUTS", "MEDIA_OUTPUTS", "UPLOADS"}
    ):
        raise RestoreError("restored configuration is unsafe or incomplete")
    key_catalog = read_json(restored / "signing-key-catalog.json")
    if (
        key_catalog.get("contains_private_key_material") is not False
        or key_catalog.get("custody_evidence") != "protected_not_collected"
    ):
        raise RestoreError("local signing-key catalog overclaims protected custody")
    project = read_json(restored / "projects/project.json")
    if project.get("open_mode") != "read_only" or len(project.get("segments", [])) != 1:
        raise RestoreError("restored desktop project failed read-only schema verification")
    return {
        "foreign_keys": True,
        "integrity": True,
        "referential_counts": counts,
        "auth_session_links": auth_links,
        "object_manifest_links": 1,
        "object_sha256": True,
        "range_read": True,
        "playback_container": True,
        "public_playback_relation": playback_links,
        "configuration_no_secret_values": True,
        "signing_catalog_metadata_only": True,
        "desktop_project_read_only_open": True,
    }


def restore_assets(backup: pathlib.Path, restored: pathlib.Path) -> None:
    restored.mkdir()
    restore_database(backup / "d1-export.sql", restored / "frame-restored.sqlite3")
    for relative in (
        "objects/synthetic-h264-aac.mp4",
        "configuration.json",
        "signing-key-catalog.json",
        "projects/project.json",
    ):
        source = backup / relative
        destination = restored / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)


def negative_tests(backup: pathlib.Path, manifest: dict[str, Any]) -> dict[str, bool]:
    corrupted = copy.deepcopy(manifest)
    corrupted["entries"][0]["sha256"] = "f" * 64
    try:
        verify_manifest(backup, corrupted)
    except RestoreError:
        corruption_rejected = True
    else:
        corruption_rejected = False

    missing = copy.deepcopy(manifest)
    missing["entries"] = [
        entry
        for entry in missing["entries"]
        if entry["path"] != "objects/synthetic-h264-aac.mp4"
    ]
    try:
        verify_manifest(backup, missing)
    except RestoreError:
        missing_object_rejected = True
    else:
        missing_object_rejected = False
    if not corruption_rejected or not missing_object_rejected:
        raise RestoreError("backup negative-path verification failed")
    return {
        "corrupt_manifest_rejected": corruption_rejected,
        "missing_object_manifest_rejected": missing_object_rejected,
    }


def run_rehearsal() -> dict[str, Any]:
    policy = read_json(POLICY_PATH)
    recovery = policy["recovery"]
    if not OBJECT_FIXTURE.is_file():
        raise RestoreError("synthetic media fixture is missing")
    object_digest = sha256_file(OBJECT_FIXTURE)
    started = time.monotonic_ns()
    with tempfile.TemporaryDirectory(prefix="frame-restore-") as temporary:
        root = pathlib.Path(temporary)
        source_db = root / "source.sqlite3"
        create_source(source_db, object_digest, OBJECT_FIXTURE.stat().st_size)
        backup = root / "backup"
        manifest = write_backup(source_db, backup)
        verify_manifest(backup, manifest)
        negatives = negative_tests(backup, manifest)
        restored = root / "isolated-restored"
        restore_assets(backup, restored)
        checks = validate_restored(restored)
    elapsed_ms = max(1, (time.monotonic_ns() - started + 999_999) // 1_000_000)
    observed_rpo = 120_000
    if observed_rpo > recovery["rpo_ms"] or elapsed_ms > recovery["rto_ms"]:
        raise RestoreError("local synthetic RPO/RTO bound was exceeded")
    return {
        "schema_version": 1,
        "evidence_scope": "local_synthetic_isolated_sqlite_and_files_not_provider_restore",
        "fixture_sha256": object_digest,
        "rpo": {"target_ms": recovery["rpo_ms"], "observed_ms": observed_rpo, "passed": True},
        "rto": {"target_ms": recovery["rto_ms"], "observed_ms": elapsed_ms, "passed": True},
        "checks": checks,
        "negative_tests": negatives,
        "protected_not_collected": [
            "provider_d1_export",
            "encrypted_immutable_backup",
            "provider_object_restore",
            "signing_key_custody_recovery",
            "production_shaped_timed_restore",
            "regional_disaster_recovery",
        ],
    }


def write_atomic(path: pathlib.Path, value: dict[str, Any]) -> None:
    payload = canonical(value)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    try:
        with temporary.open("xb") as handle:
            handle.write(payload)
        temporary.replace(path)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise RestoreError(f"cannot write restore evidence: {error}") from error


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=pathlib.Path)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        evidence = run_rehearsal()
        if args.evidence is not None:
            write_atomic(args.evidence, evidence)
        print(
            "isolated synthetic restore passed: "
            f"RPO {evidence['rpo']['observed_ms']}ms, RTO {evidence['rto']['observed_ms']}ms; "
            "provider restore remains protected"
        )
        return 0
    except (KeyError, OSError, RestoreError, sqlite3.Error) as error:
        print(f"restore/DR rehearsal failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
