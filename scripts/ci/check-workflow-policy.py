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
    "quality-gates.yml",
    "production-gate.yml",
    "production-smoke.yml",
    "cloudflare-account.yml",
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
    require("pull_request:" in quality and "push:" in quality, "quality-gates.yml: must run for pull requests and main pushes", errors)
    require("${{ secrets." not in quality, "quality-gates.yml: untrusted validation must be secret-free", errors)
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
            and "public-collaboration-sqlite-conformance.py" in quality,
            "quality-gates.yml: desktop, media-service, business, organization, and public-collaboration semantic contracts must be required", errors)
    require("GST_PLUGIN_SYSTEM_PATH_1_0" in quality
            and "gstreamer-sanitized-exec" in quality
            and "hostile-gstreamer-readiness.py" in quality
            and "gstreamer-packages-ci.tsv" in quality
            and "gstreamer-runtime-hostile-xdg-ci.json" in quality,
            "quality-gates.yml: trusted-root, package, hostile-env, and hostile-XDG media gates must be required", errors)
    require("check-media-conformance.py" in quality
            and "media-conformance-offline.json" in quality
            and "media-conformance-dashboard.json" in quality,
            "quality-gates.yml: deterministic media conformance and dashboard evidence must be required", errors)
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
            "quality-gates.yml: the native Tauri shell and pinned Leptos bundle must be required", errors)
    exception_expiry = dt.date(2026, 10, 15)
    require(dt.datetime.now(dt.timezone.utc).date() < exception_expiry,
            "FRAME-DEP-2026-01 has expired; remove or re-review the advisory exception", errors)
    require(re.search(r"^  quality-gate:\n(?:.|\n)*?if:\s*\$\{\{\s*always\(\)\s*\}\}", quality, re.MULTILINE) is not None,
            "quality-gates.yml: quality-gate must be an always-present final result", errors)

    production = texts.get("production-gate.yml", "")
    require("push:\n    branches: [main]" in production, "production-gate.yml: must run directly on every main push", errors)
    require("paths:" not in production and "paths-ignore:" not in production, "production-gate.yml: sentinel may not have path filters", errors)
    require("concurrency:" in production and "cancel-in-progress: false" in production,
            "production-gate.yml: production changes must serialize without cancellation", errors)
    require("environment: production" in production, "production-gate.yml: provider mutation must use the production environment", errors)
    require("secrets.CLOUDFLARE_API_TOKEN" in production and "secrets.CLOUDFLARE_ACCOUNT_ID" in production,
            "production-gate.yml: provider credentials must be explicit environment secrets", errors)
    require("secrets.CLOUDFLARE_D1_DATABASE_ID" in production and "prepare-wrangler-config.py" in production,
            "production-gate.yml: the production D1 ID must be injected only inside the protected job", errors)
    require("actions/download-artifact@" in production and "verify-release-bundle.sh" in production,
            "production-gate.yml: provider job must consume and verify the built artifact", errors)
    production_untrusted = production.split("\n  provider_release:\n", maxsplit=1)[0]
    production_provider = production.split("\n  provider_release:\n", maxsplit=1)[-1]
    require("${{ secrets." not in production_untrusted,
            "production-gate.yml: secrets may appear only in the protected provider job", errors)
    require("target/provider-worker/wrangler-release/index.js --no-bundle" in production_provider,
            "production-gate.yml: protected deploy must upload the verified Worker artifact", errors)
    require("cargo install worker-build" not in production_provider,
            "production-gate.yml: protected provider job must not rebuild the Worker", errors)
    require(re.search(r"^  production-gate:\n(?:.|\n)*?if:\s*\$\{\{\s*always\(\)\s*\}\}", production, re.MULTILINE) is not None,
            "production-gate.yml: production-gate must always resolve to success or failure", errors)
    require("workflow_run:" not in production, "production-gate.yml: delayed workflow_run sentinels are forbidden", errors)
    require("check-parity-evidence.py --require-full" in production_untrusted,
            "production-gate.yml: protected parity records must block provider mutation", errors)
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
    require("package-release.sh" in production and "verify-release-bundle.sh" in production,
            "production-gate.yml: release must create and verify the immutable SBOM-bearing handoff", errors)
    require("build-web-hydration.py" in production
            and "check-web-hydration-bundle.py" in production
            and "web-hydration-smoke.py" in production
            and "target/frame-release/frame-web" in production,
            "production-gate.yml: the executable-adjacent hydration package must be built and smoked", errors)

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
    require("apps/web crates scripts/ci Cargo.toml Cargo.lock rust-toolchain.toml render.yaml" in smoke,
            "production-smoke.yml: paired release paths must match Render's build filter", errors)

    change_plan_path = ROOT / "scripts" / "ci" / "release-change-plan.sh"
    change_plan = change_plan_path.read_text(encoding="utf-8") if change_plan_path.is_file() else ""
    require("apps/web/* | crates/* | scripts/ci/* | Cargo.toml | Cargo.lock | rust-toolchain.toml | render.yaml" in change_plan,
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
