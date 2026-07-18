#!/usr/bin/env python3
"""Executable known-answer tests for the shared Cap ID mapping contract."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "fixtures" / "parity" / "v1" / "cap-id-map-v1.json"


def load_mapper():
    path = ROOT / "scripts" / "migration" / "cap_id_map.py"
    specification = importlib.util.spec_from_file_location("frame_test_cap_id_map", path)
    if specification is None or specification.loader is None:
        raise RuntimeError("could not load Cap ID mapper")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


CAP_IDS = load_mapper()


def main() -> int:
    fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
    assert fixture["schema_version"] == 1
    assert fixture["corpus_version"] == "parity-v1"
    assert fixture["contract_version"] == 1
    assert fixture["source_shape"] == {
        "alphabet": "".join(sorted(CAP_IDS.CAP_NANOID_ALPHABET)),
        "length": CAP_IDS.CAP_NANOID_LENGTH,
    }
    known = {
        item["cap_nanoid"]: item["frame_uuid"] for item in fixture["mappings"]
    }
    for source, expected in known.items():
        assert CAP_IDS.map_cap_nanoid(source) == expected
        assert CAP_IDS.map_cap_nanoid(source) == expected
    for relation in fixture["foreign_key_examples"]:
        assert CAP_IDS.map_cap_nanoid(relation["cap_nanoid"]) == relation["frame_uuid"]
    for invalid in [None, 123, "short", "i" * 15, "A" * 15, "0" * 14 + "-"]:
        try:
            CAP_IDS.map_cap_nanoid(invalid)
        except ValueError as error:
            assert str(error) == "invalid_cap_nanoid"
        else:
            raise AssertionError(f"invalid ID accepted: {invalid}")
    print(json.dumps({"known_answers": len(known), "status": "ok"}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
