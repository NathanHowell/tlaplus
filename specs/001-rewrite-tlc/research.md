## Phase 0 Research Notes

### Rust Toolchain Version
- Decision: Pin Rust 1.91.0 in a repository-level `rust-toolchain.toml` and use that toolchain for all TLC workspace crates.
- Rationale: 1.91.0 is the current stable release, satisfying the constitution’s requirement to stay on the latest toolchain while capturing improvements to borrow checking, `std::sync`, and profile-guided optimization hooks relevant to TLC performance. Pinning ensures reproducible builds across developers and CI.
- Alternatives considered: Staying unpinned risks drift between developer environments; targeting nightly would increase instability and churn in CI without clear benefit for TLC.

### Checkpoint Storage Backend
- Decision: Adopt `sqlite` via the `rusqlite` crate for checkpoint persistence, complemented by binary blob columns for serialized states and metadata tables for frontier/backlog bookkeeping.
- Rationale: SQLite is mature, battle-tested at 100 GB scale, supports safe concurrent read/write patterns via WAL mode, and offers strong tooling for integrity checks. `rusqlite` bindings are well maintained and integrate cleanly with serde-based encoding.
- Alternatives considered: `sled` offers a pure-Rust option but has an unstable roadmap and less predictable recovery tooling; `redb` is emerging but lacks the long-term operational track record required for TLC checkpoints.

### Testing & Parity Harness
- Decision: Build a Rust test harness that shells out to the legacy Java TLC binary to capture golden outputs, then run identical scenarios through the new `tlc` binary under `cargo test` integration suites and nightly CI.
- Rationale: This approach preserves the existing regression suite without rewriting all models immediately, enables automatic diffing of traces/statistics, and lets us block regressions before the Java path is retired. Property-based fuzzing (e.g., `proptest`) will cover engine invariants.
- Alternatives considered: Manual parity testing would not scale; deferring to Toolbox integration lacks coverage for engine-level regressions.

### Windows Packaging Strategy
- Decision: Use `cargo dist` (or `cargo zigbuild` for cross-compilation) to generate signed Windows artifacts (`.msi`/`.zip`) from CI, relying on `cross` for Linux/macOS builds and testing Windows behavior through GitHub Actions runners.
- Rationale: `cargo dist` automates target-specific packaging, integrates well with release pipelines, and avoids hand-maintained scripts. Utilizing cross-compilation keeps local developer requirements minimal while ensuring reproducible builds across platforms.
- Alternatives considered: Maintaining bespoke PowerShell scripts would be brittle; relying solely on Windows developers for releases would slow the pipeline and reduce confidence.

### Clap Command-Line Design
- Decision: Model the CLI with `clap` derive macros, mapping legacy TLC flags to subcommands/arguments and adding new telemetric/progress options as structured arguments with `ArgGroup`s for mutually exclusive flags.
- Rationale: Derive macros keep definitions declarative, ensure help text stays synced, and simplify validation (required groups, default values) compared to manual parsing.
- Alternatives considered: Using raw `clap::Command` builders is more verbose; hand-rolled parsers risk drift from established flag semantics.

### Serde Serialization Strategy
- Decision: Use `serde` with explicit `#[serde(with = "...")]` modules for binary state encoding and `serde_json` for progress/telemetry output, ensuring compatibility with checkpoint blobs and NDJSON streams.
- Rationale: Custom serializers give control over compact binary formats while leveraging serde’s ecosystem; JSON output remains standard and debuggable.
- Alternatives considered: Implementing bespoke serialization would increase maintenance; alternative formats (CBOR/Bincode) remain possible for specific blobs but add integration work without immediate need.

### Tracing & OpenTelemetry Integration
- Decision: Instrument the engine with `tracing` spans/events, attach `tracing-subscriber` layers for CLI output, and gate OTLP export via `tracing-opentelemetry` behind an opt-in flag/environment variable.
- Rationale: This combination satisfies structured logging requirements, keeps console output ergonomic, and provides a pathway to remote telemetry when explicitly enabled, honoring privacy defaults.
- Alternatives considered: Direct `opentelemetry` APIs would duplicate effort; bespoke logging would forgo ecosystem tooling and structured filtering.

### Progress Rendering
- Decision: Use `indicatif` for TTY progress bars while emitting NDJSON progress updates via a dedicated serializer for non-TTY environments.
- Rationale: `indicatif` delivers polished terminal output with minimal code and plays well with multi-threaded updates; NDJSON keeps automation-friendly semantics.
- Alternatives considered: Writing custom progress rendering would slow delivery; `console`/`yansi` provide coloring but lack full progress management.

### Concurrency Foundations
- Decision: Combine `rayon` for data-parallel exploration over state frontiers with `crossbeam` channels for worker coordination and checkpoint triggers.
- Rationale: `rayon` excels at parallel iterators and work-stealing, while `crossbeam` offers lightweight channels and synchronization primitives suited for high-throughput workloads.
- Alternatives considered: Native threads with manual scheduling would duplicate `rayon` functionality; async runtimes (`tokio`) are unnecessary for CPU-bound search loops.
