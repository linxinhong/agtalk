pub mod cli;
pub mod commands;
pub mod ipc;
pub mod server;
pub mod storage;
pub mod transport;
pub mod paths;

pub fn run_gui() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_conversations,
            commands::get_messages,
            commands::send_message,
            commands::mark_done,
            commands::mark_read,
            commands::list_participants,
            commands::ping_daemon,
            commands::reply,
        ])
        .run(tauri::generate_context!())
        .expect("启动 GUI 失败");
}
