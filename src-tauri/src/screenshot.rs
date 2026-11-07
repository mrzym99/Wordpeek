//! 截图翻译会话：触发抓屏定格、遮罩窗口、框选后 OCR+翻译。从 main.rs 整体迁出（纯移动不改逻辑）。

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

use crate::{translate, FlagGuard};

/// 截图进行中的去重标志：连按热键时后一场直接忽略（两个会话会抢同一扇遮罩窗）
static SCREENSHOT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// 权限缺失时弹原生对话框，一键跳到对应系统设置面板；只能在后台线程调用（blocking_show 会阻塞当前线程）
fn alert_permission(app: &AppHandle, message: &str, kind: crate::platform::PermissionKind) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

    let jump = app
        .dialog()
        .message(message)
        .title("Wordpeek 需要系统权限")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "打开系统设置".to_string(),
            "知道了".to_string(),
        ))
        .blocking_show();
    if jump {
        let _ = crate::platform::open_permission_settings(app, kind);
    }
}

/// 截图会话状态：触发时保存全屏快照与设置窗口可见性，finish 时再裁剪
#[derive(Default)]
pub(crate) struct ScreenshotState(parking_lot::Mutex<ScreenshotSession>);

#[derive(Default)]
struct ScreenshotSession {
    screen: Option<crate::capture::CapturedScreen>,
    settings_was_visible: bool,
    main_was_visible: bool,
}

/// 藏起自家窗口并返回藏之前的真实可见性（供事后恢复）；与 show_window 是截图会话专用的配对原语
fn hide_window(app: &AppHandle, label: &str) -> bool {
    match app.get_webview_window(label) {
        Some(w) if w.is_visible().unwrap_or(false) => {
            let _ = w.hide();
            true
        }
        _ => false,
    }
}

/// 按会话记录的可见性恢复窗口（和 hide_window 配对，各调用点共用同一对 label）
fn show_window(app: &AppHandle, label: &str, was_visible: bool) {
    if was_visible {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.show();
        }
    }
}

/// 截图翻译触发：抓触发瞬间的快照供裁剪兼当遮罩背景（画面定格），遮罩立即显示
pub(crate) fn handle_screenshot_trigger(app: &AppHandle) {
    // 连按热键：上一场截图还开着时直接忽略，否则两个会话抢同一扇遮罩窗
    if SCREENSHOT_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        println!("[screenshot] 上一场截图仍在进行，忽略本次触发");
        return;
    }
    println!("[screenshot] triggered");
    let app = app.clone();
    std::thread::spawn(move || {
        let _guard = FlagGuard(&SCREENSHOT_IN_PROGRESS);
        // 没权限时抓屏只会拿到桌面壁纸，必须明确引导而不是静默失败。只有 request 会弹授权框并把
        // Wordpeek 登记进「屏幕录制」列表（只 preflight 没法勾）；当场勾了也常要重启进程才生效，仍继续走提示
        if !crate::platform::screen_capture_allowed() {
            let granted = crate::platform::request_screen_capture();
            if !granted {
                alert_permission(
                    &app,
                    "截图翻译需要「屏幕录制」权限。\n\n系统设置 → 隐私与安全性 → 屏幕录制，勾选 Wordpeek 后重启应用。",
                    crate::platform::PermissionKind::ScreenCapture,
                );
                return;
            }
        }
        // 抓屏前先藏自家窗口：否则设置窗/结果卡片会被拍进截图，且新遮罩弹出时旧卡片还压在屏幕上
        let was_visible = hide_window(&app, "settings");
        let main_was_visible = hide_window(&app, "main");
        // 抓屏失败/找不到遮罩窗时要恢复刚才藏掉的窗口，否则设置窗凭空消失
        let restore_windows = || {
            show_window(&app, "settings", was_visible);
            show_window(&app, "main", main_was_visible);
        };
        let screen = match crate::platform::capture_virtual_screen() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[screenshot] 抓屏失败: {e}");
                restore_windows();
                return;
            }
        };
        // 快照定格成遮罩背景（微信截图式）：框选期间画面不再动，选到的就是所见那一帧；
        // 编码失败退回旧模式（透明遮罩对实时桌面，松手后仍按触发瞬间快照裁剪）
        let frozen = match crate::capture::encode_png(&screen.rgba, screen.width, screen.height) {
            Ok(png) => {
                use base64::Engine;
                format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(png)
                )
            }
            Err(e) => {
                eprintln!("[screenshot] 快照编码失败，退回实时桌面模式: {e}");
                String::new()
            }
        };
        let (sw, sh, sx, sy) =
            (screen.width, screen.height, screen.origin_x, screen.origin_y);

        let Some(win) = app.get_webview_window("screenshot") else {
            restore_windows();
            return;
        };
        // 遮罩窗口常驻铺满虚拟屏：位置/尺寸没变就不重设，避免 resize 引起闪动
        if let Ok(pos) = win.outer_position() {
            if pos.x != sx || pos.y != sy {
                let _ = win.set_position(tauri::PhysicalPosition::new(sx, sy));
            }
        }
        if let Ok(cur) = win.outer_size() {
            if cur.width != sw as u32 || cur.height != sh as u32 {
                let _ = win.set_size(tauri::PhysicalSize::new(sw, sh));
            }
        }

        // 存快照供 finish/cancel 清理（was_visible 是抓屏前读到的真实值，此刻窗口已藏起不能再 is_visible）
        let state = app.state::<ScreenshotState>();
        {
            let mut session = state.0.lock();
            session.screen = Some(screen);
            session.settings_was_visible = was_visible;
            session.main_was_visible = main_was_visible;
        }

        // 遮罩内容常驻就绪（纯 CSS），立即显示即可框选；screenshot-start 仅让前端重置选区
        let _ = app.emit_to(
            "screenshot",
            "screenshot-start",
            serde_json::json!({ "image": frozen }),
        );
        let _ = win.show();
        let _ = win.set_focus();
        crate::platform::prepare_overlay(&win);

        // 跨屏 set_position 触发 WM_DPICHANGED 后 tao 会异步按「保持逻辑尺寸」重设窗口，覆盖刚设的
        // 虚拟屏尺寸（首次遮罩只盖一块屏，第二次触发时 DPI 已稳定）。显示后校验几何，被 churn 改动就重设，连续两次稳定为止
        let mut stable = 0;
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let ok_size = win
                .outer_size()
                .map(|s| s.width == sw as u32 && s.height == sh as u32)
                .unwrap_or(true);
            let ok_pos = win
                .outer_position()
                .map(|p| p.x == sx && p.y == sy)
                .unwrap_or(true);
            if ok_size && ok_pos {
                stable += 1;
                if stable >= 2 {
                    break;
                }
            } else {
                stable = 0;
                if !ok_pos {
                    let _ = win.set_position(tauri::PhysicalPosition::new(sx, sy));
                }
                if !ok_size {
                    let _ = win.set_size(tauri::PhysicalSize::new(sw, sh));
                }
                println!(
                    "[screenshot] 遮罩几何被 DPI churn 改动，已重设（size_ok={ok_size} pos_ok={ok_pos}）"
                );
            }
        }
    });
}

/// 截图结果 payload：ctx 里带 sel/virtual/mouse/work 四组坐标（物理像素）供前端把卡片弹到框选位置附近；
/// 四个分支只差 ok/text/info/error，其余字段统一在这里拼，避免各分支手抄三遍
fn shot_payload(
    ok: bool,
    text: Option<String>,
    info: Option<translate::WordInfo>,
    error: Option<String>,
    ctx: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "ok": ok,
        "text": text,
        "info": info,
        "error": error,
        "sel": ctx["sel"],
        "virtual": ctx["virtual"],
        "mouse": ctx["mouse"],
        "work": ctx["work"],
    })
}

/// 前端框选完成：裁剪选区 → OCR → 翻译，结果发回主窗口弹卡片
#[tauri::command]
pub(crate) fn screenshot_finish(app: AppHandle, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let state = app.state::<ScreenshotState>();
    let mut session = state.0.lock();
    let screen = session.screen.take().ok_or("没有进行中的截图会话")?;
    let was_visible = session.settings_was_visible;
    let win = app.get_webview_window("screenshot").ok_or("截图窗口不存在")?;
    let _ = win.hide();
    // 松手瞬间的光标位置（物理像素），结果卡片弹到鼠标旁边；OCR 完再取会被用户移走
    let (mx, my) = crate::platform::cursor_pos();
    // 光标所在屏的工作区（排除任务栏），前端锚制用
    let (wx, wy, ww, wh) = crate::platform::work_area_at(mx, my);

    // 钳制到快照范围内（capture::clamp_rect，有单测）
    let rect = crate::capture::clamp_rect(x, y, w, h, screen.width, screen.height);
    let (cx, cy, cw, ch) = (rect.x, rect.y, rect.w, rect.h);

    let cropped = crate::capture::crop(&screen.rgba, screen.width, screen.height, rect)?;
    // 定位信息先提取，随后释放全屏像素（OCR 期间不再需要 ~23MB RGBA）
    let (vx, vy, vw, vh) = (screen.origin_x, screen.origin_y, screen.width, screen.height);
    drop(screen.rgba);
    let ctx = serde_json::json!({
        "sel": { "x": cx, "y": cy, "w": cw, "h": ch },
        "virtual": { "x": vx, "y": vy, "width": vw, "height": vh },
        "mouse": { "x": mx, "y": my },
        "work": { "x": wx, "y": wy, "width": ww, "height": wh },
    });
    let png = crate::capture::encode_png(&cropped, rect.w, rect.h)?;
    drop(session); // 后续流程不再需要会话锁

    // OCR/翻译耗时且走网络，放后台线程避免阻塞命令；结果发主窗口弹卡片
    std::thread::spawn(move || {
        let ocr_lang = app
            .state::<parking_lot::Mutex<translate::AppConfig>>()
            .lock()
            .ocr_lang
            .clone();
        let payload = match crate::ocr::recognize(&png, &ocr_lang) {
            Ok(lines) => {
                let text = lines
                    .iter()
                    .map(|l| l.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.trim().is_empty() {
                    shot_payload(false, None, None, Some("未识别到文字，请框选包含文字的区域".into()), &ctx)
                } else {
                    // 翻译（主源→降级；整句需百度 key，错误信息会引导）
                    let cfg = app
                        .state::<parking_lot::Mutex<translate::AppConfig>>()
                        .lock()
                        .clone();
                    // OCR 文本中英不定：按内容自动检测目标语言
                    let to = translate::detect_to(&text);
                    match tauri::async_runtime::block_on(crate::translate::translate_text(
                        &cfg, &text, to
                    )) {
                        Ok(info) => shot_payload(true, Some(text), Some(info), None, &ctx),
                        Err(e) => shot_payload(true, Some(text), None, Some(e), &ctx),
                    }
                }
            }
            Err(e) => shot_payload(false, None, None, Some(e), &ctx),
        };
        let _ = app.emit_to("main", "screenshot-result", payload);

        // 截图前设置窗口可见则恢复
        show_window(&app, "settings", was_visible);
    });
    Ok(())
}

/// 前端按 ESC 或窗口失焦取消：丢弃会话并隐藏遮罩
#[tauri::command]
pub(crate) fn screenshot_cancel(app: AppHandle) {
    let (was_visible, main_was_visible);
    {
        let state = app.state::<ScreenshotState>();
        let mut session = state.0.lock();
        was_visible = session.settings_was_visible;
        main_was_visible = session.main_was_visible;
        session.screen = None;
    }
    if let Some(win) = app.get_webview_window("screenshot") {
        let _ = win.hide();
    }
    show_window(&app, "settings", was_visible);
    show_window(&app, "main", main_was_visible);
    println!("[screenshot] cancelled");
}
