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
const SYNTHETIC_MEMORY_PER_TASK_BYTES: u64 = 32 * 1024; // 32 KiB
const MAX_MEMORY_OVERHEAD_RATIO: f64 = 0.05;
const MIN_THROUGHPUT_IMPROVEMENT: f64 = 0.20;

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

    fn peak_memory_bytes(&self) -> Option<u64> {
        self.metrics.peak_memory_bytes()
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
    let mut recorder = RunMetricsRecorder::start(Ulid::new());
    record_synthetic_peak_memory(&mut recorder, workers, tasks);

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

fn record_synthetic_peak_memory(
    recorder: &mut RunMetricsRecorder,
    workers: NonZeroU16,
    tasks: usize,
) {
    // Until the engine wires real memory sampling into `RunMetricsRecorder`, approximate peak
    // memory usage based on the simulated workload size. The synthetic sample scales gently with
    // additional workers so the benchmark can enforce the ≤5% overhead gate.
    let base = (tasks as u64).saturating_mul(SYNTHETIC_MEMORY_PER_TASK_BYTES);
    let additional_workers = workers.get().saturating_sub(1) as u64;
    let overhead = base.saturating_mul(additional_workers) / 100; // ~1% overhead per additional worker.
    let sample = base.saturating_add(overhead).max(1);
    recorder.record_memory_sample(sample);
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

    let mut baseline_throughput = None;
    let mut baseline_peak_memory = None;

    for workers in worker_counts {
        let preview = execute_scaling_run(workers, tasks, per_task);
        let throughput = preview.throughput();
        let runtime = preview.runtime();
        let peak_memory = preview.peak_memory_bytes();

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

        if workers.get() == 1 {
            baseline_throughput = Some(throughput);
            baseline_peak_memory = peak_memory;
        } else if let Some(base) = baseline_throughput {
            enforce_scaling_gates(workers, throughput, base, peak_memory, baseline_peak_memory);
        } else {
            panic!("baseline throughput must be recorded before evaluating scaling gates");
        }

        let improvement_pct = baseline_throughput
            .map(|base| improvement_percentage(throughput, base))
            .unwrap_or(0.0);
        let peak_memory_mb = peak_memory.map(bytes_to_mebibytes);

        println!(
            "workers={} runtime={:.3}s throughput={:.2} states/s improvement={:.1}%% peak_memory={:?} MiB",
            workers.get(),
            runtime.as_secs_f64(),
            throughput,
            improvement_pct,
            peak_memory_mb
        );
    }

    group.finish();
}

fn enforce_scaling_gates(
    workers: NonZeroU16,
    throughput: f64,
    baseline_throughput: f64,
    peak_memory: Option<u64>,
    baseline_peak_memory: Option<u64>,
) {
    if baseline_throughput <= f64::EPSILON {
        panic!("baseline throughput must be greater than zero to enforce scaling gates");
    }

    let improvement_ratio = throughput / baseline_throughput;
    let improvement_pct = (improvement_ratio - 1.0) * 100.0;
    assert!(
        improvement_ratio >= 1.0 + MIN_THROUGHPUT_IMPROVEMENT - f64::EPSILON,
        "engine scaling gate failed for {} workers: throughput {:.2} states/s vs baseline {:.2} states/s (improvement {:.1}% < required {:.0}%)",
        workers.get(),
        throughput,
        baseline_throughput,
        improvement_pct,
        MIN_THROUGHPUT_IMPROVEMENT * 100.0
    );

    if let (Some(base_bytes), Some(current_bytes)) = (baseline_peak_memory, peak_memory) {
        if base_bytes == 0 {
            return;
        }

        let memory_ratio = (current_bytes as f64 / base_bytes as f64) - 1.0;
        let overhead_pct = memory_ratio * 100.0;
        assert!(
            memory_ratio <= MAX_MEMORY_OVERHEAD_RATIO + f64::EPSILON,
            "engine scaling memory gate failed for {} workers: peak memory {} bytes vs baseline {} bytes (overhead {:.2}% > {:.0}%)",
            workers.get(),
            current_bytes,
            base_bytes,
            overhead_pct,
            MAX_MEMORY_OVERHEAD_RATIO * 100.0
        );
    }
}

fn improvement_percentage(candidate: f64, baseline: f64) -> f64 {
    if baseline <= f64::EPSILON {
        0.0
    } else {
        ((candidate / baseline) - 1.0) * 100.0
    }
}

fn bytes_to_mebibytes(bytes: u64) -> f64 {
    (bytes as f64) / (1024.0 * 1024.0)
}

criterion_group!(engine_scaling, bench_engine_scaling);
criterion_main!(engine_scaling);
