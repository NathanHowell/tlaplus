//! Translate parsed CLI commands into validated TLC data models.

use std::{
    collections::BTreeMap,
    fs,
    num::{NonZeroU16, NonZeroUsize},
    path::{Path, PathBuf},
    time::Duration,
};

use blake3::Hasher;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tlc_util::{
    model::PathKind, Blake3Digest, ProgressMode as UtilProgressMode, RunConfiguration,
    SpecificationPackage, TelemetryMode as UtilTelemetryMode, ValidationError,
};
use ulid::Ulid;
use uuid::Uuid;

use crate::commands::{
    MemoryLimit, ParameterOverride, ProgressMode, RunCommand, TelemetryMode, TraceDumpFormat,
    WorkerCount,
};

/// End-to-end result of translating a `tlc run` invocation into runtime inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunInputs {
    pub specification: SpecificationPackage,
    pub configuration: RunConfiguration,
    pub options: RunOptions,
}

/// Additional runtime options that accompany [`RunConfiguration`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOptions {
    pub checkpoint_dir: Option<PathBuf>,
    pub checkpoint_interval: Option<Duration>,
    pub coverage_interval: Option<Duration>,
    pub continue_on_violation: bool,
    pub skip_deadlock: bool,
    pub cleanup: bool,
    pub suppress_warnings: bool,
    pub diff_trace: bool,
    pub tty_use_color: bool,
    pub dump_trace: Option<DumpTraceConfig>,
    pub post_conditions: Vec<String>,
    pub telemetry_endpoint: Option<String>,
}

/// Configuration for error trace dumping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DumpTraceConfig {
    pub format: TraceDumpFormat,
    pub output_path: PathBuf,
}

/// Errors surfaced while preparing run inputs.
#[derive(Debug, Error)]
pub enum InputError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("`--dump-trace` requires a companion `--dump-trace-file` path")]
    MissingDumpTraceFile,
    #[error("auto worker count {detected} exceeds supported range ({maximum})")]
    AutoWorkerOverflow { detected: usize, maximum: u16 },
    #[error("failed to determine available parallelism: {0}")]
    Parallelism(#[source] std::io::Error),
    #[error("failed to read module '{path}': {source}")]
    ModuleRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read configuration '{path}': {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("file '{path}' is too large to hash ({length} bytes)")]
    LengthOverflow { path: PathBuf, length: usize },
}

pub type Result<T> = std::result::Result<T, InputError>;

/// Convert a parsed [`RunCommand`] into validated TLC runtime inputs.
pub fn load_run_inputs(command: &RunCommand) -> Result<RunInputs> {
    load_run_inputs_with_progress(command, command.output.progress)
}

pub fn load_run_inputs_with_progress(
    command: &RunCommand,
    progress_mode: ProgressMode,
) -> Result<RunInputs> {
    let spec_path = canonicalize_module(&command.spec)?;
    let spec_dir = spec_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let config_path = resolve_config_path(&command.config, &spec_dir)?;

    let parameters = collect_parameters(&command.parameters);
    let modules = vec![spec_path.clone()];
    let package_hash = compute_spec_hash(&modules, &config_path, &parameters)?;
    let specification = SpecificationPackage::new(
        Uuid::new_v4(),
        modules,
        config_path.clone(),
        parameters,
        package_hash,
    )?;

    let workers = resolve_workers(command.workers)?;
    let memory_limit = command.memory_limit.map(|MemoryLimit(value)| value);
    let telemetry_mode = map_telemetry(command.output.telemetry);
    let progress_mode = map_progress(progress_mode);
    let configuration = RunConfiguration::new(
        Ulid::new(),
        workers,
        memory_limit,
        telemetry_mode,
        progress_mode,
        None,
    )?;

    let dump_trace = resolve_dump_trace(command)?;
    let options = RunOptions {
        checkpoint_dir: command.checkpoint_dir.clone(),
        checkpoint_interval: minutes_to_duration(command.checkpoint_interval),
        coverage_interval: minutes_to_duration(command.coverage_interval),
        continue_on_violation: command.continue_on_violation,
        skip_deadlock: command.skip_deadlock,
        cleanup: command.cleanup,
        suppress_warnings: command.suppress_warnings,
        diff_trace: command.diff_trace,
        tty_use_color: command.output.tty_color_enabled(),
        dump_trace,
        post_conditions: command.post_conditions.clone(),
        telemetry_endpoint: command.output.otlp_endpoint.clone(),
    };

    Ok(RunInputs {
        specification,
        configuration,
        options,
    })
}

fn canonicalize_module(path: &Path) -> Result<PathBuf> {
    path.canonicalize().map_err(|source| {
        InputError::Validation(ValidationError::PathUnreadable {
            kind: PathKind::Module,
            path: path.to_path_buf(),
            source,
        })
    })
}

fn resolve_config_path(config: &Path, spec_dir: &Path) -> Result<PathBuf> {
    if config.is_absolute() {
        return canonicalize_config(config);
    }

    let candidate = spec_dir.join(config);
    match candidate.canonicalize() {
        Ok(path) => Ok(path),
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(InputError::Validation(ValidationError::PathUnreadable {
                    kind: PathKind::Config,
                    path: candidate,
                    source: err,
                }));
            }
            canonicalize_config(config)
        }
    }
}

fn canonicalize_config(path: &Path) -> Result<PathBuf> {
    path.canonicalize().map_err(|source| {
        InputError::Validation(ValidationError::PathUnreadable {
            kind: PathKind::Config,
            path: path.to_path_buf(),
            source,
        })
    })
}

fn collect_parameters(overrides: &[ParameterOverride]) -> BTreeMap<String, String> {
    let mut parameters = BTreeMap::new();
    for override_pair in overrides {
        parameters.insert(override_pair.key.clone(), override_pair.value.clone());
    }
    parameters
}

fn compute_spec_hash(
    modules: &[PathBuf],
    config_path: &Path,
    parameters: &BTreeMap<String, String>,
) -> Result<Blake3Digest> {
    let mut hasher = Hasher::new();

    let module_count = u64::try_from(modules.len()).map_err(|_| InputError::LengthOverflow {
        path: PathBuf::from("<modules>"),
        length: modules.len(),
    })?;
    hasher.update(&module_count.to_le_bytes());

    for module in modules {
        let content = fs::read(module).map_err(|source| InputError::ModuleRead {
            path: module.clone(),
            source,
        })?;
        update_with_length(&mut hasher, &content).map_err(|length| InputError::LengthOverflow {
            path: module.clone(),
            length,
        })?;
    }

    let config_bytes = fs::read(config_path).map_err(|source| InputError::ConfigRead {
        path: config_path.to_path_buf(),
        source,
    })?;
    update_with_length(&mut hasher, &config_bytes).map_err(|length| {
        InputError::LengthOverflow {
            path: config_path.to_path_buf(),
            length,
        }
    })?;

    for (key, value) in parameters {
        update_with_length(&mut hasher, key.as_bytes()).map_err(|length| {
            InputError::LengthOverflow {
                path: PathBuf::from(format!("<parameter:{key}>")),
                length,
            }
        })?;
        update_with_length(&mut hasher, value.as_bytes()).map_err(|length| {
            InputError::LengthOverflow {
                path: PathBuf::from(format!("<parameter:{key}>")),
                length,
            }
        })?;
    }

    Ok(Blake3Digest::from_hash(hasher.finalize()))
}

fn update_with_length(hasher: &mut Hasher, data: &[u8]) -> std::result::Result<(), usize> {
    let length = data.len();
    let as_u64 = u64::try_from(length).map_err(|_| length)?;
    hasher.update(&as_u64.to_le_bytes());
    hasher.update(data);
    Ok(())
}

fn resolve_workers(count: WorkerCount) -> Result<NonZeroU16> {
    match count {
        WorkerCount::Fixed(value) => NonZeroU16::new(value)
            .ok_or_else(|| InputError::Validation(ValidationError::InvalidWorkerCount)),
        WorkerCount::Auto => {
            let parallelism: NonZeroUsize =
                std::thread::available_parallelism().map_err(InputError::Parallelism)?;
            let detected = parallelism.get();
            let auto = detected.saturating_sub(1).max(1);
            let auto_u16 = u16::try_from(auto).map_err(|_| InputError::AutoWorkerOverflow {
                detected,
                maximum: u16::MAX,
            })?;
            NonZeroU16::new(auto_u16)
                .ok_or_else(|| InputError::Validation(ValidationError::InvalidWorkerCount))
        }
    }
}

fn map_progress(mode: ProgressMode) -> UtilProgressMode {
    match mode {
        ProgressMode::Tty => UtilProgressMode::Tty,
        ProgressMode::Ndjson => UtilProgressMode::Ndjson,
    }
}

fn map_telemetry(mode: TelemetryMode) -> UtilTelemetryMode {
    match mode {
        TelemetryMode::Local => UtilTelemetryMode::Local,
        TelemetryMode::Json => UtilTelemetryMode::Json,
        TelemetryMode::Otlp => UtilTelemetryMode::Otlp,
    }
}

fn minutes_to_duration(minutes: Option<u32>) -> Option<Duration> {
    minutes.map(|value| Duration::from_secs(u64::from(value) * 60))
}

fn resolve_dump_trace(command: &RunCommand) -> Result<Option<DumpTraceConfig>> {
    match (&command.dump_trace, &command.dump_trace_file) {
        (Some(format), Some(path)) => Ok(Some(DumpTraceConfig {
            format: format.clone(),
            output_path: path.clone(),
        })),
        (Some(_), None) => Err(InputError::MissingDumpTraceFile),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{OutputOptions, ParameterOverride, RunCommand, WorkerCount};
    use std::fs::File;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn build_command(temp_dir: &Path) -> RunCommand {
        let spec_path = temp_dir.join("Main.tla");
        let config_name = PathBuf::from("MC.cfg");
        RunCommand {
            spec: spec_path,
            config: config_name,
            workers: WorkerCount::Fixed(4),
            memory_limit: Some(MemoryLimit(2 * 1024 * 1024 * 1024)),
            parameters: vec![
                ParameterOverride {
                    key: "Foo".into(),
                    value: "One".into(),
                },
                ParameterOverride {
                    key: "Foo".into(),
                    value: "Two".into(),
                },
                ParameterOverride {
                    key: "Bar".into(),
                    value: "Baz".into(),
                },
            ],
            checkpoint_dir: Some(temp_dir.join("checkpoints")),
            checkpoint_interval: Some(30),
            coverage_interval: Some(5),
            continue_on_violation: true,
            skip_deadlock: false,
            cleanup: true,
            suppress_warnings: false,
            diff_trace: true,
            dump_trace: None,
            dump_trace_file: None,
            post_conditions: vec!["Mod!Op".into()],
            output: OutputOptions {
                progress: ProgressMode::Ndjson,
                telemetry: TelemetryMode::Local,
                otlp_endpoint: None,
                no_color: false,
            },
        }
    }

    fn write_file(path: &Path, contents: &str) {
        let mut file = File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write contents");
    }

    #[test]
    fn load_run_inputs_successfully() {
        let temp_dir = tempdir().expect("tempdir");
        let spec_path = temp_dir.path().join("Main.tla");
        let config_path = temp_dir.path().join("MC.cfg");
        write_file(&spec_path, "---- MODULE Main ----\\n====");
        write_file(&config_path, "CONSTANTS Foo = 1");

        let command = build_command(temp_dir.path());
        // `build_command` stored relative config; we already wrote file in same dir.

        let inputs = load_run_inputs(&command).expect("load inputs");
        assert_eq!(inputs.configuration.workers.get(), 4);
        assert_eq!(
            inputs.configuration.memory_limit_bytes,
            Some(2 * 1024 * 1024 * 1024)
        );
        assert_eq!(inputs.configuration.resume_from, None);
        assert_eq!(inputs.configuration.progress_mode, UtilProgressMode::Ndjson);

        assert_eq!(inputs.options.continue_on_violation, true);
        assert_eq!(inputs.options.diff_trace, true);
        assert!(inputs.options.tty_use_color);
        assert_eq!(inputs.options.post_conditions, vec!["Mod!Op"]);
        assert!(inputs.options.dump_trace.is_none());
        assert_eq!(
            inputs.options.checkpoint_interval,
            Some(Duration::from_secs(1800))
        );
        assert_eq!(
            inputs.options.coverage_interval,
            Some(Duration::from_secs(300))
        );

        let modules = &inputs.specification.modules;
        assert_eq!(modules.len(), 1);
        assert!(modules[0].is_absolute());
        assert_eq!(inputs.specification.parameters["Foo"], "Two");
        assert_eq!(inputs.specification.parameters["Bar"], "Baz");
        assert!(inputs.specification.hash.to_hex().len() == 64);
    }

    #[test]
    fn dump_trace_without_file_is_rejected() {
        let temp_dir = tempdir().expect("tempdir");
        let spec_path = temp_dir.path().join("Main.tla");
        let config_path = temp_dir.path().join("MC.cfg");
        write_file(&spec_path, "---- MODULE Main ----\\n====");
        write_file(&config_path, "CONSTANTS Foo = 1");

        let mut command = build_command(temp_dir.path());
        command.dump_trace = Some(TraceDumpFormat::HumanReadable);

        let error = load_run_inputs(&command).expect_err("expected failure");
        assert!(matches!(error, InputError::MissingDumpTraceFile));
    }
}
