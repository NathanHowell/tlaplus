use std::{
    num::NonZeroU16,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;
use tlc_engine::{
    partition_frontier, EngineSizing, QueueCapacity, QueueStats, RunMetricsRecorder, WorkQueue,
    WorkerScheduler,
};
use tlc_telemetry::{QueueTelemetry, WorkerSample, WorkerTelemetry};
use tlc_util::{ProgressMode, RunConfiguration, TelemetryMode};
use ulid::Ulid;

#[test]
fn scheduler_extension_preserves_partition_and_thread_contracts() -> Result<()> {
    let workers = NonZeroU16::new(4).expect("non-zero worker count");
    let configuration = RunConfiguration::new(
        Ulid::new(),
        workers,
        Some(2 * 1024 * 1024 * 1024),
        TelemetryMode::Local,
        ProgressMode::Tty,
        None,
    )?;

    let sizing = EngineSizing::from_configuration(&configuration);
    assert_eq!(sizing.configured_workers(), workers);
    assert_eq!(sizing.worker_threads().get(), workers.get() as usize);
    assert!(
        matches!(sizing.queue_capacity(), QueueCapacity::Bounded(_)),
        "memory limit should yield bounded queue capacity"
    );

    let scheduler = WorkerScheduler::with_name_prefix(sizing.worker_threads(), "ext-scheduler")?;

    let frontier_len = sizing.worker_threads().get() * 5 + 3;
    let expected = partition_frontier(frontier_len, sizing.worker_threads());
    let mapped = scheduler.map_slices(frontier_len, |slice| slice);
    assert_eq!(
        mapped, expected,
        "scheduler must honor partition_frontier invariants"
    );

    let names = Arc::new(Mutex::new(Vec::new()));
    scheduler.for_each_slice(frontier_len, {
        let names = Arc::clone(&names);
        move |_| {
            let name = thread::current()
                .name()
                .map(str::to_string)
                .unwrap_or_else(|| "<unnamed>".to_string());
            names.lock().expect("record thread name").push(name);
        }
    });

    let mut collected = names.lock().expect("collect thread names").clone();
    collected.sort();
    collected.dedup();

    assert!(
        !collected.is_empty(),
        "expected at least one worker thread to execute a slice"
    );
    assert!(
        collected.len() <= sizing.worker_threads().get(),
        "observed more distinct worker names ({}) than configured threads ({})",
        collected.len(),
        sizing.worker_threads().get()
    );

    for name in &collected {
        assert!(
            name.starts_with("ext-scheduler-"),
            "worker threads must preserve name prefix: {:?}",
            collected
        );

        let suffix = name.trim_start_matches("ext-scheduler-");
        let index: usize = suffix
            .parse()
            .expect("thread name suffix should be numeric");
        assert!(
            index < sizing.worker_threads().get(),
            "worker thread index {index} exceeds configured worker count {}",
            sizing.worker_threads().get()
        );
    }

    Ok(())
}

#[test]
fn scheduler_extension_queue_and_telemetry_contracts() -> Result<()> {
    let workers = NonZeroU16::new(6).expect("non-zero worker count");
    let run_id = Ulid::new();
    let configuration = RunConfiguration::new(
        run_id,
        workers,
        Some(4 * 1024 * 1024 * 1024),
        TelemetryMode::Local,
        ProgressMode::Tty,
        None,
    )?;

    let sizing = EngineSizing::from_configuration(&configuration);
    let worker_threads = sizing.worker_threads();
    let capacity = match sizing.queue_capacity() {
        QueueCapacity::Bounded(cap) => cap,
        QueueCapacity::Unbounded => anyhow::bail!("expected bounded queue capacity from sizing"),
    };

    let queue = WorkQueue::bounded(capacity);
    for value in 0..capacity.get() {
        queue
            .try_enqueue(value)
            .expect("bounded queue should accept items up to capacity");
    }

    let stats_full = queue.stats();
    assert_eq!(stats_full.len(), capacity.get());
    assert!(
        stats_full.capacity().is_bounded(),
        "queue should be bounded"
    );
    assert_eq!(stats_full.remaining_capacity(), Some(0));
    assert_eq!(stats_full.saturation(), Some(1.0));

    let mut telemetry = WorkerTelemetry::new(worker_threads)?;

    let partial_active = worker_threads.get() / 2;
    assert!(partial_active > 0);
    assert!(
        (partial_active as f64) / (worker_threads.get() as f64) < telemetry.alert_threshold(),
        "partial utilization must fall below alert threshold"
    );

    let under_utilized = telemetry.record_sample(
        WorkerSample::new(partial_active).with_queue(queue_telemetry_from(stats_full)),
    );
    assert!(
        under_utilized.is_alerting() && under_utilized.alert_triggered,
        "alert should trigger when utilization falls below threshold"
    );
    assert!(
        under_utilized.average_utilization < under_utilized.alert_threshold,
        "average utilization should reflect alert condition"
    );

    while let Ok(_) = queue.try_dequeue() {}
    let stats_empty = queue.stats();
    assert_eq!(stats_empty.len(), 0);
    assert_eq!(stats_empty.saturation(), Some(0.0));

    let recovered = telemetry.record_sample(
        WorkerSample::new(worker_threads.get()).with_queue(queue_telemetry_from(stats_empty)),
    );

    assert!(
        !recovered.is_alerting() && recovered.alert_cleared,
        "alert should clear once utilization recovers"
    );
    assert!(
        recovered.average_utilization >= recovered.alert_threshold,
        "average utilization should meet or exceed the threshold after recovery"
    );

    let mut recorder =
        RunMetricsRecorder::with_start_instant(run_id, Instant::now() - Duration::from_millis(10));
    recorder.record_memory_sample(64 * 1024 * 1024);
    recorder.record_memory_sample(96 * 1024 * 1024);

    let metrics = recorder.finish(256);
    assert_eq!(metrics.run_id(), run_id);
    assert_eq!(metrics.states_explored(), 256);
    assert_eq!(metrics.peak_memory_bytes(), Some(96 * 1024 * 1024));
    assert!(
        metrics.states_per_second() > 0.0,
        "states-per-second should report throughput"
    );

    Ok(())
}

fn queue_telemetry_from(stats: QueueStats) -> QueueTelemetry {
    match stats.capacity() {
        QueueCapacity::Bounded(cap) => {
            QueueTelemetry::bounded(stats.len(), cap.get(), stats.saturation())
        }
        QueueCapacity::Unbounded => QueueTelemetry::unbounded(stats.len(), stats.saturation()),
    }
}
