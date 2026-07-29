# livtet-core

Entities, business logic, and SQLite access layer that every Livtet client (Tauri desktop, Android, iOS, future web) builds on.

## What It Is

The shared heart of the codebase. It owns the SeaORM entity graph, the database connection, the migration runner entry point, the business-logic services (works, editions, identifiers, annotations, reading lists, plugins, sync, OPDS, backup), and the type definitions those services exchange.

Every other Livtet crate either composes `livtet-core` services or talks to the database through it. The Tauri desktop app links it directly; the Android and iOS apps reach it indirectly through `crates/ffi/livtet-ffi`, which re-exports a curated subset of its surface across the FFI boundary.

## Build & Test

```bash
mise run test-rust -p livtet-core
```

## Architecture

- [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) — workspace overview
- [docs/reference/protected_data.md](../../docs/reference/protected_data.md) — data classification
- [docs/reference/adr/0005-dbid-migration.md](../../docs/reference/adr/0005-dbid-migration.md) — primary-key design
- [docs/reference/adr/0019-isbn-newtype.md](../../docs/reference/adr/0019-isbn-newtype.md) — strong-typed identifiers
