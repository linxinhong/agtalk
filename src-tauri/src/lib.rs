use tauri::Manager;

pub mod cli;
pub mod commands;
pub mod config;
pub mod identity;
pub mod ipc;
pub mod join_plugin;
pub mod notify;
pub mod paths;
pub mod server;
pub mod session;
pub mod storage;
pub mod transport;
pub mod workspace;

enum GuiMode {
    Full,
    Popup(String),
}

pub fn run_gui() {
    run_tauri(GuiMode::Full);
}

pub fn run_popup(msg_id: String) {
    run_tauri(GuiMode::Popup(msg_id));
}

fn run_tauri(mode: GuiMode) {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_conversations,
            commands::get_messages,
            commands::get_message,
            commands::get_attachment,
            commands::send_message,
            commands::mark_done,
            commands::mark_read,
            commands::list_participants,
            commands::ping_daemon,
            commands::reply,
            commands::get_popup_focus,
        ]);

    let builder = match mode {
        GuiMode::Full => builder,
        GuiMode::Popup(msg_id) => {
            std::env::set_var("AGTALK_POPUP_MSG_ID", &msg_id);
            builder.setup(move |app| {
                let win = app.get_webview_window("main").expect("主窗口");
                win.set_title("agtalk 审批")?;
                win.set_size(tauri::LogicalSize::new(420.0, 320.0))?;
                win.set_resizable(false)?;
                win.set_focus()?;
                Ok(())
            })
        }
    };

    builder
        .run(tauri::generate_context!())
        .expect("启动 GUI 失败");
}
