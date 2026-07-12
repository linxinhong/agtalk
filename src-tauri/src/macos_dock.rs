//! macOS 运行时设置 Dock / Cmd+Tab 图标（参考 AskHuman macos_dock_icon.rs）。
//!
//! 裸二进制（非 .app 包，`bundle.active=false`）运行时不会读取 bundle 的 icon.icns，
//! Dock 显示系统默认图标。这里在 GUI/popup 启动（主线程 setup）时，把内嵌 PNG 设为
//! `NSApplication.applicationIconImage`，覆盖当前进程的 Dock 图标（同时影响 Cmd+Tab）。
//!
//! 仅影响「运行中的进程」，不改变 bundle 身份；macOS 不自动加圆角，
//! 形状/留白完全取决于 icons/icon.png 本身。

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

/// 内嵌的图标位图（1024×1024，与 tauri.conf.json bundle.icon 同源）。
/// 替换 `icons/icon.png` 后重新构建即可换图。
const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

/// 把内嵌 PNG 设为当前进程的 Dock 图标。必须在主线程调用（Tauri `setup` 闭包即主线程）。
/// 任一步失败静默返回，不影响窗口主流程。
pub fn set_dock_icon() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(ICON_PNG);
    let Some(image): Option<Retained<NSImage>> = NSImage::initWithData(NSImage::alloc(), &data)
    else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setApplicationIconImage(Some(&image)) };
}
