-- fixtures/host-probe/init.lua
--
-- A bare-bones plugin whose only job is to call every host.* function
-- the host is supposed to implement and return the result through
-- capability functions. Each capability name maps 1:1 to a host
-- function (or wraps multiple into a single call), so the
-- `crates/livtet-plugin/tests/host_lua_tests.rs` integration tests
-- can load this plugin, call a capability, and assert the host's
-- round-trip behaviour.

local probe = {
  id = "host-probe",
  name = "Host Probe",
  version = "1.0.0",
  capabilities = {
    probe = true,
  },
  requires = {},
}

--- http_get ------------------------------------------------------------

function probe.http_get(url)
  local resp = host.http_get(url, {})
  return { status = resp.status, body = resp.body, has_headers = resp.headers ~= nil }
end

--- log -----------------------------------------------------------------

function probe.log(level, message)
  host.log(level, message)
  return "logged"
end

--- get_secret / set_secret --------------------------------------------

function probe.get_secret(name)
  local v = host.get_secret(name)
  return { value = v }
end

function probe.set_secret(name, value)
  host.set_secret(name, value)
  return { ok = true }
end

--- url_encode / url_decode --------------------------------------------

function probe.url_encode(s)
  return { result = host.url_encode(s) }
end

function probe.url_decode(s)
  return { result = host.url_decode(s) }
end

--- read_file ----------------------------------------------------------

function probe.read_file(path)
  local v, err = host.read_file(path)
  if v == nil then
    return { error = err or "nil result" }
  end
  return { result = v }
end

--- sqlite_query -------------------------------------------------------

function probe.sqlite_query(path, sql, params, limit)
  local v, err = host.sqlite_query(path, sql, params, limit)
  if v == nil then
    return { error = err or "nil result" }
  end
  return { result = v }
end

--- emit_event ---------------------------------------------------------

function probe.emit_event(event_type, payload)
  host.emit_event(event_type, payload)
  return { ok = true }
end

--- resolve_identifier / resolve_identifiers / edition info ----------

function probe.resolve_identifier(urn)
  local v = host.resolve_identifier(urn)
  return { result = v }
end

function probe.resolve_identifiers(urns)
  local v = host.resolve_identifiers(urns)
  return { result = v }
end

function probe.get_edition_info(edition_id)
  local v = host.get_edition_info(edition_id)
  return { result = v }
end

function probe.get_edition_identifiers(edition_id)
  local v = host.get_edition_identifiers(edition_id)
  return { result = v }
end

--- fetch_progress / upsert_progress -----------------------------

function probe.fetch_progress(urn)
  local v, err = host.fetch_progress(urn)
  if v == nil and err ~= nil then
    return { error = err }
  end
  return { result = v }
end

function probe.upsert_progress(urn, progress, last_location, total_secs)
  local v, err = host.upsert_progress(urn, progress, last_location, total_secs)
  if v == nil then
    return { error = err or "nil result" }
  end
  return { result = v }
end

--- html_strip -----------------------------------------------------------
-- Strip HTML tags, decode common HTML entities, collapse whitespace,
-- and return plain text. The bundled openlibrary plugin calls this
-- to normalize work descriptions that may contain inline HTML.

function probe.html_strip(html)
  return { result = host.html_strip(html) }
end

--- html_parse -----------------------------------------------------------
-- Parse a fragment of HTML into a `doc` userdata, run a CSS selector
-- against it, and pull the first match's `text()` + `attr(name)` back
-- out. The probe flattens the userdata into a JSON-friendly table so
-- the host's mlua→serde_json conversion doesn't have to walk opaque
-- userdata.

function probe.html_parse(html, sel, attr)
  local doc = host.html_parse(html)
  local rows = doc:select(sel)
  local first = rows[1]
  if first == nil then
    return { count = 0, text = nil, attr = nil }
  end
  local first_text = first:text()
  local first_attr = first:attr(attr)
  return {
    count = #rows,
    text = first_text,
    attr = first_attr,
  }
end

--- get_setting ----------------------------------------------------------
-- Read a plugin-level setting from the host. Returns nil when the
-- setting is not set so the caller can fall through to a default.

function probe.get_setting(key)
  local v = host.get_setting(key)
  if v == nil then
    return { value = nil }
  end
  return { value = v }
end

--- urn -----------------------------------------------------------------
-- Build a canonical URN string. The host validates the scheme
-- against [%w_-]+, so a malformed namespace surfaces as a Lua
-- error rather than a malformed wire-format string.

function probe.urn(ns, value)
  local ok, result = pcall(host.urn, ns, value)
  if not ok then
    return { error = tostring(result) }
  end
  return { result = result }
end

return probe
