# Feature Specification: Native TLC Command Line Tool

**Feature Branch**: `[001-rewrite-tlc]`  
**Created**: 2025-11-02  
**Status**: Draft  
**Input**: User description: "we are going to create a new command line tool named `tlc` that supports all of the existing `tlc` model checker features, as well as any existing issues here https://github.com/tlaplus/tlaplus/issues - it must be multicore, pass all existing tests (we will either use existing pluscal/tla+ models directly, or port any java unit tests to rust). it also must be faster than the Java equivalent. it must have a native progress bar that shows how much of the state space has been explored. while"

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
- The tool must degrade gracefully when worker threads encounter divergent performance (e.g., heterogeneous cores or throttled containers) without stalling the run.
- Runs interrupted mid-exploration (user cancel, node reboot) must provide actionable restart guidance and avoid corrupting checkpoints.
- Existing specification files containing legacy TLC quirks (e.g., unusual Unicode, deprecated options) must be parsed and reported consistently.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The new `tlc` command MUST accept the same invocation syntax, configuration files, and environment variables currently supported by legacy TLC.
- **FR-002**: The tool MUST load and execute existing PlusCal- and TLA+-based test suites without requiring file or model changes.
- **FR-003**: The tool MUST expose built-in multi-core execution with automatic worker detection and user overrides for core count and memory usage.
- **FR-004**: The command-line output MUST include a native progress indicator that displays explored states, estimated completion percentage, elapsed time, and current throughput.
- **FR-005**: For every analysis outcome (success, counterexample, liveness violation, deadlock), the tool MUST emit diagnostics, coverage summaries, and error traces that conform to current TLC semantics.
- **FR-006**: The tool MUST pass all existing automated TLC regression suites, including nightly PlusCal conversions, parser tests, and toolbox integration checks.
- **FR-007**: Identified high-impact TLC backlog issues (correctness gaps, performance defects, CLI usability blockers) MUST be resolved or explicitly retired before the tool is released.
- **FR-008**: The tool MUST collect and report run-level metrics (runtime, states-per-second, memory footprint) to enable side-by-side comparisons with the legacy implementation.

### Key Entities *(include if feature involves data)*

- **Specification Package**: A bundle containing TLA+ modules, PlusCal translations, configuration files, and parameter overrides required to execute a model check.
- **Exploration Run Record**: The structured result of a `tlc` execution, including invariants checked, explored states, counterexamples, and performance metrics.
- **Progress Telemetry**: Real-time data points describing percentage complete, throughput, worker utilization, and estimated completion time shown in the CLI.

### Assumptions

- Product leadership will curate the definitive list of backlog issues considered in-scope for this release and sign off when all are resolved or retired.
- Performance comparisons will use an agreed-upon set of representative specifications and hardware profiles that mirror current TLC adoption.
- Deployment planning assumes the organization is ready to switch automation, CI pipelines, and end-user workflows directly to the new binary once release sign-off occurs.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of current TLC regression specifications and acceptance tests complete successfully when run exclusively with the new `tlc` binary.
- **SC-002**: Across the agreed performance suite, median wall-clock runtime improves by at least 20% versus the legacy TLC tool on equivalent hardware.
- **SC-003**: During exploratory runs longer than 10 minutes, the progress indicator refreshes at least every 5 seconds and final coverage deviates by no more than 2% from actual explored states.
- **SC-004**: The curated list of in-scope TLC backlog issues reaches zero open items prior to release sign-off.

## Verification Strategy *(mandatory)*

- **Unit/Integration Tests**: Catalogue all ported TLC regression suites, PlusCal model libraries, parser coverage, and CLI flag tests, and ensure automated execution in CI for every change.
- **Golden Parity Harness**: Maintain a side-by-side comparison pipeline that runs representative models on both the new and legacy TLC binaries, capturing output diffs and performance deltas until retirement.
- **Reproduction Command**: Provide a single documented command (and configuration bundle) that executes the full regression and performance comparison suite for local repro and release validation.
- **Rollback Plan**: Define trigger conditions (parity regression, critical performance loss, missing diagnostics) and the procedure for pausing rollout, issuing hotfixes, or holding releases until blocking issues are resolved.

## Diagnostics & Documentation *(mandatory)*

- **Logging/Output Changes**: Document any new telemetry lines, progress indicators, or exit codes, and verify Toolbox and automation scripts remain compatible with revised output.
- **User-Facing Docs**: Update command reference, getting-started guides, and migration notes to explain new defaults, multi-core controls, and progress visualization.
- **Support Guidance**: Provide troubleshooting playbooks for common run failures, performance tuning tips, and guidance for teams migrating their automation and scripts to the new binary.

## Performance & Scaling *(mandatory)*

- **Benchmark Scenario**: Use the established TLC performance suite (e.g., Paxos, Raft, mutual exclusion models) to measure runtime, memory, and scalability characteristics.
- **Target Budget**: Achieve at least a 20% throughput gain and no more than 5% increase in peak memory usage compared to the baseline when running with 16 worker cores.
- **Monitoring Plan**: Schedule nightly automated runs that capture runtime, throughput, and failure trends, alerting maintainers when performance or parity deviates beyond agreed thresholds.

## Migration & Collaboration *(mandatory)*

- **Legacy TLC Decommission Plan**: Complete the handoff by updating all downstream consumers to the new binary, place the legacy implementation in archival status, and remove its distribution artifacts once replacement is verified.
- **Stakeholder Updates**: Share bi-weekly progress with Toolbox owners, release managers, and community moderators, culminating in a migration guide and release announcement.
- **Interop/FFI Notes**: Document any direct integrations required by ToolBox and automation consumers so they target the new binary interfaces and retire legacy references.
- **Risk Register**: Track risks such as performance regressions on large specs, unaddressed backlog issues, or tooling incompatibilities, and assign mitigations with responsible owners and review dates.
