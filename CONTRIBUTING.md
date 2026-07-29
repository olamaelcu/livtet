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
