# livtet-cli

Lean developer CLI.

## Commands

- `livtet reindex [--database PATH] [--index-dir PATH] [--yes] [--force]` — rebuild the Tantivy search index from the local SQLite database. Default mode runs `migrate_to`, which is a no-op when the on-disk schema is current. Pass `--force` to wipe and rebuild unconditionally. Destructive by design: pair with `--yes` and custom `--database` / `--index-dir` to target a non-production database safely.
- `livtet seed [--works N] [--database PATH] [--yes]` — populate local SQLite with demo data (`fake` builds only).
- `livtet path [all|bundle|data|config|logs]` — print canonical app-data directories.
- `livtet editions [list|get|search] ...` — query editions via the search index.

## Build & Test

```bash
mise run test-rust -p livtet-cli
```
