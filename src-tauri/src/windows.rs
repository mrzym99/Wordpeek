//! 窗口生命周期：按需创建/置前、高度自适应命令、居中。从 main.rs 整体迁出（纯移动不改逻辑）。

use tauri::{AppHandle, Manager};

use crate::geometry;

/// 前端量好内容高度后调用：只调高不调宽，按算出的新外框重新钳位（不依赖 set_size 后回读，那是异步消息）；
/// 返回实际生效高度（可能被钳小），前端据此判断 is-capped，未顶上限不出滚动条，避免高度动画中闪条
#[tauri::command]
pub(crate) fn fit_window(app: AppHandle, label: String, height: f64, viewport: Option<f64>) -> Result<f64, String> {
    let win = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("窗口不存在: {label}"))?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    let inner = win.inner_size().map_err(|e| e.to_string())?;
    let outer = win.outer_size().map_err(|e| e.to_string())?;
    let pos = win.outer_position().map_err(|e| e.to_string())?;
    // 标题栏/边框占掉的逻辑像素：无边框弹窗为 0，带标题栏的窗口约 31
    let chrome_w = outer.width.saturating_sub(inner.width) as f64 / scale;
    let chrome_h = outer.height.saturating_sub(inner.height) as f64 / scale;
    // 高度上限用窗口所在屏的工作区；work_area_at 给物理像素，内容高度是逻辑像素，先除以 scale 再钳制
    let (_, _, _, work_h) = crate::platform::work_area_at(pos.x + 8, pos.y + 8);
    let inner_w = inner.width as f64 / scale;
    // 带标题栏的窗口里窗口高 ≠ webview 视口高（前端拿到的 100vh 少一条标题栏）需补差值，无边框弹窗为 0；
    // 差值只测一次并缓存：前端上报的视口高可能落后于刚 set_size 的窗口，现算会把差值越滚越大
    let chrome_extra = viewport.map(|vh| geometry::viewport_inset(&label, inner.height as f64 / scale, vh)).unwrap_or(0.0);
    let inner_h = geometry::fit_height(
        height + chrome_extra,
        if work_h > 0 { work_h as f64 / scale } else { 0.0 },
        chrome_h,
    );
    win.set_size(tauri::LogicalSize::new(inner_w, inner_h))
        .map_err(|e| e.to_string())?;

    // 新外框尺寸 = 客户区 + 边框/标题栏，换算成物理像素再钳位；否则 150% 缩放下卡片伸出屏幕底且无滚动条（issue #8）
    let new_w = geometry::to_physical(inner_w + chrome_w, scale);
    let new_h = geometry::to_physical(inner_h + chrome_h, scale);
    let (nx, ny) = match geometry::usable_rect_at(&win, pos.x + 8, pos.y + 8) {
        Some(rect) => geometry::clamp_rect_to_monitor(rect, new_w, new_h, pos.x, pos.y),
        None => (pos.x, pos.y),
    };
    win.set_position(tauri::PhysicalPosition::new(nx, ny))
        .map_err(|e| e.to_string())?;
    Ok(inner_h)
}

/// 摆到主屏工作区中央（Dock/菜单栏/任务栏不遮挡）；不用 win.center()：按整屏居中且隐藏时可能 no-op
fn center_window(win: &tauri::WebviewWindow) {
    let Ok(Some(mon)) = win.primary_monitor() else {
        return;
    };
    let p = mon.position();
    let s = mon.size();
    let full = (p.x, p.y, s.width as i32, s.height as i32);
    let rect = geometry::clamp_rect_prefer_work(
        crate::platform::work_area_at(p.x + full.2 / 2, p.y + full.3 / 2),
        Some(full),
    );
    let (Some(rect), Ok(os)) = (rect, win.outer_size()) else {
        return;
    };
    let x = rect.0 + (rect.2 - os.width as i32) / 2;
    let y = rect.1 + (rect.3 - os.height as i32) / 2;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}

/// 已存在则 show+置前；没建过就在事件循环线程按需创建为隐藏态（内容就绪后前端自己 show），首次创建居中。
/// macOS 用 Overlay 标题栏（交通灯浮在自绘头部上）；drag_drop: true 加 disable_drag_drop_handler（设置窗要拖拽排序）
fn spawn_or_show_window(
    app: &AppHandle,
    label: &str,
    title: &str,
    url: &str,
    w: f64,
    h: f64,
    resizable: bool,
    drag_drop: bool,
) {
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    let for_builder = app.clone();
    let owned_label = label.to_string();
    let owned_title = title.to_string();
    let owned_url = url.to_string();
    // Windows 上 webview 窗口必须在事件循环线程创建，托盘/快捷键/齿轮回调不保证在主线程
    let spawn = app.run_on_main_thread(move || {
        // macOS 分支会重新赋值 builder，Windows 构建下可能才是 unused_mut
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut builder = tauri::WebviewWindowBuilder::new(
            &for_builder,
            &owned_label,
            tauri::WebviewUrl::App(owned_url.into()),
        )
        .title(&owned_title)
        .inner_size(w, h)
        .resizable(resizable)
        .visible(false);
        if drag_drop {
            builder = builder.disable_drag_drop_handler();
        }
        #[cfg(target_os = "macos")]
        {
            builder = builder
                .title_bar_style(tauri::TitleBarStyle::Overlay)
                .hidden_title(true);
        }
        match builder.build() {
            Ok(win) => center_window(&win),
            Err(e) => eprintln!("[{owned_label}] 创建窗口失败: {e}"),
        }
    });
    if let Err(e) = spawn {
        eprintln!("[{label}] 调度创建失败: {e}");
    }
}

/// 显示设置窗口（齿轮按钮/托盘菜单共用）。点 ✕ 即销毁回收常驻 renderer；一级菜单选择已存配置，重开自动回到上次停留的菜单
#[tauri::command]
pub(crate) fn open_settings(app: AppHandle) {
    spawn_or_show_window(
        &app,
        "settings",
        "Wordpeek 设置",
        "/?page=settings",
        600.0,
        440.0,
        false,
        true,
    );
}

/// 显示输入翻译窗口（托盘菜单/快捷键共用）。点 ✕ 即销毁，回收常驻 WebView2 renderer（~60MB）
pub(crate) fn open_input_window(app: &AppHandle) {
    spawn_or_show_window(
        app,
        "input",
        "Wordpeek 输入翻译",
        "/?page=input",
        460.0,
        240.0,
        true,
        false,
    );
}
