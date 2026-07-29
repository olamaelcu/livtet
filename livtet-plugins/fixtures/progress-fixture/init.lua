-- fixtures/progress-fixture/init.lua
--
-- Minimal reading_progress plugin used by host_manager_tests.rs to verify
-- that the Rust dispatcher round-trips progress_sources() and
-- fetch_progress() through the host. Returns canned data — no HTTP calls,
-- no real KOReader server. Mirrors the shape from
-- docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md §3
-- so the dispatcher code and the spec stay in lockstep.

local provider = {
  id = "progress-fixture",
  name = "Progress Fixture",
  version = "1.0.0",
  capabilities = { reading_progress = true },
  requires = { http = false, secrets = false },
}

function provider.progress_sources()
  return {
    {
      id = "fixture-source",
      label = "Fixture Source",
      description = "Returns canned entries for tests",
      config_fields = {
        { key = "ignore", label = "ignored", type = "text" },
      },
    },
  }
end

function provider.fetch_progress(source_id, _config)
  if source_id ~= "fixture-source" then
    error("unknown source_id: " .. tostring(source_id))
  end
  return {
    entries = {
      {
        identifiers = { "urn:isbn:9780441172719" },
        progress = 0.65,
        progress_type = "percentage",
        last_location = "65%",
        total_reading_time_secs = 3600,
        last_read_at = "2026-05-28T14:30:00Z",
        device_info = {
          device_name = "Fixture Device",
          app_version = "fixture-1.0",
        },
      },
    },
    has_more = false,
    cursor = nil,
  }
end

return provider
