use agtalk_app::{run_cli, run_gui, run_popup};
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: agtalk <daemon|gui|__popup|...>");
        return ExitCode::FAILURE;
    }

    match args[1].as_str() {
        "gui" => {
            run_gui();
            ExitCode::SUCCESS
        }
        "__popup" => {
            run_popup();
            ExitCode::SUCCESS
        }
        _ => run_cli(),
    }
}
