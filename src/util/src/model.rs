use blake3::Hash as Blake3Hash;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::num::NonZeroU16;
use std::path::{Path, PathBuf};
use thiserror::Error;
use ulid::Ulid;
use uuid::Uuid;

const MIN_MEMORY_LIMIT_BYTES: u64 = 1_073_741_824; // 1 GiB

/// Errors that can occur while validating TLC model data structures.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("module list cannot be empty")]
    EmptyModuleList,
    #[error("{kind} path '{path}' is unreadable: {source}")]
    PathUnreadable {
        kind: PathKind,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{kind} path '{path}' must be a file")]
    PathNotFile { kind: PathKind, path: PathBuf },
    #[error("parameter keys must not be empty or whitespace only")]
    EmptyParameterKey,
    #[error("memory limit must be at least {minimum} bytes (got {provided})")]
    MemoryLimitTooSmall { minimum: u64, provided: u64 },
    #[error("workers must be at least 1")]
    InvalidWorkerCount,
    #[error("completed_at timestamp requires terminal status (got {status})")]
    CompletedAtWithoutTerminalStatus { status: RunStatus },
    #[error("coverage_percent must be between 0.0 and 100.0 (got {0})")]
    InvalidCoverage(f32),
    #[error("throughput_eps cannot be negative (got {0})")]
    NegativeThroughput(f64),
    #[error("checkpoint list contains duplicate id {0}")]
    DuplicateCheckpointId(Ulid),
    #[error("completion timestamp {completed_at} precedes start timestamp {started_at}")]
    InvalidCompletionTiming {
        started_at: DateTime<Utc>,
        completed_at: DateTime<Utc>,
    },
    #[error("invalid blake3 hash: {0}")]
    InvalidHash(String),
}

/// Describes the type of file path under validation to enrich error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathKind {
    Module,
    Config,
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            PathKind::Module => "module",
            PathKind::Config => "configuration",
        };
        f.write_str(label)
    }
}

/// Newtype for BLAKE3-256 digests that serializes to/from hex strings.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Blake3Digest {
    inner: Blake3Hash,
}

impl Blake3Digest {
    pub fn from_hash(hash: Blake3Hash) -> Self {
        Self { inner: hash }
    }

    pub fn from_hex<S: AsRef<str>>(hex: S) -> Result<Self, ValidationError> {
        let hex_str = hex.as_ref();
        let hash = Blake3Hash::from_hex(hex_str)
            .map_err(|_| ValidationError::InvalidHash(hex_str.to_owned()))?;
        Ok(Self { inner: hash })
    }

    pub fn to_hex(&self) -> String {
        self.inner.to_hex().to_string()
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        self.inner.as_bytes()
    }

    pub fn into_inner(self) -> Blake3Hash {
        self.inner
    }
}

impl From<Blake3Hash> for Blake3Digest {
    fn from(hash: Blake3Hash) -> Self {
        Self::from_hash(hash)
    }
}

impl From<Blake3Digest> for Blake3Hash {
    fn from(value: Blake3Digest) -> Self {
        value.inner
    }
}

impl Serialize for Blake3Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Blake3Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hex = String::deserialize(deserializer)?;
        Blake3Digest::from_hex(&hex).map_err(serde::de::Error::custom)
    }
}

/// Immutable description of a specification package submitted to TLC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecificationPackage {
    pub id: Uuid,
    pub modules: Vec<PathBuf>,
    pub config_path: PathBuf,
    pub parameters: BTreeMap<String, String>,
    pub hash: Blake3Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SpecificationPackageSerde {
    pub id: Uuid,
    pub modules: Vec<PathBuf>,
    pub config_path: PathBuf,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    pub hash: Blake3Digest,
}

impl TryFrom<SpecificationPackageSerde> for SpecificationPackage {
    type Error = ValidationError;

    fn try_from(value: SpecificationPackageSerde) -> Result<Self, Self::Error> {
        let package = SpecificationPackage {
            id: value.id,
            modules: value.modules,
            config_path: value.config_path,
            parameters: value.parameters,
            hash: value.hash,
        };
        package.validate()?;
        Ok(package)
    }
}

impl From<SpecificationPackage> for SpecificationPackageSerde {
    fn from(value: SpecificationPackage) -> Self {
        SpecificationPackageSerde {
            id: value.id,
            modules: value.modules,
            config_path: value.config_path,
            parameters: value.parameters,
            hash: value.hash,
        }
    }
}

impl Serialize for SpecificationPackage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SpecificationPackageSerde::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SpecificationPackage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = SpecificationPackageSerde::deserialize(deserializer)?;
        SpecificationPackage::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl SpecificationPackage {
    pub fn new(
        id: Uuid,
        modules: Vec<PathBuf>,
        config_path: PathBuf,
        parameters: BTreeMap<String, String>,
        hash: Blake3Digest,
    ) -> Result<Self, ValidationError> {
        SpecificationPackage {
            id,
            modules,
            config_path,
            parameters,
            hash,
        }
        .validated()
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.modules.is_empty() {
            return Err(ValidationError::EmptyModuleList);
        }

        for module in &self.modules {
            ensure_path_is_file(module, PathKind::Module)?;
        }

        ensure_path_is_file(&self.config_path, PathKind::Config)?;

        if self.parameters.keys().any(|key| key.trim().is_empty()) {
            return Err(ValidationError::EmptyParameterKey);
        }

        Ok(())
    }

    fn validated(self) -> Result<Self, ValidationError> {
        self.validate()?;
        Ok(self)
    }
}

fn ensure_path_is_file(path: &Path, kind: PathKind) -> Result<(), ValidationError> {
    match fs::metadata(path) {
        Ok(meta) => {
            if !meta.is_file() {
                Err(ValidationError::PathNotFile {
                    kind,
                    path: path.to_path_buf(),
                })
            } else {
                Ok(())
            }
        }
        Err(source) => Err(ValidationError::PathUnreadable {
            kind,
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Telemetry emission modes supported by the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TelemetryMode {
    Local,
    Json,
    Otlp,
}

/// Progress output variants favored by different runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressMode {
    Tty,
    Ndjson,
}

/// Execution status of an exploration run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Pending,
    Running,
    Checkpointing,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for RunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            RunStatus::Pending => "Pending",
            RunStatus::Running => "Running",
            RunStatus::Checkpointing => "Checkpointing",
            RunStatus::Completed => "Completed",
            RunStatus::Failed => "Failed",
            RunStatus::Cancelled => "Cancelled",
        };
        f.write_str(value)
    }
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunStatus::Completed | RunStatus::Failed | RunStatus::Cancelled
        )
    }
}

/// Configuration supplied before launching an exploration run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunConfiguration {
    pub run_id: Ulid,
    pub workers: NonZeroU16,
    pub memory_limit_bytes: Option<u64>,
    pub telemetry_mode: TelemetryMode,
    pub progress_mode: ProgressMode,
    pub resume_from: Option<Ulid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunConfigurationSerde {
    pub run_id: Ulid,
    pub workers: u16,
    #[serde(default)]
    pub memory_limit_bytes: Option<u64>,
    pub telemetry_mode: TelemetryMode,
    pub progress_mode: ProgressMode,
    #[serde(default)]
    pub resume_from: Option<Ulid>,
}

impl TryFrom<RunConfigurationSerde> for RunConfiguration {
    type Error = ValidationError;

    fn try_from(value: RunConfigurationSerde) -> Result<Self, Self::Error> {
        let workers = NonZeroU16::new(value.workers).ok_or(ValidationError::InvalidWorkerCount)?;
        let config = RunConfiguration {
            run_id: value.run_id,
            workers,
            memory_limit_bytes: value.memory_limit_bytes,
            telemetry_mode: value.telemetry_mode,
            progress_mode: value.progress_mode,
            resume_from: value.resume_from,
        };
        config.validate()?;
        Ok(config)
    }
}

impl From<RunConfiguration> for RunConfigurationSerde {
    fn from(value: RunConfiguration) -> Self {
        RunConfigurationSerde {
            run_id: value.run_id,
            workers: value.workers.get(),
            memory_limit_bytes: value.memory_limit_bytes,
            telemetry_mode: value.telemetry_mode,
            progress_mode: value.progress_mode,
            resume_from: value.resume_from,
        }
    }
}

impl Serialize for RunConfiguration {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        RunConfigurationSerde::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RunConfiguration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RunConfigurationSerde::deserialize(deserializer)?;
        RunConfiguration::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl RunConfiguration {
    pub fn new(
        run_id: Ulid,
        workers: NonZeroU16,
        memory_limit_bytes: Option<u64>,
        telemetry_mode: TelemetryMode,
        progress_mode: ProgressMode,
        resume_from: Option<Ulid>,
    ) -> Result<Self, ValidationError> {
        RunConfiguration {
            run_id,
            workers,
            memory_limit_bytes,
            telemetry_mode,
            progress_mode,
            resume_from,
        }
        .validated()
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Some(limit) = self.memory_limit_bytes {
            if limit < MIN_MEMORY_LIMIT_BYTES {
                return Err(ValidationError::MemoryLimitTooSmall {
                    minimum: MIN_MEMORY_LIMIT_BYTES,
                    provided: limit,
                });
            }
        }

        Ok(())
    }

    fn validated(self) -> Result<Self, ValidationError> {
        self.validate()?;
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplorationRun {
    pub run_id: Ulid,
    pub spec_package_id: Uuid,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub states_explored: u128,
    pub coverage_percent: f32,
    pub throughput_eps: f64,
    pub fingerprint_space: u128,
    pub checkpoint_ids: Vec<Ulid>,
    pub regression_trace_path: Option<PathBuf>,
}

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExplorationRunSerde {
    pub run_id: Ulid,
    pub spec_package_id: Uuid,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde_as(as = "DisplayFromStr")]
    pub states_explored: u128,
    pub coverage_percent: f32,
    pub throughput_eps: f64,
    #[serde_as(as = "DisplayFromStr")]
    pub fingerprint_space: u128,
    #[serde(default)]
    pub checkpoint_ids: Vec<Ulid>,
    #[serde(default)]
    pub regression_trace_path: Option<PathBuf>,
}

impl TryFrom<ExplorationRunSerde> for ExplorationRun {
    type Error = ValidationError;

    fn try_from(value: ExplorationRunSerde) -> Result<Self, Self::Error> {
        let run = ExplorationRun {
            run_id: value.run_id,
            spec_package_id: value.spec_package_id,
            status: value.status,
            started_at: value.started_at,
            completed_at: value.completed_at,
            states_explored: value.states_explored,
            coverage_percent: value.coverage_percent,
            throughput_eps: value.throughput_eps,
            fingerprint_space: value.fingerprint_space,
            checkpoint_ids: value.checkpoint_ids,
            regression_trace_path: value.regression_trace_path,
        };
        run.validate()?;
        Ok(run)
    }
}

impl From<ExplorationRun> for ExplorationRunSerde {
    fn from(value: ExplorationRun) -> Self {
        ExplorationRunSerde {
            run_id: value.run_id,
            spec_package_id: value.spec_package_id,
            status: value.status,
            started_at: value.started_at,
            completed_at: value.completed_at,
            states_explored: value.states_explored,
            coverage_percent: value.coverage_percent,
            throughput_eps: value.throughput_eps,
            fingerprint_space: value.fingerprint_space,
            checkpoint_ids: value.checkpoint_ids,
            regression_trace_path: value.regression_trace_path,
        }
    }
}

impl Serialize for ExplorationRun {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ExplorationRunSerde::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExplorationRun {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = ExplorationRunSerde::deserialize(deserializer)?;
        ExplorationRun::try_from(raw).map_err(serde::de::Error::custom)
    }
}

impl ExplorationRun {
    pub fn new(
        run_id: Ulid,
        spec_package_id: Uuid,
        status: RunStatus,
        started_at: DateTime<Utc>,
        completed_at: Option<DateTime<Utc>>,
        states_explored: u128,
        coverage_percent: f32,
        throughput_eps: f64,
        fingerprint_space: u128,
        checkpoint_ids: Vec<Ulid>,
        regression_trace_path: Option<PathBuf>,
    ) -> Result<Self, ValidationError> {
        ExplorationRun {
            run_id,
            spec_package_id,
            status,
            started_at,
            completed_at,
            states_explored,
            coverage_percent,
            throughput_eps,
            fingerprint_space,
            checkpoint_ids,
            regression_trace_path,
        }
        .validated()
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Some(completed_at) = self.completed_at {
            if !self.status.is_terminal() {
                return Err(ValidationError::CompletedAtWithoutTerminalStatus {
                    status: self.status,
                });
            }

            if completed_at < self.started_at {
                return Err(ValidationError::InvalidCompletionTiming {
                    started_at: self.started_at,
                    completed_at,
                });
            }
        }

        if !(0.0..=100.0).contains(&self.coverage_percent) || !self.coverage_percent.is_finite() {
            return Err(ValidationError::InvalidCoverage(self.coverage_percent));
        }

        if !self.throughput_eps.is_finite() || self.throughput_eps < 0.0 {
            return Err(ValidationError::NegativeThroughput(self.throughput_eps));
        }

        let mut seen = HashSet::with_capacity(self.checkpoint_ids.len());
        for id in &self.checkpoint_ids {
            if !seen.insert(*id) {
                return Err(ValidationError::DuplicateCheckpointId(*id));
            }
        }

        Ok(())
    }

    fn validated(self) -> Result<Self, ValidationError> {
        self.validate()?;
        Ok(self)
    }
}
