-- fixtures/watch-fixture/init.lua
--
-- Minimal watch provider used by host_manager_tests.rs to verify
-- that the Rust dispatcher round-trips `provider.watch(since)`
-- through the host. Returns canned data — no HTTP, no real
-- polling. Mirrors the shape from
-- docs/superpowers/plugins/2026-06-07-plugin-roadmap.md P3 §"Watch
-- capability" so the dispatcher code and the spec stay in
-- lockstep.
--
-- Calling `provider.watch(nil)` (the host's first poll) returns
-- one batch. Calling it again with the returned `next_cursor`
-- returns a tail batch and `has_more = false`. Any other cursor
-- returns an empty batch — the fixture's "you've reached the end
-- of the demo data" signal.

local provider = {
  id = "watch-fixture",
  name = "Watch Fixture",
  version = "1.0.0",
  capabilities = { watch = true },
  requires = { http = false },
}

local FIRST_CURSOR = "cursor:batch-1"
local SECOND_CURSOR = "cursor:batch-2"

function provider.watch(since)
  if since == nil or since == "" then
    return {
      changes = {
        {
          kind = "availability",
          identifier = "urn:isbn:9780441172719",
          title = "Dune",
          available = true,
          observed_at = "2026-06-09T12:00:00Z",
        },
        {
          kind = "availability",
          identifier = "urn:isbn:9780553103540",
          title = "A Game of Thrones",
          available = false,
          observed_at = "2026-06-09T12:00:00Z",
        },
      },
      has_more = true,
      next_cursor = FIRST_CURSOR,
    }
  elseif since == FIRST_CURSOR then
    return {
      changes = {
        {
          kind = "availability",
          identifier = "urn:isbn:9780345339706",
          title = "The Hobbit",
          available = true,
          observed_at = "2026-06-09T12:05:00Z",
        },
      },
      has_more = false,
      next_cursor = SECOND_CURSOR,
    }
  else
    return {
      -- Returning `nil` (i.e. omitting `changes`) for the
      -- exhausted-cursor case matches what the Rust decoder
      -- accepts as "empty batch". mlua serializes a Lua `{}`
      -- table as a JSON object, which serde would reject
      -- because `changes` is typed as `Vec<JsonValue>` —
      -- `nil` cleanly serializes to `null`, which
      -- `#[serde(default)]` then turns into an empty vec.
      has_more = false,
      next_cursor = since,
    }
  end
end

return provider
