# tlaplus Development Guidelines

Auto-generated from all feature plans. Last updated: 2025-11-02

## Active Technologies
- Rust 1.91.0 (stable via `rustup`, pinned in `rust-toolchain.toml`) + `clap` (derive), `serde`/`serde_json`, `tracing` + `tracing-subscriber` + `tracing-opentelemetry`, `indicatif`, `rayon`, `crossbeam`, `rusqlite` for SQLite-backed checkpoints (001-rewrite-tlc)
- SQLite checkpoints (via `rusqlite`) supporting 10–100 GB with WAL mode and serde-managed binary blobs (001-rewrite-tlc)

- Rust (latest stable via `rustup`, pinned with `rust-toolchain.toml`) + `clap` (derive), `serde`/`serde_json`, `tracing` + `tracing-subscriber` + `tracing-opentelemetry`, `indicatif`, `rayon`, `crossbeam` (001-rewrite-tlc)

## Project Structure

```text
src/
tests/
```

## Commands

cargo test [ONLY COMMANDS FOR ACTIVE TECHNOLOGIES][ONLY COMMANDS FOR ACTIVE TECHNOLOGIES] cargo clippy

## Code Style

Rust (latest stable via `rustup`, pinned with `rust-toolchain.toml`): Follow standard conventions

## Recent Changes
- 001-rewrite-tlc: Added Rust 1.91.0 (stable via `rustup`, pinned in `rust-toolchain.toml`) + `clap` (derive), `serde`/`serde_json`, `tracing` + `tracing-subscriber` + `tracing-opentelemetry`, `indicatif`, `rayon`, `crossbeam`, `rusqlite` for SQLite-backed checkpoints

- 001-rewrite-tlc: Added Rust (latest stable via `rustup`, pinned with `rust-toolchain.toml`) + `clap` (derive), `serde`/`serde_json`, `tracing` + `tracing-subscriber` + `tracing-opentelemetry`, `indicatif`, `rayon`, `crossbeam`

<!-- MANUAL ADDITIONS START -->
<!-- MANUAL ADDITIONS END -->
