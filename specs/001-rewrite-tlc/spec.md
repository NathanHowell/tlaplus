# Feature Specification: Native TLC Command Line Tool

**Feature Branch**: `[001-rewrite-tlc]`  
**Created**: 2025-11-02  
**Status**: Draft  
**Input**: User description: "we are going to create a new command line tool named `tlc` that supports all of the existing `tlc` model checker features, as well as any existing issues here https://github.com/tlaplus/tlaplus/issues - it must be multicore, pass all existing tests (we will either use existing pluscal/tla+ models directly, or port any java unit tests to rust). it also must be faster than the Java equivalent. it must have a native progress bar that shows how much of the state space has been explored. while"

## Clarifications

### Session 2025-11-02

- Q: How should the new `tlc` handle mid-run interruptions when checkpoints exist? → A: Match legacy TLC by persisting checkpoints in a Rust-native format (serde for compact states; embedded DB or serde for larger ones) and requiring an explicit resume flag or command.
- Q: What checkpoint size envelope should the new Rust-native storage support? → A: Plan for routine 10–100 GB checkpoints.
- Q: Which runtime artifacts must be captured inside each checkpoint? → A: Persist the frontier/backlog, visited metadata, worker RNG seeds, and a hash of the spec/config inputs.
- Q: Are embedded databases acceptable for checkpoint persistence when serde alone is insufficient? → A: Yes—bundle a lightweight embedded DB like SQLite or sled with the binary when needed.
- Q: What state identity fingerprint should the new engine standardize on? → A: Use a stable 128-bit hash/fingerprint for every explored state.
- Q: How should distributed/remote execution be scoped for the rewrite? → A: Limit GA to single-host multi-core while designing clear extension seams for future Kubernetes/batch schedulers.
- Q: How should the progress indicator behave when stdout is not a TTY? → A: Emit structured JSON progress events instead of the interactive bar.
- Q: What observability stack should the telemetry integrate with? → A: Instrument runs with OpenTelemetry spans via the `tracing` crate and attach a tracing subscriber for console output when applicable.
- Q: What should be the default scope and redaction posture for telemetry exports? → A: Default to local-only spans, remove spec/module identifiers, and require an explicit flag or env var before enabling remote export; console JSON output remains available.
- Q: What default styling should the progress indicator use on TTY outputs? → A: Default to ANSI-colored output with a `--no-color` opt-out flag.
- Q: How should non-TTY progress events be structured? → A: Emit newline-delimited JSON objects (NDJSON) so streaming consumers can parse incremental updates.
- Q: What compatibility guarantee should checkpoint storage offer across releases? → A: No compatibility guarantees; mismatched versions must discard checkpoints and start fresh.
- Q: What should the default worker count be when auto-detecting cores? → A: Default to `max(logical cores − 1, 1)` to reserve headroom.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Preserve TLC Parity (Priority: P1)

Spec engineers can run the new `tlc` binary with their existing specifications, configuration files, and command-line flags to obtain parity with the legacy tool.

**Why this priority**: Without immediate parity, engineers would lose confidence in the new tool and block migration off the legacy implementation.

**Independent Test**: Execute the standard TLC regression suite with the new binary only and confirm outputs, exit codes, and error traces match the baseline.

**Acceptance Scenarios**:

1. **Given** an engineer has a specification and `MC.cfg` that currently passes on legacy TLC, **When** they invoke the new `tlc` binary with identical arguments, **Then** the run completes successfully with the same invariants satisfied and the same exit status.
2. **Given** an engineer runs a spec that triggers a known counterexample today, **When** they execute the same run with the new `tlc`, **Then** the counterexample trace, violation messages, and summary statistics match the legacy output.

---

### User Story 2 - Track Long Runs Visually (Priority: P2)

Spec engineers can observe run progress and estimated coverage directly in the terminal while a long-running exploration executes.

**Why this priority**: Visibility into run progress reduces support load and helps engineers decide whether to continue, adjust parameters, or stop a run.

**Independent Test**: Launch a long-running model on the new `tlc` and verify the progress indicator renders, updates, and completes without relying on legacy tooling.

**Acceptance Scenarios**:

1. **Given** a run exploring a large state space, **When** the engineer monitors the new `tlc` terminal output, **Then** a native progress bar shows explored states, estimated completion percentage, and elapsed time without requiring additional tools.
2. **Given** a run that finishes successfully, **When** the progress indicator reaches completion, **Then** it reports 100% coverage and final statistics without truncation or misreporting.

---

### User Story 3 - Scale Across Cores (Priority: P3)

Infrastructure engineers can provision multi-core hardware and configure the new `tlc` to saturate available cores for faster exploration.

**Why this priority**: Multi-core execution is required to outperform the legacy runtime and justify the migration effort.

**Independent Test**: Run the same specification twice—once with single-core mode, once with configured multi-core—and verify runtime improvements and consistent results without involving other features.

**Acceptance Scenarios**:

1. **Given** a machine with N processor cores, **When** the engineer sets the new `tlc` to use N workers, **Then** the runtime decreases relative to a single-core run while preserving result parity.
2. **Given** a spec that consumes most available memory, **When** the engineer throttles the worker count via CLI flags, **Then** the run respects the configured limit and completes without exceeding resource constraints.

---

### Edge Cases

- Progress indicator must remain responsive when total state count is unknown or changes significantly during exploration.
- The tool must degrade gracefully when worker threads encounter divergent performance (e.g., heterogeneous cores or throttled containers) by maintaining at least 70% aggregate worker utilization, surfacing skew telemetry, and avoiding starvation or deadlock.
- Runs interrupted mid-exploration (user cancel, node reboot) must provide actionable restart guidance and avoid corrupting checkpoints.
- Existing specification files containing legacy TLC quirks (e.g., unusual Unicode, deprecated options) must be parsed and reported consistently.
- On restart after an interruption, the tool MUST keep legacy behavior by persisting checkpoint state in the Rust-native format and requiring the user to explicitly resume using the dedicated CLI flag or command, providing clear CLI guidance that references the checkpoint manifest defined in FR-009.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The new `tlc` command MUST accept the same invocation syntax, configuration files, and environment variables currently supported by legacy TLC.
- **FR-002**: The tool MUST load and execute existing PlusCal- and TLA+-based test suites without requiring file or model changes.
- **FR-003**: The tool MUST expose built-in multi-core execution with automatic worker detection (defaulting to `max(logical cores − 1, 1)` workers) and user overrides for core count and memory usage.
- **FR-004**: The command-line output MUST include a native progress indicator that displays explored states, estimated completion percentage, elapsed time, and current throughput when attached to a TTY, default to ANSI-colored styling while honoring a `--no-color` flag that falls back to monochrome, and MUST emit newline-delimited JSON (NDJSON) progress events with equivalent fields when stdout is non-interactive.
- **FR-005**: For every analysis outcome (success, counterexample, liveness violation, deadlock), the tool MUST emit diagnostics, coverage summaries, and error traces that conform to current TLC semantics.
- **FR-006**: The tool MUST pass all existing automated TLC regression suites, including nightly PlusCal conversions, parser tests, and toolbox integration checks.
- **FR-007**: Identified high-impact TLC backlog issues (correctness gaps, performance defects, CLI usability blockers) MUST be resolved or explicitly retired before the tool is released, with status, owner, and resolution notes recorded in `specs/001-rewrite-tlc/checklists/backlog.csv` and reviewed by TLC maintainers. “High-impact” is defined as any backlog row marked `priority:P0`/`priority:P1` or `severity:critical`, plus any issue explicitly tagged as release-blocking in the curated backlog scope.
- **FR-008**: The tool MUST collect and report run-level metrics (runtime, states-per-second, memory footprint) via CLI output and structured telemetry so regression and benchmark harnesses can compare results against the legacy implementation.
- **FR-009**: Checkpoint persistence MUST use a Rust-native format, preferring serde for compact state payloads and evaluating an embedded local database when state volume or performance constraints exceed serde-only capabilities; Java checkpoint blobs MUST NOT be reused. Solutions MUST comfortably handle checkpoint files in the 10–100 GB range, capture the exploration frontier/backlog, visited-set metadata, worker RNG seeds, and a hash of spec/config inputs, and MAY bundle lightweight embedded databases (e.g., SQLite, sled) when serde alone is insufficient. Cross-version compatibility is NOT guaranteed; mismatched binary versions SHOULD refuse to resume and require fresh runs.
- **FR-010**: State identity MUST rely on a deterministic 128-bit fingerprint across runs and platforms, ensuring collision risk remains negligible while keeping storage efficient.
- **FR-011**: Initial GA scope MUST restrict execution to single-host multi-core operation; distributed or remote worker orchestration is out of scope but the architecture MUST expose extension seams (documented scheduler interfaces, workload handoff contracts, and integration tests) so future Kubernetes or batch orchestrators can coordinate workers without invasive rewrites.
- **FR-012**: Runtime telemetry MUST emit OpenTelemetry-compliant spans using the Rust `tracing` crate, default the subscriber to local-only emission with spec/module identifiers stripped, render structured JSON when writing to the console sink, and require an explicit CLI flag or environment variable before enabling any remote exporter.

### Non-Functional Requirements

- **NFR-001**: Progress indicators MUST refresh at least every 5 seconds during runs exceeding 10 minutes and report coverage within ±2% of actual explored states, with automated validation covering both TTY and NDJSON modes.
- **NFR-002**: Nightly automated runs MUST track throughput, memory usage, and failure rates, publish historical trends, and alert maintainers when deviations exceed thresholds of ≥10 % throughput regression, ≥5 % peak-memory growth, or ≥0.5 % failure rate across the monitored suite.

### Key Entities *(include if feature involves data)*

- **Specification Package**: A bundle containing TLA+ modules, PlusCal translations, configuration files, and parameter overrides required to execute a model check.
- **Exploration Run Record**: The structured result of a `tlc` execution, including invariants checked, explored states, counterexamples, and performance metrics.
- **Progress Telemetry**: Real-time data describing percentage complete, throughput, worker utilization, and estimated completion time, surfaced as an interactive TTY bar or as newline-delimited JSON (NDJSON) events on non-TTY outputs.
- **Checkpoint Snapshot**: A persisted runtime bundle storing the exploration frontier/backlog, visited-state metadata, worker RNG seeds, and an integrity hash derived from the specification and configuration inputs. Serialization uses serde for compact payloads and may bundle a lightweight embedded database (e.g., SQLite, sled) for larger checkpoint data.
- **State Fingerprint**: A deterministic 128-bit hash computed from each canonicalized state representation, reused across runs to minimize collisions and ensure consistent resume behavior.

### Assumptions

- Product leadership will curate the definitive list of backlog issues considered in-scope for this release, record them in `specs/001-rewrite-tlc/checklists/backlog.csv`, and sign off when every entry is resolved or explicitly retired.
- Performance comparisons will use an agreed-upon set of representative specifications and hardware profiles that mirror current TLC adoption.
- Deployment planning assumes the organization is ready to switch automation, CI pipelines, and end-user workflows directly to the new binary once release sign-off occurs.
- Future distributed execution will be delivered via external schedulers (e.g., Kubernetes gang scheduling) that plug into defined extension seams; no coordinator for cross-host workers ships in this release.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of current TLC regression specifications and acceptance tests complete successfully when run exclusively with the new `tlc` binary.
- **SC-002**: Across the agreed performance suite, median wall-clock runtime improves by at least 20% versus the legacy TLC tool on equivalent hardware.
- **SC-003**: During exploratory runs longer than 10 minutes, automated progress validation demonstrates refresh intervals of ≤5 seconds and final coverage deviations of ≤2% from actual explored states across both TTY and NDJSON outputs.
- **SC-004**: The curated list of in-scope TLC backlog issues reaches zero open items prior to release sign-off.
- **SC-005**: Large-checkpoint soak tests covering at least 10 GB and 100 GB resume scenarios complete successfully without data loss, corruption, or parity regressions relative to the legacy implementation.

## Verification Strategy *(mandatory)*

- **Unit/Integration Tests**: Catalogue all ported TLC regression suites, PlusCal model libraries, parser coverage, and CLI flag tests, and ensure automated execution in CI for every change.
- **Golden Parity Harness**: Maintain a side-by-side comparison pipeline that runs representative models on both the new and legacy TLC binaries, capturing output diffs and performance deltas until retirement.
- **Large Checkpoint Soak Tests**: Execute dedicated 10 GB and 100 GB checkpoint generation/resume scenarios that validate storage throughput, resumability, and parity diagnostics under stress.
- **Reproduction Command**: Provide a single documented command (and configuration bundle) that executes the full regression and performance comparison suite for local repro and release validation.
- **Rollback Plan**: Define trigger conditions (parity regression, critical performance loss, missing diagnostics) and the procedure for pausing rollout, issuing hotfixes, or holding releases until blocking issues are resolved.

## Diagnostics & Documentation *(mandatory)*

- **Logging/Output Changes**: Document any new telemetry lines, progress indicators (interactive bar and JSON mode), or exit codes, and verify Toolbox and automation scripts remain compatible with revised output.
- **User-Facing Docs**: Update command reference, getting-started guides, and migration notes to explain new defaults, multi-core controls, and progress visualization.
- **Support Guidance**: Provide troubleshooting playbooks for common run failures, performance tuning tips, and guidance for teams migrating their automation and scripts to the new binary.

## Performance & Scaling *(mandatory)*

- **Benchmark Scenario**: Use the established TLC performance suite (e.g., Paxos, Raft, mutual exclusion models) to measure runtime, memory, and scalability characteristics.
- **Target Budget**: Achieve at least a 20% throughput gain and no more than 5% increase in peak memory usage compared to the baseline when running with 16 worker cores.
- **Monitoring Plan**: Schedule nightly automated runs that capture runtime, throughput, worker-utilization, and failure trends, publish dashboards for historical comparison, and trigger alerts when parity or performance deviates beyond agreed thresholds, including aggregate worker utilization dropping below 70% under skewed workloads.

## Migration & Collaboration *(mandatory)*

- **Legacy TLC Decommission Plan**: Complete the handoff by updating all downstream consumers to the new binary, define cutover criteria and rollback procedures, place the legacy implementation in archival status, and remove its distribution artifacts once replacement is verified and signed off.
- **Stakeholder Updates**: Share bi-weekly progress with Toolbox owners, release managers, and community moderators, culminating in a migration guide and release announcement.
- **Interop/FFI Notes**: Document and version the direct integrations required by ToolBox and automation consumers (interfaces, file formats, invocation semantics) so they target the new binary interfaces and retire legacy references on an agreed timeline.
- **Risk Register**: Track risks such as performance regressions on large specs, unaddressed backlog issues, or tooling incompatibilities, and assign mitigations with responsible owners and review dates.
