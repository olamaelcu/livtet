# livtet-types

Cross-crate value types, newtypes, and result enums that flow through services, IPC, and the database. The shared vocabulary the rest of the workspace types against.

## What It Is

The leaf of the dependency graph. It holds ULID wrappers, ISBN/identifier newtypes, FRBR-aligned enums, time and money types, and a few strongly-typed result markers that other crates need to compile against without pulling in the full database stack. Most other Livtet crates depend on it; it depends only on serde, specta, and the time/ulid ecosystem.

## Build & Test

```bash
mise run test-rust -p livtet-types
```
