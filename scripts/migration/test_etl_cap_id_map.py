#!/usr/bin/env python3
"""Prove the Cap ID mapping is the ETL transform used by every PK/FK column."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures" / "parity" / "v1" / "cap-id-map-v1.json"


def load_module(name: str, path: pathlib.Path):
    specification = importlib.util.spec_from_file_location(name, path)
    if specification is None or specification.loader is None:
        raise RuntimeError(f"could not load {name}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[name] = module
    specification.loader.exec_module(module)
    return module


CAP_IDS = load_module("frame_test_cap_id_map", ROOT / "scripts/migration/cap_id_map.py")
ETL = load_module("frame_test_etl", ROOT / "scripts/migration/etl.py")


def column(target: str):
    return ETL.Column(
        source=target,
        target=target,
        transform="cap_nanoid_uuid_v1",
        nullable=False,
        options={},
        has_default=False,
        default=None,
    )


def main() -> int:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    primary_key = column("id")
    foreign_key = column("parent_id")
    for item in fixture["mappings"]:
        source = item["cap_nanoid"]
        expected = item["frame_uuid"]
        assert ETL.transform_value(primary_key, source) == expected
        assert ETL.transform_value(foreign_key, source) == expected
        assert ETL.transform_value(primary_key, source) == CAP_IDS.map_cap_nanoid(source)

    for invalid in [None, 123, "short", "i" * 15, "A" * 15]:
        try:
            ETL.transform_value(primary_key, invalid)
        except ValueError as error:
            assert str(error) in {"required_value_missing", "invalid_cap_nanoid"}
        else:
            raise AssertionError("invalid Cap identifier reached a D1 bundle")

    plan = json.loads((ROOT / "fixtures/etl/v1/plan.json").read_text(encoding="utf-8"))
    plan["tables"][0]["columns"][0]["transform"] = "cap_nanoid_uuid_v1"
    plan["tables"][0]["columns"][0]["options"] = {"namespace": "forbidden"}
    with tempfile.TemporaryDirectory() as directory:
        path = pathlib.Path(directory) / "invalid-cap-options.json"
        path.write_text(json.dumps(plan), encoding="utf-8")
        try:
            ETL.load_plan(path)
        except ETL.EtlError as error:
            assert str(error) == "plan transform options are invalid"
        else:
            raise AssertionError("option-bearing Cap ID transform was accepted")

    print(
        json.dumps(
            {
                "contract_version": fixture["contract_version"],
                "known_answers": len(fixture["mappings"]),
                "pk_fk_stability": True,
                "status": "ok",
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
