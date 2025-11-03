//! Shared utilities for the Rust-native TLC workspace.
pub mod prelude {
    //! Convenient re-exports for commonly used utility traits and helpers.
    pub use anyhow::{anyhow, Context, Result};
    pub use serde::{Deserialize, Serialize};
    pub use thiserror::Error;
    pub use tracing::{debug, error, info, instrument, trace, warn};
}

/// Placeholder helper that proves the crate wires up correctly.
pub fn initialize_runtime() -> anyhow::Result<()> {
    tracing::debug!("tlc-util runtime initialized");
    Ok(())
}
