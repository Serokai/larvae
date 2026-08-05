//! Worm loading and the host half of the wasm ABI

pub mod host;

pub use host::{Outcome, Worm};

/// The ABI revision this host speaks, matching `api` in a worm's `worm.toml`
pub const ABI_VERSION: u32 = 1;
