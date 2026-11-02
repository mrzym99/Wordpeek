//! 平台层：Windows / macOS 各自实现同一组接口，cfg 门控保证互不影响。
//! Windows 实现全在 windows.rs，macOS 实现全在 macos.rs；
//! 共享逻辑（main.rs / translate.rs / ocr.rs / capture.rs 纯函数）不掺平台细节。

// ---------- 划词图标：全局鼠标监听回调槽（两个平台共用，钩子/监视器从这里取） ----------

/// 鼠标事件回调（进程内只装一次，钩子函数从静态里取）
pub(crate) type MouseCallback = Box<dyn Fn() + Send + Sync>;
/// 鼠标移动回调：参数是钩子/事件自带的光标物理像素坐标（不额外调 GetCursorPos）。
/// macOS 端未用 move：远离图标自动隐藏目前只在 Windows 实现，全局监视器加
/// mouseMoved 会每帧唤醒主线程，不值得
pub(crate) type MoveCallback = Box<dyn Fn(i32, i32) + Send + Sync>;
pub(crate) static MOUSE_ON_DOWN: std::sync::OnceLock<MouseCallback> = std::sync::OnceLock::new();
pub(crate) static MOUSE_ON_UP: std::sync::OnceLock<MouseCallback> = std::sync::OnceLock::new();
pub(crate) static MOUSE_ON_MOVE: std::sync::OnceLock<MoveCallback> = std::sync::OnceLock::new();

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
#[cfg(windows)]
pub use windows::foreground_app;
#[cfg(windows)]
pub use windows::sanitize_stdio;
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

/// 申请屏幕录制权限（弹系统授权框；Windows 无此概念，恒成功）
#[cfg(windows)]
pub fn request_screen_capture() -> bool {
    true
}
#[cfg(target_os = "macos")]
pub fn request_screen_capture() -> bool {
    macos::request_screen_capture()
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

/// 全局鼠标监听：左键按下 → on_down，左键松开 → on_up，鼠标移动 → on_move（带物理像素坐标；
/// macOS 端未用 move）。回调可能很频繁，必须立刻返回（耗时工作自己丢线程）。
#[cfg(windows)]
pub use windows::start_mouse_watch;
#[cfg(target_os = "macos")]
pub use macos::start_mouse_watch;

/// 焦点控件里被选中的文本（系统 AX/UIA 接口，不碰剪贴板；不支持的应用返回 None）
#[cfg(windows)]
pub use windows::disable_rounding;
#[cfg(windows)]
pub use windows::focused_selection;
#[cfg(target_os = "macos")]
pub use macos::focused_selection;
