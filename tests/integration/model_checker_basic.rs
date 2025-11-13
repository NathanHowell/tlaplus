use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Once,
};

use anyhow::{anyhow, Context, Result};
use tempfile::tempdir;

const SAFE_SPEC: &str = r#"---- MODULE Safe ----
VARIABLE x

Init == x = 0

Next == x' = x

Inv == x = 0
===="#;

const SAFE_CFG: &str = "INVARIANT Inv\n";

const VIOLATING_SPEC: &str = r#"---- MODULE Unsafe ----
VARIABLE x

Init == x = 0

Next == x' = x + 1

Inv == x = 0
===="#;

const VIOLATING_CFG: &str = "INVARIANT Inv\n";

static BUILD_TLC: Once = Once::new();

#[test]
fn cli_reports_successful_run_summary() -> Result<()> {
    let temp = tempdir()?;
    let spec = write_fixture(temp.path(), "Safe.tla", SAFE_SPEC)?;
    let cfg = write_fixture(temp.path(), "Safe.cfg", SAFE_CFG)?;

    let output = run_tlc(&spec, &cfg, temp.path())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "CLI exited with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout,
        stderr
    );

    assert!(
        stdout.contains("Model checking completed"),
        "summary missing success banner: {stdout}"
    );

    let (generated, distinct, queued) = parse_state_counts(&stdout);
    assert_eq!(generated, 1);
    assert_eq!(distinct, 1);
    assert_eq!(queued, 0);

    let optimistic_line = stdout
        .lines()
        .find(|line| line.contains("optimistic"))
        .expect("fingerprint probability optimistic line present");
    assert!(
        optimistic_line.trim().ends_with('0'),
        "expected zero probability, saw: {optimistic_line}"
    );

    Ok(())
}

#[test]
fn cli_reports_invariant_violation_summary() -> Result<()> {
    let temp = tempdir()?;
    let spec = write_fixture(temp.path(), "Unsafe.tla", VIOLATING_SPEC)?;
    let cfg = write_fixture(temp.path(), "Unsafe.cfg", VIOLATING_CFG)?;

    let output = run_tlc(&spec, &cfg, temp.path())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap_or_default();
    assert_eq!(
        code, 2,
        "expected violation exit code, got {code}\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr
    );

    assert!(
        stdout.contains("Invariant 'Inv' violated"),
        "violation banner missing: {stdout}"
    );

    let (generated, distinct, queued) = parse_state_counts(&stdout);
    assert_eq!(generated, 2);
    assert_eq!(distinct, 2);
    assert_eq!(queued, 0);

    Ok(())
}

fn run_tlc(spec: &Path, cfg: &Path, temp_root: &Path) -> Result<Output> {
    let workspace = workspace_root();
    let checkpoints = temp_root.join("checkpoints");
    fs::create_dir_all(&checkpoints)?;

    BUILD_TLC.call_once(|| {
        let status = Command::new("cargo")
            .current_dir(&workspace)
            .args(["build", "-p", "tlc-cli", "--quiet"])
            .status()
            .expect("cargo build -p tlc-cli succeeded");
        if !status.success() {
            panic!("cargo build -p tlc-cli failed with status {status:?}");
        }
    });

    let binary = tlc_binary_path(&workspace)?;

    let output = Command::new(binary)
        .current_dir(&workspace)
        .arg("run")
        .arg("--spec")
        .arg(spec)
        .arg("--config")
        .arg(cfg)
        .arg("--progress")
        .arg("ndjson")
        .arg("--telemetry")
        .arg("local")
        .arg("--checkpoint-dir")
        .arg(&checkpoints)
        .output()
        .with_context(|| "failed to launch tlc CLI binary")?;

    Ok(output)
}

fn parse_state_counts(output: &str) -> (u128, u128, u128) {
    let line = output
        .lines()
        .find(|line| line.contains("states generated"))
        .unwrap_or_else(|| panic!("state summary line missing: {output}"));
    let segments: Vec<&str> = line.split(',').collect();
    let generated = parse_first_number(segments.get(0).copied().unwrap_or_default());
    let distinct = parse_first_number(segments.get(1).copied().unwrap_or_default());
    let queued = parse_first_number(segments.get(2).copied().unwrap_or_default());
    (generated, distinct, queued)
}

fn parse_first_number(segment: &str) -> u128 {
    segment
        .split_whitespace()
        .find_map(|token| token.replace(',', "").parse().ok())
        .unwrap_or_else(|| panic!("unable to parse number from segment: {segment}"))
}

fn write_fixture(root: &Path, name: &str, contents: &str) -> Result<PathBuf> {
    let path = root.join(name);
    fs::write(&path, contents.trim_start_matches('\n'))?;
    Ok(path)
}

fn workspace_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn tlc_binary_path(root: &Path) -> Result<PathBuf> {
    let mut path = root.join("target").join("debug").join("tlc");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    if path.exists() {
        Ok(path)
    } else {
        Err(anyhow!("tlc binary missing at {}", path.display()))
    }
}
