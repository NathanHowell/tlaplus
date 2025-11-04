use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::ProgressEvent;

const PROGRESS_RESOLUTION: u64 = 10_000;

/// Configuration flags for the TTY renderer.
#[derive(Debug, Clone, Copy)]
pub struct TtyOptions {
    /// Whether to enable ANSI color styling. Defaults to `true`.
    pub use_color: bool,
}

impl Default for TtyOptions {
    fn default() -> Self {
        Self { use_color: true }
    }
}

#[derive(Debug, Clone)]
struct RenderedState {
    message: String,
    #[cfg(test)]
    percent_complete: f32,
    #[cfg(test)]
    position: u64,
}

/// Interactive TTY progress renderer backed by `indicatif`.
pub struct TtyRenderer {
    bar: ProgressBar,
    start_timestamp: Option<DateTime<Utc>>,
    last_render: Option<RenderedState>,
    options: TtyOptions,
    style_template: &'static str,
    progress_chars: &'static str,
}

impl TtyRenderer {
    /// Create a renderer with default styling (colors enabled).
    pub fn new() -> Result<Self> {
        Self::with_options(TtyOptions::default())
    }

    /// Create a renderer with custom options.
    pub fn with_options(options: TtyOptions) -> Result<Self> {
        let bar = ProgressBar::new(PROGRESS_RESOLUTION);
        bar.set_draw_target(ProgressDrawTarget::stderr_with_hz(12));
        bar.enable_steady_tick(Duration::from_millis(125));

        let style_template: &'static str = if options.use_color {
            "{prefix:.bold.blue} {wide_bar:.cyan/blue} {percent:>6.2}% | {msg}"
        } else {
            "{prefix} {wide_bar} {percent:>6.2}% | {msg}"
        };
        let progress_chars: &'static str = if options.use_color { "━╺ " } else { "=> " };

        let style = ProgressStyle::with_template(style_template)
            .context("failed to construct TTY progress style")?
            .progress_chars(progress_chars);

        bar.set_style(style);
        bar.set_prefix("tlc");
        bar.set_message("initializing progress");

        Ok(Self {
            bar,
            start_timestamp: None,
            last_render: None,
            options,
            style_template,
            progress_chars,
        })
    }

    /// Provide access to the underlying progress bar for advanced configuration (e.g., multi-progress).
    pub fn progress_bar(&self) -> ProgressBar {
        self.bar.clone()
    }

    /// Update the progress bar with the latest exploration statistics.
    pub fn update(&mut self, event: &ProgressEvent) -> Result<()> {
        let percent = event.percent_complete().clamp(0.0, 100.0);
        let position = ((percent * 100.0).round() as u64).min(PROGRESS_RESOLUTION);
        self.bar.set_position(position);

        let start = match self.start_timestamp {
            Some(ts) => ts,
            None => {
                let ts = event.timestamp();
                self.start_timestamp = Some(ts);
                ts
            }
        };

        let elapsed = clamp_duration(event.timestamp().signed_duration_since(start));
        let elapsed_display = format_hms(duration_to_seconds(elapsed));
        let eta_display = event
            .eta_seconds()
            .map(format_hms)
            .unwrap_or_else(|| String::from("--"));

        let message = format!(
            "states {} | throughput {} | workers {} | elapsed {} | eta {}",
            format_states(event.states_explored()),
            format_throughput(event.throughput_eps()),
            event.workers_active(),
            elapsed_display,
            eta_display,
        );

        self.bar.set_message(message.clone());
        self.last_render = Some(RenderedState {
            message,
            #[cfg(test)]
            percent_complete: percent,
            #[cfg(test)]
            position,
        });

        Ok(())
    }

    /// Finish the progress bar, optionally preserving the last rendered message.
    pub fn finish(&self) {
        if let Some(render) = &self.last_render {
            self.bar.finish_with_message(render.message.clone());
        } else {
            self.bar.finish_and_clear();
        }
    }

    /// Override the draw target (useful for tests or alternate output sinks).
    pub fn set_draw_target(&self, target: ProgressDrawTarget) {
        self.bar.set_draw_target(target);
    }

    /// Report whether ANSI colors are enabled for this renderer.
    pub fn use_color(&self) -> bool {
        self.options.use_color
    }

    /// Return the styling template applied to the underlying progress bar.
    pub fn style_template(&self) -> &'static str {
        self.style_template
    }

    /// Return the character sequence used for the progress bar body.
    pub fn bar_characters(&self) -> &'static str {
        self.progress_chars
    }

    /// Access the effective options used to configure the renderer.
    pub fn options(&self) -> TtyOptions {
        self.options
    }

    #[cfg(test)]
    pub(crate) fn last_message(&self) -> Option<&str> {
        self.last_render
            .as_ref()
            .map(|state| state.message.as_str())
    }

    #[cfg(test)]
    pub(crate) fn last_position(&self) -> Option<u64> {
        self.last_render.as_ref().map(|state| state.position)
    }

    #[cfg(test)]
    pub(crate) fn last_percent(&self) -> Option<f32> {
        self.last_render
            .as_ref()
            .map(|state| state.percent_complete)
    }
}

fn format_states(value: u128) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, ch) in digits.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }

    formatted.chars().rev().collect()
}

fn format_throughput(value: f64) -> String {
    const UNITS: [(&str, f64); 4] = [("T", 1e12), ("B", 1e9), ("M", 1e6), ("K", 1e3)];

    for (suffix, threshold) in UNITS {
        if value >= threshold {
            return format!("{:.1}{suffix} states/s", value / threshold);
        }
    }

    if value >= 100.0 {
        format!("{value:.0} states/s")
    } else if value >= 1.0 {
        format!("{value:.1} states/s")
    } else {
        format!("{value:.2} states/s")
    }
}

fn clamp_duration(duration: ChronoDuration) -> ChronoDuration {
    if duration < ChronoDuration::zero() {
        ChronoDuration::zero()
    } else {
        duration
    }
}

fn duration_to_seconds(duration: ChronoDuration) -> u64 {
    duration.num_seconds().max(0) as u64
}

fn format_hms(total_seconds: u64) -> String {
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use chrono::TimeZone;
    use ulid::Ulid;

    fn sample_event() -> Result<ProgressEvent> {
        ProgressEvent::new(
            Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJK")?,
            Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?,
            Utc.with_ymd_and_hms(2025, 11, 2, 3, 25, 45)
                .single()
                .expect("valid timestamp"),
            18_446_744_073_709_551_616,
            62.5,
            125_000.0,
            15,
            Some(5_400),
        )
    }

    #[test]
    fn renders_status_message() -> Result<()> {
        let mut renderer = TtyRenderer::new()?;
        renderer.set_draw_target(ProgressDrawTarget::hidden());

        let mut event = sample_event()?;
        renderer.update(&event)?;

        let message = renderer.last_message().expect("message recorded");

        assert!(
            message.contains("states 18,446,744,073,709,551,616"),
            "state count rendered"
        );
        assert!(
            message.contains("throughput 125.0K states/s"),
            "throughput formatted with suffix"
        );
        assert!(message.contains("workers 15"), "worker count included");
        assert!(message.contains("elapsed 00:00:00"), "elapsed tracked");
        assert!(message.contains("eta 01:30:00"), "eta displayed");

        assert_eq!(renderer.last_position().expect("position recorded"), 6_250);

        // Simulate a later event to ensure elapsed time advances.
        event = ProgressEvent::new(
            Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJN")?,
            event.run_id(),
            Utc.with_ymd_and_hms(2025, 11, 2, 3, 35, 45)
                .single()
                .expect("valid timestamp"),
            18_446_744_173_709_551_616,
            75.0,
            135_000.0,
            15,
            None,
        )?;
        renderer.update(&event)?;

        let message = renderer.last_message().expect("message recorded");
        assert!(message.contains("elapsed 00:10:00"));
        assert!(message.contains("eta --"));

        assert_eq!(renderer.last_position().expect("position recorded"), 7_500);
        assert_eq!(renderer.last_percent().expect("percent recorded"), 75.0);

        Ok(())
    }

    #[test]
    fn format_helpers_cover_edge_cases() {
        assert_eq!(format_states(1_234_567_890), "1,234,567,890".to_string());
        assert_eq!(format_throughput(0.0), "0.00 states/s");
        assert_eq!(format_throughput(999.0), "999 states/s");
        assert_eq!(format_throughput(12_345.0), "12.3K states/s");
        assert_eq!(format_throughput(12_345_678.0), "12.3M states/s");
        assert_eq!(format_throughput(12_345_678_900.0), "12.3B states/s");
        assert_eq!(format_throughput(12_345_678_900_000.0), "12.3T states/s");

        let neg = clamp_duration(ChronoDuration::seconds(-30));
        assert_eq!(neg.num_seconds(), 0);
        assert_eq!(duration_to_seconds(ChronoDuration::seconds(90)), 90);
        assert_eq!(format_hms(3661), "01:01:01".to_string());
    }
}
