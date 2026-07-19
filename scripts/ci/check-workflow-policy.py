#!/usr/bin/env python3
"""Enforce security-critical invariants for Frame-owned delivery workflows."""

from __future__ import annotations

import datetime as dt
import os
import re
import stat
import sys
from pathlib import Path


ROOT = Path(os.environ.get("FRAME_WORKFLOW_POLICY_ROOT", Path(__file__).resolve().parents[2])).resolve()
WORKFLOW_DIR = ROOT / ".github" / "workflows"
REQUIRED_WORKFLOWS = {
    "ci.yml",
    "cross-repository-contract.yml",
    "desktop-real-hardware.yml",
    "quality-gates.yml",
    "production-gate.yml",
    "production-smoke.yml",
    "cloudflare-account.yml",
    "contract-migrations.yml",
}
ACTION_PINS = {
    "actions/checkout": "34e114876b0b11c390a56381ad16ebd13914f8d5",
    "actions/upload-artifact": "ea165f8d65b6e75b540449e92b4886f43607fa02",
    "actions/download-artifact": "d3f86a106a0bac45b974a628896c90dbdf5c8093",
    "Swatinem/rust-cache": "e18b497796c12c097a38f9edb9d0641fb99eee32",
    "actions/setup-node": "49933ea5288caeca8642d1e84afbd3f7d6820020",
    "actions/setup-python": "a309ff8b426b58ec0e2a45f0f869d46889d02405",
}


def require(condition: bool, message: str, errors: list[str]) -> None:
    if not condition:
        errors.append(message)


def workflow_step(job: str, name: str) -> str:
    marker = f"      - name: {name}\n"
    if marker not in job:
        return ""
    return job.split(marker, maxsplit=1)[1].split("\n      - name:", maxsplit=1)[0]


def main() -> int:
    errors: list[str] = []
    texts: dict[str, str] = {}
    owned = {path.name for path in WORKFLOW_DIR.glob("*.yml")}
    for required in sorted(REQUIRED_WORKFLOWS - owned):
        errors.append(f"missing owned workflow: .github/workflows/{required}")
    gstreamer_launcher = ROOT / "scripts" / "ci" / "gstreamer-sanitized-exec"
    cargo_config = ROOT / ".cargo" / "config.toml"
    require(
        gstreamer_launcher.is_file()
        and bool(gstreamer_launcher.stat().st_mode & stat.S_IXUSR),
        "GStreamer sanitized launcher must exist and be executable",
        errors,
    )
    require(
        cargo_config.is_file()
        and "[target.'cfg(unix)']" in cargo_config.read_text(encoding="utf-8")
        and 'runner = ["scripts/ci/gstreamer-sanitized-exec"]'
        in cargo_config.read_text(encoding="utf-8"),
        "Unix Cargo run/test artifacts must use the sanitized GStreamer runner",
        errors,
    )

    for name in sorted(owned):
        path = WORKFLOW_DIR / name
        require(path.is_file(), f"missing owned workflow: {path.relative_to(ROOT)}", errors)
        if path.is_file():
            texts[name] = path.read_text(encoding="utf-8")

    for name, text in texts.items():
        path = f".github/workflows/{name}"
        lowered = text.lower()
        forbidden_triggers = ["pull_request_target:", "repository_dispatch:"]
        if name != "production-smoke.yml":
            forbidden_triggers.append("workflow_run:")
        for forbidden in forbidden_triggers:
            require(forbidden not in lowered, f"{path}: forbidden trigger {forbidden}", errors)
        require("permissions:\n  contents: read" in text, f"{path}: top-level permissions must be contents: read", errors)
        require(re.search(r"curl[^\n|]*\|\s*(?:ba)?sh\b", lowered) is None,
                f"{path}: pipe-to-shell installers are forbidden", errors)
        require("continue-on-error: true" not in lowered, f"{path}: release checks may not be made advisory", errors)

        for action, reference in re.findall(r"^\s*-?\s*uses:\s*([^@\s]+)@([^\s#]+)", text, re.MULTILINE):
            if action.startswith("./"):
                continue
            expected = ACTION_PINS.get(action)
            require(expected is not None, f"{path}: unapproved external action {action}", errors)
            if expected is not None:
                require(reference == expected, f"{path}: {action} must use immutable SHA {expected}", errors)

    quality = texts.get("quality-gates.yml", "")
    api_parity = texts.get("api-workflow-parity.yml", "")
    parity_runner = "python3 -I scripts/ci/run-legacy-api-parity.py"
    parity_runner_test = "python3 -I scripts/ci/test-legacy-api-parity-runner.py"
    require(
        (ROOT / "scripts/ci/run-legacy-api-parity.py").is_file()
        and (ROOT / "scripts/ci/test-legacy-api-parity-runner.py").is_file(),
        "legacy/API parity aggregate runner and its fail-fast tests must exist",
        errors,
    )
    require(
        api_parity.count(parity_runner) == 1
        and api_parity.count(parity_runner_test) == 1,
        "api-workflow-parity.yml: the aggregate legacy/API runner and its tests must run exactly once",
        errors,
    )
    require("pull_request:" in quality and "push:" in quality, "quality-gates.yml: must run for pull requests and main pushes", errors)
    require("${{ secrets." not in quality, "quality-gates.yml: untrusted validation must be secret-free", errors)
    require(
        "group: quality-${{ github.workflow }}-${{ github.event.pull_request.number || github.run_id }}"
        in quality
        and "cancel-in-progress: ${{ github.event_name == 'pull_request' }}"
        in quality,
        "quality-gates.yml: stale same-PR checks may cancel, but landed-main and manual runs must be unique and noncancellable",
        errors,
    )
    require(
        re.search(
            r"^defaults:\n  run:\n(?:    #.*\n)*    shell: bash$",
            quality,
            re.MULTILINE,
        )
        is not None,
        "quality-gates.yml: every hosted OS must fail immediately after a native command exits nonzero",
        errors,
    )
    quality_shells = re.findall(r"^\s+shell:\s*([^\s#]+)", quality, re.MULTILINE)
    require(
        bool(quality_shells) and all(shell == "bash" for shell in quality_shells),
        "quality-gates.yml: step-level shell overrides must preserve fail-fast bash semantics",
        errors,
    )
    require("check-parity-evidence.py" in quality,
            "quality-gates.yml: the fast parity evidence lane must be required", errors)
    require("check-secrets.py" in quality and "cargo deny check" in quality,
            "quality-gates.yml: secret and dependency policy must be required", errors)
    require("generate-cyclonedx.py" in quality and "hermetic-journey.py" in quality,
            "quality-gates.yml: SBOM and provider-free walking-slice checks must be required", errors)
    require("d1-repository-conformance.py" in quality
            and "auth-d1-conformance.py" in quality
            and "organization-d1-conformance.py" in quality
            and "api-workflow-d1-conformance.py" in quality
            and "target/evidence/d1-repository-conformance.json" in quality
            and "target/evidence/auth-d1-conformance.json" in quality
            and "target/evidence/organization-d1-conformance.json" in quality
            and "target/evidence/api-workflow-d1-conformance.json" in quality,
            "quality-gates.yml: every isolated local D1 conformance suite and its evidence must be required", errors)
    require("r2-storage-conformance.py" in quality
            and "target/evidence/r2-storage-conformance.json" in quality,
            "quality-gates.yml: credential-free local R2 adapter conformance and evidence must be required", errors)
    require("check-desktop-product.py" in quality
            and "check-media-service.py" in quality
            and "business-sqlite-semantic-conformance.py" in quality
            and "organization-sqlite-semantic-conformance.py" in quality
            and "public-collaboration-sqlite-conformance.py" in quality
            and "direct-upload-sqlite-conformance.py" in quality
            and "instant-finalize-sqlite-conformance.py" in quality
            and "media-job-inputs-sqlite-conformance.py" in quality
            and "worker-auth-sqlite-conformance.py" in quality,
            "quality-gates.yml: desktop, media-service, business, organization, public-collaboration, and Worker-auth semantic contracts must be required", errors)
    require("GST_PLUGIN_SYSTEM_PATH_1_0" in quality
            and "gstreamer-sanitized-exec" in quality
            and "hostile-gstreamer-readiness.py" in quality
            and "gstreamer-packages-ci.tsv" in quality
            and "gstreamer-runtime-hostile-xdg-ci.json" in quality,
            "quality-gates.yml: trusted-root, package, hostile-env, and hostile-XDG media gates must be required", errors)
    media_job = quality.split("\n  media:\n", maxsplit=1)[-1].split(
        "\n  portability:\n", maxsplit=1
    )[0]
    runtime_step = workflow_step(media_job, "Verify the audited runtime and factory contract")
    require(
        bool(runtime_step)
        and "libgstreamer1.0*" not in runtime_step
        and "libgstreamer-plugins-base1.0*" not in runtime_step
        and "gstreamer1.0-*" not in runtime_step
        and all(
            package in runtime_step
            for package in (
                "libgstreamer1.0-0",
                "libgstreamer1.0-dev",
                "libgstreamer-plugins-base1.0-0",
                "libgstreamer-plugins-base1.0-dev",
                "gstreamer1.0-tools",
                "gstreamer1.0-plugins-base",
                "gstreamer1.0-plugins-base-apps",
                "gstreamer1.0-plugins-good",
            )
        ),
        "quality-gates.yml: package evidence must query only the eight audited installed GStreamer packages",
        errors,
    )
    require("check-media-conformance.py" in quality
            and "media-conformance-offline.json" in quality
            and "media-conformance-dashboard.json" in quality,
            "quality-gates.yml: deterministic media conformance and dashboard evidence must be required", errors)
    require(
        "cargo test --locked -p frame-desktop-core --features instant-finalize" in quality
        and "cargo clippy --locked -p frame-desktop-core --features instant-finalize --all-targets -- -D warnings" in quality,
        "quality-gates.yml: the optional native Instant-finalize adapter must be fully tested and linted in the GStreamer lane",
        errors,
    )
    require("build-web-hydration.py" in quality
            and "check-web-hydration-bundle.py" in quality
            and "web-hydration-smoke.py" in quality
            and "render-web-runtime-smoke.py" in quality
            and "render-web-runtime-linux.json" in quality
            and "frame-web-hydrate" in quality
            and "google-chrome --version" in quality,
            "quality-gates.yml: locked hydration, browser, and Render-runtime smokes must be required", errors)
    require("cross-repo-preview-e2e.py" in quality and "--self-test --timeout 20" in quality,
            "quality-gates.yml: credential-free two-origin preview controls must be required", errors)
    require("check-launch-observability.py" in quality,
            "quality-gates.yml: launch SLO, observability, privacy, drift, and rollback contract must be required", errors)
    require("check-cross-repo-contract.py" in quality
            and "test-cross-repo-contract.py" in quality,
            "quality-gates.yml: cross-repository ownership/policy mutations must be required", errors)
    require("test-desktop-real-hardware.py" in quality,
            "quality-gates.yml: signed desktop hardware evidence regressions must be tested", errors)
    authenticated_web = texts.get("leptos-authenticated-web.yml", "")
    for dependency in (
        "apps/control-plane/src/auth_repository.rs",
        "apps/control-plane/src/compatibility_rate_limit.rs",
        "apps/control-plane/queries/auth/**",
        "apps/control-plane/queries/api_workflow/compatibility_rate_limit_*.sql",
        "apps/control-plane/migrations/0034_compatibility_rate_limits.sql",
        "crates/domain/src/identity.rs",
        "crates/ports/src/identity.rs",
    ):
        require(
            authenticated_web.count(dependency) == 2,
            f"leptos-authenticated-web.yml: pull-request and push filters must include {dependency}",
            errors,
        )
    require("macos-15" in quality and "windows-2022" in quality and "macos-14" not in quality,
            "quality-gates.yml: portable core checks must cover macOS and Windows", errors)
    require("desktop_shell:" in quality and "trunk --version 0.21.14 --locked" in quality
            and "build-desktop-ui.py" in quality
            and "--no-color=false" not in quality
            and "--features tauri-app --bin frame-desktop" in quality
            and "--features tauri-app,custom-protocol --bin frame-desktop" in quality
            and "desktop-shell-smoke.py" in quality
            and "check-desktop-bundle.py" in quality
            and "check-desktop-advisory-exception.py" in quality,
            "quality-gates.yml: the portable Tauri shell and pinned Leptos bundle must be required", errors)
    desktop_shell = quality.split("\n  desktop_shell:\n", maxsplit=1)[-1].split(
        "\n  quality-gate:\n", maxsplit=1
    )[0]
    require(
        "macos-native" not in desktop_shell
        and "FRAME_GSTREAMER_COMPILE_ONLY" not in desktop_shell
        and "SYSTEM_DEPS_GSTREAMER_1_0_NO_PKG_CONFIG" not in desktop_shell
        and "SYSTEM_DEPS_GSTREAMER_1_0_LIB" not in desktop_shell,
        "quality-gates.yml: the portable desktop shell must not enable or mask native media dependencies",
        errors,
    )
    dependency_step = workflow_step(
        desktop_shell, "Prove the portable desktop dependency closure"
    )
    require(
        bool(dependency_step)
        and "cargo tree --locked -p frame-desktop-core" in dependency_step
        and "--features tauri-app,custom-protocol --edges normal" in dependency_step
        and "frame-media|frame-macos-screen-capture|gstreamer" in dependency_step,
        "quality-gates.yml: the portable desktop dependency graph must reject native media crates",
        errors,
    )
    for step_name in (
        "Lint and test the portable Tauri command boundary",
        "Build the portable desktop executable",
    ):
        step = workflow_step(desktop_shell, step_name)
        require(bool(step), f"quality-gates.yml: missing {step_name}", errors)
        require(
            re.search(r"^\s*DOCS_RS\s*:", step, re.MULTILINE) is None,
            f"quality-gates.yml: {step_name} must not disable unrelated native linking with DOCS_RS",
            errors,
        )
    portable_smoke = workflow_step(
        desktop_shell, "Exercise the portable production-CSP WebView command boundary"
    )
    require(
        bool(portable_smoke) and "--expected-adapter unavailable" in portable_smoke,
        "quality-gates.yml: the portable shell smoke must require the unavailable recorder adapter",
        errors,
    )

    hardware = texts.get("desktop-real-hardware.yml", "")
    require(
        "macos_display:" in hardware
        and "runs-on: frame-macos-hardware" in hardware
        and "environment: desktop-macos-hardware" in hardware
        and re.search(
            r"^concurrency:\n  group: desktop-macos-hardware\n"
            r"  cancel-in-progress: false$",
            hardware,
            re.MULTILINE,
        ) is not None
        and "fetch-depth: 0" in hardware
        and 'git merge-base --is-ancestor "$RELEASE_SHA" refs/remotes/origin/main' in hardware
        and "secrets.FRAME_CODESIGN_IDENTITY" in hardware
        and "scripts/frame desktop-macos-bundle" in hardware
        and "sign-macos-local-app.sh verify-trusted" in hardware
        and "--app-bundle target/release/bundle/macos/Frame.app" in hardware
        and "vars.FRAME_EXPECTED_APPLE_TEAM_ID" in hardware
        and "--expected-source-sha" in hardware
        and "--expected-run-id" in hardware
        and "--capability macos_display_webm_v1" in hardware
        and "--expected-capability macos_display_webm_v1" in hardware
        and "desktop-macos-display-hardware-v1" in hardware,
        "desktop-real-hardware.yml: the protected signed-app macOS display gate is required",
        errors,
    )
    require(
        "frame-windows-hardware" not in hardware
        and "frame-desktop.exe" not in hardware
        and "--binary target/release/frame-desktop" not in hardware
        and 'FRAME_CODESIGN_IDENTITY: "-"' not in hardware,
        "desktop-real-hardware.yml: raw, ad-hoc, or unavailable capture builds must not be accepted as native evidence",
        errors,
    )
    exception_expiry = dt.date(2026, 10, 15)
    require(dt.datetime.now(dt.timezone.utc).date() < exception_expiry,
            "FRAME-DEP-2026-01 has expired; remove or re-review the advisory exception", errors)
    require(re.search(r"^  quality-gate:\n(?:.|\n)*?if:\s*\$\{\{\s*always\(\)\s*\}\}", quality, re.MULTILINE) is not None,
            "quality-gates.yml: quality-gate must be an always-present final result", errors)

    production = texts.get("production-gate.yml", "")
    require(
        "pull_request:" in production
        and "push:\n    branches: [main]" in production
        and "workflow_dispatch:" in production,
        "production-gate.yml: the same unprivileged build path must run on pull requests and every main push, with release remaining explicit",
        errors,
    )
    require("paths:" not in production and "paths-ignore:" not in production, "production-gate.yml: sentinel may not have path filters", errors)
    require(
        "group: ${{ github.event_name == 'pull_request' && format('frame-production-preflight-pr-{0}', github.event.pull_request.number) || 'frame-production-release' }}"
        in production
        and "cancel-in-progress: ${{ github.event_name == 'pull_request' }}"
        in production,
        "production-gate.yml: stale same-PR builds must cancel without making main or provider releases cancellable",
        errors,
    )
    require("environment: production" in production, "production-gate.yml: provider mutation must use the production environment", errors)
    require("secrets.CLOUDFLARE_API_TOKEN" in production and "secrets.CLOUDFLARE_ACCOUNT_ID" in production,
            "production-gate.yml: provider credentials must be explicit environment secrets", errors)
    require("secrets.CLOUDFLARE_D1_DATABASE_ID" in production and "prepare-wrangler-config.py" in production,
            "production-gate.yml: the production D1 ID must be injected only inside the protected job", errors)
    require("vars.FRAME_EXPECTED_D1_DATABASE_ID" in production
            and "vars.FRAME_EXPECTED_D1_DATABASE_NAME" in production,
            "production-gate.yml: provider-returned D1 identity must match independent protected expectations", errors)
    require("vars.FRAME_APPROVED_ROLLBACK_SOURCE_SHA" in production
            and "vars.FRAME_APPROVED_ROLLBACK_WORKER_SHA256" in production
            and "vars.FRAME_APPROVED_ROLLBACK_DEPLOYMENT_ID" in production
            and "vars.FRAME_APPROVED_ROLLBACK_VERSION_ID" in production
            and "vars.FRAME_APPROVED_ROLLBACK_EXPAND_LEVEL" in production
            and "vars.FRAME_APPROVED_ROLLBACK_CONTRACT_LEVEL" in production
            and "vars.FRAME_APPROVED_ROLLBACK_IDENTITY_MODE" in production
            and "vars.FRAME_APPROVED_ROLLBACK_PROVIDER_ETAG" in production
            and "vars.FRAME_APPROVED_ROLLBACK_BOOTSTRAP_CONFIRMATION" in production,
            "production-gate.yml: an exact protected Worker rollback predecessor must be approved", errors)
    require("actions/download-artifact@" in production and "verify-release-bundle.sh" in production,
            "production-gate.yml: provider job must consume and verify the built artifact", errors)
    production_untrusted = production.split("\n  provider_release:\n", maxsplit=1)[0]
    production_after_provider = production.split("\n  provider_release:\n", maxsplit=1)[-1]
    production_provider = production_after_provider.split(
        "\n  production-gate:\n", maxsplit=1
    )[0]
    production_build_gate = production.split(
        "\n  production-gate:\n", maxsplit=1
    )[-1].split("\n  provider-release-gate:\n", maxsplit=1)[0]
    provider_release_gate = production.split(
        "\n  provider-release-gate:\n", maxsplit=1
    )[-1]
    require(
        "if: ${{ github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main' }}"
        in production_provider
        and "needs: [evaluate, preflight, build_release, production-gate]"
        in production_provider,
        "production-gate.yml: protected provider access must require an explicit main-branch dispatch",
        errors,
    )
    require('--outdir "${GITHUB_WORKSPACE}/target/wrangler-release"' in production_untrusted,
            "production-gate.yml: Wrangler output must use the workspace-absolute release directory", errors)
    require('--outdir "${GITHUB_WORKSPACE}/target/wrangler-ci"' in quality,
            "quality-gates.yml: Wrangler output must use the workspace-absolute CI directory", errors)
    require("${{ secrets." not in production_untrusted,
            "production-gate.yml: secrets may appear only in the protected provider job", errors)
    require("${{ secrets." not in production_build_gate
            and "${{ secrets." not in provider_release_gate,
            "production-gate.yml: build and release sentinels must remain secret-free", errors)
    require("target/provider-worker/wrangler-release/shim.js --no-bundle" in production_provider,
            "production-gate.yml: protected deploy must upload the verified Worker artifact", errors)
    require("cargo install worker-build" not in production_provider,
            "production-gate.yml: protected provider job must not rebuild the Worker", errors)
    require("--tag \"${GITHUB_SHA}\"" in production_provider
            and "frame-source:${GITHUB_SHA};frame-worker-sha256:" in production_provider
            and "frame-expand:${expand_level};frame-contract:${contract_level}" in production_provider,
            "production-gate.yml: provider version must bind source, bundle digest, expand, and contract identity", errors)
    require("contract-migration-authority.py" in production_provider
            and "--phase release-pre" in production_provider
            and "--phase release" in production_provider
            and "contract-migration-authority.sql" in production_provider
            and production_provider.count("pre-d1-authority.json") >= 2
            and "target/provider-release-manifest.json" in production_provider
            and "protected_provider_approved" in production_provider,
            "production-gate.yml: provider release record must verify immutable Worker and D1 authority", errors)
    require("workers/scripts/frame-control-plane/deployments" in production_provider
            and "workers/workers/frame-control-plane/versions/${active_version_id}" in production_provider
            and "workers/workers/frame-control-plane/versions/${FRAME_APPROVED_ROLLBACK_VERSION_ID}" in production_provider,
            "production-gate.yml: active and rollback provider versions must be observed after deployment", errors)
    require("protected-unannotated-bootstrap" in production_provider
            and "adopt-current-unannotated-worker-once" in production_provider
            and production_provider.count("workers/scripts/frame-control-plane/versions/${FRAME_APPROVED_ROLLBACK_VERSION_ID}") >= 2
            and "rollback-script-version.json" in production_provider,
            "production-gate.yml: one-time unannotated rollback adoption must bind an explicit protected provider version etag", errors)
    release_pre_marker = "--phase release-pre"
    expand_apply_marker = "d1 migrations apply frame --remote"
    deploy_marker = "npx --yes wrangler@4.111.0 deploy \\\n"
    protected_parity_marker = "check-parity-evidence.py --require-full"
    if all(
        marker in production_provider
        for marker in (
            protected_parity_marker,
            release_pre_marker,
            expand_apply_marker,
            deploy_marker,
        )
    ):
        require(
            production_provider.index(protected_parity_marker)
            < production_provider.index(release_pre_marker)
            < production_provider.index(expand_apply_marker)
            < production_provider.index(deploy_marker),
            "production-gate.yml: protected parity plus read-only D1 and rollback authority must pass before any provider mutation",
            errors,
        )
    require(re.search(r"^  production-gate:\n(?:.|\n)*?if:\s*\$\{\{\s*always\(\)\s*\}\}", production, re.MULTILINE) is not None,
            "production-gate.yml: production-gate must always resolve to success or failure", errors)
    require(
        "needs: [evaluate, preflight, build_release]" in production_build_gate
        and "provider_release" not in production_build_gate
        and "PROVIDER_RESULT" not in production_build_gate
        and 'test "${EVALUATE_RESULT}" = success' in production_build_gate
        and 'test "${PREFLIGHT_RESULT}" = success' in production_build_gate
        and 'test "${BUILD_RESULT}" = success' in production_build_gate,
        "production-gate.yml: the required production-gate must resolve only the identical PR/main build path",
        errors,
    )
    require(
        "name: provider-release-gate" in provider_release_gate
        and "if: ${{ always() && github.event_name == 'workflow_dispatch' }}"
        in provider_release_gate
        and "needs: [production-gate, provider_release]" in provider_release_gate
        and 'test "${GITHUB_REF}" = refs/heads/main' in provider_release_gate
        and 'test "${BUILD_GATE_RESULT}" = success' in provider_release_gate
        and 'test "${PROVIDER_RESULT}" = success' in provider_release_gate,
        "production-gate.yml: explicit manual provider release must have an independent fail-closed sentinel",
        errors,
    )
    require("workflow_run:" not in production, "production-gate.yml: delayed workflow_run sentinels are forbidden", errors)
    require(
        "python3 scripts/ci/check-parity-evidence.py\n" in production_untrusted
        and "check-parity-evidence.py --require-full" not in production_untrusted
        and "check-parity-evidence.py --require-full" in production_provider,
        "production-gate.yml: PR/main preflight must validate local parity while protected parity blocks only provider access",
        errors,
    )
    require("check-secrets.py" in production_untrusted and "cargo deny check" in production_untrusted,
            "production-gate.yml: release preflight must enforce secret and dependency policy", errors)
    require("generate-cyclonedx.py" in production_untrusted and "hermetic-journey.py" in production_untrusted,
            "production-gate.yml: release preflight must validate SBOM and hermetic journey", errors)
    require("GST_PLUGIN_SYSTEM_PATH_1_0" in production_untrusted
            and "gstreamer-sanitized-exec" in production_untrusted,
            "production-gate.yml: native preflight must pin the GStreamer root and sanitize loader overrides", errors)
    require("check-media-conformance.py" in production_untrusted,
            "production-gate.yml: local media conformance definition must be validated before protected parity", errors)
    require("check-launch-observability.py" in production_untrusted,
            "production-gate.yml: launch SLO, observability, privacy, drift, and rollback contract must be required", errors)
    require("check-cross-repo-contract.py" in production_untrusted,
            "production-gate.yml: cross-repository contract policy must block release", errors)
    require("test-release-change-plan.py" in production_untrusted
            and "test-release-change-plan.py" in quality,
            "release-change-plan compile-time fixture behavior must be tested before release", errors)
    require(
        production_untrusted.count(parity_runner) == 1
        and production_untrusted.count(parity_runner_test) == 1
        and quality.count(parity_runner_test) == 1,
        "aggregate legacy/API parity must block production and its runner tests must block PR and release lanes",
        errors,
    )
    require("media-job-inputs-sqlite-conformance.py" in production_untrusted
            and "r2-completion-reconciliation-sqlite-conformance.py" in production_untrusted
            and "r2-storage-conformance.py" in production_untrusted
            and "r2-storage-conformance-production-preflight.json" in production_untrusted,
            "production-gate.yml: media-input and compiled R2 contract proofs must block provider mutation", errors)
    for workflow_name, workflow_text in (
        ("quality-gates.yml", quality),
        ("production-gate.yml", production_untrusted),
    ):
        worker_prebuild = workflow_step(
            workflow_text, "Prebuild the local Worker before bounded readiness probes"
        )
        require(
            bool(worker_prebuild)
            and "timeout-minutes: 10" in worker_prebuild
            and "worker-build --release apps/control-plane" in worker_prebuild
            and workflow_text.index("worker-build --release apps/control-plane")
            < workflow_text.index("r2-storage-conformance.py"),
            f"{workflow_name}: the Worker must be built before bounded local binding readiness probes",
            errors,
        )
    require("release-join-conformance.py --self-test" in production_untrusted,
            "production-gate.yml: release-join semantics must block provider mutation", errors)
    require("package-release.sh" in production and "verify-release-bundle.sh" in production,
            "production-gate.yml: release must create and verify the immutable SBOM-bearing handoff", errors)
    package_path = ROOT / "scripts" / "ci" / "package-release.sh"
    package = package_path.read_text(encoding="utf-8") if package_path.is_file() else ""
    verify_path = ROOT / "scripts" / "ci" / "verify-release-bundle.sh"
    verify = verify_path.read_text(encoding="utf-8") if verify_path.is_file() else ""
    for marker in (
        "expand_migration_level",
        "contract_migration_level",
        "minimum_compatible_worker",
        "approved_rollback",
        "pending_protected_provider_observation",
    ):
        require(marker in package and marker in verify,
                f"release bundle authority omits or does not verify {marker}", errors)
    require('[[ -f "${worker_bundle}/shim.js" ]]' in package
            and 'if "wrangler-release/shim.js" not in names:' in verify,
            "release bundle must preserve Wrangler's emitted shim.js module entrypoint", errors)
    require("build-web-hydration.py" in production
            and "check-web-hydration-bundle.py" in production
            and "web-hydration-smoke.py" in production
            and "target/frame-release/frame-web" in production,
            "production-gate.yml: the executable-adjacent hydration package must be built and smoked", errors)

    contract_workflow = texts.get("contract-migrations.yml", "")
    require("workflow_dispatch:" in contract_workflow
            and "pull_request:" not in contract_workflow
            and "push:" not in contract_workflow
            and "schedule:" not in contract_workflow,
            "contract-migrations.yml: contract authority must be manual-only", errors)
    require("environment: production" in contract_workflow
            and "group: frame-production-release" in contract_workflow
            and "cancel-in-progress: false" in contract_workflow,
            "contract-migrations.yml: contract mutation must share protected serialized production authority", errors)
    require("secrets.CLOUDFLARE_API_TOKEN" in contract_workflow
            and "secrets.CLOUDFLARE_ACCOUNT_ID" in contract_workflow
            and "secrets.CLOUDFLARE_D1_DATABASE_ID" in contract_workflow
            and "vars.FRAME_EXPECTED_D1_DATABASE_ID" in contract_workflow
            and "vars.FRAME_EXPECTED_D1_DATABASE_NAME" in contract_workflow,
            "contract-migrations.yml: protected actual and independent expected D1 identities are required", errors)
    require("apply-protected-contract-migrations" in contract_workflow
            and "contract-migration-authority.py --self-test" in contract_workflow
            and contract_workflow.count("--phase pre") >= 2
            and "--phase post" in contract_workflow
            and "contract-migration-authority.sql" in contract_workflow,
            "contract-migrations.yml: exact source, drain, and postcondition authority is incomplete", errors)
    require("--contract-migrations" in contract_workflow
            and "wrangler.contract.production.ci.toml" in contract_workflow
            and "d1 migrations apply frame --remote" in contract_workflow,
            "contract-migrations.yml: only the protected contract directory may be promoted", errors)
    require("workers/workers/frame-control-plane/versions/${ACTIVE_VERSION_ID}" in contract_workflow
            and "workers/workers/frame-control-plane/versions/${ROLLBACK_VERSION_ID}" in contract_workflow
            and "workers/scripts/frame-control-plane/deployments" in contract_workflow,
            "contract-migrations.yml: exact active and rollback provider identities must be observed", errors)
    require(contract_workflow.count("workers/scripts/frame-control-plane/deployments") >= 3
            and contract_workflow.count("workers/workers/frame-control-plane/versions/${ACTIVE_VERSION_ID}") >= 3
            and contract_workflow.count("workers/workers/frame-control-plane/versions/${ROLLBACK_VERSION_ID}") >= 3
            and "--deployments target/contract-authority/pre-apply-deployments.json" in contract_workflow
            and "--deployments target/contract-authority/post-deployments.json" in contract_workflow
            and "--active-version target/contract-authority/post-active-version.json" in contract_workflow
            and "--rollback-version target/contract-authority/post-rollback-version.json" in contract_workflow,
            "contract-migrations.yml: fresh pre-apply and post-contract Worker fences must be verified", errors)
    pre_apply_marker = "- name: Re-observe the complete authority fence immediately before mutation"
    contract_apply_marker = "- name: Apply only the protected contract migration directory"
    post_observe_marker = "- name: Re-observe the complete authority fence after contract mutation"
    post_verify_marker = "- name: Prove durable contract ledger and enforcement"
    if all(
        marker in contract_workflow
        for marker in (
            pre_apply_marker,
            contract_apply_marker,
            post_observe_marker,
            post_verify_marker,
        )
    ):
        require(
            contract_workflow.index(pre_apply_marker)
            < contract_workflow.index(contract_apply_marker)
            < contract_workflow.index(post_observe_marker)
            < contract_workflow.index(post_verify_marker),
            "contract-migrations.yml: Worker authority observations do not fence the contract mutation",
            errors,
        )
    require("npx --yes wrangler@4.111.0 deploy" not in contract_workflow,
            "contract-migrations.yml: contract authority may not deploy Worker code", errors)

    smoke = texts.get("production-smoke.yml", "")
    require("schedule:" in smoke and "workflow_dispatch:" in smoke and "workflow_run:" in smoke,
            "production-smoke.yml: requires scheduled, manual, and post-gate entrypoints", errors)
    require("${{ secrets." not in smoke, "production-smoke.yml: canonical smoke must be secret-free", errors)
    require("https://frame.engmanager.xyz" in smoke and "https://frame-staging.engmanager.xyz" in smoke,
            "production-smoke.yml: smoke origins must be a fixed allowlist", errors)

    ci = texts.get("ci.yml", "")
    require("GST_PLUGIN_SYSTEM_PATH_1_0" in ci and "gstreamer-sanitized-exec" in ci,
            "ci.yml: workspace tests and media smoke must pin the GStreamer root and sanitize loader overrides", errors)
    require("github.event.workflow_run.conclusion == 'success'" in smoke and "expected_release_sha" in smoke,
            "production-smoke.yml: paired release verification must follow only a successful production gate", errors)
    require("apps/web crates 'fixtures/web-authenticated/**' \\" in smoke
            and "scripts/ci Cargo.toml Cargo.lock rust-toolchain.toml render.yaml" in smoke,
            "production-smoke.yml: paired release paths must match Render's build filter", errors)

    change_plan_path = ROOT / "scripts" / "ci" / "release-change-plan.sh"
    change_plan = change_plan_path.read_text(encoding="utf-8") if change_plan_path.is_file() else ""
    require("fixtures/api-parity/*" in change_plan,
            "release-change-plan.sh: API parity fixtures must trigger a Worker release", errors)
    require("crates/authenticated-client/*" in change_plan,
            "release-change-plan.sh: authenticated-client changes must trigger a Worker release", errors)
    require("fixtures/web-authenticated/**" in change_plan,
            "release-change-plan.sh: authenticated web fixtures must trigger a web release", errors)
    require("apps/web/* | crates/* | fixtures/web-authenticated/** | scripts/ci/* | Cargo.toml | Cargo.lock | rust-toolchain.toml | render.yaml" in change_plan,
            "release-change-plan.sh: web impact must match Render's committed build filter", errors)

    terraform = texts.get("cloudflare-account.yml", "")
    require("pull_request:" in terraform and "workflow_dispatch:" in terraform,
            "cloudflare-account.yml: must expose untrusted validation and protected manual operation", errors)
    require("github.event_name == 'workflow_dispatch'" in terraform,
            "cloudflare-account.yml: credentialed job must be unreachable from pull_request", errors)
    terraform_untrusted = terraform.split("\n  terraform:\n", maxsplit=1)[0]
    require("${{ secrets." not in terraform_untrusted,
            "cloudflare-account.yml: pull-request validation must not reference secrets", errors)
    require("environment:\n      name: cloudflare-${{ inputs.environment }}" in terraform,
            "cloudflare-account.yml: credentials must come from the selected protected environment", errors)
    require("TF_BACKEND_CONFIG" in terraform and re.search(r"terraform(?:\s+-[^\s]+)*\s+apply", terraform) is not None,
            "cloudflare-account.yml: protected remote-state plan/apply path is incomplete", errors)
    require(terraform.count("-lockfile=readonly") >= 2,
            "cloudflare-account.yml: validation and protected state must honor the committed lockfile", errors)
    require("infra/cloudflare-zone" not in terraform,
            "cloudflare-account.yml: Frame account state must not own shared zone resources", errors)

    for workflow in WORKFLOW_DIR.glob("*.yml"):
        if workflow.name == "production-gate.yml":
            continue
        workflow_text = workflow.read_text(encoding="utf-8")
        deploy_blocks = re.findall(r"npx[^\n]*wrangler@4\.111\.0 deploy(?:[^\n]*\n){0,4}", workflow_text)
        for block in deploy_blocks:
            require("--dry-run" in block, f"{workflow.relative_to(ROOT)}: only production-gate may deploy a Worker", errors)

    if errors:
        print("Workflow policy violations:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(f"Workflow policy checks passed for all {len(owned)} Frame-owned workflows.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
