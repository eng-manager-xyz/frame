#!/usr/bin/env python3
"""Mutation tests for the release-authority policy checker."""

from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import sys
import tempfile


ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "ci" / "check-workflow-policy.py"


def run(root: pathlib.Path) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["FRAME_WORKFLOW_POLICY_ROOT"] = str(root)
    return subprocess.run(
        [sys.executable, "-I", str(CHECKER)],
        cwd=ROOT,
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=20,
    )


def replace(path: pathlib.Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    if old not in text:
        raise AssertionError(f"mutation seed missing from {path.name}: {old!r}")
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="frame-workflow-policy-") as temporary:
        fixture = pathlib.Path(temporary)
        shutil.copytree(ROOT / ".github", fixture / ".github")
        (fixture / ".cargo").mkdir(parents=True)
        shutil.copy2(ROOT / ".cargo/config.toml", fixture / ".cargo/config.toml")
        (fixture / "scripts/ci").mkdir(parents=True)
        shutil.copy2(
            ROOT / "scripts/ci/gstreamer-sanitized-exec",
            fixture / "scripts/ci/gstreamer-sanitized-exec",
        )
        shutil.copy2(
            ROOT / "scripts/ci/release-change-plan.sh",
            fixture / "scripts/ci/release-change-plan.sh",
        )
        shutil.copy2(
            ROOT / "scripts/ci/package-release.sh",
            fixture / "scripts/ci/package-release.sh",
        )
        shutil.copy2(
            ROOT / "scripts/ci/verify-release-bundle.sh",
            fixture / "scripts/ci/verify-release-bundle.sh",
        )
        shutil.copy2(
            ROOT / "scripts/ci/run-legacy-api-parity.py",
            fixture / "scripts/ci/run-legacy-api-parity.py",
        )
        shutil.copy2(
            ROOT / "scripts/ci/test-legacy-api-parity-runner.py",
            fixture / "scripts/ci/test-legacy-api-parity-runner.py",
        )

        baseline = run(fixture)
        if baseline.returncode != 0:
            print(baseline.stderr, file=sys.stderr)
            return 1

        production = fixture / ".github/workflows/production-gate.yml"
        smoke = fixture / ".github/workflows/production-smoke.yml"
        share = fixture / ".github/workflows/share-player.yml"
        authenticated_web = fixture / ".github/workflows/leptos-authenticated-web.yml"
        api_parity = fixture / ".github/workflows/api-workflow-parity.yml"
        quality = fixture / ".github/workflows/quality-gates.yml"
        desktop_hardware = fixture / ".github/workflows/desktop-real-hardware.yml"
        change_plan = fixture / "scripts/ci/release-change-plan.sh"
        contract = fixture / ".github/workflows/contract-migrations.yml"
        package = fixture / "scripts/ci/package-release.sh"
        verify = fixture / "scripts/ci/verify-release-bundle.sh"
        mutations = (
            (
                production,
                "  push:\n    branches: [main]",
                "  push:\n    branches: [main]\n    paths: [apps/control-plane/**]",
                "path-filtered sentinel",
            ),
            (
                production,
                "  production-gate:\n    name: production-gate\n    if: ${{ always() }}",
                "  production-gate:\n    name: production-gate\n    if: ${{ success() }}",
                "skippable sentinel",
            ),
            (
                production,
                "  workflow_dispatch:",
                "  workflow_run:\n    workflows: [Quality gates]\n    types: [completed]\n  workflow_dispatch:",
                "delayed sentinel",
            ),
            (
                production,
                "      - name: Resolve every release phase to a binary result",
                "      - name: Resolve every release phase to a binary result\n        continue-on-error: true",
                "advisory sentinel",
            ),
            (
                share,
                "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5",
                "actions/checkout@v4",
                "mutable action pin",
            ),
            (
                quality,
                "auth-d1-conformance.py",
                "auth-d1-advisory.py",
                "missing auth D1 conformance",
            ),
            (
                quality,
                "r2-storage-conformance.py",
                "r2-storage-advisory.py",
                "missing local R2 conformance",
            ),
            (
                production,
                "media-job-inputs-sqlite-conformance.py",
                "media-job-inputs-sqlite-advisory.py",
                "missing production media-input conformance",
            ),
            (
                production,
                "r2-completion-reconciliation-sqlite-conformance.py",
                "r2-completion-reconciliation-sqlite-advisory.py",
                "missing production R2 reconciliation conformance",
            ),
            (
                production,
                "r2-storage-conformance.py",
                "r2-storage-advisory.py",
                "missing production compiled R2 conformance",
            ),
            (
                quality,
                "worker-auth-sqlite-conformance.py",
                "worker-auth-sqlite-advisory.py",
                "missing Worker-auth SQLite conformance",
            ),
            (
                authenticated_web,
                '      - "apps/control-plane/src/compatibility_rate_limit.rs"',
                '      - "apps/control-plane/src/compatibility_rate_limit-disabled.rs"',
                "missing authenticated-web limiter dependency",
            ),
            (
                smoke,
                "apps/web crates 'fixtures/web-authenticated/**' \\",
                "apps/web crates fixtures/web-authenticated/v1/route-matrix.json \\",
                "narrowed authenticated web smoke release path",
            ),
            (
                quality,
                "--features tauri-app,custom-protocol --edges normal",
                "--features macos-native,custom-protocol --edges normal",
                "native media injected into the portable desktop graph",
            ),
            (
                quality,
                "frame-media|frame-macos-screen-capture|gstreamer",
                "frame-unrelated|frame-macos-unrelated|not-gstreamer",
                "portable desktop native-media dependency rejection removed",
            ),
            (
                quality,
                "--expected-adapter unavailable",
                "--expected-adapter native_macos_display",
                "portable desktop falsely advertises a native adapter",
            ),
            (
                desktop_hardware,
                "scripts/frame desktop-macos-bundle",
                "cargo build --release --bin frame-desktop",
                "macOS hardware lane bypassed signed application bundling",
            ),
            (
                desktop_hardware,
                "runs-on: frame-macos-hardware",
                "runs-on: frame-windows-hardware",
                "unavailable Windows capture admitted as native hardware evidence",
            ),
            (
                desktop_hardware,
                "--app-bundle target/release/bundle/macos/Frame.app",
                "--binary target/release/frame-desktop",
                "raw macOS executable admitted as TCC hardware evidence",
            ),
            (
                desktop_hardware,
                "secrets.FRAME_CODESIGN_IDENTITY",
                "vars.FRAME_CODESIGN_IDENTITY",
                "certificate signing identity no longer sourced from the protected secret",
            ),
            (
                desktop_hardware,
                "sign-macos-local-app.sh verify-trusted",
                "sign-macos-local-app.sh verify",
                "ad-hoc macOS signature admitted as protected evidence",
            ),
            (
                desktop_hardware,
                "--expected-source-sha",
                "--untrusted-source-sha",
                "hardware evidence detached from the checked-out source SHA",
            ),
            (
                desktop_hardware,
                "--expected-run-id",
                "--untrusted-run-id",
                "hardware evidence detached from the protected workflow run",
            ),
            (
                desktop_hardware,
                "group: desktop-macos-hardware",
                "group: desktop-macos-hardware-${{ inputs.release_sha }}",
                "macOS signing/TCC hardware jobs no longer serialized",
            ),
            (
                desktop_hardware,
                "cancel-in-progress: false",
                "cancel-in-progress: true",
                "active macOS signing/TCC evidence can be cancelled",
            ),
            (
                quality,
                "test-desktop-real-hardware.py",
                "disabled-desktop-real-hardware.py",
                "signed desktop hardware validator regressions no longer tested",
            ),
            (
                quality,
                "cargo test --locked -p frame-desktop-core --features instant-finalize",
                "cargo test --locked -p frame-desktop-core",
                "missing explicit Instant-finalize adapter coverage",
            ),
            (
                quality,
                "libgstreamer1.0-0 libgstreamer1.0-dev \\",
                "'libgstreamer1.0*' \\",
                "wildcard GStreamer package evidence",
            ),
            (
                change_plan,
                "fixtures/api-parity/*",
                "fixtures/api-parity-disabled/*",
                "missing API parity Worker impact",
            ),
            (
                change_plan,
                "crates/authenticated-client/*",
                "crates/authenticated-client-disabled/*",
                "missing authenticated-client Worker impact",
            ),
            (
                change_plan,
                "fixtures/web-authenticated/**",
                "fixtures/web-authenticated-disabled/*",
                "missing authenticated web fixture impact",
            ),
            (
                contract,
                "  workflow_dispatch:",
                "  pull_request:\n  workflow_dispatch:",
                "untrusted contract migration trigger",
            ),
            (
                contract,
                "            --phase pre \\",
                "            --phase release \\",
                "missing pre-contract authority phase",
            ),
            (
                contract,
                "            --contract-migrations",
                "            --expand-migrations",
                "contract path redirected away from protected migrations",
            ),
            (
                production,
                '--tag "${GITHUB_SHA}"',
                '--tag "mutable-latest"',
                "mutable Worker source tag",
            ),
            (
                production,
                '--outdir "${GITHUB_WORKSPACE}/target/wrangler-release"',
                "--outdir target/wrangler-release",
                "config-relative production Worker output",
            ),
            (
                quality,
                '--outdir "${GITHUB_WORKSPACE}/target/wrangler-ci"',
                "--outdir target/wrangler-ci",
                "config-relative quality Worker output",
            ),
            (
                production,
                "target/provider-worker/wrangler-release/shim.js --no-bundle",
                "target/provider-worker/wrangler-release/index.js --no-bundle",
                "nonexistent Worker bundle entrypoint",
            ),
            (
                production,
                "vars.FRAME_EXPECTED_D1_DATABASE_ID",
                "vars.FRAME_UNCHECKED_D1_DATABASE_ID",
                "missing independent D1 identity",
            ),
            (
                production,
                "vars.FRAME_APPROVED_ROLLBACK_DEPLOYMENT_ID",
                "vars.FRAME_UNAPPROVED_ROLLBACK_DEPLOYMENT_ID",
                "missing approved rollback deployment",
            ),
            (
                production,
                "vars.FRAME_APPROVED_ROLLBACK_IDENTITY_MODE",
                "vars.FRAME_UNCHECKED_ROLLBACK_IDENTITY_MODE",
                "missing protected rollback identity mode",
            ),
            (
                production,
                "            --phase release-pre \\",
                "            --phase release-advisory \\",
                "missing pre-mutation provider authority proof",
            ),
            (
                production,
                "target/provider-authority/pre-d1-authority.json",
                "target/provider-authority/pre-d1-unverified.json",
                "missing pre-mutation D1 contract-state authority",
            ),
            (
                production,
                "workers/scripts/frame-control-plane/versions/${FRAME_APPROVED_ROLLBACK_VERSION_ID}",
                "workers/scripts/frame-control-plane/versions/unbound-bootstrap-version",
                "bootstrap provider etag not bound to the approved rollback version",
            ),
            (
                api_parity,
                "python3 -I scripts/ci/run-legacy-api-parity.py",
                "python3 -I scripts/ci/run-legacy-api-parity-advisory.py",
                "missing PR aggregate legacy/API parity",
            ),
            (
                production,
                "python3 -I scripts/ci/run-legacy-api-parity.py",
                "python3 -I scripts/ci/run-legacy-api-parity-advisory.py",
                "missing production aggregate legacy/API parity",
            ),
            (
                production,
                "workers/workers/frame-control-plane/versions/${active_version_id}",
                "workers/scripts/frame-control-plane/versions/${active_version_id}",
                "annotation-free active Worker version endpoint",
            ),
            (
                contract,
                "workers/workers/frame-control-plane/versions/${ROLLBACK_VERSION_ID}",
                "workers/scripts/frame-control-plane/versions/${ROLLBACK_VERSION_ID}",
                "annotation-free rollback Worker version endpoint",
            ),
            (
                contract,
                "--deployments target/contract-authority/pre-apply-deployments.json",
                "--deployments target/contract-authority/deployments.json",
                "stale pre-apply Worker deployment fence",
            ),
            (
                contract,
                "--deployments target/contract-authority/post-deployments.json",
                "--deployments target/contract-authority/deployments.json",
                "stale post-contract Worker deployment fence",
            ),
            (
                package,
                "pending_protected_provider_observation",
                "provider_observation_not_required",
                "release manifest without protected provider observation",
            ),
            (
                package,
                '${worker_bundle}/shim.js',
                '${worker_bundle}/index.js',
                "packaged nonexistent Worker entrypoint",
            ),
            (
                verify,
                'if "wrangler-release/shim.js" not in names:',
                'if "wrangler-release/index.js" not in names:',
                "unverified Worker module entrypoint",
            ),
        )
        for path, old, new, label in mutations:
            original = path.read_text(encoding="utf-8")
            replace(path, old, new)
            result = run(fixture)
            path.write_text(original, encoding="utf-8")
            if result.returncode == 0:
                print(f"workflow policy accepted {label}", file=sys.stderr)
                return 1

    print(f"workflow release-authority mutation suite rejected {len(mutations)} unsafe designs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
