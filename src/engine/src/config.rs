//! Engine-specific sizing helpers for workers and memory guards.
//!
//! This module centralizes the logic for deriving worker thread counts and queue capacity
//! heuristics from a [`RunConfiguration`]. The engine relies on these defaults to provision the
//! scheduler and frontier queue while respecting user-provided memory limits.

use std::num::{NonZeroU16, NonZeroUsize};

use tlc_util::RunConfiguration;

use crate::work_queue::QueueCapacity;

/// Minimum number of frontier entries reserved per worker when throttling.
const MIN_QUEUE_ENTRIES_PER_WORKER: usize = 128;
/// Approximate memory budget (in bytes) attributed to each frontier entry when deriving a bounded
/// queue capacity. This conservative estimate keeps throttled frontiers from exhausting the
/// configured memory limit.
const APPROX_FRONTIER_ENTRY_BYTES: u64 = 512 * 1024; // 512 KiB

/// Derived engine sizing that captures worker/thread counts and memory guards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineSizing {
    configured_workers: NonZeroU16,
    worker_threads: NonZeroUsize,
    memory_guard: MemoryGuard,
}

impl EngineSizing {
    /// Build sizing information from a [`RunConfiguration`].
    pub fn from_configuration(configuration: &RunConfiguration) -> Self {
        Self::new(configuration.workers, configuration.memory_limit_bytes)
    }

    /// Build sizing information from explicit worker and memory settings.
    pub fn new(configured_workers: NonZeroU16, memory_limit_bytes: Option<u64>) -> Self {
        let worker_threads = NonZeroUsize::new(configured_workers.get() as usize)
            .expect("NonZeroU16 guarantees a non-zero usize value");
        let memory_guard = MemoryGuard::new(memory_limit_bytes, worker_threads);
        Self {
            configured_workers,
            worker_threads,
            memory_guard,
        }
    }

    /// Worker count requested by the caller (often surfaced in CLI/UI).
    pub fn configured_workers(&self) -> NonZeroU16 {
        self.configured_workers
    }

    /// Worker count expressed as `NonZeroUsize`, suitable for Rayon pools.
    pub fn worker_threads(&self) -> NonZeroUsize {
        self.worker_threads
    }

    /// Memory guard derived from the sizing inputs.
    pub fn memory_guard(&self) -> &MemoryGuard {
        &self.memory_guard
    }

    /// Optional memory limit in bytes, as provided by the configuration.
    pub fn memory_limit_bytes(&self) -> Option<u64> {
        self.memory_guard.limit_bytes()
    }

    /// Queue capacity selected for frontier throttling.
    pub fn queue_capacity(&self) -> QueueCapacity {
        self.memory_guard.queue_capacity()
    }
}

/// Memory guard derived from run configuration inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryGuard {
    limit_bytes: Option<u64>,
    queue_capacity: QueueCapacity,
}

impl MemoryGuard {
    /// Construct a guard from the optional memory limit and worker count.
    pub fn new(limit_bytes: Option<u64>, workers: NonZeroUsize) -> Self {
        let queue_capacity = match limit_bytes {
            Some(limit) => QueueCapacity::bounded(derive_bounded_capacity(limit, workers)),
            None => QueueCapacity::unbounded(),
        };

        Self {
            limit_bytes,
            queue_capacity,
        }
    }

    /// Optional memory limit in bytes.
    pub fn limit_bytes(&self) -> Option<u64> {
        self.limit_bytes
    }

    /// Queue capacity enforced for the frontier.
    pub fn queue_capacity(&self) -> QueueCapacity {
        self.queue_capacity
    }

    /// Whether the guard enforces throttling.
    pub fn is_bounded(&self) -> bool {
        matches!(self.queue_capacity, QueueCapacity::Bounded(_))
    }
}

fn derive_bounded_capacity(limit_bytes: u64, workers: NonZeroUsize) -> NonZeroUsize {
    let min_entries = workers
        .get()
        .saturating_mul(MIN_QUEUE_ENTRIES_PER_WORKER)
        .max(1);

    let approx_entries_u64 = if APPROX_FRONTIER_ENTRY_BYTES == 0 {
        min_entries as u64
    } else {
        (limit_bytes / APPROX_FRONTIER_ENTRY_BYTES).max(min_entries as u64)
    };

    let approx_entries = if approx_entries_u64 > usize::MAX as u64 {
        usize::MAX
    } else {
        approx_entries_u64 as usize
    };

    let capacity = approx_entries.max(min_entries).max(1);

    NonZeroUsize::new(capacity).expect("capacity cannot be zero")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tlc_util::{ProgressMode, RunConfiguration, TelemetryMode};
    use ulid::Ulid;

    fn sample_configuration(memory_limit: Option<u64>, workers: u16) -> RunConfiguration {
        RunConfiguration::new(
            Ulid::new(),
            std::num::NonZeroU16::new(workers).expect("non-zero workers"),
            memory_limit,
            TelemetryMode::Local,
            ProgressMode::Tty,
            None,
        )
        .expect("valid configuration")
    }

    #[test]
    fn memory_guard_unbounded_without_limit() {
        let workers = NonZeroUsize::new(4).expect("non-zero workers");
        let guard = MemoryGuard::new(None, workers);
        assert!(!guard.is_bounded());
        assert_eq!(guard.limit_bytes(), None);
        assert!(matches!(guard.queue_capacity(), QueueCapacity::Unbounded));
    }

    #[test]
    fn memory_guard_bounded_with_limit() {
        let workers = NonZeroUsize::new(4).expect("non-zero workers");
        let limit = 2 * 1024 * 1024 * 1024; // 2 GiB
        let guard = MemoryGuard::new(Some(limit), workers);
        assert!(guard.is_bounded());
        assert_eq!(guard.limit_bytes(), Some(limit));

        match guard.queue_capacity() {
            QueueCapacity::Bounded(capacity) => {
                assert_eq!(capacity.get(), 4096);
            }
            QueueCapacity::Unbounded => panic!("expected bounded queue capacity"),
        }
    }

    #[test]
    fn engine_sizing_from_configuration_respects_memory_limit() {
        let configuration = sample_configuration(Some(1_073_741_824), 8); // 1 GiB
        let sizing = EngineSizing::from_configuration(&configuration);

        assert_eq!(sizing.configured_workers().get(), 8);
        assert_eq!(sizing.worker_threads().get(), 8);
        assert_eq!(sizing.memory_limit_bytes(), Some(1_073_741_824));

        match sizing.queue_capacity() {
            QueueCapacity::Bounded(capacity) => {
                // 1 GiB / 512 KiB = 2048 entries (greater than minimum 1024)
                assert_eq!(capacity.get(), 2048);
            }
            QueueCapacity::Unbounded => panic!("expected bounded queue capacity"),
        }
    }

    #[test]
    fn engine_sizing_without_memory_limit_is_unbounded() {
        let configuration = sample_configuration(None, 2);
        let sizing = EngineSizing::from_configuration(&configuration);

        assert!(!sizing.memory_guard().is_bounded());
        assert!(matches!(sizing.queue_capacity(), QueueCapacity::Unbounded));
    }
}
