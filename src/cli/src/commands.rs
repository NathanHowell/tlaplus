use std::fmt;
use std::io::{self, IsTerminal};
use std::num::ParseIntError;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{
    builder::PossibleValue, value_parser, ArgAction, Args, Parser, Subcommand, ValueEnum, ValueHint,
};
use serde::{Deserialize, Serialize};

/// Root CLI entry point defining the `tlc` command surface.
#[derive(Debug, Parser)]
#[command(
    name = "tlc",
    version,
    about = "Rust-native TLC command-line interface",
    propagate_version = true,
    subcommand_required = true,
    arg_required_else_help = true,
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// High-level commands supported by the TLC CLI.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Execute a model check from scratch using the provided specification.
    Run(RunCommand),
    /// Resume an exploration from a persisted checkpoint manifest.
    Resume(ResumeCommand),
}

/// Shared output-related options used by both `run` and `resume`.
#[derive(Debug, Clone, Args)]
pub struct OutputOptions {
    /// Control terminal progress rendering (`tty`) or machine-readable NDJSON stream (`ndjson`).
    #[arg(
        long = "progress",
        value_enum,
        default_value_t = ProgressMode::Tty,
        value_name = "MODE"
    )]
    pub progress: ProgressMode,

    /// Configure telemetry sink; remote OTLP requires explicit opt-in.
    #[arg(
        long = "telemetry",
        value_enum,
        default_value_t = TelemetryMode::Local,
        value_name = "MODE"
    )]
    pub telemetry: TelemetryMode,

    /// OTLP collector endpoint when `--telemetry=otlp` (e.g. https://collector:4317).
    #[arg(long = "otlp-endpoint", value_name = "URL", requires = "telemetry")]
    pub otlp_endpoint: Option<String>,

    /// Disable ANSI colors in TTY progress output (useful for monochrome terminals).
    #[arg(long = "no-color", action = ArgAction::SetTrue)]
    pub no_color: bool,
}

/// CLI arguments for `tlc run`.
#[derive(Debug, Clone, Args)]
pub struct RunCommand {
    /// Path to the primary TLA+ module (`Main.tla`).
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub spec: PathBuf,

    /// Path to the model configuration (`MC.cfg`).
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub config: PathBuf,

    /// Override worker count (`auto` uses logical cores - 1, minimum 1).
    #[arg(
        long = "workers",
        default_value_t = WorkerCount::Auto,
        value_parser = value_parser!(WorkerCount),
        value_name = "COUNT",
        env = "TLC_WORKERS"
    )]
    pub workers: WorkerCount,

    /// Upper bound for in-memory state cache (accepts suffixes like `4GiB`).
    #[arg(
        long = "memory-limit",
        value_parser = value_parser!(MemoryLimit),
        value_name = "BYTES",
        env = "TLC_MEMORY_LIMIT"
    )]
    pub memory_limit: Option<MemoryLimit>,

    /// Key/value spec parameter overrides (`--param Foo=Bar --param Baz=Quux`).
    #[arg(
        long = "param",
        action = ArgAction::Append,
        value_parser = value_parser!(ParameterOverride),
        value_name = "KEY=VALUE"
    )]
    pub parameters: Vec<ParameterOverride>,

    /// Directory for persisted checkpoints (defaults to repository runtime location).
    #[arg(long = "checkpoint-dir", value_hint = ValueHint::DirPath, value_name = "PATH")]
    pub checkpoint_dir: Option<PathBuf>,

    /// Time-based checkpoint interval in minutes (`0` disables periodic checkpoints).
    #[arg(long = "checkpoint-interval", value_name = "MINUTES")]
    pub checkpoint_interval: Option<u32>,

    /// Emit coverage statistics every N minutes in addition to final summary.
    #[arg(long = "coverage", value_name = "MINUTES")]
    pub coverage_interval: Option<u32>,

    /// Continue exploration after invariant violation (legacy `-continue`).
    #[arg(long = "continue-on-violation", action = ArgAction::SetTrue)]
    pub continue_on_violation: bool,

    /// Skip deadlock checks (`-deadlock` legacy flag).
    #[arg(long = "skip-deadlock", action = ArgAction::SetTrue)]
    pub skip_deadlock: bool,

    /// Remove legacy states directory before starting (legacy `-cleanup`).
    #[arg(long = "cleanup", action = ArgAction::SetTrue)]
    pub cleanup: bool,

    /// Suppress TLC warnings equivalent to legacy `-nowarning`.
    #[arg(long = "suppress-warnings", action = ArgAction::SetTrue)]
    pub suppress_warnings: bool,

    /// Continue printing only differing trace frames (legacy `-difftrace`).
    #[arg(long = "diff-trace", action = ArgAction::SetTrue)]
    pub diff_trace: bool,

    /// Dump counterexample trace in the requested format (legacy `-dumpTrace`).
    #[arg(long = "dump-trace", value_name = "FORMAT", value_parser = value_parser!(TraceDumpFormat))]
    pub dump_trace: Option<TraceDumpFormat>,

    /// Target file for `--dump-trace`; required when format provided.
    #[arg(long = "dump-trace-file", value_name = "PATH", requires = "dump_trace")]
    pub dump_trace_file: Option<PathBuf>,

    /// Post-condition operators to evaluate after run completion (legacy `-postCondition`).
    #[arg(
        long = "post-condition",
        value_name = "MODULE!OP",
        action = ArgAction::Append
    )]
    pub post_conditions: Vec<String>,

    #[command(flatten)]
    pub output: OutputOptions,
}

/// CLI arguments for `tlc resume`.
#[derive(Debug, Clone, Args)]
pub struct ResumeCommand {
    /// Path to checkpoint manifest produced by `tlc run`.
    #[arg(long, value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub checkpoint: PathBuf,

    /// Override worker count when resuming (same semantics as `tlc run`).
    #[arg(
        long = "workers",
        default_value_t = WorkerCount::Auto,
        value_parser = value_parser!(WorkerCount),
        value_name = "COUNT",
        env = "TLC_WORKERS"
    )]
    pub workers: WorkerCount,

    /// Skip spec hash compatibility check (legacy `-ignoreStateHash`).
    #[arg(long = "ignore-hash", action = ArgAction::SetTrue)]
    pub ignore_hash: bool,

    #[command(flatten)]
    pub output: OutputOptions,
}

/// Acceptable values for `--progress`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum ProgressMode {
    Tty,
    Ndjson,
}

/// Concrete progress renderer selection after accounting for environment constraints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressRenderer {
    Tty { use_color: bool },
    Ndjson,
}

impl ProgressRenderer {
    pub fn mode(self) -> ProgressMode {
        match self {
            ProgressRenderer::Tty { .. } => ProgressMode::Tty,
            ProgressRenderer::Ndjson => ProgressMode::Ndjson,
        }
    }

    /// Report whether ANSI styling should be used for TTY progress output.
    pub fn tty_use_color(self) -> Option<bool> {
        match self {
            ProgressRenderer::Tty { use_color } => Some(use_color),
            ProgressRenderer::Ndjson => None,
        }
    }
}

impl OutputOptions {
    /// Resolve the requested progress mode into a renderer compatible with the current stdout.
    pub fn resolve_progress(&self) -> ProgressRenderer {
        self.resolve_progress_internal(io::stdout().is_terminal())
    }

    /// Resolve the progress mode into a `ProgressMode` suitable for configuration structs.
    pub fn resolve_progress_mode(&self) -> ProgressMode {
        self.resolve_progress().mode()
    }

    fn resolve_progress_internal(&self, stdout_is_terminal: bool) -> ProgressRenderer {
        let use_color = !self.no_color;
        match self.progress {
            ProgressMode::Ndjson => ProgressRenderer::Ndjson,
            ProgressMode::Tty if stdout_is_terminal => ProgressRenderer::Tty { use_color },
            ProgressMode::Tty => ProgressRenderer::Ndjson,
        }
    }

    #[cfg(test)]
    pub(crate) fn resolve_progress_for(&self, stdout_is_terminal: bool) -> ProgressRenderer {
        self.resolve_progress_internal(stdout_is_terminal)
    }

    /// Report whether TTY progress should retain ANSI colors.
    pub fn tty_color_enabled(&self) -> bool {
        !self.no_color
    }
}

/// Acceptable values for `--telemetry`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum TelemetryMode {
    Local,
    Json,
    Otlp,
}

/// Representation of worker count flag values (`auto` or explicit number).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCount {
    Auto,
    Fixed(u16),
}

impl Default for WorkerCount {
    fn default() -> Self {
        WorkerCount::Auto
    }
}

impl FromStr for WorkerCount {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case("auto") {
            return Ok(Self::Auto);
        }

        let parsed: u16 = value
            .parse()
            .map_err(|err: ParseIntError| format!("invalid worker count `{value}`: {err}"))?;
        if parsed == 0 {
            return Err("worker count must be at least 1".to_string());
        }
        Ok(Self::Fixed(parsed))
    }
}

impl fmt::Display for WorkerCount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerCount::Auto => write!(f, "auto"),
            WorkerCount::Fixed(value) => write!(f, "{value}"),
        }
    }
}

/// Memory limit wrapper storing parsed bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryLimit(pub u64);

impl FromStr for MemoryLimit {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let cleaned = input.trim();
        if cleaned.is_empty() {
            return Err("memory limit cannot be empty".to_string());
        }

        let normalized = cleaned.replace('_', "");
        let lower = normalized.to_ascii_lowercase();

        const KIB: u64 = 1024;
        const MIB: u64 = KIB * 1024;
        const GIB: u64 = MIB * 1024;
        const TIB: u64 = GIB * 1024;

        let (number, multiplier) = if let Some(value) = lower.strip_suffix("gib") {
            (value, GIB)
        } else if let Some(value) = lower.strip_suffix("mib") {
            (value, MIB)
        } else if let Some(value) = lower.strip_suffix("kib") {
            (value, KIB)
        } else if let Some(value) = lower.strip_suffix("tib") {
            (value, TIB)
        } else if let Some(value) = lower.strip_suffix('b') {
            (value, 1)
        } else {
            (lower.as_str(), 1)
        };

        let bytes: u64 = number
            .parse()
            .map_err(|err: ParseIntError| format!("invalid memory limit `{input}`: {err}"))?;
        bytes
            .checked_mul(multiplier)
            .map(MemoryLimit)
            .ok_or_else(|| format!("memory limit `{input}` exceeds u64 range"))
    }
}

/// Parsed key/value overrides supplied via `--param`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParameterOverride {
    pub key: String,
    pub value: String,
}

impl FromStr for ParameterOverride {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (key, value) = raw
            .split_once('=')
            .ok_or_else(|| format!("expected KEY=VALUE format, got `{raw}`"))?;
        if key.trim().is_empty() {
            return Err("parameter key cannot be empty".into());
        }
        Ok(Self {
            key: key.trim().to_string(),
            value: value.to_string(),
        })
    }
}

/// Formats supported by `--dump-trace`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TraceDumpFormat {
    HumanReadable,
    Action,
    Dot,
    Tla,
}

impl FromStr for TraceDumpFormat {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input.to_ascii_lowercase().as_str() {
            "human" | "human-readable" => Ok(Self::HumanReadable),
            "action" => Ok(Self::Action),
            "dot" => Ok(Self::Dot),
            "tla" => Ok(Self::Tla),
            other => Err(format!(
                "unsupported dump trace format `{other}` (expected human|action|dot|tla)"
            )),
        }
    }
}

impl ValueEnum for TraceDumpFormat {
    fn value_variants<'a>() -> &'a [Self] {
        const VARIANTS: &[TraceDumpFormat; 4] = &[
            TraceDumpFormat::HumanReadable,
            TraceDumpFormat::Action,
            TraceDumpFormat::Dot,
            TraceDumpFormat::Tla,
        ];
        VARIANTS
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(match self {
            TraceDumpFormat::HumanReadable => PossibleValue::new("human"),
            TraceDumpFormat::Action => PossibleValue::new("action"),
            TraceDumpFormat::Dot => PossibleValue::new("dot"),
            TraceDumpFormat::Tla => PossibleValue::new("tla"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_options(progress: ProgressMode) -> OutputOptions {
        OutputOptions {
            progress,
            telemetry: TelemetryMode::Local,
            otlp_endpoint: None,
            no_color: false,
        }
    }

    #[test]
    fn tty_progress_selected_when_terminal() {
        let options = default_options(ProgressMode::Tty);
        assert_eq!(
            options.resolve_progress_for(true),
            ProgressRenderer::Tty { use_color: true }
        );
    }

    #[test]
    fn tty_progress_falls_back_to_ndjson_without_terminal() {
        let options = default_options(ProgressMode::Tty);
        assert_eq!(
            options.resolve_progress_for(false),
            ProgressRenderer::Ndjson
        );
    }

    #[test]
    fn explicit_ndjson_request_remains_ndjson() {
        let options = default_options(ProgressMode::Ndjson);
        assert_eq!(options.resolve_progress_for(true), ProgressRenderer::Ndjson);
        assert_eq!(
            options.resolve_progress_for(false),
            ProgressRenderer::Ndjson
        );
    }

    #[test]
    fn no_color_flag_disables_tty_colors_when_terminal() {
        let mut options = default_options(ProgressMode::Tty);
        options.no_color = true;

        let renderer = options.resolve_progress_for(true);
        assert_eq!(renderer, ProgressRenderer::Tty { use_color: false });
        assert_eq!(options.tty_color_enabled(), false);
        assert_eq!(renderer.tty_use_color(), Some(false));
    }

    #[test]
    fn worker_count_parses_and_displays_defaults() {
        assert_eq!(WorkerCount::default(), WorkerCount::Auto);
        assert_eq!("auto", WorkerCount::Auto.to_string());
        assert_eq!(
            WorkerCount::from_str("auto").expect("parse auto"),
            WorkerCount::Auto
        );

        assert_eq!(
            WorkerCount::from_str("8").expect("parse fixed"),
            WorkerCount::Fixed(8)
        );
        assert_eq!("8", WorkerCount::Fixed(8).to_string());
    }
}
