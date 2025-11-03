# Implementation Plan: Native TLC Command Line Tool

**Branch**: `[001-rewrite-tlc]` | **Date**: 2025-11-02 | **Spec**: `/specs/001-rewrite-tlc/spec.md`
**Input**: Feature specification from `/specs/001-rewrite-tlc/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Deliver a Rust-native `tlc` CLI that preserves the full legacy TLC feature surface while introducing a multi-core exploration engine, resumable checkpoints, interactive progress reporting, and structured telemetry built on idiomatic Rust crates.

## Technical Context

**Language/Version**: Rust 1.91.0 (stable via `rustup`, pinned in `rust-toolchain.toml`)  
**Primary Dependencies**: `clap` (derive), `serde`/`serde_json`, `tracing` + `tracing-subscriber` + `tracing-opentelemetry`, `indicatif`, `rayon`, `crossbeam`, `rusqlite` for SQLite-backed checkpoints  
**Storage**: SQLite checkpoints (via `rusqlite`) supporting 10–100 GB with WAL mode and serde-managed binary blobs  
**Testing**: `cargo test`, golden regression harness diffing against the Java TLC binary, and property-based engine tests (`proptest`) enforced locally and in CI  
**Target Platform**: Cross-platform CLI (Linux/macOS/Windows) with NDJSON progress for non-TTY and `cargo dist`-driven Windows packaging  
**Project Type**: Rust workspace producing the `tlc` binary plus shared libraries  
**Performance Goals**: ≥20 % throughput improvement and ≤5 % additional peak memory versus Java TLC at 16 workers; responsive progress and telemetry under large workloads  
**Constraints**: Deterministic 128-bit state fingerprints, resumable checkpoints, JSON progress stream for non-TTY runs, explicit resume flag, default core detection (`max(logical−1,1)`)  
**Scale/Scope**: Full TLC spec/config coverage, nightly regression + telemetry integration, single-host multi-core with future-ready distributed seams

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Rust-First Modernization**: Rewrite replaces Java TLC with pure Rust modules and uses best-of-class crates (`clap`, `serde`, `tracing`, `rayon`, `crossbeam`, `rusqlite`), with SQLite checkpoints providing the Rust-native persistence layer.
- **Behavioral Parity & Safety Nets**: Plan includes the full TLC regression suite, golden trace comparisons, and documentation of any telemetry output differences to keep parity with the Java baseline until decommissioned.
- **Idiomatic Performance & Concurrency**: Multi-core execution relies on `rayon`/`crossbeam` worker pools, benchmarked against Paxos/Raft/mutual exclusion workloads, with profiling and telemetry hooks scheduled to detect regressions.
- **Evergreen Toolchain & Dependencies**: Work targets Rust 1.91.0, enforcing `cargo fmt`, `cargo clippy -D warnings`, `cargo test`, and `cargo audit` gates within CI and local workflows.
- **Transparent Migration & Collaboration**: Spec `/specs/001-rewrite-tlc/spec.md` drives scope, stakeholders include TLC maintainers, Toolbox integrators, release managers, and community moderators, and communications run via bi-weekly updates, migration notes, and release documentation.

> If any checklist item is unmet, record the remediation plan and pause execution until resolved.

**Post-Design Re-evaluation (Phase 1 Completion)**: Technical design confirms all gates remain satisfied with Rust 1.91.0, SQLite-backed checkpoints via `rusqlite`, golden parity harness coverage, and CI-enforced formatting/linting. No constitution violations identified.

## Project Structure

### Documentation (this feature)

```text
specs/001-rewrite-tlc/
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── contracts/
└── tasks.md
```

### Source Code (repository root)

```text
src/
├── cli/                 # Clap-driven argument parsing and command dispatch
├── engine/              # Core TLC exploration engine and worker orchestration
├── checkpoint/          # Checkpoint serialization and embedded DB adapters
├── telemetry/           # Tracing subscribers, OTLP exporters, diagnostics
├── progress/            # TTY progress bars and NDJSON emitters
└── util/                # Fingerprinting, config loading, shared helpers

tests/
├── golden/              # Golden comparisons against legacy TLC outputs
├── integration/         # End-to-end CLI, checkpoint resume, telemetry tests
└── unit/                # Module-level and property-based engine tests
```

**Structure Decision**: Build a Rust workspace rooted under `src/` with focused modules for CLI, engine, persistence, and telemetry plus aligned test suites to enforce parity and concurrency safety.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
