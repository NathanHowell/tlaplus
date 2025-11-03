# Implementation Plan: Native TLC Command Line Tool

**Branch**: `001-rewrite-tlc` | **Date**: 2025-11-02 | **Spec**: `/specs/001-rewrite-tlc/spec.md`
**Input**: Feature specification from `/specs/001-rewrite-tlc/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/commands/plan.md` for the execution workflow.

## Summary

Port the TLC model checker to a native Rust CLI that preserves full behavioral parity with the legacy Java tool, delivers 20% faster multi-core execution, and introduces an integrated TTY progress bar plus NDJSON progress telemetry for non-interactive workflows. The rewrite must retire Java components once golden parity and toolchain gates pass, backed by Rust-native checkpointing, OpenTelemetry instrumentation, and nightly regression/performance automation.

## Technical Context

<!--
  ACTION REQUIRED: Replace the content in this section with the technical details
  for the project. The structure here is presented in advisory capacity to guide
  the iteration process.
-->

**Language/Version**: Rust (latest stable via `rustup`, pinned with `rust-toolchain.toml`)  
**Primary Dependencies**: `clap` (derive), `serde`/`serde_json`, `tracing` + `tracing-subscriber` + `tracing-opentelemetry`, `indicatif`, `rayon`, `crossbeam`  
**Storage**: Chunked `serde` + `zstd` blobs indexed by `sled` embedded key-value store; JSON manifest alongside  
**Testing**: `cargo test`, `cargo nextest`, parity harness via `tlc-parity` crate diffing Java vs Rust, property-based tests with `proptest`  
**Target Platform**: macOS (x86_64/arm64), Linux (x86_64/arm64), Windows (x86_64)  
**Project Type**: Native CLI tool within Rust workspace  
**Performance Goals**: ≥20% throughput gain vs Java TLC on 16-core benchmark suite; ≤5% memory regression  
**Constraints**: 10–100 GB checkpoint support; deterministic 128-bit fingerprints; OpenTelemetry tracing + NDJSON progress schema; zero long-lived Java shims post-parity; CLI-only surface (future distributed orchestration via containers, no RPC); legacy `_PERIODIC` and `_RL_REWARD` hooks retired with documented migration guidance  
**Scale/Scope**: Full TLC feature parity including backlog fixes; single-host multi-core GA with future distributed extension seams

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Rust-First Modernization**: All new code ships in Rust, leveraging best-of-class crates (`tracing`, `rayon`, `sled`, `tokio`) to reimagine the design; parity harness wrappers may invoke Java temporarily, but shims must be retired after two consecutive green parity releases documented in `docs/migration/tlc-rust.md`.
- **Behavioral Parity & Safety Nets**: Nightly golden harness runs full TLC regression corpus through both binaries via `tlc-parity`, owned by the Rust TLC migration team, gating merges before GA.
- **Idiomatic Performance & Concurrency**: Use `rayon` + `crossbeam` work-stealing scheduler, capture benchmarks via `cargo bench`/`criterion` on Paxos/Raft suites, and require review + dedicated tests for any `unsafe`.
- **Evergreen Toolchain & Dependencies**: Pin Rust 1.83 stable in `rust-toolchain.toml`, enforce `fmt`, `clippy -D warnings`, `test`, `nextest`, and `cargo audit` per CI run, document upgrades.
- **Transparent Migration & Collaboration**: Publish bi-weekly updates to maintainers mailing list + Slack, maintain migration register, document retirement of MailSender, `_PERIODIC`, and `_RL_REWARD`, and secure Toolbox release manager sign-off ahead of Java removal.

> Constitution gates satisfied for research kickoff. Post-Phase 1 review: design artifacts uphold principles; no remediation required.

## Project Structure

### Documentation (this feature)

```text
specs/[###-feature]/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)
<!--
  ACTION REQUIRED: Replace the placeholder tree below with the concrete layout
  for this feature. Delete unused options and expand the chosen structure with
  real paths (e.g., apps/admin, packages/something). The delivered plan must
  not include Option labels.
-->

```text
rust/
├── Cargo.toml
├── crates/
│   ├── tlc/                  # Main binary + engine modules
│   │   ├── src/
│   │   │   ├── cli/
│   │   │   ├── engine/
│   │   │   ├── storage/
│   │   │   ├── progress/
│   │   │   └── telemetry/
│   │   ├── benches/
│   │   └── tests/
│   ├── tlc-parity/           # Golden harness + diff utilities
│   │   ├── src/
│   │   └── tests/
│   ├── tlc-checkpoint/       # Checkpoint serialization + sled bindings
│   │   ├── src/
│   │   └── benches/
│   └── tlc-cli-support/      # Shared CLI UX helpers (progress/formatting)
│       └── src/
├── xtask/                    # Developer automation tasks
│   └── src/
└── tools/
    └── parity/               # Scripts + fixtures for regression suite

tests/
├── integration/              # End-to-end CLI exercises
├── parity/                   # Legacy vs Rust diff fixtures
└── smoke/                    # Fast deterministic coverage
```

**Structure Decision**: Adopt a `rust/` workspace housing `tlc` (binary), support crates (`tlc-parity`, `tlc-checkpoint`, `tlc-cli-support`), and shared tooling (`xtask`, `tools/parity`). Integration/parity/smoke tests live under `tests/` to keep regression assets separate from crate sources.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| [e.g., 4th project] | [current need] | [why 3 projects insufficient] |
| [e.g., Repository pattern] | [specific problem] | [why direct DB access insufficient] |
