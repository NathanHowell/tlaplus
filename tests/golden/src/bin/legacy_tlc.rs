use std::process::ExitCode;

/// Temporary shim that will wrap the legacy TLC Java launcher.
fn main() -> ExitCode {
    eprintln!("legacy TLC launcher shim not yet implemented");
    ExitCode::from(1)
}
