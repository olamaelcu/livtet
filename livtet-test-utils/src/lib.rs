//! Test utilities shared across the workspace. Each fixture family sits
//! behind its own feature so a consumer pays only for what it uses: `sync`
//! for the sync crates (see [`sync`]), `cli` for the front-end crates (see
//! [`cli`]). With neither enabled this crate is empty. Extend as new tests
//! need it.

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "sync")]
pub mod sync;
