# Livtet Glossary

One entry per concept. Define Livtet-specific meanings; omit standard
technical definitions, code signatures, and tutorials.

Entry style: `**Term** — short definition. [source]` where source links to
the ADR, crate README, or docs that explain the concept. Expand abbreviations
on first use. Place foundational terms before terms that depend on them,
not alphabetically.

Section order: Data, Domain, Interfaces. Each term lives in exactly one section.

## Data

**DbId** — Universal primary key for every row; Universally Unique Lexicographically Sortable Identifier (ULID) wrapper stored as 16-byte binary, rendered as 26-character string. [livtet-types](../livtet-types/README.md)

**URN** — Uniform Resource Name in canonical `urn:<scheme>:<value>` form; the wire format for all external and Livtet-internal identifiers. [livtet-types/src/urn.rs](../livtet-types/src/urn.rs)

**Identifier** — One external identity attached to a Work or Edition; an IdentifierKind plus URN pair whose URN string is the canonical wire format. [livtet-types/src/identifier.rs](../livtet-types/src/identifier.rs)

**ISBN** — International Standard Book Number validated and normalized to canonical 13-digit form; accepted input includes ISBN-10, hyphens, and prefixes. [livtet-types/src/isbn.rs](../livtet-types/src/isbn.rs)

**SharedState** — Process-global SQLite pool initialized once and shared by all clients; enforces Write-Ahead Logging (WAL) with foreign keys on. [livtet-data/src/state.rs](../livtet-data/src/state.rs)

## Domain

**Work** — Abstract literary creation that owns Editions, contributors, subjects, and reading status. [livtet-core](../livtet-core/README.md)

**Edition** — Concrete publication of a Work with format, language, and publication date; the row inventory, loans, and reading attach to. [livtet-data/src/entities/editions.rs](../livtet-data/src/entities/editions.rs)

**Edition Group** — Named bucket grouping variant Editions of the same Work. [livtet-data/src/entities/edition_groups.rs](../livtet-data/src/entities/edition_groups.rs)

**WorkStatus** — Singleton per-Work reading state (to-read, reading, finished, abandoned, queued, active) stored as a Livtet URN. [livtet-types/src/work_status.rs](../livtet-types/src/work_status.rs)

**Contributor Role** — Functional Requirements for Bibliographic Records (FRBR)-aligned creator role such as author, translator, editor, illustrator, or narrator, stored as a Livtet URN. [livtet-types/src/contributor_role.rs](../livtet-types/src/contributor_role.rs)

## Interfaces

**SearchIndex** — Tantivy full-text index maintained alongside SQLite for ranked search over titles, authors, descriptions, and annotations. [livtet-search](../livtet-search/README.md)

**livtet-cli** — Lean command-line tool for seeding demo data and printing canonical app-data directories. [livtet-cli](../livtet-cli/README.md)
