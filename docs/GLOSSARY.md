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

**Note** — Reader-authored mark attached to a passage or position, carrying optional text, quote, and colour. _Avoid_: annotation, highlight. [ADR-0010](adr/0010-notes-converge-by-id-with-tombstones.md)

**Tombstone** — A deleted Note retained with its deletion metadata so the deletion survives sync and cannot be resurrected by a stale peer. _Avoid_: soft delete, delete marker. [ADR-0010](adr/0010-notes-converge-by-id-with-tombstones.md)

## Interfaces

**Pairing** — Trust relationship between two devices that lets each sync with the other; established by one claim and one approval. [ADR-0009](adr/0009-pairing-mints-both-tokens.md)

**Pairing Claim** — One device's request to pair with another, identified by the peer's node identity. _Avoid_: pairing request, invite. [ADR-0009](adr/0009-pairing-mints-both-tokens.md)

**Pairing Approval** — The human decision on a Pairing Claim that creates the Paired Device relationship and mints both credentials. [ADR-0009](adr/0009-pairing-mints-both-tokens.md)

**Paired Device** — A peer this device holds a Pairing with. [ADR-0009](adr/0009-pairing-mints-both-tokens.md)

**Inbound Token** — Credential this device issued to a Paired Device, authorizing that peer's requests to this device. _Avoid_: session token. [ADR-0009](adr/0009-pairing-mints-both-tokens.md)

**Outbound Token** — Credential a Paired Device issued to this device, presented when this device contacts that peer. _Avoid_: peer session token. [ADR-0009](adr/0009-pairing-mints-both-tokens.md)

**One-way Pairing** — Legacy state where only one direction holds credentials; repaired by revoking and pairing again. [ADR-0009](adr/0009-pairing-mints-both-tokens.md)

**SearchIndex** — Tantivy full-text index maintained alongside SQLite for ranked search over titles, authors, descriptions, and annotations. [livtet-search](../livtet-search/README.md)

**livtet-cli** — Lean command-line tool for seeding demo data and printing canonical app-data directories. [livtet-cli](../livtet-cli/README.md)
