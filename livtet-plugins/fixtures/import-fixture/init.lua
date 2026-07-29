-- crates/livtet-plugins/fixtures/import-fixture/init.lua
--
-- Mock-host integration test fixture for the `import_*` capability
-- family. Returns canned responses for every import_* method, scoped
-- to sources that look like a Calibre `metadata.db` (matches on the
-- `metadata.db` suffix). Anything else returns nil for `import_detect`
-- or an empty list for `import_list_items` / `import_items` — same as
-- the bundled Calibre plugin's expected "decline" behaviour.
--
-- No host.fs_copy / host.fs_symlink calls are exercised in this
-- fixture; the test asserts return shapes only. A future slice can
-- extend the fixture to invoke those host functions on
-- `import_items` so the test pins the path shape the plugin would
-- pass to the host.

local provider = {
  id = "import-fixture",
  name = "Import (Fixture)",
  version = "0.1.0",
  capabilities = {
    import_detect = true,
    import_list_items = true,
    import_items = true,
  },
  requires = { http = false, secrets = false, filesystem = false },
}

--- True when the source looks like a Calibre metadata.db file path.
--- The path-based detection is intentionally stringly-typed: real
--- sources don't carry a magic-byte header in this fixture, so a
--- suffix check on `metadata.db` is the cheapest way to differentiate
--- the "Calibre library" branch from the "declined" branch without
--- touching the host.sqlite_query surface.
local function is_calibre_db(source)
  if type(source) ~= "table" then
    return false
  end
  local path = source.path
  if type(path) ~= "string" then
    return false
  end
  return path:find("metadata%.db$") ~= nil
end

function provider.import_detect(source)
  if not is_calibre_db(source) then
    return nil
  end
  return {
    confidence = 1.0,
    format_name = "Calibre SQLite",
    estimated_count = 3,
  }
end

function provider.import_list_items(source, _options)
  if not is_calibre_db(source) then
    return {}
  end
  return {
    {
      id = "1",
      title = "Test Book 1",
      authors = { "Author One" },
      identifiers = { "urn:isbn:9780000000001" },
      cover_url = "/tmp/livtet-fixture/cover-1.jpg",
    },
    {
      id = "2",
      title = "Test Book 2",
      authors = { "Author Two" },
      identifiers = { "urn:isbn:9780000000002" },
      cover_url = "/tmp/livtet-fixture/cover-2.jpg",
    },
    {
      id = "3",
      title = "Test Book 3",
      authors = { "Author Three" },
      identifiers = { "urn:isbn:9780000000003" },
    },
  }
end

function provider.import_items(source, options)
  if not is_calibre_db(source) then
    return {}
  end
  local selected = {}
  if type(options) == "table" and type(options.selected_items) == "table" then
    for _, id in ipairs(options.selected_items) do
      selected[id] = true
    end
  end
  local records = {}
  local fixtures = {
    {
      id = "1",
      title = "Test Book 1",
      authors = { "Author One" },
      identifiers = { "urn:isbn:9780000000001" },
      cover_url = "/tmp/livtet-fixture/cover-1.jpg",
      file = {
        path = "/tmp/livtet-fixture/book1.epub",
        format = "epub",
        size = 12345,
      },
      series_name = "Test Series",
      series_position = 1,
      original_id = "urn:calibre:uuid:aaa-bbb-ccc-1",
    },
    {
      id = "2",
      title = "Test Book 2",
      authors = { "Author Two" },
      identifiers = { "urn:isbn:9780000000002" },
      cover_url = "/tmp/livtet-fixture/cover-2.jpg",
      file = {
        path = "/tmp/livtet-fixture/book2.epub",
        format = "epub",
        size = 23456,
      },
      series_name = "Test Series",
      series_position = 2,
      original_id = "urn:calibre:uuid:aaa-bbb-ccc-2",
    },
    {
      id = "3",
      title = "Test Book 3",
      authors = { "Author Three" },
      identifiers = { "urn:isbn:9780000000003" },
      file = {
        path = "/tmp/livtet-fixture/book3.mobi",
        format = "mobi",
      },
      original_id = "urn:calibre:uuid:aaa-bbb-ccc-3",
    },
  }
  for _, f in ipairs(fixtures) do
    if next(selected) == nil or selected[f.id] then
      table.insert(records, {
        title = f.title,
        authors = f.authors,
        identifiers = f.identifiers,
        cover_url = f.cover_url,
        files = { f.file },
        series_name = f.series_name,
        series_position = f.series_position,
        original_id = f.original_id,
      })
    end
  end
  return records
end

return provider
