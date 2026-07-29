# livtet-backup

Snapshot and restore primitives for the Livtet database. The portable archive format that the desktop app, the CLI, and the sync backup subsystem all share.

## What It Is

Defines a directory-shaped archive of the SQLite database plus its file-bearing tables (book files, cover images) with enough metadata to restore on a different device. The actual on-disk layout, integrity hashing, and the trait surface that pluggable storage backends implement live here. Higher-level crates (`livtet-tauri` for desktop, `livtet-sync-backup` for the LAN sync path, `livtet-cli` for the developer tool) call into this for the actual encode/decode work.

## Build & Test

```bash
mise run test-rust -p livtet-backup
```

## Architecture

- [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) — workspace overview
- [docs/reference/adr/0012-book-file-sync-pr7-9.md](../../docs/reference/adr/0012-book-file-sync-pr7-9.md) — end-to-end file sync
