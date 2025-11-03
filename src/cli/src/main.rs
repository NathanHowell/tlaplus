mod commands;
mod input_loader;
mod output;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;
use ulid::Ulid;

use commands::{
    Cli, Command, MemoryLimit, ParameterOverride, ProgressMode, ResumeCommand, RunCommand,
    TelemetryMode, TraceDumpFormat,
};
use input_loader::{load_run_inputs, DumpTraceConfig, RunInputs, RunOptions};
use tlc_checkpoint::{CheckpointStore, StoreOptions};
use tlc_engine::{
    prepare_resume, prepare_run, DumpTraceOptions, EngineOptions, ResumeRequest, TraceExportFormat,
};
use tlc_util::{RunConfiguration, TelemetryMode as UtilTelemetryMode};

const RUN_MANIFEST_VERSION: u32 = 1;
const MANIFEST_EXTENSION: &str = "manifest.json";
const DEFAULT_CHECKPOINT_DIR: &str = "checkpoints";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunManifest {
    version: u32,
    inputs: RunInputs,
    lineage: Vec<Ulid>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Run(command) => execute_run(command),
        Command::Resume(command) => execute_resume(command),
    }
}

fn execute_run(command: RunCommand) -> Result<()> {
    let inputs = load_run_inputs(&command).map_err(anyhow::Error::new)?;

    let _telemetry_guard = install_telemetry(
        inputs.configuration.telemetry_mode,
        inputs.options.telemetry_endpoint.clone(),
    );

    tlc_util::initialize_runtime()?;
    tlc_engine::bootstrap_engine()?;

    let engine_options = engine_options_from(&inputs.options);
    let run_context = prepare_run(
        inputs.specification.clone(),
        inputs.configuration.clone(),
        engine_options.clone(),
    )?;

    let checkpoint_path = derive_checkpoint_path(run_context.configuration(), &inputs.options);
    ensure_checkpoint_store(&checkpoint_path)?;

    let (specification, configuration, normalized_options, worker_count, _) =
        run_context.clone().into_parts();
    let manifest_inputs = RunInputs {
        specification,
        configuration: configuration.clone(),
        options: run_options_from_engine(&normalized_options),
    };
    let manifest = RunManifest::new(manifest_inputs, vec![configuration.run_id]);
    persist_manifest(&checkpoint_path, &manifest)?;

    tracing::info!(
        run_id = %configuration.run_id,
        spec_id = %manifest.inputs.specification.id,
        checkpoint = %checkpoint_path.display(),
        workers = %worker_count.get(),
        "prepared TLC run context"
    );

    Ok(())
}

fn execute_resume(command: ResumeCommand) -> Result<()> {
    let manifest = load_manifest(&command.checkpoint)?;
    let resume_command = build_resume_command(&manifest, &command)?;
    let inputs = load_run_inputs(&resume_command).map_err(anyhow::Error::new)?;

    let mut request = ResumeRequest::new(&command.checkpoint);
    if command.ignore_hash {
        request = request.with_ignore_hash(true);
    }
    if !manifest.lineage.is_empty() {
        request = request.with_prior_lineage(manifest.lineage.clone());
    }

    let _telemetry_guard = install_telemetry(
        inputs.configuration.telemetry_mode,
        inputs.options.telemetry_endpoint.clone(),
    );

    tlc_util::initialize_runtime()?;
    tlc_engine::bootstrap_engine()?;

    let engine_options = engine_options_from(&inputs.options);
    let resume_context = prepare_resume(
        inputs.specification.clone(),
        inputs.configuration.clone(),
        engine_options.clone(),
        request,
    )?;

    let (run_context, checkpoint_store, metadata, lineage, checkpoint_path) =
        resume_context.into_parts();
    drop(checkpoint_store);

    let (specification, configuration, normalized_options, worker_count, _) =
        run_context.clone().into_parts();
    let manifest_inputs = RunInputs {
        specification,
        configuration: configuration.clone(),
        options: run_options_from_engine(&normalized_options),
    };
    let updated_manifest = RunManifest::new(manifest_inputs, lineage.chain().to_vec());
    persist_manifest(&command.checkpoint, &updated_manifest)?;

    tracing::info!(
        resume_run = %configuration.run_id,
        checkpoint = %checkpoint_path.display(),
        parent_run = %metadata.run_id,
        lineage = ?updated_manifest.lineage,
        workers = %worker_count.get(),
        "prepared TLC resume context"
    );

    Ok(())
}

fn install_telemetry(
    mode: UtilTelemetryMode,
    endpoint: Option<String>,
) -> tlc_telemetry::TelemetryGuard {
    let mut config = tlc_telemetry::TelemetryConfig::default();
    config.mode = mode;
    if let Some(endpoint) = endpoint {
        config.otlp_endpoint = Some(endpoint);
    }

    match tlc_telemetry::init_tracing(config.clone()) {
        Ok(guard) => guard,
        Err(err) => {
            install_fallback_tracing();
            tracing::warn!(error = ?err, "falling back to CLI tracing bootstrap");
            tlc_telemetry::TelemetryGuard::disabled()
        }
    }
}

fn install_fallback_tracing() {
    let filter = std::env::var("TLC_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .compact()
        .try_init();
}

fn engine_options_from(options: &RunOptions) -> EngineOptions {
    EngineOptions {
        checkpoint_dir: options.checkpoint_dir.clone(),
        checkpoint_interval: options.checkpoint_interval,
        coverage_interval: options.coverage_interval,
        continue_on_violation: options.continue_on_violation,
        skip_deadlock: options.skip_deadlock,
        cleanup: options.cleanup,
        suppress_warnings: options.suppress_warnings,
        diff_trace: options.diff_trace,
        dump_trace: options.dump_trace.as_ref().map(|cfg| DumpTraceOptions {
            format: map_trace_format(cfg.format.clone()),
            output_path: cfg.output_path.clone(),
        }),
        post_conditions: options.post_conditions.clone(),
        telemetry_endpoint: options.telemetry_endpoint.clone(),
    }
}

fn run_options_from_engine(options: &EngineOptions) -> RunOptions {
    RunOptions {
        checkpoint_dir: options.checkpoint_dir.clone(),
        checkpoint_interval: options.checkpoint_interval,
        coverage_interval: options.coverage_interval,
        continue_on_violation: options.continue_on_violation,
        skip_deadlock: options.skip_deadlock,
        cleanup: options.cleanup,
        suppress_warnings: options.suppress_warnings,
        diff_trace: options.diff_trace,
        dump_trace: options.dump_trace.as_ref().map(|cfg| DumpTraceConfig {
            format: reverse_map_trace_format(cfg.format),
            output_path: cfg.output_path.clone(),
        }),
        post_conditions: options.post_conditions.clone(),
        telemetry_endpoint: options.telemetry_endpoint.clone(),
    }
}

fn map_trace_format(format: TraceDumpFormat) -> TraceExportFormat {
    match format {
        TraceDumpFormat::HumanReadable => TraceExportFormat::HumanReadable,
        TraceDumpFormat::Action => TraceExportFormat::Action,
        TraceDumpFormat::Dot => TraceExportFormat::Dot,
        TraceDumpFormat::Tla => TraceExportFormat::Tla,
    }
}

fn reverse_map_trace_format(format: TraceExportFormat) -> TraceDumpFormat {
    match format {
        TraceExportFormat::HumanReadable => TraceDumpFormat::HumanReadable,
        TraceExportFormat::Action => TraceDumpFormat::Action,
        TraceExportFormat::Dot => TraceDumpFormat::Dot,
        TraceExportFormat::Tla => TraceDumpFormat::Tla,
    }
}

fn derive_checkpoint_path(configuration: &RunConfiguration, options: &RunOptions) -> PathBuf {
    let base = options
        .checkpoint_dir
        .clone()
        .unwrap_or_else(default_checkpoint_dir);
    base.join(format!("{}.chk", configuration.run_id))
}

fn default_checkpoint_dir() -> PathBuf {
    PathBuf::from(DEFAULT_CHECKPOINT_DIR)
}

fn ensure_checkpoint_store(path: &Path) -> Result<()> {
    CheckpointStore::open(path, StoreOptions::default())
        .map(|_| ())
        .with_context(|| {
            format!(
                "failed to initialize checkpoint store at '{}'",
                path.display()
            )
        })
}

fn manifest_path(checkpoint_path: &Path) -> PathBuf {
    let mut manifest_path = checkpoint_path.to_path_buf();
    manifest_path.set_extension(MANIFEST_EXTENSION);
    manifest_path
}

fn persist_manifest(checkpoint_path: &Path, manifest: &RunManifest) -> Result<()> {
    let manifest_path = manifest_path(checkpoint_path);
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create parent directory for manifest at '{}'",
                parent.display()
            )
        })?;
    }
    let data = serde_json::to_vec_pretty(manifest)?;
    fs::write(&manifest_path, data)
        .with_context(|| format!("failed to write manifest to '{}'", manifest_path.display()))
}

fn load_manifest(checkpoint_path: &Path) -> Result<RunManifest> {
    let path = manifest_path(checkpoint_path);
    let data = fs::read(&path)
        .with_context(|| format!("failed to read manifest from '{}'", path.display()))?;
    let manifest: RunManifest = serde_json::from_slice(&data)
        .with_context(|| format!("manifest at '{}' is invalid JSON", path.display()))?;
    Ok(manifest)
}

fn build_resume_command(manifest: &RunManifest, command: &ResumeCommand) -> Result<RunCommand> {
    let specification = &manifest.inputs.specification;
    let primary_module = specification
        .modules
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("manifest is missing module list for resume"))?;
    let parameters = parameters_from_map(&specification.parameters);

    let memory_limit = manifest
        .inputs
        .configuration
        .memory_limit_bytes
        .map(MemoryLimit);
    let checkpoint_interval = manifest
        .inputs
        .options
        .checkpoint_interval
        .map(duration_to_minutes)
        .transpose()?;
    let coverage_interval = manifest
        .inputs
        .options
        .coverage_interval
        .map(duration_to_minutes)
        .transpose()?;

    let (dump_trace, dump_trace_file) = manifest
        .inputs
        .options
        .dump_trace
        .as_ref()
        .map(|cfg| (Some(cfg.format.clone()), Some(cfg.output_path.clone())))
        .unwrap_or((None, None));
    let otlp_endpoint = command
        .output
        .otlp_endpoint
        .clone()
        .or_else(|| manifest.inputs.options.telemetry_endpoint.clone());

    Ok(RunCommand {
        spec: primary_module,
        config: specification.config_path.clone(),
        workers: command.workers,
        memory_limit,
        parameters,
        checkpoint_dir: manifest.inputs.options.checkpoint_dir.clone(),
        checkpoint_interval,
        coverage_interval,
        continue_on_violation: manifest.inputs.options.continue_on_violation,
        skip_deadlock: manifest.inputs.options.skip_deadlock,
        cleanup: manifest.inputs.options.cleanup,
        suppress_warnings: manifest.inputs.options.suppress_warnings,
        diff_trace: manifest.inputs.options.diff_trace,
        dump_trace,
        dump_trace_file,
        post_conditions: manifest.inputs.options.post_conditions.clone(),
        output: crate::commands::OutputOptions {
            progress: command.output.progress,
            telemetry: command.output.telemetry,
            otlp_endpoint,
        },
    })
}

fn parameters_from_map(parameters: &BTreeMap<String, String>) -> Vec<ParameterOverride> {
    parameters
        .iter()
        .map(|(key, value)| ParameterOverride {
            key: key.clone(),
            value: value.clone(),
        })
        .collect()
}

fn duration_to_minutes(duration: Duration) -> Result<u32> {
    let minutes = duration
        .as_secs()
        .checked_div(60)
        .ok_or_else(|| anyhow!("duration exceeds supported range for minutes conversion"))?;
    u32::try_from(minutes)
        .map_err(|_| anyhow!("duration minutes value exceeds u32::MAX ({minutes})"))
}

impl RunManifest {
    fn new(mut inputs: RunInputs, mut lineage: Vec<Ulid>) -> Self {
        if inputs.options.checkpoint_dir.is_none() {
            inputs.options.checkpoint_dir = Some(default_checkpoint_dir());
        }
        if lineage.is_empty() {
            lineage.push(inputs.configuration.run_id);
        }
        Self {
            version: RUN_MANIFEST_VERSION,
            inputs,
            lineage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use commands::{OutputOptions, WorkerCount};
    use std::io::Write;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        let mut file = fs::File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write contents");
    }

    fn build_sample_run_command(dir: &Path) -> RunCommand {
        RunCommand {
            spec: dir.join("Main.tla"),
            config: dir.join("MC.cfg"),
            workers: WorkerCount::Fixed(4),
            memory_limit: None,
            parameters: vec![
                ParameterOverride {
                    key: "Foo".into(),
                    value: "Bar".into(),
                },
                ParameterOverride {
                    key: "Baz".into(),
                    value: "Quux".into(),
                },
            ],
            checkpoint_dir: Some(dir.join("checkpoints")),
            checkpoint_interval: Some(5),
            coverage_interval: Some(1),
            continue_on_violation: true,
            skip_deadlock: false,
            cleanup: true,
            suppress_warnings: false,
            diff_trace: false,
            dump_trace: None,
            dump_trace_file: None,
            post_conditions: vec!["Mod!Op".into()],
            output: OutputOptions {
                progress: ProgressMode::Ndjson,
                telemetry: TelemetryMode::Local,
                otlp_endpoint: Some("https://collector:4317".into()),
            },
        }
    }

    #[test]
    fn manifest_round_trip_persists_inputs() -> Result<()> {
        let temp_dir = tempdir()?;
        let spec_path = temp_dir.path().join("Main.tla");
        let cfg_path = temp_dir.path().join("MC.cfg");
        write_file(&spec_path, "---- MODULE Main ----\n====");
        write_file(&cfg_path, "CONSTANTS Foo = 1");

        let command = build_sample_run_command(temp_dir.path());
        let inputs = load_run_inputs(&command).map_err(anyhow::Error::new)?;
        let manifest = RunManifest::new(inputs.clone(), vec![inputs.configuration.run_id]);

        let checkpoint_path = temp_dir.path().join("checkpoint.chk");
        persist_manifest(&checkpoint_path, &manifest)?;
        let loaded = load_manifest(&checkpoint_path)?;

        assert_eq!(loaded.version, RUN_MANIFEST_VERSION);
        assert_eq!(
            loaded.inputs.specification.modules,
            manifest.inputs.specification.modules
        );
        assert_eq!(
            loaded.inputs.options.post_conditions,
            manifest.inputs.options.post_conditions
        );
        assert_eq!(loaded.lineage, manifest.lineage);

        Ok(())
    }

    #[test]
    fn build_resume_command_applies_cli_overrides() -> Result<()> {
        let temp_dir = tempdir()?;
        let spec_path = temp_dir.path().join("Main.tla");
        let cfg_path = temp_dir.path().join("MC.cfg");
        write_file(&spec_path, "---- MODULE Main ----\n====");
        write_file(&cfg_path, "CONSTANTS Foo = 1");

        let mut run_command = build_sample_run_command(temp_dir.path());
        run_command.dump_trace = Some(TraceDumpFormat::Dot);
        run_command.dump_trace_file = Some(temp_dir.path().join("trace.dot"));

        let inputs = load_run_inputs(&run_command).map_err(anyhow::Error::new)?;
        let manifest = RunManifest::new(inputs.clone(), vec![inputs.configuration.run_id]);

        let resume_command = ResumeCommand {
            checkpoint: temp_dir.path().join("checkpoint.chk"),
            workers: WorkerCount::Fixed(8),
            ignore_hash: false,
            output: OutputOptions {
                progress: ProgressMode::Tty,
                telemetry: TelemetryMode::Json,
                otlp_endpoint: Some("https://cli-endpoint:4317".into()),
            },
        };

        let resume_run_command = build_resume_command(&manifest, &resume_command)?;
        assert_eq!(resume_run_command.workers, WorkerCount::Fixed(8));
        assert_eq!(resume_run_command.output.progress, ProgressMode::Tty);
        assert_eq!(resume_run_command.output.telemetry, TelemetryMode::Json);
        assert_eq!(
            resume_run_command.output.otlp_endpoint.as_deref(),
            Some("https://cli-endpoint:4317")
        );
        assert_eq!(resume_run_command.dump_trace, Some(TraceDumpFormat::Dot));
        assert_eq!(
            resume_run_command.dump_trace_file.as_ref(),
            run_command.dump_trace_file.as_ref()
        );

        Ok(())
    }
}
