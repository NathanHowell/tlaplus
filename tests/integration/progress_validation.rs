use std::num::NonZeroU16;

use anyhow::{ensure, Result};
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use tlc_engine::{ProgressEmitter, ProgressSnapshot};
use tlc_progress::ProgressEvent;
use ulid::Ulid;

#[derive(Debug, Clone, Copy)]
struct ProgressThresholds {
    max_refresh_gap: ChronoDuration,
    max_coverage_delta: f32,
}

impl Default for ProgressThresholds {
    fn default() -> Self {
        Self {
            max_refresh_gap: ChronoDuration::seconds(5),
            max_coverage_delta: 2.0,
        }
    }
}

#[derive(Debug)]
struct ProgressRunAnalysis {
    event_count: usize,
    max_refresh_gap: ChronoDuration,
    final_reported_percent: f32,
    actual_percent: f32,
}

impl ProgressRunAnalysis {
    fn coverage_delta(&self) -> f32 {
        (self.final_reported_percent - self.actual_percent).abs()
    }

    fn meets(&self, thresholds: &ProgressThresholds) -> bool {
        self.max_refresh_gap <= thresholds.max_refresh_gap
            && self.coverage_delta() <= thresholds.max_coverage_delta
    }

    fn max_refresh_gap(&self) -> ChronoDuration {
        self.max_refresh_gap
    }
}

fn analyze_progress(
    events: &[ProgressEvent],
    expected_total_states: u128,
) -> Result<ProgressRunAnalysis> {
    ensure!(
        !events.is_empty(),
        "progress validation requires at least one event"
    );
    ensure!(
        expected_total_states > 0,
        "expected_total_states must be greater than zero"
    );

    let mut max_gap = ChronoDuration::zero();
    let mut previous_timestamp = events[0].timestamp();
    let mut final_states = events[0].states_explored();
    let mut final_percent = events[0].percent_complete();

    for event in events.iter().skip(1) {
        let delta = event.timestamp().signed_duration_since(previous_timestamp);
        ensure!(
            delta >= ChronoDuration::zero(),
            "progress events must be ordered by timestamp"
        );

        if delta > max_gap {
            max_gap = delta;
        }

        previous_timestamp = event.timestamp();
        final_states = event.states_explored();
        final_percent = event.percent_complete();
    }

    let actual_fraction =
        (final_states.min(expected_total_states) as f64) / (expected_total_states as f64);
    let actual_percent = (actual_fraction * 100.0).min(100.0) as f32;

    Ok(ProgressRunAnalysis {
        event_count: events.len(),
        max_refresh_gap: max_gap,
        final_reported_percent: final_percent,
        actual_percent,
    })
}

#[test]
fn progress_harness_accepts_sequences_within_thresholds() -> Result<()> {
    let (events, expected_total) = sample_events_within_thresholds()?;
    let analysis = analyze_progress(&events, expected_total)?;
    let thresholds = ProgressThresholds::default();

    assert_eq!(analysis.event_count, events.len());
    assert!(
        analysis.meets(&thresholds),
        "analysis should meet thresholds: {:?}",
        analysis
    );

    Ok(())
}

#[test]
fn progress_harness_flags_refresh_and_coverage_violations() -> Result<()> {
    let (events, expected_total) = sample_events_with_violations()?;
    let analysis = analyze_progress(&events, expected_total)?;
    let thresholds = ProgressThresholds::default();

    assert_eq!(analysis.event_count, events.len());
    assert!(
        analysis.max_refresh_gap() > thresholds.max_refresh_gap,
        "expected refresh gap violation (max gap {:?})",
        analysis.max_refresh_gap()
    );
    assert!(
        analysis.coverage_delta() > thresholds.max_coverage_delta,
        "expected coverage delta violation ({:.2}%)",
        analysis.coverage_delta()
    );
    assert!(
        !analysis.meets(&thresholds),
        "analysis should fail threshold check: {:?}",
        analysis
    );

    Ok(())
}

fn sample_events_within_thresholds() -> Result<(Vec<ProgressEvent>, u128)> {
    let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJW")?;
    let workers = NonZeroU16::new(6).expect("validated non-zero workers");
    let start = Utc
        .with_ymd_and_hms(2025, 11, 2, 3, 45, 0)
        .single()
        .expect("valid start time");

    let mut emitter = ProgressEmitter::with_start_time(run_id, workers, start);
    let expected_total = 80_000_u128;
    let timeline = [
        (0, ProgressSnapshot::new(0, 0.0)),
        (3, ProgressSnapshot::new(12_000, 15.0)),
        (7, ProgressSnapshot::new(32_000, 40.0)),
        (10, ProgressSnapshot::new(52_000, 65.0)),
        (14, ProgressSnapshot::new(80_000, 100.0)),
    ];

    let mut events = Vec::with_capacity(timeline.len());
    for (offset_seconds, snapshot) in timeline {
        let timestamp = start + ChronoDuration::seconds(offset_seconds);
        let event = emitter
            .record_at(snapshot, timestamp)?
            .with_eta_seconds(None)?;
        events.push(event);
    }

    Ok((events, expected_total))
}

fn sample_events_with_violations() -> Result<(Vec<ProgressEvent>, u128)> {
    let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJX")?;
    let workers = NonZeroU16::new(6).expect("validated non-zero workers");
    let start = Utc
        .with_ymd_and_hms(2025, 11, 2, 4, 0, 0)
        .single()
        .expect("valid start time");

    let mut emitter = ProgressEmitter::with_start_time(run_id, workers, start);
    let expected_total = 80_000_u128;
    let timeline = [
        (0, ProgressSnapshot::new(0, 0.0)),
        (5, ProgressSnapshot::new(10_000, 12.5)),
        (12, ProgressSnapshot::new(25_000, 28.0)),
        (20, ProgressSnapshot::new(40_000, 45.0)),
        (35, ProgressSnapshot::new(80_000, 85.0)),
    ];

    let mut events = Vec::with_capacity(timeline.len());
    for (offset_seconds, snapshot) in timeline {
        let timestamp = start + ChronoDuration::seconds(offset_seconds);
        let event = emitter
            .record_at(snapshot, timestamp)?
            .with_eta_seconds(None)?;
        events.push(event);
    }

    Ok((events, expected_total))
}
