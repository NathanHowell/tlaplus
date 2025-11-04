//! Helpers for simulating engine scaling scenarios in integration tests.

use std::{
    num::NonZeroU16,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Duration,
};

use anyhow::{ensure, Context, Result};
use tlc_engine::{EngineSizing, RunMetrics, RunMetricsRecorder, WorkerScheduler};
use ulid::Ulid;

const THREAD_NAME_PREFIX: &str = "tlc-scale";

/// Outcome of a simulated scaling run, including the recorded metrics.
#[derive(Debug)]
pub struct ScalingRun {
    worker_count: NonZeroU16,
    tasks: usize,
    metrics: RunMetrics,
}

impl ScalingRun {
    /// Worker count exercised during the run.
    pub fn worker_count(&self) -> NonZeroU16 {
        self.worker_count
    }

    /// Number of synthetic tasks processed by the run.
    pub fn task_count(&self) -> usize {
        self.tasks
    }

    /// Metrics captured for the run.
    pub fn metrics(&self) -> &RunMetrics {
        &self.metrics
    }

    /// Wall-clock runtime recorded by the metrics snapshot.
    pub fn runtime(&self) -> Duration {
        self.metrics.runtime()
    }

    /// Throughput (tasks per second) derived from the metrics snapshot.
    pub fn throughput(&self) -> f64 {
        self.metrics.states_per_second()
    }
}

/// Simulate a scaling run by distributing sleeping tasks across the requested worker count.
pub fn simulate_sleeping_work(
    workers: NonZeroU16,
    tasks: usize,
    per_task: Duration,
) -> Result<ScalingRun> {
    ensure!(tasks > 0, "task count must be positive");
    ensure!(
        !per_task.is_zero(),
        "per-task duration must be greater than zero"
    );

    let sizing = EngineSizing::new(workers, None);
    let scheduler = WorkerScheduler::with_name_prefix(sizing.worker_threads(), THREAD_NAME_PREFIX)
        .context("failed to create worker scheduler")?;

    let completed = AtomicUsize::new(0);
    let run_id = Ulid::new();
    let recorder = RunMetricsRecorder::start(run_id);

    scheduler.for_each_slice(tasks, |slice| {
        for _ in slice.start..slice.end {
            thread::sleep(per_task);
            completed.fetch_add(1, Ordering::Relaxed);
        }
    });

    let processed = completed.load(Ordering::Relaxed);
    ensure!(
        processed == tasks,
        "expected to process {tasks} tasks, processed {processed}"
    );

    let metrics = recorder.finish(processed as u128);
    Ok(ScalingRun {
        worker_count: workers,
        tasks: processed,
        metrics,
    })
}

/// Compute the proportional throughput delta between a baseline and scaled run.
pub fn throughput_delta(baseline: &ScalingRun, scaled: &ScalingRun) -> f64 {
    let baseline_throughput = baseline.throughput();
    if baseline_throughput <= f64::EPSILON {
        return 0.0;
    }

    (scaled.throughput() / baseline_throughput) - 1.0
}
