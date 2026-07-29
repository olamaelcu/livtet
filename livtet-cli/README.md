# livtet-cli

The `livtet-cli` developer tool. Headless command-line access to plugin signing, install/inspect, and other host operations that don't need the desktop GUI.

## What It Is

A `clap`-driven binary that wraps the same library functions the Tauri app uses: sign a plugin archive, verify a signature, install a plugin into a target install root, dump a manifest, generate keys. The CLI exists so plugin authors and CI pipelines can exercise the plugin toolchain without booting the desktop app. It links against the same `livtet-plugin` core the GUI uses, so the CLI is the canonical reference for what the install lifecycle does.

## Build & Test

```bash
mise run test-rust -p livtet-cli
```

## Architecture

- [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md) — workspace overview
- [docs/reference/adr/0008-plugin-signing-repositories.md](../../docs/reference/adr/0008-plugin-signing-repositories.md) — signing workflow
