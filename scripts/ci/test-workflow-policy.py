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

        quality_original = quality.read_text(encoding="utf-8")
        alternate_yaml = quality_original.replace(
            "    branches: [main]",
            "    branches:\n      - main",
            1,
        ).replace(
            "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
            'cancel-in-progress: ${{github.event_name=="pull_request"}}',
            1,
        )
        quality.write_text(alternate_yaml, encoding="utf-8")
        alternate_baseline = run(fixture)
        if alternate_baseline.returncode != 0:
            print(
                "workflow policy rejected equivalent block-list/spacing YAML",
                file=sys.stderr,
            )
            print(alternate_baseline.stderr, file=sys.stderr)
            return 1
        quality.write_text(
            alternate_yaml.replace("github.run_id", "github.ref", 1),
            encoding="utf-8",
        )
        alternate_unsafe = run(fixture)
        quality.write_text(quality_original, encoding="utf-8")
        if alternate_unsafe.returncode == 0:
            print(
                "workflow policy accepted ref-grouped main runs in equivalent YAML",
                file=sys.stderr,
            )
            return 1

        mutations = (
            (
                production,
                "on:\n  pull_request:\n  push:",
                "on:\n  push:",
                "main-only unprivileged release path",
            ),
            (
                production,
                "  push:\n    branches: [main]",
                "  push:\n    branches: [main]\n    paths: [apps/control-plane/**]",
                "path-filtered sentinel",
            ),
            (
                production,
                "  group: ${{ github.event_name == 'pull_request' && format('frame-production-preflight-pr-{0}', github.event.pull_request.number) || 'frame-production-release' }}",
                "  group: frame-production-release",
                "pull request preflight shares protected release concurrency",
            ),
            (
                production,
                "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
                "  cancel-in-progress: false",
                "stale same-PR production builds accumulate",
            ),
            (
                production,
                "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
                "  cancel-in-progress: true",
                "main or provider production release can be cancelled",
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
                "      - name: Resolve the event-identical build path to a binary result",
                "      - name: Resolve the event-identical build path to a binary result\n        continue-on-error: true",
                "advisory sentinel",
            ),
            (
                production,
                "if: ${{ github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main' }}",
                "if: ${{ github.event_name == 'workflow_dispatch' }}",
                "provider dispatch allowed from a non-main ref",
            ),
            (
                production,
                "if: ${{ github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main' }}",
                "if: ${{ github.event_name == 'push' }}",
                "provider release reachable from an automatic main push",
            ),
            (
                production,
                "needs: [evaluate, preflight, build_release, production-gate]",
                "needs: [evaluate, preflight, build_release]",
                "provider release detached from the successful build sentinel",
            ),
            (
                production,
                "needs: [evaluate, preflight, build_release]",
                "needs: [evaluate, preflight, build_release, provider_release]",
                "protected provider evidence coupled to the required build gate",
            ),
            (
                production,
                "if: ${{ always() && github.event_name == 'workflow_dispatch' }}",
                "if: ${{ always() }}",
                "provider release sentinel made active on pull requests and main pushes",
            ),
            (
                production,
                "needs: [production-gate, provider_release]",
                "needs: [production-gate]",
                "provider release sentinel detached from provider outcome",
            ),
            (
                production,
                "run: worker-build --release apps/control-plane",
                "run: worker-build --help",
                "production Worker readiness probe without prebuild",
            ),
            (
                production,
                "python3 scripts/ci/check-parity-evidence.py\n",
                "python3 scripts/ci/check-parity-evidence.py --require-full\n",
                "protected evidence required inside shared PR build preflight",
            ),
            (
                production,
                "python3 scripts/ci/check-parity-evidence.py --require-full",
                "python3 scripts/ci/check-parity-evidence.py",
                "provider mutation without protected parity evidence",
            ),
            (
                production,
                "      - name: Require protected release parity before provider access\n        run: python3 scripts/ci/check-parity-evidence.py --require-full",
                "      - name: Require protected release parity before provider access\n        run: |\n          npx --yes wrangler@4.111.0 d1 migrations apply frame --remote\n          python3 scripts/ci/check-parity-evidence.py --require-full",
                "protected parity evaluated after provider mutation",
            ),
            (
                share,
                "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5",
                "actions/checkout@v4",
                "mutable action pin",
            ),
            (
                quality,
                "cancel-in-progress: ${{ github.event_name == 'pull_request' }}",
                "cancel-in-progress: true",
                "cancelled landed-main build",
            ),
            (
                quality,
                "github.event.pull_request.number || github.run_id",
                "github.event.pull_request.number || github.ref",
                "ref-grouped landed-main build replacement",
            ),
            (
                quality,
                "    shell: bash\n",
                "    shell: pwsh\n",
                "Windows native-command failure masking",
            ),
            (
                quality,
                "      - name: Validate workflow policy\n        run: |",
                "      - name: Validate workflow policy\n        shell: pwsh\n        run: |",
                "step-level Windows native-command failure masking",
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
                "frame-media|frame-macos-screen-capture|frame-macos-av-capture|frame-windows-screen-capture|frame-windows-capture-ffi|wgc|gstreamer",
                "frame-unrelated|frame-macos-unrelated|not-gstreamer",
                "portable desktop native-media dependency rejection removed",
            ),
            (
                quality,
                "cargo check --locked -p frame-windows-capture-ffi -p frame-windows-screen-capture --all-targets",
                "cargo check --locked -p frame-windows-secure-spool --all-targets",
                "Windows native capture check omitted",
            ),
            (
                quality,
                "cargo check --locked -p frame-desktop-core --features windows-native,custom-protocol --all-targets",
                "cargo check --locked -p frame-desktop-core --features tauri-app,custom-protocol --all-targets",
                "Windows native desktop composition check omitted",
            ),
            (
                quality,
                "\n  macos_native_capture:\n",
                "\n  macos_native_capture_disabled:\n",
                "required macOS native-capture job removed",
            ),
            (
                quality,
                "cargo test --locked -p frame-macos-screen-capture -p frame-macos-av-capture",
                "cargo test --locked -p frame-macos-screen-capture",
                "native A/V capture crate omitted from macOS tests",
            ),
            (
                quality,
                "brew install gstreamer",
                "brew install gst-plugins-base",
                "incomplete macOS GStreamer installation",
            ),
            (
                quality,
                "python scripts/ci/build-desktop-ui.py",
                "python scripts/ci/check-desktop-ui.py",
                "native production desktop skips its WebView build",
            ),
            (
                quality,
                "scripts/ci/gstreamer-sanitized-exec cargo check --locked -p frame-desktop-core --features macos-native,custom-protocol --bin frame-desktop",
                "scripts/ci/gstreamer-sanitized-exec cargo check --locked -p frame-desktop-core --features tauri-app,custom-protocol --bin frame-desktop",
                "production desktop check omits native capture",
            ),
            (
                quality,
                "needs: [policy, native, contract_worker, media, macos_native_capture, portability, desktop_shell]",
                "needs: [policy, native, contract_worker, media, portability, desktop_shell]",
                "quality-gate omits macOS native-capture dependency",
            ),
            (
                quality,
                "MACOS_NATIVE_CAPTURE_RESULT: ${{ needs.macos_native_capture.result }}",
                "MACOS_NATIVE_CAPTURE_RESULT: success",
                "quality-gate does not consume the macOS native-capture result",
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
                quality,
                "      G_DEBUG: fatal-criticals",
                "      G_DEBUG: gc-friendly",
                "media job permits GLib/GStreamer criticals",
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

        original = quality.read_text(encoding="utf-8")
        fatal_env = "    env:\n      G_DEBUG: fatal-criticals\n"
        relocated = original.replace(fatal_env, "", 1)
        macos_start = relocated.index("\n  macos_native_capture:\n")
        macos_steps = relocated.index("    steps:\n", macos_start)
        relocated = relocated[:macos_steps] + fatal_env + relocated[macos_steps:]
        quality.write_text(relocated, encoding="utf-8")
        result = run(fixture)
        quality.write_text(original, encoding="utf-8")
        if result.returncode == 0:
            print("workflow policy accepted media fatal-critical env relocated to macOS", file=sys.stderr)
            return 1

        step_marker = "      - name: Verify the audited runtime and factory contract\n"
        step_start = original.index(step_marker)
        step_end = original.index("\n      - name:", step_start + len(step_marker))
        step = original[step_start:step_end]
        relocated = original[:step_start] + original[step_end + 1 :]
        macos_start = relocated.index("\n  macos_native_capture:\n")
        macos_steps = relocated.index("    steps:\n", macos_start) + len("    steps:\n")
        relocated = relocated[:macos_steps] + step + "\n" + relocated[macos_steps:]
        quality.write_text(relocated, encoding="utf-8")
        result = run(fixture)
        quality.write_text(original, encoding="utf-8")
        if result.returncode == 0:
            print("workflow policy accepted a required media step relocated to macOS", file=sys.stderr)
            return 1

    print(f"workflow release-authority mutation suite rejected {len(mutations) + 3} unsafe designs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
