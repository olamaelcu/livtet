-- fixtures/series-fixture/init.lua
--
-- Minimal series plugin used by host_manager_tests.rs to verify that
-- the Rust dispatcher round-trips detect_series() and
-- get_series_order() through the host. Returns canned data — no HTTP,
-- no real Open Library calls. Mirrors the shape from
-- docs/superpowers/plugins/2026-06-02-plugin-capability-interfaces.md §6
-- so the dispatcher code and the spec stay in lockstep.

local provider = {
  id = "series-fixture",
  name = "Series Fixture",
  version = "1.0.0",
  capabilities = { series = true },
  requires = { http = false },
}

function provider.detect_series(edition_info)
  if not edition_info or not edition_info.id then
    return { series = {} }
  end
  return {
    series = {
      {
        name = "Dune Chronicles",
        series_type = "novel",
        external_id = "fixture:series:dune-chronicles",
        source_url = "https://example.com/series/dune-chronicles",
        position = 1,
        total_entries = 6,
      },
    },
  }
end

function provider.get_series_order(series_info)
  if not series_info or series_info.external_id ~= "fixture:series:dune-chronicles" then
    return {
      entries = {},
      order_type = "publication",
      available_orders = { "publication" },
    }
  end
  return {
    entries = {
      {
        position = 1,
        title = "Dune",
        identifiers = { "urn:isbn:9780441172719" },
        published_date = "1965-08-01",
        in_universe_order = nil,
      },
      {
        position = 2,
        title = "Dune Messiah",
        identifiers = { "urn:isbn:9780441172699" },
        published_date = "1969-01-01",
        in_universe_order = nil,
      },
    },
    order_type = "publication",
    available_orders = { "publication", "chronological" },
  }
end

return provider
