//! UniFFI FFI surface over the livtet workspace.
//!
//! This crate is the single boundary crossed by the Android and iOS
//! consumers in the `mobile` repo. All data crossing the boundary uses
//! flat DTO records defined here, plus the domain primitives that
//! `livtet-types` exposes directly (with its `uniffi` feature).

uniffi::setup_scaffolding!();

mod dto;
mod error;
mod mutations;
mod search;
mod store;
mod works;

pub use dto::*;
pub use error::LivtetError;
pub use mutations::EditionPatch;
pub use store::LivtetStore;
