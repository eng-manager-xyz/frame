#!/usr/bin/env python3
"""Credential-free adversarial proof for the MySQL snapshot exporter."""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import sqlite3
import subprocess
import sys
import tempfile

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import etl
import mysql_snapshot


STARTED = 1_735_689_600_100
COMPLETED = 1_735_689_600_200


def plan_value() -> dict[str, object]:
    return {
        "schema_version": 1,
        "run_id": "mysql-snapshot-local-v1",
        "source_schema": "cap-mysql-pinned-v1",
        "target_migration": "frame-d1-v1",
        "code_sha": "a" * 64,
        "window": {
            "start_ms": STARTED - 100,
            "end_ms": COMPLETED + 100,
        },
        "mysql_snapshot": {
            "database_sha256": "1" * 64,
            "server_uuid_sha256": "2" * 64,
            "server_version_sha256": "3" * 64,
            "tables_sha256": "4" * 64,
            "columns_sha256": "5" * 64,
            "indexes_sha256": "6" * 64,
            "constraints_sha256": "7" * 64,
        },
        "tables": [
            {
                "name": "records",
                "source_table": "legacy_records",
                "tenant_column": "tenant_id",
                "primary_key": ["id"],
                "columns": [
                    {
                        "source": "org_id",
                        "target": "tenant_id",
                        "transform": "identity",
                        "nullable": False,
                    },
                    {
                        "source": "legacy_id",
                        "target": "id",
                        "transform": "identity",
                        "nullable": False,
                    },
                    {
                        "source": "is_enabled",
                        "target": "enabled",
                        "transform": "boolean",
                        "nullable": False,
                    },
                    {
                        "source": "amount_decimal",
                        "target": "amount_micros",
                        "transform": "decimal_scaled",
                        "nullable": False,
                        "options": {"scale": 2},
                    },
                    {
                        "source": "payload_json",
                        "target": "payload_json",
                        "transform": "canonical_json",
                        "nullable": False,
                    },
                    {
                        "source": "occurred_at",
                        "target": "occurred_at_ms",
                        "transform": "timestamp_ms",
                        "nullable": False,
                    },
                    {
                        "source": "display_label",
                        "target": "normalized_label",
                        "transform": "casefold_nfkc",
                        "nullable": False,
                    },
                    {
                        "source": "legacy_state",
                        "target": "state",
                        "transform": "enum",
                        "nullable": False,
                        "options": {"mapping": {"ACTIVE": "active", "OFF": "inactive"}},
                    },
                ],
                "foreign_keys": [],
                "aggregates": [
                    {"name": "rows", "operation": "count", "column": "*"},
                    {
                        "name": "amount",
                        "operation": "sum",
                        "column": "amount_micros",
                    },
                ],
            }
        ],
        "semantic_rules": [
            {
                "name": "record_labels_unique",
                "kind": "unique_per_tenant",
                "table": "records",
                "column": "normalized_label",
            }
        ],
    }


def source_rows() -> list[dict[str, object]]:
    return [
        {
            "table": "records",
            "tenant": "tenant-a",
            "row": {
                "org_id": "tenant-a",
                "legacy_id": "record-a",
                "is_enabled": "1",
                "amount_decimal": "12.34",
                "payload_json": '{"z":2,"a":1}',
                "occurred_at": "2025-01-01T00:00:00.100000Z",
                "display_label": " Alpha ",
                "legacy_state": "ACTIVE",
            },
        },
        {
            "table": "records",
            "tenant": "tenant-b",
            "row": {
                "org_id": "tenant-b",
                "legacy_id": "record-b",
                "is_enabled": "false",
                "amount_decimal": "0.10",
                "payload_json": "[]",
                "occurred_at": "2025-01-01T00:00:00.200000Z",
                "display_label": "Beta\u2028Line",
                "legacy_state": "OFF",
            },
        },
    ]


def write_json(path: pathlib.Path, value: object, mode: int = 0o600) -> None:
    path.write_text(f"{etl.canonical(value)}\n", encoding="utf-8")
    path.chmod(mode)


def rewrite_manifest(bundle: pathlib.Path, manifest: object) -> None:
    payload = f"{etl.canonical(manifest)}\n".encode("utf-8")
    (bundle / "manifest.json").write_bytes(payload)
    (bundle / "manifest.sha256").write_text(
        f"{etl.sha256_bytes(payload)}  manifest.json\n", encoding="ascii"
    )


def fake_mysql(path: pathlib.Path, *, mode: str = "success") -> None:
    rows = source_rows()
    body = f"""#!/usr/bin/env python3
import base64
import json
import os
import sys
import time

MODE = {mode!r}
STARTED = {STARTED}
COMPLETED = {COMPLETED}
ROWS = {rows!r}
if MODE == 'transform_reject':
    ROWS[0]['row']['legacy_state'] = 'UNKNOWN'
if MODE == 'large_row':
    ROWS[0]['row']['display_label'] = 'x' * 2048
query = sys.stdin.read()
assert sys.argv[1].startswith('--defaults-file=')
assert '--tls-version=TLSv1.2,TLSv1.3' in sys.argv
assert all('password' not in argument.casefold() for argument in sys.argv)
mysql_environment = [name for name in os.environ if name.startswith('MYSQL')]
assert mysql_environment == ['MYSQL_TEST_LOGIN_FILE']
assert not os.path.exists(os.environ['MYSQL_TEST_LOGIN_FILE'])
assert 'START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY;' in query
assert query.count('START TRANSACTION') == 1
assert query.count('COMMIT;') == 1
if MODE == 'timeout':
    time.sleep(2)
prerequisites = '\\t1\\tON\\tON\\t1\\t1\\tROW\\tFULL\\t\\t0\\t1'
source_binding = '\\t' + '\\t'.join([character * 64 for character in '1234567'])
if '__FRAME_MYSQL_SOURCE_BINDING_V1__' in query:
    print('__FRAME_MYSQL_SOURCE_BINDING_V1__' + prerequisites + source_binding)
    raise SystemExit(0)
assert 'FROM `legacy_records` AS `source` ORDER BY `source`.`org_id`, `source`.`legacy_id`' in query
gtid_bytes = b'a' * 256 if MODE == 'large_gtid' else b'24bc7850-0000-0000-0000-000000000001:1-19'
gtid = base64.b64encode(gtid_bytes).decode('ascii')
if MODE == 'client_failure':
    print('credential=super-secret-value', file=sys.stderr)
    raise SystemExit(9)
if MODE == 'bad_begin':
    print('not-a-marker')
    raise SystemExit(0)
started = STARTED if MODE != 'outside_window' else STARTED - 1000
if MODE == 'bad_prerequisite':
    prerequisites = '\\t1\\tON_PERMISSIVE\\tON\\t1\\t1\\tROW\\tFULL\\t\\t0\\t1'
if MODE == 'binding_truncated':
    prerequisites = '\\t1\\tON\\tON\\t1\\t1\\tROW\\tFULL\\t\\t1\\t1'
if MODE == 'wrong_source':
    source_binding = '\\t' + '\\t'.join(['0' * 64] + [character * 64 for character in '234567'])
print(
    '__FRAME_MYSQL_SNAPSHOT_V1_BEGIN__\\t' + str(started) + '\\t' + gtid
    + prerequisites + source_binding
)
for row in ROWS:
    print(json.dumps(row, ensure_ascii=False, separators=(',', ':')))
if MODE == 'bad_row':
    print('{{"table":"records","tenant":"tenant-a","row":null}}')
print('__FRAME_MYSQL_SNAPSHOT_V1_END__\\t' + str(COMPLETED))
if MODE == 'after_end':
    print(json.dumps(ROWS[0], separators=(',', ':')))
if MODE == 'large_stderr':
    print('x' * 2048, file=sys.stderr)
print('credential=ignored-success-secret', file=sys.stderr)
"""
    path.write_text(body, encoding="utf-8")
    path.chmod(0o700)


def target(path: pathlib.Path) -> None:
    connection = sqlite3.connect(path)
    connection.executescript(
        """
        CREATE TABLE records (
          tenant_id TEXT NOT NULL,
          id TEXT PRIMARY KEY NOT NULL,
          enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
          amount_micros INTEGER NOT NULL,
          payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
          occurred_at_ms INTEGER NOT NULL,
          normalized_label TEXT NOT NULL,
          state TEXT NOT NULL CHECK(state IN ('active', 'inactive'))
        );
        """
    )
    connection.close()


def expect_snapshot_error(callable_object, *args, **kwargs) -> None:
    try:
        callable_object(*args, **kwargs)
    except mysql_snapshot.SnapshotError as error:
        rendered = f"{error!s} {error!r}"
        assert "secret" not in rendered.casefold()
        assert "tenant-a" not in rendered
        return
    raise AssertionError("expected SnapshotError")


def expect_etl_error(callable_object, *args, **kwargs) -> None:
    try:
        callable_object(*args, **kwargs)
    except etl.EtlError:
        return
    raise AssertionError("expected EtlError")


def test_happy_path(root: pathlib.Path) -> dict[str, object]:
    plan_path = root / "plan.json"
    defaults = root / "mysql.cnf"
    client = root / "fake-mysql"
    write_json(plan_path, plan_value())
    defaults.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\n"
        "password=not-a-real-secret\ndatabase=frame\nssl-ca=/fixture/ca.pem\n",
        encoding="utf-8",
    )
    defaults.chmod(0o600)
    fake_mysql(client)
    first = root / "bundle-a"
    second = root / "bundle-b"
    result = mysql_snapshot.export_mysql_snapshot(
        plan_path=plan_path,
        defaults_file=defaults,
        mysql_bin=str(client),
        bundle=first,
        chunk_rows=1,
        timeout_seconds=30,
    )
    duplicate = mysql_snapshot.export_mysql_snapshot(
        plan_path=plan_path,
        defaults_file=defaults,
        mysql_bin=str(client),
        bundle=second,
        chunk_rows=1,
        timeout_seconds=30,
    )
    assert result == duplicate
    first_files = {
        path.relative_to(first).as_posix(): path.read_bytes()
        for path in first.rglob("*")
        if path.is_file()
    }
    second_files = {
        path.relative_to(second).as_posix(): path.read_bytes()
        for path in second.rglob("*")
        if path.is_file()
    }
    assert first_files == second_files
    assert all((path.stat().st_mode & 0o077) == 0 for path in first.rglob("*") if path.is_file())
    proof = result["proof"]
    assert isinstance(proof, dict)
    assert proof["captured_row_count"] == 2
    assert proof["contains_source_values"] is False
    assert "tenant-a" not in etl.canonical(proof)
    boundary = etl.load_json(first / "snapshot-boundary.protected.json", "boundary")
    assert boundary["gtid_before_snapshot"].endswith(":1-19")
    assert boundary["gtid_sha256"] == proof["gtid_sha256"]

    crash_bundle = root / "bundle-crash"
    original_write = etl.immutable_private_write

    def injected_write(path: pathlib.Path, data: bytes) -> None:
        if path.name == etl.MYSQL_SNAPSHOT_PROOF:
            raise OSError("injected attestation write failure")
        original_write(path, data)

    etl.immutable_private_write = injected_write
    try:
        try:
            mysql_snapshot.export_mysql_snapshot(
                plan_path=plan_path,
                defaults_file=defaults,
                mysql_bin=str(client),
                bundle=crash_bundle,
                chunk_rows=1,
                timeout_seconds=30,
            )
        except OSError:
            pass
        else:
            raise AssertionError("expected injected publication failure")
    finally:
        etl.immutable_private_write = original_write
    assert not crash_bundle.exists()
    crash_lock = root / ".bundle-crash.publish.lock"
    assert crash_lock.is_file()
    assert crash_lock.stat().st_mode & 0o077 == 0
    recovered = mysql_snapshot.export_mysql_snapshot(
        plan_path=plan_path,
        defaults_file=defaults,
        mysql_bin=str(client),
        bundle=crash_bundle,
        chunk_rows=1,
        timeout_seconds=30,
    )
    assert recovered["manifest"]["row_count"] == 2
    expect_snapshot_error(
        mysql_snapshot.export_mysql_snapshot,
        plan_path=plan_path,
        defaults_file=defaults,
        mysql_bin=str(client),
        bundle=crash_bundle,
        chunk_rows=1,
        timeout_seconds=30,
    )

    database = root / "target.sqlite"
    target(database)
    applied = etl.import_bundle(database, first, dry_run=False, max_rows_per_second=0)
    assert applied["validated_rows"] == 2
    report = etl.reconcile_bundle(database, first)
    assert report["clean"] is True
    connection = sqlite3.connect(database)
    rows = connection.execute(
        "SELECT tenant_id, enabled, amount_micros, payload_json, normalized_label, state "
        "FROM records ORDER BY tenant_id"
    ).fetchall()
    connection.close()
    assert rows == [
        ("tenant-a", 1, 1234, '{"a":1,"z":2}', "alpha", "active"),
        ("tenant-b", 0, 10, "[]", "beta\u2028line", "inactive"),
    ]
    cli_target = root / "cli-target.sqlite"
    target(cli_target)
    command = [sys.executable, "-I", str(pathlib.Path(etl.__file__))]
    imported = subprocess.run(
        [*command, "import", "--target", str(cli_target), "--bundle", str(first), "--dry-run"],
        check=False,
        capture_output=True,
        text=True,
    )
    assert imported.returncode == 0, imported.stderr
    report_path = root / "cli-reconciliation.json"
    reconciled = subprocess.run(
        [
            *command,
            "reconcile",
            "--target",
            str(database),
            "--bundle",
            str(first),
            "--report",
            str(report_path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    assert reconciled.returncode == 0, reconciled.stderr
    assert etl.load_json(report_path, "CLI reconciliation")["clean"] is True

    empty_source = root / "empty-source.ndjson"
    empty_source.write_bytes(b"")
    empty_bundle = root / "empty-bundle"
    etl.export_bundle(empty_source, etl.load_plan(plan_path), empty_bundle, chunk_rows=1)
    extra_target = root / "extra-target.sqlite"
    target(extra_target)
    connection = sqlite3.connect(extra_target)
    connection.execute(
        "INSERT INTO records VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        ("tenant-extra", "record-extra", 1, 1, "{}", STARTED, "extra", "active"),
    )
    connection.commit()
    connection.close()
    extra_report = etl.reconcile_bundle(extra_target, empty_bundle)
    assert extra_report["clean"] is False
    assert extra_report["tables"][0]["extra_primary_keys"] == 1

    tampered_core = root / "bundle-tampered-core"
    shutil.copytree(first, tampered_core)
    tampered_manifest = etl.load_json(tampered_core / "manifest.json", "manifest")
    tampered_chunk = tampered_manifest["tables"][0]["tenants"][0]["chunks"][0]
    tampered_chunk_path = tampered_core / pathlib.PurePosixPath(tampered_chunk["path"])
    tampered_payload = tampered_chunk_path.read_bytes().replace(b'"alpha"', b'"omega"')
    tampered_chunk_path.write_bytes(tampered_payload)
    tampered_chunk["sha256"] = etl.sha256_bytes(tampered_payload)
    rewrite_manifest(tampered_core, tampered_manifest)
    expect_etl_error(etl.load_manifest, tampered_core)

    tampered_proof = root / "bundle-tampered-proof-core"
    shutil.copytree(first, tampered_proof)
    proof_manifest = etl.load_json(tampered_proof / "manifest.json", "manifest")
    proof_payload_value = etl.load_json(tampered_proof / etl.MYSQL_SNAPSHOT_PROOF, "proof")
    proof_payload_value["manifest_core_sha256"] = "0" * 64
    false_proof_payload = f"{etl.canonical(proof_payload_value)}\n".encode("utf-8")
    (tampered_proof / etl.MYSQL_SNAPSHOT_PROOF).write_bytes(false_proof_payload)
    proof_manifest["source_attestation"]["proof_sha256"] = etl.sha256_bytes(
        false_proof_payload
    )
    rewrite_manifest(tampered_proof, proof_manifest)
    expect_etl_error(etl.load_manifest, tampered_proof)
    second_proof = second / etl.MYSQL_SNAPSHOT_PROOF
    second_proof.write_bytes(second_proof.read_bytes() + b" ")
    expect_etl_error(etl.load_manifest, second)
    second_proof.write_bytes((first / etl.MYSQL_SNAPSHOT_PROOF).read_bytes())
    (second / etl.MYSQL_SNAPSHOT_BOUNDARY).unlink()
    expect_etl_error(etl.import_bundle, database, second, dry_run=True, max_rows_per_second=0)
    return dict(proof)


def test_private_defaults(root: pathlib.Path) -> None:
    plan_path = root / "plan-private.json"
    source = root / "private-source.ndjson"
    client = root / "fake-private-mysql"
    write_json(plan_path, plan_value())
    fake_mysql(client)
    loose = root / "loose.cnf"
    loose.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\npassword=value\n"
        "database=frame\nssl-ca=/fixture/ca.pem\n",
        encoding="utf-8",
    )
    loose.chmod(0o644)
    plan = etl.load_plan(plan_path)
    expect_snapshot_error(
        mysql_snapshot.capture_snapshot,
        plan=plan,
        defaults_file=loose,
        mysql_bin=str(client),
        source_path=source,
        timeout_seconds=30,
    )
    fifo = root / "defaults.fifo"
    os.mkfifo(fifo, 0o600)
    expect_snapshot_error(
        mysql_snapshot.capture_snapshot,
        plan=plan,
        defaults_file=fifo,
        mysql_bin=str(client),
        source_path=source,
        timeout_seconds=30,
    )
    unsafe = root / "unsafe.cnf"
    unsafe.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\npassword=value\n"
        "database=frame\nssl-ca=/fixture/ca.pem\ninit-command=DELETE FROM records\n",
        encoding="utf-8",
    )
    unsafe.chmod(0o600)
    expect_snapshot_error(
        mysql_snapshot.capture_snapshot,
        plan=plan,
        defaults_file=unsafe,
        mysql_bin=str(client),
        source_path=source,
        timeout_seconds=30,
    )
    downgraded = root / "downgraded.cnf"
    downgraded.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\npassword=value\n"
        "database=frame\nssl-ca=/fixture/ca.pem\ntls-version=TLSv1\n",
        encoding="utf-8",
    )
    downgraded.chmod(0o600)
    expect_snapshot_error(
        mysql_snapshot.capture_snapshot,
        plan=plan,
        defaults_file=downgraded,
        mysql_bin=str(client),
        source_path=source,
        timeout_seconds=30,
    )
    included = root / "included.cnf"
    included.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\ndatabase=frame\n"
        "ssl-ca=/fixture/ca.pem\n!include /tmp/credentials.cnf\n",
        encoding="utf-8",
    )
    included.chmod(0o600)
    expect_snapshot_error(
        mysql_snapshot.capture_snapshot,
        plan=plan,
        defaults_file=included,
        mysql_bin=str(client),
        source_path=source,
        timeout_seconds=30,
    )
    assert not source.exists()
    private = root / "private.cnf"
    private.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\npassword=value\n"
        "database=frame\nssl-ca=/fixture/ca.pem\n",
        encoding="utf-8",
    )
    private.chmod(0o600)
    writable_client = root / "writable-client"
    fake_mysql(writable_client)
    writable_client.chmod(0o722)
    expect_snapshot_error(
        mysql_snapshot.capture_snapshot,
        plan=plan,
        defaults_file=private,
        mysql_bin=str(writable_client),
        source_path=source,
        timeout_seconds=30,
    )
    symlink = root / "symlink.cnf"
    symlink.symlink_to(private)
    expect_snapshot_error(
        mysql_snapshot.capture_snapshot,
        plan=plan,
        defaults_file=symlink,
        mysql_bin=str(client),
        source_path=source,
        timeout_seconds=30,
    )


def test_faults(root: pathlib.Path) -> None:
    plan_path = root / "plan-faults.json"
    defaults = root / "faults.cnf"
    write_json(plan_path, plan_value())
    defaults.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\npassword=value\n"
        "database=frame\nssl-ca=/fixture/ca.pem\n",
        encoding="utf-8",
    )
    defaults.chmod(0o600)
    plan = etl.load_plan(plan_path)
    for mode in (
        "client_failure",
        "bad_begin",
        "bad_prerequisite",
        "binding_truncated",
        "wrong_source",
        "outside_window",
        "bad_row",
        "after_end",
    ):
        client = root / f"fake-mysql-{mode}"
        fake_mysql(client, mode=mode)
        source = root / f"source-{mode}.ndjson"
        expect_snapshot_error(
            mysql_snapshot.capture_snapshot,
            plan=plan,
            defaults_file=defaults,
            mysql_bin=str(client),
            source_path=source,
            timeout_seconds=30,
        )
        assert not source.exists()
    bounded_faults = (
        ("large_row", "MAX_ROW_BYTES", 1024, 30),
        ("large_gtid", "MAX_GTID_BYTES", 64, 30),
        ("large_stderr", "MAX_STDERR_BYTES", 1024, 30),
        ("success", "MAX_SOURCE_BYTES", 100, 30),
        ("timeout", None, None, 1),
    )
    for index, (mode, attribute, limit, timeout) in enumerate(bounded_faults):
        client = root / f"fake-mysql-bounded-{index}"
        fake_mysql(client, mode=mode)
        source = root / f"source-bounded-{index}.ndjson"
        original = getattr(mysql_snapshot, attribute) if attribute is not None else None
        if attribute is not None:
            setattr(mysql_snapshot, attribute, limit)
        try:
            expect_snapshot_error(
                mysql_snapshot.capture_snapshot,
                plan=plan,
                defaults_file=defaults,
                mysql_bin=str(client),
                source_path=source,
                timeout_seconds=timeout,
            )
        finally:
            if attribute is not None:
                setattr(mysql_snapshot, attribute, original)
        assert not source.exists()


def test_quarantine_bundle(root: pathlib.Path) -> None:
    plan_path = root / "plan-quarantine.json"
    defaults = root / "quarantine.cnf"
    client = root / "fake-mysql-quarantine"
    write_json(plan_path, plan_value())
    defaults.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\npassword=value\n"
        "database=frame\nssl-ca=/fixture/ca.pem\n",
        encoding="utf-8",
    )
    defaults.chmod(0o600)
    fake_mysql(client, mode="transform_reject")
    bundle = root / "bundle-quarantine"
    result = mysql_snapshot.export_mysql_snapshot(
        plan_path=plan_path,
        defaults_file=defaults,
        mysql_bin=str(client),
        bundle=bundle,
        chunk_rows=1,
        timeout_seconds=30,
    )
    manifest = result["manifest"]
    proof = result["proof"]
    assert manifest["row_count"] == 1
    assert manifest["reject_count"] == 1
    assert proof["captured_row_count"] == 2
    assert etl.load_manifest(bundle) == manifest
    quarantine = (bundle / "quarantine.ndjson").read_text(encoding="utf-8")
    assert "unknown_enum" in quarantine
    assert "tenant-a" not in quarantine


def test_query_contract(root: pathlib.Path) -> None:
    plan_path = root / "plan-query.json"
    write_json(plan_path, plan_value())
    plan = etl.load_plan(plan_path)
    query = mysql_snapshot.snapshot_sql(plan)
    assert query.index("SET @frame_gtid_before_snapshot") < query.index("START TRANSACTION")
    assert query.index("START TRANSACTION") < query.index(mysql_snapshot.BEGIN_MARKER)
    assert query.index(mysql_snapshot.BEGIN_MARKER) < query.index("SELECT JSON_OBJECT(")
    assert query.index("SELECT JSON_OBJECT(") < query.index(mysql_snapshot.END_MARKER)
    assert "SELECT 1 FROM `legacy_records` LIMIT 0;" in query
    assert "DATE_FORMAT(`source`.`occurred_at`, '%Y-%m-%dT%H:%i:%s.%fZ')" in query
    assert "ORDER BY `source`.`org_id`, `source`.`legacy_id`" in query
    assert "@frame_binding_warnings" in query
    assert "expression, is_visible" in query
    assert "information_schema.check_constraints" in query
    assert "column_default, is_nullable" in query
    malformed = plan_value()
    tables = malformed["tables"]
    assert isinstance(tables, list) and isinstance(tables[0], dict)
    tables[0]["source_table"] = "records; DROP TABLE users"
    malformed_path = root / "plan-malformed.json"
    write_json(malformed_path, malformed)
    expect_snapshot_error(mysql_snapshot.snapshot_sql, etl.load_plan(malformed_path))


def test_source_fingerprint(root: pathlib.Path) -> None:
    draft = plan_value()
    draft.pop("mysql_snapshot")
    plan_path = root / "plan-fingerprint-draft.json"
    defaults = root / "fingerprint.cnf"
    client = root / "fake-fingerprint-mysql"
    write_json(plan_path, draft)
    defaults.write_text(
        "[client]\nhost=mysql.invalid\nuser=fixture\npassword=value\n"
        "database=frame\nssl-ca=/fixture/ca.pem\n",
        encoding="utf-8",
    )
    defaults.chmod(0o600)
    fake_mysql(client)
    result = mysql_snapshot.fingerprint_mysql_source(
        plan_path=plan_path,
        defaults_file=defaults,
        mysql_bin=str(client),
        scratch_directory=root,
        timeout_seconds=30,
    )
    assert result["mysql_snapshot"] == plan_value()["mysql_snapshot"]
    assert result["minimum_mysql_version"] == "8.0.26"
    assert result["contains_source_values"] is False
    completed = subprocess.run(
        [
            sys.executable,
            "-I",
            str(pathlib.Path(mysql_snapshot.__file__)),
            "--fingerprint-source",
            "--plan",
            str(plan_path),
            "--defaults-file",
            str(defaults),
            "--mysql-bin",
            str(client),
            "--scratch-directory",
            str(root),
            "--timeout-seconds",
            "30",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    assert completed.returncode == 0, completed.stderr
    assert json.loads(completed.stdout)["mysql_snapshot"] == result["mysql_snapshot"]


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-mysql-snapshot-test-") as raw:
        root = pathlib.Path(raw)
        root.chmod(0o700)
        proof = test_happy_path(root)
        test_private_defaults(root)
        test_faults(root)
        test_quarantine_bundle(root)
        test_query_contract(root)
        test_source_fingerprint(root)
    print(
        etl.canonical(
            {
                "deterministic_fake_mysql_export_contract": True,
                "gtid_before_snapshot_boundary": True,
                "owner_private_credentials": True,
                "redacted_failure_surface": True,
                "etl_import_and_reconciliation": True,
                "value_free_source_fingerprint": True,
                "bounded_fault_injection": True,
                "atomic_immutable_publication": True,
                "manifest_core_attestation": True,
                "target_only_scope_detection": True,
                "row_count": proof["captured_row_count"],
                "production_evidence": False,
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
