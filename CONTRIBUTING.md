# Contributing

Quick notes:

- Tool dependencies are managed by [mise](https://mise.jxa.ch). See
  [mise.toml](mise.toml).
- The workspace is a standard Cargo workspace. There is no project task
  runner — `cargo` and `rustc` are invoked directly. `cargo` invocations
  in this guide assume the workspace root.
- See [GETTING_STARTED.md](GETTING_STARTED.md) for setup and
  [README.md](README.md) for the workspace overview and crate map.

## New Contributor Guide

This repo is a single Cargo workspace with eleven crates. The list
below is the shortest path from "just cloned" to "I have an idea where
to make a change."

### Prerequisites

- [mise](https://mise.jxa.ch) — pins the Rust toolchain per
  [mise.toml](mise.toml).
- A C toolchain. `sea-orm-migration`, `reqwest`'s rustls bindings, and
  the Tantivy native deps all link C. Linux: `build-essential` plus
  `pkg-config`. macOS: Xcode Command Line Tools.
- A POSIX shell. The tests don't depend on shell features, but the
  release script (when we add one) will.

### Repo layout

```
livtet-data          DB owner (sea-orm, sqlx, migrator, seed, error)
livtet-types         Leaf type definitions (DbId, URN, identifiers, format metadata)
livtet-covers        Cover subsystem (provider trait, cache, fetcher)
livtet-search        Search index (Tantivy)
livtet-core          Unison crate (data + covers + search + types)
livtet-sync          Sync engine (server + client + change log)
livtet-plugins       Plugin runtime (host, signing, archives, lua)
livtet-plugins-lua   Lua plugin stub (merger target; not yet implemented)
livtet-ffi           FFI surface over core (uniffi — Android, iOS)
livtet-cli           CLI binary
livtet-test-utils    Shared test helpers
```

The dependency graph is acyclic. `livtet-data` is the only crate that
declares a direct `sea-orm` or `sqlx` dependency; everyone else obtains
them through `livtet_data::orm` and `livtet_data::sql`. `livtet-types`
is the one sanctioned exception — it sits below `livtet-data` and uses
sea-orm derive paths.

### Where to find things

- Each crate has its own `README.md` explaining what it owns and what
  depends on it. Start with [livtet-core/README.md](livtet-core/README.md)
  for the unison crate that wires the rest together.
- A workspace overview does not yet exist. When it does, it will live
  at `docs/ARCHITECTURE.md`.

### Before you open a PR

- Read [AI Disclosure](#ai-disclosure) below.
- Make sure you can run `cargo check --workspace --all-targets`,
  `cargo test --workspace`, and `cargo clippy --workspace --all-targets
  -- -D warnings` from the workspace root.
- Keep PRs focused. One logical change per PR. If you find a second
  thing to fix while writing the first, open it as a separate PR.

### Coverage

`mise run coverage` measures line and region coverage with
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov). It runs
the suite once, prints a per-file summary, and writes a browsable
report to `target/llvm-cov/html/index.html`.

**The floor is 90% workspace-wide line coverage.** `mise run
coverage-check` fails below it. The floor is deliberately workspace-wide
rather than per-crate: a handful of files are dominated by process
lifecycle — `livtet-sync-server`'s `daemon.rs` most of all, with its
socket binding, signal handling and shutdown — where a test that
*executes* the lines verifies almost nothing, and the harness to reach
them costs more than it returns. Holding every crate to 90%
independently would force that work; holding the workspace to it does
not. If you drop a crate well below the floor, say why in the PR.

Some things the number does not include:

- Binary entrypoints (`*/src/main.rs`) and the `livtet-test-utils`
  crate. The `main.rs` wrappers are runtime glue — arg parsing, daemon
  bootstrap, process exit — whose logic is tested in the libraries they
  call; `livtet-test-utils` is fixtures, fault injection, and snapshot /
  sync doubles that serve *other* crates' tests, the same class as the
  scaffolding below. Both are listed in `coverage-ignore.txt`.
- Test-data scaffolding: `livtet-types/src/fake.rs`,
  `livtet-data/src/seed.rs`, and the `#[cfg(feature = "fake")] impl
  fake::Dummy` blocks in `livtet-data/src/{entities,client_entities}`.
  These generate fixtures for *other* crates' tests, so nothing in a
  coverage run ever executes them and no test of them would assert
  anything about behaviour. The exclusion patterns live in
  `coverage-ignore.txt` (one extended-regex fragment per line; blank and
  `#` lines ignored) and are applied via `--ignore-filename-regex` by all
  three coverage tasks — to exclude a new file, add a line there rather
  than editing `mise.toml`. Every excluded file is 0% covered, so nothing
  covered is being hidden.
- `livtet-ffi` and `livtet-ruby`. The uniffi bindings contract test is
  slow and the Ruby crate is a `magnus` cdylib exercised from
  `livtet-ruby/test`, so neither is worth instrumenting from here.
- Doctests. `cargo llvm-cov --doctests` still requires nightly.
  `mise run test` keeps running them; they just do not contribute to
  the reported coverage.

Instrumented builds use their own `target/llvm-cov-target` directory,
so a coverage run will not invalidate your normal `cargo` build cache.
Expect the first run to rebuild the workspace from scratch.

**Quoting a number?** Use `mise run coverage-clean`. Deleting or renaming
a source file leaves its object behind in `target/llvm-cov-target`, and
llvm-cov keeps reporting the vanished file's lines as uncovered.
`cargo llvm-cov clean` will not remove them — that directory has no
`CACHEDIR.TAG`, so `cargo clean` refuses it and exits non-zero — so
`coverage-clean` deletes the directory outright and rebuilds. The plain
`coverage` task is the fast one for "did the code I just wrote get
exercised?"; it is not trustworthy for a baseline or a trend.

### Mutation testing

`mise run mutants` injects bugs into `livtet-types` and reports which
ones no test caught. Coverage tells you a line ran; this tells you a
test would notice if it broke. Worth running after adding table-driven
tests over large `match` statements, which can execute every arm while
asserting almost nothing. Scope other crates with `cargo mutants -p
<crate>`; annotate genuinely untestable code with `#[mutants::skip]`.

Shared settings live in `.cargo/mutants.toml`: it excludes generated and
test-support code (mirroring the coverage exclusions) and gives the slow
DB/Tantivy tests timeout headroom. Modules behind a cargo feature (e.g.
`livtet-types`'s `search`) don't build under a plain per-crate run and show
as "unviable" — pass the feature through, e.g. `cargo mutants -p
livtet-types -- --features search,uniffi`.

### Fuzzing

The parsers that take untrusted strings — the saved-search DSL, ISBNs,
published dates, placeholders — are fuzzed by the `fuzz/` crate (libFuzzer
via [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)). Because libFuzzer
needs nightly, that crate is excluded
from the stable workspace and run out of band:

```sh
cargo +nightly fuzz run parse_dsl       # saved-search DSL
cargo +nightly fuzz run parse_scalars   # ISBN / date / placeholder
```

Harnesses are one-liners that call the real parser, so the same function is
also covered by stable regression tests (known crashers live next to the
parser — see the deep-nesting case in `search/saved_search.rs`) and by the
property tests below. When fuzzing finds a new crasher, add the minimized
input to both `fuzz/corpus/<target>/` and a stable regression test.

### Property-based tests

`proptest` asserts general laws over a wide input space where example tests
only pin cases — `parse ∘ display` identity, serde round-trips, ISBN
canonicalisation idempotence, id byte/string round-trips. The baseline is
`livtet-types/tests/roundtrip_properties.rs`; add invariants there (or
inline, as `reading_length.rs` does) when a type has a round-trip or
idempotence law worth guarding.

## AI Disclosure

**Important**: If you are using **any kind of AI assistance** to
contribute to this project, it MUST be disclosed in the pull request.

### Requirements

When using AI tools while contributing to this project, you must:

- **Disclose in PR description**: Mention if AI tools were used and
  to what extent.
- **Indicate scope**: Clarify how much AI assistance was involved
  (e.g., "AI helped with boilerplate" vs "AI wrote core logic").
- **Commit attribution**: Add a model-specific trailer to commits
  (see [Commit attribution](#commit-attribution) below).

### Example PR description

> This PR adds the `livtet_search::label_resolver::LabelResolver::resolve`
> cache. Claude assisted with the trait shape and the doc comments; the
> actual LRU eviction logic was hand-written (~80% AI assistance).

### Why this matters

Disclosing AI usage is a **matter of respect for reviewers and
maintainers**. It allows us to:

- Apply appropriate scrutiny to AI-generated code.
- Understand the context and limitations of the contribution.
- Provide better feedback based on how the code was written.

We support and encourage AI-assisted development, but transparency is
essential for maintaining trust in the project.

## Commit attribution

This repo does not (yet) install pre-commit hooks. Until a project-local
hook setup lands, contributors are responsible for attribution manually.

### `Signed-off-by:` (DCO)

Every commit must include a `Signed-off-by: Name <email>` line
certifying you have the right to submit the code under the project's
license (see [LICENSE](LICENSE)). Use `git commit -s` to add it
automatically:

```bash
git commit -s -m "livtet-data: extract test_db re-export"
```

If your `user.name` and `user.email` aren't set, git will use the
OS-level account name and refuse to sign off. Set them first:

```bash
git config user.name  "Your Name"
git config user.email "you@example.com"
```

The signed-off-by line in the final commit should match these.

### `Co-Authored-By:` (AI attribution)

For code developed with AI tools, add a model-specific trailer. Mirror
whatever attribution the AI agent appends to its commits. For
human-authored commits, omit it.

```bash
git commit -s -m "livtet-search: cache label lookups" \
    -m "" \
    -m "Co-Authored-By: Claude <noreply@anthropic.com>"
```

The exact format is not enforced (no hook to enforce it in this repo
yet), but reviewers will ask if it's missing on a non-trivial change.

## Commit messages

We use scope-prefixed commit messages (no type prefix). Format:

```
<scope>: <description>
```

- `<scope>` is required, lowercase, and indicates the area of the
  codebase touched. Common scopes: `livtet-data`, `livtet-core`,
  `livtet-covers`, `livtet-search`, `livtet-sync`, `livtet-plugins`,
  `livtet-plugins-lua`, `livtet-ffi`, `livtet-cli`, `livtet-types`,
  `livtet-test-utils`. Workspace-wide scopes: `workspace`, `rust`,
  `cargo`, `mise`, `ci`, `docs`.
- `<description>` is in sentence case, imperative mood, no trailing
  period, and must fit on one line (≤ 72 characters).
- Skip the type prefix (`feat`, `fix`, `chore`, ...). The description
  already conveys intent; the scope is what readers actually search for
  in the log.

Examples:

```
livtet-data: extract test_db re-export
livtet-covers: dedupe fetch error variants
livtet-search: cache label_resolver lookups
livtet-ffi: stub handle_http_request to return empty
workspace: rename livtet-cover -> livtet-covers
rust: format code
```

Merge, revert, and fixup commits bypass this format.

## Reporting issues

Open a GitHub issue. Include:

- A minimal reproduction. For build problems, the exact command and the
  output of `mise --version`, `rustc --version`, and `cargo --version`.
- For a single-crate bug, the crate name and the failing command or
  API call.
- For FFI bugs, the platform (Android / iOS / desktop) and whether it
  reproduces against `livtet-ffi` directly.

The maintainer triage rotates; please be patient while we route your
issue.

## Why this matters

The DCO provides legal clarity about contributions:

- Confirms you have the right to contribute the code under the MPL-2.0
  license this repo publishes under.
- Creates a clear record of contribution provenance.
- Required for projects with corporate contributors.
- AI attribution ensures transparency about development methods.
