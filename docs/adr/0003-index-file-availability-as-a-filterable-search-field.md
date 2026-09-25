# 3. Index file availability as a filterable search field

Date: 2026-09-24

## Status

Accepted

## Context

Editions can exist without a local file: catalog metadata imported from a
source such as an OPDS collection, a work referenced remotely, or an edition
whose `digital_inventory` row was removed. The search index already recorded
this as a `has_file: bool` field (set from `digital_inventory` during reindex
and single-doc writes) and surfaced it on `SearchHit`, but the field was
`STORED | FAST` only — the query layer had no way to filter on it. Consumers
that want to separate on-disk editions from virtual ones (starting with the
desktop library) would otherwise have to post-filter pages, breaking counts
and pagination.

## Decision

Promote file availability to a first-class filter dimension on the search
index:

1. **Domain.** Add `has_file: Option<bool>` to `livtet_types::WorkFilters`.
   `None` applies no constraint; `Some(true)` keeps only editions with a
   `digital_inventory` row; `Some(false)` keeps only those without one. The
   field flows through the UniFFI `Record` and every `WorkFiltersQuery`.
2. **Schema.** Mark `fields::HAS_FILE` `STORED | FAST | INDEXED` and bump
   `SCHEMA_VERSION` to 4. `SearchIndex::migrate_to` rebuilds the index on the
   next open, so the change is applied without an explicit reindex command.
3. **Query.** `WorkFiltersQuery::build_query` emits a boolean `TermQuery`
   (`Term::from_field_bool`) for `Some(_)`, and counts `has_file` in its
   `has_filters` guard so a has_file-only filter does not fall through to
   `AllQuery`.
4. **Out of scope.** The FFI's SQL-backed `list_works_filtered` /
   `count_works_filtered` do not implement `has_file` yet and reject it
   (`InvalidInput`) to stay fail-closed.

## Consequences

### Easier

* Availability is filterable server-side, so pagination, counts, and
  select-all stay correct.
* The same `WorkFilters` contract serves the desktop, CLI, and any consumer
  that resolves format/language labels, with no bespoke query path.

### Harder

* Adding a field to `WorkFilters` is a breaking change for exhaustive struct
  literals in downstream repositories (desktop, opds), which must set
  `has_file` to keep compiling.
* Every consumer reindexes once on its next launch because of the schema
  version bump.

### Follow-up

* Implement `has_file` in the FFI SQL filter path (join `digital_inventory`)
  or keep it permanently out of the mobile filter surface.
* Consider extending availability beyond a boolean if remote provenance
  (OPDS, plugin source) needs to be distinguishable from "no file".
