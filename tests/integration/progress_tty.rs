use anyhow::Result;
use indicatif::ProgressDrawTarget;
use tlc_progress::{ProgressEvent, TtyOptions, TtyRenderer};
use ulid::Ulid;

fn sample_event() -> Result<ProgressEvent> {
    let event = ProgressEvent::new(
        Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJR")?,
        Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJS")?,
        chrono::Utc::now(),
        1_234_567,
        12.5,
        42.0,
        4,
        Some(3600),
    )?;
    Ok(event)
}

#[test]
fn tty_renderer_uses_color_by_default() -> Result<()> {
    let mut renderer = TtyRenderer::new()?;
    renderer.set_draw_target(ProgressDrawTarget::hidden());
    assert!(
        renderer.use_color(),
        "TTY renderer should enable color by default"
    );
    assert_eq!(
        renderer.style_template(),
        "{prefix:.bold.blue} {wide_bar:.cyan/blue} {percent:>6.2}% | {msg}"
    );
    assert_eq!(renderer.bar_characters(), "━╺ ");

    let event = sample_event()?;
    renderer.update(&event)?;

    Ok(())
}

#[test]
fn tty_renderer_disables_color_when_requested() -> Result<()> {
    let mut renderer = TtyRenderer::with_options(TtyOptions { use_color: false })?;
    renderer.set_draw_target(ProgressDrawTarget::hidden());
    assert!(
        !renderer.use_color(),
        "TTY renderer should disable color when requested"
    );
    assert_eq!(
        renderer.style_template(),
        "{prefix} {wide_bar} {percent:>6.2}% | {msg}"
    );
    assert_eq!(renderer.bar_characters(), "=> ");

    let event = sample_event()?;
    renderer.update(&event)?;

    Ok(())
}
