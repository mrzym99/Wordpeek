//! 纯几何工具：窗口高度自适应、坐标钳制、DPI 换算。从 main.rs 整体迁出（纯移动不改逻辑），
//! 与具体领域无关，单测随行。

use tauri::{AppHandle, Manager};

/// 内容高度 → 客户区逻辑高度：夹在 60px 与「工作区−标题栏−40px」间，超出才内部滚动；拿不到工作区退回 800
pub(crate) fn fit_height(content_h: f64, work_h: f64, chrome_h: f64) -> f64 {
    let max = if work_h > 0.0 {
        work_h - chrome_h - 40.0
    } else {
        800.0
    };
    content_h.clamp(60.0, max.max(60.0))
}

/// 逻辑像素 → 物理像素（四舍五入、至少 1px）；钳位前必须乘 scale，否则缩放 ≠100% 时钳不住
pub(crate) fn to_physical(logical: f64, scale: f64) -> i32 {
    (logical * scale).round().max(1.0) as i32
}

/// 向上取到「乘 scale 后得整数物理像素」的最小逻辑粒度的倍数。
/// WebView2 在分数缩放（如 125%=5/4）下会把视口向上取整，渲染表面可比窗口多 1 物理像素，
/// 贴底的卡片底边框被裁（issue「底边框偶发消失」）。逻辑尺寸取分母倍数后视口无分数，两边对齐。
pub(crate) fn snap_to_scale(logical: f64, scale: f64) -> f64 {
    // 最小 n 使 n×scale 为整数（1.25→4，1.5→2，1/2→1）；上限兙死避免病态 scale 空转
    let mut n = 1u32;
    while n < 100 && ((n as f64 * scale) - (n as f64 * scale).round()).abs() > 1e-6 {
        n += 1;
    }
    (logical / n as f64).ceil() * n as f64
}

/// 把左上角在 (x, y)、尺寸 win_w×win_h 的窗口钳进显示器矩形；窗口比屏大时贴左上角
pub(crate) fn clamp_rect_to_monitor(
    monitor: (i32, i32, i32, i32),
    win_w: i32,
    win_h: i32,
    x: i32,
    y: i32,
) -> (i32, i32) {
    let (mx, my, mw, mh) = monitor;
    let cx = x.clamp(mx, (mx + mw - win_w).max(mx));
    let cy = y.clamp(my, (my + mh - win_h).max(my));
    (cx, cy)
}

/// 找到包含点 (x, y) 的显示器矩形（物理像素）。查不到显示器信息时返回 None。
pub(crate) fn monitor_rect_at(
    win: &tauri::WebviewWindow,
    x: i32,
    y: i32,
) -> Option<(i32, i32, i32, i32)> {
    let monitors = win.available_monitors().ok()?;
    let m = monitors.into_iter().find(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x && x < p.x + s.width as i32 && y >= p.y && y < p.y + s.height as i32
    })?;
    let p = m.position();
    let s = m.size();
    Some((p.x, p.y, s.width as i32, s.height as i32))
}

/// 钳制矩形优先用工作区（排除菜单栏/Dock/任务栏），全 0 才退回整屏——整屏钳会让贴底卡片钻到 Dock 下
pub(crate) fn clamp_rect_prefer_work(
    work: (i32, i32, i32, i32),
    monitor: Option<(i32, i32, i32, i32)>,
) -> Option<(i32, i32, i32, i32)> {
    if work.2 > 0 && work.3 > 0 {
        Some(work)
    } else {
        monitor
    }
}

/// 光标所在处的位置钳制矩形（见 clamp_rect_prefer_work）
pub(crate) fn usable_rect_at(
    win: &tauri::WebviewWindow,
    x: i32,
    y: i32,
) -> Option<(i32, i32, i32, i32)> {
    clamp_rect_prefer_work(
        crate::platform::work_area_at(x, y),
        monitor_rect_at(win, x, y),
    )
}

/// 把弹窗位置钳进光标所在屏幕；任一步骤拿不到信息（找不到窗口/显示器）就原样返回不钳制
pub(crate) fn clamp_to_screen(app: &AppHandle, label: &str, x: i32, y: i32) -> (i32, i32) {
    let Some(win) = app.get_webview_window(label) else {
        return (x, y);
    };
    let Some(rect) = usable_rect_at(&win, x, y) else {
        return (x, y);
    };
    let Ok(win_size) = win.outer_size() else {
        return (x, y);
    };
    let (cx, cy) = clamp_rect_to_monitor(
        rect,
        win_size.width as i32,
        win_size.height as i32,
        x,
        y,
    );
    println!(
        "[pos] cursor ({x}, {y}) -> clamped ({cx}, {cy}); work area {rect:?}, window {win_size:?}"
    );
    (cx, cy)
}

/// 窗口高与 webview 视口高的差值（标题栏高度），按窗口名缓存一次
pub(crate) fn viewport_inset(label: &str, win_h: f64, viewport_h: f64) -> f64 {
    static INSETS: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashMap<String, f64>>> =
        std::sync::OnceLock::new();
    let mut cache = INSETS.get_or_init(Default::default).lock();
    if let Some(v) = cache.get(label) {
        return *v;
    }
    // 窗口还没显示时视口高不可信，先不缓存
    if viewport_h < 50.0 {
        return 0.0;
    }
    let inset = (win_h - viewport_h).clamp(0.0, 120.0);
    cache.insert(label.to_string(), inset);
    inset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_height_clamps_between_floor_and_work_area() {
        // 工作区 1040px（逻辑）：上限 = 1040-40 = 1000px
        assert_eq!(fit_height(120.0, 1040.0, 0.0), 120.0); // 常见短卡片：原样
        assert_eq!(fit_height(10.0, 1040.0, 0.0), 60.0); // 下限，别只剩一条缝
        assert_eq!(fit_height(900.0, 1040.0, 0.0), 900.0); // 没到上限：铺满内容
        assert_eq!(fit_height(1100.0, 1040.0, 0.0), 1000.0); // 超出上限 → 内部滚动
        assert_eq!(fit_height(1100.0, 1040.0, 31.0), 969.0); // 有标题栏：上限再扣掉标题栏
        assert_eq!(fit_height(900.0, 0.0, 0.0), 800.0); // 拿不到工作区 → 退回 800
        assert_eq!(fit_height(9000.0, 50.0, 31.0), 60.0); // 工作区极小时不低于下限
    }

    #[test]
    fn clamp_rect_prefers_work_area_so_dock_does_not_cover_card() {
        // macOS 屏 1512×982，Dock 在底部：工作区高 871（差 111）
        let monitor = (0, 0, 1512, 982);
        let work = (0, 25, 1512, 871);
        assert_eq!(clamp_rect_prefer_work(work, Some(monitor)), Some(work));
        // 卡片贴底：整屏钳会停在 y=682（底边 982，被 Dock 盖住），工作区钳停在 596（底边 896）
        assert_eq!(clamp_rect_to_monitor(monitor, 340, 300, 100, 900), (100, 682));
        assert_eq!(clamp_rect_to_monitor(work, 340, 300, 100, 900), (100, 596));
        // 拿不到工作区（全 0）时退回整屏，不能不钳制
        assert_eq!(clamp_rect_prefer_work((0, 0, 0, 0), Some(monitor)), Some(monitor));
        assert_eq!(clamp_rect_prefer_work((0, 0, 0, 0), None), None);
    }

    #[test]
    fn to_physical_scales_logical_size() {
        assert_eq!(to_physical(600.0, 1.0), 600); // 100% 缩放：原样
        assert_eq!(to_physical(600.0, 1.5), 900); // 150%：卡片真实物理高，按它钳才钳得住
        assert_eq!(to_physical(340.0, 1.25), 425); // 125% 宽度同理
        assert_eq!(to_physical(0.0, 1.5), 1); // 向下归零也不允许 0 尺寸
    }

    #[test]
    fn tall_card_at_150_scale_is_pulled_inside_screen() {
        // 光标在屏底附近（物理 y=950，屏高 1080）：逻辑 600px 的卡片物理高 900
        let (x, y) = clamp_rect_to_monitor((0, 0, 1920, 1080), to_physical(340.0, 1.5), to_physical(600.0, 1.5), 100, 950 + 16);
        // 修复前：按逻辑高 600 钳位，y 保持 966，底边 1866 伸出屏幕 786px；
        // 修复后：整卡收进屏内，底边贴屏底
        assert_eq!(y, 1080 - 900, "卡片底边应贴屏幕下沿而不是伸出屏幕");
        assert_eq!(x, 100);
    }

    #[test]
    fn snap_to_scale_yields_integral_physical() {
        // 125%：逻辑 214×1.25=267.5 会引发 WebView2 视口取整比表面多 1px，须向上取到 216（→270 物理）
        assert_eq!(snap_to_scale(214.0, 1.25), 216.0);
        assert_eq!(snap_to_scale(216.0, 1.25), 216.0);
        assert_eq!(snap_to_scale(213.6, 1.25), 216.0);
        // 150%：粒度 2
        assert_eq!(snap_to_scale(215.0, 1.5), 216.0);
        // 100%/200%：任何整数都不产生分数物理
        assert_eq!(snap_to_scale(214.0, 1.0), 214.0);
        assert_eq!(snap_to_scale(214.5, 2.0), 215.0);
        assert_eq!(snap_to_scale(400.0, 1.25), 400.0);
    }

    #[test]
    fn clamp_rect_to_monitor_keeps_window_inside() {
        // 主屏 1920×1080，窗口 400×300
        assert_eq!(clamp_rect_to_monitor((0, 0, 1920, 1080), 400, 300, -50, -20), (0, 0));
        assert_eq!(clamp_rect_to_monitor((0, 0, 1920, 1080), 400, 300, 1900, 1070), (1520, 780));
        assert_eq!(clamp_rect_to_monitor((0, 0, 1920, 1080), 400, 300, 500, 500), (500, 500));
        // 副屏在左侧（负坐标）
        assert_eq!(clamp_rect_to_monitor((-1920, 0, 1920, 1080), 400, 300, -1900, 10), (-1900, 10));
        // 窗口比屏还宽时贴左上角而不是弹出去
        assert_eq!(clamp_rect_to_monitor((-1920, 0, 1920, 1080), 2400, 300, -1900, 10), (-1920, 10));
    }
}
