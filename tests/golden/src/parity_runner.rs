use std::{
    collections::BTreeSet,
    ffi::OsString,
    fmt::Write as _,
    fs, io,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

use anyhow::{anyhow, ensure, Context, Result};
use clap::Parser;
use globset::{GlobBuilder, GlobMatcher};
use proptest::{
    prelude::*,
    test_runner::{Config as ProptestConfig, TestRunner},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::TempDir;
use tlc_engine::{partition_frontier, FrontierSlice};
use tlc_test_support::progress::{analyze_progress, parse_progress_ndjson, ProgressThresholds};
use ulid::Ulid;

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
        let inputs = resolve_spec_inputs(spec)?;
        let rust_result = self.run_rust_tlc(&inputs)?;
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

    fn run_rust_tlc(&self, inputs: &SpecExecutionInputs) -> Result<ExecutionResult> {
        let mut args = Vec::new();
        args.push(OsString::from("run"));
        args.push(OsString::from("--spec"));
        args.push(inputs.spec_path.clone().into_os_string());

        if let Some(config) = inputs.config_path.as_ref() {
            args.push(OsString::from("--config"));
            args.push(config.clone().into_os_string());
        }

        args.push(OsString::from("--progress"));
        args.push(OsString::from("ndjson"));

        if let Some(workers) = self.config.workers {
            args.push(OsString::from("--workers"));
            args.push(OsString::from(workers.to_string()));
        }

        let mut user_file = None;
        if let Some(logging) = inputs
            .legacy_quirks
            .as_ref()
            .and_then(|meta| meta.logging.as_ref())
        {
            if logging.suppress_warnings {
                args.push(OsString::from("--suppress-warnings"));
            }
            if logging.debug {
                args.push(OsString::from("--debug"));
            }
            if logging.terse {
                args.push(OsString::from("--terse"));
            }
            if let Some(path) = logging.user_file.as_ref() {
                args.push(OsString::from("--user-file"));
                args.push(path.clone().into_os_string());
                user_file = Some(path.clone());
            }
        }

        let result = run_command(CommandDescriptor {
            binary: self.rust_binary.clone(),
            args,
            working_dir: inputs.working_dir.clone(),
            role: CommandRole::Rust,
            user_file,
        });

        if result.executed() {
            let metrics = validate_run_metrics(&result.descriptor.working_dir)?;
            validate_progress_accuracy(&result.stdout, &metrics)?;
        }

        Ok(result)
    }

    fn run_legacy_tlc(&self, inputs: &SpecExecutionInputs) -> ExecutionResult {
        let mut args = Vec::new();
        if let Some(config) = inputs.config_path.as_ref() {
            args.push(OsString::from("-config"));
            args.push(config.clone().into_os_string());
        }
        args.push(inputs.spec_path.clone().into_os_string());

        let mut user_file = None;
        if let Some(logging) = inputs
            .legacy_quirks
            .as_ref()
            .and_then(|meta| meta.logging.as_ref())
        {
            if logging.suppress_warnings {
                args.push(OsString::from("-nowarning"));
            }
            if logging.debug {
                args.push(OsString::from("-debug"));
            }
            if logging.terse {
                args.push(OsString::from("-terse"));
            }
            if let Some(path) = logging.user_file.as_ref() {
                args.push(OsString::from("-userFile"));
                args.push(path.clone().into_os_string());
                user_file = Some(path.clone());
            }
        }

        run_command(CommandDescriptor {
            binary: self.config.legacy_launcher.clone(),
            args,
            working_dir: inputs.working_dir.clone(),
            role: CommandRole::Legacy,
            user_file,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpecExecutionInputs {
    spec_path: PathBuf,
    config_path: Option<PathBuf>,
    working_dir: PathBuf,
    legacy_quirks: Option<LegacyQuirkMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyQuirkMetadata {
    name: Option<String>,
    description: Option<String>,
    deprecated_flags: Vec<String>,
    cli_notes: Option<String>,
    unicode_identifiers: Vec<String>,
    unicode_strings: Vec<String>,
    logging: Option<LoggingToggles>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoggingToggles {
    suppress_warnings: bool,
    debug: bool,
    terse: bool,
    user_file: Option<PathBuf>,
}

fn resolve_spec_inputs(spec: &DiscoveredSpec) -> Result<SpecExecutionInputs> {
    if spec.path.is_dir() {
        let preferred = spec.path.join(format!("{}.tla", spec.display_name));
        let spec_path = if preferred.is_file() {
            preferred
        } else {
            spec.path.clone()
        };
        let config_path = find_config_file(&spec.path, &spec.display_name);
        let legacy_quirks = load_legacy_quirks(&spec.path)?;
        Ok(SpecExecutionInputs {
            spec_path,
            config_path,
            working_dir: spec.path.clone(),
            legacy_quirks,
        })
    } else {
        let working_dir = spec
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let config_path = find_config_file(&working_dir, &spec.display_name);
        let legacy_quirks = load_legacy_quirks(&working_dir)?;
        Ok(SpecExecutionInputs {
            spec_path: spec.path.clone(),
            config_path,
            working_dir,
            legacy_quirks,
        })
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

fn load_legacy_quirks(dir: &Path) -> Result<Option<LegacyQuirkMetadata>> {
    let fixture_path = dir.join("fixture.toml");
    if !fixture_path.is_file() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&fixture_path).with_context(|| {
        format!(
            "failed to read legacy fixture metadata '{}'",
            fixture_path.display()
        )
    })?;
    let raw: RawLegacyFixture = toml::from_str(&contents).with_context(|| {
        format!(
            "failed to parse legacy fixture metadata '{}'",
            fixture_path.display()
        )
    })?;

    let mut deprecated_flags = Vec::new();
    let mut cli_notes = None;
    let mut logging = None;
    if let Some(cli) = raw.cli {
        cli_notes = cli.notes.clone();
        let suppress_warnings = cli.suppress_warnings;
        let debug = cli.debug;
        let terse = cli.terse;
        let user_file_cfg = cli.user_file.clone();

        let mut seen = BTreeSet::new();
        for flag in cli.deprecated_flags {
            let trimmed = flag.trim();
            ensure!(
                !trimmed.is_empty(),
                "legacy fixture '{}' contains an empty deprecated flag entry",
                fixture_path.display()
            );
            ensure!(
                trimmed.starts_with('-'),
                "legacy fixture '{}' lists deprecated flag '{}' without '-' prefix",
                fixture_path.display(),
                flag
            );
            if seen.insert(trimmed.to_string()) {
                deprecated_flags.push(trimmed.to_string());
            }
        }

        if suppress_warnings || debug || terse || user_file_cfg.is_some() {
            let user_file = user_file_cfg.map(|value| {
                let candidate = PathBuf::from(value);
                if candidate.is_absolute() {
                    candidate
                } else {
                    dir.join(candidate)
                }
            });
            logging = Some(LoggingToggles {
                suppress_warnings,
                debug,
                terse,
                user_file,
            });
        }
    }

    let unicode = raw.unicode.unwrap_or_default();
    let unicode_identifiers =
        normalize_unicode_values(unicode.identifiers, &fixture_path, "unicode.identifiers")?;
    let unicode_strings =
        normalize_unicode_values(unicode.strings, &fixture_path, "unicode.strings")?;

    Ok(Some(LegacyQuirkMetadata {
        name: raw.name,
        description: raw.description,
        deprecated_flags,
        cli_notes,
        unicode_identifiers,
        unicode_strings,
        logging,
    }))
}

fn normalize_unicode_values(
    values: Vec<String>,
    fixture_path: &Path,
    field: &str,
) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        ensure!(
            !trimmed.is_empty(),
            "legacy fixture '{}' contains an empty entry in '{}'",
            fixture_path.display(),
            field
        );
        if seen.insert(trimmed.to_string()) {
            normalized.push(trimmed.to_string());
        }
    }
    Ok(normalized)
}

#[derive(Debug, Deserialize)]
struct RawLegacyFixture {
    name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    cli: Option<RawLegacyCli>,
    #[serde(default)]
    unicode: Option<RawLegacyUnicode>,
}

#[derive(Debug, Deserialize)]
struct RawLegacyCli {
    #[serde(default)]
    deprecated_flags: Vec<String>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    suppress_warnings: bool,
    #[serde(default)]
    debug: bool,
    #[serde(default)]
    terse: bool,
    #[serde(default)]
    user_file: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLegacyUnicode {
    #[serde(default)]
    identifiers: Vec<String>,
    #[serde(default)]
    strings: Vec<String>,
}

#[derive(Debug, Clone)]
struct CommandDescriptor {
    binary: PathBuf,
    args: Vec<OsString>,
    working_dir: PathBuf,
    role: CommandRole,
    user_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    user_output: Option<String>,
    user_file_error: Option<String>,
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

#[derive(Debug, Clone)]
struct RunMetrics {
    states_explored: u128,
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
            user_output: None,
            user_file_error: None,
            executed: false,
        };
    }

    if descriptor.role == CommandRole::Rust {
        clear_metrics_log(&descriptor.working_dir);
    }

    let mut command = Command::new(&descriptor.binary);
    command.args(&descriptor.args);
    command.current_dir(&descriptor.working_dir);
    command.env("TLC_PARITY_ROLE", descriptor.role.env_value());

    match command.output() {
        Ok(output) => {
            let (user_output, user_file_error) = read_user_output(&descriptor);
            ExecutionResult {
                descriptor,
                status: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                error: None,
                user_output,
                user_file_error,
                executed: true,
            }
        }
        Err(error) => ExecutionResult {
            descriptor,
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(error.to_string()),
            user_output: None,
            user_file_error: None,
            executed: false,
        },
    }
}

fn read_user_output(descriptor: &CommandDescriptor) -> (Option<String>, Option<String>) {
    let path = match descriptor.user_file.as_ref() {
        Some(path) => path,
        None => return (None, None),
    };

    match fs::read_to_string(path) {
        Ok(contents) => (Some(contents), None),
        Err(err) => (
            None,
            Some(format!(
                "failed to read user output '{}': {err}",
                path.display()
            )),
        ),
    }
}

fn clear_metrics_log(working_dir: &Path) {
    let log_path = metrics_log_path(working_dir);
    match fs::remove_file(&log_path) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            tracing::warn!(
                path = %log_path.display(),
                error = %err,
                "failed to clear metrics log before run"
            );
        }
    }
}

fn validate_run_metrics(working_dir: &Path) -> Result<RunMetrics> {
    let log_path = metrics_log_path(working_dir);
    let contents = fs::read_to_string(&log_path).with_context(|| {
        format!(
            "failed to read telemetry log for run metrics at '{}'",
            log_path.display()
        )
    })?;

    for (index, line) in contents.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let event: Value = serde_json::from_str(trimmed).with_context(|| {
            format!(
                "failed to parse telemetry log '{}', line {}",
                log_path.display(),
                index + 1
            )
        })?;

        let fields = match event.get("fields").and_then(Value::as_object) {
            Some(fields) => fields,
            None => continue,
        };

        if fields
            .get("metric")
            .and_then(Value::as_str)
            .map(|metric| metric == "run_metrics")
            != Some(true)
        {
            continue;
        }

        let run_id_str = parse_string_field(fields, "run_id")?;
        let _run_id = Ulid::from_str(run_id_str)
            .with_context(|| format!("run metrics recorded invalid run_id '{}'", run_id_str))?;

        let runtime_ms = parse_f64_field(fields, "runtime_ms")?;
        ensure!(
            runtime_ms >= 0.0,
            "run metrics reported negative runtime: {}",
            runtime_ms
        );

        let states_explored = parse_u128_field(fields, "states_explored")?;
        ensure!(
            states_explored > 0,
            "run metrics reported zero states explored"
        );

        let _states_per_second = parse_f64_field(fields, "states_per_second")?;
        let _peak_memory_bytes = parse_u64_field(fields, "peak_memory_bytes")?;
        let _peak_memory_reported = parse_bool_field(fields, "peak_memory_reported")?;

        return Ok(RunMetrics { states_explored });
    }

    Err(anyhow!(
        "run metrics event 'run_metrics' not recorded in telemetry log '{}'",
        log_path.display()
    ))
}

fn metrics_log_path(working_dir: &Path) -> PathBuf {
    working_dir.join("logs").join("tlc-trace.json")
}

fn validate_progress_accuracy(stdout: &str, metrics: &RunMetrics) -> Result<()> {
    let samples = parse_progress_ndjson(stdout)
        .context("failed to parse progress NDJSON from Rust TLC stdout")?;
    let thresholds = ProgressThresholds::default();
    let analysis = analyze_progress(&samples, metrics.states_explored)
        .context("progress accuracy analysis failed")?;

    if !analysis.meets(&thresholds) {
        let max_gap_secs = analysis.max_refresh_gap().num_milliseconds() as f64 / 1_000.0;
        let allowed_gap_secs = thresholds.max_refresh_gap.num_milliseconds() as f64 / 1_000.0;
        return Err(anyhow!(
            "progress thresholds violated: max refresh gap {:.2}s (allowed {:.2}s), coverage delta {:.2}% (allowed {:.2}%), events observed {}",
            max_gap_secs,
            allowed_gap_secs,
            analysis.coverage_delta(),
            thresholds.max_coverage_delta,
            analysis.event_count()
        ));
    }

    Ok(())
}

fn parse_string_field<'a>(
    fields: &'a serde_json::Map<String, Value>,
    name: &str,
) -> Result<&'a str> {
    fields
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("run metrics missing string field '{}'", name))
}

fn parse_f64_field(fields: &serde_json::Map<String, Value>, name: &str) -> Result<f64> {
    let value = fields
        .get(name)
        .ok_or_else(|| anyhow!("run metrics missing numeric field '{}'", name))?;
    if let Some(number) = value.as_f64() {
        return Ok(number);
    }
    if let Some(as_str) = value.as_str() {
        return as_str
            .parse::<f64>()
            .with_context(|| format!("run metrics field '{}' not a float", name));
    }
    Err(anyhow!(
        "run metrics field '{}' must be a number or string",
        name
    ))
}

fn parse_u64_field(fields: &serde_json::Map<String, Value>, name: &str) -> Result<u64> {
    let value = fields
        .get(name)
        .ok_or_else(|| anyhow!("run metrics missing integer field '{}'", name))?;
    if let Some(number) = value.as_u64() {
        return Ok(number);
    }
    let text = if let Some(as_str) = value.as_str() {
        as_str.to_string()
    } else {
        value.to_string()
    };
    text.parse::<u64>()
        .with_context(|| format!("run metrics field '{}' not a u64", name))
}

fn parse_u128_field(fields: &serde_json::Map<String, Value>, name: &str) -> Result<u128> {
    let value = fields
        .get(name)
        .ok_or_else(|| anyhow!("run metrics missing integer field '{}'", name))?;
    if let Some(as_str) = value.as_str() {
        return as_str
            .parse::<u128>()
            .with_context(|| format!("run metrics field '{}' not a u128", name));
    }
    if let Some(number) = value.as_u64() {
        return Ok(number as u128);
    }
    value
        .to_string()
        .parse::<u128>()
        .with_context(|| format!("run metrics field '{}' not a u128", name))
}

fn parse_bool_field(fields: &serde_json::Map<String, Value>, name: &str) -> Result<bool> {
    let value = fields
        .get(name)
        .ok_or_else(|| anyhow!("run metrics missing boolean field '{}'", name))?;
    if let Some(flag) = value.as_bool() {
        return Ok(flag);
    }
    if let Some(as_str) = value.as_str() {
        return as_str
            .parse::<bool>()
            .with_context(|| format!("run metrics field '{}' not a boolean", name));
    }
    Err(anyhow!("run metrics field '{}' must be a boolean", name))
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

    if let Some(error) = &rust.user_file_error {
        mismatch_reasons.push(format!("rust user output error: {error}"));
    }
    if let Some(error) = &legacy.user_file_error {
        mismatch_reasons.push(format!("legacy user output error: {error}"));
    }

    match (&rust.user_output, &legacy.user_output) {
        (Some(rust_output), Some(legacy_output)) => {
            if rust_output != legacy_output {
                mismatch_reasons.push("user output differs".to_string());
            }
        }
        (Some(_), None) => mismatch_reasons.push("legacy user output missing".to_string()),
        (None, Some(_)) => mismatch_reasons.push("rust user output missing".to_string()),
        (None, None) => {}
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
    if let Some(user_file) = result.descriptor.user_file.as_ref() {
        writeln!(buffer, "User file: {}", user_file.display())?;
        if let Some(error) = result.user_file_error.as_ref() {
            writeln!(buffer, "User file error: {error}")?;
        }
        if let Some(output) = result.user_output.as_ref() {
            format_multiline(buffer, "User Output", output)?;
        } else if result.user_file_error.is_none() {
            format_multiline(buffer, "User Output", "<empty>")?;
        }
    }
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
    use std::{fs, io::Write, process::Command};
    use tempfile::tempdir;

    const FINAL_SUMMARY: &str = concat!(
        "Model checking completed. No error has been found.\n",
        "  Estimates of the probability that TLC did not check all reachable states\n",
        "  because two distinct states had the same fingerprint:\n",
        "  calculated (optimistic):  1.0000000000000000E-12\n",
        "  based on the actual fingerprints:  2.0000000000000000E-12\n",
        "1,337 states generated, 1,000 distinct states found, 0 states left on queue.\n",
        "The depth of the complete state graph search is 7.\n",
        "Finished in 02min 05s at (2025-11-02 04:00:00)"
    );

    const MISMATCHED_FINAL_SUMMARY: &str = concat!(
        "Model checking completed. No error has been found.\n",
        "  Estimates of the probability that TLC did not check all reachable states\n",
        "  because two distinct states had the same fingerprint:\n",
        "  calculated (optimistic):  1.0000000000000000E-12\n",
        "  based on the actual fingerprints:  2.0000000000000000E-12\n",
        "1,337 states generated, 1,000 distinct states found, 0 states left on queue.\n",
        "The depth of the complete state graph search is 7.\n",
        "Finished in 02min 06s at (2025-11-02 04:00:00)"
    );

    const MP_WARNING_LINES: &str = concat!(
        "Warning: Temporal properties (PROPERTY or PROPERTIES) are being verified without a behavior specification (SPECIFICATION). Only INIT and NEXT have been provided. This is likely to result in (trivial) counterexamples showing infinite stuttering following the initial state. It is recommended to use SPECIFICATION Spec, with Spec asserting a suitable fairness constraint (compare Chapter 8, page 87ff of Specifying Systems at https://lamport.azurewebsites.net/tla/book.html).\n",
        "Warning: Temporal properties (PROPERTY or PROPERTIES) are being verified without a fairness constraint conjoined to the behavior specification Spec defined at Spec.tla:42. This may lead to trivial counterexamples in which the system exhibits infinite stuttering immediately after the initial state. To avoid this, it is recommended to conjoin a suitable fairness constraint to Spec (compare Chapter 8, page 87ff of Specifying Systems at https://lamport.azurewebsites.net/tla/book.html).\n"
    );

    fn escape_for_string_literal(input: &str) -> String {
        let mut escaped = String::with_capacity(input.len());
        for ch in input.chars() {
            match ch {
                '\\' => escaped.push_str("\\\\"),
                '"' => escaped.push_str("\\\""),
                '\n' => escaped.push_str("\\n"),
                '\r' => escaped.push_str("\\r"),
                _ => escaped.push(ch),
            }
        }
        escaped
    }

    fn stub_with_progress(stdout: &str, events: &[&str]) -> String {
        let mut code = String::new();
        code.push_str("use std::fs::{self, OpenOptions};\n");
        code.push_str("use std::io::Write;\n\n");
        code.push_str("fn main() {\n");
        code.push_str("    write_metrics().expect(\"metrics log written\");\n");
        code.push_str("    emit_progress().expect(\"progress stream written\");\n");
        code.push_str("    println!(\"");
        code.push_str(&escape_for_string_literal(stdout));
        code.push_str("\");\n");
        code.push_str("}\n\n");
        code.push_str("fn write_metrics() -> std::io::Result<()> {\n");
        code.push_str("    let cwd = std::env::current_dir()?;\n");
        code.push_str("    let log_dir = cwd.join(\"logs\");\n");
        code.push_str("    fs::create_dir_all(&log_dir)?;\n");
        code.push_str("    let log_path = log_dir.join(\"tlc-trace.json\");\n");
        code.push_str("    let mut file = OpenOptions::new()\n");
        code.push_str("        .create(true)\n");
        code.push_str("        .write(true)\n");
        code.push_str("        .truncate(true)\n");
        code.push_str("        .open(&log_path)?;\n");
        code.push_str("    let payload = r#\"{\"timestamp\":\"2025-11-02T00:00:00Z\",\"level\":\"INFO\",\"fields\":{\"message\":\"run metrics recorded\",\"metric\":\"run_metrics\",\"run_id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAV\",\"runtime_ms\":2000.0,\"states_explored\":\"1337\",\"states_per_second\":668.5,\"peak_memory_bytes\":\"1048576\",\"peak_memory_reported\":true}}\"#;\n");
        code.push_str("    writeln!(file, \"{payload}\")?;\n");
        code.push_str("    Ok(())\n");
        code.push_str("}\n");
        code.push_str("\n");
        code.push_str("fn emit_progress() -> std::io::Result<()> {\n");
        code.push_str("    let events = [\n");
        for event in events {
            code.push_str("        ");
            code.push_str("r#\"");
            code.push_str(event);
            code.push_str("\"#");
            code.push_str(",\n");
        }
        code.push_str("    ];\n");
        code.push_str("    for event in &events {\n");
        code.push_str("        println!(\"{}\", event);\n");
        code.push_str("    }\n");
        code.push_str("    Ok(())\n");
        code.push_str("}\n");
        code
    }

    fn warning_sensitive_stub(stdout: &str, warning: &str) -> String {
        let mut code = metrics_stub_source(stdout);
        let target = format!("    println!(\"{}\");\n", escape_for_string_literal(stdout));
        let replacement = format!(
            "    let args: Vec<String> = std::env::args().collect();\n    let suppress = args.iter().any(|arg| arg == \"--suppress-warnings\" || arg == \"-nowarning\");\n    if !suppress {{\n        println!(\"{}\");\n    }}\n    println!(\"{}\");\n",
            escape_for_string_literal(warning),
            escape_for_string_literal(stdout)
        );
        if let Some(index) = code.find(&target) {
            code.replace_range(index..index + target.len(), &replacement);
        }
        code
    }

    fn user_file_stub(stdout: &str, user_content: &str) -> String {
        let mut code = metrics_stub_source(stdout);
        let target = format!("    println!(\"{}\");\n", escape_for_string_literal(stdout));
        let replacement = format!(
            "    let mut args = std::env::args().skip(1);\n    let mut user_file = None;\n    while let Some(arg) = args.next() {{\n        if arg == \"--user-file\" || arg == \"-userFile\" {{\n            user_file = args.next();\n            break;\n        }}\n    }}\n    if let Some(path) = user_file {{\n        fs::write(&path, \"{}\").expect(\"write user file\");\n    }}\n    println!(\"{}\");\n",
            escape_for_string_literal(user_content),
            escape_for_string_literal(stdout)
        );
        if let Some(index) = code.find(&target) {
            code.replace_range(index..index + target.len(), &replacement);
        }
        code
    }

    fn metrics_stub_source(stdout: &str) -> String {
        let events = [
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FA","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:00Z","states_explored":"0","percent_complete":0.0,"throughput_eps":0.0,"workers_active":4}"#,
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FB","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:03Z","states_explored":"400","percent_complete":30.0,"throughput_eps":150.0,"workers_active":4,"eta_seconds":7}"#,
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FC","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:06Z","states_explored":"900","percent_complete":65.0,"throughput_eps":180.0,"workers_active":4,"eta_seconds":4}"#,
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FD","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:09Z","states_explored":"1337","percent_complete":100.0,"throughput_eps":200.0,"workers_active":4}"#,
        ];
        stub_with_progress(stdout, &events)
    }

    fn missing_progress_stub_source(stdout: &str) -> String {
        let mut code = String::new();
        code.push_str("use std::fs::{self, OpenOptions};\n");
        code.push_str("use std::io::Write;\n\n");
        code.push_str("fn main() {\n");
        code.push_str("    write_metrics().expect(\"metrics log written\");\n");
        code.push_str("    println!(\"");
        code.push_str(&escape_for_string_literal(stdout));
        code.push_str("\");\n");
        code.push_str("}\n\n");
        code.push_str("fn write_metrics() -> std::io::Result<()> {\n");
        code.push_str("    let cwd = std::env::current_dir()?;\n");
        code.push_str("    let log_dir = cwd.join(\"logs\");\n");
        code.push_str("    fs::create_dir_all(&log_dir)?;\n");
        code.push_str("    let log_path = log_dir.join(\"tlc-trace.json\");\n");
        code.push_str("    let mut file = OpenOptions::new()\n");
        code.push_str("        .create(true)\n");
        code.push_str("        .write(true)\n");
        code.push_str("        .truncate(true)\n");
        code.push_str("        .open(&log_path)?;\n");
        code.push_str("    let payload = r#\"{\"timestamp\":\"2025-11-02T00:00:00Z\",\"level\":\"INFO\",\"fields\":{\"message\":\"run metrics recorded\",\"metric\":\"run_metrics\",\"run_id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAV\",\"runtime_ms\":2000.0,\"states_explored\":\"1337\",\"states_per_second\":668.5,\"peak_memory_bytes\":\"1048576\",\"peak_memory_reported\":true}}\"#;\n");
        code.push_str("    writeln!(file, \"{payload}\")?;\n");
        code.push_str("    Ok(())\n");
        code.push_str("}\n");
        code
    }

    fn progress_violation_stub_source(stdout: &str) -> String {
        let events = [
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FE","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:00Z","states_explored":"0","percent_complete":0.0,"throughput_eps":0.0,"workers_active":4}"#,
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FF","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:12Z","states_explored":"400","percent_complete":30.0,"throughput_eps":33.0,"workers_active":4,"eta_seconds":20}"#,
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FG","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:27Z","states_explored":"900","percent_complete":65.0,"throughput_eps":20.0,"workers_active":4,"eta_seconds":15}"#,
            r#"{"event_id":"01J5F7K4MZQ8X5Y3S9B7C6D8FH","run_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","timestamp":"2025-11-02T00:00:45Z","states_explored":"1337","percent_complete":85.0,"throughput_eps":10.0,"workers_active":4}"#,
        ];
        stub_with_progress(stdout, &events)
    }

    fn missing_metrics_stub_source(stdout: &str) -> String {
        format!(
            "fn main() {{\n    println!(\"{}\");\n}}\n",
            escape_for_string_literal(stdout)
        )
    }

    fn invalid_metrics_stub_source() -> String {
        let mut code = String::new();
        code.push_str("use std::fs::{self, OpenOptions};\n");
        code.push_str("use std::io::Write;\n\n");
        code.push_str("fn main() {\n");
        code.push_str("    write_metrics().expect(\"invalid metrics log\");\n");
        code.push_str("}\n\n");
        code.push_str("fn write_metrics() -> std::io::Result<()> {\n");
        code.push_str("    let cwd = std::env::current_dir()?;\n");
        code.push_str("    let log_dir = cwd.join(\"logs\");\n");
        code.push_str("    fs::create_dir_all(&log_dir)?;\n");
        code.push_str("    let log_path = log_dir.join(\"tlc-trace.json\");\n");
        code.push_str("    let mut file = OpenOptions::new()\n");
        code.push_str("        .create(true)\n");
        code.push_str("        .write(true)\n");
        code.push_str("        .truncate(true)\n");
        code.push_str("        .open(&log_path)?;\n");
        code.push_str("    let payload = r#\"{\"timestamp\":\"2025-11-02T00:00:00Z\",\"level\":\"INFO\",\"fields\":{\"metric\":\"run_metrics\",\"run_id\":\"not-a-ulid\",\"runtime_ms\":-1.0,\"states_explored\":\"not-an-integer\",\"states_per_second\":\"nan\",\"peak_memory_bytes\":\"abc\",\"peak_memory_reported\":\"maybe\"}}\"#;\n");
        code.push_str("    writeln!(file, \"{payload}\")?;\n");
        code.push_str("    Ok(())\n");
        code.push_str("}\n");
        code
    }

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
    fn loads_legacy_quirk_fixture_metadata() {
        let temp = tempdir().expect("temp dir");
        let spec_dir = temp.path().join("UnicodeLegacy");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(
            spec_dir.join("UnicodeLegacy.tla"),
            "---- MODULE UnicodeLegacy ----",
        )
        .unwrap();
        fs::write(
            spec_dir.join("fixture.toml"),
            r#"
name = "unicode_legacy_flags"
description = "Fixture capturing legacy CLI quirks."

[cli]
deprecated_flags = ["-tool", "-nowarning", "-tool"]
notes = "Deprecated flags must be accepted without crashing."

[unicode]
identifiers = ["UTF_IDENT_A", "UTF_IDENT_A", "UTF_IDENT_B"]
strings = ["UTF_STRING_START", "UTF_STRING_END"]
"#,
        )
        .unwrap();

        let spec = super::DiscoveredSpec {
            display_name: "UnicodeLegacy".to_string(),
            path: spec_dir.clone(),
        };

        let inputs = super::resolve_spec_inputs(&spec).expect("resolve spec inputs");
        let quirks = inputs
            .legacy_quirks
            .expect("expected legacy quirks metadata to be loaded");

        assert_eq!(
            quirks.name.as_deref(),
            Some("unicode_legacy_flags"),
            "fixture name should roundtrip"
        );
        assert_eq!(
            quirks.description.as_deref(),
            Some("Fixture capturing legacy CLI quirks.")
        );
        assert_eq!(
            quirks.deprecated_flags,
            vec!["-tool".to_string(), "-nowarning".to_string()],
            "duplicate deprecated flags should be deduplicated while preserving order"
        );
        assert_eq!(
            quirks.cli_notes.as_deref(),
            Some("Deprecated flags must be accepted without crashing.")
        );
        assert_eq!(
            quirks.unicode_identifiers,
            vec!["UTF_IDENT_A".to_string(), "UTF_IDENT_B".to_string()],
            "duplicate unicode identifiers should be removed"
        );
        assert_eq!(
            quirks.unicode_strings,
            vec!["UTF_STRING_START".to_string(), "UTF_STRING_END".to_string()]
        );
        assert!(
            quirks.logging.is_none(),
            "logging metadata should be absent when not configured"
        );
    }

    #[test]
    fn rejects_invalid_deprecated_flag_in_fixture() {
        let temp = tempdir().expect("temp dir");
        let spec_dir = temp.path().join("LegacySpec");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(
            spec_dir.join("LegacySpec.tla"),
            "---- MODULE LegacySpec ----",
        )
        .unwrap();
        fs::write(
            spec_dir.join("fixture.toml"),
            r#"
[cli]
deprecated_flags = ["tool"]
"#,
        )
        .unwrap();

        let spec = super::DiscoveredSpec {
            display_name: "LegacySpec".to_string(),
            path: spec_dir.clone(),
        };

        let error = super::resolve_spec_inputs(&spec)
            .expect_err("invalid fixture metadata should return an error");

        assert!(
            error.to_string().contains("deprecated flag"),
            "error should mention invalid deprecated flag: {error:?}"
        );
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
        let stub_source = metrics_stub_source(FINAL_SUMMARY);
        let stub = build_stub_binary(temp.path(), "tlc_stub_match", &stub_source);

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
            assert!(
                contents.contains("Finished in 02min 05s"),
                "diff should record final summary banner"
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
        let rust_stub_source = metrics_stub_source(FINAL_SUMMARY);
        let rust_stub = build_stub_binary(temp.path(), "rust_stub", &rust_stub_source);
        let legacy_stub_source = metrics_stub_source(MISMATCHED_FINAL_SUMMARY);
        let legacy_stub = build_stub_binary(temp.path(), "legacy_stub", &legacy_stub_source);

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
            contents.contains("Finished in 02min 05s")
                && contents.contains("Finished in 02min 06s"),
            "diff contents should capture divergent final summaries"
        );
    }

    #[test]
    fn reports_inconclusive_when_legacy_launcher_missing() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();
        fs::write(specs_root.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = metrics_stub_source(FINAL_SUMMARY);
        let rust_stub = build_stub_binary(temp.path(), "rust_stub_inconclusive", &rust_stub_source);

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

    #[test]
    fn mp_warning_outputs_match_legacy() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        let spec_dir = specs_root.join("SpecA");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(spec_dir.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let stdout_payload = format!("{MP_WARNING_LINES}\n{FINAL_SUMMARY}");
        let rust_stub_source = metrics_stub_source(&stdout_payload);
        let rust_stub = build_stub_binary(temp.path(), "rust_mp_stub", &rust_stub_source);
        let legacy_stub_source = metrics_stub_source(&stdout_payload);
        let legacy_stub = build_stub_binary(temp.path(), "legacy_mp_stub", &legacy_stub_source);

        let config = HarnessConfig {
            legacy_launcher: legacy_stub,
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        run_with_config(config).expect("run harness");

        let summary_path = output_dir.join("summary.json");
        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 1);
        assert_eq!(summary.specs[0].status, ParityStatus::Match);
    }

    #[test]
    fn mp_warning_mismatch_detected() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        let spec_dir = specs_root.join("SpecA");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(spec_dir.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stdout = format!("{MP_WARNING_LINES}\n{FINAL_SUMMARY}");
        let rust_stub_source = metrics_stub_source(&rust_stdout);
        let rust_stub = build_stub_binary(temp.path(), "rust_mp_stub_mismatch", &rust_stub_source);

        let legacy_warning = MP_WARNING_LINES.replace("Spec.tla:42", "Spec.tla:24");
        let legacy_stdout = format!("{legacy_warning}\n{FINAL_SUMMARY}");
        let legacy_stub_source = metrics_stub_source(&legacy_stdout);
        let legacy_stub =
            build_stub_binary(temp.path(), "legacy_mp_stub_mismatch", &legacy_stub_source);

        let config = HarnessConfig {
            legacy_launcher: legacy_stub,
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: rust_stub,
            workers: None,
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
            "expected stdout mismatch note, got: {note}"
        );
    }

    #[test]
    fn suppress_warnings_toggle_applied_to_both_binaries() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        let spec_dir = specs_root.join("WarningsSpec");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(
            spec_dir.join("WarningsSpec.tla"),
            "---- MODULE WarningsSpec ----",
        )
        .unwrap();
        fs::write(
            spec_dir.join("fixture.toml"),
            r#"
[cli]
suppress_warnings = true
"#,
        )
        .unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = warning_sensitive_stub(FINAL_SUMMARY, "Warning: rust warning");
        let rust_stub = build_stub_binary(temp.path(), "rust_warning_stub", &rust_stub_source);
        let legacy_stub_source = warning_sensitive_stub(FINAL_SUMMARY, "Warning: legacy warning");
        let legacy_stub =
            build_stub_binary(temp.path(), "legacy_warning_stub", &legacy_stub_source);

        let config = HarnessConfig {
            legacy_launcher: legacy_stub,
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        run_with_config(config).expect("parity harness run");

        let summary_path = output_dir.join("summary.json");
        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 1);
        assert_eq!(summary.specs[0].status, ParityStatus::Match);
    }

    #[test]
    fn user_file_outputs_match() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        let spec_dir = specs_root.join("UserFileSpec");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(
            spec_dir.join("UserFileSpec.tla"),
            "---- MODULE UserFileSpec ----",
        )
        .unwrap();
        fs::write(
            spec_dir.join("fixture.toml"),
            r#"
[cli]
user_file = "user.log"
"#,
        )
        .unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = user_file_stub(FINAL_SUMMARY, "rust user output\n");
        let rust_stub = build_stub_binary(temp.path(), "rust_user_stub", &rust_stub_source);
        let legacy_stub_source = user_file_stub(FINAL_SUMMARY, "rust user output\n");
        let legacy_stub = build_stub_binary(temp.path(), "legacy_user_stub", &legacy_stub_source);

        let config = HarnessConfig {
            legacy_launcher: legacy_stub,
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        run_with_config(config).expect("parity harness run");

        let summary_path = output_dir.join("summary.json");
        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 1);
        assert_eq!(summary.specs[0].status, ParityStatus::Match);
    }

    #[test]
    fn user_file_mismatch_detected() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        let spec_dir = specs_root.join("UserFileMismatch");
        fs::create_dir_all(&spec_dir).unwrap();
        fs::write(
            spec_dir.join("UserFileMismatch.tla"),
            "---- MODULE UserFileMismatch ----",
        )
        .unwrap();
        fs::write(
            spec_dir.join("fixture.toml"),
            r#"
[cli]
user_file = "user.log"
"#,
        )
        .unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = user_file_stub(FINAL_SUMMARY, "matching output\n");
        let rust_stub = build_stub_binary(temp.path(), "rust_user_mismatch", &rust_stub_source);
        let legacy_stub_source = user_file_stub(FINAL_SUMMARY, "legacy different\n");
        let legacy_stub =
            build_stub_binary(temp.path(), "legacy_user_mismatch", &legacy_stub_source);

        let config = HarnessConfig {
            legacy_launcher: legacy_stub,
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        run_with_config(config).expect("parity harness run");

        let summary_path = output_dir.join("summary.json");
        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 1);
        let report = &summary.specs[0];
        assert_eq!(report.status, ParityStatus::Mismatch);
        let note = report.notes.as_deref().unwrap_or("");
        assert!(
            note.contains("user output differs"),
            "expected user output diff note, got: {note}"
        );
    }

    #[test]
    fn fails_when_metrics_log_missing() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();
        fs::write(specs_root.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = missing_metrics_stub_source(FINAL_SUMMARY);
        let rust_stub =
            build_stub_binary(temp.path(), "rust_stub_missing_metrics", &rust_stub_source);

        let config = HarnessConfig {
            legacy_launcher: rust_stub.clone(),
            specs_root: specs_root.clone(),
            filter: None,
            output_dir,
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        let error = run_with_config(config).expect_err("metrics validation should fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("run metrics"),
            "error should mention run metrics validation, got: {message}"
        );
    }

    #[test]
    fn fails_when_progress_events_missing() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();
        fs::write(specs_root.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = missing_progress_stub_source(FINAL_SUMMARY);
        let rust_stub =
            build_stub_binary(temp.path(), "rust_stub_missing_progress", &rust_stub_source);

        let config = HarnessConfig {
            legacy_launcher: rust_stub.clone(),
            specs_root: specs_root.clone(),
            filter: None,
            output_dir,
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        let error = run_with_config(config).expect_err("missing progress should fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("progress"),
            "error should mention progress validation, got: {message}"
        );
    }

    #[test]
    fn fails_when_progress_thresholds_exceeded() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();
        fs::write(specs_root.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = progress_violation_stub_source(FINAL_SUMMARY);
        let rust_stub = build_stub_binary(
            temp.path(),
            "rust_stub_progress_violation",
            &rust_stub_source,
        );

        let config = HarnessConfig {
            legacy_launcher: rust_stub.clone(),
            specs_root: specs_root.clone(),
            filter: None,
            output_dir,
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        let error = run_with_config(config).expect_err("progress violation should fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("progress thresholds violated"),
            "error should mention thresholds violation, got: {message}"
        );
    }

    #[test]
    fn fails_when_metrics_log_invalid() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();
        fs::write(specs_root.join("SpecA.tla"), "---- MODULE SpecA ----").unwrap();

        let output_dir = temp.path().join("artifacts");
        let rust_stub_source = invalid_metrics_stub_source();
        let rust_stub =
            build_stub_binary(temp.path(), "rust_stub_invalid_metrics", &rust_stub_source);

        let config = HarnessConfig {
            legacy_launcher: rust_stub.clone(),
            specs_root: specs_root.clone(),
            filter: None,
            output_dir,
            rust_binary: rust_stub,
            workers: None,
            golden_cache: None,
        };

        let error = run_with_config(config).expect_err("invalid metrics should fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("run metrics"),
            "error should mention run metrics validation, got: {message}"
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
