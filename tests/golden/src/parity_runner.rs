use std::{
    ffi::OsString,
    fmt::Write as _,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, ensure, Context, Result};
use clap::Parser;
use globset::{GlobBuilder, GlobMatcher};
use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, TestRunner},
};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tlc_engine::{partition_frontier, FrontierSlice};

/// CLI arguments for the parity harness stub.
#[derive(Debug, Parser)]
#[command(name = "tlc-parity", about = "Rust TLC parity harness (stub)")]
struct HarnessArgs {
    /// Path to the legacy TLC launcher script (Java distribution).
    #[arg(long = "legacy", value_name = "PATH")]
    legacy_launcher: PathBuf,
    /// Directory containing regression suite specifications.
    #[arg(long = "specs", value_name = "DIR")]
    specs_root: PathBuf,
    /// Glob filter limiting which specs run (e.g., `Paxos*`).
    #[arg(long = "filter")]
    filter: Option<String>,
    /// Output directory for parity artifacts.
    #[arg(
        long = "output",
        value_name = "DIR",
        default_value = "artifacts/parity"
    )]
    output_dir: PathBuf,
    /// Optional worker count for the Rust TLC binary.
    #[arg(long = "workers", value_name = "N")]
    workers: Option<usize>,
    /// Directory to cache legacy outputs (not used by stub).
    #[arg(long = "golden-cache", value_name = "DIR")]
    golden_cache: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct HarnessConfig {
    legacy_launcher: PathBuf,
    specs_root: PathBuf,
    filter: Option<String>,
    output_dir: PathBuf,
    rust_binary: PathBuf,
    workers: Option<usize>,
    golden_cache: Option<PathBuf>,
}

impl TryFrom<HarnessArgs> for HarnessConfig {
    type Error = anyhow::Error;

    fn try_from(args: HarnessArgs) -> Result<Self> {
        let legacy_launcher = absolutize(args.legacy_launcher)?;
        let specs_root = absolutize(args.specs_root)?;
        ensure!(
            specs_root.is_dir(),
            "specs directory '{}' does not exist or is not a directory",
            specs_root.display()
        );

        let output_dir = absolutize(args.output_dir)?;
        let golden_cache = match args.golden_cache {
            Some(path) => Some(absolutize(path)?),
            None => None,
        };

        if !legacy_launcher.exists() {
            tracing::warn!(
                legacy = %legacy_launcher.display(),
                "Legacy TLC launcher not found; stub will proceed without executing Java TLC."
            );
        }

        let rust_binary = resolve_rust_binary()?;

        Ok(Self {
            legacy_launcher,
            specs_root,
            filter: args.filter,
            output_dir,
            rust_binary,
            workers: args.workers,
            golden_cache,
        })
    }
}

fn absolutize(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        let cwd =
            std::env::current_dir().context("failed to determine current working directory")?;
        Ok(cwd.join(path))
    }
}

fn resolve_rust_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("TLC_PARITY_RUST_BIN") {
        let resolved = absolutize(PathBuf::from(path))?;
        ensure!(
            resolved.is_file(),
            "Rust TLC binary '{}' does not exist",
            resolved.display()
        );
        return Ok(resolved);
    }

    if let Ok(path) = std::env::var("CARGO_BIN_EXE_tlc") {
        let resolved = absolutize(PathBuf::from(path))?;
        ensure!(
            resolved.is_file(),
            "Rust TLC binary '{}' does not exist",
            resolved.display()
        );
        return Ok(resolved);
    }

    Err(anyhow!(
        "unable to locate Rust TLC binary; run parity harness via `cargo test` or set TLC_PARITY_RUST_BIN"
    ))
}

/// Public entrypoint used by the binary.
pub fn run() -> Result<()> {
    let args = HarnessArgs::parse();
    let config = HarnessConfig::try_from(args)?;
    run_with_config(config)
}

fn run_with_config(config: HarnessConfig) -> Result<()> {
    let context = HarnessContext::new(config)?;
    context.run()
}

const DEFAULT_PROPTEST_CASES: u32 = 128;

fn resolve_invariant_case_count() -> u32 {
    match std::env::var("TLC_PARITY_PROPTEST_CASES") {
        Ok(value) => value
            .parse::<u32>()
            .ok()
            .filter(|cases| *cases > 0)
            .unwrap_or(DEFAULT_PROPTEST_CASES),
        Err(_) => DEFAULT_PROPTEST_CASES,
    }
}

fn run_engine_invariant_suite<F>(partitioner: F, case_count: u32) -> Result<()>
where
    F: Fn(usize, NonZeroUsize) -> Vec<FrontierSlice>,
{
    let mut config = ProptestConfig::default();
    config.cases = case_count;
    config.failure_persistence = None;

    let mut runner = TestRunner::new(config);
    let strategy = (0usize..=100_000usize, 1usize..=64usize).prop_map(|(frontier_len, workers)| {
        let workers = NonZeroUsize::new(workers).expect("worker range never yields zero");
        (frontier_len, workers)
    });

    runner
        .run(&strategy, |(frontier_len, workers)| {
            let slices = partitioner(frontier_len, workers);
            prop_assert_eq!(
                slices.len(),
                workers.get(),
                "frontier partition should produce one slice per worker"
            );

            let mut cursor = 0usize;
            for slice in &slices {
                prop_assert_eq!(
                    slice.start,
                    cursor,
                    "slice expected to begin where previous slice ended (start={}, cursor={})",
                    slice.start,
                    cursor
                );
                prop_assert!(
                    slice.end >= slice.start,
                    "slice end must not precede slice start"
                );
                cursor = slice.end;
            }
            prop_assert_eq!(
                cursor,
                frontier_len,
                "frontier slices must cover the entire frontier"
            );

            let lengths: Vec<usize> = slices.iter().map(FrontierSlice::len).collect();
            prop_assert_eq!(
                lengths.iter().sum::<usize>(),
                frontier_len,
                "slice lengths must total the frontier length"
            );
            if let (Some(min), Some(max)) = (lengths.iter().min(), lengths.iter().max()) {
                prop_assert!(
                    max - min <= 1,
                    "slice length imbalance should be at most one element"
                );
            }

            Ok(())
        })
        .map_err(|error| anyhow!("engine invariant check failed: {error}"))
}

fn verify_engine_invariants() -> Result<()> {
    let cases = resolve_invariant_case_count();
    tracing::debug!(cases, "Running engine invariant property suite.");
    run_engine_invariant_suite(partition_frontier, cases)
}

#[derive(Debug)]
struct HarnessContext {
    config: HarnessConfig,
    workspace: TempDir,
    rust_binary: PathBuf,
}

impl HarnessContext {
    fn new(config: HarnessConfig) -> Result<Self> {
        let workspace = tempfile::tempdir()
            .context("failed to create temporary workspace for parity harness")?;
        Ok(Self {
            rust_binary: config.rust_binary.clone(),
            config,
            workspace,
        })
    }

    fn run(&self) -> Result<()> {
        verify_engine_invariants().context("engine invariants must pass before parity runs")?;
        tracing::debug!("Engine invariants passed; continuing with parity execution.");

        tracing::debug!(
            workspace = %self.workspace.path().display(),
            specs = %self.config.specs_root.display(),
            output = %self.config.output_dir.display(),
            legacy_launcher = %self.config.legacy_launcher.display(),
            rust_binary = %self.rust_binary.display(),
            "Parity harness starting execution runs."
        );

        if let Some(workers) = self.config.workers {
            tracing::debug!(workers, "Worker override captured for parity runs.");
        }
        if let Some(cache) = self.config.golden_cache.as_ref() {
            tracing::debug!(
                cache = %cache.display(),
                "Golden cache directory recorded for future reuse."
            );
        }

        let specs = discover_specs(&self.config)?;
        if specs.is_empty() {
            tracing::warn!(
                specs_root = %self.config.specs_root.display(),
                "No specifications discovered; summary will be empty."
            );
        }

        fs::create_dir_all(&self.config.output_dir).with_context(|| {
            format!(
                "failed to create parity output directory '{}'",
                self.config.output_dir.display()
            )
        })?;

        let mut reports = Vec::with_capacity(specs.len());
        for spec in specs {
            let report = self.execute_spec(&spec)?;
            reports.push(report);
        }

        let summary = SummaryReport { specs: reports };
        self.write_summary(&summary)?;

        let total = summary.specs.len();
        let matches = summary
            .specs
            .iter()
            .filter(|report| report.status == ParityStatus::Match)
            .count();
        let mismatches = summary
            .specs
            .iter()
            .filter(|report| report.status == ParityStatus::Mismatch)
            .count();
        let inconclusive = total.saturating_sub(matches + mismatches);

        tracing::info!(
            total,
            matches,
            mismatches,
            inconclusive,
            output = %self.config.output_dir.display(),
            "Parity harness completed diff report generation."
        );

        Ok(())
    }

    fn execute_spec(&self, spec: &DiscoveredSpec) -> Result<SpecReport> {
        let inputs = resolve_spec_inputs(spec);
        let rust_result = self.run_rust_tlc(&inputs);
        let legacy_result = self.run_legacy_tlc(&inputs);

        let status_notes = determine_status(&rust_result, &legacy_result);
        let diff_path = self.write_diff(spec, &rust_result, &legacy_result, &status_notes)?;

        Ok(SpecReport {
            spec: spec.display_name.clone(),
            status: status_notes.status,
            diff_artifact: diff_path,
            notes: status_notes.note,
        })
    }

    fn run_rust_tlc(&self, inputs: &SpecExecutionInputs) -> ExecutionResult {
        let mut args = Vec::new();
        args.push(OsString::from("run"));
        args.push(OsString::from("--spec"));
        args.push(inputs.spec_path.clone().into_os_string());

        if let Some(config) = inputs.config_path.as_ref() {
            args.push(OsString::from("--config"));
            args.push(config.clone().into_os_string());
        }

        if let Some(workers) = self.config.workers {
            args.push(OsString::from("--workers"));
            args.push(OsString::from(workers.to_string()));
        }

        run_command(CommandDescriptor {
            binary: self.rust_binary.clone(),
            args,
            working_dir: inputs.working_dir.clone(),
            role: CommandRole::Rust,
        })
    }

    fn run_legacy_tlc(&self, inputs: &SpecExecutionInputs) -> ExecutionResult {
        let mut args = Vec::new();
        if let Some(config) = inputs.config_path.as_ref() {
            args.push(OsString::from("-config"));
            args.push(config.clone().into_os_string());
        }
        args.push(inputs.spec_path.clone().into_os_string());

        run_command(CommandDescriptor {
            binary: self.config.legacy_launcher.clone(),
            args,
            working_dir: inputs.working_dir.clone(),
            role: CommandRole::Legacy,
        })
    }

    fn write_summary(&self, summary: &SummaryReport) -> Result<()> {
        let summary_path = self.config.output_dir.join("summary.json");
        let payload = serde_json::to_vec_pretty(summary)
            .context("failed to serialize parity summary placeholders")?;
        fs::write(&summary_path, payload).with_context(|| {
            format!(
                "failed to write parity summary file '{}'",
                summary_path.display()
            )
        })?;
        Ok(())
    }

    fn write_diff(
        &self,
        spec: &DiscoveredSpec,
        rust: &ExecutionResult,
        legacy: &ExecutionResult,
        status_notes: &StatusNotes,
    ) -> Result<PathBuf> {
        let relative = PathBuf::from("diffs")
            .join(sanitize_spec_name(&spec.display_name))
            .join("diff.txt");
        let full_path = self.config.output_dir.join(&relative);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create diff directory '{}'", parent.display())
            })?;
        }

        let mut contents = String::new();
        writeln!(contents, "Spec: {}", spec.display_name)?;
        writeln!(contents, "Status: {:?}", status_notes.status)?;
        if let Some(note) = status_notes.note.as_ref() {
            writeln!(contents, "Notes: {}", note)?;
        }
        writeln!(contents)?;
        format_execution_block(&mut contents, "Rust TLC", rust)?;
        writeln!(contents)?;
        format_execution_block(&mut contents, "Legacy TLC", legacy)?;

        fs::write(&full_path, contents).with_context(|| {
            format!("failed to write parity diff file '{}'", full_path.display())
        })?;

        Ok(relative)
    }
}

#[derive(Debug)]
struct DiscoveredSpec {
    display_name: String,
    path: PathBuf,
}

fn discover_specs(config: &HarnessConfig) -> Result<Vec<DiscoveredSpec>> {
    let matcher = if let Some(pattern) = config.filter.as_ref() {
        Some(build_filter(pattern)?)
    } else {
        None
    };

    let mut specs = Vec::new();
    let entries = fs::read_dir(&config.specs_root).with_context(|| {
        format!(
            "failed to read specs directory '{}'",
            config.specs_root.display()
        )
    })?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if let Some(display_name) = derive_display_name(&path) {
            if let Some(matcher) = matcher.as_ref() {
                if !matcher.is_match(&display_name) {
                    continue;
                }
            }
            specs.push(DiscoveredSpec { display_name, path });
        }
    }

    specs.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(specs)
}

fn build_filter(pattern: &str) -> Result<GlobMatcher> {
    let glob = GlobBuilder::new(pattern)
        .case_insensitive(true)
        .backslash_escape(true)
        .build()
        .with_context(|| format!("invalid filter glob pattern '{}'", pattern))?;
    Ok(glob.compile_matcher())
}

fn derive_display_name(path: &Path) -> Option<String> {
    if path.is_dir() {
        return path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|s| s.to_string());
    }

    if path.is_file() {
        let extension = path.extension().and_then(|ext| ext.to_str())?;
        if extension.eq_ignore_ascii_case("tla") {
            return path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(|s| s.to_string());
        }
    }

    None
}

fn sanitize_spec_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' => c,
            _ => '_',
        })
        .collect()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ParityStatus {
    Match,
    Mismatch,
    Inconclusive,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct SpecReport {
    spec: String,
    status: ParityStatus,
    diff_artifact: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
struct SummaryReport {
    specs: Vec<SpecReport>,
}

#[derive(Debug, Clone)]
struct SpecExecutionInputs {
    spec_path: PathBuf,
    config_path: Option<PathBuf>,
    working_dir: PathBuf,
}

fn resolve_spec_inputs(spec: &DiscoveredSpec) -> SpecExecutionInputs {
    if spec.path.is_dir() {
        let preferred = spec.path.join(format!("{}.tla", spec.display_name));
        let spec_path = if preferred.is_file() {
            preferred
        } else {
            spec.path.clone()
        };
        let config_path = find_config_file(&spec.path, &spec.display_name);
        SpecExecutionInputs {
            spec_path,
            config_path,
            working_dir: spec.path.clone(),
        }
    } else {
        let working_dir = spec
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let config_path = find_config_file(&working_dir, &spec.display_name);
        SpecExecutionInputs {
            spec_path: spec.path.clone(),
            config_path,
            working_dir,
        }
    }
}

fn find_config_file(dir: &Path, display_name: &str) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let preferred = dir.join(format!("{}.cfg", display_name));
    if preferred.is_file() {
        return Some(preferred);
    }
    let mut fallback = None;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("cfg"))
                .unwrap_or(false)
            {
                fallback = Some(path);
                break;
            }
        }
    }
    fallback
}

#[derive(Debug, Clone)]
struct CommandDescriptor {
    binary: PathBuf,
    args: Vec<OsString>,
    working_dir: PathBuf,
    role: CommandRole,
}

#[derive(Debug, Clone, Copy)]
enum CommandRole {
    Rust,
    Legacy,
}

impl CommandRole {
    fn env_value(self) -> &'static str {
        match self {
            CommandRole::Rust => "rust",
            CommandRole::Legacy => "legacy",
        }
    }
}

#[derive(Debug, Clone)]
struct ExecutionResult {
    descriptor: CommandDescriptor,
    status: Option<i32>,
    stdout: String,
    stderr: String,
    error: Option<String>,
    executed: bool,
}

impl ExecutionResult {
    fn executed(&self) -> bool {
        self.executed
    }

    fn error_message(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

fn run_command(descriptor: CommandDescriptor) -> ExecutionResult {
    if !descriptor.binary.exists() {
        let message = format!("binary '{}' not found", descriptor.binary.display());
        return ExecutionResult {
            descriptor,
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(message),
            executed: false,
        };
    }

    let mut command = Command::new(&descriptor.binary);
    command.args(&descriptor.args);
    command.current_dir(&descriptor.working_dir);
    command.env("TLC_PARITY_ROLE", descriptor.role.env_value());

    match command.output() {
        Ok(output) => ExecutionResult {
            descriptor,
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            error: None,
            executed: true,
        },
        Err(error) => ExecutionResult {
            descriptor,
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(error.to_string()),
            executed: false,
        },
    }
}

#[derive(Debug, Clone)]
struct StatusNotes {
    status: ParityStatus,
    note: Option<String>,
}

fn determine_status(rust: &ExecutionResult, legacy: &ExecutionResult) -> StatusNotes {
    let mut notes = Vec::new();

    if !rust.executed() {
        notes.push(match rust.error_message() {
            Some(message) => format!("rust TLC not executed: {message}"),
            None => "rust TLC not executed".to_string(),
        });
    }
    if !legacy.executed() {
        notes.push(match legacy.error_message() {
            Some(message) => format!("legacy TLC not executed: {message}"),
            None => "legacy TLC not executed".to_string(),
        });
    }

    if !rust.executed() || !legacy.executed() {
        return StatusNotes {
            status: ParityStatus::Inconclusive,
            note: join_notes(&notes),
        };
    }

    let mut mismatch_reasons = Vec::new();

    if rust.status != legacy.status {
        mismatch_reasons.push(format!(
            "exit codes differ (rust: {}, legacy: {})",
            display_status(rust.status),
            display_status(legacy.status)
        ));
    } else if rust.status != Some(0) {
        mismatch_reasons.push(format!(
            "exit code {} returned by both binaries",
            display_status(rust.status)
        ));
    }

    let stdout_equal = rust.stdout == legacy.stdout;
    let stderr_equal = rust.stderr == legacy.stderr;

    if !stdout_equal {
        mismatch_reasons.push("stdout differs".to_string());
    }
    if !stderr_equal {
        mismatch_reasons.push("stderr differs".to_string());
    }

    if mismatch_reasons.is_empty() {
        StatusNotes {
            status: ParityStatus::Match,
            note: join_notes(&notes),
        }
    } else {
        notes.extend(mismatch_reasons);
        StatusNotes {
            status: ParityStatus::Mismatch,
            note: join_notes(&notes),
        }
    }
}

fn join_notes(notes: &[String]) -> Option<String> {
    if notes.is_empty() {
        None
    } else {
        Some(notes.join("; "))
    }
}

fn display_status(status: Option<i32>) -> String {
    match status {
        Some(code) => code.to_string(),
        None => "signal".to_string(),
    }
}

fn format_execution_block(
    buffer: &mut String,
    header: &str,
    result: &ExecutionResult,
) -> Result<()> {
    writeln!(buffer, "{header}")?;
    writeln!(buffer, "{}", "=".repeat(header.len()))?;
    writeln!(buffer, "Binary: {}", result.descriptor.binary.display())?;
    if !result.descriptor.args.is_empty() {
        writeln!(
            buffer,
            "Args: {}",
            format_args_list(&result.descriptor.args)
        )?;
    }
    writeln!(
        buffer,
        "Working dir: {}",
        result.descriptor.working_dir.display()
    )?;
    writeln!(buffer, "Executed: {}", result.executed)?;
    writeln!(
        buffer,
        "Exit status: {}",
        if result.executed {
            display_status(result.status)
        } else {
            "n/a".to_string()
        }
    )?;
    if let Some(error) = result.error_message() {
        writeln!(buffer, "Error: {error}")?;
    }
    format_multiline(buffer, "Stdout", &result.stdout)?;
    format_multiline(buffer, "Stderr", &result.stderr)?;
    Ok(())
}

fn format_args_list(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| {
            let value = arg.to_string_lossy();
            if value.chars().any(char::is_whitespace) {
                format!("\"{value}\"")
            } else {
                value.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_multiline(buffer: &mut String, label: &str, content: &str) -> Result<()> {
    if content.is_empty() {
        writeln!(buffer, "{label}: <empty>")?;
        return Ok(());
    }

    writeln!(buffer, "{label}:")?;
    for line in content.lines() {
        writeln!(buffer, "  {line}")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, process::Command};
    use tempfile::tempdir;

    #[test]
    fn engine_invariants_detect_invalid_partition() {
        let result = super::run_engine_invariant_suite(
            |frontier_len, workers| {
                let worker_count = workers.get();
                let mut slices = Vec::with_capacity(worker_count);
                let mut cursor = 0usize;
                for idx in 0..worker_count {
                    let extra = usize::from(idx < (frontier_len % worker_count));
                    let chunk = frontier_len / worker_count + extra;
                    let end = cursor + chunk;
                    // Introduce a gap at the start of the first slice.
                    let start = if idx == 0 {
                        cursor.saturating_add(1)
                    } else {
                        cursor
                    };
                    slices.push(FrontierSlice { start, end });
                    cursor = end;
                }
                slices
            },
            8,
        );
        assert!(
            result.is_err(),
            "invalid partitioning should be flagged by invariants"
        );
    }

    #[test]
    fn engine_invariants_pass_for_partition_frontier() {
        super::run_engine_invariant_suite(partition_frontier, 8)
            .expect("partition_frontier should satisfy engine invariants");
    }

    #[test]
    fn reports_match_when_outputs_are_identical() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();

        // Directory-style spec.
        let spec_a = specs_root.join("SpecA");
        fs::create_dir_all(&spec_a).unwrap();

        // File-style spec.
        let spec_b = specs_root.join("SpecB.tla");
        fs::write(&spec_b, "---- MODULE SpecB ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let stub = build_stub_binary(
            temp.path(),
            "tlc_stub_match",
            r#"
fn main() {
    println!("parity stub output");
}
"#,
        );

        let config = HarnessConfig {
            legacy_launcher: stub.clone(),
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: stub,
            workers: Some(4),
            golden_cache: None,
        };

        run_with_config(config).expect("run harness");

        let summary_path = output_dir.join("summary.json");
        assert!(
            summary_path.is_file(),
            "summary file should exist at {:?}",
            summary_path
        );

        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 2);

        let mut spec_names: Vec<_> = summary.specs.iter().map(|spec| spec.spec.clone()).collect();
        spec_names.sort();
        assert_eq!(spec_names, vec!["SpecA", "SpecB"]);

        for spec in summary.specs {
            assert_eq!(spec.status, ParityStatus::Match);
            assert!(spec.notes.is_none());
            let diff_path = output_dir.join(&spec.diff_artifact);
            assert!(
                diff_path.is_file(),
                "diff artifact should exist at {:?}",
                diff_path
            );
            let contents = fs::read_to_string(diff_path).unwrap();
            assert!(
                contents.contains("Status: Match"),
                "diff contents should mention match status"
            );
        }
    }

    #[test]
    fn marks_mismatch_when_outputs_differ() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();

        fs::create_dir_all(specs_root.join("SpecA")).unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub = build_stub_binary(
            temp.path(),
            "rust_stub",
            r#"
fn main() {
    println!("rust output");
}
"#,
        );
        let legacy_stub = build_stub_binary(
            temp.path(),
            "legacy_stub",
            r#"
fn main() {
    println!("legacy output");
}
"#,
        );

        let config = HarnessConfig {
            legacy_launcher: legacy_stub,
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: rust_stub,
            workers: Some(2),
            golden_cache: None,
        };

        run_with_config(config).expect("run harness mismatch");

        let summary_path = output_dir.join("summary.json");
        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 1);
        let report = &summary.specs[0];
        assert_eq!(report.status, ParityStatus::Mismatch);
        let note = report.notes.as_deref().unwrap_or("");
        assert!(
            note.contains("stdout differs"),
            "notes should mention stdout diff, got: {note}"
        );
        let diff_path = output_dir.join(&report.diff_artifact);
        let contents = fs::read_to_string(diff_path).unwrap();
        assert!(
            contents.contains("rust output") && contents.contains("legacy output"),
            "diff contents should capture both outputs"
        );
    }

    #[test]
    fn reports_inconclusive_when_legacy_launcher_missing() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();
        fs::write(specs_root.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub = build_stub_binary(
            temp.path(),
            "rust_stub_inconclusive",
            r#"
fn main() {
    println!("rust output");
}
"#,
        );

        let config = HarnessConfig {
            legacy_launcher: temp.path().join("does-not-exist"),
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        run_with_config(config).expect("run harness inconclusive");

        let summary_path = output_dir.join("summary.json");
        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 1);
        let report = &summary.specs[0];
        assert_eq!(report.status, ParityStatus::Inconclusive);
        let note = report.notes.as_deref().unwrap_or("");
        assert!(
            note.contains("legacy TLC not executed"),
            "expected note about missing legacy binary"
        );
        let diff_path = output_dir.join(&report.diff_artifact);
        let contents = fs::read_to_string(diff_path).unwrap();
        assert!(
            contents.contains("Executed: false"),
            "diff should record missing execution"
        );
    }

    fn build_stub_binary(dir: &Path, name: &str, body: &str) -> PathBuf {
        let source_path = dir.join(format!("{name}.rs"));
        let mut source = fs::File::create(&source_path).expect("create stub source");
        source
            .write_all(
                format!(
                    r#"
{body}
"#
                )
                .as_bytes(),
            )
            .expect("write stub source");
        let binary_path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        let status = Command::new("rustc")
            .arg(&source_path)
            .arg("-O")
            .arg("-o")
            .arg(&binary_path)
            .status()
            .expect("invoke rustc");
        assert!(status.success(), "rustc failed to compile stub binary");
        binary_path
    }
}
