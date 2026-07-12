use agtalk_app::{run_cli, run_gui, run_popup};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("gui") => {
            run_gui();
            ExitCode::SUCCESS
        }
        Some("__popup") => {
            run_popup(args.get(2).cloned());
            ExitCode::SUCCESS
        }
        _ => run_cli(),
    }
}
