//! Telemetry helpers for emitting run-level metrics.

use std::time::{Duration, Instant};

use ulid::Ulid;

const METRICS_TARGET: &str = "tlc::metrics";
const METRICS_NAME: &str = "run_metrics";

/// Snapshot describing aggregate metrics for a completed TLC run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunMetrics {
    run_id: Ulid,
    runtime: Duration,
    states_explored: u128,
    peak_memory_bytes: Option<u64>,
}

impl RunMetrics {
    /// Construct a new metrics snapshot with optional peak-memory reporting.
    pub fn new(
        run_id: Ulid,
        runtime: Duration,
        states_explored: u128,
        peak_memory_bytes: Option<u64>,
    ) -> Self {
        Self {
            run_id,
            runtime,
            states_explored,
            peak_memory_bytes,
        }
    }

    /// Identifier of the run these metrics describe.
    pub fn run_id(&self) -> Ulid {
        self.run_id
    }

    /// Wall-clock runtime of the exploration.
    pub fn runtime(&self) -> Duration {
        self.runtime
    }

    /// Total number of states explored by the run.
    pub fn states_explored(&self) -> u128 {
        self.states_explored
    }

    /// Peak memory consumption reported during the run, if captured.
    pub fn peak_memory_bytes(&self) -> Option<u64> {
        self.peak_memory_bytes
    }

    /// Derived throughput in states-per-second.
    pub fn states_per_second(&self) -> f64 {
        throughput(self.states_explored, self.runtime)
    }

    /// Emit the metrics snapshot as a structured tracing event.
    pub fn emit(&self) {
        let runtime_ms = self.runtime.as_secs_f64() * 1_000.0;
        let throughput = self.states_per_second();
        let peak_bytes = self.peak_memory_bytes.unwrap_or_default();
        let peak_reported = self.peak_memory_bytes.is_some();

        tracing::info!(
            target: METRICS_TARGET,
            metric = METRICS_NAME,
            run_id = %self.run_id,
            runtime_ms,
            states_explored = %self.states_explored,
            states_per_second = throughput,
            peak_memory_bytes = peak_bytes,
            peak_memory_reported = peak_reported,
            "run metrics recorded"
        );
    }
}

/// Helper that records runtime metadata and emits run-level metrics on completion.
#[derive(Debug, Clone)]
pub struct RunMetricsRecorder {
    run_id: Ulid,
    started_at: Instant,
    peak_memory_bytes: Option<u64>,
}

impl RunMetricsRecorder {
    /// Create a new metrics recorder starting at the current instant.
    pub fn start(run_id: Ulid) -> Self {
        Self::with_start_instant(run_id, Instant::now())
    }

    /// Create a metrics recorder with an explicit start instant (primarily for testing).
    pub fn with_start_instant(run_id: Ulid, started_at: Instant) -> Self {
        Self {
            run_id,
            started_at,
            peak_memory_bytes: None,
        }
    }

    /// Record an observed heap or resident-set size sample in bytes.
    pub fn record_memory_sample(&mut self, bytes: u64) {
        self.peak_memory_bytes = Some(
            self.peak_memory_bytes
                .map_or(bytes, |current| current.max(bytes)),
        );
    }

    /// Finalize the recorder, emitting and returning the aggregated metrics.
    pub fn finish(self, states_explored: u128) -> RunMetrics {
        let runtime = self.started_at.elapsed();
        let metrics = RunMetrics::new(
            self.run_id,
            runtime,
            states_explored,
            self.peak_memory_bytes,
        );
        metrics.emit();
        metrics
    }
}

fn throughput(states: u128, runtime: Duration) -> f64 {
    let seconds = runtime.as_secs_f64();
    if seconds <= f64::EPSILON {
        return states as f64;
    }
    (states as f64) / seconds
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };
    use tracing::{subscriber::with_default, Level, Subscriber};
    use tracing_subscriber::{
        layer::{Context, Layer, SubscriberExt},
        registry,
    };

    #[test]
    fn computes_states_per_second_from_runtime() {
        let run_id = Ulid::new();
        let metrics = RunMetrics::new(
            run_id,
            Duration::from_millis(1500),
            45_000,
            Some(2 * 1024 * 1024),
        );

        let expected = 30_000.0;
        let actual = metrics.states_per_second();
        let delta = (expected - actual).abs();
        assert!(delta < 1e-6, "expected throughput {expected}, got {actual}");
    }

    #[test]
    fn emit_logs_structured_metrics_event() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let layer = RecordingLayer::new(events.clone());
        let subscriber = registry().with(layer);

        let run_id = Ulid::new();
        let metrics = RunMetrics::new(run_id, Duration::from_secs(2), 10_000, Some(1_048_576));

        with_default(subscriber, || {
            metrics.emit();
        });

        let recorded = events.lock().expect("lock events");
        let event = recorded
            .iter()
            .find(|event| event.target == METRICS_TARGET)
            .expect("metrics event recorded");

        assert_eq!(event.level, Level::INFO);
        assert_eq!(event.fields.get("metric"), Some(&METRICS_NAME.to_string()));
        assert_eq!(event.fields.get("run_id"), Some(&run_id.to_string()));

        let runtime_ms: f64 = event
            .fields
            .get("runtime_ms")
            .expect("runtime_ms field")
            .parse()
            .expect("runtime_ms parse");
        assert!(
            runtime_ms >= 2000.0,
            "expected runtime >=2000ms, saw {runtime_ms}"
        );

        let throughput: f64 = event
            .fields
            .get("states_per_second")
            .expect("states_per_second field")
            .parse()
            .expect("states_per_second parse");
        assert!(throughput > 0.0);

        assert_eq!(
            event.fields.get("peak_memory_bytes"),
            Some(&"1048576".to_string())
        );
        assert_eq!(
            event.fields.get("peak_memory_reported"),
            Some(&"true".to_string())
        );
        assert!(
            event.fields.get("states_explored").is_some(),
            "states_explored recorded"
        );
    }

    #[derive(Debug)]
    struct RecordedEvent {
        target: String,
        level: Level,
        fields: BTreeMap<String, String>,
    }

    #[derive(Clone)]
    struct RecordingLayer {
        events: Arc<Mutex<Vec<RecordedEvent>>>,
    }

    impl RecordingLayer {
        fn new(events: Arc<Mutex<Vec<RecordedEvent>>>) -> Self {
            Self { events }
        }
    }

    impl<S> Layer<S> for RecordingLayer
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = FieldVisitor::default();
            event.record(&mut visitor);
            let metadata = event.metadata();
            let mut events = self.events.lock().expect("lock events");
            events.push(RecordedEvent {
                target: metadata.target().to_string(),
                level: *metadata.level(),
                fields: visitor.fields,
            });
        }
    }

    #[derive(Default)]
    struct FieldVisitor {
        fields: BTreeMap<String, String>,
    }

    impl tracing::field::Visit for FieldVisitor {
        fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }
}
