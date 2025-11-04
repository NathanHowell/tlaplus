use std::num::NonZeroU16;

use anyhow::Result;
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use tlc_engine::{ProgressEmitter, ProgressSnapshot};
use tlc_progress::ProgressEvent;
use tlc_test_support::progress::{analyze_progress, ProgressSample, ProgressThresholds};
use ulid::Ulid;

#[test]
fn progress_harness_accepts_sequences_within_thresholds() -> Result<()> {
    let (events, expected_total) = sample_events_within_thresholds()?;
    let samples: Vec<ProgressSample> = events.iter().map(ProgressSample::from).collect();
    let analysis = analyze_progress(&samples, expected_total)?;
    let thresholds = ProgressThresholds::default();

    assert_eq!(analysis.event_count(), events.len());
    assert!(analysis.meets(&thresholds));

    Ok(())
}

#[test]
fn progress_harness_flags_refresh_and_coverage_violations() -> Result<()> {
    let (events, expected_total) = sample_events_with_violations()?;
    let samples: Vec<ProgressSample> = events.iter().map(ProgressSample::from).collect();
    let analysis = analyze_progress(&samples, expected_total)?;
    let thresholds = ProgressThresholds::default();

    assert_eq!(analysis.event_count(), events.len());
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
