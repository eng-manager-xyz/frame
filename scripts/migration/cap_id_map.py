#!/usr/bin/env python3
"""Deterministic pinned-Cap NanoID to Frame UUIDv8 mapping contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import uuid
from collections.abc import Sequence


CONTRACT_VERSION = 1
CAP_NANOID_LENGTH = 15
CAP_NANOID_ALPHABET = frozenset("0123456789abcdefghjkmnpqrstvwxyz")
NAMESPACE = b"frame-cap-nanoid-to-uuid-v1\0"


def map_cap_nanoid(value: str) -> str:
    """Map one validated Cap NanoID identically in every PK/FK position."""
    if not isinstance(value, str) or len(value) != CAP_NANOID_LENGTH or any(
        character not in CAP_NANOID_ALPHABET for character in value
    ):
        raise ValueError("invalid_cap_nanoid")
    raw = bytearray(hashlib.sha256(NAMESPACE + value.encode("ascii")).digest()[:16])
    raw[6] = (raw[6] & 0x0F) | 0x80
    raw[8] = (raw[8] & 0x3F) | 0x80
    return str(uuid.UUID(bytes=bytes(raw)))


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ids", nargs="+", help="15-character pinned-Cap NanoIDs")
    return parser.parse_args(argv)


def main(argv: Sequence[str]) -> int:
    args = parse_args(argv)
    result = {
        "contract_version": CONTRACT_VERSION,
        "namespace": NAMESPACE[:-1].decode("ascii"),
        "mappings": [
            {"cap_nanoid": value, "frame_uuid": map_cap_nanoid(value)} for value in args.ids
        ],
    }
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except ValueError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2) from error
