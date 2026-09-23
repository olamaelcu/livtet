# 2. Own the sync client schema and audit triggers in livtet-data migrations

Date: 2026-09-23

## Status

Accepted

## Context

The sync tables had two owners and no single home. `change_log` and `conflicts`
were created by runtime DDL inside the sync crate, while the client-side tables
(device types, pairing, settings) were created by a separate migrator that
existed only on an unpublished branch. The desktop's `livtet-data` shipped only
the business schema and had no `Kind::Client`, so it could not create the sync
tables at all.

## Decision

Move the whole client schema and the audit triggers into `livtet-data`, under a
`client_migration` module with its own `client_migrations` bookkeeping table:

* `m0001_change_log` — `change_log` and `conflicts`.
* `m0002_pairing_tables` — `device_types`, `pairing_statuses`, `paired_devices`,
  `pending_pairings`, seeded from `DeviceType::all()` / `PairingStatus::all()`.
* `m0003_session_tokens` — `paired_devices.session_token` and its unique index.
* `m0004_client_settings` — the `client_settings` key/value table.
* `m0005_sync_triggers` — the 53 audit triggers that write `change_log`.
* `m0006_pending_pairings_device_id` — the remote device id captured at pairing.

`Kind::Client` is added to the migrator, and `run_kinds` runs Business then
Client. The migration table name and the first three migration names are
identical to the unpublished branch, so an already-migrated database is
recognised. Every migration is idempotent: `if_not_exists` / `IF NOT EXISTS` on
creation, `INSERT OR IGNORE` on seeds, catalog-guarded `ALTER` / `DROP`, and
trigger DDL guarded on the presence of its target table. A test runs the
migrator twice against one pool.

The migration owns `change_log`, `conflicts`, and the triggers. `livtet-sync`
contains no DDL and installs nothing; it only reads and writes rows.

## Consequences

### Easier

* One schema owner for the sync tables, with guarded, re-runnable migrations.
* A database migrated by the unpublished branch carries over without replaying
  `m0001`–`m0003`.

### Harder

* Consumers must request `Kind::Client` when connecting; the desktop connects
  with `Business` and `Client`.
* Two processes (the desktop app and its daemon) may open the same SQLite file;
  WAL with `busy_timeout = 5000` covers it, but writes serialize.
