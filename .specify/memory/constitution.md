<!--
Sync Impact Report
Version: 1.0.0 → 2.0.0
Modified Principles:
- I. Safety-Critical Correctness → I. Rust-First TLC Core
- II. Deterministic Reproducibility → II. Behavioral Parity & Safety Nets
- III. Transparent Diagnostics → III. Idiomatic Performance & Concurrency
- IV. Scalable Performance Discipline → IV. Evergreen Toolchain & Dependencies
- V. Open Collaboration & Traceability → V. Transparent Migration & Collaboration
Added Sections:
- None
Removed Sections:
- None
Template Updates:
- ✅ .specify/templates/plan-template.md
- ✅ .specify/templates/spec-template.md
- ✅ .specify/templates/tasks-template.md
Follow-ups:
- ⚠ Update `DEVELOPING.md` and onboarding docs to reflect Rust-first toolchain once migration guides are prepared
-->

# TLA+ TLC Model Checker Constitution

## Core Principles

### I. Rust-First TLC Core
- All new TLC code and test coverage MUST be implemented in Rust using idiomatic crates; no new Java-based TLC logic may be introduced.
- Migration work MUST retire legacy Java components once feature parity is proven and provide a clear removal plan for residual shims.
- FFI or interop layers MUST remain thin, audited, and documented to enable short-lived coexistence during the migration window.
*Rationale: Centering the rewrite in Rust ensures we gain memory safety, modern tooling, and a sustainable long-term codebase.*

### II. Behavioral Parity & Safety Nets
- Every Rust port MUST ship with golden tests that compare outputs against the Java baseline until that baseline is decommissioned.
- Safety properties MUST be enforced through unit tests, property-based tests, and TLC self-check models covering semantics and error modes.
- Regression harnesses MUST be automated in CI so parity drift is detected within the same commit.
*Rationale: Preserving TLC’s semantics while rewriting prevents regressions from escaping during the cross-language transition.*

### III. Idiomatic Performance & Concurrency
- Prefer safe Rust concurrency primitives (`std::sync`, async runtimes) and profiling before resorting to `unsafe`; any `unsafe` block MUST include justification and targeted tests.
- Performance-sensitive changes MUST include before/after metrics from cargo benches or dedicated benchmarks mirroring large model workloads.
- Memory footprints and worker scaling behavior MUST be documented and validated under representative loads prior to release.
*Rationale: Rust’s strengths lie in safe performance; disciplined concurrency and measurement keep TLC scalable.*

### IV. Evergreen Toolchain & Dependencies
- The project MUST track the latest stable Rust toolchain via `rustup`, updating within two weeks of each stable release and recording the version in docs.
- Crate dependencies MUST pin to the latest compatible releases, accompanied by security and license checks before merge.
- Build artifacts MUST be reproducible via `cargo` workflows, including cross-compilation targets needed by users.
*Rationale: Staying current on toolchains and crates unlocks ecosystem improvements and avoids supply-chain risk.*

### V. Transparent Migration & Collaboration
- Each Rust port effort MUST begin with a shared plan/spec referencing prior Java behavior, migration risks, and stakeholder sign-off.
- Documentation (`docs/`, `DEVELOPING.md`, migration guides) MUST be updated alongside code to explain new architectures, toolchains, and any remaining Java shims.
- Stakeholders (TLC maintainers, Toolbox integrators, community) MUST receive progress updates and deprecation notices ahead of removals.
*Rationale: Coordinated communication keeps the community aligned while significant architectural changes ship.*

## Additional Constraints
- Toolchain: Adopt the latest stable Rust (via `rustup toolchain install stable`), ensuring CI validates with `cargo fmt`, `cargo clippy`, and `cargo test`. Maintain compatibility notes for required Rust features.
- Dependency Management: Use Cargo workspaces for TLC crates, pin dependencies in `Cargo.toml`, and document security audit results (e.g., `cargo audit`) in release notes.
- Interop: Maintain minimal FFI adapters to integrate remaining Java components; document interfaces and removal timelines in `docs/migration`.
- Testing: Migration tasks MUST convert Java tests to Rust (`cargo test`, property-based tests) and keep parity with existing TLAPS/TLC models until confirmed redundant.
- Documentation: Update CLI help, Toolbox integration notes, and developer onboarding to reflect Rust build steps (`cargo build`, `cargo bench`, `cargo nextest` if used).

## Development Workflow
- Initiate each migration slice with `/speckit.plan` and `/speckit.spec`, outlining parity validation, toolchain implications, and Rust-specific risks.
- Implement features in small, reviewable Rust modules with accompanying tests before deleting Java code; keep dual paths only as long as parity harnesses require.
- Run `cargo test`, `cargo fmt --check`, `cargo clippy -- -D warnings`, relevant TLC model regressions, and performance benchmarks prior to merge.
- Coordinate release notes to flag completed ports, pending Java removals, and any user-facing behavioral changes stemming from the Rust rewrite.

## Governance
- Authority: TLC Rust migration stewards, endorsed by the TLA+ Foundation, interpret this constitution and arbitrate migration decisions.
- Amendments: Submit constitution changes via tracked issues, include comparative drafts, and secure approval from two Rust maintainers plus one Toolbox representative.
- Versioning: Use semantic versioning—MAJOR for rewrites or principle replacements, MINOR for new guidance or sections, PATCH for clarifications.
- Compliance: Each milestone release MUST include a migration compliance checklist covering toolchain updates, parity verification, dependency audits, and documentation status.

**Version**: 2.0.0 | **Ratified**: 2025-11-02 | **Last Amended**: 2025-11-02
