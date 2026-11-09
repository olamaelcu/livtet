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
- Stage B ([ADR-0010](0010-notes-converge-by-id-with-tombstones.md)) merges
  `reading_progress` rows on apply, so they no longer produce a conflict;
  any rows left from before the upgrade can still be resolved in the UI.

## Consequences

- Repeated sync becomes idempotent; conflict counts reflect real divergence.
- Notes survive a pull that would otherwise replace the whole progress row;
  progress updates with equal notes still apply normally.
- The hash covers the whole payload; for `reading_progress` the Stage B merge
  runs first, so note and progress differences converge instead of conflicting.
