//! Formatting utilities for TLC CLI diagnostics and legacy-compatible summaries.

use std::{fmt, path::Path};

use thiserror::Error;
use tlc_util::RunStatus;
use ulid::Ulid;

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
    Custom(u32),
}

impl MessageCode {
    fn value(self) -> u32 {
        match self {
            MessageCode::Success => 2193,
            MessageCode::Stats => 2199,
            MessageCode::StatsDfid => 2204,
            MessageCode::SearchDepth => 2194,
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
}

impl RunSummary {
    pub fn new(
        status: RunStatus,
        states_generated: u128,
        distinct_states: u128,
        states_left_on_queue: Option<u128>,
        search_depth: u64,
        fingerprint: Option<FingerprintEstimates>,
    ) -> Self {
        Self {
            status,
            states_generated,
            distinct_states,
            states_left_on_queue,
            search_depth,
            fingerprint,
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
        let body = render_body(diagnostic.code, &diagnostic.parameters)?;
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

        Ok(lines)
    }
}

fn render_body(code: MessageCode, params: &[String]) -> Result<String, FormatterError> {
    match code {
        MessageCode::Success => render_success(params),
        MessageCode::Stats => render_stats(params, true),
        MessageCode::StatsDfid => render_stats(params, false),
        MessageCode::SearchDepth => render_search_depth(params),
        MessageCode::Custom(value) => Err(FormatterError::UnknownCode(value)),
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
    use tlc_util::RunStatus;

    #[test]
    fn formats_success_banner_with_two_probabilities() {
        let formatter = Formatter::default();
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
        );

        let lines = formatter.format_summary(&summary).expect("summary");
        assert_eq!(lines.len(), 3);
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
    }

    #[test]
    fn formats_success_banner_with_single_probability() {
        let formatter = Formatter::default();
        let summary = RunSummary::new(
            RunStatus::Completed,
            12345,
            6789,
            None,
            42,
            Some(FingerprintEstimates::new("0.0", None)),
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
        let summary = RunSummary::new(RunStatus::Completed, 1, 1, Some(0), 1, None);

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
}
