//! Telemetry scaffolding for tracing subscribers and OTLP integration.

use anyhow::Result;
use opentelemetry::global;
use serde::{Deserialize, Serialize};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    pub service_name: String,
    pub enable_otlp: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "tlc".to_string(),
            enable_otlp: false,
        }
    }
}

/// Initialize a tracing subscriber with optional OTLP export disabled by default.
pub fn init_tracing(config: TelemetryConfig) -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact();

    let registry = Registry::default().with(env_filter).with(fmt_layer);

    if config.enable_otlp {
        let tracer = global::tracer(config.service_name.clone());
        let otlp = tracing_opentelemetry::layer().with_tracer(tracer);
        registry.with(otlp).try_init()?;
    } else {
        registry.try_init()?;
    }

    let _ = serde_json::to_string(&config)?;
    tlc_util::initialize_runtime()?;
    tracing::debug!(
        service = config.service_name,
        remote = config.enable_otlp,
        "telemetry initialized"
    );
    Ok(())
}
