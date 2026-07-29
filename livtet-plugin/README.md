# livtet-plugin

The out-of-process Lua plugin host: capability interfaces, signing & verification, archive install flow, and the `livtet-plugin-host-lua` binary that loads a single plugin.

## What It Is

The extensibility seam for third-party Livtet integrations. It defines the typed contribution interfaces plugins implement (link resolvers, metadata sources, OPDS catalogs, annotation importers, cover providers, etc.), the signed-archive install lifecycle, the trust and revocation model, and the IPC protocol the host uses to call into a sandboxed Lua plugin process. The Tauri app and the developer CLI both rely on this crate to discover, load, and manage plugins.

## Build & Test

```bash
mise run test-rust -p livtet-plugin
```

## Architecture

- [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) — workspace overview
- [docs/reference/adr/0007-plugin-infrastructure.md](../../docs/reference/adr/0007-plugin-infrastructure.md) — process model & IPC
- [docs/reference/adr/0008-plugin-signing-repositories.md](../../docs/reference/adr/0008-plugin-signing-repositories.md) — trust model
- [docs/reference/adr/0015-enhanced-plugin-host-logging.md](../../docs/reference/adr/0015-enhanced-plugin-host-logging.md) — host logging
