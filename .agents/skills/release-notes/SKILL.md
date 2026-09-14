---
name: release-notes
description: Draft CHANGELOG entries and GitHub release notes from tags, commits, and PRs.
---

# Livtet Release Notes

Write release notes for Livtet from git state, tags, changelog entries, and PRs.
Paths below are relative to the repository root.

## Release And Evidence

- Determine the target release from the user's request, the latest git tag, or
  the version in `Cargo.toml`. Stop and explain if the target release cannot
  be determined.
- Compare against the previous stable tag. Include the full range since that
  tag, not only changes since the last prerelease or commit.
- Check commits, PR descriptions, and existing `CHANGELOG.md` sections (root or
  per-crate). Include later `fix:` commits not yet in the changelog. Check docs
  and code when the effect on readers is unclear.

## Existing Text

Treat existing `CHANGELOG.md` entries and GitHub release bodies as manually
edited. Preserve their wording, order, and unrelated content. Make targeted
edits when the user requests an update. Without that request, put proposed
changes in a new draft for review instead of overwriting. Restructure an
existing entry only when the user requests it.

## Content

- Write for users, integrators (desktop/mobile via `livtet-ffi`), and operators.
  Describe what changes for them. Omit internal refactors, test work, CI, and
  generated-file changes unless they change visible behavior or require reader action.
- Rank changes by their effect on readers. Put the largest changes first.
  Describe each change once, in its final form.
- Put compatibility requirements and migration actions in an `Upgrade Notes`
  section. Storage, sync-protocol, and FFI-surface changes are not features
  unless they define a separately intended capability. Do not invent a feature
  from an incidental effect.
- Include every bug fix relevant to readers of the previous release. Omit fixes
  to behavior introduced and repaired within the same release cycle.
- Use short, factual sentences and concrete subjects. Avoid marketing claims,
  descriptions of the notes themselves, emojis, PR lists, and copied changelog text.
- Describe changes at the product level. Do not list every affected crate,
  module, or function. Keep each item understandable on its own.
- For performance changes, name the visible benefit (startup, import, search,
  sync, memory use).

## Verification

- Re-check the tag range and changelog diff before presenting the draft.
- Report which tags, commits, and PRs were used as evidence.
