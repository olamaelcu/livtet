# Getting Started

Setup for working on the Livtet Core Rust workspace. By the end of this
you will have a working build of every crate, a green
`cargo test --workspace`, and a clean `cargo clippy`.

## Prerequisites

| Tool | Version | Why |
| --- | --- | --- |
| [mise](https://mise.jxa.ch) | latest | Pins the Rust toolchain per `mise.toml` |
| Rust | 1.97.0 | Workspace `rust-version` and the pinned toolchain in `mise.toml` |
| C toolchain | any | `sea-orm-migration`, `reqwest`'s rustls bindings, and the Tantivy native deps all link C |
| `pkg-config` | any | Linux only; needed for native dep discovery |

You almost certainly already have mise. If you don't:

```bash
curl https://mise.run | sh
```

## Step 1 — Trust the repo and install tools

```bash
cd /path/to/livtet-ecosystem/core
mise trust mise.toml
mise install
```

`mise install` reads `[tools]` from `mise.toml` and pins Rust 1.97.0.
Verify with `mise current`.

## Step 2 — Verify the workspace builds

```bash
cargo check --workspace --all-targets
```

This exercises every crate's `lib`, `bin`, example, integration test,
and benchmark target. Expect a clean compile. The build is large the
first time (a few minutes on a fast machine) because every dependency
is compiled from source — `sea-orm`, `sqlx`, `tantivy`, `uniffi`,
`reqwest` with rustls, and the rest.

## Step 3 — Run the test suite

```bash
cargo test --workspace
```

Most crates run their tests in-process. The notable exceptions:

- `livtet-data` tests use a per-test temporary SQLite database via
  `livtet_data::test_db::TestDb` and run migrations against it before
  each test.
- `livtet-covers`, `livtet-plugins`, and `livtet-ffi` integration tests
  exercise HTTP / filesystem fixtures and may take longer to compile
  and run than the unit tests.

You don't need an external database, message broker, or network
service. Everything is in-process.

## Step 4 — Lint and format

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Clippy runs with `unsafe_code = "deny"` (workspace-wide). Don't add
`unsafe` without a clear justification and a `// SAFETY:` comment.

## Step 5 — Pick a crate to work in

Each crate has its own `README.md` under `livtet-<name>/README.md` that
explains what it owns and what depends on it. The dependency graph:

```
livtet-types    (leaf; direct sea-orm dep — the one sanctioned exception)
   ^
   |
livtet-data     (DB owner; re-exports sea-orm as `orm` and sqlx as `sql`)
   ^
   |
livtet-covers   --> livtet-data
livtet-search   --> livtet-data, livtet-types
livtet-core     --> livtet-data, livtet-covers, livtet-search, livtet-types (unison)
livtet-plugins  (leaf; plugin runtime)
livtet-plugins-lua  (leaf; lua stub — not yet implemented)
livtet-ffi      --> livtet-core, livtet-data, livtet-sync, livtet-types, livtet-plugins, livtet-plugins-lua
livtet-sync     --> livtet-core, livtet-data
livtet-cli      --> livtet-core, livtet-plugins
livtet-test-utils (test-only helpers)
```

### Adding a new dependency to a crate

Three rules:

1. **Never declare a direct `sea-orm` or `sqlx` dependency** outside
   `livtet-data` and `livtet-types`. Use `livtet_data::orm::...` and
   `livtet_data::sql::...` instead. The macro hygiene emits
   `::sea_orm::...` paths in derived code; the only safe place for
   that is `livtet-types`.
2. New workspace dependencies go in the root `Cargo.toml`'s
   `[workspace.dependencies]` table. Consumers reference them with
   `{ workspace = true }` — never re-declare a version inline.
3. The workspace uses `resolver = "3"`. Edition 2024 features are on,
   including `unsafe_code = "deny"` at the workspace level.

## What's next

- See [CONTRIBUTING.md](CONTRIBUTING.md) for commit-message conventions,
  DCO sign-off, and AI-assisted-by attribution.
- See [README.md](README.md) for the workspace overview and the crate
  map.
- A workspace `ARCHITECTURE.md` does not yet exist. When it does, it
  will live at `docs/ARCHITECTURE.md`.

## Common gotchas

- **`cargo check -p <single-crate>` fails with `tokio::sync::Mutex`
  missing.** Some test crates reference `tokio` features that aren't
  enabled by the workspace's default `tokio = "..."` entry. Run
  `cargo check --workspace --all-targets` instead — feature unification
  picks up the missing features from sibling crates.
- **`cargo check --all-features` fails on `livtet-cli/src/seed.rs`.**
  That's a known TBD — the seed module uses `?` with error types that
  don't yet have `From` impls in `CliError`. The default-features build
  is clean; `--all-features` will surface this until the seed module
  is implemented properly.
- **You added a `use sea_orm::...` somewhere outside `livtet-data` and
  `livtet-types`.** Use `livtet_data::orm::...` instead. The DB owner
  is `livtet-data`; only `livtet-types` (the leaf) gets a direct
  `sea-orm` dep.
- **You added a `use sqlx::...` somewhere outside `livtet-data`.** Same
  rule — use `livtet_data::sql::...`.
- **`./target` is taking many gigabytes.** Run `cargo clean` from the
  workspace root. The workspace has eleven crates, each with its own
  dependency graph, and SeaORM / Tantivy / uniffi pull in large native
  deps.
- **`mise install` complains about a hash mismatch.** `mise.lock` is
  the source of truth for the pinned toolchain. Update it with
  `mise lock` if you intentionally bump a version.
