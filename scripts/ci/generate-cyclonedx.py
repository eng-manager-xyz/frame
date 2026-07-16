#!/usr/bin/env python3
"""Generate a deterministic CycloneDX 1.6 SBOM from Cargo metadata.

The default mode runs Cargo without a shell and without network access.  A saved
metadata document can be supplied for hermetic tests or release replay.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path
from typing import Any, Iterable
from urllib.parse import quote, urlparse


SCHEMA = "http://cyclonedx.org/schema/bom-1.6.schema.json"
CRATES_IO_SOURCES = {
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
}
SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")
SPDX_TOKEN_RE = re.compile(
    r"DocumentRef-[A-Za-z0-9.-]+:LicenseRef-[A-Za-z0-9.-]+"
    r"|LicenseRef-[A-Za-z0-9.-]+"
    r"|AND|OR|WITH"
    r"|[A-Za-z0-9][A-Za-z0-9.+-]*"
    r"|[()]"
)


class SbomError(RuntimeError):
    """A user-actionable SBOM generation error."""


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--metadata",
        type=Path,
        help="read Cargo metadata JSON from this path instead of invoking Cargo",
    )
    parser.add_argument(
        "--cargo-lock",
        type=Path,
        help="Cargo.lock used to add registry SHA-256 checksums",
    )
    parser.add_argument(
        "--workspace",
        type=Path,
        default=Path.cwd(),
        help="workspace directory or Cargo.toml (default: current directory)",
    )
    parser.add_argument("--workspace-name", help="override the aggregate component name")
    parser.add_argument("--cargo", default="cargo", help="Cargo executable (default: cargo)")
    parser.add_argument(
        "--online",
        action="store_true",
        help="allow Cargo metadata to access the network (offline is the default)",
    )
    parser.add_argument(
        "--require-registry-checksums",
        action="store_true",
        help="fail if a registry component has no valid Cargo.lock checksum",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="write the BOM atomically to this path (default: stdout)",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run the hermetic fixture and determinism tests",
    )
    return parser.parse_args(argv)


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SbomError(f"cannot read metadata JSON {path}: {error}") from error
    if not isinstance(value, dict):
        raise SbomError(f"metadata JSON {path} must contain an object")
    return value


def cargo_metadata(cargo: str, workspace: Path, online: bool) -> dict[str, Any]:
    workspace = workspace.resolve()
    if workspace.is_file():
        manifest = workspace
        cwd = workspace.parent
    else:
        manifest = workspace / "Cargo.toml"
        cwd = workspace
    if not manifest.is_file():
        raise SbomError(f"Cargo manifest does not exist: {manifest}")

    command = [
        cargo,
        "metadata",
        "--format-version",
        "1",
        "--locked",
        "--all-features",
        "--manifest-path",
        str(manifest),
    ]
    if not online:
        command.append("--offline")
    try:
        completed = subprocess.run(
            command,
            cwd=cwd,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except FileNotFoundError as error:
        raise SbomError(f"Cargo executable not found: {cargo}") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or f"exit status {error.returncode}"
        raise SbomError(f"cargo metadata failed: {detail}") from error
    try:
        metadata = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise SbomError(f"cargo metadata emitted invalid JSON: {error}") from error
    if not isinstance(metadata, dict):
        raise SbomError("cargo metadata did not emit a JSON object")
    return metadata


def infer_lock_path(
    explicit: Path | None, metadata_path: Path | None, workspace: Path, metadata: dict[str, Any]
) -> Path | None:
    if explicit is not None:
        return explicit
    candidates: list[Path] = []
    if metadata_path is not None:
        candidates.append(metadata_path.resolve().parent / "Cargo.lock")
    root = metadata.get("workspace_root")
    if isinstance(root, str):
        candidates.append(Path(root) / "Cargo.lock")
    workspace = workspace.resolve()
    candidates.append((workspace if workspace.is_dir() else workspace.parent) / "Cargo.lock")
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def load_lock_checksums(path: Path | None) -> dict[tuple[str, str, str | None], str]:
    if path is None:
        return {}
    try:
        document = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise SbomError(f"cannot read Cargo lockfile {path}: {error}") from error
    packages = document.get("package", [])
    if not isinstance(packages, list):
        raise SbomError(f"Cargo lockfile {path} has an invalid package table")

    checksums: dict[tuple[str, str, str | None], str] = {}
    for package in packages:
        if not isinstance(package, dict):
            continue
        name = package.get("name")
        version = package.get("version")
        source = package.get("source")
        checksum = package.get("checksum")
        if not all(isinstance(value, str) for value in (name, version, checksum)):
            continue
        if not SHA256_RE.fullmatch(checksum):
            raise SbomError(
                f"Cargo lockfile {path} has a non-SHA-256 checksum for {name} {version}"
            )
        key = (name, version, source if isinstance(source, str) else None)
        checksums[key] = checksum.lower()
    return checksums


def encode_purl(value: str) -> str:
    return quote(value, safe="-._~")


def purl(name: str, version: str, qualifiers: dict[str, str] | None = None) -> str:
    result = f"pkg:cargo/{encode_purl(name)}@{encode_purl(version)}"
    if qualifiers:
        query = "&".join(
            f"{encode_purl(key)}={encode_purl(value)}"
            for key, value in sorted(qualifiers.items())
        )
        result = f"{result}?{query}"
    return result


def relative_package_path(package: dict[str, Any], workspace_root: Path) -> str | None:
    manifest = package.get("manifest_path")
    if not isinstance(manifest, str):
        return None
    try:
        relative = Path(manifest).resolve().parent.relative_to(workspace_root.resolve())
    except ValueError:
        return None
    value = relative.as_posix()
    return value if value != "." else None


def source_qualifiers(
    package: dict[str, Any], workspace_ids: set[str], workspace_root: Path
) -> dict[str, str]:
    source = package.get("source")
    package_id = package.get("id")
    if package_id in workspace_ids:
        qualifiers = {"workspace": "true"}
        path = relative_package_path(package, workspace_root)
        if path:
            qualifiers["workspace_path"] = path
        return qualifiers
    if not isinstance(source, str):
        path = relative_package_path(package, workspace_root)
        return {"local_path": path} if path else {"local": "true"}
    if source in CRATES_IO_SOURCES:
        return {}
    if source.startswith("git+"):
        return {"vcs_url": source}
    if source.startswith("registry+") or source.startswith("sparse+"):
        return {"repository_url": source.split("+", 1)[1]}
    return {"download_url": source}


def package_checksum(
    package: dict[str, Any], checksums: dict[tuple[str, str, str | None], str]
) -> str | None:
    embedded = package.get("checksum")
    if isinstance(embedded, str):
        if not SHA256_RE.fullmatch(embedded):
            raise SbomError(
                f"metadata has a non-SHA-256 checksum for {package.get('name')} "
                f"{package.get('version')}"
            )
        return embedded.lower()

    name = package.get("name")
    version = package.get("version")
    source = package.get("source")
    exact = checksums.get((name, version, source))
    if exact:
        return exact
    # Cargo can describe crates.io using its sparse or historical git index.
    # Never fall back by name/version for git dependencies: that could attach a
    # registry archive's checksum to unrelated source code with the same name.
    if source not in CRATES_IO_SOURCES:
        return None
    matches = {
        checksum
        for (candidate_name, candidate_version, candidate_source), checksum in checksums.items()
        if candidate_name == name and candidate_version == version
        and candidate_source in CRATES_IO_SOURCES
    }
    return next(iter(matches)) if len(matches) == 1 else None


def is_http_url(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    parsed = urlparse(value)
    return parsed.scheme in {"http", "https"} and bool(parsed.netloc)


def external_references(package: dict[str, Any]) -> list[dict[str, str]]:
    references: set[tuple[str, str]] = set()
    for field, reference_type in (
        ("repository", "vcs"),
        ("homepage", "website"),
        ("documentation", "documentation"),
    ):
        value = package.get(field)
        if is_http_url(value):
            references.add((reference_type, value))

    source = package.get("source")
    if source in CRATES_IO_SOURCES:
        name = package.get("name")
        version = package.get("version")
        references.add(("distribution", f"https://crates.io/crates/{name}/{version}"))
    elif isinstance(source, str) and source.startswith("git+"):
        value = source.removeprefix("git+")
        if "#" in value:
            value = value.rsplit("#", 1)[0]
        if is_http_url(value):
            references.add(("vcs", value))
    return [
        {"type": reference_type, "url": url}
        for reference_type, url in sorted(references)
    ]


def target_kinds(package: dict[str, Any]) -> list[str]:
    kinds: set[str] = set()
    for target in package.get("targets", []):
        if isinstance(target, dict):
            target_kind = target.get("kind", [])
            if isinstance(target_kind, list):
                kinds.update(kind for kind in target_kind if isinstance(kind, str))
    return sorted(kinds)


def component_type(package: dict[str, Any], workspace_ids: set[str]) -> str:
    if package.get("id") in workspace_ids:
        kinds = set(target_kinds(package))
        if kinds.intersection({"bin", "cdylib", "staticlib"}):
            return "application"
    return "library"


def is_spdx_expression(value: str) -> bool:
    """Conservatively check SPDX expression syntax without a license catalog."""
    tokens = SPDX_TOKEN_RE.findall(value)
    if not tokens or "".join(tokens) != re.sub(r"\s+", "", value):
        return False
    position = 0

    def parse_factor() -> bool:
        nonlocal position
        if position >= len(tokens):
            return False
        if tokens[position] == "(":
            position += 1
            if not parse_expression() or position >= len(tokens) or tokens[position] != ")":
                return False
            position += 1
            return True
        if tokens[position] in {"AND", "OR", "WITH", ")"}:
            return False
        position += 1
        if position < len(tokens) and tokens[position] == "WITH":
            position += 1
            if position >= len(tokens) or tokens[position] in {"AND", "OR", "WITH", "(", ")"}:
                return False
            position += 1
        return True

    def parse_expression() -> bool:
        nonlocal position
        if not parse_factor():
            return False
        while position < len(tokens) and tokens[position] in {"AND", "OR"}:
            position += 1
            if not parse_factor():
                return False
        return True

    return parse_expression() and position == len(tokens)


def make_component(
    package: dict[str, Any],
    workspace_ids: set[str],
    workspace_root: Path,
    checksums: dict[tuple[str, str, str | None], str],
) -> dict[str, Any]:
    name = package.get("name")
    version = package.get("version")
    if not isinstance(name, str) or not isinstance(version, str):
        raise SbomError("every Cargo package must have a string name and version")
    package_purl = purl(name, version, source_qualifiers(package, workspace_ids, workspace_root))
    component: dict[str, Any] = {
        "bom-ref": package_purl,
        "name": name,
        "purl": package_purl,
        "type": component_type(package, workspace_ids),
        "version": version,
    }
    description = package.get("description")
    if isinstance(description, str) and description.strip():
        component["description"] = description.strip()

    license_expression = package.get("license")
    if isinstance(license_expression, str) and license_expression.strip():
        license_expression = license_expression.strip()
        if is_spdx_expression(license_expression):
            component["licenses"] = [{"expression": license_expression}]
        else:
            # Some older crates use legacy declarations such as MIT/Apache-2.0.
            # Preserve those declarations verbatim without falsely presenting
            # them as valid SPDX expressions.
            component["licenses"] = [{"license": {"name": license_expression}}]
    else:
        license_file = package.get("license_file")
        if isinstance(license_file, str) and license_file:
            component["licenses"] = [
                {"license": {"name": f"Declared in {Path(license_file).name}"}}
            ]

    checksum = package_checksum(package, checksums)
    if checksum:
        component["hashes"] = [{"alg": "SHA-256", "content": checksum}]

    references = external_references(package)
    if references:
        component["externalReferences"] = references

    properties: list[dict[str, str]] = []
    if package.get("id") in workspace_ids:
        properties.append({"name": "cargo:workspace_member", "value": "true"})
    kinds = target_kinds(package)
    if kinds:
        properties.append({"name": "cargo:target_kinds", "value": ",".join(kinds)})
    rust_version = package.get("rust_version")
    if isinstance(rust_version, str):
        properties.append({"name": "cargo:rust_version", "value": rust_version})
    if properties:
        component["properties"] = sorted(properties, key=lambda item: item["name"])
    return component


def repository_workspace_name(packages: Iterable[dict[str, Any]], fallback: str) -> str:
    repositories = {
        package["repository"]
        for package in packages
        if is_http_url(package.get("repository"))
    }
    if len(repositories) == 1:
        path = urlparse(next(iter(repositories))).path.rstrip("/")
        candidate = Path(path).name.removesuffix(".git")
        if candidate:
            return candidate
    return fallback


def workspace_component(
    member_packages: list[dict[str, Any]], workspace_root: Path, name_override: str | None
) -> dict[str, Any]:
    fallback = workspace_root.name or "cargo"
    base_name = name_override or repository_workspace_name(member_packages, fallback)
    name = base_name if base_name.endswith("-workspace") else f"{base_name}-workspace"
    versions = {package.get("version") for package in member_packages}
    version = next(iter(versions)) if len(versions) == 1 else "0"
    if not isinstance(version, str):
        version = "0"
    root_purl = purl(name, version, {"workspace": "true"})
    return {
        "bom-ref": root_purl,
        "name": name,
        "purl": root_purl,
        "type": "application",
        "version": version,
    }


def generate_bom(
    metadata: dict[str, Any],
    checksums: dict[tuple[str, str, str | None], str],
    workspace_name: str | None = None,
    require_registry_checksums: bool = False,
) -> dict[str, Any]:
    packages = metadata.get("packages")
    workspace_members = metadata.get("workspace_members")
    resolve = metadata.get("resolve")
    workspace_root_value = metadata.get("workspace_root")
    if not isinstance(packages, list) or not packages:
        raise SbomError("Cargo metadata must include a non-empty packages array")
    if not isinstance(workspace_members, list) or not workspace_members:
        raise SbomError("Cargo metadata must include workspace_members")
    if not isinstance(resolve, dict) or not isinstance(resolve.get("nodes"), list):
        raise SbomError("Cargo metadata must include a dependency resolve graph; omit --no-deps")
    if not isinstance(workspace_root_value, str):
        raise SbomError("Cargo metadata must include workspace_root")
    if not all(isinstance(package, dict) for package in packages):
        raise SbomError("Cargo metadata packages must be objects")

    workspace_ids = {member for member in workspace_members if isinstance(member, str)}
    package_by_id: dict[str, dict[str, Any]] = {}
    for package in packages:
        package_id = package.get("id")
        if not isinstance(package_id, str):
            raise SbomError("every Cargo package must have a string id")
        if package_id in package_by_id:
            raise SbomError(f"duplicate Cargo package id: {package_id}")
        package_by_id[package_id] = package
    missing_members = workspace_ids.difference(package_by_id)
    if missing_members:
        raise SbomError("workspace members are absent from packages")

    workspace_root = Path(workspace_root_value)
    components = [
        make_component(package, workspace_ids, workspace_root, checksums)
        for package in packages
    ]
    refs = [component["bom-ref"] for component in components]
    if len(refs) != len(set(refs)):
        duplicates = sorted({ref for ref in refs if refs.count(ref) > 1})
        raise SbomError(f"non-unique component purls: {', '.join(duplicates)}")
    ref_by_id = {
        package["id"]: component["bom-ref"]
        for package, component in zip(packages, components, strict=True)
    }

    if require_registry_checksums:
        missing = sorted(
            f"{package.get('name')} {package.get('version')}"
            for package, component in zip(packages, components, strict=True)
            if isinstance(package.get("source"), str)
            and (package["source"].startswith("registry+") or package["source"].startswith("sparse+"))
            and "hashes" not in component
        )
        if missing:
            raise SbomError(f"registry components missing lockfile checksums: {', '.join(missing)}")

    dependencies: dict[str, set[str]] = {ref: set() for ref in refs}
    for node in resolve["nodes"]:
        if not isinstance(node, dict) or not isinstance(node.get("id"), str):
            raise SbomError("Cargo resolve nodes must have string ids")
        node_id = node["id"]
        if node_id not in ref_by_id:
            raise SbomError(f"resolve node is absent from packages: {node_id}")
        raw_dependencies = node.get("dependencies", [])
        if not isinstance(raw_dependencies, list):
            raise SbomError(f"resolve dependencies must be an array for {node_id}")
        for dependency_id in raw_dependencies:
            if dependency_id not in ref_by_id:
                raise SbomError(f"resolve dependency is absent from packages: {dependency_id}")
            dependencies[ref_by_id[node_id]].add(ref_by_id[dependency_id])

    member_packages = [package_by_id[member] for member in sorted(workspace_ids)]
    root_component = workspace_component(member_packages, workspace_root, workspace_name)
    dependencies[root_component["bom-ref"]] = {
        ref_by_id[member] for member in workspace_ids
    }
    bom = {
        "$schema": SCHEMA,
        "bomFormat": "CycloneDX",
        "components": sorted(components, key=lambda component: component["bom-ref"]),
        "dependencies": [
            {"dependsOn": sorted(depends_on), "ref": ref}
            for ref, depends_on in sorted(dependencies.items())
        ],
        "metadata": {"component": root_component},
        "specVersion": "1.6",
        "version": 1,
    }
    validate_bom(bom)
    return bom


def validate_bom(bom: dict[str, Any]) -> None:
    if bom.get("bomFormat") != "CycloneDX" or bom.get("specVersion") != "1.6":
        raise SbomError("generated document is not CycloneDX 1.6")
    components = bom.get("components")
    dependencies = bom.get("dependencies")
    root = bom.get("metadata", {}).get("component")
    if not isinstance(components, list) or not isinstance(dependencies, list) or not isinstance(root, dict):
        raise SbomError("generated BOM is missing components, dependencies, or metadata.component")
    refs = {component.get("bom-ref") for component in components}
    if None in refs or len(refs) != len(components):
        raise SbomError("generated BOM component references are missing or duplicated")
    refs.add(root.get("bom-ref"))
    for dependency in dependencies:
        if dependency.get("ref") not in refs:
            raise SbomError(f"dependency graph has an unknown ref: {dependency.get('ref')}")
        unknown = set(dependency.get("dependsOn", [])).difference(refs)
        if unknown:
            raise SbomError(f"dependency graph has unknown targets: {sorted(unknown)}")
    for component in components:
        if component.get("purl") != component.get("bom-ref"):
            raise SbomError("component bom-ref and purl must match")
        for checksum in component.get("hashes", []):
            if checksum.get("alg") != "SHA-256" or not SHA256_RE.fullmatch(
                checksum.get("content", "")
            ):
                raise SbomError(f"component has an invalid checksum: {component.get('name')}")


def serialized(bom: dict[str, Any]) -> str:
    return json.dumps(bom, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def write_output(document: str, output: Path | None) -> None:
    if output is None:
        sys.stdout.write(document)
        return
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary_name: str | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            dir=output.parent,
            prefix=f".{output.name}.",
            delete=False,
        ) as temporary:
            temporary.write(document)
            temporary_name = temporary.name
        os.replace(temporary_name, output)
    finally:
        if temporary_name is not None:
            try:
                Path(temporary_name).unlink()
            except FileNotFoundError:
                pass


def run_self_test() -> None:
    repository_root = Path(__file__).resolve().parents[2]
    fixture_root = repository_root / "fixtures" / "sbom"
    metadata = load_json(fixture_root / "metadata.json")
    checksums = load_lock_checksums(fixture_root / "Cargo.lock")
    bom = generate_bom(metadata, checksums, require_registry_checksums=True)
    first = serialized(bom)

    reordered = copy.deepcopy(metadata)
    reordered["packages"].reverse()
    reordered["workspace_members"].reverse()
    reordered["resolve"]["nodes"].reverse()
    for node in reordered["resolve"]["nodes"]:
        node["dependencies"].reverse()
    second = serialized(
        generate_bom(reordered, checksums, require_registry_checksums=True)
    )
    if first != second:
        raise SbomError("fixture BOM changes when Cargo metadata arrays are reordered")

    components = {component["name"]: component for component in bom["components"]}
    serde = components.get("serde")
    if serde is None or serde.get("hashes") != [
        {
            "alg": "SHA-256",
            "content": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        }
    ]:
        raise SbomError("fixture registry checksum was not emitted")
    if serde.get("licenses") != [{"expression": "MIT OR Apache-2.0"}]:
        raise SbomError("fixture SPDX license expression was not emitted")
    if components["demo-app"].get("licenses") != [
        {"license": {"name": "MIT/Apache-2.0"}}
    ]:
        raise SbomError("fixture legacy license declaration was not preserved as a name")
    root_ref = bom["metadata"]["component"]["bom-ref"]
    graph = {entry["ref"]: entry["dependsOn"] for entry in bom["dependencies"]}
    if len(graph.get(root_ref, [])) != 2:
        raise SbomError("fixture workspace aggregate does not depend on both members")
    if "/workspace/" in first or str(repository_root) in first:
        raise SbomError("fixture BOM leaks an absolute checkout path")
    digest = hashlib.sha256(first.encode("utf-8")).hexdigest()
    print(f"CycloneDX self-test passed: {len(components)} components, sha256={digest}")


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        if args.self_test:
            run_self_test()
            return 0
        metadata = (
            load_json(args.metadata)
            if args.metadata is not None
            else cargo_metadata(args.cargo, args.workspace, args.online)
        )
        lock_path = infer_lock_path(args.cargo_lock, args.metadata, args.workspace, metadata)
        checksums = load_lock_checksums(lock_path)
        bom = generate_bom(
            metadata,
            checksums,
            workspace_name=args.workspace_name,
            require_registry_checksums=args.require_registry_checksums,
        )
        write_output(serialized(bom), args.output)
    except SbomError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
