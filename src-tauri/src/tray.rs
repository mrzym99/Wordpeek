//! 系统托盘：应用常驻后台的唯一可见入口。从 main.rs 整体迁出（纯移动不改逻辑）。

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::App;

use crate::windows;

/// 建托盘图标 + 菜单；菜单事件直接调窗口模块
pub(crate) fn setup_tray(app: &App) -> Result<(), tauri::Error> {
    // 系统托盘：应用常驻后台的唯一可见入口
    let input = MenuItem::with_id(app, "input", "输入翻译", true, None::<&str>)?;
    let settings =
        MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let tray_menu = Menu::with_items(app, &[&input, &settings, &quit])?;
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
        .on_menu_event(|app, event| match event.id().as_ref() {
            "input" => windows::open_input_window(app),
            "settings" => windows::open_settings(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}
