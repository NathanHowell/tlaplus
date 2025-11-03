use clap::Parser;
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use tlc_progress::ProgressEvent;

/// Minimal argument placeholder to prove the CLI wiring.
#[derive(Debug, Parser, Serialize, Deserialize)]
#[command(
    name = "tlc",
    version,
    about = "Rust-native TLC command-line interface (scaffold)"
)]
struct Cli {
    /// Path to the primary specification module.
    #[arg(long)]
    spec: Option<String>,

    /// Optional configuration file path.
    #[arg(long)]
    config: Option<String>,
}

fn init_tracing() {
    let filter = std::env::var("TLC_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .compact()
        .try_init();
}

fn main() -> anyhow::Result<()> {
    let _args = Cli::parse();
    let telemetry_config = tlc_telemetry::TelemetryConfig::default();
    let _telemetry_guard = match tlc_telemetry::init_tracing(telemetry_config.clone()) {
        Ok(guard) => guard,
        Err(err) => {
            init_tracing();
            tracing::warn!(error = ?err, "falling back to CLI tracing bootstrap");
            tlc_telemetry::TelemetryGuard::disabled()
        }
    };

    tlc_util::initialize_runtime()?;
    tlc_engine::bootstrap_engine()?;
    let _checkpoint_store =
        tlc_checkpoint::CheckpointStore::ephemeral(tlc_checkpoint::StoreOptions::default())?;
    tlc_progress::render_placeholder(ProgressEvent {
        message: "workspace scaffolding".into(),
    })?;
    tracing::info!("tlc CLI scaffolding initialized");
    Ok(())
}
