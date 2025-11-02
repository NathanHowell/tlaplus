# Quickstart — Native TLC Command Line Tool

## Prerequisites
- Rust toolchain pinned to stable 1.83 (`rustup toolchain install 1.83.0 && rustup override set 1.83.0` in repo root).
- `cargo binstall` (optional) for installing helper binaries such as `cargo-nextest`.
- Java 17 runtime to execute the legacy TLC binary for parity harness comparisons.
- `zstd` CLI (optional) for inspecting compressed checkpoint chunks.

## Repository Setup
```bash
git clone https://github.com/tlaplus/tlaplus.git
cd tlaplus
git checkout 001-rewrite-tlc
rustup show  # confirm stable 1.83 active
cargo fetch  # pre-warm crates
```

## Building the CLI
```bash
cargo build -p tlc
```

- Binary output: `target/debug/tlc`.
- Use `cargo build --release -p tlc` for benchmark runs.

## Running TLC
```bash
./target/debug/tlc run \
  --spec /path/to/Main.tla \
  --config /path/to/MC.cfg \
  --workers auto \
  --progress tty
```

- `--workers auto` defaults to `max(logical cores − 1, 1)`.
- Use `--progress ndjson` to emit newline-delimited JSON events to stdout.
- Telemetry defaults to local spans; enable OTLP export with `--telemetry otlp --otlp-endpoint https://collector:4317`.

### Resuming from a Checkpoint
```bash
./target/debug/tlc resume \
  --checkpoint checkpoints/run-2025-11-02T03-15-00Z.chk \
  --progress tty
```

- Checkpoint filenames encode the `CheckpointSnapshot.checkpoint_id`.
- Resume refuses mismatched spec hashes; override with `--ignore-hash` only for debugging.

## Parity Harness
```bash
cargo run -p tlc-parity -- \
  --java-bin path/to/legacy/tlc.sh \
  --specs-path tests/parity/regression-suite \
  --output-dir artifacts/parity
```

- Produces diff artifacts when outputs diverge.
- CI runs this nightly; local execution helps investigate mismatches.

## Testing & Linting
```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo nextest run
cargo test -p tlc --lib --bins
```

- Benchmarks live under `crates/tlc/benches`. Execute via `cargo bench -p tlc`.

## Observability
- TTY progress uses `indicatif`; collapse output with `--no-color` for monochrome terminals.
- NDJSON progress events follow the schema documented in `/specs/001-rewrite-tlc/contracts/cli.md`.
- OpenTelemetry spans flush to local JSON by default (`logs/tlc-trace.json`); provide OTLP endpoint to stream to collectors.

## Support Channels
- File migration updates in `docs/migration/tlc-rust.md`.
- Stakeholder cadence: bi-weekly status notes in `#tlc-rust` Slack and maintainers mailing list.
- Report blockers via GitHub issues labeled `rust-tlc`.
