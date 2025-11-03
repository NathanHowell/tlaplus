# Data Model — Native TLC Command Line Tool

## SpecificationPackage
- **Fields**
  - `id` (UUID) — uniquely identifies the package across runs.
  - `modules` (list<Path>) — ordered TLA+/PlusCal modules to load.
  - `config_path` (Path) — location of the `MC.cfg` or equivalent configuration.
  - `parameters` (map<string, string>) — flag/value overrides supplied at runtime.
  - `hash` (blake3-256) — deterministic digest of modules + config for checkpoint validation.
- **Relationships**
  - Referenced by `ExplorationRun.spec_package_id`.
  - Versioned by `CheckpointSnapshot.spec_hash`.
- **Validation Rules**
  - Required files must exist and be readable before run submission.
  - `hash` must match the canonical digest computed during run startup.
- **State Transitions**
  - Immutable once registered; updates create a new package record with a fresh `id`.

## RunConfiguration
- **Fields**
  - `run_id` (ULID) — monotonic identifier for the execution.
  - `workers` (u16) — number of worker threads (defaults to `max(logical_cores − 1, 1)`).
  - `memory_limit_bytes` (u64, optional) — cap for state cache/checkpoint buffering.
  - `telemetry_mode` (`local`, `json`, `otlp`) — governs subscriber wiring.
  - `progress_mode` (`tty`, `ndjson`) — chosen output stream behavior.
  - `resume_from` (CheckpointId, optional) — pointer to checkpoint for resume flows.
- **Relationships**
  - Owned by `ExplorationRun.configuration`.
  - Consumed when instantiating worker pools and telemetry subscribers.
- **Validation Rules**
  - `workers >= 1` and cannot exceed detected logical cores unless `--force`.
  - `memory_limit_bytes` must be ≥ 1 GiB when specified.
  - When `resume_from` is set, `telemetry_mode` must align with checkpoint metadata.
- **State Transitions**
  - Mutable until run enters `Pending` → `Running`; thereafter read-only.

## ExplorationRun
- **Fields**
  - `run_id` (ULID).
  - `spec_package_id` (UUID).
  - `status` (`Pending`, `Running`, `Checkpointing`, `Completed`, `Failed`, `Cancelled`).
  - `started_at` / `completed_at` (RFC3339 timestamps).
  - `states_explored` (u128) — aggregate count.
  - `coverage_percent` (f32) — last known coverage estimate.
  - `throughput_eps` (f64) — states explored per second.
  - `fingerprint_space` (u128) — total unique fingerprints persisted.
  - `checkpoint_ids` (ordered list<CheckpointId>).
  - `regression_trace_path` (Path, optional) — diff artifact when parity fails.
- **Relationships**
  - Owns many `ProgressEvent` entries (stream).
  - Owns many `CheckpointSnapshot` records.
  - Linked to `ParityRunResult` for golden harness comparisons.
- **Validation Rules**
  - Only one run per `run_id`.
  - `completed_at` requires terminal `status`.
  - Coverage must monotonically increase or stay flat; decreases flagged for review.
- **State Transitions**
  - `Pending` → `Running` → (`Checkpointing` ↔ `Running`)* → `Completed`/`Failed`/`Cancelled`.
  - Resumes load prior `CheckpointSnapshot` and re-enter `Running`.

## ProgressEvent
- **Fields**
  - `event_id` (ULID, per-run ordered).
  - `run_id` (ULID).
  - `timestamp` (RFC3339).
  - `states_explored` (u128).
  - `percent_complete` (f32, 0–100).
  - `throughput_eps` (f64).
  - `workers_active` (u16).
  - `eta_seconds` (u64, optional).
  - `is_tty_render` (bool) — differentiates TTY vs NDJSON emission.
- **Relationships**
  - Streamed to clients during execution.
  - Archived with the `ExplorationRun`.
- **Validation Rules**
  - `states_explored` must be non-decreasing.
  - `percent_complete` must live within [0, 100].
  - `eta_seconds` omitted when estimate unavailable.
- **State Transitions**
  - Append-only log keyed by `event_id`; no updates.

## CheckpointSnapshot
- **Fields**
  - `checkpoint_id` (ULID).
  - `run_id` (ULID).
  - `spec_hash` (blake3-256) — ensures resume compatibility.
  - `created_at` (RFC3339).
  - `sqlite_page_size` (u32) — page size used when checkpoint DB created.
  - `frontier_blocks` (table: `frontier_blocks`) — each row stores `block_id`, compressed frontier blob (`BLOB`), and deque ordering.
  - `visited_index` (table: `visited_states`) — columns `fingerprint` (u128 stored as `BLOB`), `generation`, `metadata`.
  - `rng_seeds` (table: `worker_rng`) — per-worker seeds keyed by worker index.
  - `resume_flags` (table: `resume_flags`) — persisted CLI toggles required for restart.
- **Relationships**
  - Many-to-one with `ExplorationRun`.
  - Provides input for `RunConfiguration.resume_from`.
- **Validation Rules**
  - `spec_hash` must match active `SpecificationPackage.hash`.
  - SQLite WAL + SHM files, when present, must pass integrity check (`PRAGMA integrity_check`).
  - `frontier_blocks` must contain at least one row unless run completed.
- **State Transitions**
  - Immutable after write; superseded checkpoints append to list.

## StateFingerprint
- **Fields**
  - `value` (u128) — canonical hash of normalized state.
  - `generation` (u64) — BFS/DFS layer where first observed.
  - `checksum` (u32) — optional fast guard for corruption detection.
- **Relationships**
  - Stored inside `CheckpointSnapshot.visited_index`.
  - Referenced by `ParityRunResult` comparisons.
- **Validation Rules**
  - `value` generated via approved fingerprinting algorithm (documented in engine module).
- **State Transitions**
  - Created upon first visit; immutable thereafter.

## ParityRunResult
- **Fields**
  - `comparison_id` (ULID).
  - `run_id` (ULID).
  - `legacy_run_id` (string) — identifier from Java baseline harness.
  - `status` (`Match`, `Mismatch`, `Inconclusive`).
  - `diff_artifact` (Path, optional) — pointer to diff output.
  - `metrics_delta` (struct: runtime_delta_ms, memory_delta_bytes, states_delta).
- **Relationships**
  - Associated with each `ExplorationRun` executed in the golden harness.
  - Consumed by CI gate reporting.
- **Validation Rules**
  - `diff_artifact` required when `status = Mismatch`.
- **State Transitions**
  - Set after harness execution; re-run overwrites record for same `run_id`.

## TelemetrySpan
- **Fields**
  - `span_id` (string).
  - `trace_id` (string).
  - `name` (string) — e.g., `"tlc.run.worker"`.
  - `start_time` / `end_time` (RFC3339).
  - `attributes` (map<string, string|int|float>) — includes redacted spec metadata.
- **Relationships**
  - Linked to `ExplorationRun` and optionally exported to OTLP collectors.
- **Validation Rules**
  - Attribute keys sanitized to avoid leaking spec names; remote export requires opt-in flag.
- **State Transitions**
  - Generated during execution; retained per run as telemetry archive.
