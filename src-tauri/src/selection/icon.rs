//! 划词图标显隐：矩形缓存（钩子回调专用的廉价判断）+ show/hide 原语 + 点图标进卡片。

use std::sync::atomic::AtomicBool;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager};

use super::{SessionManager, now_ms};

/// 本次按下时光标是否落在划词图标矩形内（钩子回调只做廉价判断，hide 由 worker 做）
pub(crate) static DOWN_ON_ICON: AtomicBool = AtomicBool::new(false);
/// 划词图标最近一次 show 的屏幕矩形（物理像素 x,y,w,h）：钩子回调用它判断是否点在图标上，
/// 避免回调里跨线程查窗口
pub(crate) static ICON_RECT: Mutex<Option<(i32, i32, u32, u32)>> = Mutex::new(None);

pub(crate) fn hide_selection_icon(app: &AppHandle) {
    *ICON_RECT.lock() = None;
    if let Some(win) = app.get_webview_window("icon") {
        let _ = win.hide();
    }
}

/// 在松手位置显示图标（窗口已在 tauri.conf.json 预建，只挪位置 + show）。锚定松手点而非实时光标：
/// 探测要等 120ms+，追光标的话用户挪开鼠标，图标会在别处冒出来
pub(crate) fn show_selection_icon(app: &AppHandle, text: String, ax: i32, ay: i32) {
    app.state::<SessionManager>().set_icon_text(Some(text));
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(win) = inner.get_webview_window("icon") else {
            crate::platform::debug_log("[icon] 找不到预建的 icon 窗口");
            return;
        };
        let (x, y) = crate::geometry::clamp_to_screen(&inner, "icon", ax + 8, ay + 12);
        let moved = win.set_position(tauri::PhysicalPosition::new(x, y));
        let size = win.outer_size();
        if let Ok(size) = &size {
            *ICON_RECT.lock() = Some((x, y, size.width, size.height));
        }
        let shown = win.show();
        crate::platform::debug_log(&format!(
            "[icon] show ({x},{y}) size={size:?} set_position={moved:?} show={shown:?}"
        ));
    });
}

/// 光标是否落在缓存的图标矩形内；只在钩子回调用，只碰 GetCursorPos + 内存读，绝不做跨线程窗口查询
pub(crate) fn cursor_on_icon_rect() -> bool {
    let Some((x, y, w, h)) = *ICON_RECT.lock() else {
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
    // 藏起来的窗口还留着上次的位置尺寸，不判可见的话卡片收起后那块屏幕区域就再也划不出图标
    if !win.is_visible().unwrap_or(false) {
        return false;
    }
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return false;
    };
    let (x, y) = crate::platform::cursor_pos();
    x >= pos.x && x < pos.x + size.width as i32 && y >= pos.y && y < pos.y + size.height as i32
}

/// 点划词图标：把上次探测到的选区交给弹窗卡片（与热键同一合流点，绝不碰剪贴板）
#[tauri::command]
pub(crate) fn translate_selection_icon(app: AppHandle) -> Result<(), String> {
    let (cx, cy) = crate::platform::cursor_pos();
    let text = app
        .state::<SessionManager>()
        .take_icon_text(now_ms())
        .ok_or("没有缓存的选中文本")?;
    hide_selection_icon(&app);
    SessionManager::show_card(&app, &text, cx + 12, cy + 16, super::SOURCE_ICON);
    Ok(())
}
