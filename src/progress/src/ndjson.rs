use std::io::Write;
use std::num::NonZeroU16;

use anyhow::{ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Progress update emitted for non-TTY observers.
///
/// Values follow the contract defined in `specs/001-rewrite-tlc/contracts/cli.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgressEvent {
    event_id: Ulid,
    run_id: Ulid,
    timestamp: DateTime<Utc>,
    states_explored: u128,
    percent_complete: f32,
    throughput_eps: f64,
    workers_active: NonZeroU16,
    eta_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProgressEventSerde {
    event_id: Ulid,
    run_id: Ulid,
    timestamp: DateTime<Utc>,
    #[serde(with = "u128_as_string")]
    states_explored: u128,
    percent_complete: f32,
    throughput_eps: f64,
    workers_active: u16,
    #[serde(default)]
    eta_seconds: Option<u64>,
}

impl ProgressEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: Ulid,
        run_id: Ulid,
        timestamp: DateTime<Utc>,
        states_explored: u128,
        percent_complete: f32,
        throughput_eps: f64,
        workers_active: u16,
        eta_seconds: Option<u64>,
    ) -> Result<Self> {
        ensure!(
            (0.0..=100.0).contains(&percent_complete),
            "percent_complete must be within [0, 100] (got {percent_complete})"
        );
        ensure!(
            throughput_eps >= 0.0,
            "throughput_eps cannot be negative (got {throughput_eps})"
        );
        ensure!(
            workers_active > 0,
            "workers_active must be greater than zero (got {workers_active})"
        );
        if let Some(eta) = eta_seconds {
            ensure!(
                eta > 0,
                "eta_seconds must be positive when provided (got {eta})"
            );
        }

        Ok(Self {
            event_id,
            run_id,
            timestamp,
            states_explored,
            percent_complete,
            throughput_eps,
            workers_active: NonZeroU16::new(workers_active)
                .expect("validated non-zero workers_active"),
            eta_seconds,
        })
    }

    pub fn event_id(&self) -> Ulid {
        self.event_id
    }

    pub fn run_id(&self) -> Ulid {
        self.run_id
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

    pub fn throughput_eps(&self) -> f64 {
        self.throughput_eps
    }

    pub fn workers_active(&self) -> u16 {
        self.workers_active.get()
    }

    pub fn eta_seconds(&self) -> Option<u64> {
        self.eta_seconds
    }

    pub fn with_eta_seconds(mut self, eta_seconds: Option<u64>) -> Result<Self> {
        if let Some(eta) = eta_seconds {
            ensure!(
                eta > 0,
                "eta_seconds must be positive when provided (got {eta})"
            );
        }
        self.eta_seconds = eta_seconds;
        Ok(self)
    }
}

impl Serialize for ProgressEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ProgressEventSerde {
            event_id: self.event_id,
            run_id: self.run_id,
            timestamp: self.timestamp,
            states_explored: self.states_explored,
            percent_complete: self.percent_complete,
            throughput_eps: self.throughput_eps,
            workers_active: self.workers_active.get(),
            eta_seconds: self.eta_seconds,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ProgressEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = ProgressEventSerde::deserialize(deserializer)?;
        ProgressEvent::new(
            raw.event_id,
            raw.run_id,
            raw.timestamp,
            raw.states_explored,
            raw.percent_complete,
            raw.throughput_eps,
            raw.workers_active,
            raw.eta_seconds,
        )
        .map_err(serde::de::Error::custom)
    }
}

pub struct NdjsonWriter<W: Write> {
    sink: W,
}

impl<W: Write> NdjsonWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { sink: writer }
    }

    pub fn write_event(&mut self, event: &ProgressEvent) -> Result<()> {
        let line = serde_json::to_string(event).context("failed to serialize progress event")?;
        self.sink
            .write_all(line.as_bytes())
            .context("failed to write progress event line")?;
        self.sink
            .write_all(b"\n")
            .context("failed to write progress event newline")?;
        self.sink
            .flush()
            .context("failed to flush progress writer")?;
        Ok(())
    }

    pub fn into_inner(self) -> W {
        self.sink
    }
}

mod u128_as_string {
    use serde::{de::Error as DeError, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        text.parse::<u128>()
            .map_err(|err| DeError::custom(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use serde_json::Value;
    use std::io::Cursor;

    #[test]
    fn serialization_matches_contract() -> Result<()> {
        let event = ProgressEvent::new(
            Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJK")?,
            Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?,
            "2025-11-02T03:25:45.123Z".parse::<DateTime<Utc>>()?,
            18_446_744_073_709_551_616,
            62.5,
            125_000.0,
            15,
            Some(5_400),
        )?;
        let json = serde_json::to_string(&event)?;
        let value: Value = serde_json::from_str(&json)?;

        assert_eq!(
            value
                .get("states_explored")
                .and_then(Value::as_str)
                .expect("states_explored present"),
            "18446744073709551616"
        );
        assert_eq!(
            value
                .get("eta_seconds")
                .expect("eta_seconds present")
                .as_u64()
                .expect("eta_seconds is integer"),
            5_400
        );

        Ok(())
    }

    #[test]
    fn writer_emits_newline_delimited_json() -> Result<()> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = NdjsonWriter::new(&mut buffer);

        let event = ProgressEvent::new(
            Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJN")?,
            Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?,
            "2025-11-02T03:26:15.987Z".parse::<DateTime<Utc>>()?,
            18_446_744_073_719_551_616,
            63.0,
            124_850.5,
            15,
            None,
        )?;

        writer.write_event(&event)?;

        let output = String::from_utf8(buffer.into_inner())?;
        assert!(
            output.ends_with('\n'),
            "writer must terminate each event with newline"
        );

        let line = output.trim_end_matches('\n');
        let round_trip: ProgressEvent = serde_json::from_str(line)?;
        assert_eq!(round_trip, event);

        Ok(())
    }

    #[test]
    fn percent_complete_out_of_bounds_rejected() {
        let result = ProgressEvent::new(
            Ulid::new(),
            Ulid::new(),
            Utc::now(),
            10,
            150.0,
            100.0,
            1,
            None,
        );
        assert!(result.is_err());
    }
}
