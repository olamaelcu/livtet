# livtet-cover

The cover-image subsystem: provider trait, default-source catalog, image storage layout, and the cache that backs the cover-picker UI.

## What It Is

The piece of Livtet that turns a "find me a cover for this ISBN / Open Library work / Wikidata ID" request into a stored image. It defines the provider trait plugins implement to contribute new cover sources, the priority and dedup logic that picks the winner, and the on-disk cache that downstream consumers read from. Both the desktop UI and the FFI surface go through this crate when they need a cover.

## Build & Test

```bash
mise run test-rust -p livtet-cover
```

## Architecture

- [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) — workspace overview
- [docs/plans/in-progress/multi-provider-cover-system.md](../../docs/plans/in-progress/multi-provider-cover-system.md) — current work
