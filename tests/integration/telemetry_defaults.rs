use std::{fs, thread, time::Duration};

use anyhow::Result;
use serde_json::Value;
use tlc_telemetry::TelemetryConfig;
use tlc_util::TelemetryMode;

#[test]
fn telemetry_defaults_are_local_and_redacted() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;

    let mut config = TelemetryConfig::default();
    config.log_directory = temp_dir.path().to_path_buf();
    config.log_filename = "telemetry.json".to_string();
    config.mode = TelemetryMode::Local;
    config.env_filter = Some("info".to_string());
    config.default_directives = "info".to_string();

    let guard = tlc_telemetry::init_tracing(config)?;
    assert!(
        !guard.otlp_active(),
        "default telemetry must not activate OTLP exporters"
    );

    let span = tracing::info_span!(
        "tlc.telemetry.test",
        spec_name = "SampleSpec",
        module = "ModuleA"
    );
    let _span_guard = span.enter();

    tracing::info!(
        spec_path = "/tmp/SampleSpec.tla",
        primary_module = "ModuleA",
        "telemetry defaults test event"
    );

    drop(_span_guard);
    drop(span);
    drop(guard);

    // Give the non-blocking writer a moment to flush contents to disk.
    thread::sleep(Duration::from_millis(25));

    let log_path = temp_dir.path().join("telemetry.json");
    let raw = fs::read_to_string(&log_path)?;

    assert!(
        raw.contains("telemetry defaults test event"),
        "expected event to be written to the local telemetry file"
    );
    assert!(
        !raw.contains("SampleSpec"),
        "redaction should remove spec identifiers from output"
    );
    assert!(
        !raw.contains("ModuleA"),
        "redaction should remove module identifiers from output"
    );
    assert!(
        raw.contains("[REDACTED]"),
        "redacted placeholders should appear in place of identifiers"
    );

    let event_line = raw
        .lines()
        .rev()
        .find(|line| line.contains("telemetry defaults test event"))
        .expect("expected telemetry event line");
    let parsed: Value = serde_json::from_str(event_line)?;

    let fields = parsed
        .get("fields")
        .and_then(|value| value.as_object())
        .expect("fields object present");
    assert_eq!(
        fields.get("spec_path").and_then(|value| value.as_str()),
        Some("[REDACTED]")
    );
    assert_eq!(
        fields
            .get("primary_module")
            .and_then(|value| value.as_str()),
        Some("[REDACTED]")
    );
    assert_eq!(
        fields.get("message").and_then(|value| value.as_str()),
        Some("telemetry defaults test event")
    );

    if let Some(span_obj) = parsed.get("span").and_then(|value| value.as_object()) {
        if let Some(value) = span_obj.get("spec_name").and_then(|value| value.as_str()) {
            assert_eq!(value, "[REDACTED]");
        }
        if let Some(value) = span_obj.get("module").and_then(|value| value.as_str()) {
            assert_eq!(value, "[REDACTED]");
        }
    }

    if let Some(span_list) = parsed.get("spans").and_then(|value| value.as_array()) {
        for entry in span_list.iter().filter_map(|value| value.as_object()) {
            if let Some(value) = entry.get("spec_name").and_then(|value| value.as_str()) {
                assert_eq!(value, "[REDACTED]");
            }
            if let Some(value) = entry.get("module").and_then(|value| value.as_str()) {
                assert_eq!(value, "[REDACTED]");
            }
        }
    }

    Ok(())
}
