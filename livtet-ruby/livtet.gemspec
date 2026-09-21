# frozen_string_literal: true

require_relative "lib/livtet/version"

Gem::Specification.new do |s|
  s.name = "livtet"
  s.version = Livtet::VERSION
  s.authors = ["livtet"]
  s.summary = "Ruby bindings for livtet-core"
  s.description = "Minimal Ruby wrapper around livtet-core: well-known paths and database open/close. No query APIs in v1."
  s.license = "BUSL-1.1"
  s.required_ruby_version = ">= 3.1"
  s.files = Dir["lib/**/*.rb"]
  s.require_paths = ["lib"]
end
