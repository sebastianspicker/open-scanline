use super::args::{Cli, RunMode};
use crate::gui::run_gui;
use crate::plugin::run_plugin_mode_with_token;

mod cancellation;
mod commands;
mod common;
mod info;
mod process;
mod router;
mod scan;

pub(super) fn dispatch(cli: Cli) -> i32 {
    let config_path = cli.config.as_deref();

    if matches!(cli.mode, RunMode::Plugin) && cli.cmd.is_none() {
        return cancellation::with_registered_token(|cancellation| {
            run_plugin_mode_with_token(config_path, None, false, None, cancellation)
        });
    }

    match cli.cmd {
        None => run_gui(config_path),
        Some(command) => router::dispatch_command(command, config_path),
    }
}
