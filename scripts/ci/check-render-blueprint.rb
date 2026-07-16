#!/usr/bin/env ruby
# frozen_string_literal: true

require "psych"

path = ARGV.fetch(0, "render.yaml")
blueprint = Psych.safe_load(File.read(path), aliases: false, filename: path)
errors = []

unless blueprint.is_a?(Hash)
  warn "#{path}: expected a top-level mapping"
  exit 1
end

services = blueprint.fetch("services", [])
web = services.find { |service| service.is_a?(Hash) && service["name"] == "frame-web" }

if web.nil?
  errors << "services must contain frame-web"
else
  expected = {
    "type" => "web",
    "runtime" => "rust",
    "buildCommand" => "cargo install trunk --version 0.21.14 --locked && python3 -I scripts/ci/build-web-hydration.py --runtime-dir target/release/web-dist && python3 -I scripts/ci/check-web-hydration-bundle.py && cargo build --locked --release -p frame-web",
    "startCommand" => "./target/release/frame-web",
    "healthCheckPath" => "/health/ready",
    "autoDeployTrigger" => "checksPass"
  }
  expected.each do |key, value|
    errors << "frame-web.#{key} must equal #{value.inspect}" unless web[key] == value
  end

  errors << "frame-web must not declare a deploy hook" if web.keys.any? { |key| key.to_s.downcase.include?("deployhook") }

  domains = web.fetch("domains", [])
  errors << "frame-web must own only the canonical production domain" unless domains == ["frame.engmanager.xyz"]

  paths = web.dig("buildFilter", "paths") || []
  %w[apps/web/** crates/** Cargo.toml Cargo.lock rust-toolchain.toml scripts/ci/** render.yaml].each do |required|
    errors << "frame-web buildFilter.paths is missing #{required}" unless paths.include?(required)
  end

  env = web.fetch("envVars", []).each_with_object({}) do |entry, index|
    index[entry["key"]] = entry if entry.is_a?(Hash) && entry["key"]
  end
  errors << "FRAME_PUBLIC_ORIGIN must be the canonical HTTPS origin" unless env.dig("FRAME_PUBLIC_ORIGIN", "value") == "https://frame.engmanager.xyz"
  errors << "FRAME_API_ORIGIN must remain same-origin in production" unless env.dig("FRAME_API_ORIGIN", "value") == "https://frame.engmanager.xyz"
  errors << "FRAME_DIAGNOSTIC_TOKEN must be configured as a Render secret" unless env.dig("FRAME_DIAGNOSTIC_TOKEN", "sync") == false
end

serialized = File.read(path)
errors << "Blueprint must not contain a Render deploy-hook URL" if serialized.match?(%r{https://api\.render\.com/deploy/})

unless errors.empty?
  warn errors.map { |error| "#{path}: #{error}" }.join("\n")
  exit 1
end

puts "Render Blueprint authority and production invariants passed."
