# livtet-cli

Lean developer CLI: `seed` (with `fake` feature) and `path` only.

## Commands

- `livtet seed [--works N] [--database PATH] [--yes]` — populate local SQLite with demo data (`fake` builds only).
- `livtet path [all|bundle|data|config|logs]` — print canonical app-data directories.

## Build & Test

```bash
mise run test-rust -p livtet-cli
```
