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
- A whole-row `reading_progress` apply may not erase notes. When the local row
  has a non-empty `annotations` array and the incoming row carries a different
  or empty array (or a DELETE), record a conflict and leave the row untouched,
  even on the authoritative offer path. The guard reads the row directly, so a
  progress row with no local change-log entry cannot bypass the hash check.
- Until Stage B merges notes, `reading_progress` conflicts are read-only in the
  UI with an explicit “notes merge in the next update”; Keep/Use/Merge actions
  are disabled rather than pretending to resolve them.

## Consequences

- Repeated sync becomes idempotent; conflict counts reflect real divergence.
- Notes survive a pull that would otherwise replace the whole progress row;
  progress updates with equal notes still apply normally.
- The hash covers the whole payload, so any field change is a conflict until a
  field-level merge exists.
