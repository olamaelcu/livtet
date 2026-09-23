# 1. Split sync into domain, transport, and daemon crates

Date: 2026-09-23

## Status

Accepted

## Context

The sync implementation combined three concerns in one crate: the local
database engine, an HTTP client built on `reqwest`, and an HTTP server built on
`poem`. Its feature flags were nominal — the modules were exposed
unconditionally, so `--no-default-features --features client` could not build —
and `reqwest` was a hard dependency. A consumer that wanted only the engine to
read and write `change_log` still compiled an HTTP stack, and no crate could be
depended on without it.

## Decision

Split the crate along its existing seams:

1. **`livtet-sync` — domain.** `SyncEngine` over `change_log` / `conflicts`,
   the wire DTOs, the syncable-entity registry, and `SyncError`. No HTTP
   dependency.
2. **`livtet-sync-http` — transport.** The protocol's HTTP layer, generic over
   the HTTP library. The generic layer (the `SyncHttpClient` trait, wire
   request types, route constants, `SyncHttpError`, and `SyncSession<C>`) has
   zero HTTP dependencies and builds with `--no-default-features`. Backends are
   off by default: `reqwest` enables `ReqwestHttpClient`; `poem` enables
   `make_sync_routes`, `SyncServerInstance`, and the pairing fan-out.
3. **`livtet-sync-server` — daemon.** `pub fn run(ServerConfig)` plus a binary,
   hosting the poem server and a JSON-RPC 2.0 NDJSON control channel on stdio.
   This is the crate the desktop sidecar calls.

## Consequences

### Easier

* The engine can be depended on without an HTTP stack; the transport backend is
  swappable and each feature compiles standalone.
* The desktop and the mobile client can share the same crates.

### Harder

* One more crate boundary to maintain, and consumers must select the right
  backend features (`reqwest`, `poem`).
* `livtet-sync-http`'s generic layer still depends on `livtet-data`, because
  `SyncSession` names the pooled database connection type.

### Follow-up

* Session-token validation on the `/sync/*` routes (see ADR 2 for the schema).
* `Conflict::id` is a `DbId` while the `conflicts` primary key is an `INTEGER`;
  `list_conflicts` and `resolve_conflict` disagree and need reconciling.
