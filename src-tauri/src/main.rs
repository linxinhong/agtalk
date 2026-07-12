use agtalk_app::{run_cli, run_popup};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("__popup") => {
            run_popup(args.get(2).cloned());
            ExitCode::SUCCESS
        }
        _ => run_cli(),
    }
}
