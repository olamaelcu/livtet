-- fixtures/provider-fixture/init.lua
--
-- Minimal search/lookup/enrich/cover provider used by
-- host_manager_tests.rs to verify that the Rust dispatcher
-- round-trips these capabilities through the host. Returns canned
-- data — no HTTP, no real network. Mirrors the shape from
-- docs/superpowers/plugins/2026-06-01-plugin-system.md §3 and the
-- openlibrary plugin's function signatures so the dispatcher code
-- and the spec stay in lockstep.

local provider = {
  id = "provider-fixture",
  name = "Provider Fixture",
  version = "1.0.0",
  capabilities = {
    search = true,
    lookup = true,
    enrich = true,
    cover = true,
  },
  requires = { http = false, secrets = false },
}

function provider.search(query, _options)
  if type(query) ~= "string" or query == "" then
    -- Returning an empty Lua table matches what the bundled
    -- openlibrary plugin does for blank queries. mlua serializes
    -- it to an empty JSON object; the Rust dispatcher handles both
    -- shapes via the test's match arm.
    return {}
  end
  return {
    {
      id = "fixture-hit-1",
      title = "Fixture Book " .. query,
      authors = { "Fixture Author" },
      year = 2026,
      identifiers = { "urn:isbn:9780441172719" },
      relevance_score = 1.0,
    },
    {
      id = "fixture-hit-2",
      title = "Fixture Companion " .. query,
      authors = { "Fixture Author" },
      year = 2026,
      identifiers = { "urn:isbn:9780553103540" },
      relevance_score = 0.7,
    },
  }
end

function provider.lookup(identifier)
  if type(identifier) ~= "string" or identifier == "" then
    return nil
  end
  return {
    id = "fixture-lookup-" .. identifier,
    title = "Lookup Result for " .. identifier,
    authors = { "Fixture Author" },
    year = 2026,
    identifiers = { identifier },
  }
end

function provider.enrich(work_info)
  if type(work_info) ~= "table" then
    return nil
  end
  return {
    id = work_info.id or "fixture-enrich",
    title = work_info.title or "Enriched Title",
    authors = work_info.authors or { "Original Author" },
    year = work_info.year or 2026,
    description = "Enriched description for " .. tostring(work_info.id or "unknown"),
    subjects = { "fixture", "test" },
    identifiers = work_info.identifiers or {},
  }
end

function provider.get_cover(work_info, _edition_info)
  local id = (type(work_info) == "table" and work_info.id) or "unknown"
  return {
    url = "https://covers.example.org/covers/" .. id .. "-L.jpg",
    size = "L",
    source = "provider-fixture",
  }
end

return provider
