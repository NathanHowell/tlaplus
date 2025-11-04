//! Worker utilization telemetry with configurable alert thresholds.
use std::num::NonZeroUsize;

use thiserror::Error;
use tracing::Level;

const WORKER_TELEMETRY_TARGET: &str = "tlc::workers";
const WORKER_UTILIZATION_METRIC: &str = "worker_utilization";
const DEFAULT_ALERT_THRESHOLD: f64 = 0.70;

/// Errors surfaced while configuring worker telemetry.
#[derive(Debug, Error, PartialEq)]
pub enum WorkerTelemetryError {
    /// Alert threshold must live within the open interval `(0.0, 1.0]`.
    #[error("alert threshold must be within (0.0, 1.0]; received {0}")]
    InvalidAlertThreshold(f64),
}

/// Result alias for worker telemetry operations.
pub type Result<T> = std::result::Result<T, WorkerTelemetryError>;

/// Snapshot describing queue state for telemetry purposes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueueTelemetry {
    len: usize,
    capacity: Option<usize>,
    saturation: Option<f64>,
}

impl QueueTelemetry {
    /// Create a snapshot describing the queue with an optional capacity and saturation.
    pub fn new(len: usize, capacity: Option<usize>, saturation: Option<f64>) -> Self {
        Self {
            len,
            capacity,
            saturation: sanitize_saturation(saturation),
        }
    }

    /// Construct a bounded queue snapshot with the provided capacity.
    pub fn bounded(len: usize, capacity: usize, saturation: Option<f64>) -> Self {
        Self::new(len, Some(capacity), saturation)
    }

    /// Construct an unbounded queue snapshot.
    pub fn unbounded(len: usize, saturation: Option<f64>) -> Self {
        Self::new(len, None, saturation)
    }

    /// Items currently buffered.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Queue capacity when bounded.
    pub fn capacity(&self) -> Option<usize> {
        self.capacity
    }

    /// Saturation ratio in `[0.0, 1.0]` when known.
    pub fn saturation(&self) -> Option<f64> {
        self.saturation
    }

    /// Indicates whether the queue enforces a bounded capacity.
    pub fn is_bounded(&self) -> bool {
        self.capacity.is_some()
    }
}

/// Sample describing active workers and optional queue telemetry.
#[derive(Debug, Clone, Copy)]
pub struct WorkerSample {
    active_workers: usize,
    queue: Option<QueueTelemetry>,
}

impl WorkerSample {
    /// Create a sample with the specified active worker count.
    pub fn new(active_workers: usize) -> Self {
        Self {
            active_workers,
            queue: None,
        }
    }

    /// Attach queue telemetry to the sample.
    pub fn with_queue(mut self, queue: QueueTelemetry) -> Self {
        self.queue = Some(queue);
        self
    }

    fn active_workers(&self) -> usize {
        self.active_workers
    }

    fn queue(&self) -> Option<QueueTelemetry> {
        self.queue
    }
}

/// Aggregated telemetry details produced after recording a sample.
#[derive(Debug, Clone, Copy)]
pub struct WorkerUtilizationReport {
    pub sample_index: usize,
    pub active_workers: usize,
    pub total_workers: NonZeroUsize,
    pub utilization: f64,
    pub average_utilization: f64,
    pub alert_threshold: f64,
    pub alert_active: bool,
    pub alert_triggered: bool,
    pub alert_cleared: bool,
    pub queue: Option<QueueTelemetry>,
}

impl WorkerUtilizationReport {
    /// Returns `true` when the utilization alert is currently active.
    pub fn is_alerting(&self) -> bool {
        self.alert_active
    }
}

/// Telemetry helper that tracks worker utilization and emits alerts when thresholds are violated.
#[derive(Debug)]
pub struct WorkerTelemetry {
    total_workers: NonZeroUsize,
    alert_threshold: f64,
    sample_count: usize,
    utilization_sum: f64,
    alert_active: bool,
}

impl WorkerTelemetry {
    /// Construct telemetry with the default 70% alert threshold.
    pub fn new(total_workers: NonZeroUsize) -> Result<Self> {
        Self::with_threshold(total_workers, DEFAULT_ALERT_THRESHOLD)
    }

    /// Construct telemetry with a custom alert threshold in `(0.0, 1.0]`.
    pub fn with_threshold(total_workers: NonZeroUsize, alert_threshold: f64) -> Result<Self> {
        if !alert_threshold.is_finite() || alert_threshold <= 0.0 || alert_threshold > 1.0 {
            return Err(WorkerTelemetryError::InvalidAlertThreshold(alert_threshold));
        }

        Ok(Self {
            total_workers,
            alert_threshold,
            sample_count: 0,
            utilization_sum: 0.0,
            alert_active: false,
        })
    }

    /// Total worker threads being monitored.
    pub fn total_workers(&self) -> NonZeroUsize {
        self.total_workers
    }

    /// Active alert threshold expressed as a ratio in `(0.0, 1.0]`.
    pub fn alert_threshold(&self) -> f64 {
        self.alert_threshold
    }

    /// Record a worker utilization sample, emitting telemetry events and returning the aggregated
    /// report.
    pub fn record_sample(&mut self, sample: WorkerSample) -> WorkerUtilizationReport {
        let total = self.total_workers.get();
        let active = sample.active_workers().min(total);
        let utilization = (active as f64 / total as f64).min(1.0);

        self.sample_count += 1;
        self.utilization_sum += utilization;
        let average = self.utilization_sum / self.sample_count as f64;

        let queue = sample.queue();
        let queue_len = queue.map(|q| q.len()).unwrap_or_default();
        let queue_capacity = queue.and_then(|q| q.capacity()).unwrap_or_default();
        let queue_bounded = queue.map(|q| q.is_bounded()).unwrap_or(false);
        let saturation = queue.and_then(|q| q.saturation()).unwrap_or_default();
        let saturation_reported = queue.and_then(|q| q.saturation()).is_some();

        let is_alert = utilization < self.alert_threshold;
        let mut alert_triggered = false;
        let mut alert_cleared = false;

        tracing::info!(
            target: WORKER_TELEMETRY_TARGET,
            metric = WORKER_UTILIZATION_METRIC,
            workers_active = active,
            workers_total = total,
            utilization,
            average_utilization = average,
            alert_threshold = self.alert_threshold,
            alert = is_alert,
            sample_index = self.sample_count as u64,
            queue_len = queue_len as u64,
            queue_capacity = queue_capacity as u64,
            queue_bounded,
            queue_reported = queue.is_some(),
            queue_saturation = saturation,
            queue_saturation_reported = saturation_reported,
            "worker utilization sample recorded"
        );

        if is_alert && !self.alert_active {
            self.alert_active = true;
            alert_triggered = true;
            emit_alert_event(
                Level::WARN,
                "worker utilization below alert threshold",
                active,
                total,
                utilization,
                average,
                self.alert_threshold,
                queue_len,
                queue_capacity,
                queue_bounded,
                saturation,
                saturation_reported,
            );
        } else if !is_alert && self.alert_active {
            self.alert_active = false;
            alert_cleared = true;
            emit_alert_event(
                Level::INFO,
                "worker utilization recovered above alert threshold",
                active,
                total,
                utilization,
                average,
                self.alert_threshold,
                queue_len,
                queue_capacity,
                queue_bounded,
                saturation,
                saturation_reported,
            );
        }

        WorkerUtilizationReport {
            sample_index: self.sample_count,
            active_workers: active,
            total_workers: self.total_workers,
            utilization,
            average_utilization: average,
            alert_threshold: self.alert_threshold,
            alert_active: self.alert_active,
            alert_triggered,
            alert_cleared,
            queue,
        }
    }
}

fn emit_alert_event(
    level: Level,
    message: &str,
    active: usize,
    total: usize,
    utilization: f64,
    average: f64,
    threshold: f64,
    queue_len: usize,
    queue_capacity: usize,
    queue_bounded: bool,
    saturation: f64,
    saturation_reported: bool,
) {
    match level {
        Level::WARN => {
            tracing::warn!(
                target: WORKER_TELEMETRY_TARGET,
                metric = WORKER_UTILIZATION_METRIC,
                workers_active = active,
                workers_total = total,
                utilization,
                average_utilization = average,
                alert_threshold = threshold,
                queue_len = queue_len as u64,
                queue_capacity = queue_capacity as u64,
                queue_bounded,
                queue_saturation = saturation,
                queue_saturation_reported = saturation_reported,
                "{message}"
            );
        }
        Level::INFO => {
            tracing::info!(
                target: WORKER_TELEMETRY_TARGET,
                metric = WORKER_UTILIZATION_METRIC,
                workers_active = active,
                workers_total = total,
                utilization,
                average_utilization = average,
                alert_threshold = threshold,
                queue_len = queue_len as u64,
                queue_capacity = queue_capacity as u64,
                queue_bounded,
                queue_saturation = saturation,
                queue_saturation_reported = saturation_reported,
                "{message}"
            );
        }
        _ => {}
    }
}

fn sanitize_saturation(value: Option<f64>) -> Option<f64> {
    value.and_then(|sample| {
        if sample.is_finite() {
            Some(sample.clamp(0.0, 1.0))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::sync::{Arc, Mutex};
    use tracing::subscriber::with_default;
    use tracing::Subscriber;
    use tracing_subscriber::{
        layer::{Context, Layer, SubscriberExt},
        registry,
    };

    #[test]
    fn rejects_invalid_threshold() {
        let workers = NonZeroUsize::new(4).unwrap();
        match WorkerTelemetry::with_threshold(workers, 0.0) {
            Err(WorkerTelemetryError::InvalidAlertThreshold(value)) => {
                assert_eq!(value, 0.0);
            }
            outcome => panic!("expected invalid threshold error, saw {outcome:?}"),
        }
        match WorkerTelemetry::with_threshold(workers, 1.2) {
            Err(WorkerTelemetryError::InvalidAlertThreshold(value)) => {
                assert_eq!(value, 1.2);
            }
            outcome => panic!("expected invalid threshold error, saw {outcome:?}"),
        }
        match WorkerTelemetry::with_threshold(workers, f64::NAN) {
            Err(WorkerTelemetryError::InvalidAlertThreshold(value)) => {
                assert!(value.is_nan());
            }
            outcome => panic!("expected invalid threshold error, saw {outcome:?}"),
        }
    }

    #[test]
    fn records_sample_and_emits_info_event() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = registry().with(RecordingLayer::new(events.clone()));

        let workers = NonZeroUsize::new(4).unwrap();
        let mut telemetry = WorkerTelemetry::new(workers).expect("default threshold");

        let queue = QueueTelemetry::bounded(3, 8, Some(0.4));
        let report = with_default(subscriber, || {
            telemetry.record_sample(WorkerSample::new(3).with_queue(queue))
        });

        assert_eq!(report.sample_index, 1);
        assert!((report.utilization - 0.75).abs() < 1e-10);
        assert!((report.average_utilization - 0.75).abs() < 1e-10);
        assert!(!report.alert_active);
        assert!(!report.alert_triggered);
        assert!(report.queue.is_some());

        let recorded = events.lock().expect("lock events");
        let info_event = recorded
            .iter()
            .find(|event| {
                event.fields.get("message")
                    == Some(&"worker utilization sample recorded".to_string())
            })
            .expect("info event recorded");

        assert_eq!(info_event.level, Level::INFO);
        assert_eq!(info_event.target, WORKER_TELEMETRY_TARGET);
        assert_eq!(
            info_event.fields.get("metric"),
            Some(&WORKER_UTILIZATION_METRIC.to_string())
        );
        assert_eq!(
            info_event.fields.get("workers_active"),
            Some(&"3".to_string())
        );
        assert_eq!(
            info_event.fields.get("workers_total"),
            Some(&"4".to_string())
        );
        assert_eq!(
            info_event.fields.get("queue_bounded"),
            Some(&"true".to_string())
        );
        assert_eq!(info_event.fields.get("queue_len"), Some(&"3".to_string()));
        assert_eq!(
            info_event.fields.get("queue_reported"),
            Some(&"true".to_string())
        );
        assert_eq!(
            info_event.fields.get("queue_saturation_reported"),
            Some(&"true".to_string())
        );
        let utilization: f64 = info_event
            .fields
            .get("utilization")
            .expect("utilization recorded")
            .parse()
            .expect("utilization parse");
        assert!((utilization - 0.75).abs() < 1e-10);
    }

    #[test]
    fn triggers_alert_and_recovery_events() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let make_subscriber = || registry().with(RecordingLayer::new(events.clone()));

        let workers = NonZeroUsize::new(4).unwrap();
        let mut telemetry =
            WorkerTelemetry::with_threshold(workers, 0.75).expect("custom threshold");

        let report_alert = with_default(make_subscriber(), || {
            telemetry.record_sample(WorkerSample::new(2))
        });
        assert!(report_alert.alert_triggered);
        assert!(report_alert.alert_active);
        assert!(!report_alert.alert_cleared);

        let report_recovery = with_default(make_subscriber(), || {
            telemetry.record_sample(WorkerSample::new(3))
        });
        assert!(report_recovery.alert_cleared);
        assert!(!report_recovery.alert_active);
        assert!(!report_recovery.alert_triggered);

        let recorded = events.lock().expect("lock events");
        let warn_event = recorded
            .iter()
            .find(|event| event.level == Level::WARN)
            .expect("warn event present");
        assert_eq!(
            warn_event.fields.get("message"),
            Some(&"worker utilization below alert threshold".to_string())
        );
        assert_eq!(
            warn_event.fields.get("workers_active"),
            Some(&"2".to_string())
        );

        let recovery_event = recorded
            .iter()
            .find(|event| {
                event.fields.get("message")
                    == Some(&"worker utilization recovered above alert threshold".to_string())
            })
            .expect("recovery event present");
        assert_eq!(recovery_event.level, Level::INFO);
        assert_eq!(
            recovery_event.fields.get("workers_active"),
            Some(&"3".to_string())
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

        fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
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
