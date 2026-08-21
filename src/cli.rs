//! CLI entry (OSL-CLI) — command surface matching product contracts.

mod args;
mod handlers;
mod manual;

use args::Cli;
use clap::Parser;

/// Run CLI with explicit argv (excluding program name).
pub fn run(argv: &[String]) -> i32 {
    let mut full = vec!["open-scanline".to_string()];
    full.extend(argv.iter().cloned());
    match Cli::try_parse_from(&full) {
        Ok(cli) => handlers::dispatch(cli),
        Err(e) => {
            // clap handles --version / --help via print
            let _ = e.print();
            if e.use_stderr() {
                2
            } else {
                // --help / --version exit success
                0
            }
        }
    }
}

/// Run CLI from process environment / args. Returns process exit code.
pub fn run_from_env() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    run(&args)
}
