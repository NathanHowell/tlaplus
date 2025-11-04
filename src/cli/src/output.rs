//! Formatting utilities for TLC CLI diagnostics and legacy-compatible summaries.

use std::{
    fmt,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};

use chrono::{DateTime, FixedOffset};

use thiserror::Error;
use tlc_util::RunStatus;
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputConfig {
    pub debug: bool,
    pub suppress_warnings: bool,
    pub terse: bool,
    pub user_output: Option<PathBuf>,
    pub tool_mode: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            debug: false,
            suppress_warnings: false,
            terse: false,
            user_output: None,
            tool_mode: false,
        }
    }
}

pub struct OutputBuilder {
    config: OutputConfig,
    stdout: StreamTarget,
    stderr: StreamTarget,
}

impl OutputBuilder {
    pub fn new() -> Self {
        Self {
            config: OutputConfig::default(),
            stdout: StreamTarget::Stdout,
            stderr: StreamTarget::Stderr,
        }
    }

    pub fn config(mut self, config: OutputConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_stdout(mut self, target: StreamTarget) -> Self {
        self.stdout = target;
        self
    }

    pub fn with_stderr(mut self, target: StreamTarget) -> Self {
        self.stderr = target;
        self
    }

    pub fn install(self) -> Result<(), OutputError> {
        install_internal(self.config, self.stdout, self.stderr)
    }
}

#[derive(Debug, Clone)]
pub enum StreamTarget {
    Stdout,
    Stderr,
    Buffer(Arc<Mutex<Vec<u8>>>),
}

impl StreamTarget {
    pub fn buffer() -> (Self, Arc<Mutex<Vec<u8>>>) {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        (StreamTarget::Buffer(Arc::clone(&buffer)), buffer)
    }

    fn write(&self, message: &str) -> io::Result<()> {
        self.write_bytes(message.as_bytes())
    }

    fn write_line(&self, message: &str) -> io::Result<()> {
        self.write_bytes(message.as_bytes())?;
        self.write_bytes(b"\n")?;
        self.flush()
    }

    fn write_bytes(&self, bytes: &[u8]) -> io::Result<()> {
        match self {
            StreamTarget::Stdout => {
                let mut stdout = io::stdout();
                stdout.write_all(bytes)
            }
            StreamTarget::Stderr => {
                let mut stderr = io::stderr();
                stderr.write_all(bytes)
            }
            StreamTarget::Buffer(buffer) => {
                let mut guard = buffer.lock().expect("buffer poisoned");
                guard.extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    fn flush(&self) -> io::Result<()> {
        match self {
            StreamTarget::Stdout => io::stdout().flush(),
            StreamTarget::Stderr => io::stderr().flush(),
            StreamTarget::Buffer(_) => Ok(()),
        }
    }
}

#[derive(Debug)]
struct UserSink {
    path: PathBuf,
    file: File,
}

impl UserSink {
    fn write(&mut self, message: &str, append_newline: bool) -> io::Result<()> {
        self.file.write_all(message.as_bytes())?;
        if append_newline {
            self.file.write_all(b"\n")?;
        }
        self.file.flush()
    }
}

#[derive(Debug)]
struct OutputState {
    config: OutputConfig,
    stdout: StreamTarget,
    stderr: StreamTarget,
    user_sink: Option<UserSink>,
}

impl Default for OutputState {
    fn default() -> Self {
        Self {
            config: OutputConfig::default(),
            stdout: StreamTarget::Stdout,
            stderr: StreamTarget::Stderr,
            user_sink: None,
        }
    }
}

static OUTPUT_STATE: OnceLock<Mutex<OutputState>> = OnceLock::new();

fn shared_state() -> &'static Mutex<OutputState> {
    OUTPUT_STATE.get_or_init(|| Mutex::new(OutputState::default()))
}

fn state_guard() -> MutexGuard<'static, OutputState> {
    match shared_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("failed to open user output file at '{path}': {source}")]
    UserFileOpen {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write output: {0}")]
    Io(#[from] io::Error),
}

pub fn install(config: OutputConfig) -> Result<(), OutputError> {
    install_internal(config, StreamTarget::Stdout, StreamTarget::Stderr)
}

pub fn install_with_streams(
    config: OutputConfig,
    stdout: StreamTarget,
    stderr: StreamTarget,
) -> Result<(), OutputError> {
    install_internal(config, stdout, stderr)
}

fn install_internal(
    config: OutputConfig,
    stdout: StreamTarget,
    stderr: StreamTarget,
) -> Result<(), OutputError> {
    let mut guard = state_guard();
    guard.stdout = stdout;
    guard.stderr = stderr;
    guard.config = config.clone();
    guard.user_sink = match config.user_output {
        Some(ref path) => {
            let file = File::create(path).map_err(|source| OutputError::UserFileOpen {
                path: path.clone(),
                source,
            })?;
            Some(UserSink {
                path: path.clone(),
                file,
            })
        }
        None => None,
    };
    Ok(())
}

pub fn emit(message: &str) -> Result<(), OutputError> {
    let guard = state_guard();
    guard.stdout.write(message)?;
    Ok(())
}

pub fn emit_warning(message: &str) -> Result<(), OutputError> {
    let guard = state_guard();
    if guard.config.suppress_warnings {
        return Ok(());
    }
    guard.stdout.write(message)?;
    Ok(())
}

pub fn emit_error(message: &str) -> Result<(), OutputError> {
    let guard = state_guard();
    guard.stderr.write(message)?;
    Ok(())
}

pub fn emit_debug(message: &str) -> Result<(), OutputError> {
    let guard = state_guard();
    if !guard.config.debug {
        return Ok(());
    }
    guard.stdout.write(message)?;
    Ok(())
}

pub fn emit_user(message: &str) -> Result<(), OutputError> {
    write_user(message, false)
}

pub fn emit_user_line(message: &str) -> Result<(), OutputError> {
    write_user(message, true)
}

fn write_user(message: &str, append_newline: bool) -> Result<(), OutputError> {
    let mut guard = state_guard();
    if let Some(sink) = guard.user_sink.as_mut() {
        sink.write(message, append_newline)?;
    } else if append_newline {
        guard.stdout.write_line(message)?;
    } else {
        guard.stdout.write(message)?;
    }
    Ok(())
}

pub fn expand_values() -> bool {
    !state_guard().config.terse
}

pub fn debug_enabled() -> bool {
    state_guard().config.debug
}

const TOOL_DELIMITER: &str = "@!@!@";
const TOOL_START: &str = "STARTMSG ";
const TOOL_END: &str = "ENDMSG ";
const TOOL_COLON: &str = ":";
const TOOL_SPACE: &str = " ";
const TOOL_NEWLINE: &str = "\n";

/// Severity levels mirroring the legacy TLC message catalog (`MP`).
#[allow(dead_code)] // Future diagnostics will exercise all severity classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageClass {
    None = 0,
    Error = 1,
    TlcBug = 2,
    Warning = 3,
    State = 4,
}

impl fmt::Display for MessageClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", *self as u8)
    }
}

/// Known diagnostic codes used by the CLI formatter.
#[allow(dead_code)] // Additional message codes will be added alongside parity tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageCode {
    Success,
    Stats,
    StatsDfid,
    SearchDepth,
    Finished,
    Custom(u32),
}

impl MessageCode {
    fn value(self) -> u32 {
        match self {
            MessageCode::Success => 2193,
            MessageCode::Stats => 2199,
            MessageCode::StatsDfid => 2204,
            MessageCode::SearchDepth => 2194,
            MessageCode::Finished => 2186,
            MessageCode::Custom(value) => value,
        }
    }
}

/// Structured diagnostic produced by the formatter.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: MessageCode,
    pub class: MessageClass,
    pub parameters: Vec<String>,
}

impl Diagnostic {
    /// Success banner mirroring `EC.TLC_SUCCESS`.
    pub fn success(estimates: FingerprintEstimates) -> Self {
        let FingerprintEstimates { optimistic, actual } = estimates;
        let mut parameters = Vec::with_capacity(2);
        parameters.push(optimistic);
        if let Some(actual) = actual {
            parameters.push(actual);
        }
        Diagnostic {
            code: MessageCode::Success,
            class: MessageClass::None,
            parameters,
        }
    }

    /// Statistics summary (`EC.TLC_STATS` or `EC.TLC_STATS_DFID`).
    pub fn stats(states: u128, distinct: u128, queue: Option<u128>) -> Self {
        let formatter = NumberFormatter;
        let mut parameters = vec![formatter.format(states), formatter.format(distinct)];
        let code = if let Some(queue) = queue {
            parameters.push(formatter.format(queue));
            MessageCode::Stats
        } else {
            MessageCode::StatsDfid
        };
        Diagnostic {
            code,
            class: MessageClass::None,
            parameters,
        }
    }

    /// Search depth summary (`EC.TLC_SEARCH_DEPTH`).
    pub fn search_depth(depth: u64) -> Self {
        Diagnostic {
            code: MessageCode::SearchDepth,
            class: MessageClass::None,
            parameters: vec![NumberFormatter.format(depth.into())],
        }
    }

    /// Final runtime banner (`EC.TLC_FINISHED`).
    pub fn finished(runtime: String, timestamp: String) -> Self {
        Diagnostic {
            code: MessageCode::Finished,
            class: MessageClass::None,
            parameters: vec![runtime, timestamp],
        }
    }

    /// Construct a diagnostic from a raw legacy message code.
    pub fn custom<P, S>(code: u32, class: MessageClass, parameters: P) -> Self
    where
        P: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Diagnostic {
            code: MessageCode::Custom(code),
            class,
            parameters: parameters.into_iter().map(Into::into).collect(),
        }
    }
}

/// Probability estimates for fingerprint collisions printed in the success banner.
#[derive(Debug, Clone)]
pub struct FingerprintEstimates {
    pub optimistic: String,
    pub actual: Option<String>,
}

impl FingerprintEstimates {
    pub fn new<O: Into<String>>(optimistic: O, actual: Option<String>) -> Self {
        Self {
            optimistic: optimistic.into(),
            actual,
        }
    }
}

/// Summary inputs describing a completed TLC run.
#[derive(Debug, Clone)]
pub struct RunSummary {
    pub status: RunStatus,
    pub states_generated: u128,
    pub distinct_states: u128,
    pub states_left_on_queue: Option<u128>,
    pub search_depth: u64,
    pub fingerprint: Option<FingerprintEstimates>,
    pub runtime: Duration,
    pub finished_at: DateTime<FixedOffset>,
}

impl RunSummary {
    pub fn new(
        status: RunStatus,
        states_generated: u128,
        distinct_states: u128,
        states_left_on_queue: Option<u128>,
        search_depth: u64,
        fingerprint: Option<FingerprintEstimates>,
        runtime: Duration,
        finished_at: DateTime<FixedOffset>,
    ) -> Self {
        Self {
            status,
            states_generated,
            distinct_states,
            states_left_on_queue,
            search_depth,
            fingerprint,
            runtime,
            finished_at,
        }
    }
}

/// Metadata surfaced when checkpoints are persisted or validated.
#[derive(Debug, Clone)]
pub struct CheckpointManifestInfo<'a> {
    pub checkpoint_path: &'a Path,
    pub manifest_path: &'a Path,
    pub manifest_version: u32,
    pub lineage: &'a [Ulid],
    pub checkpoint_id: Option<Ulid>,
}

impl<'a> CheckpointManifestInfo<'a> {
    fn formatted_lineage(&self) -> Option<String> {
        if self.lineage.is_empty() {
            return None;
        }
        let mut chain = self
            .lineage
            .iter()
            .map(Ulid::to_string)
            .collect::<Vec<_>>()
            .join(" \u{2192} ");
        if self.lineage.len() == 1 {
            chain = self.lineage[0].to_string();
        }
        Some(chain)
    }

    fn checkpoint_display(&self) -> String {
        format!("{}", self.checkpoint_path.display())
    }

    fn manifest_display(&self) -> String {
        format!("{}", self.manifest_path.display())
    }

    fn resume_command(&self) -> String {
        format!("tlc resume --checkpoint {:?}", self.checkpoint_path)
    }
}

/// Format guidance when a run stops before completion but checkpoints were written.
pub fn format_checkpoint_interruption(info: &CheckpointManifestInfo<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Exploration interrupted before completion.".to_string());

    let mut checkpoint_line = format!("Checkpoint snapshot: {}", info.checkpoint_display());
    if let Some(id) = info.checkpoint_id {
        checkpoint_line.push_str(&format!(" (checkpoint ID {id})"));
    }
    lines.push(checkpoint_line);

    lines.push(format!(
        "Checkpoint manifest v{}: {}",
        info.manifest_version,
        info.manifest_display()
    ));

    if let Some(lineage) = info.formatted_lineage() {
        if info.lineage.len() == 1 {
            lines.push(format!("Run ID: {lineage}"));
        } else {
            lines.push(format!("Resume lineage: {lineage}"));
        }
    }

    lines.push(format!("Resume with: {}", info.resume_command()));
    lines
}

/// Format guidance when a resume attempt fails due to spec hash mismatch.
pub fn format_checkpoint_mismatch(
    info: &CheckpointManifestInfo<'_>,
    expected_spec_hash: &str,
    recorded_spec_hash: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Cannot resume from checkpoint: specification hash mismatch.".to_string());

    let mut checkpoint_line = format!("Checkpoint snapshot: {}", info.checkpoint_display());
    if let Some(id) = info.checkpoint_id {
        checkpoint_line.push_str(&format!(" (checkpoint ID {id})"));
    }
    lines.push(checkpoint_line);

    lines.push(format!(
        "Checkpoint manifest v{}: {}",
        info.manifest_version,
        info.manifest_display()
    ));
    lines.push(format!("Recorded spec hash: {recorded_spec_hash}"));
    lines.push(format!("Current spec hash: {expected_spec_hash}"));

    if let Some(lineage) = info.formatted_lineage() {
        if info.lineage.len() == 1 {
            lines.push(format!("Run ID: {lineage}"));
        } else {
            lines.push(format!("Resume lineage: {lineage}"));
        }
    }

    lines.push(
        "Regenerate the checkpoint by rerunning `tlc run` with matching modules and configuration."
            .to_string(),
    );
    lines.push(format!(
        "To bypass this safety check (not recommended), rerun with: {} --ignore-hash",
        info.resume_command()
    ));
    lines
}

/// Errors produced while rendering diagnostics.
#[derive(Debug, Error)]
pub enum FormatterError {
    #[error("unknown diagnostic code {0}")]
    UnknownCode(u32),
    #[error("diagnostic code {code} expected parameter at index {index}, but only {provided} were supplied")]
    MissingParameter {
        code: u32,
        index: usize,
        provided: usize,
    },
    #[error("success summary requires fingerprint probability estimates")]
    MissingFingerprintEstimates,
}

/// Formats diagnostics for either human-readable or tool (IDE) consumers.
#[derive(Debug, Clone)]
pub struct Formatter {
    tool_mode: bool,
}

impl Default for Formatter {
    fn default() -> Self {
        Self { tool_mode: false }
    }
}

impl Formatter {
    /// Create a formatter with optional tool-mode wrapping.
    pub fn new(tool_mode: bool) -> Self {
        Self { tool_mode }
    }

    /// Render a diagnostic into the legacy string representation.
    pub fn format(&self, diagnostic: &Diagnostic) -> Result<String, FormatterError> {
        let body = render_body(diagnostic.code, diagnostic.class, &diagnostic.parameters)?;
        Ok(if self.tool_mode {
            wrap_tool_message(diagnostic.class, diagnostic.code.value(), &body)
        } else {
            format_non_tool_message(diagnostic.class, &body)
        })
    }

    /// Emit the legacy success banner and summary statistics for a completed run.
    pub fn format_summary(&self, summary: &RunSummary) -> Result<Vec<String>, FormatterError> {
        let mut lines = Vec::new();

        if summary.status == RunStatus::Completed {
            let estimates = summary
                .fingerprint
                .clone()
                .ok_or(FormatterError::MissingFingerprintEstimates)?;
            let success = Diagnostic::success(estimates);
            lines.push(self.format(&success)?);
        }

        let stats = Diagnostic::stats(
            summary.states_generated,
            summary.distinct_states,
            summary.states_left_on_queue,
        );
        lines.push(self.format(&stats)?);

        let depth = Diagnostic::search_depth(summary.search_depth);
        lines.push(self.format(&depth)?);

        let runtime = format_runtime(summary.runtime, self.tool_mode);
        let timestamp = format_timestamp(&summary.finished_at);
        let finished = Diagnostic::finished(runtime, timestamp);
        lines.push(self.format(&finished)?);

        Ok(lines)
    }
}

fn render_body(
    code: MessageCode,
    class: MessageClass,
    params: &[String],
) -> Result<String, FormatterError> {
    match code {
        MessageCode::Success => render_success(params),
        MessageCode::Stats => render_stats(params, true),
        MessageCode::StatsDfid => render_stats(params, false),
        MessageCode::SearchDepth => render_search_depth(params),
        MessageCode::Finished => render_finished(params),
        MessageCode::Custom(value) => message_catalog::render(value, class, params),
    }
}

fn render_success(params: &[String]) -> Result<String, FormatterError> {
    ensure_parameter(params, 0, MessageCode::Success)?;
    let mut message = String::from(
        "Model checking completed. No error has been found.\n  Estimates of the probability that TLC did not check all reachable states\n  because two distinct states had the same fingerprint:\n  calculated (optimistic):  ",
    );
    message.push_str(&params[0]);
    if let Some(actual) = params.get(1) {
        message.push('\n');
        message.push_str("  based on the actual fingerprints:  ");
        message.push_str(actual);
    }
    Ok(message)
}

fn render_stats(params: &[String], include_queue: bool) -> Result<String, FormatterError> {
    ensure_parameter(
        params,
        1,
        if include_queue {
            MessageCode::Stats
        } else {
            MessageCode::StatsDfid
        },
    )?;

    if include_queue {
        ensure_parameter(params, 2, MessageCode::Stats)?;
        Ok(format!(
            "{} states generated, {} distinct states found, {} states left on queue.",
            params[0], params[1], params[2]
        ))
    } else {
        Ok(format!(
            "{} states generated, {} distinct states found.",
            params[0], params[1]
        ))
    }
}

fn render_search_depth(params: &[String]) -> Result<String, FormatterError> {
    ensure_parameter(params, 0, MessageCode::SearchDepth)?;
    Ok(format!(
        "The depth of the complete state graph search is {}.",
        params[0]
    ))
}

fn render_finished(params: &[String]) -> Result<String, FormatterError> {
    ensure_parameter(params, 0, MessageCode::Finished)?;
    ensure_parameter(params, 1, MessageCode::Finished)?;
    Ok(format!("Finished in {} at ({})", params[0], params[1]))
}

mod message_catalog {
    use super::{ensure_parameter, FormatterError, MessageClass, MessageCode};

    pub(super) fn render(
        code: u32,
        class: MessageClass,
        params: &[String],
    ) -> Result<String, FormatterError> {
        match code {
            2104 => assumption_false(class, params),
            2114 => deadlock_reached(class, params),
            2146 => invariant_not_state_predicate(class, params),
            2256 => no_state_satisfies_constraint(class, params),
            2284 => liveness_constraints_warning(class, params),
            2285 => feature_incompatible(class, params),
            9296 => no_specification_but_properties(class, params),
            9297 => no_fairness_for_properties(class, params),
            _ => Err(FormatterError::UnknownCode(code)),
        }
    }

    fn ensure_class(
        expected: MessageClass,
        actual: MessageClass,
        code: u32,
    ) -> Result<(), FormatterError> {
        if expected == actual {
            Ok(())
        } else {
            Err(FormatterError::UnknownCode(code))
        }
    }

    fn assumption_false(class: MessageClass, params: &[String]) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Error, class, 2104)?;
        ensure_parameter(params, 0, MessageCode::Custom(2104))?;
        Ok(format!("Assumption {} is false.", params[0]))
    }

    fn deadlock_reached(class: MessageClass, params: &[String]) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Error, class, 2114)?;
        if !params.is_empty() {
            return Err(FormatterError::MissingParameter {
                code: 2114,
                index: 0,
                provided: params.len(),
            });
        }
        Ok("Deadlock reached.".to_string())
    }

    fn invariant_not_state_predicate(
        class: MessageClass,
        params: &[String],
    ) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Error, class, 2146)?;
        ensure_parameter(params, 0, MessageCode::Custom(2146))?;
        let mut message = format!(
            "The invariant {} is not a state predicate (one with no primes or temporal operators).",
            params[0]
        );

        if params.len() > 1 {
            message.push_str(
                "\nNote that a bug can cause TLC to incorrectly report this error.\n\
                 If you believe your TLA+ or PlusCal specification to be correct,\n\
                 please check if this bug described in LevelNode.java starting at line 590ff affects you.",
            );
        }

        Ok(message)
    }

    fn no_state_satisfies_constraint(
        class: MessageClass,
        params: &[String],
    ) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Error, class, 2256)?;
        if !params.is_empty() {
            return Err(FormatterError::MissingParameter {
                code: 2256,
                index: 0,
                provided: params.len(),
            });
        }
        Ok(
            "There is no state satisfying the initial state predicate and the state-constraint(s)."
                .to_string(),
        )
    }

    fn liveness_constraints_warning(
        class: MessageClass,
        params: &[String],
    ) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Warning, class, 2284)?;
        if !params.is_empty() {
            return Err(FormatterError::MissingParameter {
                code: 2284,
                index: 0,
                provided: params.len(),
            });
        }
        Ok("Declaring state or action constraints during liveness checking is dangerous: Please read section 14.3.5 on page 247 of Specifying Systems (https://lamport.azurewebsites.net/tla/book.html) and optionally the discussion at https://discuss.tlapl.us/msg00994.html for more details.".to_string())
    }

    fn feature_incompatible(
        class: MessageClass,
        params: &[String],
    ) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Warning, class, 2285)?;
        ensure_parameter(params, 0, MessageCode::Custom(2285))?;
        Ok(format!(
            "Feature {} is not supported in the current TLC mode.",
            params[0]
        ))
    }

    fn no_specification_but_properties(
        class: MessageClass,
        params: &[String],
    ) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Warning, class, 9296)?;
        if !params.is_empty() {
            return Err(FormatterError::MissingParameter {
                code: 9296,
                index: 0,
                provided: params.len(),
            });
        }
        Ok("Temporal properties (PROPERTY or PROPERTIES) are being verified without a behavior specification (SPECIFICATION). Only INIT and NEXT have been provided. This is likely to result in (trivial) counterexamples showing infinite stuttering following the initial state. It is recommended to use SPECIFICATION Spec, with Spec asserting a suitable fairness constraint (compare Chapter 8, page 87ff of Specifying Systems at https://lamport.azurewebsites.net/tla/book.html).".to_string())
    }

    fn no_fairness_for_properties(
        class: MessageClass,
        params: &[String],
    ) -> Result<String, FormatterError> {
        ensure_class(MessageClass::Warning, class, 9297)?;
        ensure_parameter(params, 0, MessageCode::Custom(9297))?;
        ensure_parameter(params, 1, MessageCode::Custom(9297))?;
        Ok(format!("Temporal properties (PROPERTY or PROPERTIES) are being verified without a fairness constraint conjoined to the behavior specification {} defined at {}. This may lead to trivial counterexamples in which the system exhibits infinite stuttering immediately after the initial state. To avoid this, it is recommended to conjoin a suitable fairness constraint to {} (compare Chapter 8, page 87ff of Specifying Systems at https://lamport.azurewebsites.net/tla/book.html).", params[0], params[1], params[0]))
    }
}

fn format_runtime(duration: Duration, tool_mode: bool) -> String {
    if tool_mode {
        return format!("{}ms", duration.as_millis());
    }

    const DAY: Duration = Duration::from_secs(24 * 60 * 60);
    const HOUR: Duration = Duration::from_secs(60 * 60);
    const MINUTE: Duration = Duration::from_secs(60);

    if duration >= DAY {
        let days = duration.as_secs() / DAY.as_secs();
        let hours = (duration.as_secs() % DAY.as_secs()) / HOUR.as_secs();
        return format!("{days}d {hours:02}h");
    }

    if duration >= HOUR {
        let hours = duration.as_secs() / HOUR.as_secs();
        let minutes = (duration.as_secs() % HOUR.as_secs()) / MINUTE.as_secs();
        return format!("{hours:02}h {minutes:02}min");
    }

    if duration >= MINUTE {
        let minutes = duration.as_secs() / MINUTE.as_secs();
        let seconds = duration.as_secs() % MINUTE.as_secs();
        return format!("{minutes:02}min {seconds:02}s");
    }

    let seconds = duration.as_secs();
    format!("{seconds:02}s")
}

fn format_timestamp(timestamp: &DateTime<FixedOffset>) -> String {
    timestamp.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn ensure_parameter(
    params: &[String],
    index: usize,
    code: MessageCode,
) -> Result<(), FormatterError> {
    if params.len() <= index {
        return Err(FormatterError::MissingParameter {
            code: code.value(),
            index,
            provided: params.len(),
        });
    }
    Ok(())
}

fn wrap_tool_message(class: MessageClass, code: u32, body: &str) -> String {
    [
        TOOL_DELIMITER,
        TOOL_START,
        &code.to_string(),
        TOOL_COLON,
        &class.to_string(),
        TOOL_SPACE,
        TOOL_DELIMITER,
        TOOL_NEWLINE,
        body,
        TOOL_NEWLINE,
        TOOL_DELIMITER,
        TOOL_END,
        &code.to_string(),
        TOOL_SPACE,
        TOOL_DELIMITER,
    ]
    .concat()
}

fn format_non_tool_message(class: MessageClass, body: &str) -> String {
    let prefix = match class {
        MessageClass::Error => "Error: ",
        MessageClass::TlcBug => "TLC Bug: ",
        MessageClass::Warning => "Warning: ",
        MessageClass::State => "State ",
        MessageClass::None => "",
    };
    format!("{prefix}{body}")
}

struct NumberFormatter;

impl NumberFormatter {
    fn format(&self, value: u128) -> String {
        let mut digits = value.to_string();
        let mut insert_position = digits.len() as isize - 3;
        while insert_position > 0 {
            digits.insert(insert_position as usize, ',');
            insert_position -= 3;
        }
        digits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tempfile::tempdir;
    use tlc_util::RunStatus;

    fn buffer_contents(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
        let guard = buffer.lock().expect("buffer lock");
        String::from_utf8_lossy(&guard[..]).to_string()
    }

    #[test]
    fn suppresses_warning_output_when_requested() {
        let (stdout_target, stdout_buffer) = StreamTarget::buffer();
        let (stderr_target, _) = StreamTarget::buffer();
        OutputBuilder::new()
            .config(OutputConfig {
                suppress_warnings: true,
                ..OutputConfig::default()
            })
            .with_stdout(stdout_target)
            .with_stderr(stderr_target)
            .install()
            .expect("install suppressed warnings");
        emit_warning("Warning: something unexpected")
            .expect("warning suppression should not error");
        assert_eq!(buffer_contents(&stdout_buffer), "");
    }

    #[test]
    fn debug_output_requires_flag() {
        let (stdout_target, stdout_buffer) = StreamTarget::buffer();
        let (stderr_target, _) = StreamTarget::buffer();
        OutputBuilder::new()
            .with_stdout(stdout_target)
            .with_stderr(stderr_target)
            .install()
            .expect("install default output");
        emit_debug("debug disabled").expect("emit debug without flag");
        assert_eq!(buffer_contents(&stdout_buffer), "");

        let (stdout_target, stdout_buffer) = StreamTarget::buffer();
        let (stderr_target, _) = StreamTarget::buffer();
        OutputBuilder::new()
            .config(OutputConfig {
                debug: true,
                ..OutputConfig::default()
            })
            .with_stdout(stdout_target)
            .with_stderr(stderr_target)
            .install()
            .expect("install debug output");
        emit_debug("debug enabled").expect("emit debug with flag");
        assert_eq!(buffer_contents(&stdout_buffer), "debug enabled");
    }

    #[test]
    fn user_output_redirects_to_file() {
        let temp_dir = tempdir().expect("temp dir");
        let user_log = temp_dir.path().join("user.log");
        OutputBuilder::new()
            .config(OutputConfig {
                user_output: Some(user_log.clone()),
                ..OutputConfig::default()
            })
            .install()
            .expect("install user output");
        emit_user_line("hello user").expect("write user output");
        let contents = std::fs::read_to_string(&user_log).expect("read user log");
        assert_eq!(contents, "hello user\n");
    }

    #[test]
    fn expand_values_follows_terse_flag() {
        OutputBuilder::new()
            .config(OutputConfig::default())
            .install()
            .expect("install default config");
        assert!(expand_values(), "default output should expand values");

        OutputBuilder::new()
            .config(OutputConfig {
                terse: true,
                ..OutputConfig::default()
            })
            .install()
            .expect("install terse config");
        assert!(
            !expand_values(),
            "terse config should report value expansion disabled"
        );
    }

    #[test]
    fn formats_success_banner_with_two_probabilities() {
        let formatter = Formatter::default();
        let finished_at = DateTime::parse_from_rfc3339("2025-11-02T03:25:45Z").expect("timestamp");
        let summary = RunSummary::new(
            RunStatus::Completed,
            2,
            1,
            Some(0),
            1,
            Some(FingerprintEstimates::new(
                "5.421010862427522E-20",
                Some("1.0842021724855044E-19".into()),
            )),
            Duration::from_secs(0),
            finished_at,
        );

        let lines = formatter.format_summary(&summary).expect("summary");
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            "Model checking completed. No error has been found.\n  Estimates of the probability that TLC did not check all reachable states\n  because two distinct states had the same fingerprint:\n  calculated (optimistic):  5.421010862427522E-20\n  based on the actual fingerprints:  1.0842021724855044E-19"
        );
        assert_eq!(
            lines[1],
            "2 states generated, 1 distinct states found, 0 states left on queue."
        );
        assert_eq!(
            lines[2],
            "The depth of the complete state graph search is 1."
        );
        assert_eq!(lines[3], "Finished in 00s at (2025-11-02 03:25:45)");
    }

    #[test]
    fn formats_success_banner_with_single_probability() {
        let formatter = Formatter::default();
        let finished_at = DateTime::parse_from_rfc3339("2025-11-02T04:00:00Z").expect("timestamp");
        let summary = RunSummary::new(
            RunStatus::Completed,
            12345,
            6789,
            None,
            42,
            Some(FingerprintEstimates::new("0.0", None)),
            Duration::from_secs(125),
            finished_at,
        );

        let lines = formatter.format_summary(&summary).expect("summary");
        assert_eq!(
            lines[0],
            "Model checking completed. No error has been found.\n  Estimates of the probability that TLC did not check all reachable states\n  because two distinct states had the same fingerprint:\n  calculated (optimistic):  0.0"
        );
        assert_eq!(
            lines[1],
            "12,345 states generated, 6,789 distinct states found."
        );
        assert_eq!(
            lines[2],
            "The depth of the complete state graph search is 42."
        );
        assert_eq!(lines[3], "Finished in 02min 05s at (2025-11-02 04:00:00)");
    }

    #[test]
    fn wraps_messages_in_tool_mode() {
        let formatter = Formatter::new(true);
        let stats = Diagnostic::stats(10, 5, Some(1));
        let rendered = formatter.format(&stats).expect("tool mode render");
        assert_eq!(
            rendered,
            "@!@!@STARTMSG 2199:0 @!@!@\n10 states generated, 5 distinct states found, 1 states left on queue.\n@!@!@ENDMSG 2199 @!@!@"
        );
    }

    #[test]
    fn finished_banner_uses_milliseconds_in_tool_mode() {
        let formatter = Formatter::new(true);
        let finished_at = DateTime::parse_from_rfc3339("2025-11-02T06:00:00Z").expect("timestamp");
        let summary = RunSummary::new(
            RunStatus::Failed,
            2048,
            1024,
            Some(2),
            64,
            None,
            Duration::from_millis(750),
            finished_at,
        );

        let lines = formatter.format_summary(&summary).expect("summary");
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "@!@!@STARTMSG 2199:0 @!@!@\n2,048 states generated, 1,024 distinct states found, 2 states left on queue.\n@!@!@ENDMSG 2199 @!@!@"
        );
        assert_eq!(
            lines[1],
            "@!@!@STARTMSG 2194:0 @!@!@\nThe depth of the complete state graph search is 64.\n@!@!@ENDMSG 2194 @!@!@"
        );
        assert_eq!(
            lines[2],
            "@!@!@STARTMSG 2186:0 @!@!@\nFinished in 750ms at (2025-11-02 06:00:00)\n@!@!@ENDMSG 2186 @!@!@"
        );
    }

    #[test]
    fn number_formatter_inserts_commas() {
        let formatter = NumberFormatter;
        assert_eq!(formatter.format(0), "0");
        assert_eq!(formatter.format(999), "999");
        assert_eq!(formatter.format(1_234), "1,234");
        assert_eq!(formatter.format(98_765_432_100), "98,765,432,100");
    }

    #[test]
    fn summary_requires_fingerprint_for_completed_runs() {
        let formatter = Formatter::default();
        let finished_at = DateTime::parse_from_rfc3339("2025-11-02T05:00:00Z").expect("timestamp");
        let summary = RunSummary::new(
            RunStatus::Completed,
            1,
            1,
            Some(0),
            1,
            None,
            Duration::from_secs(5),
            finished_at,
        );

        let error = formatter
            .format_summary(&summary)
            .expect_err("missing fingerprint");
        assert!(matches!(error, FormatterError::MissingFingerprintEstimates));
    }

    #[test]
    fn formats_checkpoint_interruption_guidance() {
        let checkpoint_path = Path::new("/tmp/checkpoints/run.chk");
        let manifest_path = Path::new("/tmp/checkpoints/run.manifest.json");
        let lineage = [Ulid::from_string("01HZYF3V5VZ2SXDF5C7TE0YE8N").unwrap()];
        let info = CheckpointManifestInfo {
            checkpoint_path,
            manifest_path,
            manifest_version: 1,
            lineage: &lineage,
            checkpoint_id: Some(Ulid::from_string("01HZYF3V7P8Z3M7YKB8N0YJQBJ").unwrap()),
        };

        let lines = format_checkpoint_interruption(&info);
        assert_eq!(
            lines,
            vec![
                "Exploration interrupted before completion.",
                "Checkpoint snapshot: /tmp/checkpoints/run.chk (checkpoint ID 01HZYF3V7P8Z3M7YKB8N0YJQBJ)",
                "Checkpoint manifest v1: /tmp/checkpoints/run.manifest.json",
                "Run ID: 01HZYF3V5VZ2SXDF5C7TE0YE8N",
                "Resume with: tlc resume --checkpoint \"/tmp/checkpoints/run.chk\"",
            ]
        );
    }

    #[test]
    fn formats_checkpoint_mismatch_guidance() {
        let checkpoint_path = Path::new("/data/run.chk");
        let manifest_path = Path::new("/data/run.manifest.json");
        let lineage = [
            Ulid::from_string("01HZYF3V5VZ2SXDF5C7TE0YE8N").unwrap(),
            Ulid::from_string("01HZYF3V8C4RSM4T7ZH5R7TW5Q").unwrap(),
        ];
        let info = CheckpointManifestInfo {
            checkpoint_path,
            manifest_path,
            manifest_version: 2,
            lineage: &lineage,
            checkpoint_id: None,
        };

        let lines =
            format_checkpoint_mismatch(&info, "abcdef1234567890", "feedfacecafebeefdeadbeef");
        assert_eq!(
            lines,
            vec![
                "Cannot resume from checkpoint: specification hash mismatch.",
                "Checkpoint snapshot: /data/run.chk",
                "Checkpoint manifest v2: /data/run.manifest.json",
                "Recorded spec hash: feedfacecafebeefdeadbeef",
                "Current spec hash: abcdef1234567890",
                "Resume lineage: 01HZYF3V5VZ2SXDF5C7TE0YE8N → 01HZYF3V8C4RSM4T7ZH5R7TW5Q",
                "Regenerate the checkpoint by rerunning `tlc run` with matching modules and configuration.",
                "To bypass this safety check (not recommended), rerun with: tlc resume --checkpoint \"/data/run.chk\" --ignore-hash",
            ]
        );
    }

    #[test]
    fn formats_assumption_false_error() {
        let formatter = Formatter::default();
        let diagnostic = Diagnostic::custom(2104, MessageClass::Error, ["ClockAssumption"]);
        let rendered = formatter.format(&diagnostic).expect("formatted");
        assert_eq!(rendered, "Error: Assumption ClockAssumption is false.");
    }

    #[test]
    fn formats_no_fairness_warning() {
        let formatter = Formatter::default();
        let diagnostic = Diagnostic::custom(9297, MessageClass::Warning, ["Spec", "Spec.tla:42"]);
        let rendered = formatter.format(&diagnostic).expect("formatted");
        assert_eq!(
            rendered,
            "Warning: Temporal properties (PROPERTY or PROPERTIES) are being verified without a fairness constraint conjoined to the behavior specification Spec defined at Spec.tla:42. This may lead to trivial counterexamples in which the system exhibits infinite stuttering immediately after the initial state. To avoid this, it is recommended to conjoin a suitable fairness constraint to Spec (compare Chapter 8, page 87ff of Specifying Systems at https://lamport.azurewebsites.net/tla/book.html)."
        );
    }
}
