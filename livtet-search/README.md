# livtet-search

Tantivy-backed full-text search index over the library: titles, authors, descriptions, and user annotations.

## What It Is

Maintains a Tantivy index alongside the SQLite database so that library queries can return ranked results in milliseconds. It owns the schema mapping (which entity columns map to which indexed fields), the index-on-disk layout, and the rebuild / incremental-update flow. Higher crates — the Tauri command surface and the FFI bridge — call into it from the search box; the OPDS server can also use it for catalog-wide search.

## Build & Test

```bash
mise run test-rust -p livtet-search
```

## Architecture

- [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) — workspace overview
