//! Progress event integration for the TLC engine.
//!
//! This module translates engine runtime snapshots into [`ProgressEvent`]s that downstream
//! consumers (TTY renderers, NDJSON writers, CLI adapters) can forward to users.

use std::num::NonZeroU16;

use anyhow::{ensure, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tlc_progress::ProgressEvent;
use ulid::Ulid;

/// Snapshot of the exploration state reported by the engine loop.
#[derive(Debug, Clone)]
pub struct ProgressSnapshot {
    states_explored: u128,
    coverage_percent: f32,
    workers_active: Option<NonZeroU16>,
}

impl ProgressSnapshot {
    /// Create a snapshot with the latest exploration counters.
    pub fn new(states_explored: u128, coverage_percent: f32) -> Self {
        Self {
            states_explored,
            coverage_percent,
            workers_active: None,
        }
    }

    /// Override the number of actively working threads for this snapshot.
    pub fn with_workers_active(mut self, workers: NonZeroU16) -> Self {
        self.workers_active = Some(workers);
        self
    }

    fn workers(&self, default: NonZeroU16) -> NonZeroU16 {
        self.workers_active.unwrap_or(default)
    }
}

/// Engine-side helper that converts snapshots into [`ProgressEvent`] updates.
#[derive(Debug)]
pub struct ProgressEmitter {
    run_id: Ulid,
    configured_workers: NonZeroU16,
    start_time: DateTime<Utc>,
    last_event_time: DateTime<Utc>,
    last_states_explored: u128,
    last_throughput: f64,
}

impl ProgressEmitter {
    /// Create an emitter with the current timestamp as the run start.
    pub fn new(run_id: Ulid, workers: NonZeroU16) -> Self {
        Self::with_start_time(run_id, workers, Utc::now())
    }

    /// Create an emitter with an explicit start timestamp (primarily for testing).
    pub fn with_start_time(run_id: Ulid, workers: NonZeroU16, start_time: DateTime<Utc>) -> Self {
        Self {
            run_id,
            configured_workers: workers,
            start_time,
            last_event_time: start_time,
            last_states_explored: 0,
            last_throughput: 0.0,
        }
    }

    /// Convert a snapshot into a [`ProgressEvent`] using the current timestamp.
    pub fn record(&mut self, snapshot: ProgressSnapshot) -> Result<ProgressEvent> {
        self.record_at(snapshot, Utc::now())
    }

    /// Convert a snapshot into a [`ProgressEvent`] using an explicit timestamp.
    pub fn record_at(
        &mut self,
        snapshot: ProgressSnapshot,
        timestamp: DateTime<Utc>,
    ) -> Result<ProgressEvent> {
        ensure!(
            timestamp >= self.last_event_time,
            "progress timestamps must be monotonic"
        );

        let states = snapshot.states_explored;
        ensure!(
            states >= self.last_states_explored,
            "states explored cannot decrease (prev {}, current {states})",
            self.last_states_explored
        );

        let percent = snapshot.coverage_percent.clamp(0.0, 100.0);
        let workers = snapshot.workers(self.configured_workers);

        let throughput = self.compute_throughput(states, timestamp);
        let eta_seconds = self.estimate_eta(percent, timestamp, throughput);

        let event = ProgressEvent::new(
            Ulid::new(),
            self.run_id,
            timestamp,
            states,
            percent,
            throughput,
            workers.get(),
            eta_seconds,
        )?;

        self.last_event_time = timestamp;
        self.last_states_explored = states;
        self.last_throughput = throughput;

        Ok(event)
    }

    fn compute_throughput(&self, states: u128, timestamp: DateTime<Utc>) -> f64 {
        let delta_states = states.saturating_sub(self.last_states_explored) as f64;
        let delta = timestamp.signed_duration_since(self.last_event_time);
        let delta_secs = duration_to_seconds(delta);

        if delta_secs > f64::EPSILON {
            let rate = delta_states / delta_secs;
            return normalize_rate(rate, self.last_throughput);
        }

        let overall_elapsed = timestamp.signed_duration_since(self.start_time);
        let overall_secs = duration_to_seconds(overall_elapsed);
        if overall_secs > f64::EPSILON {
            let rate = states as f64 / overall_secs;
            return normalize_rate(rate, self.last_throughput);
        }

        self.last_throughput
    }

    fn estimate_eta(&self, percent: f32, timestamp: DateTime<Utc>, throughput: f64) -> Option<u64> {
        if percent <= 0.0 || percent >= 100.0 {
            return None;
        }
        if throughput <= f64::EPSILON {
            return None;
        }

        let elapsed = timestamp.signed_duration_since(self.start_time);
        let elapsed_secs = duration_to_seconds(elapsed);
        if elapsed_secs <= f64::EPSILON {
            return None;
        }

        let fraction_complete = f64::from(percent) / 100.0;
        if fraction_complete <= f64::EPSILON {
            return None;
        }

        let total_estimated = elapsed_secs / fraction_complete;
        let remaining = (total_estimated - elapsed_secs).max(0.0);
        Some(remaining.round() as u64)
    }
}

fn normalize_rate(candidate: f64, fallback: f64) -> f64 {
    if candidate.is_finite() && candidate >= 0.0 {
        candidate
    } else {
        fallback.max(0.0)
    }
}

fn duration_to_seconds(duration: ChronoDuration) -> f64 {
    if duration <= ChronoDuration::zero() {
        return 0.0;
    }

    if let Some(nanos) = duration.num_nanoseconds() {
        (nanos as f64) / 1_000_000_000.0
    } else {
        duration.num_seconds() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use chrono::TimeZone;

    #[test]
    fn produces_initial_event_with_defaults() -> Result<()> {
        let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?;
        let workers = NonZeroU16::new(4).expect("non-zero workers");
        let start = Utc
            .with_ymd_and_hms(2025, 11, 2, 3, 25, 45)
            .single()
            .expect("valid timestamp");

        let mut emitter = ProgressEmitter::with_start_time(run_id, workers, start);
        let snapshot = ProgressSnapshot::new(0, 0.0);
        let event = emitter.record_at(snapshot, start)?;

        assert_eq!(event.run_id(), run_id);
        assert_eq!(event.states_explored(), 0);
        assert_eq!(event.percent_complete(), 0.0);
        assert_eq!(event.throughput_eps(), 0.0);
        assert_eq!(event.workers_active(), workers.get());
        assert_eq!(event.eta_seconds(), None);

        Ok(())
    }

    #[test]
    fn computes_throughput_and_eta_from_snapshots() -> Result<()> {
        let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?;
        let workers = NonZeroU16::new(6).expect("non-zero workers");
        let start = Utc
            .with_ymd_and_hms(2025, 11, 2, 3, 25, 45)
            .single()
            .expect("valid timestamp");

        let mut emitter = ProgressEmitter::with_start_time(run_id, workers, start);

        let first_snapshot = ProgressSnapshot::new(10, 5.0).with_workers_active(workers);
        let first_event = emitter.record_at(first_snapshot, start)?;
        assert_eq!(first_event.throughput_eps(), 0.0);
        assert_eq!(first_event.eta_seconds(), None);

        let later = start + ChronoDuration::seconds(10);
        let second_snapshot = ProgressSnapshot::new(100, 50.0);
        let second_event = emitter.record_at(second_snapshot, later)?;

        assert_eq!(second_event.states_explored(), 100);
        assert!((second_event.throughput_eps() - 9.0).abs() < 1e-6);
        assert_eq!(second_event.workers_active(), workers.get());
        assert_eq!(second_event.percent_complete(), 50.0);
        assert_eq!(second_event.eta_seconds(), Some(10));

        Ok(())
    }

    #[test]
    fn clamps_percent_and_preserves_throughput_on_zero_delta_time() -> Result<()> {
        let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?;
        let workers = NonZeroU16::new(2).expect("non-zero workers");
        let start = Utc
            .with_ymd_and_hms(2025, 11, 2, 3, 25, 45)
            .single()
            .expect("valid timestamp");
        let mut emitter = ProgressEmitter::with_start_time(run_id, workers, start);

        let first = ProgressSnapshot::new(1_000, 120.0);
        let _ = emitter.record_at(first, start)?;

        // Immediately emit another snapshot—delta time is zero so throughput should remain.
        let second = ProgressSnapshot::new(1_500, -10.0);
        let event = emitter.record_at(second, start)?;

        assert_eq!(event.percent_complete(), 0.0);
        assert_eq!(event.throughput_eps(), 0.0);

        Ok(())
    }

    #[test]
    fn rejects_regressing_state_counts() {
        let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM").expect("ulid");
        let workers = NonZeroU16::new(2).expect("workers");
        let start = Utc
            .with_ymd_and_hms(2025, 11, 2, 3, 25, 45)
            .single()
            .expect("timestamp");

        let mut emitter = ProgressEmitter::with_start_time(run_id, workers, start);
        let first = ProgressSnapshot::new(100, 10.0);
        emitter.record_at(first, start).expect("first event");

        let regressing = ProgressSnapshot::new(90, 12.0);
        let error = emitter
            .record_at(regressing, start + ChronoDuration::seconds(5))
            .expect_err("expected regression error");
        assert!(
            error
                .to_string()
                .contains("states explored cannot decrease"),
            "unexpected error: {error}"
        );
    }
}
