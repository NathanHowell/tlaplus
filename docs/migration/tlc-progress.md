# TLC Progress Output Migration

This note explains how the Rust-native `tlc` binary renders exploration
progress, how to select between interactive and machine-readable modes, and
what downstream tooling such as the TLA\+ Toolbox must do to remain compatible.
Keep it updated as progress-related flags or schemas evolve.

## Modes at a Glance
- **TTY progress (`tty`)** &mdash; Uses `indicatif` to render an animated bar with
  explored state counts, completion percentage, elapsed time, throughput, and an
  optional ETA. Color output tracks the terminal theme and honors the
  `--no-color` flag for monochrome environments.
- **NDJSON progress (`ndjson`)** &mdash; Emits one JSON object per line to stdout,
  mirroring the same metrics as the TTY view and designed for log pipelines,
  Toolbox integrations, and CI dashboards.

The CLI defaults to `tty` but automatically falls back to NDJSON when stdout is
not a TTY (for example when piping to `jq` or redirecting to a file). You can
override the choice explicitly with `--progress tty|ndjson`.

```bash
# Explicit TTY rendering (fails over to ndjson if stdout is not a terminal)
tlc run --spec specs/Main.tla --config model/MC.cfg --progress tty

# Force NDJSON even when running interactively
tlc run --spec specs/Main.tla --config model/MC.cfg --progress ndjson | jq .
```

## NDJSON Contract

Every emitted line is valid JSON following the schema enforced by
`ProgressEvent` in `src/progress/src/ndjson.rs`. All numeric fields fit within
native JSON number ranges except for identifiers and the total state count,
which are string encoded to avoid precision loss.

```json
{
  "event_id": "01J0Y6M4F2A5B7C8D9E0F1GHJK",
  "run_id": "01J0Y6M4F2A5B7C8D9E0F1GHJM",
  "timestamp": "2025-11-02T03:25:45.123Z",
  "states_explored": "18446744073709551616",
  "percent_complete": 62.5,
  "throughput_eps": 125000.0,
  "workers_active": 15,
  "eta_seconds": 5400
}
```

Contract highlights:
- `event_id` / `run_id` &mdash; ULIDs encoded as strings. Consumers should treat
  them as opaque identifiers.
- `states_explored` &mdash; Stringified `u128` so very large counts round-trip
  safely. Parse as arbitrary-precision integers where possible.
- `eta_seconds` &mdash; Optional. Omitted when the engine cannot produce a
  stable estimate.
- Each event flushes immediately with a trailing newline, enabling streaming
  parsers to process updates as they arrive.

The integration test `tests/integration/progress_ndjson.rs` guards this schema.
Update the test and this document together whenever fields change.

## Toolbox and Automation Guidance

- **Toolbox default**: Continue to launch TLC with `--progress ndjson`. The
  Toolbox should parse the schema above and surface throughput/coverage in its
  UI. Because stdout redirect triggers automatic NDJSON fallback, existing
  Toolbox launchers that pipe the output need no flag changes, but explicitly
  passing `--progress ndjson` guards against future defaults.
- **CI pipelines and log shippers**: Consume the NDJSON stream directly or pipe
  it through tooling such as `jq`, `fluent-bit`, or `Vector`. Remember to treat
  `states_explored` as a string when mapping into numeric types.
- **Human-in-the-loop sessions**: When TLC is spawned from wrappers that capture
  stdout (e.g., `tmux` logging panes), force `--progress tty` if you want the
  interactive bar and ensure the sink is a real terminal. Otherwise expect the
  NDJSON stream.
- **Color handling**: The `tty` renderer honors terminal color support
  automatically. Set `--no-color` (or the corresponding configuration knob) in
  environments that require monochrome output, and update Toolbox integrations
  if they rely on ANSI codes.

## Checklist Before Shipping Progress Changes

- Run `cargo test -p tlc-progress` and the integration suite to confirm the
  NDJSON schema and TTY renderer behavior remain intact.
- Verify progress mode detection by redirecting stdout to both a terminal and a
  file:
  - `tlc run ... > /tmp/run.ndjson` → NDJSON lines
  - `tlc run ...` (interactive shell) → TTY bar
- Update this document and the CLI contract (`specs/001-rewrite-tlc/contracts/cli.md`)
  whenever new fields, flags, or color-handling semantics change.
