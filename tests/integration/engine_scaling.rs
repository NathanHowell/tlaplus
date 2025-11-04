use std::{num::NonZeroU16, time::Duration};

use anyhow::Result;
use tlc_test_support::scaling::{simulate_sleeping_work, throughput_delta};

#[test]
fn multi_worker_runs_outpace_single_worker_baseline() -> Result<()> {
    let tasks = 16usize;
    let per_task = Duration::from_millis(15);

    let baseline_workers = NonZeroU16::new(1).expect("non-zero baseline workers");
    let scaled_workers = NonZeroU16::new(4).expect("non-zero multi-worker count");

    let baseline = simulate_sleeping_work(baseline_workers, tasks, per_task)?;
    let scaled = simulate_sleeping_work(scaled_workers, tasks, per_task)?;

    let delta = throughput_delta(&baseline, &scaled);
    let baseline_throughput = baseline.throughput();
    let scaled_throughput = scaled.throughput();

    assert!(
        scaled_throughput > baseline_throughput,
        "scaled run should exceed baseline throughput (baseline={baseline_throughput:.2}, scaled={scaled_throughput:.2})"
    );
    assert!(
        delta >= 0.5,
        "expected >=50% throughput improvement; observed delta {:.0}%",
        delta * 100.0
    );
    assert!(
        scaled.runtime() < baseline.runtime(),
        "scaled run should complete faster (baseline={:?}, scaled={:?})",
        baseline.runtime(),
        scaled.runtime()
    );
    assert_eq!(
        baseline.task_count(),
        scaled.task_count(),
        "scaling scenario should process the same task volume"
    );

    Ok(())
}
