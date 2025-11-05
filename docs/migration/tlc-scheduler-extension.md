# TLC Scheduler Extension Seams

**Audience**: Engine and platform teams preparing to plug alternative worker schedulers (e.g., Kubernetes batch controllers) into the Rust TLC engine.  
**Scope**: Documents the contracts required by FR-011 — keeping the shipping release single-host while exposing stable seams for future distributed orchestrators.

---

## 1. Architecture Overview

The engine keeps scheduling concerns isolated inside the `tlc-engine` crate:

- **Sizing & Memory Guards** — `EngineSizing` and `MemoryGuard` derive worker counts and queue capacity from a `RunConfiguration`.
- **Worker Scheduler** — `WorkerScheduler` wraps a dedicated `rayon::ThreadPool` and partitions work via `partition_frontier`.
- **Frontier Queue** — `WorkQueue<T>` offers bounded or unbounded crossbeam channels with throttling telemetry (`QueueStats`).
- **Instrumentation** — `RunMetricsRecorder` emits run-level throughput and peak-memory metrics; `WorkerTelemetry` reports worker utilisation and queue saturation.

These components are intentionally decoupled from the rest of the engine so an external orchestrator can replace or augment the in-process scheduler without rewriting core exploration logic.

```
RunConfiguration ──► EngineSizing ──► {WorkerScheduler, WorkQueue}
       │                                   │              │
       ▼                                   ▼              ▼
  MemoryGuard (queue cap)          Frontier slices   QueueStats ▷ WorkerTelemetry
       │                                   │
       ▼                                   ▼
  External scheduler ⇄ partition_frontier / install / map_slices
```

---

## 2. Extension Surfaces

| Surface | Location | Purpose | Key Contracts |
|---------|----------|---------|----------------|
| `EngineSizing` | `src/engine/src/config.rs` | Computes worker counts (`NonZeroU16` → `NonZeroUsize`) and queue capacity. | Must be reused when provisioning remote workers to honour CLI expectations and memory guards. |
| `partition_frontier` & `FrontierSlice` | `src/engine/src/lib.rs` | Evenly splits frontier workloads. | Extensions MUST preserve contiguous ranges, cover the full frontier, and keep slice length deviation ≤1 (validated by `enforce_partition_invariants`). |
| `WorkerScheduler` | `src/engine/src/scheduler.rs` | Provides `for_each_slice`, `map_slices`, and `install` helpers on top of a Rayon pool. | Third parties may wrap/replace this struct but MUST preserve thread-name observability and `Send + Sync` closure semantics. |
| `WorkQueue` & `QueueStats` | `src/engine/src/work_queue.rs` | Bounded/unbounded queue with throttling telemetry. | External producers must respect blocking semantics for bounded queues and propagate `QueueStats` into telemetry. |
| `WorkerTelemetry` & `QueueTelemetry` | `src/telemetry/src/workers.rs` | Emits utilisation metrics and 70% alerts. | Integrations must continue to feed `WorkerSample` data at ≤1 ms granularity (per `worker_skew` test) and surface queue saturation when available. |
| `RunMetricsRecorder` | `src/engine/src/metrics.rs` | Captures runtime, throughput, and peak memory. | Alternative schedulers must call `record_memory_sample` and `finish` to ensure perf gates (T069/T070) remain meaningful. |

---

## 3. Handoff Contract

1. **Context Preparation**  
   Use `tlc_engine::prepare_run` to validate the spec, configuration, and options. The returned `RunContext` exposes the canonical worker count (`RunContext::worker_count()`) and sizing that downstream schedulers must honour.

2. **Worker Provisioning**  
   - Preserve the worker count and thread-name prefixing used today (`tlc-worker-{idx}` via `WorkerScheduler::with_name_prefix`). New schedulers may add metadata but MUST keep deterministic naming for diagnostics and CI benchmarks.
   - Uphold partition invariants by either invoking `partition_frontier` or reproducing its logic exactly. The integration tests in `tests/integration/engine_scaling.rs` and `tests/integration/worker_skew.rs` assume balanced partitions.

3. **Frontier Handoff**  
   - The queue surface remains the primary seam. External controllers may consume `WorkQueue::sender()` to push remote work or replace the queue with a compatible channel, provided they publish equivalent `QueueStats` for telemetry.
   - Bounded queues MUST block producers when at capacity. Distributed schedulers should mimic this behaviour (back-pressure or explicit flow-control) before dispatching to remote nodes.

4. **Telemetry & Metrics**  
   - Sample worker utilisation at the cadence enforced by `WorkerTelemetry` (≤1 ms) to keep the ≥70 % alert (`T069`) meaningful.
   - Continue to drive `RunMetricsRecorder` so performance gates in `benches/engine_scaling.rs` (`T070`) remain valid after swapping schedulers.
   - Propagate queue saturation and length into `QueueTelemetry::bounded` / `::unbounded` so alert routing does not lose context.

5. **Shutdown & Ownership**  
   - Consumers are responsible for calling `WorkerScheduler::install` (or equivalent) when executing jobs that must remain on the scheduler pool (e.g., checkpoint writes) to avoid mixing runtimes.
   - External schedulers must own graceful shutdown: drain the frontier queue, ensure outstanding slices commit telemetry, and flush metrics via `RunMetricsRecorder::finish`.

---

## 4. Integration Checklist

Before adopting an alternative scheduler:

- [ ] Instantiate the new scheduler through `RunContext::worker_count()` and record the worker IDs you will export for telemetry.
- [ ] Demonstrate workload partitioning that passes `partition_frontier` invariants (slice coverage, ≤1 imbalance).
- [ ] Surface queue depth/saturation into `WorkerTelemetry` samples.
- [ ] Re-run scaling tests (`engine_scaling`, `worker_skew`) and benchmarks (T070 gate) to confirm no regression.
- [ ] Update CI automation to capture the new scheduler metrics while keeping existing log formats intact for downstream tooling.

---

## 5. Forward Work

- **T058** will add integration tests that exercise these seams end-to-end. Do not break public APIs (`EngineSizing`, `WorkerScheduler`, `WorkQueue`, telemetry structs) without updating those tests and this document.
- Long-term distributed execution may wrap this seam with gRPC/IPC handoff. Any such design should preserve the contracts above to keep parity with single-host behaviour and alerting thresholds.

Questions or proposals should be routed through the TLC maintainers list and referenced against FR-011 in `specs/001-rewrite-tlc/spec.md`.
