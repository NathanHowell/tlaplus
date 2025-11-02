<!--
Sync Impact Report
Version: 0.0.0 → 1.0.0
Modified Principles:
- I. Safety-Critical Correctness
- II. Deterministic Reproducibility
- III. Transparent Diagnostics
- IV. Scalable Performance Discipline
- V. Open Collaboration & Traceability
Added Sections:
- Additional Constraints
- Development Workflow
Removed Sections:
- None
Template Updates:
- ✅ .specify/templates/plan-template.md
- ✅ .specify/templates/spec-template.md
- ✅ .specify/templates/tasks-template.md
Follow-ups:
- None
-->

# TLA+ TLC Model Checker Constitution

## Core Principles

### I. Safety-Critical Correctness
- Every code or configuration change MUST land with executable verification: unit tests, TLC regression models under `tlatools/org.lamport.tlatools/test-model`, or both, proving the expected safety or liveness behavior.
- Changes that affect error handling MUST demonstrate the failure mode through tests or deterministic reproductions, and reviewers MUST confirm the added coverage before merge.
- Feature work that alters TLC semantics MUST ship with updated user documentation in `docs/` describing the model-checking contract.
*Rationale: TLC underpins safety-critical systems; rigorous automated proofs and documentation prevent regressions from reaching users.*

### II. Deterministic Reproducibility
- Build tooling (Ant, Maven, Java runtime) MUST remain pinned to recorded versions; deviations require docs updates and reproducible upgrade pathways.
- Any new TLC runtime option or default change MUST provide a deterministic reproduction script (e.g., `ant -f customBuild.xml test-set ...`) so maintainers can validate behavior.
- State persistence, checkpoints, and distributed execution features MUST keep deterministic fingerprints; introduce seeding controls when randomness is unavoidable.
*Rationale: Deterministic tooling lets engineers reproduce defects, trust CI results, and compare runs across environments.*

### III. Transparent Diagnostics
- TLC output, logging, and coverage reporting MUST remain parseable by the Toolbox; new messages or tags require synchronized parser updates and doc updates.
- Performance or correctness warnings MUST include actionable guidance (flag name, config path, remediation steps) in human-visible output.
- Telemetry additions MUST default to opt-in and respect existing privacy assurances; shipped diagnostics need alignment with open-source expectations.
*Rationale: Clear, actionable diagnostics reduce support load and allow rapid triage of complex model-checking sessions.*

### IV. Scalable Performance Discipline
- Any change impacting worker scheduling, fingerprinting, or state exploration MUST include before/after measurements using benchmarks in `tlatools/org.lamport.tlatools/test-benchmark` or newly added scripts.
- Memory usage ceilings MUST be documented for new features along with guidance on tuning (`-workers`, `-memory`, checkpoint cadence); regressions require mitigation plans.
- Distributed TLC paths MUST continue to support current compatibility matrix (Toolbox launches, CLI scripts); compatibility breaks need migration notices.
*Rationale: TLC users rely on predictable throughput at scale; performance regressions directly block adoption.*

### V. Open Collaboration & Traceability
- Significant features MUST begin with a shared plan/spec authored via `.specify/templates` outputs, linked in PR descriptions before coding begins.
- Each merged change MUST reference issues or RFCs capturing stakeholder consensus (TLA+ Foundation stewards, contributors), including reviewer sign-off.
- Knowledge transfer requires updating `docs/`, `DEVELOPING.md`, or inline ADR-style notes so future maintainers can follow decisions without archaeology.
*Rationale: Transparent process builds trust in a global volunteer community and keeps institutional knowledge searchable.*

## Additional Constraints
- Toolchain: Target Java 11 LTS; additions must confirm compatibility with Ant 1.9.8+ and document Maven adjustments in `DEVELOPING.md`.
- Dependency management: All third-party jars belong under `tlatools/org.lamport.tlatools/lib` with recorded license metadata; upgrades mandate checksum verification.
- Testing: `ant -f customBuild.xml test` and distributed TLC smoke tests MUST pass before release tagging; failures block publishing.
- Documentation: CLI options, Toolbox integration, and new diagnostics MUST be reflected in `docs/` with runnable examples and expected outputs.

## Development Workflow
- Start every significant enhancement by running `/speckit.plan` and `/speckit.spec` to capture intent, success criteria, and constitution compliance checkpoints.
- Implement incrementally by user story, ensuring each slice is independently testable and reviewable; avoid cross-story entanglement.
- Prior to merge, execute the full regression suite relevant to TLC (`test`, targeted `test-set`, benchmarks) and attach reproducible commands to the review.
- After merge, update release notes and notify downstream stakeholders (e.g., Toolbox maintainers) when interface-affecting changes ship.

## Governance
- Authority: This constitution supersedes conflicting engineering practices for TLC. Stewards include the TLC maintainers delegated by the TLA+ Foundation.
- Amendments: Proposals require an issue referencing the desired change, a redlined draft, and approval from two maintainers plus one Toolbox representative before merge.
- Versioning: Update the constitution using semantic versioning—MAJOR for principle changes/removals, MINOR for new sections or material expansions, PATCH for clarifications.
- Compliance: Before each tagged release, run a constitution review checklist covering principles, toolchain constraints, and workflow adherence; record findings in release notes.

**Version**: 1.0.0 | **Ratified**: 2025-11-02 | **Last Amended**: 2025-11-02
