#!/usr/bin/env ruby
# frozen_string_literal: true

require "psych"

files = if ARGV.empty?
          Dir[".github/workflows/*.{yml,yaml}"] + ["render.yaml"]
        else
          ARGV
        end

errors = []

def scalar_key(node)
  return nil unless node.is_a?(Psych::Nodes::Scalar)

  [node.tag, node.value]
end

def check_duplicate_keys(node, path, errors)
  if node.is_a?(Psych::Nodes::Mapping)
    seen = {}
    node.children.each_slice(2) do |key, value|
      signature = scalar_key(key)
      if signature && seen.key?(signature)
        errors << "#{path}: duplicate mapping key #{key.value.inspect} at line #{key.start_line + 1}"
      elsif signature
        seen[signature] = true
      end
      check_duplicate_keys(value, path, errors)
    end
  else
    Array(node.children).each { |child| check_duplicate_keys(child, path, errors) }
  end
end

files.sort.uniq.each do |path|
  unless File.file?(path)
    errors << "#{path}: file does not exist"
    next
  end

  begin
    document = Psych.parse_file(path)
    check_duplicate_keys(document, path, errors) if document
  rescue Psych::SyntaxError => e
    errors << "#{path}:#{e.line}:#{e.column}: #{e.problem}"
  end
end

unless errors.empty?
  warn errors.join("\n")
  exit 1
end

puts "YAML syntax and duplicate-key checks passed for #{files.length} file(s)."
