# Notes converge by id with tombstones

Date: 2026-11-08

Status: Accepted.

## Context

Notes are one JSON array on `reading_progress.annotations`. Sync applies that
array as a whole, so two devices that each wrote a note on the same edition
overwrite one another; deletes are simply an element disappearing, which a
stale peer can resurrect. There is no per-note history to merge.

## Decision

- Each note entry carries `updatedAt`, `updatedBy`, and, when deleted,
  `deletedAt`. Existing entries are backfilled with `updatedAt` at the epoch and
  order by note id.
- A delete keeps the full original entry and adds tombstone metadata. Tombstones
  are filtered by `reading_annotations` and by every reader; they are retained
  indefinitely until per-peer pull watermarks can make collection safe.
- Merge unions notes by id. A tombstone is final: no later edit resurrects that
  id. Otherwise the greatest `(updatedAt, updatedBy, id)` wins.
- `reading_progress` gains `updated_at` and `updated_by`. Progress, unit, and
  last location come from the greatest `(updatedAt, updatedBy)`; accumulated
  reading time takes the maximum; the row's `created_at` is preserved.
- This is Stage B of the sync repair; Stage A only deduplicates conflicts and
  leaves note differences read-only.

## Consequences

- Both devices can read and write offline; the union converges after sync.
- A tombstone may outlive the note forever; that is the price of not tracking
  per-peer watermarks yet.
- Readers must ignore tombstoned entries even though they remain in the array.
