//! Test utilities shared across the workspace. The `sync` feature adds
//! database-backed fixtures for the sync crates (see [`sync`]). Without
//! it this crate is empty. Extend as new tests need it.

#[cfg(feature = "sync")]
pub mod sync;
