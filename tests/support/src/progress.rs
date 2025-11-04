use anyhow::{anyhow, ensure, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Deserialize;
use tlc_progress::ProgressEvent;

/// Progress thresholds covering refresh cadence and coverage accuracy.
#[derive(Debug, Clone, Copy)]
pub struct ProgressThresholds {
    pub max_refresh_gap: ChronoDuration,
    pub max_coverage_delta: f32,
}

impl Default for ProgressThresholds {
    fn default() -> Self {
        Self {
            max_refresh_gap: ChronoDuration::seconds(5),
            max_coverage_delta: 2.0,
        }
    }
}

/// Normalized progress sample used by the analysis helpers.
#[derive(Debug, Clone)]
pub struct ProgressSample {
    timestamp: DateTime<Utc>,
    states_explored: u128,
    percent_complete: f32,
}

impl ProgressSample {
    pub fn new(timestamp: DateTime<Utc>, states_explored: u128, percent_complete: f32) -> Self {
        Self {
            timestamp,
            states_explored,
            percent_complete,
        }
    }

    pub fn timestamp(&self) -> DateTime<Utc> {
        self.timestamp
    }

    pub fn states_explored(&self) -> u128 {
        self.states_explored
    }

    pub fn percent_complete(&self) -> f32 {
        self.percent_complete
    }
}

impl From<&ProgressEvent> for ProgressSample {
    fn from(event: &ProgressEvent) -> Self {
        Self {
            timestamp: event.timestamp(),
            states_explored: event.states_explored(),
            percent_complete: event.percent_complete(),
        }
    }
}

impl From<ProgressEvent> for ProgressSample {
    fn from(event: ProgressEvent) -> Self {
        Self::from(&event)
    }
}

/// Aggregated analysis describing progress refresh cadence and accuracy.
#[derive(Debug, Clone)]
pub struct ProgressRunAnalysis {
    event_count: usize,
    max_refresh_gap: ChronoDuration,
    final_reported_percent: f32,
    actual_percent: f32,
}

impl ProgressRunAnalysis {
    pub fn event_count(&self) -> usize {
        self.event_count
    }

    pub fn max_refresh_gap(&self) -> ChronoDuration {
        self.max_refresh_gap
    }

    pub fn final_reported_percent(&self) -> f32 {
        self.final_reported_percent
    }

    pub fn actual_percent(&self) -> f32 {
        self.actual_percent
    }

    pub fn coverage_delta(&self) -> f32 {
        (self.final_reported_percent - self.actual_percent).abs()
    }

    pub fn meets(&self, thresholds: &ProgressThresholds) -> bool {
        self.max_refresh_gap <= thresholds.max_refresh_gap
            && self.coverage_delta() <= thresholds.max_coverage_delta
    }
}

/// Analyze progress samples and report refresh cadence plus coverage accuracy.
pub fn analyze_progress(
    samples: &[ProgressSample],
    expected_total_states: u128,
) -> Result<ProgressRunAnalysis> {
    ensure!(
        !samples.is_empty(),
        "progress validation requires at least one event"
    );
    ensure!(
        expected_total_states > 0,
        "expected_total_states must be greater than zero"
    );

    let mut max_gap = ChronoDuration::zero();
    let mut previous_timestamp = samples[0].timestamp();
    let mut final_states = samples[0].states_explored();
    let mut final_percent = samples[0].percent_complete();

    for sample in samples.iter().skip(1) {
        let delta = sample.timestamp().signed_duration_since(previous_timestamp);
        ensure!(
            delta >= ChronoDuration::zero(),
            "progress events must be ordered by timestamp"
        );

        if delta > max_gap {
            max_gap = delta;
        }

        previous_timestamp = sample.timestamp();
        final_states = sample.states_explored();
        final_percent = sample.percent_complete();
    }

    let actual_fraction =
        (final_states.min(expected_total_states) as f64) / (expected_total_states as f64);
    let actual_percent = (actual_fraction * 100.0).min(100.0) as f32;

    Ok(ProgressRunAnalysis {
        event_count: samples.len(),
        max_refresh_gap: max_gap,
        final_reported_percent: final_percent,
        actual_percent,
    })
}

/// Attempt to parse progress NDJSON output, returning the samples discovered.
pub fn parse_progress_ndjson(source: &str) -> Result<Vec<ProgressSample>> {
    let mut samples = Vec::new();

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<WireProgressEvent>(trimmed) {
            Ok(event) => samples.push(ProgressSample::new(
                event.timestamp,
                event.states_explored,
                event.percent_complete,
            )),
            Err(error) => {
                // Skip non-JSON diagnostics but fail for malformed JSON objects.
                if trimmed.starts_with('{') {
                    return Err(anyhow!(
                        "failed to parse progress event on line {}: {error}",
                        index + 1
                    ));
                }
            }
        }
    }

    if samples.is_empty() {
        return Err(anyhow!("no progress events parsed from NDJSON output"));
    }

    Ok(samples)
}

#[derive(Debug, Deserialize)]
struct WireProgressEvent {
    timestamp: DateTime<Utc>,
    #[serde(with = "u128_as_string")]
    states_explored: u128,
    percent_complete: f32,
}

mod u128_as_string {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse::<u128>()
            .map_err(|_| serde::de::Error::custom("states_explored must be u128 encoded as string"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tlc_progress::{NdjsonWriter, ProgressEvent as NdjsonEvent};
    use ulid::Ulid;

    fn build_sample_events() -> Result<(Vec<ProgressSample>, u128)> {
        let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?;
        let start = Utc
            .with_ymd_and_hms(2025, 11, 2, 3, 45, 0)
            .single()
            .expect("valid start");

        let mut writer = NdjsonWriter::new(Vec::new());
        let expected_total = 80_000_u128;
        let events = [
            (0, 0, 0.0),
            (3, 12_000, 15.0),
            (7, 32_000, 40.0),
            (10, 52_000, 65.0),
            (14, 80_000, 100.0),
        ];

        for (offset, states, percent) in events {
            let timestamp = start + ChronoDuration::seconds(offset);
            let event = NdjsonEvent::new(
                Ulid::new(),
                run_id,
                timestamp,
                states,
                percent,
                12.5,
                6,
                None,
            )?;
            writer.write_event(&event)?;
        }

        let payload = String::from_utf8(writer.into_inner()).expect("utf8");
        let samples = parse_progress_ndjson(&payload)?;

        Ok((samples, expected_total))
    }

    #[test]
    fn analyzes_progress_within_thresholds() -> Result<()> {
        let (samples, expected_total) = build_sample_events()?;
        let thresholds = ProgressThresholds::default();
        let analysis = analyze_progress(&samples, expected_total)?;

        assert_eq!(analysis.event_count(), samples.len());
        assert!(analysis.meets(&thresholds));
        assert_eq!(analysis.coverage_delta(), 0.0);

        Ok(())
    }

    #[test]
    fn detect_progress_violations() -> Result<()> {
        let mut samples = Vec::new();
        let start = Utc
            .with_ymd_and_hms(2025, 11, 2, 4, 0, 0)
            .single()
            .expect("valid start");

        samples.push(ProgressSample::new(start, 0, 0.0));
        samples.push(ProgressSample::new(
            start + ChronoDuration::seconds(12),
            40_000,
            45.0,
        ));
        samples.push(ProgressSample::new(
            start + ChronoDuration::seconds(35),
            80_000,
            85.0,
        ));

        let analysis = analyze_progress(&samples, 80_000)?;
        let thresholds = ProgressThresholds::default();

        assert!(!analysis.meets(&thresholds));
        assert!(analysis.max_refresh_gap() > thresholds.max_refresh_gap);
        assert!(analysis.coverage_delta() > thresholds.max_coverage_delta);

        Ok(())
    }

    #[test]
    fn parse_ndjson_skips_non_json_lines_and_errors_on_invalid_objects() -> Result<()> {
        let payload = r#"
{"timestamp":1730511900,"states_explored":"1000","percent_complete":10.0}
non-json diagnostic
{"timestamp":1730511905,"states_explored":"2000","percent_complete":25.0}
{"timestamp":1730511910,"states_explored":"oops","percent_complete":40.0}
"#;

        let error = parse_progress_ndjson(payload).expect_err("should fail parsing invalid event");
        assert!(
            error.to_string().contains("failed to parse progress event"),
            "expected parsing error, got: {error:#}"
        );

        Ok(())
    }

    #[test]
    fn parse_ndjson_errors_when_no_events() {
        let error = parse_progress_ndjson("diagnostic-only").expect_err("no events should fail");
        assert!(
            error.to_string().contains("no progress events"),
            "expected missing progress events error, got: {error:#}"
        );
    }
}
