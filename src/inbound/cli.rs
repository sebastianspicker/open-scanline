//! CLI entry (OSL-CLI) — command surface matching product contracts.

mod args;
mod handlers;
mod manual;

use args::Cli;
use clap::Parser;

/// Run CLI with explicit argv (excluding program name).
pub fn run(argv: &[String]) -> i32 {
    #[cfg(windows)]
    {
        return match parse_with_windows_stack(argv) {
            Ok(result) => dispatch_parse_result(result),
            Err(error) => {
                eprintln!("{error}");
                1
            }
        };
    }
    #[cfg(not(windows))]
    dispatch_parse_result(parse(argv))
}

#[cfg(windows)]
fn parse_with_windows_stack(argv: &[String]) -> Result<Result<Cli, clap::Error>, String> {
    const CLI_STACK_SIZE: usize = 8 * 1024 * 1024;

    let argv = argv.to_vec();
    let worker = std::thread::Builder::new()
        .name("open-scanline-cli".into())
        .stack_size(CLI_STACK_SIZE)
        .spawn(move || parse(&argv))
        .map_err(|error| format!("could not start CLI parser thread: {error}"))?;
    match worker.join() {
        Ok(result) => Ok(result),
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn parse(argv: &[String]) -> Result<Cli, clap::Error> {
    let mut full = vec!["open-scanline".to_string()];
    full.extend(argv.iter().cloned());
    Cli::try_parse_from(&full)
}

fn dispatch_parse_result(result: Result<Cli, clap::Error>) -> i32 {
    match result {
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
