-- fixtures/annotations-fixture/init.lua
--
-- Minimal annotations plugin used by host_manager_tests.rs to verify
-- that the Rust dispatcher round-trips annotation_sources() and
-- fetch_annotations() through the host. Returns canned data — no
-- host.read_file, no real Kindle database. Mirrors the shape from
-- docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md §4
-- so the dispatcher code and the spec stay in lockstep.

local provider = {
  id = "annotations-fixture",
  name = "Annotations Fixture",
  version = "1.0.0",
  capabilities = { annotations = true },
  requires = { http = false, secrets = false },
}

function provider.annotation_sources()
  return {
    {
      id = "fixture-source",
      label = "Fixture Source",
      description = "Returns canned annotations for tests",
      config_fields = {
        { key = "ignore", label = "ignored", type = "text" },
      },
    },
  }
end

function provider.fetch_annotations(source_id, _config)
  if source_id ~= "fixture-source" then
    error("unknown source_id: " .. tostring(source_id))
  end
  return {
    annotations = {
      {
        identifiers = { "urn:isbn:9780441013593" },
        title = "Dune",
        author = "Frank Herbert",
        content = "I must not fear. Fear is the mind-killer.",
        note = "Great opening line",
        location = "Loc. 45-47",
        location_type = "range",
        color = "yellow",
        tags = { "favorite" },
        created_at = "2026-05-28T14:30:00Z",
      },
    },
    has_more = false,
  }
end

return provider
