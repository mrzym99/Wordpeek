#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

mod capture;
mod geometry;
mod hotkeys;
mod ocr;
mod screenshot;
mod windows;
mod updater;
#[cfg(any(windows, target_os = "macos"))]
mod platform;
mod translate;
mod tray;
mod selection;

// 平台相关实现（光标/工作区/复制注入/抓屏/权限）在 platform/windows.rs 与 platform/macos.rs

/// RAII 复位标志；guard 被丢弃（含提前 return / panic 展开）时释放互斥
pub(crate) struct FlagGuard(pub(crate) &'static AtomicBool);
impl Drop for FlagGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 全局最近一次鼠标按下的 (物理 x, 物理 y, epoch ms)。弹窗卡片在 macOS makeKey 失灵
/// （焦点没立起来 → 无 blur 通道）时靠它判「show 之后是否点过卡片外」，点了外部即收
#[tauri::command]
fn last_mouse_down() -> Option<(i32, i32, u64)> {
    selection::gesture::last_mouse_down()
}

/// 全局当前光标位置（物理像素）。卡片收起判定要看「鼠标是否真的在卡片上」，
/// 不能用 DOM 的 :hover —— 窗口拿不到鼠标事件时 WebKit 会让它永久停在 true
#[tauri::command]
fn cursor_pos() -> (i32, i32) {
    #[cfg(any(windows, target_os = "macos"))]
    return platform::cursor_pos();
    #[cfg(not(any(windows, target_os = "macos")))]
    (0, 0)
}

/// 复制文本到剪贴板（结果卡片复制按钮）
#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| format!("复制失败: {e}"))
}

/// 系统权限状态（设置页「系统权限」区块展示 + 跳系统设置；Windows 恒可用，macOS 实现在 platform/macos.rs）
#[derive(serde::Serialize)]
struct PermissionStatus {
    accessibility: bool,
    screen_capture: bool,
}

#[tauri::command]
fn check_permissions() -> PermissionStatus {
    #[cfg(any(windows, target_os = "macos"))]
    return PermissionStatus {
        accessibility: platform::accessibility_allowed(),
        screen_capture: platform::screen_capture_allowed(),
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    PermissionStatus {
        accessibility: true,
        screen_capture: true,
    }
}

/// 打开某项权限的系统设置面板（仅 macOS 有权限页；Windows 返回错误）
#[tauri::command]
fn open_permission_settings(app: tauri::AppHandle, kind: String) -> Result<(), String> {
    let kind = platform::PermissionKind::parse(&kind).ok_or("未知权限类型")?;
    // 屏幕录制面板只列出「申请过权限」的 App：没申请过时先触发一次系统授权框完成登记。
    // 放后台线程，避免系统模态框卡住命令线程；首次点击后系统一般不会再重复弹。
    // 已弹过系统框就跳过，否则用户点应用卡片里的「打开系统设置」会又撞见系统框
    #[cfg(target_os = "macos")]
    if kind == platform::PermissionKind::ScreenCapture
        && !platform::screen_capture_allowed()
        && !platform::screen_capture_prompted(&app)
    {
        platform::mark_screen_capture_prompted(&app);
        std::thread::spawn(|| {
            let _ = platform::request_screen_capture();
        });
    }
    platform::open_permission_settings(&app, kind)
}

fn main() {
    // 第一件事：拆 stdout/stderr 死管道炸弹（必须在任何 println! 之前）。
    // dev 构建不拆：cargo tauri dev 的控制台日志全靠 npm 那根管道
    #[cfg(windows)]
    if cfg!(not(debug_assertions)) {
        platform::sanitize_stdio();
    }

    let builder = tauri::Builder::default();
    builder
        // 单实例：二次启动把设置窗拉到前台；必须排在其它 plugin 之前（官方要求），否则可能先拿到已注册的系统资源
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            windows::open_settings(app.clone());
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 平台初始化（macOS：Dock 策略 + 主线程缓存屏幕信息）
            #[cfg(any(windows, target_os = "macos"))]
            platform::init(&app.handle());

            // 启动时加载翻译配置（有默认值，配置文件不存在也能跑）
            let handle = app.handle().clone();
            app.manage(parking_lot::Mutex::new(translate::load_config(&handle)));

            // 恢复用户拖过的弹窗宽度（高度由前端自适应，不持久化）
            if let Some(win) = app.get_webview_window("main") {
                let w = handle
                    .state::<parking_lot::Mutex<translate::AppConfig>>()
                    .lock()
                    .popup_width;
                if let Ok(sz) = win.inner_size() {
                    let s = win.scale_factor().unwrap_or(1.0);
                    let _ = win.set_size(tauri::LogicalSize::new(w, sz.height as f64 / s));
                }
            }

            app.manage(updater::UpdateState::default());
            // macOS 手动更新：缓存最近一次查到的远端版本号，供设置页打开时秒回
            app.manage(updater::LatestVersionState::default());
            app.manage(screenshot::ScreenshotState::default());
            app.manage(selection::SessionManager::default());
            
            // Win11 给透明弹窗再叠一层 ~8px 系统圆角，弹窗类全关掉只留 CSS 一层（设置/输入窗有标题栏，保留系统圆角）
            #[cfg(windows)]
            for label in ["main", "icon", "screenshot"] {
                if let Some(win) = app.get_webview_window(label) {
                    platform::disable_rounding(&win);
                }
            }

    // panic 钩子：tokio 任务里的 panic 默认只打到 stderr（release 下不可见），落一份到配置目录 panic.log 供排查
    let panic_dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    std::panic::set_hook(Box::new(move |info| {
        translate::log_panic(&panic_dir, &info.to_string());
    }));

            // 划词图标：全局鼠标监听（跟设置里的开关无关，开关在探测时现读）
            selection::start_selection_icon(&handle);

            // 启动后后台把截图遮罩窗口预铺到虚拟屏尺寸（常驻隐藏），首次截图无需 resize，避免闪动
            let shot_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                if let (Some((ox, oy, w, h)), Some(win)) = (
                    crate::platform::virtual_screen_bounds(),
                    shot_handle.get_webview_window("screenshot"),
                ) {
                    let _ = win.set_position(tauri::PhysicalPosition::new(ox, oy));
                    let _ = win.set_size(tauri::PhysicalSize::new(w as u32, h as u32));
                }
            });

            // 预热 OCR（仅 macOS）：首次调 Vision 加载模型要 30-60s，不预热第一次截图翻译会卡半分钟；Windows 无需预热
            #[cfg(target_os = "macos")]
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(2));
                crate::ocr::warmup();
            });

            // 预热 UIA（仅 Windows）：进程首次 COM/UIA 初始化要加载 UIA 核心，耗时可达秒级，
            // 不预热的话第一次划词探测被拖慢，结果因代次过期被丢弃，图标就不弹
            // （热键路径会顺带预热，但用户不该先按一次热键才正常）
            #[cfg(windows)]
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(1));   // 错开启动高峰
                crate::platform::warm_uia();
            });

            // 启动后延迟几秒静默检查一次更新，结果缓存供设置页查询。
            // macOS 也查：只读 latest.json 的版本号（没有可安装的更新包，查到新版本时前端给下载按钮）
            {
                let updater_handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(6));
                    if let Err(e) = tauri::async_runtime::block_on(updater::check_update(&updater_handle)) {
                        eprintln!("[updater] startup check failed: {e}");
                    }
                });
            }

            // 设置窗口按需创建（open_settings），点 ✕ 即销毁，无需关闭时隐藏的守卫

            tray::setup_tray(app)?;

            // 启动提示：屏上 HUD 胶囊 1.2s 自动关，mac/Windows 同一套（原 Windows 系统 toast 驻留约 7s 且 mac 不支持）
            let hud_handle = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                windows::show_hud(&hud_handle, "Wordpeek 已启动，划词翻译正在后台运行");
            });

            let cfg = app
                .state::<parking_lot::Mutex<translate::AppConfig>>()
                .lock()
                .clone();
            hotkeys::register_hotkeys(&handle, &cfg);
            // 启动时注册失败只打日志：窗口还没起来，无处展示；设置页保存时会二次注册并报错

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_permissions,
            open_permission_settings,
            updater::restart_app,
            windows::open_settings,
            updater::get_update_status,
            updater::check_update_now,
            updater::install_update,
            screenshot::screenshot_finish,
            screenshot::screenshot_cancel,
            windows::fit_window,
            windows::hud_take_text,
            copy_text,
            last_mouse_down,
            cursor_pos,
            selection::icon::translate_selection_icon,
            translate::translate,
            translate::translate_all,
            translate::get_config,
            translate::save_sources,
            translate::save_baidu,
            translate::save_tencent,
            translate::save_settings,
            translate::save_popup_width,
            translate::save_settings_section,
            translate::save_theme
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| {
            // 系统深浅色切换的原生信号在这里最可靠：Windows 的 WebView2 会翻转
            // prefers-color-scheme 状态但不向页面派发 change 回调。统一广播解析
            // 值，各窗口前端按自己的设置（system 档才跟随）落地 DOM 主题
            if let tauri::RunEvent::WindowEvent {
                event: tauri::WindowEvent::ThemeChanged(theme),
                ..
            } = event
            {
                let _ = app_handle.emit(
                    "system-theme-changed",
                    match theme {
                        tauri::Theme::Dark => "dark",
                        _ => "light",
                    },
                );
                // 只管「跟随系统」档：强制档由前端显式 setTheme(具体值) 走 tao 自己的
                // 重绘路径，这里不能越俎代庖把它画回去
                #[cfg(windows)]
                if app_handle
                    .state::<parking_lot::Mutex<translate::AppConfig>>()
                    .lock()
                    .theme
                    == "system"
                {
                    // 标题栏直接 Win32 补一刀：tao 在 WM_SETTINGCHANGE 路径只写 DWM
                    // 属性不触发重绘（redraw=false）。绝不可改用 window.set_theme 做
                    // 「钉住再解除」——那会在同一事件迭代里重入 tao 的主题状态机，
                    // 读数与钉值不一致时两个 ThemeChanged 来回振荡，事件风暴卡死整界面
                    use ::windows::Win32::Graphics::Dwm::{
                        DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
                    };
                    use ::windows::Win32::UI::WindowsAndMessaging::{
                        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                        SWP_NOZORDER,
                    };
                    let dark = ::windows::Win32::Foundation::BOOL::from(matches!(
                        theme,
                        tauri::Theme::Dark
                    ));
                    for (_label, window) in app_handle.webview_windows() {
                        // 无标题栏的透明弹窗（main/icon/screenshot）跳过，避免无谓的 NC 重算
                        if !window.is_decorated().unwrap_or(false) {
                            continue;
                        }
                        let Ok(hwnd) = window.hwnd() else {
                            continue;
                        };
                        // tauri 的 HWND 来自另一版 windows crate，按裸指针转成本地类型
                        let hwnd = ::windows::Win32::Foundation::HWND(hwnd.0 as *mut _);
                        unsafe {
                            let _ = DwmSetWindowAttribute(
                                hwnd,
                                DWMWA_USE_IMMERSIVE_DARK_MODE,
                                &dark as *const _ as *const core::ffi::c_void,
                                std::mem::size_of::<::windows::Win32::Foundation::BOOL>() as u32,
                            );
                            // 强制 NC 重算/重绘，让刚写入的 DWM 属性立刻显形
                            let _ = SetWindowPos(
                                hwnd,
                                None,
                                0,
                                0,
                                0,
                                0,
                                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
                                    | SWP_FRAMECHANGED,
                            );
                        }
                    }
                }
            }
        });
}
