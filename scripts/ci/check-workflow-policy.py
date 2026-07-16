#!/usr/bin/env python3
"""Enforce security-critical invariants for Frame-owned delivery workflows."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
OWNED = {
    "ci.yml",
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
}


def require(condition: bool, message: str, errors: list[str]) -> None:
    if not condition:
        errors.append(message)


def main() -> int:
    errors: list[str] = []
    texts: dict[str, str] = {}

    for name in sorted(OWNED):
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
    require("secrets." not in quality, "quality-gates.yml: untrusted validation must be secret-free", errors)
    require("check-parity-evidence.py" in quality,
            "quality-gates.yml: the fast parity evidence lane must be required", errors)
    require("macos-14" in quality and "windows-2022" in quality,
            "quality-gates.yml: portable core checks must cover macOS and Windows", errors)
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
    require("secrets." not in production_untrusted,
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

    smoke = texts.get("production-smoke.yml", "")
    require("schedule:" in smoke and "workflow_dispatch:" in smoke and "workflow_run:" in smoke,
            "production-smoke.yml: requires scheduled, manual, and post-gate entrypoints", errors)
    require("secrets." not in smoke, "production-smoke.yml: canonical smoke must be secret-free", errors)
    require("https://frame.engmanager.xyz" in smoke and "https://frame-staging.engmanager.xyz" in smoke,
            "production-smoke.yml: smoke origins must be a fixed allowlist", errors)
    require("github.event.workflow_run.conclusion == 'success'" in smoke and "expected_release_sha" in smoke,
            "production-smoke.yml: paired release verification must follow only a successful production gate", errors)

    terraform = texts.get("cloudflare-account.yml", "")
    require("pull_request:" in terraform and "workflow_dispatch:" in terraform,
            "cloudflare-account.yml: must expose untrusted validation and protected manual operation", errors)
    require("github.event_name == 'workflow_dispatch'" in terraform,
            "cloudflare-account.yml: credentialed job must be unreachable from pull_request", errors)
    terraform_untrusted = terraform.split("\n  terraform:\n", maxsplit=1)[0]
    require("secrets." not in terraform_untrusted,
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

    print(f"Workflow policy checks passed for all {len(OWNED)} Frame-owned workflows.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
