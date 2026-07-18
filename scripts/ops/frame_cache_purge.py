#!/usr/bin/env python3
"""Plan or apply an exact Frame-host cache purge; whole-zone purge is impossible."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
from urllib.parse import urlsplit


HOST = "frame.engmanager.xyz"
TAG = re.compile(r"^frame:[a-z0-9][a-z0-9:_-]{0,127}$")


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--url", action="append", dest="urls")
    group.add_argument("--tag", action="append", dest="tags")
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--confirm-host")
    return parser.parse_args()


def payload(urls: list[str] | None, tags: list[str] | None) -> dict[str, list[str]]:
    values = urls or tags or []
    if not values or len(values) > 30 or len(set(values)) != len(values):
        raise ValueError("provide 1-30 unique exact URLs or namespaced tags")
    if urls:
        for value in values:
            parsed = urlsplit(value)
            if (
                parsed.scheme != "https"
                or parsed.hostname != HOST
                or parsed.port is not None
                or parsed.username is not None
                or parsed.password is not None
                or not parsed.path.startswith("/")
                or parsed.query
                or parsed.fragment
            ):
                raise ValueError("purge URLs must be exact query-free canonical Frame HTTPS URLs")
        return {"files": values}
    if any(TAG.fullmatch(value) is None for value in values):
        raise ValueError("purge tags must use the bounded frame: namespace")
    return {"tags": values}


def main() -> int:
    args = arguments()
    try:
        body = payload(args.urls, args.tags)
    except ValueError as error:
        print(str(error), file=sys.stderr)
        return 2
    if not args.apply:
        print(json.dumps({"mode": "dry-run", "host": HOST, "purge": body}, sort_keys=True))
        return 0
    if args.confirm_host != HOST:
        print(f"--apply requires --confirm-host {HOST}", file=sys.stderr)
        return 2
    token = os.environ.get("CLOUDFLARE_API_TOKEN", "")
    zone_id = os.environ.get("CLOUDFLARE_ZONE_ID", "")
    if not token or re.fullmatch(r"[0-9a-f]{32}", zone_id) is None:
        print("protected Cloudflare token and 32-character zone ID are required", file=sys.stderr)
        return 2
    request = urllib.request.Request(
        f"https://api.cloudflare.com/client/v4/zones/{zone_id}/purge_cache",
        data=json.dumps(body, separators=(",", ":")).encode(),
        method="POST",
        headers={"authorization": f"Bearer {token}", "content-type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            result = json.loads(response.read(65_536))
    except (OSError, urllib.error.URLError, json.JSONDecodeError) as error:
        print(f"scoped purge failed ({type(error).__name__})", file=sys.stderr)
        return 1
    if not result.get("success"):
        print("scoped purge was rejected", file=sys.stderr)
        return 1
    print(json.dumps({"mode": "applied", "host": HOST, "kind": next(iter(body))}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
