#!/usr/bin/env python3
"""Create and verify deterministic release provenance and SSH signatures.

Production trust is deliberately external: the verifier accepts only an
explicit allowed-signers file.  ``--self-test`` creates a temporary Ed25519
key to prove signing, tamper rejection, and subject binding; that key is never
release evidence.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
from typing import Any


PREDICATE_TYPE = "https://slsa.dev/provenance/v1"
STATEMENT_TYPE = "https://in-toto.io/Statement/v1"
SIGNATURE_NAMESPACE = "frame-release"
SHA256_HEX = frozenset("0123456789abcdef")


class ProvenanceError(RuntimeError):
    pass


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode()


def read_json(path: pathlib.Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ProvenanceError(f"cannot read {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise ProvenanceError(f"{label} must be a JSON object")
    return value


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    try:
        with path.open("rb") as handle:
            while block := handle.read(1024 * 1024):
                digest.update(block)
    except OSError as error:
        raise ProvenanceError(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def is_sha256(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and set(value) <= SHA256_HEX


def is_git_sha(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and set(value) <= SHA256_HEX


def safe_relative_path(value: Any) -> pathlib.PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value:
        raise ProvenanceError("artifact path must be a non-empty POSIX path")
    path = pathlib.PurePosixPath(value)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise ProvenanceError(f"unsafe artifact path: {value}")
    return path


def manifest_subjects(manifest: dict[str, Any], bundle: pathlib.Path) -> list[dict[str, Any]]:
    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, dict) or not artifacts:
        raise ProvenanceError("release manifest artifacts must be a non-empty object")
    subjects: list[dict[str, Any]] = []
    paths: set[str] = set()
    for artifact_id, record in sorted(artifacts.items()):
        if not isinstance(artifact_id, str) or not isinstance(record, dict):
            raise ProvenanceError("release manifest artifact entry is invalid")
        relative = safe_relative_path(record.get("path"))
        relative_text = relative.as_posix()
        expected = record.get("sha256")
        if not is_sha256(expected):
            raise ProvenanceError(f"artifact {artifact_id} has an invalid SHA-256")
        if relative_text in paths:
            raise ProvenanceError(f"duplicate release artifact path: {relative_text}")
        path = bundle.joinpath(*relative.parts)
        if not path.is_file():
            raise ProvenanceError(f"release artifact is missing: {relative_text}")
        if sha256_file(path) != expected:
            raise ProvenanceError(f"release artifact digest mismatch: {relative_text}")
        paths.add(relative_text)
        subjects.append({"name": relative_text, "digest": {"sha256": expected}})
    return subjects


def build_statement(
    manifest_path: pathlib.Path,
    source_uri: str,
    builder_id: str,
) -> dict[str, Any]:
    manifest = read_json(manifest_path, "release manifest")
    git_sha = manifest.get("git_sha")
    if not is_git_sha(git_sha):
        raise ProvenanceError("release manifest git_sha must be a full lowercase Git SHA")
    if not source_uri.startswith("https://") or "@" in source_uri:
        raise ProvenanceError("source URI must be an HTTPS repository URI without a revision")
    if not builder_id.startswith("https://"):
        raise ProvenanceError("builder ID must be an HTTPS URI")
    subjects = manifest_subjects(manifest, manifest_path.parent)
    sbom = next((item for item in subjects if item["name"] == "frame.cdx.json"), None)
    metadata = next((item for item in subjects if item["name"] == "cargo-metadata.json"), None)
    if sbom is None or metadata is None:
        raise ProvenanceError("release subjects must include the SBOM and Cargo metadata")
    manifest_digest = sha256_file(manifest_path)
    return {
        "_type": STATEMENT_TYPE,
        "subject": subjects,
        "predicateType": PREDICATE_TYPE,
        "predicate": {
            "buildDefinition": {
                "buildType": "https://frame.engmanager.xyz/build/release-bundle/v1",
                "externalParameters": {
                    "releaseGitSha": git_sha,
                    "contractMajor": manifest.get("contract_major"),
                    "migrationLevel": manifest.get("migration_level"),
                },
                "internalParameters": {},
                "resolvedDependencies": [
                    {"uri": f"git+{source_uri}@{git_sha}", "digest": {"gitCommit": git_sha}},
                    {"uri": "pkg:generic/frame-cargo-metadata", "digest": metadata["digest"]},
                    {"uri": "pkg:generic/frame-sbom@cyclonedx-1.6", "digest": sbom["digest"]},
                ],
            },
            "runDetails": {
                "builder": {"id": builder_id},
                "metadata": {"invocationId": git_sha, "startedOn": None, "finishedOn": None},
                "byproducts": [
                    {"name": "release-manifest.json", "digest": {"sha256": manifest_digest}}
                ],
            },
        },
    }


def write_atomic(path: pathlib.Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    try:
        with temporary.open("xb") as handle:
            handle.write(payload)
        temporary.replace(path)
    except OSError as error:
        temporary.unlink(missing_ok=True)
        raise ProvenanceError(f"cannot write {path}: {error}") from error


def validate_statement(
    provenance_path: pathlib.Path,
    manifest_path: pathlib.Path,
    expected_sha: str,
) -> bytes:
    payload = provenance_path.read_bytes()
    statement = read_json(provenance_path, "provenance")
    if payload != canonical_json(statement):
        raise ProvenanceError("provenance must use canonical JSON with one trailing newline")
    if not is_git_sha(expected_sha):
        raise ProvenanceError("expected SHA must be a full lowercase Git SHA")
    expected = build_statement(
        manifest_path,
        source_uri=_source_uri(statement),
        builder_id=_builder_id(statement),
    )
    parameters = statement.get("predicate", {}).get("buildDefinition", {}).get(
        "externalParameters", {}
    )
    if parameters.get("releaseGitSha") != expected_sha:
        raise ProvenanceError("provenance source identity does not match expected SHA")
    if statement != expected:
        raise ProvenanceError("provenance does not exactly bind the release manifest and subjects")
    return payload


def _source_uri(statement: dict[str, Any]) -> str:
    try:
        uri = statement["predicate"]["buildDefinition"]["resolvedDependencies"][0]["uri"]
        git_sha = statement["predicate"]["buildDefinition"]["externalParameters"][
            "releaseGitSha"
        ]
    except (KeyError, IndexError, TypeError) as error:
        raise ProvenanceError("provenance source dependency is missing") from error
    suffix = f"@{git_sha}"
    if not isinstance(uri, str) or not uri.startswith("git+https://") or not uri.endswith(suffix):
        raise ProvenanceError("provenance source dependency is invalid")
    return uri.removeprefix("git+")[: -len(suffix)]


def _builder_id(statement: dict[str, Any]) -> str:
    try:
        value = statement["predicate"]["runDetails"]["builder"]["id"]
    except (KeyError, TypeError) as error:
        raise ProvenanceError("provenance builder identity is missing") from error
    if not isinstance(value, str):
        raise ProvenanceError("provenance builder identity is invalid")
    return value


def sign(provenance: pathlib.Path, private_key: pathlib.Path, signature: pathlib.Path) -> None:
    if shutil.which("ssh-keygen") is None:
        raise ProvenanceError("ssh-keygen is required for release provenance signatures")
    generated = provenance.with_name(f"{provenance.name}.sig")
    generated.unlink(missing_ok=True)
    try:
        subprocess.run(
            [
                "ssh-keygen",
                "-Y",
                "sign",
                "-q",
                "-f",
                str(private_key),
                "-n",
                SIGNATURE_NAMESPACE,
                str(provenance),
            ],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        write_atomic(signature, generated.read_bytes())
    except (OSError, subprocess.CalledProcessError) as error:
        raise ProvenanceError(f"provenance signing failed: {error}") from error
    finally:
        generated.unlink(missing_ok=True)


def verify_signature(payload: bytes, signature: pathlib.Path, allowed_signers: pathlib.Path) -> None:
    if shutil.which("ssh-keygen") is None:
        raise ProvenanceError("ssh-keygen is required for release provenance signatures")
    try:
        result = subprocess.run(
            [
                "ssh-keygen",
                "-Y",
                "verify",
                "-f",
                str(allowed_signers),
                "-I",
                "frame-release",
                "-n",
                SIGNATURE_NAMESPACE,
                "-s",
                str(signature),
            ],
            input=payload,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
    except OSError as error:
        raise ProvenanceError(f"cannot invoke signature verifier: {error}") from error
    if result.returncode != 0:
        raise ProvenanceError("release provenance signature is missing, untrusted, or invalid")


def create_fixture_bundle(root: pathlib.Path) -> tuple[pathlib.Path, str]:
    sha = "a" * 40
    artifacts: dict[str, dict[str, str]] = {}
    for artifact_id, name, payload in (
        ("web", "frame-web", b"synthetic web binary\n"),
        ("worker", "frame-worker.tar.gz", b"synthetic worker archive\n"),
        ("cargo_metadata", "cargo-metadata.json", b'{"packages":[]}\n'),
        ("sbom", "frame.cdx.json", b'{"bomFormat":"CycloneDX","specVersion":"1.6"}\n'),
    ):
        (root / name).write_bytes(payload)
        artifacts[artifact_id] = {"path": name, "sha256": hashlib.sha256(payload).hexdigest()}
    manifest = {
        "schema_version": 1,
        "git_sha": sha,
        "contract_major": 1,
        "migration_level": "0015_fixture.sql",
        "artifacts": artifacts,
    }
    path = root / "release-manifest.json"
    path.write_bytes(canonical_json(manifest))
    return path, sha


def self_test() -> None:
    if shutil.which("ssh-keygen") is None:
        raise ProvenanceError("ssh-keygen is required for the provenance self-test")
    with tempfile.TemporaryDirectory(prefix="frame-provenance-") as temporary:
        root = pathlib.Path(temporary)
        manifest, sha = create_fixture_bundle(root)
        provenance = root / "provenance.json"
        statement = build_statement(
            manifest,
            "https://github.com/eng-manager-xyz/frame",
            "https://github.com/eng-manager-xyz/frame/.github/workflows/production-gate.yml",
        )
        write_atomic(provenance, canonical_json(statement))
        payload = validate_statement(provenance, manifest, sha)

        key = root / "release_signer"
        subprocess.run(
            ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(key)],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        public = (root / "release_signer.pub").read_text(encoding="utf-8").strip()
        allowed = root / "allowed_signers"
        allowed.write_text(f"frame-release {public}\n", encoding="utf-8")
        signature = root / "provenance.sshsig"
        sign(provenance, key, signature)
        verify_signature(payload, signature, allowed)

        tampered = copy.deepcopy(statement)
        tampered["subject"][0]["digest"]["sha256"] = "b" * 64
        provenance.write_bytes(canonical_json(tampered))
        try:
            validate_statement(provenance, manifest, sha)
        except ProvenanceError:
            pass
        else:
            raise ProvenanceError("tampered subject unexpectedly verified")
        try:
            verify_signature(provenance.read_bytes(), signature, allowed)
        except ProvenanceError:
            pass
        else:
            raise ProvenanceError("tampered signature payload unexpectedly verified")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    commands = parser.add_subparsers(dest="command")
    generate = commands.add_parser("generate")
    generate.add_argument("--release-manifest", type=pathlib.Path, required=True)
    generate.add_argument("--output", type=pathlib.Path, required=True)
    generate.add_argument("--source-uri", required=True)
    generate.add_argument("--builder-id", required=True)
    signing = commands.add_parser("sign")
    signing.add_argument("--provenance", type=pathlib.Path, required=True)
    signing.add_argument("--private-key", type=pathlib.Path, required=True)
    signing.add_argument("--signature", type=pathlib.Path, required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("--provenance", type=pathlib.Path, required=True)
    verify.add_argument("--release-manifest", type=pathlib.Path, required=True)
    verify.add_argument("--expected-sha", required=True)
    verify.add_argument("--signature", type=pathlib.Path, required=True)
    verify.add_argument("--allowed-signers", type=pathlib.Path, required=True)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        if args.self_test:
            if args.command is not None:
                raise ProvenanceError("--self-test cannot be combined with a command")
            self_test()
            print("release provenance self-test passed (ephemeral signer; not release evidence)")
            return 0
        if args.command == "generate":
            statement = build_statement(args.release_manifest, args.source_uri, args.builder_id)
            write_atomic(args.output, canonical_json(statement))
            print(f"wrote deterministic release provenance: {args.output}")
            return 0
        if args.command == "sign":
            sign(args.provenance, args.private_key, args.signature)
            print(f"wrote SSH release signature: {args.signature}")
            return 0
        if args.command == "verify":
            payload = validate_statement(
                args.provenance, args.release_manifest, args.expected_sha
            )
            verify_signature(payload, args.signature, args.allowed_signers)
            print("verified release subjects, source identity, provenance, and trusted signature")
            return 0
        raise ProvenanceError("choose --self-test or a generate, sign, or verify command")
    except (OSError, ProvenanceError, subprocess.CalledProcessError) as error:
        print(f"release provenance failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
