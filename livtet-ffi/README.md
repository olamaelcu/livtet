# livtet-ffi

UniFFI surface over the livtet workspace, consumed by the Android and iOS
apps in the `mobile` repository. Proc-macro based (no UDL); all functions
and DTOs are declared in this crate, while domain primitives
(`DbId`, `Isbn`, `Urn`, enums, ...) are exported directly from
`livtet-types` behind its `uniffi` feature.

## Generating bindings

From the workspace root, after `cargo build -p livtet-ffi`:

```sh
BINDINGS=target/debug/liblivtet_ffi.so
cargo run -p livtet-ffi --bin uniffi-bindgen --features cli -- \
  generate --library "$BINDINGS" --language kotlin --out-dir bindings/kotlin
cargo run -p livtet-ffi --bin uniffi-bindgen --features cli -- \
  generate --library "$BINDINGS" --language swift --out-dir bindings/swift
```

Foreign-side config lives in [`uniffi.toml`](uniffi.toml) (Kotlin package
name, immutable records, Swift `Codable` conformance).
