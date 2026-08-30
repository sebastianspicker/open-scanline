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
        return run_with_windows_stack(argv);
    }
    #[cfg(not(windows))]
    parse_and_dispatch(argv)
}

#[cfg(windows)]
fn run_with_windows_stack(argv: &[String]) -> i32 {
    const CLI_STACK_SIZE: usize = 8 * 1024 * 1024;

    let argv = argv.to_vec();
    let worker = std::thread::Builder::new()
        .name("open-scanline-cli".into())
        .stack_size(CLI_STACK_SIZE)
        .spawn(move || parse_and_dispatch(&argv));
    match worker {
        Ok(worker) => worker.join().unwrap_or_else(|_| {
            eprintln!("CLI worker thread panicked");
            1
        }),
        Err(error) => {
            eprintln!("could not start CLI worker thread: {error}");
            1
        }
    }
}

fn parse_and_dispatch(argv: &[String]) -> i32 {
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
