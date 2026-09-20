# Spec: `livtet` Ruby gem (magnus binding of livtet-core)

Status: proposed — awaiting implementation.
Date: 2026-09-20

## Goal

Expose `livtet-core` to Ruby through a `magnus` native extension, starting
with the smallest honest surface: opening and closing the database. The
architecture is built so the surface can grow (records, search, seed, reindex)
without re-architecting the boundary.

## Decisions made

| Decision | Choice |
|---|---|
| Surface | `Livtet.paths` + `Livtet::Database` open/close only. No query API, no Record/Edition classes, no JSON bridge, no seed/search/reindex. |
| Packaging | Dev-only gem, built via `rake-compiler` + `rb-sys`. No publish machinery. |
| Location | New workspace member `core/livtet-ruby/`. |
| Dependency | `livtet-core` only. **Never** `livtet-cli-common`. |
| Async boundary | Embedded tokio runtime; every call `block_on`s. GVL held during block. |
| Data marshalling | Deferred — nothing crosses the boundary yet. |
| Migration | `Database.open` auto-runs `Migrator::up`. |
| Workspace lints | Crate does **not** opt into `[lints] workspace = true` (workspace denies `unsafe_code`, magnus requires it). |

## Structure

```
livtet-ruby/
  Cargo.toml              # crate-type = ["cdylib"], name livtet-ruby
  livtet.gemspec          # dev-only, no publish config
  Gemfile + Rakefile      # rake-compiler via rb-sys gem
  lib/livtet.rb           # require "livtet/livtet"; nothing else
  ext/livtet/extconf.rb   # create_rust_makefile("livtet/livtet")
  src/lib.rs              # #[magnus::init] — register module + class
  src/runtime.rs          # OnceLock<tokio Runtime> + block_on helper
  src/error.rs            # → Livtet::Error < StandardError
  src/paths.rs            # Livtet.paths
  src/database.rs         # Livtet::Database
```

## Rust internals

- `runtime()` — lazily initialized multi-thread tokio runtime in `OnceLock`;
  all calls `block_on`.
- `Database` typed-data handle:
  - `open` → `SharedState::from_pool(pool, path)` (deliberately **not** the
    `init_state` global singleton), runs `Migrator::up` before returning.
  - `close` → `optimize_and_close`, then marks handle dead; any subsequent call
    raises `Livtet::Error`.
  - Block form handled in pure Ruby (`ensure` + `close`).
- No `livtet-cli-common` anywhere. No query API, no Record/Edition classes, no
  JSON bridge (no data crosses yet), no seed/search/reindex — the layout leaves
  room to add them later.

## Public API

```ruby
require "livtet"

Livtet.paths                          # => {bundle:, data:, config:, logs:}

db = Livtet::Database.open                     # default data dir (paths.rs resolution)
db = Livtet::Database.open("/tmp/x.db")        # explicit path
db.path                               # => "/tmp/x.db"
db.close                              # optimize + close pool; every later call raises Livtet::Error
Livtet::Database.open { |db| ... }    # block form: closes on exit, like File.open
```

## Verification

```
cargo check -p livtet-ruby
bundle install && rake compile
```

Smoke script:

```ruby
require "livtet"
pp Livtet.paths
db = Livtet::Database.open("/tmp/livtet-test.db") { |d| d.path } # auto-migrates, auto-closes
```

## Next steps on approval

Write spec to `docs/superpowers/specs/2026-09-20-livtet-ruby-gem-design.md`
(needs write access), then the implementation plan. Git commit of the spec only
on explicit word.