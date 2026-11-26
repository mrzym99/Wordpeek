//! 划词轻提示显隐：矩形缓存（钩子回调专用的廉价判断）+ show/hide 原语 + 点小点进卡片。

use std::sync::atomic::AtomicBool;

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use super::{SessionManager, now_ms};

/// 本次按下时光标是否落在划词轻提示矩形内（钩子回调只做廉价判断，hide 由 worker 做）
pub(crate) static DOWN_ON_DOT: AtomicBool = AtomicBool::new(false);
/// 划词轻提示最近一次 show 的屏幕矩形（物理像素 x,y,w,h）：钩子回调用它判断是否点在小点上，
/// 避免回调里跨线程查窗口
pub(crate) static DOT_RECT: Mutex<Option<(i32, i32, u32, u32)>> = Mutex::new(None);
/// 光标当前是否在小点 proximity 盒内（move 回调记、边沿检测用）
pub(crate) static CURSOR_NEAR: AtomicBool = AtomicBool::new(false);
/// 「翻译」胶囊是否处于展开态（超时定时器据此不误收展开态）
pub(crate) static DOT_EXPANDED: AtomicBool = AtomicBool::new(false);
/// 超时定时器纪元：每次武装（show / Far 收回重启计时）递增，旧定时器醒来发现
/// 纪元已变就让位——防「展开又移开后，show 时的旧定时器提前把小点收掉」
pub(crate) static DOT_TIMER_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// dot-state 事件载荷：near=光标在 proximity 盒内（展开/收回），guide=本次展示带引导气泡
#[derive(serde::Serialize, Clone)]
pub(crate) struct DotState {
    pub near: bool,
    pub guide: bool,
}

pub(crate) fn hide_selection_dot(app: &AppHandle) {
    *DOT_RECT.lock() = None;
    if let Some(win) = app.get_webview_window("icon") {
        let _ = win.hide();
    }
}

/// 静置判定通过后在松手位置显示轻提示（窗口已在 tauri.conf.json 预建，挪位 + show）。
/// 首次展示带引导气泡（窗口临时扩到 220×78 容纳气泡文案），展示即置位落盘，
/// 之后永不再弹；非引导态恢复常驻尺寸
pub(crate) fn show_selection_dot(app: &AppHandle, text: String, ax: i32, ay: i32) {
    app.state::<SessionManager>().set_dot_text(Some(text));
    let guide = {
        let already = app
            .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
            .lock()
            .dot_guide_shown;
        if !already {
            crate::translate::with_config_write(app, |c| c.dot_guide_shown = true);
        }
        !already
    };
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(win) = inner.get_webview_window("icon") else {
            crate::platform::debug_log("[dot] 找不到预建的 icon 窗口");
            return;
        };
        let size = if guide {
            tauri::LogicalSize::new(220.0, 78.0)
        } else {
            tauri::LogicalSize::new(84.0, 44.0)
        };
        let _ = win.set_size(size);
        let (x, y) = crate::geometry::clamp_to_screen(&inner, "icon", ax + 8, ay + 12);
        let moved = win.set_position(tauri::PhysicalPosition::new(x, y));
        let outer = win.outer_size();
        if let Ok(size) = &outer {
            *DOT_RECT.lock() = Some((x, y, size.width, size.height));
        }
        let shown = win.show();
        let _ = inner.emit(
            "dot-state",
            DotState { near: false, guide },
        );
        crate::platform::debug_log(&format!(
            "[dot] show guide={guide} ({x},{y}) size={outer:?} set_position={moved:?} show={shown:?}"
        ));
    });
}

/// 光标是否落在缓存的小点矩形内；只在钩子回调用，只碰 GetCursorPos + 内存读，绝不做跨线程窗口查询
pub(crate) fn cursor_on_dot_rect() -> bool {
    let Some((x, y, w, h)) = *DOT_RECT.lock() else {
        return false;
    };
    let (cx, cy) = crate::platform::cursor_pos();
    cx >= x && cx < x + w as i32 && cy >= y && cy < y + h as i32
}

/// 光标是否落在指定窗口内（物理像素）。取不到窗口/位置按不在处理
pub(crate) fn cursor_on_window(app: &AppHandle, label: &str) -> bool {
    let Some(win) = app.get_webview_window(label) else {
        return false;
    };
    // 藏起来的窗口还留着上次的位置尺寸，不判可见的话卡片收起后那块屏幕区域就再也划不出小点
    if !win.is_visible().unwrap_or(false) {
        return false;
    }
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return false;
    };
    let (x, y) = crate::platform::cursor_pos();
    x >= pos.x && x < pos.x + size.width as i32 && y >= pos.y && y < pos.y + size.height as i32
}

/// 点划词轻提示：把上次探测到的选区交给弹窗卡片（与热键同一合流点，绝不碰剪贴板）
#[tauri::command]
pub(crate) fn translate_selection_dot(app: AppHandle) -> Result<(), String> {
    let (cx, cy) = crate::platform::cursor_pos();
    let text = app
        .state::<SessionManager>()
        .take_dot_text(now_ms())
        .ok_or("没有缓存的选中文本")?;
    hide_selection_dot(&app);
    SessionManager::show_card(&app, &text, cx, cy, super::SOURCE_ICON);
    Ok(())
}
