# Detect sync conflicts by payload hash

Date: 2026-11-08

Status: Accepted.

## Context

`change_log.version` is `MAX(version)+1` per device, so comparing it across
devices is meaningless; the engine recorded a conflict whenever the local
counter happened to be greater, and identical payloads could conflict. Conflict
rows were inserted unconditionally, so repeated pulls created thousands of
duplicates. The desktop “Merge” action only stored bookkeeping and never
replayed anything.

## Decision

- Compare canonical JSON payload hashes on both push and pull. Equal hashes are
  a no-op; a different hash is the only condition that can conflict.
- Deduplicate conflicts by entity and the pair of payload hashes. Repeated
  pulls or pushes update one row instead of inserting another.
- Delete the existing conflict backlog once; it is duplicated and cannot be
  replayed.
- Until Stage B merges notes, `reading_progress` conflicts are read-only in the
  UI with an explicit “notes merge in the next update”; Keep/Use/Merge actions
  are disabled rather than pretending to resolve them.

## Consequences

- Repeated sync becomes idempotent; conflict counts reflect real divergence.
- The hash covers the whole payload, so any field change is a conflict until a
  field-level merge exists.
