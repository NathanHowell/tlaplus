//! Telemetry scaffolding for tracing subscribers and OTLP integration.
use std::{
    collections::HashMap,
    env, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use opentelemetry::{global, KeyValue};
use opentelemetry_otlp::{HttpExporterBuilder, WithExportConfig};
use opentelemetry_sdk::{trace, Resource};
use tracing::level_filters::LevelFilter;
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{
    fmt::writer::BoxMakeWriter,
    layer::{Identity, Layer, SubscriberExt},
    util::SubscriberInitExt,
    EnvFilter, Registry,
};

use tlc_util::TelemetryMode;

const DEFAULT_SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Configuration for installing TLC telemetry subscribers.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Human-readable service name recorded in exported traces.
    pub service_name: String,
    /// Optional semantic version recorded alongside spans.
    pub service_version: Option<String>,
    /// Execution environment tag (e.g., "dev", "ci").
    pub environment: Option<String>,
    /// Which telemetry sink to activate.
    pub mode: TelemetryMode,
    /// Explicit directive string for the [`EnvFilter`] layer.
    pub env_filter: Option<String>,
    /// Fallback directives used when no explicit filter is provided.
    pub default_directives: String,
    /// Directory for on-disk JSON traces.
    pub log_directory: PathBuf,
    /// Filename for JSON trace output within [`log_directory`](Self::log_directory).
    pub log_filename: String,
    /// Optional OTLP collector endpoint when `mode = TelemetryMode::Otlp`.
    pub otlp_endpoint: Option<String>,
    /// HTTP headers forwarded to the OTLP exporter.
    pub otlp_headers: Vec<(String, String)>,
    /// Timeout applied to OTLP export requests.
    pub otlp_timeout: Duration,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "tlc".to_string(),
            service_version: Some(DEFAULT_SERVICE_VERSION.to_string()),
            environment: env::var("TLC_ENVIRONMENT").ok(),
            mode: TelemetryMode::Local,
            env_filter: env::var("TLC_LOG").ok(),
            default_directives: "info".to_string(),
            log_directory: PathBuf::from("logs"),
            log_filename: "tlc-trace.json".to_string(),
            otlp_endpoint: env::var("TLC_OTLP_ENDPOINT").ok(),
            otlp_headers: Vec::new(),
            otlp_timeout: Duration::from_secs(10),
        }
    }
}

/// Guard object holding resources that must remain alive while telemetry is active.
#[derive(Debug, Default)]
pub struct TelemetryGuard {
    _file_guard: Option<WorkerGuard>,
    otlp_active: bool,
}

impl TelemetryGuard {
    /// Returns a guard with telemetry disabled, suitable for fallback scenarios.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Indicates whether an OTLP exporter was activated.
    pub fn otlp_active(&self) -> bool {
        self.otlp_active
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if self.otlp_active {
            global::shutdown_tracer_provider();
        }
    }
}

/// Initialize tracing subscribers according to the supplied [`TelemetryConfig`].
///
/// Returns a [`TelemetryGuard`] that must be kept alive until telemetry shutdown.
pub fn init_tracing(config: TelemetryConfig) -> Result<TelemetryGuard> {
    let directives = config
        .env_filter
        .clone()
        .unwrap_or_else(|| config.default_directives.clone());
    let env_filter = EnvFilter::try_new(directives.clone()).with_context(|| {
        format!(
            "invalid telemetry directives '{}'; see `EnvFilter` syntax",
            directives
        )
    })?;

    ensure_directory(&config.log_directory)?;

    let file_appender = rolling::never(&config.log_directory, &config.log_filename);
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_target(false)
        .with_writer(file_writer)
        .with_filter(LevelFilter::TRACE);

    let stdout_writer = BoxMakeWriter::new(|| io::stdout());
    let console_filter = if config.mode == TelemetryMode::Json {
        LevelFilter::TRACE
    } else {
        LevelFilter::OFF
    };
    let console_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_target(false)
        .with_writer(stdout_writer)
        .with_filter(console_filter);

    let mut otlp_active = false;
    let otlp_layer: Box<dyn Layer<Registry> + Send + Sync> =
        match build_otlp_layer(&config).transpose()? {
            Some(layer) => {
                otlp_active = true;
                Box::new(layer)
            }
            None => Box::new(Identity::new()),
        };

    let subscriber = Registry::default()
        .with(otlp_layer)
        .with(file_layer)
        .with(console_layer)
        .with(env_filter);
    subscriber.try_init()?;

    tracing::debug!(
        mode = ?config.mode,
        log_path = %config.log_directory.join(&config.log_filename).display(),
        otlp = otlp_active,
        "telemetry subscribers installed"
    );

    Ok(TelemetryGuard {
        _file_guard: Some(file_guard),
        otlp_active,
    })
}

fn ensure_directory(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(anyhow!("telemetry log directory must not be empty"));
    }
    fs::create_dir_all(path).with_context(|| {
        format!(
            "failed to create telemetry log directory at '{}'",
            path.display()
        )
    })
}

fn build_otlp_layer(config: &TelemetryConfig) -> Option<Result<impl Layer<Registry>>> {
    if config.mode != TelemetryMode::Otlp {
        return None;
    }

    let endpoint = config
        .otlp_endpoint
        .clone()
        .or_else(|| env::var("TLC_OTLP_ENDPOINT").ok())
        .ok_or_else(|| anyhow!("OTLP telemetry requested but no endpoint provided"));

    Some(endpoint.and_then(|endpoint| {
        let resource = build_resource(config);
        let exporter = build_otlp_exporter(config, endpoint)?;
        let _tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_trace_config(trace::Config::default().with_resource(resource))
            .with_exporter(exporter)
            .install_simple()
            .context("failed to install OTLP exporter")?;

        Ok(tracing_opentelemetry::layer())
    }))
}

fn build_otlp_exporter(config: &TelemetryConfig, endpoint: String) -> Result<HttpExporterBuilder> {
    let mut exporter = opentelemetry_otlp::new_exporter()
        .http()
        .with_endpoint(endpoint)
        .with_timeout(config.otlp_timeout);

    if !config.otlp_headers.is_empty() {
        let headers: HashMap<String, String> = config.otlp_headers.iter().cloned().collect();
        exporter = exporter.with_headers(headers);
    }

    Ok(exporter)
}

fn build_resource(config: &TelemetryConfig) -> Resource {
    let mut attributes = vec![
        KeyValue::new("service.name", config.service_name.clone()),
        KeyValue::new("telemetry.sdk.language", "rust"),
    ];

    if let Some(version) = config
        .service_version
        .clone()
        .or_else(|| Some(DEFAULT_SERVICE_VERSION.to_string()))
    {
        attributes.push(KeyValue::new("service.version", version));
    }

    if let Some(environment) = config.environment.clone() {
        attributes.push(KeyValue::new("deployment.environment", environment));
    }

    Resource::new(attributes)
}
