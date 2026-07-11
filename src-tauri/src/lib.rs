pub mod cli;
pub mod commands;
pub mod config;
pub mod human;
pub mod identity;
pub mod mem;
pub mod notify;
pub mod paths;
pub mod proto;
pub mod routing;
pub mod server;
pub mod storage;
pub mod tool;
pub mod transport;

pub fn run_gui() {
    // TODO: initialize Tauri GUI app
}

pub fn run_popup() {
    // TODO: spawn approval popup window
}

pub fn run_cli() -> std::process::ExitCode {
    crate::cli::run_cli()
}
