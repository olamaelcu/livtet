# Livtet Core

The shared Rust workspace for the Livtet personal book-collection
manager. This is the engine: the database layer, business logic,
cover subsystem, search index, plugin runtime, sync engine, CLI, and
the FFI surface used by the desktop and mobile apps.

Everything in this repo is published under the
[Mozilla Public License 2.0](LICENSE).

The rest of Livtet (the [desktop][1], [Android and iOS][2] apps) lives in
sibling repositories and links against the crates built here.

## What It Is

Every Livtet client embeds this workspace, directly or indirectly:

- The [Tauri desktop][1] app links the individual crates it needs.
- The [Android and iOS][2] apps reach the same surface through
  `livtet-ffi`, a uniffi-generated binding that re-exports a curated
  subset across the language boundary.

`livtet-core` is _the unison crate_. It composes `livtet-data`,
`livtet-covers`, and `livtet-search` into the public surface every
client depends on, and re-exports the curated names so consumers can
import a single crate &mdash.

## Layout

```
livtet-data          DB owner (sea-orm, sqlx, migrator, seed, error)
livtet-types         Leaf type definitions (DbId, URN, identifiers, format metadata)
livtet-covers        Cover subsystem (provider trait, cache, fetcher)
livtet-search        Search index (Tantivy)
livtet-core          Unison crate (data + covers + search + types)
livtet-sync          Sync engine (server + client + change log)
livtet-plugins       Plugin runtime (host, signing, archives, lua)
livtet-plugins-lua   Lua plugin stub (merger target; not yet implemented)
livtet-ffi           FFI surface over core (uniffi — Android, iOS)
livtet-cli           CLI binary
livtet-test-utils    Shared test helpers
```

The dependency graph is acyclic. `livtet-data` is the only crate that
declares a direct `sea-orm` or `sqlx` dependency; everyone else obtains
them through `livtet_data::orm` and `livtet_data::sql`. `livtet-types`
is the one sanctioned exception — it sits below `livtet-data` and uses
sea-orm derive paths.

```
livtet-types    (leaf; direct sea-orm dep)
   ^
   |
livtet-data     (DB owner)
   ^
   |
livtet-covers   --> livtet-data
livtet-search   --> livtet-data, livtet-types
livtet-core     --> livtet-data, livtet-covers, livtet-search, livtet-types

livtet-plugins  (leaf)
livtet-plugins-lua (leaf)

livtet-ffi      --> livtet-core, livtet-data, livtet-sync, livtet-types, livtet-plugins, livtet-plugins-lua

livtet-sync     --> livtet-core, livtet-data
livtet-cli      --> livtet-core, livtet-plugins

livtet-test-utils (test helpers)
```

## Crate map

Each crate has its own `README.md` describing its surface and
responsibilities:

- livtet-data — DB owner; sea-orm + sqlx re-exports, migration runner,
  seed helpers.
- [livtet-types](livtet-types/README.md) — leaf; strong-typed `DbId`,
  `URN`, identifiers, format metadata, device types.
- [livtet-covers](livtet-covers/README.md) — cover provider trait,
  default-source catalog, image storage, on-disk cache.
- [livtet-search](livtet-search/README.md) — Tantivy index, label
  resolver, sea-orm resource lookup glue.
- [livtet-core](livtet-core/README.md) — unison; works, editions,
  identifiers, plugins, sync, OPDS, backup, and the user-agent /
  cover fetcher used by the desktop and mobile apps.
- [livtet-sync](livtet-sync/README.md) — sync engine. Server-side
  change tracking and the client engine that consumes it.
- [livtet-plugins](livtet-plugins/README.md) — out-of-process Lua
  plugin host. Capability interfaces, signing & verification, archive
  install flow, IPC protocol.
- livtet-plugins-lua — Lua plugin stub. Merger target for the former
  `livtet-lua-plugins` and `livtet-lua-stdlib`; not yet implemented.
- [livtet-ffi](livtet-ffi/README.md) — uniffi surface over core. The
  Android and iOS apps bind against this crate.
- [livtet-cli](livtet-cli/README.md) — developer CLI. Sign and
  install plugin archives, dump manifests, generate keys, run the
  search indexer.
- [livtet-test-utils](livtet-test-utils/README.md) — shared test
  helpers used by integration tests across the workspace.

## Conventions

- **sea-orm / sqlx ownership.** `livtet-data` is the only crate that
  declares a direct `sea-orm` or `sqlx` dependency. Other crates obtain
  them through `livtet_data::orm` / `livtet_data::sql`.
  `livtet-types` is the sanctioned exception (it sits below
  `livtet-data` and uses sea-orm derive paths).
- **Public API via `livtet-core`.** Consumers _SHOULD_ depend on
  `livtet-core`, not on the individual sub-crates. `livtet-core`
  re-exports the curated surface (`pub use livtet_data as data;`,
  `pub use livtet_covers as covers;`, `pub use livtet_search as search;`).

## Quick start

See [GETTING_STARTED.md](GETTING_STARTED.md) for the full setup. The
short version:

```bash
mise install
cargo check --workspace --all-targets
cargo test  --workspace
```

## Build commands

```bash
cargo check --workspace --all-targets   # verify every crate compiles
cargo test  --workspace                  # run every crate's tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                         # apply workspace formatting
```

## License

[Mozilla Public License 2.0](LICENSE). Copyright (c) 2026 Jacky Alcine
<yo@jacky.wtf>.

See [CONTRIBUTING.md](CONTRIBUTING.md) for contribution guidelines
(DCO sign-off, AI-assisted-by attribution, scope-prefixed commit
messages).

[1]: https://github.com/olamaelcu/livtet-desktop
[2]: https://github.com/olamaelcu/livtet-mobile
