pub mod cli;
pub mod commands;
pub mod config;
pub mod feishu;
pub mod graph;
pub mod human;
pub mod identity;
#[cfg(target_os = "macos")]
pub mod macos_dock;
pub mod mem;
pub mod notify;
pub mod paths;
pub mod proto;
pub mod routing;
pub mod server;
pub mod storage;
#[cfg(test)]
pub mod testutil;
pub mod tool;
pub mod transport;

/// Tauri context 只能由 generate_context!() 展开一次（嵌入 Info.plist 符号唯一），
/// gui / popup 入口共用；argv 分派保证单进程只会走一个入口。
fn app_context() -> tauri::Context {
    tauri::generate_context!()
}

pub fn run_gui() {
    // GUI 主窗口（配置界面）：窗口在 setup 显式创建——tauri.conf.json 不配 windows，
    // 否则 __popup 进程启动时也会自动创建配置窗口；配置读写经 Tauri 命令桥 → daemon HTTP API（薄客户端）。
    build_gui_app("index.html".into(), "agtalk".into(), 900.0, 700.0, true)
}

/// 图工程管理界面（M4）：`agtalk graph gui` 加载 `index.html?view=graph`。
/// 与配置 GUI 共用命令桥（含 graph 命令与 SSE 订阅），窗口 1100×760 可调。
pub fn run_graph_gui() {
    build_gui_app(
        "index.html?view=graph".into(),
        "agtalk - 图工程".into(),
        1100.0,
        760.0,
        true,
    )
}

fn build_gui_app(url: String, title: String, width: f64, height: f64, resizable: bool) {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(commands::GraphStreamState::default())
        .invoke_handler(tauri::generate_handler![
            commands::gui_load_config,
            commands::gui_set_config,
            commands::gui_feishu_setup_begin,
            commands::gui_feishu_setup_poll,
            commands::gui_graph_list,
            commands::gui_graph_show,
            commands::gui_graph_events,
            commands::gui_graph_submit,
            commands::gui_graph_cancel,
            commands::gui_graph_stream_start,
            commands::gui_graph_stream_stop,
            commands::gui_node_prompt
        ])
        .setup(move |app| {
            // 裸二进制无 .app 包图标来源，运行时设置 Dock/Cmd+Tab 图标
            #[cfg(target_os = "macos")]
            macos_dock::set_dock_icon();
            tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(url.into()))
                .title(title)
                .inner_size(width, height)
                .resizable(resizable)
                .build()?;
            Ok(())
        })
        .build(app_context())
        .expect("failed to build gui app");
    app.run(|_, _| {});
}

/// 桌面审批弹窗：`agtalk __popup <message-id>` 由 daemon 的 PopupTransport 拉起。
/// 窗口 480×480 不可调；展示消息正文/选项，Reply/Done 经 human API 提交后自动关窗；
/// 直接关窗（Later）不改变消息状态（dismissed，消息仍在 human inbox）。
pub fn run_popup(message_id: Option<String>) {
    let message_id = match message_id {
        Some(id) if !id.trim().is_empty() => id,
        _ => {
            eprintln!("usage: agtalk __popup <message-id>");
            std::process::exit(2);
        }
    };
    let app = tauri::Builder::default()
        .manage(commands::PopupState {
            message_id: message_id.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            commands::popup_load,
            commands::popup_reply,
            commands::popup_done,
            commands::popup_cancel
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            macos_dock::set_dock_icon();
            // ?popup=1 让前端切到弹窗模式；消息 id 由 PopupState 持有，不进 URL
            tauri::WebviewWindowBuilder::new(
                app,
                "popup",
                tauri::WebviewUrl::App("index.html?popup=1".into()),
            )
            .title("agtalk")
            .inner_size(480.0, 480.0)
            .resizable(false)
            .build()?;
            Ok(())
        })
        .build(app_context())
        .expect("failed to build popup app");
    app.run(|_, _| {});
}

pub fn run_cli() -> std::process::ExitCode {
    crate::cli::run_cli()
}
