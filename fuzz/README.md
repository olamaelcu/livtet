# livtet-fuzz

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) / libFuzzer harnesses for the untrusted-input parsers in
`livtet-types`. These parsers accept arbitrary user- and catalog-supplied
strings (saved-search DSL, ISBNs, published dates, placeholders), so a
malformed input must only ever produce a typed error — never a panic,
stack overflow, or hang.

This crate is **excluded from the stable workspace** (`exclude = ["fuzz"]`
in the root `Cargo.toml`) because libFuzzer requires nightly. It is built
and run out of band.

## Running

```sh
rustup toolchain install nightly
cargo install cargo-fuzz        # once
cargo +nightly fuzz run parse_dsl
cargo +nightly fuzz run parse_scalars
```

## Targets

- **`parse_dsl`** — `saved_search::parse_user_dsl`, the highest-risk target
  (it feeds tantivy's recursive-descent grammar). The `MAX_DSL_LEN` /
  `MAX_DSL_DEPTH` guards in `parse_user_dsl` exist to keep this safe.
- **`parse_scalars`** — `Isbn::parse`, `PublishedDate::parse`, and
  `placeholder::parse_placeholder`.

## Harness-first

The targets are deliberately one-liners that call the real parsers, so the
same functions are exercised three ways:

1. here, under coverage-guided fuzzing (nightly);
2. by the regression tests for known crashers — e.g. the deep-nesting case
   in `livtet-types/src/search/saved_search.rs` — which run on stable in CI;
3. by the round-trip property tests in
   `livtet-types/tests/roundtrip_properties.rs`.

When fuzzing turns up a new crasher, add the minimized input both to this
crate's `corpus/<target>/` and as a stable regression test next to the
parser, so CI protects against it without needing nightly.
