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
| `-config file` | Load `.cfg` or inline config; defaults to `SPEC.cfg`. | `RunConfiguration` resolves a `ConfigSource` from the active `SpecSource`: filesystem specs look for `${spec}.cfg`; archives probe `/model/MC.cfg`; monolithic `SPEC.tla` can contain an embedded `----- CONFIG SPEC -----` section. Explicit `-config` accepts `.cfg`, bare names, or inline `.tla` bundles; path resolution goes through the same resolver chain used for modules. The Rust `config` crate re-implements `ModelConfig` semantics (constants, overrides, module-scoped values, CHECK_DEADLOCK default, ALIAS, SYMMETRY, etc.) atop the shared TLA+ lexer so comments/tokens match Java. It preserves raw constant text for re-serialization, maps parse faults to legacy `MP` codes, and rejects deprecated `_PERIODIC`/`_RL_REWARD` with targeted diagnostics. | ✅ Parser design locked; implementation tracked under config parity epic. |
| `-workers num|auto` | Set worker threads (default 1; `auto` uses logical cores). | Implement in `engine::scheduler` with `rayon` pool sizing & `auto` policy; enforce `-debugger` forcing 1 worker. | ✅ |
| `-checkpoint minutes` | Minutes between background checkpoints (default 30). | Mirror Java scheduler: accept non-negative integer minutes->ms (0 disables), preserve sentinel default (30 min + 42 ms) to detect explicit opt-in, let distributed runs override to 0, and schedule the first checkpoint only after a full interval (subject to progress tick jitter). | ✅ |
| `-metadir path` | Override state / metadata directory (default `SPEC/states`). | Mirror Java by creating per-run subdirs under the supplied root, respecting relative paths, reuse explicit recovery dirs verbatim, and funnel all state/fingerprint artifacts plus temp files through that base. | ✅ |
| `-recover id` | Resume from checkpoint ID. | Ported via `tlc resume` command; ensure ID naming matches legacy to keep automation working. | ✅ |
| `-cleanup` | Remove states dir before run. | Add pre-run cleanup option toggling storage manager. | ✅ |
| `-continue` | Do not halt on invariant violation. | CLI flag sets `RunPolicy::ContinueOnViolation`. | ✅ |
| `-deadlock` | Skip deadlock checking (equivalent to `CHECK_DEADLOCK=FALSE`). | CLI flag toggles engine behavior & overrides config default. | ✅ |
| `-coverage minutes` | Emit module coverage data at interval + final report. | Keep minute→ms parsing, reuse the progress ticker to schedule `CostModel` snapshots, stream the exact `MP` message catalog (`TLC_COVERAGE_*`) with eval/new-state counts and cost metrics, warn after 5 min of collection, and share collectors with simulation + final summary. | ✅ |
| `-postCondition mod!oper` | Evaluate constant-level operator at end. | Accept repeated `mod!op` arguments, ensure modules are auto-extended, allow optional operator rebindings for built-in exporters, and after each run (success or failure) validate via `Tool.checkPostCondition`, passing `TLCExt!CounterExample` on failure. | ✅ |
| `-difftrace` | Print only differences between successive states. | Mirror legacy trace printer behavior in Rust `trace` module. | ✅ |
| `-dumpTrace format file` | Dump error trace as TLA or JSON to file; supports multiple invocations. | Implement writer supporting `tla`/`json` (pluggable). Ensure file path relative to spec dir. | ✅ |
| `-inv expr` | Evaluate additional invariant expression. | CLI maps to `RunConfiguration.extra_invariants`; reused in engine. | ✅ |
| `-invlevel n` | Stop after finding a trace of length `n` unless `-continue`. | Synthesize runtime invariant `TLCGet("level") < n` via spec augmentation (auto-extends `TLC`/`Naturals`), reuse legacy CLI parsing + message codes, and propagate continuation semantics/tracing identical to Java. | ✅ Level cap parity detailed below. |
| `-debug` | Enable verbose internal diagnostics. | Map to `tracing` level + additional debug assertions. | ✅ |
| `-dump [format] file` | Dump reachable states; optional DOT with modifiers (`colorize`, `actionlabels`, `constrained`). | `StateDumpService` mirrors legacy `StateWriter`: plain `.dump` stream (`State N:` blocks), DOT exporter with `colorize`/`actionlabels`/`stuttering`/`constrained`/`snapshot`/`strict` toggles, `${metadir}` placeholder expansion, and `class,<impl>` plugin dispatch (bridged to JVM via JNI shim for existing writers). Writers run behind an event queue to preserve ordering and feed liveness `_liveness.dot` captures when DOT is active. | ✅ Design captured in Dump Export Parity Notes. |
| `-fp N` | Pick specific irreducible polynomial for fingerprints. | Mirror the legacy `FP64` polynomial catalog (131 entries), default to randomized selection unless overridden, keep legacy diagnostics, surface the chosen index in telemetry, and share it with workers/checkpoints so fingerprints stay stable. | ✅ Design captured in Fingerprint Polynomial Parity Notes. |
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
| `POSTCONDITION` | Evaluate zero-arity operator after search completes; Toolbox writes one per line. | Shares evaluator with CLI options, requires operator defined in spec/model, and errors mirror `ASSUME` failures (`TLC_ASSUMPTION_*`). | ✅ |
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

## Config Loader Parity Notes

- **Source resolution**: `RunConfiguration` delegates to `ConfigSource::resolve(spec_source, cli_override)` which mirrors Java semantics. If the user omits `-config`, we probe (in order) the explicit archive payload (`/model/MC.cfg`), a sibling `${spec}.cfg` next to the primary module, and finally a monolithic `SPEC.tla` section marked with `----- CONFIG SPEC -----`. CLI `-config foo` accepts bare basenames (`foo`), explicit `.cfg` paths, or `.tla` monoliths; relative paths are interpreted against the launch CWD unless `--spec-root` (from the spec bundle flow) overrides it.
- **Parsing strategy**: A dedicated `config::parser` crate reuses the shared TLA+ lexer (same tokenization as SANY) so comments, Unicode escapes, and Toolbox-generated spacing stay legal. The recursive-descent parser follows `ModelConfig`’s grammar exactly: single- and multi-entry `CONSTANT(S)` blocks, `<-` expression overrides (stored as TLA AST strings), `=` literal assignments (numbers, strings, booleans, finite sets, or model values), module-scoped overrides via `<-[Mod]`, and module-specific constant lists via `=[Mod]`. During parsing we capture both typed data (for the engine) and a `raw_constants` list preserving the original text block for round-tripping back to UIs.
- **Keyword coverage**: All supported keywords (`SPECIFICATION`, `INIT`, `NEXT`, `SYMMETRY`, `CONSTRAINT(S)`, `ACTION_CONSTRAINT(S)`, `INVARIANT(S)`, `PROPERTY(IES)`, `ALIAS`, `VIEW`, `POSTCONDITION`, `CHECK_DEADLOCK`, etc.) map to strongly typed fields on `RunConfiguration`. Deprecated hooks (`_PERIODIC`, `_RL_REWARD`) still lex but surface as explicit “removed in TLC Rust” diagnostics that reference the migration doc.
- **Error fidelity**: Parse failures emit structured `ConfigError` values that carry the legacy `EC.CFG_*` message codes so `-tool` mode and the Toolbox continue to receive identical `MP` output. This includes duplicate keyword detection, missing identifiers, invalid constants, and bad boolean literals for `CHECK_DEADLOCK`. File resolution failures also re-use `EC.CFG_ERROR_READING_FILE` with the same wording.
- **Resolver integration**: The parser operates on streams from the unified resolver stack (`SpecSource`-aware filesystem/archive resolver + standard modules). This keeps toolbox library paths, temp directories (for jars), and override modules (`Foo.class` compiles) working without special cases.
- **Serialization**: Downstream writers (migration tooling, future `tlc bundle` command) consume a `ConfigDoc` surface that can regenerate `.cfg` files byte-for-byte when no semantic edits occur—matching Toolbox expectations for preserving user formatting and comments.


## Checkpoint Scheduler Parity Notes

- **CLI validation**: Legacy `TLC.java` parses `-checkpoint` as a non-negative integer minute count, multiplies it by 60_000, and errors on missing or negative input. The Rust CLI keeps the same guardrails and reuses the legacy command-line error codes so Toolbox automation sees consistent failures.
- **Default sentinel**: `TLCGlobals.DEFAULT_CHECKPOINT_DURATION` equals `30 * 60 * 1000 + 42`, letting TLC detect when users explicitly changed the interval (`chkptExplicitlyEnabled`). We will carry forward the +42 ms sentinel so post-run cleanup only snapshots when the user opted in.
- **Scheduling cadence**: `TLCGlobals.doCheckPoint()` seeds `lastChkpt` at process start and only returns true once `now - lastChkpt >= duration`. Because it runs on the progress-loop tick (`progressInterval`, default 60 seconds), the first checkpoint fires after roughly one full interval and later checkpoints adhere to the same cadence with at most one progress tick of jitter. The Rust scheduler will mirror this by running the check in the progress worker and updating its timer only after a committed checkpoint.
- **Forced checkpoints**: Legacy `forceChkpt()` flips a flag that makes the next `doCheckPoint()` return true without advancing `lastChkpt`, ensuring manual or JMX-triggered checkpoints do not reset the cadence. We'll replicate that behavior so forced snapshots do not delay the next scheduled checkpoint.
- **Distributed/bundle overrides**: `TLCApp` sets `chkptDuration = 0` for distributed runs and for in-jar models, disabling background checkpoints to avoid expensive network or archive writes. The Rust CLI will do the same when server/resume pathways are active or when `SpecSource::Archive` handles a bundle without an explicit positional `SPEC`.

---

## Metadir & Storage Layout Notes

- **Default structure**: `FileUtil.makeMetaDir` points the base to `${specDir}/states` when `-metadir` is absent (with `specDir` derived from the main module’s directory or the launch CWD). Each run gets a timestamped child directory (format `yy-MM-dd-HH-mm-ss.SSS` by default) created via `createExclusiveDirectoryWithApproximateName`, which guarantees uniqueness even when multiple TLC instances start within the same millisecond.
- **CLI handling**: `-metadir` requires an argument, appends the platform separator as parsed, and performs no other normalization. Relative paths remain relative to the process working directory (after the absolute-path clutch sets `ToolIO.setUserDir`), while absolute paths are respected verbatim.
- **Recovery semantics**: When `-recover` supplies a checkpoint directory, `makeMetaDir` simply returns that path without creating a new subfolder, letting TLC continue inside the existing artifact tree. The same rule applies to Toolbox-generated resume flows.
- **Artifact routing**: The selected metadir is passed to `DiskStateQueue`, `DiskByteArrayQueue`, fingerprint set factories, trace storage, and liveness stacks, so all checkpoint/state artifacts land under the chosen root. `FileUtil.createTempFile` also seeds its temp directories beneath `metaDir` when set, keeping scratch files co-located with run outputs.
- **Cleanup behavior**: `-cleanup` removes the default `states/` root before execution when not recovering, matching Java’s behavior. Runs that opted into `-metadir` are expected to manage their custom directories, so Rust will avoid deleting user-specified paths automatically.
- **Distributed parity**: `TLCApp` accepts the same flag and forwards the resulting base to distributed workers; the Rust CLI/server pair will share the same configuration flow so cloud and local runs keep identical directory layouts.

---

## Coverage Reporting Notes

- **Interval semantics**: `-coverage N` stores `coverageInterval = N * 60_000` (minutes). Progress polling runs every `progressInterval` (default 60 s via system property `tlc2.TLC.progressInterval`), and TLC divides the two to decide how many polls to skip. Integer division means `N < progressInterval_minutes` collapses to zero, yielding coverage output on every poll; `0` is accepted and treated the same. Rust will preserve this quotient logic so long-standing scripts stay compatible.
- **Data collection**: Enabling coverage wraps each evaluated operator in a `CostModel` tree. Variables record distinct values via HyperLogLog sketches, init/next/invariant/constraint actions report `evaluations:successes`, and nested expressions emit their evaluation counts plus optional allocation cost when they build compound values. The CLI prints via the `MP` catalog codes `TLC_COVERAGE_*` (`_START`, `_VAR`, `_INIT`, `_NEXT`, `_VALUE[_COST]`, `_END[_OVERHEAD]`, etc.) so Toolbox parsers can consume structured data. Rust’s evaluator must mirror this instrumentation, including cost accounting for collections.
- **Output cadence**: During both model checking and simulation, coverage snapshots fire on the progress loop, then again inside the final `printSummary()`/`Simulator` shutdown so the last report always includes totals. If coverage has been active for more than five minutes, TLC emits `TLC_COVERAGE_END_OVERHEAD` to remind users of the runtime impact; shorter runs just print `TLC_COVERAGE_END`. We’ll reproduce the same heuristic by comparing wall-clock runtime to a 5-minute threshold.
- **Guard rails**: Coverage only runs when actions exist and `coverageInterval >= 0`; `-coverage` is ignored in distributed TLC (`TLCApp` currently comments out parsing) but Toolbox passes it for local runs. Rust will short-circuit the collectors when coverage is disabled to avoid the ~40 % overhead described in the legacy comments.
- **Spec interaction**: Coverage statistics respect implied inits/actions when the optional system property `tlc2.tool.coverage.CostModelCreator.implied=true` is set, and they are symmetry-blind (assume uniform coverage within an orbit). Rust should retain the property hook and document symmetry caveats alongside the legacy behavior.

---

## Postcondition Notes

- **Sources & parsing**: CLI `-postCondition module!Operator` (case-insensitive) may appear multiple times; the parser validates `module!operator` without dots, collects them into `ParameterizedSpecObj.POST_CONDITIONS`, and `ParameterizedSpecObj` auto-extends the root module so the referenced module is available. Config `POSTCONDITION Foo` contributes the same operator list by name, and both sources are merged before spec elaboration.
- **Built-in exporters**: `-dumpTrace` piggybacks on the same pipeline by registering postconditions that live in helper modules (e.g., `_JsonTrace`, `_TLCTrace`). Each registration can redefine helper operator constants (such as `_TLCTraceFile`) to the user-supplied output path via `PostCondition.redefinitions`. Rust must keep this hook so new formats can be delivered as pure TLA modules.
- **Evaluation timing**: After safety checking (and final liveness check) succeeds, `ModelChecker` calls `Tool.checkPostCondition()`. On any violation or init failure, the engine invokes `checkPostConditionWithCounterExample`, supplying a `TLCExt!CounterExample` record that encodes the (alias-mapped) trace; `Worker` and `LiveCheck` trigger this in error paths, and simulation mode does the same. The code sets `EvalControl.Const` so evaluation happens in the “constant context” used for ASSUME statements.
- **Return codes**: A postcondition that evaluates to `FALSE` surfaces as `EC.TLC_ASSUMPTION_FALSE`; evaluation failures use `EC.TLC_ASSUMPTION_EVALUATION_ERROR`, matching ASSUME diagnostics so Toolbox tooling stays compatible.
- **Constraints**: Operators must be zero-arity and constant-level. Config validation rejects missing or non-operator references (`EC.TLC_CONFIG_SPECIFIED_NOT_DEFINED`, `EC.TLC_CONFIG_ID_REQUIRES_NO_ARG`). CLI syntax prevents dotted names to avoid parameterized instantiations; extending modules via `ModulePointer.getRelatives().addExtendee` mirrors Java behavior when ported.
- **Counterexample helpers**: `TLCExt!CounterExample` exposes the trace as a record of `state`/`action` tuples, and `CounterExample.toTrace()` yields a tuple for TE specs. Any postcondition that depends on the violation context (e.g., writing JSON) must import `TLCExt` and will continue to work when `checkPostConditionWithCounterExample` supplies the value.
- **Concurrency & retries**: With `-continue`, every discovered violation triggers the postcondition check (with its corresponding counterexample record). Rust should run postconditions after each failing behavior and after the run finishes to maintain parity.

## Dump Export Parity Notes

- **CLI parsing & file placement**: `tlatools/org.lamport.tlatools/src/tlc2/TLC.java:619-704` handles `-dump`, appends `.dump`/`.dot` when missing, expands `${metadir}` once `FileUtil.makeMetaDir` returns, and enforces the same error strings for missing arguments. `dot,<mods>` and `class,<impl>` share the flag and remain case-insensitive.
- **Plain writer semantics**: `tlc2/util/StateWriter.java` prints `State N:` blocks for unseen states only (guarded by `IStateWriter.IsSeen` flags) and honours `TLCGlobals.printDiffsOnly` for diff-style output. Rust’s plain exporter must maintain the sequential counter and string format so diff tools and existing scripts still match.
- **DOT exporter & modifiers**: `tlc2/util/DotStateWriter.java` implements `colorize`, `actionlabels`, `stuttering`, `snapshot`, and `strict` switches plus Graphviz rank bookkeeping. Action labels use `Action.getInvocationSignature()` with escaping, stuttering edges render dashed, and `strict` deduplicates edges via an XOR fingerprint set while emitting `strict digraph` headers. Rust must emit identical Graphviz, including the `paired12` color scheme and rank clusters.
- **Constraint visualization**: When the `constrained` modifier is present, workers feed excluded states through `writeState(..., IsNotInModel, action, predicate)` (`tlatools/org.lamport.tlatools/src/tlc2/tool/Worker.java:441-503`), which Dot writer highlights (filled lightyellow with predicate text). Rust must surface the same annotations for state/action constraints.
- **Snapshots & liveness coupling**: `snapshot` creates `<file>_snapshot.dot` snapshots after each write, and enabling DOT automatically spawns a matching liveness graph via `DotLivenessStateWriter` (`tlc2/tool/liveness/LiveCheck.java:70`, `_liveness.dot`). Flush behaviour, suffixes, and close-time legend output need to match so Toolbox visual tooling keeps working.
- **Custom writers (`class,<impl>` option)**: Java reflectively instantiates a zero-arg `IStateWriter` subclass (`TLC.java:623-636`) and hands it the full lifecycle (`writeState`, `close`). For parity we will ship an optional JNI bridge that can host legacy Java writers while exposing a native plugin registry so Rust exporters can register by name; CLI retains the `class,<FQN>` syntax and uses the same diagnostic wording when instantiation fails.
- **Execution model & testing**: State writers are invoked concurrently from worker threads; Java synchronizes each method. Rust will queue writer events behind a bounded channel per exporter to preserve order and provide backpressure. Golden-file tests mirroring `DumpAsDotTest` and `DotConstrainedTest` will lock output parity (plain dump trace counts, DOT structure, constrained highlights).

---

## Fingerprint Polynomial Parity Notes

- **Polynomial catalog**: `tlatools/org.lamport.tlatools/src/tlc2/util/FP64.java:188-382` hosts the 131-entry `Polys` array that seeds `FP64`. We will port the constants verbatim (order preserved) into `fingerprint::poly::IRREDUCIBLE_POLYS: [u64; 131]` so every legacy `-fp` index maps to the same irreducible polynomial.
- **Selection semantics**: `tlatools/org.lamport.tlatools/src/tlc2/TLC.java:226` (and the Toolbox launcher) pick a random index when `-fp` is omitted, while `-fp N` locks the index via `TLC.java:978-1007`. `tlc2/tool/distributed/TLCApp.java:286-409` mirrors the same logic for distributed coordinators. Rust’s CLI keeps the `-fp` flag (default `None` → random index) and emits the chosen index in the startup telemetry (`fpidx`) so Toolbox and scripts continue to recover it.
- **Diagnostics & CLI parity**: Inputs outside `[0, FP64.Polys.length)` trigger `EC.WRONG_COMMANDLINE_PARAMS_TLC` with the exact wording `Error: The number for -fp must be between 0 and 130 (inclusive).` (same for distributed runs). We’ll surface the same message code and phrasing via the Rust error catalog and guard against missing arguments (`Error: A number for -fp is required...`).
- **Runtime propagation & resume**: `TLC.java:1257-1314` initializes `FP64` before the engine starts; the active polynomial is exported via `AbstractChecker.createConfig` (`tlc2/tool/AbstractChecker.java:672-708`) and distributed servers/ workers exchange it through `TLCServer#getIrredPolyForFP` and `TLCWorker` (`tlc2/tool/distributed/TLCWorker.java:361-364`). The Rust runtime will persist both `fp_index` and the 64-bit polynomial in the run manifest/checkpoint header so `tlc resume` and remote workers can rehydrate the same fingerprint kernel even when the CLI omits `-fp`.
- **Rust mapping & validation**: The new `fingerprint` crate will expose `Fingerprinter::with_index(idx)` / `::random()` that drive the shared `IRREDUCIBLE_POLYS` table and rebuild the byte-mod table exactly like `FP64.Init`. Golden tests will compare known fingerprints (e.g., `Extend(New(), "abc")`) for several indices against values generated by the Java engine, and integration tests will assert `fpidx` printing/tool-mode telemetry matches legacy output.

---

## Level Cap (`-invlevel`) Notes

- **CLI parsing & diagnostics**: `tlatools/org.lamport.tlatools/src/tlc2/TLC.java:514-533` parses the flag, requires the next argument to be an integer, and reuses the generic `Error: An integer for -invlevel required...` diagnostic when parsing fails. Negative or zero values are accepted and simply make the generated invariant fail earlier. Rust CLI will reuse this validation path and surface the same error wording/message code.
- **Generated invariant shape**: The Java CLI appends a `RuntimeInvariantTemplate` that instantiates `TLC` and `Naturals`, emitting `LET _T == INSTANCE TLC _N == INSTANCE Naturals IN _N!<(_T!TLCGet("level"), n)` (`TLC.java:526-533`). This ensures the invariant name is the synthesized `__DebuggerExpr__k` operator produced by `TLCDebuggerExpression.process` and keeps Toolbox-visible names identical. We will synthesize the same expression string (with leading underscores) to preserve naming and semantics.
- **Module extension plumbing**: `ParameterizedSpecObj.findOrCreateParsedUnit` auto-extends any modules declared on the invariant template (`ParameterizedSpecObj.java:70-104`), so the inserted invariant can resolve `TLCGet`/`<`. The Rust spec builder must register the same extendees before elaboration; otherwise the parser would reject the generated expression.
- **Level semantics**: State levels start at 1 for initial states and increment when successors adopt their predecessor’s level + 1 (`TLCState.java:24-88`, `setPredecessor`). `TLCGet("level")` returns 0 inside constant/init contexts and the state’s level elsewhere (`TLCGetSet.java:360-382`). This matches the legacy expectation that a violation at level `n` yields a trace of length `n`.
- **Evaluation & continuation behavior**: Invariant checks only run for unseen states; violations call `doNextSetErr` unless `TLCGlobals.continuation` is true, in which case TLC logs the error but continues exploration (`ModelChecker.java:399-508`). Rust must wire the synthesized invariant through the standard invariant pipeline so `-continue` keeps working the same way.
- **Reporting & exit status**: The failure path emits `EC.TLC_INVARIANT_VIOLATED_BEHAVIOR` with the generated name (`MP.java:520-523`) and the final summary still prints `EC.TLC_SEARCH_DEPTH` using the trace level (`ModelChecker.java:874-885`). Regression tests such as `InvParameterizedATest` demonstrate the expected exit code (`ExitStatus.VIOLATION_SAFETY`) and depth reporting. Our implementation will reuse these message codes and surface telemetry through the same reporting hooks.

---

## Behavioral Domains & Engine Capabilities

- **State exploration modes**: Legacy supports BFS (default), DFID, and random simulation (`RunMode.MODEL_CHECK` vs `SIMULATE`) with shared CLI flags (`-simulate`, `-dfid`). Rust engine must expose identical modes and combinations, including `-dfid` + simulation restrictions (`TLC.java` lines 1734–1759). Status: 🟡 (DFID design outstanding).
- **Liveness checking**: TLC builds behavior graphs, handles fairness (`WF_/SF_`) and temporal formulas declared via `PROPERTY`. Need to port strongly-connected component algorithm, tableau handling, and fairness warnings (`SpecProcessor` fairness warnings at `SpecProcessor.java:1090`). Status: 🟡.
- **Fairness & stuttering semantics**: Support `WF_/SF_` definitions, stuttering steps, weak fairness toggles—ensuring parity in counterexample generation. Status: 🟡 (requires spec-level evaluator parity).
- **Symmetry reduction**: `SYMMETRY` config reduces state space using user-provided symmetry operator. Must mirror orbits and fingerprint canonicalization. Status: 🟡 high complexity.
- **Randomization**: `RandomGenerator` features (aril adjustment, simulation trace writing) must be replicated to maintain reproducibility with seeds. Status: 🟡 (seed math validation).
- **Fingerprint storage**: Multi-tier `FPSet` implementations (in-memory, disk, off-heap) selected based on memory flags. Rust version must offer equivalent robustness, including polynomial selection (`FP64`) and `-fpmem`. Status: 🟡 (storage layering pending; polynomial selection parity captured above).
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
