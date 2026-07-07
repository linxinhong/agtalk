//! agtalk-notify-tmux
//!
//! 命令：
//!   discover  - 从当前 shell 环境发现 tmux endpoint，输出 JSON
//!   send      - 从 stdin 读取 notify payload，执行 tmux 提醒
//!   （无参数默认等价 send）

mod protocol;
mod tmux;

use std::io::{self, Read};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("send");
    let dry_run = args.iter().any(|a| a == "--dry-run");

    match cmd {
        "discover" => run_discover(),
        "send" => run_send(dry_run),
        _ => {
            eprintln!("usage: agtalk-notify-tmux [discover | send [--dry-run]]");
            std::process::exit(1);
        }
    }
}

fn run_discover() {
    let output = match tmux::discover() {
        Ok(pane) => protocol::DiscoverOutput::ready(pane),
        Err(e) => protocol::DiscoverOutput::not_ready(e.to_string()),
    };
    println!("{}", serde_json::to_string(&output).unwrap());
}

fn run_send(dry_run: bool) {
    let mut payload = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut payload) {
        eprintln!("读取 stdin 失败: {}", e);
        std::process::exit(1);
    }

    let input: protocol::SendInput = match serde_json::from_str(&payload) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("解析 payload 失败: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = tmux::send(&input, dry_run) {
        eprintln!("{}", e);
        std::process::exit(1);
    }
}
