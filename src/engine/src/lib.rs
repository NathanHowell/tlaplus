//! Core exploration engine entrypoint and invariants for the TLC rewrite.

use std::{num::NonZeroUsize, path::PathBuf, time::Duration};

use anyhow::Result as AnyhowResult;
use chrono::{DateTime, Utc};
use crossbeam::channel;
use rayon::prelude::*;
use thiserror::Error;
use tlc_util::{
    ExplorationRun, RunConfiguration, RunStatus, SpecificationPackage, TelemetryMode,
    ValidationError,
};

mod metrics;
mod resume;

pub use metrics::{RunMetrics, RunMetricsRecorder};
pub use resume::{prepare_resume, ResumeContext, ResumeError, ResumeLineage, ResumeRequest};

/// Slice of the frontier assigned to a single worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontierSlice {
    pub start: usize,
    pub end: usize,
}

impl FrontierSlice {
    #[inline]
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// Evenly partition a frontier into contiguous slices for each worker.
pub fn partition_frontier(frontier_len: usize, workers: NonZeroUsize) -> Vec<FrontierSlice> {
    let worker_count = workers.get();
    let base = frontier_len / worker_count;
    let remainder = frontier_len % worker_count;

    let mut start = 0usize;
    let mut slices = Vec::with_capacity(worker_count);

    for idx in 0..worker_count {
        let extra = usize::from(idx < remainder);
        let end = start + base + extra;
        slices.push(FrontierSlice { start, end });
        start = end;
    }

    slices
}

/// Formats supported for dumping counterexample traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceExportFormat {
    HumanReadable,
    Action,
    Dot,
    Tla,
}

/// Configuration for dumping counterexample traces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpTraceOptions {
    pub format: TraceExportFormat,
    pub output_path: PathBuf,
}

/// Additional runtime options applied to an exploration run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOptions {
    pub checkpoint_dir: Option<PathBuf>,
    pub checkpoint_interval: Option<Duration>,
    pub coverage_interval: Option<Duration>,
    pub continue_on_violation: bool,
    pub skip_deadlock: bool,
    pub cleanup: bool,
    pub suppress_warnings: bool,
    pub diff_trace: bool,
    pub dump_trace: Option<DumpTraceOptions>,
    pub post_conditions: Vec<String>,
    pub telemetry_endpoint: Option<String>,
}

impl EngineOptions {
    fn normalize(self, telemetry_mode: TelemetryMode) -> Result<Self> {
        let EngineOptions {
            checkpoint_dir,
            checkpoint_interval,
            coverage_interval,
            continue_on_violation,
            skip_deadlock,
            cleanup,
            suppress_warnings,
            diff_trace,
            dump_trace,
            post_conditions,
            telemetry_endpoint,
        } = self;

        let checkpoint_interval = sanitize_interval(checkpoint_interval);
        let coverage_interval = sanitize_interval(coverage_interval);

        let telemetry_endpoint = match telemetry_endpoint {
            Some(endpoint) => {
                let trimmed = endpoint.trim();
                if trimmed.is_empty() {
                    return Err(EngineError::EmptyTelemetryEndpoint);
                }
                Some(trimmed.to_string())
            }
            None => None,
        };

        if matches!(telemetry_mode, TelemetryMode::Otlp) && telemetry_endpoint.is_none() {
            return Err(EngineError::MissingTelemetryEndpoint);
        }

        let post_conditions = post_conditions
            .into_iter()
            .map(normalize_post_condition)
            .collect::<Result<Vec<_>>>()?;

        Ok(EngineOptions {
            checkpoint_dir,
            checkpoint_interval,
            coverage_interval,
            continue_on_violation,
            skip_deadlock,
            cleanup,
            suppress_warnings,
            diff_trace,
            dump_trace,
            post_conditions,
            telemetry_endpoint,
        })
    }
}

fn sanitize_interval(duration: Option<Duration>) -> Option<Duration> {
    duration.filter(|value| !value.is_zero())
}

fn normalize_post_condition(raw: String) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(EngineError::InvalidPostCondition(raw));
    }

    let mut parts = trimmed.split('!');
    let module = parts.next().unwrap_or_default();
    let operator = parts.next();
    if module.is_empty() || operator.map_or(true, str::is_empty) || parts.next().is_some() {
        return Err(EngineError::InvalidPostCondition(trimmed.to_string()));
    }

    Ok(format!(
        "{}!{}",
        module,
        operator.expect("operator extracted above")
    ))
}

/// Errors surfaced while preparing exploration runs.
#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("telemetry endpoint required when telemetry mode is `otlp`")]
    MissingTelemetryEndpoint,
    #[error("telemetry endpoint cannot be empty or whitespace")]
    EmptyTelemetryEndpoint,
    #[error("post-condition '{0}' must use MODULE!Operator format")]
    InvalidPostCondition(String),
    #[error("engine invariant violated: {0}")]
    InvariantViolation(String),
}

/// Result alias for engine operations.
pub type Result<T> = std::result::Result<T, EngineError>;

/// Prepared context returned by [`prepare_run`] prior to launching the engine.
#[derive(Debug, Clone)]
pub struct RunContext {
    specification: SpecificationPackage,
    configuration: RunConfiguration,
    options: EngineOptions,
    worker_count: NonZeroUsize,
    prepared_at: DateTime<Utc>,
}

impl RunContext {
    /// Access the validated specification package for this run.
    pub fn specification(&self) -> &SpecificationPackage {
        &self.specification
    }

    /// Access the validated run configuration.
    pub fn configuration(&self) -> &RunConfiguration {
        &self.configuration
    }

    /// Access the normalized engine options.
    pub fn options(&self) -> &EngineOptions {
        &self.options
    }

    /// Retrieve the worker count as a `NonZeroUsize` for runtime scheduling.
    pub fn worker_count(&self) -> NonZeroUsize {
        self.worker_count
    }

    /// Timestamp indicating when the engine context was prepared.
    pub fn prepared_at(&self) -> DateTime<Utc> {
        self.prepared_at
    }

    /// Generate a pending [`ExplorationRun`] record for downstream consumers.
    pub fn pending_run_record(&self) -> Result<ExplorationRun> {
        ExplorationRun::new(
            self.configuration.run_id,
            self.specification.id,
            RunStatus::Pending,
            self.prepared_at,
            None,
            0,
            0.0,
            0.0,
            0,
            Vec::new(),
            None,
        )
        .map_err(EngineError::from)
    }

    /// Consume the context and return its constituent parts.
    pub fn into_parts(
        self,
    ) -> (
        SpecificationPackage,
        RunConfiguration,
        EngineOptions,
        NonZeroUsize,
        DateTime<Utc>,
    ) {
        (
            self.specification,
            self.configuration,
            self.options,
            self.worker_count,
            self.prepared_at,
        )
    }
}

/// Prepare an exploration run by validating inputs and enforcing engine invariants.
pub fn prepare_run(
    specification: SpecificationPackage,
    configuration: RunConfiguration,
    options: EngineOptions,
) -> Result<RunContext> {
    specification.validate()?;
    configuration.validate()?;

    let worker_count =
        NonZeroUsize::new(configuration.workers.get() as usize).expect("u16 > 0 converts to usize");
    enforce_partition_invariants(worker_count)?;

    let options = options.normalize(configuration.telemetry_mode)?;
    let prepared_at = Utc::now();

    tracing::debug!(
        run_id = %configuration.run_id,
        spec_id = %specification.id,
        workers = worker_count.get(),
        "Prepared TLC exploration context"
    );

    Ok(RunContext {
        specification,
        configuration,
        options,
        worker_count,
        prepared_at,
    })
}

fn enforce_partition_invariants(workers: NonZeroUsize) -> Result<()> {
    let sample_lengths = [
        0usize,
        workers.get(),
        workers.get().saturating_mul(3).saturating_add(1),
    ];

    for frontier in sample_lengths {
        let slices = partition_frontier(frontier, workers);
        if slices.len() != workers.get() {
            return Err(EngineError::InvariantViolation(format!(
                "expected {} slices for frontier {} (got {})",
                workers.get(),
                frontier,
                slices.len()
            )));
        }

        let mut cursor = 0usize;
        for slice in &slices {
            if slice.start != cursor {
                return Err(EngineError::InvariantViolation(format!(
                    "frontier slice expected to start at {} but started at {}",
                    cursor, slice.start
                )));
            }
            if slice.end < slice.start {
                return Err(EngineError::InvariantViolation(
                    "frontier slice end precedes start".to_string(),
                ));
            }
            cursor = slice.end;
        }

        if cursor != frontier {
            return Err(EngineError::InvariantViolation(format!(
                "frontier coverage mismatch: expected {}, got {}",
                frontier, cursor
            )));
        }

        let lengths: Vec<usize> = slices.iter().map(FrontierSlice::len).collect();
        if lengths.iter().sum::<usize>() != frontier {
            return Err(EngineError::InvariantViolation(
                "frontier slice lengths do not sum to original length".to_string(),
            ));
        }

        if let (Some(min), Some(max)) = (lengths.iter().min(), lengths.iter().max()) {
            if max - min > 1 {
                return Err(EngineError::InvariantViolation(format!(
                    "slice imbalance exceeded threshold: min={}, max={}",
                    min, max
                )));
            }
        }

        tracing::trace!(
            frontier,
            workers = workers.get(),
            "validated frontier partition invariants"
        );
    }

    Ok(())
}

/// Ensure the engine crate links required concurrency dependencies.
pub fn bootstrap_engine() -> AnyhowResult<()> {
    let numbers = (0..4_u8).into_par_iter().map(|n| n as usize);
    let sum: usize = numbers.sum();
    tracing::trace!(sum, "engine parallel iterator sample complete");

    let (tx, rx) = channel::unbounded::<usize>();
    tx.send(sum)?;
    let _ = rx.try_recv().unwrap_or(sum);

    tlc_util::initialize_runtime()?;
    tracing::debug!("tlc-engine bootstrap initialized");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use blake3::hash;
    use std::{
        collections::BTreeMap,
        fs::{self, File},
        io::Write,
        num::NonZeroU16,
        path::Path,
    };
    use tempfile::tempdir;
    use tlc_util::{Blake3Digest, ProgressMode};
    use ulid::Ulid;
    use uuid::Uuid;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        let mut file = File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write contents");
    }

    fn build_specification(temp_dir: &Path) -> SpecificationPackage {
        let spec_path = temp_dir.join("Main.tla");
        let cfg_path = temp_dir.join("MC.cfg");
        write_file(&spec_path, "---- MODULE Main ----\n====");
        write_file(&cfg_path, "CONSTANTS Foo = 1");

        SpecificationPackage::new(
            Uuid::new_v4(),
            vec![spec_path],
            cfg_path,
            BTreeMap::new(),
            Blake3Digest::from_hash(hash(b"demo")),
        )
        .expect("create specification package")
    }

    fn build_configuration(telemetry: TelemetryMode) -> RunConfiguration {
        RunConfiguration::new(
            Ulid::new(),
            NonZeroU16::new(4).expect("non-zero workers"),
            Some(2 * 1024 * 1024 * 1024),
            telemetry,
            ProgressMode::Tty,
            None,
        )
        .expect("create run configuration")
    }

    fn base_options(temp_dir: &Path) -> EngineOptions {
        EngineOptions {
            checkpoint_dir: Some(temp_dir.join("checkpoints")),
            checkpoint_interval: Some(Duration::ZERO),
            coverage_interval: Some(Duration::from_secs(300)),
            continue_on_violation: true,
            skip_deadlock: false,
            cleanup: false,
            suppress_warnings: false,
            diff_trace: false,
            dump_trace: Some(DumpTraceOptions {
                format: TraceExportFormat::HumanReadable,
                output_path: temp_dir.join("trace.out"),
            }),
            post_conditions: vec![" Foo!Bar ".into()],
            telemetry_endpoint: None,
        }
    }

    #[test]
    fn prepare_run_normalizes_options_and_generates_pending_record() {
        let temp_dir = tempdir().expect("tempdir");
        let specification = build_specification(temp_dir.path());
        let configuration = build_configuration(TelemetryMode::Local);
        let options = base_options(temp_dir.path());

        let context = prepare_run(specification.clone(), configuration.clone(), options).unwrap();
        assert_eq!(
            context.worker_count().get(),
            usize::from(configuration.workers.get())
        );
        assert!(
            context.options().checkpoint_interval.is_none(),
            "zero checkpoint interval disables periodic checkpoints"
        );
        assert_eq!(
            context.options().coverage_interval,
            Some(Duration::from_secs(300))
        );
        assert_eq!(context.options().post_conditions, vec!["Foo!Bar"]);

        let run = context.pending_run_record().unwrap();
        assert_eq!(run.run_id, configuration.run_id);
        assert_eq!(run.spec_package_id, specification.id);
        assert_eq!(run.status, RunStatus::Pending);
        assert_eq!(run.states_explored, 0);
    }

    #[test]
    fn telemetry_endpoint_required_for_otlp() {
        let temp_dir = tempdir().expect("tempdir");
        let specification = build_specification(temp_dir.path());
        let configuration = build_configuration(TelemetryMode::Otlp);
        let options = base_options(temp_dir.path());

        let error = prepare_run(specification, configuration, options)
            .expect_err("expected endpoint error");
        assert!(matches!(error, EngineError::MissingTelemetryEndpoint));
    }

    #[test]
    fn rejects_invalid_post_condition_format() {
        let temp_dir = tempdir().expect("tempdir");
        let specification = build_specification(temp_dir.path());
        let configuration = build_configuration(TelemetryMode::Local);
        let mut options = base_options(temp_dir.path());
        options.post_conditions = vec!["InvalidFormat".into()];

        let error = prepare_run(specification, configuration, options)
            .expect_err("expected post condition error");
        assert!(matches!(
            error,
            EngineError::InvalidPostCondition(value) if value == "InvalidFormat"
        ));
    }
}
