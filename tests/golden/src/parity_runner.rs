use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use clap::Parser;
use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

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

        Ok(Self {
            legacy_launcher,
            specs_root,
            filter: args.filter,
            output_dir,
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

/// Public entrypoint used by the binary.
pub fn run() -> Result<()> {
    let args = HarnessArgs::parse();
    let config = HarnessConfig::try_from(args)?;
    run_with_config(config)
}

fn run_with_config(config: HarnessConfig) -> Result<()> {
    let context = HarnessContext::new(config)?;
    context.run_stub()
}

#[derive(Debug)]
struct HarnessContext {
    config: HarnessConfig,
    workspace: TempDir,
}

impl HarnessContext {
    fn new(config: HarnessConfig) -> Result<Self> {
        let workspace = tempfile::tempdir()
            .context("failed to create temporary workspace for parity harness")?;
        Ok(Self { config, workspace })
    }

    fn run_stub(&self) -> Result<()> {
        tracing::debug!(
            workspace = %self.workspace.path().display(),
            specs = %self.config.specs_root.display(),
            output = %self.config.output_dir.display(),
            legacy_launcher = %self.config.legacy_launcher.display(),
            "Parity harness stub starting."
        );

        if let Some(workers) = self.config.workers {
            tracing::debug!(workers, "Worker override captured for future parity runs.");
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
            let diff_path = self.write_placeholder_diff(&spec)?;
            reports.push(SpecReport {
                spec: spec.display_name,
                status: ParityStatus::Inconclusive,
                diff_artifact: diff_path,
                notes: Some("parity harness stub does not execute TLC binaries yet".to_string()),
            });
        }

        let summary = SummaryReport { specs: reports };
        self.write_summary(&summary)?;

        tracing::info!(
            count = %summary.specs.len(),
            output = %self.config.output_dir.display(),
            "Parity harness stub created placeholder diff artifacts."
        );

        Ok(())
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

    fn write_placeholder_diff(&self, spec: &DiscoveredSpec) -> Result<PathBuf> {
        let relative = PathBuf::from("diffs")
            .join(sanitize_spec_name(&spec.display_name))
            .join("diff.txt");
        let full_path = self.config.output_dir.join(&relative);

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create diff directory '{}'", parent.display())
            })?;
        }

        let contents = format!(
            "Parity diff placeholder for spec '{}'.\n\
             The harness stub does not yet run TLC binaries; replace this artifact \
             once parity execution is implemented.\n\
             Spec source: {}\n\
             Temporary workspace: {}\n",
            spec.display_name,
            spec.path.display(),
            self.workspace.path().display()
        );
        fs::write(&full_path, contents).with_context(|| {
            format!(
                "failed to write diff placeholder file '{}'",
                full_path.display()
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn generates_placeholder_artifacts_and_summary() {
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

        let config = HarnessConfig {
            legacy_launcher: temp.path().join("legacy.sh"),
            specs_root: specs_root.clone(),
            filter: None,
            output_dir: output_dir.clone(),
            workers: Some(4),
            golden_cache: None,
        };

        run_with_config(config).expect("run stub");

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
            assert_eq!(spec.status, ParityStatus::Inconclusive);
            let diff_path = output_dir.join(&spec.diff_artifact);
            assert!(
                diff_path.is_file(),
                "diff artifact should exist at {:?}",
                diff_path
            );
            let contents = fs::read_to_string(diff_path).unwrap();
            assert!(
                contents.contains("Parity diff placeholder"),
                "diff contents should mention placeholder"
            );
        }
    }

    #[test]
    fn respects_filter_glob() {
        let temp = tempdir().expect("temp dir");
        let specs_root = temp.path().join("specs");
        fs::create_dir_all(&specs_root).unwrap();

        fs::create_dir_all(specs_root.join("Paxos")).unwrap();
        fs::write(specs_root.join("Raft.tla"), "---- MODULE Raft ----").unwrap();

        let output_dir = temp.path().join("artifacts");

        let config = HarnessConfig {
            legacy_launcher: temp.path().join("legacy.sh"),
            specs_root: specs_root.clone(),
            filter: Some("Pax*".into()),
            output_dir: output_dir.clone(),
            workers: None,
            golden_cache: None,
        };

        run_with_config(config).expect("run stub with filter");

        let summary_path = output_dir.join("summary.json");
        let summary: SummaryReport =
            serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
        assert_eq!(summary.specs.len(), 1);
        assert_eq!(summary.specs[0].spec, "Paxos");
    }
}
