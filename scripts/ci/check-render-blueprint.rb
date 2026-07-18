#!/usr/bin/env ruby
# frozen_string_literal: true

require "psych"

path = ARGV.fetch(0, "render.yaml")
blueprint = Psych.safe_load(File.read(path), aliases: false, filename: path)
errors = []
serialized = File.read(path)

unless blueprint.is_a?(Hash)
  warn "#{path}: expected a top-level mapping"
  exit 1
end

previews = blueprint.fetch("previews", {})
unless previews == {"generation" => "manual", "expireAfterDays" => 3}
  errors << "previews must be manual and expire after exactly 3 inactive days"
end

services = blueprint.fetch("services", [])
errors << "Blueprint must declare exactly one dedicated service" unless services.length == 1
web = services.find { |service| service.is_a?(Hash) && service["name"] == "frame-web" }

if web.nil?
  errors << "services must contain frame-web"
else
  expected = {
    "type" => "web",
    "runtime" => "rust",
    "plan" => "starter",
    "region" => "oregon",
    "numInstances" => 2,
    "buildCommand" => "cargo install trunk --version 0.21.14 --locked && python3 -I scripts/ci/build-web-hydration.py --runtime-dir target/release/web-dist && python3 -I scripts/ci/check-web-hydration-bundle.py && cargo build --locked --release -p frame-web",
    "startCommand" => "./target/release/frame-web",
    "healthCheckPath" => "/health/ready",
    "maxShutdownDelaySeconds" => 60,
    "renderSubdomainPolicy" => "enabled",
    "autoDeployTrigger" => "checksPass"
  }
  expected.each do |key, value|
    errors << "frame-web.#{key} must equal #{value.inspect}" unless web[key] == value
  end

  forbidden_service_fields = %w[
    branch disk diskSizeGB dockerCommand dockerfilePath preDeployCommand
    initialDeployHook afterFirstDeployCommand scaling
  ]
  forbidden_service_fields.each do |field|
    errors << "frame-web must not declare #{field}" if web.key?(field)
  end
  errors << "frame-web must not declare a deploy hook" if web.keys.any? { |key| key.to_s.downcase.include?("deployhook") }

  domains = web.fetch("domains", [])
  errors << "frame-web must own only the canonical production domain" unless domains == ["frame.engmanager.xyz"]

  paths = web.dig("buildFilter", "paths") || []
  required_paths = %w[apps/web/** crates/** fixtures/web-authenticated/** Cargo.toml Cargo.lock rust-toolchain.toml scripts/ci/** render.yaml]
  required_paths.each do |required|
    errors << "frame-web buildFilter.paths is missing #{required}" unless paths.include?(required)
  end
  errors << "frame-web buildFilter.paths contains duplicates" unless paths.uniq == paths
  errors << "frame-web must not define build-filter ignored paths" if web.dig("buildFilter", "ignoredPaths")

  env = web.fetch("envVars", []).each_with_object({}) do |entry, index|
    errors << "frame-web envVars contains a duplicate #{entry["key"]}" if entry.is_a?(Hash) && index.key?(entry["key"])
    index[entry["key"]] = entry if entry.is_a?(Hash) && entry["key"]
  end
  expected_env_keys = %w[
    FRAME_DEPLOYMENT FRAME_PUBLIC_ORIGIN FRAME_API_ORIGIN FRAME_PROXY_TRUST
    FRAME_ENABLE_PUBLIC_EMBED RUSTUP_TOOLCHAIN RUST_LOG FRAME_DIAGNOSTIC_TOKEN
    FRAME_WORKER_RELEASE FRAME_RENDER_DEPLOY FRAME_MIGRATION_LEVEL
    FRAME_PORTFOLIO_CONSUMER
  ]
  errors << "frame-web env inventory must be exact" unless env.keys.sort == expected_env_keys.sort
  expected_env = {
    "FRAME_DEPLOYMENT" => ["production", "preview"],
    "FRAME_PUBLIC_ORIGIN" => ["https://frame.engmanager.xyz", "https://frame-preview.invalid"],
    "FRAME_API_ORIGIN" => ["https://frame.engmanager.xyz", "https://frame-staging.engmanager.xyz"],
    "FRAME_PROXY_TRUST" => ["render", "render"],
    "FRAME_ENABLE_PUBLIC_EMBED" => ["false", "false"],
    "RUSTUP_TOOLCHAIN" => ["1.96.1", "1.96.1"],
    "RUST_LOG" => ["info", "info"]
  }
  expected_env.each do |key, (value, preview_value)|
    errors << "#{key} production value must equal #{value.inspect}" unless env.dig(key, "value").to_s == value
    errors << "#{key} previewValue must equal #{preview_value.inspect}" unless env.dig(key, "previewValue").to_s == preview_value
    errors << "#{key} must not be a synced secret" if env.dig(key, "sync") == false
  end
  errors << "FRAME_PUBLIC_ORIGIN must be the canonical HTTPS origin" unless env.dig("FRAME_PUBLIC_ORIGIN", "value") == "https://frame.engmanager.xyz"
  errors << "FRAME_API_ORIGIN must remain same-origin in production" unless env.dig("FRAME_API_ORIGIN", "value") == "https://frame.engmanager.xyz"
  diagnostic = env.fetch("FRAME_DIAGNOSTIC_TOKEN", {})
  errors << "FRAME_DIAGNOSTIC_TOKEN must be configured as a Render secret" unless diagnostic == {"key" => "FRAME_DIAGNOSTIC_TOKEN", "sync" => false}
  %w[
    FRAME_WORKER_RELEASE FRAME_RENDER_DEPLOY FRAME_MIGRATION_LEVEL
    FRAME_PORTFOLIO_CONSUMER
  ].each do |key|
    errors << "#{key} must be an unsynced protected promotion value" unless env[key] == {"key" => key, "sync" => false}
  end
end

errors << "Blueprint must declare the official schema URL" unless serialized.lines.first&.strip == "# yaml-language-server: $schema=https://render.com/schema/render.yaml.json"
errors << "Blueprint must not contain a Render deploy-hook URL" if serialized.match?(%r{https://api\.render\.com/deploy/})
errors << "Blueprint must not configure a persistent disk" if serialized.match?(/^\s+(disk|mountPath|diskSizeGB):/)
errors << "Blueprint must not build the workspace or media packages" if serialized.match?(/cargo build[^\n]*(--workspace|frame-media|frame-control-plane)/)
errors << "Blueprint must not contain provider credentials" if serialized.match?(/(CLOUDFLARE_API_TOKEN|R2_ACCESS_KEY|D1_DATABASE_ID|AWS_SECRET_ACCESS_KEY)/)

unless errors.empty?
  warn errors.map { |error| "#{path}: #{error}" }.join("\n")
  exit 1
end

puts "Render Blueprint authority and production invariants passed."
