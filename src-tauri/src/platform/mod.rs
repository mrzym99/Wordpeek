//! 平台层：Windows / macOS 各自实现同一组接口，cfg 门控保证互不影响。
//! Windows 实现全在 windows.rs，macOS 实现全在 macos.rs；
//! 共享逻辑（main.rs / translate.rs / ocr.rs / capture.rs 纯函数）不掺平台细节。

#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;

/// 复制注入（Windows: Ctrl+C；macOS: Cmd+C，需辅助功能权限）
#[cfg(windows)]
pub use windows::inject_copy;
#[cfg(target_os = "macos")]
pub use macos::inject_copy;

/// 光标位置（物理像素）
#[cfg(windows)]
pub use windows::cursor_pos;
#[cfg(target_os = "macos")]
pub use macos::cursor_pos;

/// 光标所在显示器的工作区（物理像素；拿不到时 (0,0,0,0)）
#[cfg(windows)]
pub use windows::work_area_at;
#[cfg(target_os = "macos")]
pub use macos::work_area_at;

/// 虚拟屏截取（所有显示器合成一张）
#[cfg(windows)]
pub use windows::capture_virtual_screen;
#[cfg(target_os = "macos")]
pub use macos::capture_virtual_screen;

/// 虚拟屏边界（物理像素）
#[cfg(windows)]
pub use windows::virtual_screen_bounds;
#[cfg(target_os = "macos")]
pub use macos::virtual_screen_bounds;

/// 辅助功能权限（Windows 无此概念，恒可用）
#[cfg(windows)]
pub fn accessibility_allowed() -> bool {
    true
}
#[cfg(target_os = "macos")]
pub fn accessibility_allowed() -> bool {
    macos::accessibility_allowed()
}

/// 屏幕录制权限（Windows 无此概念，恒可用）
#[cfg(windows)]
pub fn screen_capture_allowed() -> bool {
    true
}
#[cfg(target_os = "macos")]
pub fn screen_capture_allowed() -> bool {
    macos::screen_capture_allowed()
}

/// 应用启动时初始化平台状态（macOS：Dock 策略 + 主线程缓存屏幕信息）
#[cfg(windows)]
pub fn init(_app: &tauri::AppHandle) {}

#[cfg(target_os = "macos")]
pub fn init(app: &tauri::AppHandle) {
    macos::init(app);
}

/// 截图遮罩窗口的额外平台处理（Windows 无需）
#[cfg(windows)]
pub fn prepare_overlay(_win: &tauri::WebviewWindow) {}

#[cfg(target_os = "macos")]
pub fn prepare_overlay(win: &tauri::WebviewWindow) {
    macos::prepare_overlay(win);
}
