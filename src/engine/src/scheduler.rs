//! Worker scheduler built on top of `rayon` for state exploration.
//!
//! The scheduler owns a dedicated `rayon::ThreadPool` sized according to the engine's worker
//! configuration. Callers submit exploration jobs by providing closures that operate on frontier
//! slices; the scheduler partitions the work evenly and executes the closures in parallel.

use std::{fmt, num::NonZeroUsize, sync::Arc};

use rayon::{prelude::*, ThreadPool, ThreadPoolBuildError, ThreadPoolBuilder};

use crate::{partition_frontier, FrontierSlice};

/// Errors raised by the worker scheduler.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// Wrapper around [`rayon::ThreadPoolBuilder`] failures.
    #[error("failed to build worker thread pool: {0}")]
    ThreadPool(#[from] ThreadPoolBuildError),
}

/// Result alias for scheduler operations.
pub type Result<T> = std::result::Result<T, SchedulerError>;

/// Rayon-backed worker scheduler that partitions frontier slices across worker threads.
pub struct WorkerScheduler {
    pool: Arc<ThreadPool>,
    worker_count: NonZeroUsize,
}

impl WorkerScheduler {
    /// Construct a scheduler configured with the provided worker count and default thread name
    /// prefix (`tlc-worker`).
    pub fn new(worker_count: NonZeroUsize) -> Result<Self> {
        Self::with_name_prefix(worker_count, "tlc-worker")
    }

    /// Construct a scheduler while customizing the thread name prefix.
    pub fn with_name_prefix<S>(worker_count: NonZeroUsize, prefix: S) -> Result<Self>
    where
        S: Into<String>,
    {
        let prefix = prefix.into();
        let builder = ThreadPoolBuilder::new()
            .num_threads(worker_count.get())
            .thread_name(move |idx| {
                if prefix.is_empty() {
                    format!("tlc-worker-{idx}")
                } else {
                    format!("{prefix}-{idx}")
                }
            });

        let pool = builder.build()?;
        Ok(Self {
            pool: Arc::new(pool),
            worker_count,
        })
    }

    /// Return the configured worker count.
    pub fn worker_count(&self) -> NonZeroUsize {
        self.worker_count
    }

    /// Execute the given closure on each partitioned frontier slice without collecting results.
    ///
    /// Callers share state between workers via thread-safe primitives (channels, atomics, locks).
    pub fn for_each_slice<F>(&self, frontier_len: usize, task: F)
    where
        F: Fn(FrontierSlice) + Send + Sync,
    {
        if frontier_len == 0 {
            return;
        }

        let slices = partition_frontier(frontier_len, self.worker_count);
        self.pool
            .install(|| slices.into_par_iter().for_each(|slice| task(slice)));
    }

    /// Map the provided closure across each slice, collecting the produced results.
    pub fn map_slices<F, T>(&self, frontier_len: usize, task: F) -> Vec<T>
    where
        F: Fn(FrontierSlice) -> T + Send + Sync,
        T: Send,
    {
        if frontier_len == 0 {
            return Vec::new();
        }

        let slices = partition_frontier(frontier_len, self.worker_count);
        self.pool
            .install(|| slices.into_par_iter().map(|slice| task(slice)).collect())
    }

    /// Run an arbitrary job inside the scheduler's thread pool, returning the job's result.
    pub fn install<F, T>(&self, job: F) -> T
    where
        F: FnOnce() -> T + Send,
        T: Send,
    {
        self.pool.install(job)
    }
}

impl Clone for WorkerScheduler {
    fn clone(&self) -> Self {
        Self {
            pool: Arc::clone(&self.pool),
            worker_count: self.worker_count,
        }
    }
}

impl fmt::Debug for WorkerScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerScheduler")
            .field("worker_count", &self.worker_count)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::thread;

    #[test]
    fn builds_with_requested_worker_count() -> Result<()> {
        let workers = NonZeroUsize::new(4).expect("non-zero workers");
        let scheduler = WorkerScheduler::new(workers)?;
        assert_eq!(scheduler.worker_count(), workers);
        Ok(())
    }

    #[test]
    fn applies_custom_thread_name_prefix() -> Result<()> {
        let workers = NonZeroUsize::new(2).expect("non-zero workers");
        let scheduler = WorkerScheduler::with_name_prefix(workers, "custom-prefix")?;
        let names = Arc::new(Mutex::new(Vec::new()));

        scheduler.for_each_slice(8, {
            let names = Arc::clone(&names);
            move |_| {
                let name = thread::current()
                    .name()
                    .map(str::to_string)
                    .unwrap_or_else(|| "<unnamed>".to_string());
                names.lock().expect("lock thread name vec").push(name);
            }
        });

        let collected = names.lock().expect("collect names").clone();
        assert!(!collected.is_empty(), "expected at least one worker name");
        assert!(collected
            .iter()
            .all(|name| name.starts_with("custom-prefix-")));
        Ok(())
    }

    #[test]
    fn for_each_slice_covers_entire_frontier() -> Result<()> {
        let workers = NonZeroUsize::new(3).expect("non-zero workers");
        let scheduler = WorkerScheduler::new(workers)?;
        let visited = Arc::new(Mutex::new(Vec::new()));

        scheduler.for_each_slice(17, {
            let visited = Arc::clone(&visited);
            move |slice| {
                visited.lock().expect("lock visited").push(slice);
            }
        });

        let mut visited = visited.lock().expect("lock visited vec").clone();
        visited.sort_by_key(|slice| slice.start);
        let expected = partition_frontier(17, workers);
        assert_eq!(visited, expected);
        Ok(())
    }

    #[test]
    fn map_slices_collects_results() -> Result<()> {
        let workers = NonZeroUsize::new(4).expect("non-zero workers");
        let scheduler = WorkerScheduler::new(workers)?;

        let totals = scheduler.map_slices(12, |slice| slice.len());
        assert_eq!(totals.iter().sum::<usize>(), 12);
        assert_eq!(totals.len(), workers.get());
        Ok(())
    }

    #[test]
    fn install_executes_job() -> Result<()> {
        let workers = NonZeroUsize::new(2).expect("non-zero workers");
        let scheduler = WorkerScheduler::new(workers)?;

        let result = scheduler.install(|| 2usize.pow(3));
        assert_eq!(result, 8);
        Ok(())
    }
}
