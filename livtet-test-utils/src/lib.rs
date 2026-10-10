//! Test utilities shared across the workspace. Each family sits behind its
//! own feature so a consumer pays only for what it uses: `sync` for the sync
//! crates (see [`sync`]), `cli` for the front-end crates (see [`cli`]),
//! `snapshot` for deterministic insta settings (see [`snapshot`]), and
//! `fault` for making queries and opens fail on purpose (see [`fault`]). With
//! none enabled this crate is empty. Extend as new tests need it.

#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "fault")]
pub mod fault;
#[cfg(feature = "snapshot")]
pub mod snapshot;
#[cfg(feature = "sync")]
pub mod sync;
