# Phase 0 Research — Native TLC Command Line Tool

## Task: Research primary dependencies for the Rust TLC CLI
- **Decision**: Use `clap` (derive API) for CLI parsing, `serde`/`serde_json` for config + checkpoint serialization metadata, `tracing` + `tracing-subscriber` + `opentelemetry` exporters for telemetry, `indicatif` for TTY progress rendering, and `rayon` + `crossbeam` for worker scheduling primitives.
- **Rationale**: These crates are mature, well-maintained, and align with Rust ecosystem idioms. `clap` ensures parity with existing TLC flags, `indicatif` provides a customizable multi-thread-safe progress UI, while `rayon`/`crossbeam` support work-stealing and lock-free queues typical for state space exploration.
- **Alternatives considered**: `argh` or `gumdrop` lack advanced flag parity and validation; `structopt` is effectively superseded by `clap`. `tokio` async runtime is optimized for I/O-bound workloads and would add overhead for CPU-bound exploration. Custom progress rendering would increase maintenance without exceeding `indicatif` capabilities.

## Task: Research checkpoint storage strategy for 10–100 GB payloads
- **Decision**: Persist checkpoints as chunked binary segments encoded via `serde` + `zstd`, indexed by a Rust-native embedded key-value store using `sled`, with metadata manifests stored alongside in JSON.
- **Rationale**: `sled` offers high-throughput append-only storage, crash safety, and 128-bit key support—matching our deterministic fingerprint requirements. Chunked blobs keep write amplification manageable, and `zstd` balances compression ratio with speed for large state graphs.
- **Alternatives considered**: `rusqlite`/SQLite handles large files but adds SQL schema management overhead and weaker concurrent write performance. Plain filesystem blobs lack crash consistency and indexing guarantees. Other Rust KV stores (e.g., `heed`) require LMDB which complicates cross-platform packaging.

## Task: Research concurrency and worker orchestration for single-host multi-core runs
- **Decision**: Implement a custom scheduler atop `rayon` thread pools using `crossbeam-deque` for work-stealing, with cooperative checkpoints via atomic epoch markers; guard any unavoidable `unsafe` behind reviewed modules and dedicated tests.
- **Rationale**: Work-stealing fits TLC’s irregular branching factor and enables dynamic load balancing. `rayon` integrates nicely with scoped threads and provides proven ergonomics, while `crossbeam`’s lock-free deques align with low-latency frontier management.
- **Alternatives considered**: A pure `std::thread` pool would require bespoke work-stealing logic, increasing maintenance risk. Async runtimes (`tokio`, `async-std`) incur scheduling overhead and complicate CPU affinity management. GPU offload is out of scope for GA.

## Task: Research parity verification and golden testing strategy
- **Decision**: Stand up a golden harness that executes the full TLC regression suite and representative PlusCal/TLA+ specs through both binaries, diffing outputs via a new `tlc-parity` Rust crate, with nightly CI orchestration owned by the Rust TLC migration team.
- **Rationale**: Automated comparisons de-risk regressions and satisfy Constitution Principle II. Centralizing diff logic in a crate enables reuse for integration tests and ad-hoc investigations.
- **Alternatives considered**: Manual spot checks or selective regressions lack coverage and violate parity requirements. Reusing existing Java harness tooling would entrench legacy dependencies and slow decommissioning.

## Task: Research toolchain governance and dependency audit cadence
- **Decision**: Pin `rust-toolchain.toml` to the active stable release (initially Rust 1.83) with monthly review, enforce `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo nextest`, and `cargo audit` in CI, and document upgrades in `docs/migration/rust-toolchain.md`.
- **Rationale**: Aligns with Constitution Principle IV by staying evergreen, captures reproducibility, and provides audit trails for security posture.
- **Alternatives considered**: Using `nightly` adds instability and reviewer burden; deferring audits to release milestones delays vulnerability detection; omitting `cargo nextest` would lengthen test feedback loops on large suites.

## Task: Research target platform guarantees
- **Decision**: Support macOS (x86_64 + arm64), Linux (x86_64 + arm64), and Windows (x86_64) builds via `cargo` with cross-compilation validated in CI runners; document platform nuances in the quickstart.
- **Rationale**: Mirrors current TLC distribution footprint and ensures ToolBox integrations remain functional across developer environments.
- **Alternatives considered**: Limiting GA to Linux would block Windows/macOS users; adding tier-3 platforms (e.g., FreeBSD) would dilute focus before parity completion.

## Task: Research telemetry export and progress event format
- **Decision**: Emit TTY progress via `indicatif` multi-progress instances, with NDJSON events serialized through `serde_json` following a documented schema (`run_id`, `timestamp`, `states_explored`, `percent_complete`, `throughput`, `eta`). Attach OpenTelemetry spans via `tracing-opentelemetry`, defaulting to console and file subscribers with opt-in OTLP exporters.
- **Rationale**: Satisfies spec requirements for dual-mode progress reporting and Constitution Principle III observability expectations while keeping dependencies cohesive.
- **Alternatives considered**: Building a bespoke TUI (e.g., `ratatui`) exceeds GA scope; exposing Prometheus metrics would require running HTTP servers and complicate air-gapped use cases.

## Task: Research migration communication and Java shim retirement plan
- **Decision**: Maintain a migration register in `docs/migration/tlc-rust.md`, send bi-weekly updates through the TLC maintainers mailing list and internal Slack channel, require sign-off from Toolbox owners before removing Java artifacts, and limit Java shims to parity harness wrappers scheduled for removal once parity metrics hold for two consecutive releases.
- **Rationale**: Provides transparent collaboration per Constitution Principle V, clarifies ownership, and bounds the lifetime of any residual Java code.
- **Alternatives considered**: Ad-hoc announcements risk stakeholder drift; retaining broad Java interop would violate the Rust-first mandate.
