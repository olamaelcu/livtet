-- fixtures/reading-list-fixture/init.lua
--
-- Minimal reading_list plugin used by host_manager_tests.rs to verify
-- that the Rust dispatcher round-trips list_sources() and fetch_lists()
-- through the host. Returns canned data — no HTTP, no OAuth. Mirrors
-- the shape from
-- docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md §5
-- so the dispatcher code and the spec stay in lockstep.

local provider = {
  id = "reading-list-fixture",
  name = "Reading List Fixture",
  version = "1.0.0",
  capabilities = { reading_list = true },
  requires = { http = false, secrets = false, oauth = false },
}

function provider.list_sources()
  return {
    {
      id = "fixture-list",
      label = "Fixture List",
      description = "Returns canned reading lists for tests",
      supports_smart = false,
      supports_sync = true,
      sync_direction = "pull_only",
      config_fields = {
        { key = "ignore", label = "ignored", type = "text" },
      },
    },
  }
end

function provider.fetch_lists(source_id, _config)
  if source_id ~= "fixture-list" then
    error("unknown source_id: " .. tostring(source_id))
  end
  return {
    lists = {
      {
        name = "To Read",
        description = "Books I plan to read",
        list_type = "synced",
        external_id = "fixture:list:to-read",
        items = {
          {
            identifiers = { "urn:isbn:9780441013593" },
            position = 1,
            added_at = "2026-01-15T10:00:00Z",
          },
          {
            identifiers = { "urn:isbn:9780441172719" },
            position = 2,
            added_at = "2026-01-16T10:00:00Z",
          },
        },
      },
    },
  }
end

return provider
