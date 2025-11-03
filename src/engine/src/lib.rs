//! Exploration engine scaffolding for the TLC rewrite.

use std::num::NonZeroUsize;

use anyhow::Result;
use crossbeam::channel;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EngineConfig {
    workers: usize,
}

/// Slice of the frontier assigned to a single worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontierSlice {
    pub start: usize,
    pub end: usize,
}

impl FrontierSlice {
    #[inline]
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// Evenly partition a frontier into contiguous slices for each worker.
pub fn partition_frontier(frontier_len: usize, workers: NonZeroUsize) -> Vec<FrontierSlice> {
    let worker_count = workers.get();
    let base = frontier_len / worker_count;
    let remainder = frontier_len % worker_count;

    let mut start = 0usize;
    let mut slices = Vec::with_capacity(worker_count);

    for idx in 0..worker_count {
        let extra = usize::from(idx < remainder);
        let end = start + base + extra;
        slices.push(FrontierSlice { start, end });
        start = end;
    }

    slices
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
