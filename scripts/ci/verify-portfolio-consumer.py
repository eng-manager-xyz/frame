#!/usr/bin/env python3
"""Verify a read-only portfolio consumer checkout against a Frame candidate.

The verifier never copies fixtures, edits a lockfile, or runs a command named by
the consumer. The protected workflow invokes a fixed Cargo test target after
this structural check succeeds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


FIXTURE_NAMES = {
    "contract.schema.json",
    "error.json",
    "health.additive.json",
    "health.ok.json",
    "share.deleted.json",
    "share.failed.json",
    "share.private.json",
    "share.processing.json",
    "share.public.json",
    "share.unavailable.json",
}
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
PINNED_PORTFOLIO_TOOLCHAIN = "nightly-2026-05-08"
PINNED_CHECKOUT = "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5"


class VerificationFailure(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationFailure(message)


def read_object(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationFailure(f"{path.name} is not valid UTF-8 JSON") from error
    require(isinstance(value, dict), f"{path.name} must contain an object")
    return value


def fixture_digest(directory: Path) -> str:
    names = {path.name for path in directory.glob("*.json") if path.name != "source.json"}
    require(names == FIXTURE_NAMES, "portfolio fixture inventory drifted")
    digest = hashlib.sha256()
    for name in sorted(names):
        body = (directory / name).read_bytes()
        require(len(body) <= 128 * 1024, f"{name} exceeds the consumer fixture limit")
        json.loads(body)
        digest.update(name.encode("utf-8"))
        digest.update(b"\0")
        digest.update(body)
        digest.update(b"\0")
    return digest.hexdigest()


def git_head(root: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
    )
    require(result.returncode == 0, "portfolio checkout has no readable Git revision")
    return result.stdout.strip()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="verify a protected portfolio contract-consumer checkout"
    )
    parser.add_argument("--portfolio-root", required=True)
    parser.add_argument("--candidate-root", required=True)
    parser.add_argument("--expected-portfolio-sha", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    portfolio = Path(args.portfolio_root).resolve()
    candidate = Path(args.candidate_root).resolve()
    require(portfolio.is_dir() and candidate.is_dir(), "checkout root is missing")
    require(
        SHA_RE.fullmatch(args.expected_portfolio_sha) is not None,
        "expected portfolio revision must be a lowercase full SHA",
    )
    require(
        git_head(portfolio) == args.expected_portfolio_sha,
        "portfolio checkout does not match the recorded last-released revision",
    )
    require((portfolio / "Cargo.lock").is_file(), "portfolio root Cargo.lock is missing")

    toolchain = (portfolio / "rust-toolchain.toml").read_text(encoding="utf-8")
    require(
        f'channel = "{PINNED_PORTFOLIO_TOOLCHAIN}"' in toolchain,
        "portfolio toolchain pin drifted",
    )
    workflow = (portfolio / ".github/workflows/frame-contract.yml").read_text(
        encoding="utf-8"
    )
    require(
        "permissions:\n  contents: read" in workflow
        and PINNED_CHECKOUT in workflow
        and "pull_request:" in workflow
        and "cargo test --locked -p website --test frame_contract" in workflow
        and "${{ secrets." not in workflow
        and "continue-on-error: true" not in workflow,
        "portfolio contract workflow is mutable, privileged, or incomplete",
    )

    copy_root = portfolio / "fixtures/frame-api/v1"
    source = read_object(copy_root / "source.json")
    require(
        set(source)
        == {
            "schema_version",
            "source_repository",
            "source_commit_sha",
            "fixture_set_sha256",
            "drift_check",
        },
        "portfolio fixture source metadata drifted",
    )
    require(
        source["schema_version"] == 1
        and source["source_repository"] == "eng-manager-xyz/frame"
        and isinstance(source["source_commit_sha"], str)
        and SHA_RE.fullmatch(source["source_commit_sha"]) is not None
        and source["drift_check"] == "sha256_path_and_bytes_v1",
        "portfolio fixture source authority is invalid",
    )
    require(
        source["fixture_set_sha256"] == fixture_digest(copy_root),
        "portfolio fixture copy does not match its recorded digest",
    )
    fixture_digest(candidate / "fixtures/frame-api/v1")

    consumer_test = portfolio / "website/tests/frame_contract.rs"
    consumer_source = consumer_test.read_text(encoding="utf-8")
    require(
        "FRAME_CONTRACT_FIXTURE_ROOT" in consumer_source
        and "source.json" in consumer_source
        and "health.additive.json" in consumer_source
        and "compatibility" in consumer_source.lower(),
        "portfolio test target does not consume the candidate fixture boundary",
    )
    print(
        "verified read-only last-released portfolio consumer structure and candidate fixture inventory"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (VerificationFailure, OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        print(f"portfolio consumer verification failed: {error}", file=sys.stderr)
        raise SystemExit(1) from None
