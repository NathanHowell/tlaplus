fn main() {
    if let Err(error) = tlc_parity::parity_runner::run() {
        eprintln!("parity harness failed: {error:?}");
        std::process::exit(1);
    }
}
