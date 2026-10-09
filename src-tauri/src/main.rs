#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Settings, Keyboard};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_updater::{Update, UpdaterExt};

#[cfg(windows)]
use windows::Win32::Foundation::POINT;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

mod capture;
mod translate;

#[derive(serde::Serialize, Clone)]
struct Selection {
    text: String,
    x: i32,
    y: i32,
}

/// 模拟 Ctrl+C 抓取当前选中文本，并把原剪贴板内容还原
fn grab_selection() -> Option<String> {
    let mut enigo = Enigo::new(&Settings::default()).ok()?;
    let mut clipboard = Clipboard::new().ok()?;

    let old = clipboard.get_text().unwrap_or_default();
    println!("[grab] old clipboard: {:?}", old);

    // 先把剪贴板替换成哨兵值：若 Ctrl+C 成功，剪贴板必然变成选中文本。
    // 不能用"剪贴板内容是否变化"来判断，否则选中与剪贴板相同的文本时会被误判。
    const SENTINEL: &str = "\u{200b}__pot-mini-sentinel__\u{200b}";
    clipboard.set_text(SENTINEL).ok()?;

    // 快捷键是 Ctrl+Alt+T，触发时物理 Alt 键仍被按着，
    // 直接模拟 Ctrl+C 会被应用当成 Ctrl+Alt+C 而拒绝复制。
    // 先注入一个 Alt 松开事件，让目标应用只看到 Ctrl+C。
    enigo.key(Key::Alt, Direction::Release).ok()?;
    enigo.key(Key::Control, Direction::Press).ok()?;
    enigo.key(Key::C, Direction::Click).ok()?;
    enigo.key(Key::Control, Direction::Release).ok()?;

    std::thread::sleep(std::time::Duration::from_millis(150));

    let text = clipboard.get_text().unwrap_or_default();
    println!("[grab] after ctrl+c: {:?}", text);
    let _ = clipboard.set_text(&old);

    // 剪贴板还是哨兵值说明 Ctrl+C 没有生效（目标应用没有可复制的内容）
    if text == SENTINEL {
        return None;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(windows)]
fn cursor_pos() -> (i32, i32) {
    unsafe {
        let mut p = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut p);
        (p.x, p.y)
    }
}

#[cfg(not(windows))]
fn cursor_pos() -> (i32, i32) {
    (100, 100)
}

/// 把弹窗位置钳制在光标所在屏幕内，避免边缘选词时弹窗伸出屏幕外。
/// 任一步骤拿不到信息（找不到窗口/显示器）就原样返回，不钳制。
fn clamp_to_screen(app: &AppHandle, x: i32, y: i32) -> (i32, i32) {
    let Some(win) = app.get_webview_window("main") else {
        return (x, y);
    };
    let Ok(monitors) = win.available_monitors() else {
        return (x, y);
    };
    // 找到光标落在哪块屏幕上
    let Some(m) = monitors.into_iter().find(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x && x < p.x + s.width as i32 && y >= p.y && y < p.y + s.height as i32
    }) else {
        return (x, y);
    };
    let Ok(win_size) = win.outer_size() else {
        return (x, y);
    };
    let p = m.position();
    let s = m.size();
    let max_x = p.x + s.width as i32 - win_size.width as i32;
    let max_y = p.y + s.height as i32 - win_size.height as i32;
    let cx = x.clamp(p.x, max_x.max(p.x));
    let cy = y.clamp(p.y, max_y.max(p.y));
    println!(
        "[pos] cursor ({x}, {y}) -> clamped ({cx}, {cy}); monitor {:?} {:?}, window {win_size:?}",
        p, s
    );
    (cx, cy)
}

/// 显示独立的设置窗口（弹窗齿轮按钮 / 托盘菜单共用）
#[tauri::command]
fn open_settings(app: AppHandle) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// 已缓存的可用更新（启动 / 手动检查后写入）
type UpdateState = std::sync::Mutex<Option<Update>>;

#[derive(serde::Serialize, Debug)]
struct UpdateStatus {
    available: bool,
    current_version: String,
    new_version: Option<String>,
}

fn status_of(app: &AppHandle, update: Option<&Update>) -> UpdateStatus {
    UpdateStatus {
        available: update.is_some(),
        current_version: app.package_info().version.to_string(),
        new_version: update.map(|u| u.version.clone()),
    }
}

/// 检查更新并把结果缓存到应用状态
fn check_update(app: &AppHandle) -> Result<UpdateStatus, String> {
    let update = tauri::async_runtime::block_on(
        app.updater()
            .map_err(|e| e.to_string())?
            .check(),
    )
    .map_err(|e| e.to_string())?;
    let status = status_of(app, update.as_ref());
    println!("[updater] check: {status:?}");
    *app.state::<UpdateState>().lock().unwrap() = update;
    Ok(status)
}

/// 设置页打开时查询缓存的检查结果（不发网络请求）
#[tauri::command]
fn get_update_status(app: AppHandle) -> UpdateStatus {
    let cached = app.state::<UpdateState>().lock().unwrap().as_ref().cloned();
    status_of(&app, cached.as_ref())
}

/// 手动检查更新（设置页「检查更新」按钮）
#[tauri::command]
fn check_update_now(app: AppHandle) -> Result<UpdateStatus, String> {
    check_update(&app)
}

/// 下载并静默安装已缓存的更新，完成后重启应用（后台线程执行）
#[tauri::command]
fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .state::<UpdateState>()
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "没有已缓存的更新".to_string())?;
    std::thread::spawn(move || {
        println!("[updater] downloading & installing...");
        let install = update.download_and_install(|_chunk, _total| {}, || {});
        if let Err(e) = tauri::async_runtime::block_on(install) {
            eprintln!("[updater] install failed: {e}");
            return;
        }
        // NSIS 静默安装结束后旧进程通常已被安装器接管，restart 兜底
        app.restart();
    });
    Ok(())
}

/// 解析 "Ctrl+Alt+T" 形式的快捷键字符串
fn parse_hotkey(s: &str) -> Result<Shortcut, String> {
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;
    for part in s.split('+') {
        let p = part.trim().to_lowercase();
        if p.is_empty() {
            continue;
        }
        match p.as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "super" | "win" | "meta" | "cmd" => mods |= Modifiers::SUPER,
            other => {
                if key.is_some() {
                    return Err(format!("无效的快捷键: {s}"));
                }
                key = Some(parse_code(other).ok_or_else(|| format!("不支持的按键: {part}"))?);
            }
        }
    }
    let key = key.ok_or_else(|| format!("无效的快捷键: {s}"))?;
    if mods.is_empty() {
        return Err(format!("快捷键必须包含 Ctrl/Alt/Shift 修饰键: {s}"));
    }
    Ok(Shortcut::new(Some(mods), key))
}

/// 支持单个字母、数字、F1-F12
fn parse_code(s: &str) -> Option<Code> {
    use std::str::FromStr;
    let up = s.to_uppercase();
    if up.len() == 1 {
        let c = up.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return Code::from_str(&format!("Key{c}")).ok();
        }
        if c.is_ascii_digit() {
            return Code::from_str(&format!("Digit{c}")).ok();
        }
        return None;
    }
    if up.starts_with('F') && up[1..].chars().all(|c| c.is_ascii_digit()) {
        return Code::from_str(&up).ok();
    }
    None
}

/// 快捷键触发：抓取选中文本并通知弹窗
fn handle_hotkey_trigger(app: &AppHandle) {
    println!("[shortcut] triggered");
    // 抓取选中文本需要模拟按键并等待剪贴板，放到独立线程，
    // 避免阻塞主线程；且此时用户可能已松开按键。
    let app = app.clone();
    std::thread::spawn(move || {
        match grab_selection() {
            Some(text) => {
                let (cx, cy) = cursor_pos();
                println!("[grab] got text: {:?} at ({}, {})", text, cx, cy);
                // 在 Rust 端钳制到屏幕内后再发事件，前端直接使用
                let (x, y) = clamp_to_screen(&app, cx + 12, cy + 16);
                let _ = app.emit("selection", Selection { text, x, y });
            }
            None => println!("[grab] no text captured"),
        }
    });
}

/// 按配置字符串注册全局快捷键（会先注销所有旧的）
pub(crate) fn register_hotkey(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut = parse_hotkey(hotkey)?;
    app.global_shortcut().unregister_all().map_err(|e| e.to_string())?;
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                handle_hotkey_trigger(app);
            }
        })
        .map_err(|e| format!("{e}"))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // 启动时加载翻译配置（有默认值，配置文件不存在也能跑）
            let handle = app.handle().clone();
            app.manage(std::sync::Mutex::new(translate::load_config(&handle)));
            app.manage(UpdateState::default());

            // 启动后延迟几秒静默检查一次更新，结果缓存供设置页查询
            let updater_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(6));
                if let Err(e) = check_update(&updater_handle) {
                    eprintln!("[updater] startup check failed: {e}");
                }
            });

            // 设置窗口点 ✕ 时只隐藏不销毁，否则窗口一旦关闭
            // 就无法再次打开（get_webview_window 返回 None）
            if let Some(win) = app.get_webview_window("settings") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            // 系统托盘：应用常驻后台的唯一可见入口，
            // 左键/右键弹出菜单，可打开设置或退出
            let settings =
                MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&settings, &quit])?;
            TrayIconBuilder::with_id("main")
                .tooltip("Wordpeek 划词翻译")
                .icon(
                    app.default_window_icon()
                        .expect("缺少应用图标 icons/icon.ico")
                        .clone(),
                )
                .menu(&tray_menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => open_settings(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // 按配置注册全局快捷键（可在设置窗口修改）。
            // 注册失败（如快捷键被其他程序占用）时不 panic，应用仍能启动。
            let hotkey_str = app
                .state::<std::sync::Mutex<translate::AppConfig>>()
                .lock()
                .unwrap()
                .hotkey
                .clone();
            if let Err(e) = register_hotkey(&handle, &hotkey_str) {
                eprintln!("[shortcut] {hotkey_str} 注册失败: {e}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_settings,
            get_update_status,
            check_update_now,
            install_update,
            translate::translate,
            translate::get_config,
            translate::save_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_common_hotkeys() {
        assert!(parse_hotkey("Ctrl+Alt+T").is_ok());
        assert!(parse_hotkey("Ctrl+Shift+D").is_ok());
        assert!(parse_hotkey("Alt+F2").is_ok());
        assert!(parse_hotkey("Ctrl+1").is_ok());
        assert!(parse_hotkey("D").is_err()); // 缺修饰键
        assert!(parse_hotkey("Ctrl+").is_err());
    }
}
