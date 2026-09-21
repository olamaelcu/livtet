# livtet-types

Cross-crate value types, newtypes, and result enums that flow through services, IPC, and the database. The shared vocabulary the rest of the workspace types against.

## What It Is

The leaf of the dependency graph. It holds ULID wrappers, ISBN/identifier newtypes, FRBR-aligned enums, time and money types, and a few strongly-typed result markers that other crates need to compile against without pulling in the full database stack. Most other Livtet crates depend on it; it depends only on serde, specta, and the time/ulid ecosystem.

## UniFFI

With the `uniffi` feature the core types carry UniFFI derives: simple enums become `uniffi::Enum`, records derive `uniffi::Record`, and the validated string newtypes (`DbId`, `DiskPath`, `Isbn`, `Urn`, `Identifier`) register fail-closed `custom_type!` converters. `livtet-ffi` builds on this; nothing else needs the feature.

## Build & Test

```bash
mise run test-rust -p livtet-types
```
