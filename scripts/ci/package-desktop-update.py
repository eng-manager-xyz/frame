#!/usr/bin/env python3
"""Create the exact R2 payload for one signed Tauri desktop update."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import tempfile
from pathlib import Path


MAX_ARTIFACT_BYTES = 512 * 1024 * 1024
PREFIX = Path("system/desktop-updates/v1/stable")
VERSION = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


class PackageError(ValueError):
    pass


def fail(message: str) -> None:
    raise PackageError(message)


def coordinates(target: str, arch: str, bundle: str) -> None:
    if target not in {"darwin", "windows"}:
        fail("target must be darwin or windows")
    if arch not in {"aarch64", "x86_64"}:
        fail("architecture is unsupported")
    if (target, bundle) not in {
        ("darwin", "app"),
        ("windows", "nsis"),
        ("windows", "msi"),
    }:
        fail("bundle does not belong to the target")


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def signature(path: Path) -> str:
    value = path.read_text(encoding="ascii").strip()
    if (
        not 32 <= len(value) <= 2048
        or any(character.isspace() for character in value)
        or not value.isascii()
    ):
        fail("signature is not a bounded Tauri minisign payload")
    return value


def package(
    *,
    target: str,
    arch: str,
    bundle: str,
    version: str,
    artifact: Path,
    signature_path: Path,
    output: Path,
    notes: str | None,
    pub_date: str | None,
) -> dict[str, object]:
    coordinates(target, arch, bundle)
    if not VERSION.fullmatch(version) or len(version) > 64:
        fail("version must be canonical semver")
    if not artifact.is_file() or artifact.is_symlink():
        fail("artifact must be a regular file")
    size = artifact.stat().st_size
    if not 1 <= size <= MAX_ARTIFACT_BYTES:
        fail("artifact size is outside the updater budget")
    if notes is not None and (len(notes) > 8192 or "\0" in notes):
        fail("release notes are invalid")
    if pub_date is not None and (
        len(pub_date) > 64 or not pub_date.isascii() or any(c.isspace() for c in pub_date)
    ):
        fail("publication date is invalid")

    root = output / PREFIX / target / arch / bundle
    version_root = root / version
    version_root.mkdir(parents=True, exist_ok=False)
    target_artifact = version_root / "artifact"
    shutil.copyfile(artifact, target_artifact)
    pointer: dict[str, object] = {
        "schema_version": 1,
        "version": version,
        "signature": signature(signature_path),
        "bytes": size,
        "sha256": digest(target_artifact),
        "notes": notes,
        "pub_date": pub_date,
    }
    (root / "latest.json").write_text(
        json.dumps(pointer, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return pointer


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="frame-desktop-update-") as temporary:
        root = Path(temporary)
        artifact = root / "Frame.app.tar.gz"
        signature_path = root / "Frame.app.tar.gz.sig"
        artifact.write_bytes(b"signed-tauri-artifact")
        signature_path.write_text("A" * 64 + "\n", encoding="ascii")
        output = root / "payload"
        pointer = package(
            target="darwin",
            arch="aarch64",
            bundle="app",
            version="1.2.3",
            artifact=artifact,
            signature_path=signature_path,
            output=output,
            notes="security update",
            pub_date="2026-07-29T12:00:00Z",
        )
        assert pointer["bytes"] == len(b"signed-tauri-artifact")
        assert (
            output
            / PREFIX
            / "darwin/aarch64/app/1.2.3/artifact"
        ).read_bytes() == b"signed-tauri-artifact"
        try:
            package(
                target="darwin",
                arch="../escape",
                bundle="app",
                version="1.2.4",
                artifact=artifact,
                signature_path=signature_path,
                output=root / "escape",
                notes=None,
                pub_date=None,
            )
        except PackageError:
            pass
        else:
            raise AssertionError("unsafe coordinate accepted")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=("darwin", "windows"))
    parser.add_argument("--arch", choices=("aarch64", "x86_64"))
    parser.add_argument("--bundle", choices=("app", "nsis", "msi"))
    parser.add_argument("--version")
    parser.add_argument("--artifact", type=Path)
    parser.add_argument("--signature", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--notes")
    parser.add_argument("--pub-date")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        print("desktop updater packaging contract passed")
        return 0
    required = {
        "target": args.target,
        "arch": args.arch,
        "bundle": args.bundle,
        "version": args.version,
        "artifact": args.artifact,
        "signature": args.signature,
        "output": args.output,
    }
    missing = [name for name, value in required.items() if value is None]
    if missing:
        parser.error(f"missing required arguments: {', '.join(missing)}")
    try:
        pointer = package(
            target=args.target,
            arch=args.arch,
            bundle=args.bundle,
            version=args.version,
            artifact=args.artifact,
            signature_path=args.signature,
            output=args.output,
            notes=args.notes,
            pub_date=args.pub_date,
        )
    except (OSError, UnicodeError, PackageError) as error:
        raise SystemExit(f"desktop update packaging failed: {error}") from error
    print(json.dumps(pointer, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
