#!/usr/bin/env python3
"""Materialize the production D1 identifier without committing or logging it."""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path


PLACEHOLDER = 'database_id = "replace-with-d1-database-id"'


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: prepare-wrangler-config.py INPUT OUTPUT", file=sys.stderr)
        return 2

    source = Path(sys.argv[1])
    destination = Path(sys.argv[2])
    database_id = os.environ.get("CLOUDFLARE_D1_DATABASE_ID", "")
    if re.fullmatch(r"[0-9a-f]{32}", database_id) is None:
        print("CLOUDFLARE_D1_DATABASE_ID must be a 32-character lowercase hexadecimal ID", file=sys.stderr)
        return 1

    text = source.read_text(encoding="utf-8")
    required_fragments = (
        'workers_dev = false',
        'FRAME_DEPLOYMENT = "production"',
        'FRAME_PUBLIC_HOST = "frame.engmanager.xyz"',
        'pattern = "frame.engmanager.xyz/api*"',
        'binding = "DB"',
    )
    missing = [fragment for fragment in required_fragments if fragment not in text]
    if missing:
        print("Wrangler production invariants are missing: " + ", ".join(missing), file=sys.stderr)
        return 1
    if text.count(PLACEHOLDER) != 1:
        print("Wrangler config must contain exactly one protected D1 placeholder", file=sys.stderr)
        return 1

    rendered = text.replace(PLACEHOLDER, f'database_id = "{database_id}"')
    if "replace-with-" in rendered:
        print("Wrangler config still contains an unresolved production placeholder", file=sys.stderr)
        return 1

    # The protected job deploys the checksummed dry-run bundle. Removing the
    # custom source build is what makes a provider-side rebuild impossible.
    lines: list[str] = []
    skipping_build = False
    build_sections = 0
    for line in rendered.splitlines(keepends=True):
        if line.strip() == "[build]":
            skipping_build = True
            build_sections += 1
            continue
        if skipping_build and line.lstrip().startswith("["):
            skipping_build = False
        if not skipping_build:
            lines.append(line)
    if build_sections != 1:
        print("Wrangler config must contain exactly one removable build section", file=sys.stderr)
        return 1
    rendered = "".join(lines)

    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        handle.write(rendered)
    print(f"Prepared artifact-only Wrangler config at {destination} (identifier redacted).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
