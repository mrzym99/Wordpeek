//! 系统托盘：应用常驻后台的唯一可见入口。从 main.rs 整体迁出（纯移动不改逻辑）。

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::App;

use crate::windows;

/// 建托盘图标：左键直接开设置页（不再藏着「输入翻译/设置」两项菜单），右键只留「退出」
pub(crate) fn setup_tray(app: &App) -> Result<(), tauri::Error> {
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let tray_menu = Menu::with_items(app, &[&quit])?;
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
        // 左键不开菜单而是走 on_tray_icon_event；右键仍弹 menu（只剩退出）
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id().as_ref() == "quit" {
                app.exit(0);
            }
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