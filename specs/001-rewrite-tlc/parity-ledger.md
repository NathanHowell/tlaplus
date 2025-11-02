# TLC Feature Parity Ledger

Purpose: enumerate every legacy TLC capability we must preserve in the Rust rewrite, trace each to the existing surface area, and capture the current porting approach and risk posture.

Sources reviewed:
- CLI option listing generated in `tlatools/org.lamport.tlatools/src/tlc2/TLC.java` lines 1595–1760.
- Model configuration keywords in `tlatools/org.lamport.tlatools/src/tlc2/tool/impl/ModelConfig.java` lines 53–142.
- Coverage behavior documentation in `docs/module-coverage-statistics.md`.
- Engine package structure (`tlatools/org.lamport.tlatools/src/tlc2/tool/**`) for fairness, simulation, SpecTE, and checkpointing semantics.

Status legend:
- ✅ Covered in Rust plan/design.
- 🟡 Planned but needs deeper design detail.
- 🔴 Not yet addressed / open question.

---

## CLI Surface

| Flag / Positional | Legacy Behavior | Rust Parity Strategy | Status / Risk |
| --- | --- | --- | --- |
| `SPEC` positional | Path (or jar resource) to primary module. Supports JAR-bundled specs via `ModelInJar`. | Introduce `SpecSource` abstraction: handle filesystem specs, `.jar`/`.zip` archives, and embedded bundles. Archive loader targets `/model/` entries, asserts presence of `MC.tla` (plus optional `MC.cfg`, `generated.properties`), materializes contents into an isolated temp workspace, and pushes that directory to the front of the module search path so downstream components see the same layout that `InJarFilenameToStream` provided. CLI keeps legacy UX—omitting the positional spec triggers embedded bundle lookup, supplying a `.jar`/`.zip` path selects the archive source, and `--spec-root` overrides the bundle subdirectory when migration artifacts diverge. Distributed runners (`tlc server`, resume flows) reuse the same loader. | ✅ Bundle format + resolver parity locked; bundler tooling tracked separately. |
| `-config file` | Load `.cfg` or inline config; defaults to `SPEC.cfg`. | CLI flag maps to `RunConfiguration.config_path`; parser reuses same grammar (`ModelConfig` port). | 🟡 Config parser rewrite required; tracked under config parity epic. |
| `-workers num|auto` | Set worker threads (default 1; `auto` uses logical cores). | Implement in `engine::scheduler` with `rayon` pool sizing & `auto` policy; enforce `-debugger` forcing 1 worker. | ✅ |
| `-checkpoint minutes` | Minutes between background checkpoints (default 30). | Scheduler triggers checkpoints via `sled` persistence; support minute interval semantics and `0` = disabled. | 🟡 Need policy for interval rounding, initial checkpoint timing. |
| `-metadir path` | Override state / metadata directory (default `SPEC/states`). | Map to checkpoint root in `storage::layout`; ensure relative paths resolve like legacy. | 🟡 Entry in storage design; confirm path normalization. |
| `-recover id` | Resume from checkpoint ID. | Ported via `tlc resume` command; ensure ID naming matches legacy to keep automation working. | ✅ |
| `-cleanup` | Remove states dir before run. | Add pre-run cleanup option toggling storage manager. | ✅ |
| `-continue` | Do not halt on invariant violation. | CLI flag sets `RunPolicy::ContinueOnViolation`. | ✅ |
| `-deadlock` | Skip deadlock checking (equivalent to `CHECK_DEADLOCK=FALSE`). | CLI flag toggles engine behavior & overrides config default. | ✅ |
| `-coverage minutes` | Emit module coverage data at interval + final report. | Integrate with metrics module replicating output format documented in `docs/module-coverage-statistics.md`. | 🟡 Format fidelity + cost accounting to be validated. |
| `-postCondition mod!oper` | Evaluate constant-level operator at end. | Add `postcondition` runner executing constant expression with same semantics. | 🟡 Needs evaluation pipeline design. |
| `-difftrace` | Print only differences between successive states. | Mirror legacy trace printer behavior in Rust `trace` module. | ✅ |
| `-dumpTrace format file` | Dump error trace as TLA or JSON to file; supports multiple invocations. | Implement writer supporting `tla`/`json` (pluggable). Ensure file path relative to spec dir. | ✅ |
| `-inv expr` | Evaluate additional invariant expression. | CLI maps to `RunConfiguration.extra_invariants`; reused in engine. | ✅ |
| `-invlevel n` | Stop after finding a trace of length `n` unless `-continue`. | Implement via `trace::LevelInvariant`. | 🟡 Need semantics confirmation for interplay with parity harness. |
| `-debug` | Enable verbose internal diagnostics. | Map to `tracing` level + additional debug assertions. | ✅ |
| `-dump [format] file` | Dump reachable states; optional DOT with modifiers (`colorize`, `actionlabels`, `constrained`). | Provide state export module replicating textual + DOT outputs and modifiers. | 🟡 DOT emission + modifier handling requires design. |
| `-fp N` | Pick specific irreducible polynomial for fingerprints. | Provide deterministic mapping in fingerprint engine; ensure compatibility with `FP64`. | 🟡 Need to port polynomial table. |
| `-fpbits num` | Configure MSB bits for nested `MultiFPSet`. | Support nested fingerprint tiers; expose parameter. | 🟡 Requires port of MultiFPSet logic. |
| `-fpmem num` | Fraction of physical memory for fingerprint storage. | Accept fraction, integrate with memory manager sizing. | 🟡 Need host memory detection in Rust. |
| `-noGenerateSpecTE` | Disable Trace Explorer spec generation on failure. | Preserve TE spec generation and allow opt-out flag. | ✅ |
| `-teSpecOutDir dir` | Override TE spec output directory. | Accept path, ensure relative semantics. | ✅ |
| `-gzip` | Toggle gzip compression for value IO. | Provide equivalent compression toggle on checkpoint/value streams. | 🟡 Determine defaults & interplay with `zstd`. |
| `-h` | Print usage. | CLI auto-provides help; ensure verbose help matches legacy layout for parity tests. | ✅ |
| `-maxSetSize num` | Bound enumerated set size (default 1,000,000). | Cap in evaluator; reuse config. | 🟡 Need to port `SetEnumValue` semantics. |
| `-nowarning` | Suppress warnings. | Map to logging filter. | ✅ |
| `-terse` | Collapse `Print` output expansion. | Provide same toggle in trace printer. | ✅ |
| `-tool` | Emit message codes for Toolbox integration; auto-enabled with SpecTE. | Provide message catalog compatibility and ensure codes match legacy `MP`. | 🟡 Need mapping of message IDs → codes. |
| `-userFile file` | Redirect `Print` output to file. | Implement user output sink identical to legacy behavior. | ✅ |
| `-debugger [options]` | Enable TLC debugger (DAP), optional `nosuspend`, `nohalt`, `port=`, etc.; forces single worker. | Provide debugger adapter or interoperability layer; replicate flag parsing and worker restriction. | 🟡 Requires dedicated design for debugger front-end, port mapping. |
| `-dfid num` | Use depth-first iterative deepening with initial limit `num`. | Engine supports DFID scheduling mode; parity harness tests required. | 🟡 DFID strategy port pending. |
| `-view` | Apply VIEW operator when printing states. | Include view evaluator hooking into trace printer. | ✅ |
| `-depth num` | Random simulation depth (default 100). | Simulation runner honors depth limit. | ✅ |
| `-seed num` | Set random seed. | Support deterministic RNG seeding. | ✅ |
| `-aril num` | Adjust seed (legacy semantics). | Reproduce additive seed tweak for compatibility. | 🟡 Need to confirm formula with `RandomGenerator`. |
| `-simulate [file=…,num=…]` | Random simulation mode, with optional trace count and trace file prefix. Implies `-workers 1`. | Provide simulation mode with optional outputs to modules. | 🟡 Need output format parity for trace files. |

---

## Configuration File Keywords (`.cfg`)

| Keyword | Legacy Semantics | Rust Implementation Notes | Status / Risk |
| --- | --- | --- | --- |
| `SPECIFICATION` | Names main behavior spec (`Spec`), may reference constant operator. | Parser populates `SpecificationPackage` spec root; integrate with executor. | ✅ |
| `INIT` / `NEXT` | Overrides default behavior spec by naming init/action operators. | Support same override semantics, including absence (error). | ✅ |
| `CONSTANT` / `CONSTANTS` | Assign concrete values or model values. | Implement constant substitution engine supporting model values & overrides. | 🟡 Requires value system parity. |
| `ALIAS` | Provide shorthand names for operator fragments. | Preserve alias expansion. | 🟡 Need to port alias handling. |
| `CONSTRAINT` / `CONSTRAINTS` | State constraints limiting explored states. | Implement filter in successor generation. | ✅ |
| `ACTION_CONSTRAINT`(S) | Action-level constraints filtering transitions. | Mirror behavior in action evaluation. | ✅ |
| `INVARIANT(S)` | Additional state invariants. | Already covered via CLI + config ingestion. | ✅ |
| `PROPERTY` / `PROPERTIES` | Temporal (LTL) properties to check. | Port property checker & liveness graph. | 🟡 Requires full liveness checking port. |
| `POSTCONDITION` | Evaluate operator at end (mirrors CLI). | Share execution path with CLI option. | 🟡 Shared implementation outstanding. |
| `SYMMETRY` | Symmetry reduction specification. | Implement symmetry reduction engine parity. | 🟡 Complex: need design for orbit representatives & hashing. |
| `_PERIODIC` | Name of a 0-arity operator that TLC re-evaluates during each scheduler “periodic work” cycle; if it ever returns `FALSE`, TLC aborts with `TLC_ASSUMPTION_FALSE` (used to encode run-time assumptions during long runs). | **Will not be ported.** Document migration guidance pointing users to external run supervisors (e.g., monitor NDJSON progress + cancel). | 🔴 Intentional gap—track doc update. |
| `_RL_REWARD` | Name of a 0-arity operator whose integer value feeds the reinforcement-learning simulation workers (`RLSimulationWorker`) as the reward signal (defaults controlled by `Simulator.rl.*` system props). | **Will not be ported.** Note RL-guided simulation removal in release notes and provide alternative suggestions (e.g., external fuzzing). | 🔴 Intentional gap—track doc update. |
| `CHECK_DEADLOCK` | Toggle deadlock detection (`TRUE`/`FALSE`). | Config influences default; CLI `-deadlock` overrides. | ✅ |
| `VIEW` | Alternative view when printing states. | Reused. | ✅ |

Additional behaviors:
- Constant overrides via `CONSTANT Foo <- 3` vs `CONSTANT Foo = 3`; must support both syntaxes.
- ModelValue declarations (e.g., `CONSTANTS Client = {c1, c2}`) require canonical ordering and collision handling identical to legacy.
- Toolbox override tables (`ALIAS`, parameter dialogs) rely on config serialization layout.

---

## Behavioral Domains & Engine Capabilities

- **State exploration modes**: Legacy supports BFS (default), DFID, and random simulation (`RunMode.MODEL_CHECK` vs `SIMULATE`) with shared CLI flags (`-simulate`, `-dfid`). Rust engine must expose identical modes and combinations, including `-dfid` + simulation restrictions (`TLC.java` lines 1734–1759). Status: 🟡 (DFID design outstanding).
- **Liveness checking**: TLC builds behavior graphs, handles fairness (`WF_/SF_`) and temporal formulas declared via `PROPERTY`. Need to port strongly-connected component algorithm, tableau handling, and fairness warnings (`SpecProcessor` fairness warnings at `SpecProcessor.java:1090`). Status: 🟡.
- **Fairness & stuttering semantics**: Support `WF_/SF_` definitions, stuttering steps, weak fairness toggles—ensuring parity in counterexample generation. Status: 🟡 (requires spec-level evaluator parity).
- **Symmetry reduction**: `SYMMETRY` config reduces state space using user-provided symmetry operator. Must mirror orbits and fingerprint canonicalization. Status: 🟡 high complexity.
- **Randomization**: `RandomGenerator` features (aril adjustment, simulation trace writing) must be replicated to maintain reproducibility with seeds. Status: 🟡 (seed math validation).
- **Fingerprint storage**: Multi-tier `FPSet` implementations (in-memory, disk, off-heap) selected based on memory flags. Rust version must offer equivalent robustness, including polynomial selection (`FP64`) and `-fpmem`. Status: 🟡 (storage design pending).
- **Checkpointing & recovery**: Legacy uses per-run directories inside `states/`, incremental fingerprint saves, resume semantics via `-recover`. Rust plan uses `sled` + chunked serde; must match frequency, file naming, and resume guard conditions (e.g., spec hash). Status: 🟡 (naming + compatibility to finalize).
- **Coverage reporting**: Output format and cost metrics described in `docs/module-coverage-statistics.md` must remain identical for tooling compatibility. Status: 🟡 (format regression tests needed).
- **Error trace handling**: Provide difftrace, TLA listing, JSON, TE spec generation, `-continue`, `-terse`, `-view` interplay; ensure counterexample format matches `Messages` catalog for parity harness. Status: ✅ (design accounted for).
- **Toolbox integration**: `-tool` message codes, `SpecTE` flows, debug adapter, `ModelInJar` packaging. Need to audit Toolbox expectations (message codes, output directory layout). Status: 🟡.
- **Debugger (DAP)**: Java TLC supports DAP with `-debugger`. Need to port or provide compatibility layer; rust version must speak same protocol (TLC-specific commands). Status: 🟡 (requires dedicated epic).
- **Spec-in-JAR**: Running models packaged inside `tla` jar resources via `ModelInJar` loader (`TLC.java:1102` onwards). Status: ✅ — `SpecSource::Archive` opens `.jar`/`.zip` bundles, rehydrates `/model` contents into temp dirs, and reuses the unified resolver for CLI/distributed flows.
- **Trace Explorer (SpecTE)**: Automatic generation of trace explorer specs on failure, output location control, and integration with Toolbox. Ensure parity in file naming and module contents. Status: 🟡.
- **Email/notification hooks**: Legacy had `MailSender` integration for `-tool` mode (observed via imports). Determine if still used; if so, replicate or deprecate with stakeholder approval. Status: 🔴 needs clarification.

---

## Spec Bundle Parity Notes

- **Legacy bundle layout**: Toolbox/cloud jobs repackage `tla2tools.jar` and drop model artifacts under `/model/`—at minimum `MC.tla`, often `MC.cfg`, all dependent `.tla` modules, plus optional `generated.properties` used to seed JVM system properties (mail targets, cloud metadata). TLC auto-detects this when the positional `SPEC` is omitted, toggles `Tool` mode, disables checkpoints, and switches the resolver to `InJarFilenameToStream` so parser/evaluator reads from the jar before falling back to the filesystem or standard library.
- **Rust parity plan**: `SpecSource::Archive` (used by CLI + distributed entry points) opens `.jar`/`.zip` archives via `zip` crate, filters contents under `/model/`, validates that `MC.tla` exists, and extracts all `/model/*.tla`/`*.cfg` (plus `generated.properties`) into a dedicated temp directory. That directory is placed at the front of the module search path, mirroring legacy resolver ordering, while standard modules remain untouched. `generated.properties` is parsed into a structured map; consumers can ignore it by default (MailSender retirement) but tooling has the metadata if reintroduced.
- **Invocation flow**: `tlc` without a positional spec attempts to load an embedded bundle (for future self-contained artifacts). Providing a `.jar`/`.zip` path binds `SpecSource::Archive`; other inputs are treated as filesystem modules. Optional `--spec-root` flag defaults to `/model/` but permits alternative roots for migration/experiments. `SpecSource` is shared with `tlc server`/resumption so distributed/cloud workflows keep a single artifact story.
- **Risk & follow-up**: Implementation requires bundler CLI tooling (e.g., `tlc bundle create`) and regression tests that diff extracted archives against known-good legacy snapshots. Track these in the tooling epic; resolver parity itself is now designed.

---

## Observability & Output

- **Progress reporting**: Legacy prints textual progress (states found, depth). Rust rewrite introduces progress bar + NDJSON but must still support baseline console summaries at run end, including TLC statistics (states found, distinct states, search depth, trivially true invariants, etc.). Status: 🟡 ensure summary format parity.
- **Final statistics**: Maintain exit banners (`The number of states`, `Progress`, `Finished in ...`). Map to existing `MP` message codes for `-tool` compatibility. Status: 🟡.
- **Warning catalog**: Ensure all `MP` warnings/errors from Java exist with same codes/messages (e.g., fairness warnings, liveness info). Status: 🔴 requires inventory of message catalog.
- **Logging toggles**: `-debug`, `-nowarning`, `-terse`, `-userFile` interplay must replicate precisely. Status: 🟡 (requires message routing design).

---

## Outstanding Questions

1. Toolbox message code mapping: gather authoritative list to ensure `-tool` parity.
2. Mail/notification support: legacy `MailSender` can be eliminated per maintainers; update plan to drop feature.
3. Trace Explorer defaults: verify generating SpecTE artifacts by default (unless `-noGenerateSpecTE`) still matches Toolbox expectations.
4. Symmetry reduction + fingerprint canonicalization: identify design owners and schedule deep dive.
5. Debugger protocol (DAP): determine contract with VSCode extension and confirm compatibility expectations.

---

## De-scoped Legacy Hooks

- `_PERIODIC`: Used to encode long-running assumptions by aborting when the named operator evaluates to `FALSE` during periodic work. The Rust TLC CLI will not implement this hook; migration guidance must explain how to replicate equivalent behavior with external supervisors (e.g., monitoring progress events and invoking `SIGINT`). Track documentation update in `docs/migration/tlc-rust.md`.
- `_RL_REWARD`: Drives reinforcement-learning simulation workers. The Rust rewrite drops RL-guided simulation, so `_RL_REWARD` (and related `Simulator.rl.*` properties) will be marked as removed. Add release notes and suggest external fuzzing frameworks if needed.
