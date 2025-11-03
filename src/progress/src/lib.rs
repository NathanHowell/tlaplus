//! Progress rendering scaffolding using indicatif and serde_json.

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub message: String,
}

/// Render a single placeholder progress event to ensure wiring.
pub fn render_placeholder(event: ProgressEvent) -> Result<()> {
    tlc_util::initialize_runtime()?;
    let _serialized = serde_json::to_string(&event)?;
    let pb = ProgressBar::new(1);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{prefix:.bold} {bar:40.cyan/blue} {pos}/{len} {msg}")
            .expect("valid progress template"),
    );
    pb.set_prefix("tlc");
    pb.set_message(event.message);
    pb.inc(1);
    pb.finish_with_message("done");
    tracing::info!("progress placeholder rendered");
    Ok(())
}
