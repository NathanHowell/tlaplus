# TLC Rust Migration Notes

This document tracks the program-wide guardrails, day-to-day workflow, and the feature parity changes we must manage while transitioning from the legacy Java TLC implementation to the Rust-native CLI.

## Development Workflow

1. **Start from the pinned toolchain**  
   - Run `rustup show` in the repo root and confirm `1.91.0 (stable)` is active. All local and CI work runs against this toolchain.
2. **Sync plans and checklists before coding**  
   - Execute `.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks` to surface the current `FEATURE_DIR`.  
   - Review `/specs/001-rewrite-tlc/tasks.md`, `/specs/001-rewrite-tlc/plan.md`, and any open checklists under `/specs/001-rewrite-tlc/checklists/`. Do not start story work while Phase 2 tasks remain open.
3. **Implement incrementally with guardrails in mind**  
   - Follow task dependencies in order (sequential vs. `[P]` parallel markers) and keep code within the crate(s) listed in each task description.  
   - Keep large SQLite artifacts out of the repo—only scripts and fixtures live under source control.
4. **Run the verification pipeline locally**  
   - Use `scripts/dev/check-all.sh` as the canonical wrapper. By default it runs:
     - `cargo fmt --all`
     - `cargo clippy --all-targets -- -D warnings`
     - `cargo test --workspace --all-features`
     - `cargo audit` (set `SKIP_AUDIT=1` if the tool is unavailable)
     - Parity harness stub via `cargo test -p tlc-parity`
     - Checkpoint soak suite via `cargo test --test checkpoint_soak -- --ignored` (set `SKIP_SOAK=1` to skip large fixtures)
     - Performance hook (optional, controlled by `CHECK_ALL_PERF_CMD`)
   - The script removes any incidental `Cargo.lock` that `cargo` may generate—keep lockfiles out of commits.
5. **Document completion and commit**  
   - Mark finished tasks as `[X]` in `/specs/001-rewrite-tlc/tasks.md`, noting any follow-up TODOs inline.  
   - Include relevant migration notes or TODO breadcrumbs in this document before requesting review.  
   - Commit once the working tree is clean and all guardrails pass locally (CI re-runs `check-all`).

### Quick Flags and Environment Variables

- `SKIP_PARITY=1` — bypass the parity harness while the stub is in place or when running without the legacy TLC binary.
- `SKIP_SOAK=1` — skip generation of placeholder 100 MB / 1 GB checkpoints (use sparingly; CI keeps it enabled).
- `CHECK_ALL_PERF_CMD="cargo test -p tlc-engine -- --ignored perf_*"` — example hook for future performance gates.
- `OTLP_ENDPOINT` / `TLC_TELEMETRY_MODE` — control telemetry behavior; defaults keep spans local unless explicitly opted in.

## Constitution Guardrails

The implementation plan establishes the following guardrails. Treat them as non-negotiable review checklist items:

- **Rust-first modernization** — keep all new code in Rust crates, lean on ecosystem crates (`clap`, `serde`, `tracing`, `rayon`, `crossbeam`, `rusqlite`, `indicatif`), and reject ad-hoc equivalents.
- **Behavioral parity and safety nets** — back every major change with the parity harness, property-based tests, and documented differences in this migration guide.
- **Performance accountability** — maintain the ≥20 % throughput / ≤5 % peak-memory targets; wire benchmark hooks into `check-all` before removing the Java baseline.
- **Deterministic artifacts** — preserve 128-bit fingerprint determinism and reproducible SQLite checkpoints; never commit locally generated checkpoint blobs.
- **Transparent migration** — keep `/docs/migration/`, `/specs/001-rewrite-tlc/plan.md`, and backlog checklists current; stakeholders monitor these files for release readiness.
- **Telemetry privacy** — default to local-only spans, enforce opt-in for remote exporters, and redact spec identifiers from structured logs.

## Retired Features

- **Mail notifications**: The legacy `util.MailSender` integration is removed. Automated environments should rely on external alerting (e.g., CI pipelines, log aggregation) instead of built-in email hooks.
- **`_PERIODIC` config keyword**: Previously allowed naming a zero-argument operator that TLC evaluated during each periodic scheduler cycle, aborting the run if it returned `FALSE`. The Rust CLI does not support this hook. To replicate the behavior, monitor NDJSON progress events (or final return codes) and terminate the process from an external supervisor when your assumption no longer holds.
- **`_RL_REWARD` config keyword and RL simulation mode**: Reinforcement-learning guided simulation (`tlc2.tool.Simulator.rl=true`) and its reward operator are not ported. Users who relied on this experimental mode should switch to the standard random simulator or integrate third-party fuzzing frameworks that consume the CLI.

## Tracking Guidance

- Flag any specs/configs that reference `_PERIODIC` or `_RL_REWARD` during migration reviews and work with spec owners to replace them.
- Update release notes to call out these removals so downstream tools (e.g., Toolbox plugins, CI wrappers) can adjust before the Rust TLC GA.
