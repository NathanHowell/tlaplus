mod commands;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use commands::{Cli, Command};
use tlc_progress::ProgressEvent;

fn init_tracing() {
    let filter = std::env::var("TLC_LOG").unwrap_or_else(|_| "info".to_string());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .with_target(false)
        .compact()
        .try_init();
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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

    match cli.command {
        Command::Run(run) => {
            tracing::info!(?run, "executing `tlc run` (implementation pending)");
        }
        Command::Resume(resume) => {
            tracing::info!(?resume, "executing `tlc resume` (implementation pending)");
        }
    }

    Ok(())
}
