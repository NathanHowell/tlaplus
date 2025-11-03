# TLC Telemetry Migration

This note explains how the Rust TLC binary wires structured telemetry, what
ships by default, and the explicit steps teams must take before any data ever
leaves the host. Use it as the canonical reference when updating onboarding
docs or answering privacy/compliance reviews.

## Defaults
- Telemetry is **local-only** unless callers opt in to another mode.
- Every run writes JSON-formatted spans to `logs/tlc-trace.json` (rotated with
  timestamps once the real CLI lands).
- All sinks pass through the redaction layer that strips spec/module
  identifiers and other potentially sensitive labels before they are written or
  streamed.
- The subscriber honors `TLC_LOG` for `EnvFilter` directives (defaults to
  `info`) and picks up `TLC_ENVIRONMENT` as the OpenTelemetry
  `deployment.environment` attribute.

## Sink Modes
| Mode | How to select | What happens |
|------|---------------|--------------|
| `local` (default) | No CLI flag / env override | Spans are persisted to `logs/tlc-trace.json` only. |
| `json` | `--telemetry json` | Local file plus JSON events mirrored to stdout (useful for piping into other tooling). |
| `otlp` | `--telemetry otlp` | Local file remains; exporter streams sanitized spans to the configured OTLP endpoint. |

If the CLI sees an unknown mode it will exit with usage help and leave telemetry
disabled rather than guessing.

## Remote Export Opt-In
Remote collection requires **both** an explicit mode selection and an explicit
endpoint:

1. Supply `--telemetry otlp` (or the future config/environment equivalent).
2. Point TLC at a collector using either `--otlp-endpoint <https://collector:4317>`
   or the environment variable `TLC_OTLP_ENDPOINT`.

Without a usable endpoint the exporter will not install and the process logs a
warning before falling back to local-only mode.

### Optional Settings
- Once the CLI exposes `--otlp-header key=value` flags (or structured config),
  use them to populate authenticated requests. They land in
  `TelemetryConfig::otlp_headers`.
- Set `TLC_OTLP_TIMEOUT_SECS` (planned) or use CLI options once they land to
  raise/lower the default 10s request timeout.

## Validation Checklist
Run these checks before enabling remote export in a new environment:

- `cargo test -p tlc-integration-tests --test telemetry_defaults`  
  Confirms defaults keep instrumentation local and continue redacting sensitive
  fields.
- `TLC_OTLP_ENDPOINT=https://collector:4317 cargo run -p tlc -- run --telemetry otlp --spec ... --config ...`  
  Observes that the collector receives spans while the local log remains intact.
- Stop exporting and confirm the binary returns to local mode simply by removing
  the flag or environment variable.

## FAQ
- **Does telemetry still write locally when OTLP is enabled?** Yes. The local
  JSON file stays on disk for debugging and auditing even after remote export is
  activated.
- **How do we tag spans per environment?** Set `TLC_ENVIRONMENT` (for example
  `ci`, `staging`, or `production`) before invoking TLC.
- **What about PII in attributes?** The redaction layer removes spec/module
  identifiers by default. Additional custom attributes must avoid raw user data;
  future tasks will expose allow/deny lists if needed.

Keep this document updated as new sinks or configuration knobs are introduced so
operational teams always have a single up-to-date reference.
