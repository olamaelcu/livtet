# livtet-ffi

The UniFFI bridge between the Livtet Rust core and the Android and iOS apps. The single crate both mobile platforms depend on to reach into the Rust database and service layer.

## What It Is

The mobile-facing surface of Livtet. It compiles to a static library per platform (Android `.so` per ABI; iOS fat static lib), exposes a curated subset of the Rust core as UniFFI-exported functions, and the generated Kotlin (Android) and Swift (iOS) bindings give each app a typed bridge into the same data and services the desktop uses. The Android Gradle plugin and the iOS xcframework build both pull from this crate.

## Build & Test

```bash
mise run test-rust -p livtet-ffi
```
