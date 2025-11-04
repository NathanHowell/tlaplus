use std::{
    num::{NonZeroU16, NonZeroUsize},
    time::Duration,
};

use anyhow::Result;
use tlc_engine::partition_frontier;
use tlc_telemetry::{WorkerSample, WorkerTelemetry};

const WORKER_BASE_MILLIS: [u64; 6] = [32, 30, 28, 24, 20, 18];
const TASKS_PER_WORKER: usize = 4;
const SAMPLE_STEP: Duration = Duration::from_millis(1);
const UTILIZATION_THRESHOLD: f64 = 0.70;

#[test]
fn skewed_workload_maintains_aggregate_utilization() -> Result<()> {
    let worker_count =
        NonZeroU16::new(WORKER_BASE_MILLIS.len() as u16).expect("worker count must be non-zero");
    let worker_threads =
        NonZeroUsize::new(worker_count.get() as usize).expect("convert worker count");

    let mut durations = Vec::with_capacity(worker_threads.get() * TASKS_PER_WORKER);
    for &millis in &WORKER_BASE_MILLIS {
        durations.extend(std::iter::repeat(Duration::from_millis(millis)).take(TASKS_PER_WORKER));
    }

    let busy_times = compute_busy_times(worker_threads, &durations);
    let runtime = busy_times
        .iter()
        .copied()
        .max()
        .expect("busy times populated");

    // Sanity-check the scenario: even with the uneven distribution of work, aggregate
    // utilization should comfortably exceed the 70% enforcement threshold.
    let expected_average = aggregate_utilization(&busy_times, runtime, worker_threads);
    assert!(
        expected_average >= UTILIZATION_THRESHOLD,
        "expected aggregate utilization >= {:.0}%, observed {:.1}%",
        UTILIZATION_THRESHOLD * 100.0,
        expected_average * 100.0
    );

    let mut telemetry =
        WorkerTelemetry::new(worker_threads).expect("default utilization threshold applies");
    let step_ms = SAMPLE_STEP.as_millis() as usize;
    assert!(
        step_ms > 0,
        "sample interval must be at least one millisecond"
    );
    assert!(
        runtime.as_millis() % SAMPLE_STEP.as_millis() == 0,
        "runtime should align with the chosen sample interval"
    );
    let runtime_steps = (runtime.as_millis() / SAMPLE_STEP.as_millis()) as usize;
    assert!(
        runtime_steps > 0,
        "runtime should span at least one sample interval"
    );

    let mut report = None;
    for tick in 0..runtime_steps {
        let elapsed = Duration::from_millis((tick * step_ms) as u64);
        let active = active_workers_at(&busy_times, elapsed);
        report = Some(telemetry.record_sample(WorkerSample::new(active)));
    }

    let report = report.expect("at least one utilization sample recorded");
    assert_eq!(
        report.sample_index, runtime_steps,
        "expected one utilization sample per millisecond"
    );
    assert!(
        report.average_utilization >= UTILIZATION_THRESHOLD,
        "expected aggregate utilization >= {:.0}%, observed {:.1}%",
        UTILIZATION_THRESHOLD * 100.0,
        report.average_utilization * 100.0
    );

    let difference = (report.average_utilization - expected_average).abs();
    assert!(
        difference < 0.01,
        "aggregate utilization drifted by {:.2} (expected {:.3}, observed {:.3})",
        difference,
        expected_average,
        report.average_utilization
    );

    Ok(())
}

fn compute_busy_times(workers: NonZeroUsize, durations: &[Duration]) -> Vec<Duration> {
    let slices = partition_frontier(durations.len(), workers);
    slices
        .into_iter()
        .map(|slice| {
            durations[slice.start..slice.end]
                .iter()
                .copied()
                .fold(Duration::ZERO, |acc, duration| acc + duration)
        })
        .collect()
}

fn aggregate_utilization(busy_times: &[Duration], runtime: Duration, workers: NonZeroUsize) -> f64 {
    let busy_ms: u128 = busy_times.iter().map(|duration| duration.as_millis()).sum();
    let runtime_ms = runtime.as_millis();
    if runtime_ms == 0 {
        return 0.0;
    }

    let total_slots = runtime_ms * workers.get() as u128;
    (busy_ms as f64) / (total_slots as f64)
}

fn active_workers_at(busy_times: &[Duration], elapsed: Duration) -> usize {
    busy_times.iter().filter(|busy| **busy > elapsed).count()
}
