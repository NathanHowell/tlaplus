use std::{
    num::NonZeroU16,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Duration,
};

use criterion::{
    criterion_group, criterion_main, BenchmarkId, Criterion, SamplingMode, Throughput,
};
use tlc_engine::{EngineSizing, RunMetrics, RunMetricsRecorder, WorkerScheduler};
use ulid::Ulid;

const THREAD_NAME_PREFIX: &str = "bench-scale";

#[derive(Debug)]
struct ScalingRun {
    tasks: usize,
    metrics: RunMetrics,
}

impl ScalingRun {
    fn runtime(&self) -> Duration {
        self.metrics.runtime()
    }

    fn throughput(&self) -> f64 {
        self.metrics.states_per_second()
    }
}

fn execute_scaling_run(workers: NonZeroU16, tasks: usize, per_task: Duration) -> ScalingRun {
    assert!(tasks > 0, "task count must be positive");
    assert!(
        !per_task.is_zero(),
        "per-task duration must be greater than zero"
    );

    let sizing = EngineSizing::new(workers, None);
    let scheduler = WorkerScheduler::with_name_prefix(sizing.worker_threads(), THREAD_NAME_PREFIX)
        .expect("failed to create worker scheduler");

    let completed = AtomicUsize::new(0);
    let recorder = RunMetricsRecorder::start(Ulid::new());

    scheduler.for_each_slice(tasks, |slice| {
        for _ in slice.start..slice.end {
            thread::sleep(per_task);
            completed.fetch_add(1, Ordering::Relaxed);
        }
    });

    let processed = completed.load(Ordering::Relaxed);
    assert_eq!(
        processed, tasks,
        "expected to process {tasks} tasks, processed {processed}"
    );

    let metrics = recorder.finish(processed as u128);
    ScalingRun {
        tasks: processed,
        metrics,
    }
}

fn bench_engine_scaling(c: &mut Criterion) {
    let tasks = 64usize;
    let per_task = Duration::from_millis(2);
    let worker_counts = [
        NonZeroU16::new(1).expect("non-zero workers"),
        NonZeroU16::new(2).expect("non-zero workers"),
        NonZeroU16::new(4).expect("non-zero workers"),
    ];

    let mut group = c.benchmark_group("engine_scaling");
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);

    for workers in worker_counts {
        let preview = execute_scaling_run(workers, tasks, per_task);
        let throughput = preview.throughput();
        let runtime = preview.runtime();

        group.throughput(Throughput::Elements(preview.tasks as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(workers.get()),
            &workers,
            |b, &workers| {
                b.iter_custom(|iters| {
                    let mut total_runtime = Duration::ZERO;
                    for iteration in 0..iters {
                        let run = execute_scaling_run(workers, tasks, per_task);
                        if iteration == 0 {
                            tracing::info!(
                                worker_count = workers.get(),
                                tasks = run.tasks,
                                runtime_ms = run.runtime().as_secs_f64() * 1_000.0,
                                throughput_eps = run.throughput(),
                                "engine scaling bench iteration recorded"
                            );
                        }
                        total_runtime += run.runtime();
                    }
                    total_runtime
                });
            },
        );

        println!(
            "workers={} runtime={:.3}s throughput={:.2} states/s",
            workers.get(),
            runtime.as_secs_f64(),
            throughput
        );
    }

    group.finish();
}

criterion_group!(engine_scaling, bench_engine_scaling);
criterion_main!(engine_scaling);
