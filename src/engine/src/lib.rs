//! Exploration engine scaffolding for the TLC rewrite.

use anyhow::Result;
use crossbeam::channel;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EngineConfig {
    workers: usize,
}

/// Ensure the engine crate links required concurrency dependencies.
pub fn bootstrap_engine() -> Result<()> {
    let numbers = (0..4_u8).into_par_iter().map(|n| n as usize);
    let sum: usize = numbers.sum();
    tracing::trace!(sum, "engine parallel iterator sample complete");

    let (tx, rx) = channel::unbounded::<usize>();
    tx.send(sum)?;
    let _ = rx.try_recv().unwrap_or(sum);

    let _config = EngineConfig { workers: sum };

    tlc_util::initialize_runtime()?;
    tracing::debug!("tlc-engine bootstrap initialized");
    Ok(())
}
