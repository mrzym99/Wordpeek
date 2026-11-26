//! 系统托盘：应用常驻后台的唯一可见入口。从 main.rs 整体迁出（纯移动不改逻辑）。

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager};

use crate::windows;

/// 托盘菜单：划词轻提示勾选项（当前状态可见、随手切换）+ 退出
fn tray_menu(app: &AppHandle, dot_enabled: bool) -> Result<Menu<tauri::Wry>, tauri::Error> {
    let dot = CheckMenuItem::with_id(
        app,
        "toggle-dot",
        "划词轻提示",
        true,
        dot_enabled,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    Menu::with_items(app, &[&dot, &sep, &quit])
}

/// 开关变化后重建菜单刷新勾选态（热键与托盘两条切换路径都收敛到这里）
pub(crate) fn refresh_dot_item(app: &AppHandle, enabled: bool) {
    if let Some(tray) = app.tray_by_id("main") {
        match tray_menu(app, enabled) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
            }
            Err(e) => eprintln!("[tray] 菜单重建失败: {e}"),
        }
    }
}

/// 建托盘图标：左键直接开设置页，右键弹菜单（轻提示开关 + 退出）
pub(crate) fn setup_tray(app: &App) -> Result<(), tauri::Error> {
    let dot_enabled = app
        .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
        .lock()
        .selection_dot;
    let tray_menu = tray_menu(app.handle(), dot_enabled)?;
    // 托盘图标：macOS 用白色单色模板图（菜单栏随深浅色自动反色），其它平台用应用图标
    let tray_image = if cfg!(target_os = "macos") {
        tauri::include_image!("icons/tray-template.png")
    } else {
        app.default_window_icon()
            .expect("缺少应用图标 icons/icon.ico")
            .clone()
    };
    TrayIconBuilder::with_id("main")
        .tooltip("Wordpeek 划词翻译")
        .icon(tray_image)
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&tray_menu)
        // 左键不开菜单而是走 on_tray_icon_event；右键弹 menu（轻提示开关 + 退出）
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle-dot" => crate::translate::toggle_selection_dot(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                windows::open_settings(tray.app_handle().clone());
            }
        })
        .build(app)?;
    Ok(())
}