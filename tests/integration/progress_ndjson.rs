use std::str::FromStr;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use ulid::Ulid;

const PROGRESS_NDJSON_FIXTURE: &str = r#"{"event_id":"01J0Y6M4F2A5B7C8D9E0F1GHJK","run_id":"01J0Y6M4F2A5B7C8D9E0F1GHJM","timestamp":"2025-11-02T03:25:45.123Z","states_explored":"18446744073709551616","percent_complete":62.5,"throughput_eps":125000.0,"workers_active":15,"eta_seconds":5400}
{"event_id":"01J0Y6M4F2A5B7C8D9E0F1GHJN","run_id":"01J0Y6M4F2A5B7C8D9E0F1GHJM","timestamp":"2025-11-02T03:26:15.987Z","states_explored":"18446744073719551616","percent_complete":63.0,"throughput_eps":124850.5,"workers_active":15}"#;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressEventContract {
    event_id: String,
    run_id: String,
    timestamp: String,
    states_explored: String,
    percent_complete: f32,
    throughput_eps: f64,
    workers_active: u16,
    #[serde(default)]
    eta_seconds: Option<u64>,
}

impl ProgressEventContract {
    fn validate(&self) -> Result<()> {
        Ulid::from_str(&self.event_id)
            .with_context(|| format!("invalid ULID for event_id: {}", self.event_id))?;
        Ulid::from_str(&self.run_id)
            .with_context(|| format!("invalid ULID for run_id: {}", self.run_id))?;

        let timestamp = DateTime::parse_from_rfc3339(&self.timestamp)
            .with_context(|| format!("timestamp must be RFC3339: {}", self.timestamp))?;
        let _utc: DateTime<Utc> = timestamp.with_timezone(&Utc);

        u128::from_str(&self.states_explored).with_context(|| {
            format!(
                "states_explored must encode an integer as a string: {}",
                self.states_explored
            )
        })?;

        ensure!(
            (0.0..=100.0).contains(&self.percent_complete),
            "percent_complete must live within [0, 100]"
        );
        ensure!(
            self.throughput_eps >= 0.0,
            "throughput_eps should not be negative"
        );
        ensure!(
            self.workers_active > 0,
            "workers_active must report at least one active worker"
        );
        if let Some(eta) = self.eta_seconds {
            ensure!(eta > 0, "eta_seconds should be positive when present");
        }

        Ok(())
    }
}

#[test]
fn progress_ndjson_schema_is_stable() -> Result<()> {
    for (index, line) in PROGRESS_NDJSON_FIXTURE.lines().enumerate() {
        let raw: Value = serde_json::from_str(line)
            .with_context(|| format!("line {} failed to parse as JSON: {}", index + 1, line))?;

        ensure!(
            raw.get("event_id").and_then(Value::as_str).is_some(),
            "event_id must be present as a string"
        );
        ensure!(
            raw.get("run_id").and_then(Value::as_str).is_some(),
            "run_id must be present as a string"
        );
        ensure!(
            raw.get("states_explored").and_then(Value::as_str).is_some(),
            "states_explored must be encoded as a string to preserve precision"
        );

        let event: ProgressEventContract = serde_json::from_value(raw.clone())
            .with_context(|| format!("line {} failed contract deserialization", index + 1))?;
        event.validate()?;
    }

    Ok(())
}
