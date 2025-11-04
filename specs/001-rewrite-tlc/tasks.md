---
description: "Task list for Native TLC Command Line Tool"
---

# Tasks: Native TLC Command Line Tool

**Input**: Design documents from `/specs/001-rewrite-tlc/`  
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: Only include dedicated test tasks when stories or guardrails require them.  
**Organization**: Tasks are grouped by user story to keep increments independently deliverable and testable.

**Constitution Guardrails**:
- Establish automated verification (cargo test, parity harness, perf checks) before major feature work.
- Prefer idiomatic Rust crates (`clap`, `serde`, `rayon`, `crossbeam`, `rusqlite`, `tracing`, `indicatif`) instead of ad-hoc reimplementations.
- Provide deterministic reproduction scripts and benchmark harnesses with recorded toolchain versions.
- Keep diagnostics, telemetry, and documentation aligned with new CLI behaviors so downstream tooling adapts smoothly.
- Track performance and migration risks, escalating regressions before decommissioning the Java TLC build.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Task can run in parallel (different files, no blocking dependencies).
- **[Story]**: Applies only to user story phases (US1, US2, US3).
- All descriptions include repository-relative file paths.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Create the Rust workspace skeleton and toolchain guardrails required by every subsequent phase.

- [X] T001 Pin Rust toolchain to version 1.91.0 in `rust-toolchain.toml`
- [X] T002 Configure TLC workspace members and profiles in the root `Cargo.toml`
- [X] T003 [P] Scaffold shared utilities crate manifest with serde/tracing defaults in `src/util/Cargo.toml`
- [X] T004 [P] Scaffold CLI crate manifest with `clap` derive support in `src/cli/Cargo.toml`
- [X] T005 [P] Scaffold engine crate manifest with `rayon`/`crossbeam` dependencies in `src/engine/Cargo.toml`
- [X] T006 [P] Scaffold checkpoint crate manifest with `rusqlite` features in `src/checkpoint/Cargo.toml`
- [X] T007 [P] Scaffold telemetry crate manifest with `tracing-subscriber` and `tracing-opentelemetry` in `src/telemetry/Cargo.toml`
- [X] T008 [P] Scaffold progress crate manifest with `indicatif` and `serde_json` in `src/progress/Cargo.toml`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Deliver shared data models, telemetry, persistence, and automation harnesses that every user story depends on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [X] T009 Implement `SpecificationPackage`, `RunConfiguration`, and `ExplorationRun` structs with serde validation in `src/util/src/model.rs`
- [X] T010 [P] Implement deterministic `StateFingerprint` utilities and helpers in `src/util/src/fingerprint.rs`
- [X] T011 Wire tracing subscribers and OTLP gating for telemetry bootstrap in `src/telemetry/src/lib.rs`
- [X] T012 [P] Implement SQLite checkpoint store initialization with WAL tuning in `src/checkpoint/src/lib.rs`
- [X] T013 Define checkpoint metadata helpers for `CheckpointSnapshot` management in `src/checkpoint/src/snapshot.rs`
- [X] T074 Add integration tests confirming telemetry defaults to local-only spans with spec/module identifiers redacted in `tests/integration/telemetry_defaults.rs`
- [X] T075 Document remote telemetry opt-in flow and enforce CLI/ENV gating in `docs/migration/tlc-telemetry.md`
- [X] T078 Implement checkpoint manifest versioning and mismatch rejection in `src/checkpoint/src/version.rs`
- [X] T079 Add negative resume integration test covering stale checkpoint manifests in `tests/integration/checkpoint_version.rs`
- [X] T082 [P] Capture cross-platform fingerprint fixtures and determinism tests in `tests/unit/fingerprint_determinism.rs`
- [X] T014 Configure parity harness crate manifest and legacy TLC launcher shim in `tests/golden/Cargo.toml`
- [X] T015 [P] Stub parity runner scaffolding that captures diff artifacts in `tests/golden/src/parity_runner.rs`
- [X] T016 [P] Scaffold property-based engine invariants using `proptest` in `tests/unit/engine_prop.rs`
- [X] T017 Publish deterministic verification script chaining fmt/clippy/tests/audit/parity/perf in `scripts/dev/check-all.sh`
- [X] T065 [P] Implement large-checkpoint soak generator and resume tests covering 10 GB and 100 GB scenarios in `tests/integration/checkpoint_soak.rs`
- [X] T066 Integrate checkpoint soak runs into `scripts/dev/check-all.sh` and CI gating so failures block merges
- [X] T018 Document shared development workflow and constitution guardrails in `docs/migration/tlc-rust.md`
- [X] T019 Curate TLC backlog scope, assign owners, and define remediation status taxonomy in `specs/001-rewrite-tlc/checklists/backlog.csv`
- [X] T051 Document backlog remediation workflow (status codes, exit criteria, reviewer checklist) in `specs/001-rewrite-tlc/checklists/README.md`
- [X] T052 Record maintainer sign-off for resolved or retired backlog items directly in `specs/001-rewrite-tlc/checklists/backlog.csv`

**Checkpoint**: Foundation ready — user story implementation can now begin in parallel.

---

## Phase 3: User Story 1 - Preserve TLC Parity (Priority: P1) 🎯 MVP

**Goal**: Spec engineers can run the new `tlc` binary with existing specs/configs and match legacy TLC behavior end-to-end.  
**Independent Test**: Execute the standard TLC regression suite with only the new binary and confirm outputs, exit codes, and traces match the legacy baseline.

### Tests & Validation

- [X] T020 [P] [US1] Extend parity runner to invoke new TLC binary and generate diff reports in `tests/golden/src/parity_runner.rs`
- [X] T021 [US1] Curate regression manifest listing parity specs and legacy expectations in `tests/golden/fixtures/manifest.toml`
- [X] T022 [US1] Integrate property-based engine invariants into CI gating in `tests/golden/src/parity_runner.rs`
- [X] T067 [US1] Add regression fixtures that exercise legacy Unicode inputs and deprecated CLI flags in `tests/golden/fixtures/legacy/`
- [X] T068 [US1] Validate ingestion pipeline behavior for legacy quirks in `tests/golden/src/parity_runner.rs`

### Implementation Tasks

- [X] T023 [P] [US1] Implement `tlc run`/`tlc resume` command definitions and flag parity in `src/cli/src/commands.rs`
- [X] T024 [P] [US1] Implement spec/config ingestion pipeline mapping to data models in `src/cli/src/input_loader.rs`
- [X] T025 [US1] Implement exploration engine entrypoint with invariant checking in `src/engine/src/lib.rs`
- [X] T026 [P] [US1] Implement checkpoint resume flow and lineage tracking in `src/engine/src/resume.rs`
- [X] T027 [P] [US1] Implement CLI output and diagnostics formatter matching legacy summaries in `src/cli/src/output.rs`
- [X] T028 [US1] Wire CLI binary main to engine, telemetry, and checkpoint modules in `src/cli/src/main.rs`
- [X] T084 [US1] Reproduce legacy final run summaries (states/depth banners) in `src/cli/src/output.rs` and assert parity in `tests/golden/src/parity_runner.rs`
- [X] T085 [US1] Port TLC `MP` warning/error catalog and compare outputs against Java in `tests/golden/src/parity_runner.rs`
- [X] T086 [US1] Mirror `-debug`/`-terse`/`-nowarning`/`-userFile` behaviors in `src/cli/src/output.rs` with parity runner validation
- [X] T053 [P] [US1] Emit run-level metrics (runtime, states-per-second, memory) from engine telemetry hooks in `src/engine/src/metrics.rs`
- [X] T054 [US1] Assert metric presence and formatting in parity and regression harnesses in `tests/golden/src/parity_runner.rs`
- [X] T080 [US1] Surface checkpoint manifest details and resume instructions in CLI interruption and mismatch messages within `src/cli/src/output.rs`

**Checkpoint**: Legacy parity verified — release-ready MVP.

---

## Phase 4: User Story 2 - Track Long Runs Visually (Priority: P2)

**Goal**: Spec engineers can observe run progress and estimated coverage directly in the terminal during long explorations.  
**Independent Test**: Launch a long-running model on the new `tlc` and verify the progress indicator renders, updates, and completes without legacy tooling.

### Tests & Validation

- [X] T029 [P] [US2] Add NDJSON schema regression covering progress events in `tests/integration/progress_ndjson.rs`

### Implementation Tasks

- [X] T030 [P] [US2] Implement `ProgressEvent` models and NDJSON writer in `src/progress/src/ndjson.rs`
- [X] T031 [P] [US2] Implement TTY progress renderer with `indicatif` in `src/progress/src/tty.rs`
- [X] T032 [US2] Integrate progress event emission into the engine loop in `src/engine/src/progress.rs`
- [X] T033 [US2] Wire CLI `--progress` flag resolution to renderer selection in `src/cli/src/commands.rs`
- [X] T034 [US2] Document progress modes, NDJSON contract, and Toolbox considerations in `docs/migration/tlc-progress.md`
- [X] T035 [US2] Implement `--no-color` flag handling and validation in `src/cli/src/commands.rs` and `tests/integration/progress_tty.rs`
- [X] T055 [US2] Build progress validation harness measuring refresh cadence and coverage accuracy in `tests/integration/progress_validation.rs`
- [X] T056 [US2] Enforce progress accuracy thresholds in CI via parity harness hooks in `tests/golden/src/parity_runner.rs`

**Checkpoint**: Visual and non-TTY progress experiences complete.

---

## Phase 5: User Story 3 - Scale Across Cores (Priority: P3)

**Goal**: Infrastructure engineers can configure multi-core execution and throttle resources to outperform single-core runs safely.  
**Independent Test**: Run a representative spec once in single-core mode and once with configured multi-core workers to verify runtime improvement with consistent results.

### Tests & Validation

- [X] T036 [US3] Add multi-core scaling integration scenario covering throughput deltas in `tests/integration/engine_scaling.rs`
- [ ] T037 [P] [US3] Add benchmarking harness that records scaling metrics in `benches/engine_scaling.rs`
- [ ] T069 [US3] Add skewed workload integration test enforcing ≥70% aggregate worker utilization in `tests/integration/worker_skew.rs`
- [ ] T070 [US3] Enforce ≥20% throughput improvement and ≤5% peak-memory ceiling via benchmark gate in `benches/engine_scaling.rs`

### Implementation Tasks

- [X] T038 [P] [US3] Implement worker scheduler leveraging `rayon` for state exploration in `src/engine/src/scheduler.rs`
- [X] T039 [P] [US3] Implement crossbeam-backed work queues and throttling in `src/engine/src/work_queue.rs`
- [X] T040 [US3] Integrate worker configuration, memory guards, and defaults in `src/engine/src/config.rs`
- [X] T041 [US3] Wire CLI worker/memory flags and defaults into command parsing in `src/cli/src/commands.rs`
- [X] T042 [P] [US3] Emit worker utilization telemetry with configurable 70% alert thresholds in `src/telemetry/src/workers.rs`
- [ ] T057 [US3] Document scheduler extension seams and handoff contracts in `docs/migration/tlc-scheduler-extension.md`
- [ ] T058 [P] [US3] Add integration tests that lock extension seam stability in `tests/integration/scheduler_extension.rs`

**Checkpoint**: Multi-core execution tuned and benchmarked.

---

## Final Phase: Polish & Cross-Cutting Concerns

**Purpose**: Close documentation, packaging, and reproducibility gaps that span all user stories.

- [ ] T043 [P] Refresh quickstart instructions with new CLI flags and workflows in `specs/001-rewrite-tlc/quickstart.md`
- [ ] T044 Update migration guidance and stakeholder notes in `docs/migration/tlc-rust.md`
- [ ] T045 [P] Capture final parity, performance, and cargo-audit results in `specs/001-rewrite-tlc/parity-ledger.md`
- [ ] T046 [P] Add release packaging manifest for `cargo dist` in `dist/cargo-dist.toml`
- [ ] T047 [P] Update risk register, backlog disposition, and mitigation checkpoints in `docs/migration/tlc-risk-register.md`
- [ ] T048 Run fmt/clippy/tests/audit/parity/perf verification script in `scripts/dev/check-all.sh`
- [ ] T071 Wire performance gate binary into `scripts/dev/check-all.sh` and CI workflows so ≥20% throughput / ≤5% memory thresholds block merges
- [ ] T049 Publish support and troubleshooting guidance for the Rust TLC CLI in `docs/migration/tlc-support.md`
- [ ] T050 Share final stakeholder update and archive summary in `docs/migration/tlc-rust.md`
- [ ] T059 Automate nightly performance/parity runs with dashboards in `.github/workflows/nightly-tlc.yml`, capturing throughput, memory, utilization, and failure rates
- [ ] T060 Configure alerting thresholds (≥10% throughput regression, ≥5% memory growth, ≥0.5% failure rate, <70% utilization) for nightly metrics and document response playbooks in `docs/migration/tlc-monitoring.md`
- [ ] T061 Draft legacy TLC decommission plan with cutover and rollback criteria in `docs/migration/tlc-decommission.md`
- [ ] T062 Secure maintainer and stakeholder sign-off on decommission readiness in `docs/migration/tlc-rust.md`
- [ ] T063 Inventory Toolbox and automation integrations, capturing interface details in `docs/migration/tlc-interop.md`
- [ ] T064 Maintain interop retirement timeline and version matrix in `docs/migration/tlc-interop.md`
- [ ] T076 Publish bi-weekly stakeholder status updates and decisions in `docs/migration/tlc-status.md`
- [ ] T077 Maintain communication calendar and distribution list for Toolbox, release, and community stakeholders in `docs/migration/tlc-status.md`
- [ ] T081 Document checkpoint manifest format and restart workflow in `docs/migration/tlc-resume.md`
- [ ] T083 Run Toolbox smoke tests validating CLI output compatibility in `tests/integration/toolbox_sanity.rs`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately.
- **Foundational (Phase 2)**: Depends on Phase 1 completion; blocks all user stories.
- **User Story Phases (3–5)**: Each depends on Phase 2; proceed in priority order or parallel once dependencies clear.
- **Polish (Final Phase)**: Depends on completion of targeted user stories and foundational verification.

### User Story Dependencies

- **User Story 1 (P1)**: Requires foundational telemetry, persistence, parity harness, and backlog readiness (T009–T019, T051–T052); no downstream dependencies.
- **User Story 2 (P2)**: Requires US1 engine hooks plus foundational progress crate manifest; integrates with US1 outputs but testable independently.
- **User Story 3 (P3)**: Requires US1 engine baseline; operates independently of US2 progress features.

### Within Each User Story

- Tests (if present) should be authored before implementation tasks consume their contracts.
- Shared modules (`src/cli/src/commands.rs`, `src/engine/src/lib.rs`) should merge sequentially per story to avoid conflicts.
- Complete story validation (regression suite, progress demo, scaling benchmarks) before moving to the next increment.

### Parallel Opportunities

- Setup manifests (T003–T008) can run concurrently.
- Foundational tasks marked [P] (T010, T012, T015, T016) can proceed in parallel once workspace files exist.
- After Phase 2, separate teams can tackle US1, US2, and US3 concurrently as long as shared files are sequenced.
- Tests marked [P] within stories (e.g., T020, T030, T038) can execute alongside implementation on different files.

---

## Parallel Example: User Story 1

```bash
# Parallel tasks to accelerate parity validation
Task: "T020 Extend parity runner to invoke new TLC binary and generate diff reports in tests/golden/src/parity_runner.rs"
Task: "T024 Implement spec/config ingestion pipeline mapping to data models in src/cli/src/input_loader.rs"
```

## Parallel Example: User Story 2

```bash
# Parallel tasks to deliver progress telemetry
Task: "T030 Implement ProgressEvent models and NDJSON writer in src/progress/src/ndjson.rs"
Task: "T031 Implement TTY progress renderer with indicatif in src/progress/src/tty.rs"
```

## Parallel Example: User Story 3

```bash
# Parallel tasks to optimize multi-core scaling
Task: "T038 Implement worker scheduler leveraging rayon for state exploration in src/engine/src/scheduler.rs"
Task: "T039 Implement crossbeam-backed work queues and throttling in src/engine/src/work_queue.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Finish Phases 1–2 to guarantee parity harness, telemetry, and persistence exist.
2. Complete Phase 3 tasks (T020–T028) and run regression suite from `scripts/dev/check-all.sh`.
3. Hold release review after parity is certified.

### Incremental Delivery

1. Deliver MVP (US1) and publish updated regression results.
2. Layer progress visualization (US2) and publish NDJSON/TTY documentation.
3. Add multi-core scaling (US3) and capture benchmark deltas.
4. Execute Final Phase polish tasks to prepare migration guidance and packaging.

### Parallel Team Strategy

1. Shared effort on Setup and Foundational phases (T001–T019).
2. Assign US1 to engine/CLI specialists, US2 to progress/UX engineers, and US3 to performance engineers.
3. Coordinate shared file merges (`src/cli/src/commands.rs`, `src/engine/src/lib.rs`) via feature branches to keep stories independent.

---

## Notes

- Tasks marked [P] reside in separate files or scripts and can proceed independently.
- Each user story includes explicit validation tasks to remain independently testable.
- Update documentation and scripts as part of the same tasks to keep parity with implementation.
