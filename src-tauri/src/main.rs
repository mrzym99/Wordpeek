#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use arboard::Clipboard;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_updater::{Update, UpdaterExt};

mod capture;
mod ocr;
#[cfg(any(windows, target_os = "macos"))]
mod platform;
mod translate;

// 平台相关实现（光标/工作区/复制注入/抓屏/权限）全在 platform/ 模块：
// Windows → platform/windows.rs，macOS → platform/macos.rs

#[derive(serde::Serialize, Clone)]
struct Selection {
    text: String,
    x: i32,
    y: i32,
}

/// 抓取互斥量：grab_selection 会改写剪贴板（备份→哨兵→恢复），
/// 两个并发实例交错会把用户的剪贴板顶成哨兵串，必须串行
static GRAB_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
/// 截图会话互斥量：遮罩窗是全局唯一的，连续触发会两个会话抢同一扇遮罩
static SCREENSHOT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// RAII 复位标志；guard 被丢弃（含提前 return / panic 展开）时释放互斥
struct FlagGuard(&'static AtomicBool);
impl Drop for FlagGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// 模拟复制键抓取当前选中文本，并把原剪贴板内容还原
fn grab_selection(app: &AppHandle) -> Option<String> {
    // 已有一轮在跑（热键连按）：直接放弃本次，上一轮的结果会正常弹出
    if GRAB_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        println!("[grab] 上一轮抓取仍在进行，跳过本次触发");
        return None;
    }
    let _guard = FlagGuard(&GRAB_IN_PROGRESS);
    // 诊断打印：foreground_app 只有 Windows 实现（macOS 要走 NSWorkspace，暂不缺）
    #[cfg(windows)]
    println!("[grab] foreground app: {}", crate::platform::foreground_app());
    let mut clipboard = Clipboard::new().ok()?;

    // 备份原剪贴板：文本优先；读不到文本（可能是图片/文件）就备份图片，
    // 否则恢复时会把用户复制的非文本内容顶成空文本
    let old_text = clipboard.get_text().ok();
    let old_image = if old_text.is_none() {
        clipboard.get_image().ok()
    } else {
        None
    };

    // 先把剪贴板替换成哨兵值：若 Ctrl+C 成功，剪贴板必然变成选中文本。
    // 不能用"剪贴板内容是否变化"来判断，否则选中与剪贴板相同的文本时会被误判。
    const SENTINEL: &str = "\u{200b}__pot-mini-sentinel__\u{200b}";
    clipboard.set_text(SENTINEL).ok()?;

    // 松开物理按着的修饰键再注入复制键（Windows: Ctrl+C；macOS: Cmd+C）
    let injected = crate::platform::inject_copy(app);
    let ax = crate::platform::accessibility_allowed();
    if !injected || !ax {
        println!("[grab] inject_copy={injected}, accessibility={ax}");
    }

    // 目标应用写入选中文本有快有慢（PDF、大文档可能超过百毫秒），
    // 固定 sleep 会在慢应用上误判“没选中内容”，且恢复时可能被慢应用
    // 后写入的内容覆盖；轮询到内容不再是哨兵值为止，最多 ~600ms。
    // 首轮没等到就再注入一次复制键再等 ~300ms：偶发目标应用吞掉第一次 Ctrl+C
    // （焦点刚切回来/输入法吞键），重试一次能救回来。
    let mut text = SENTINEL.to_string();
    if injected {
        for attempt in 0..2 {
            let rounds = if attempt == 0 { 30 } else { 15 };
            for _ in 0..rounds {
                std::thread::sleep(std::time::Duration::from_millis(20));
                text = clipboard.get_text().unwrap_or_default();
                if text != SENTINEL {
                    break;
                }
            }
            if text != SENTINEL {
                break;
            }
            if attempt == 0 {
                println!("[grab] 首轮复制未生效，重试注入一次");
                if !crate::platform::inject_copy(app) {
                    break;
                }
            }
        }
    }

    // 恢复原剪贴板（文本或图片）
    if let Some(t) = old_text {
        let _ = clipboard.set_text(&t);
    } else if let Some(img) = old_image {
        let _ = clipboard.set_image(img);
    }

    // 剪贴板还是哨兵值说明复制没有生效（按键注入失败或目标应用没有可复制的内容）
    if text == SENTINEL {
        println!("[grab] clipboard 未被替换：目标应用没响应复制键");
        return None;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        println!("[grab] clipboard 被清空：选中内容为空");
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 内容高度 → 窗口客户区（inner）逻辑高度：下限 60px，上限为工作区（逻辑像素）扣掉标题栏
/// 再留 40px 边距，内容真超出才内部滚动（拿不到工作区信息时退回 800px；工作区极小时仍不低于下限）
fn fit_height(content_h: f64, work_h: f64, chrome_h: f64) -> f64 {
    let max = if work_h > 0.0 {
        work_h - chrome_h - 40.0
    } else {
        800.0
    };
    content_h.clamp(60.0, max.max(60.0))
}

/// 逻辑像素 → 物理像素（向上语义：四舍五入，至少 1px）。显示器矩形/位置都是物理像素，
/// 缩放 ≠100% 时钳位前必须先乘 scale，否则钳不住，窗口会伸出屏幕外
fn to_physical(logical: f64, scale: f64) -> i32 {
    (logical * scale).round().max(1.0) as i32
}

/// 把「左上角在 (x, y)、尺寸 win_w×win_h」的窗口钳进显示器矩形内。
/// 窗口比屏还大时贴左上角（不弹到屏外）。
fn clamp_rect_to_monitor(
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
fn monitor_rect_at(win: &tauri::WebviewWindow, x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
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

/// 位置钳制矩形：优先工作区（macOS 排除菜单栏与 Dock，Windows 排除任务栏），
/// 拿不到工作区（全 0）才退回显示器整块。用整块钳会让贴底的卡片钻到 Dock 下面。
fn clamp_rect_prefer_work(
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
fn usable_rect_at(win: &tauri::WebviewWindow, x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    clamp_rect_prefer_work(
        crate::platform::work_area_at(x, y),
        monitor_rect_at(win, x, y),
    )
}

/// 把弹窗位置钳制在光标所在屏幕内，避免边缘选词时弹窗伸出屏幕外。
/// 任一步骤拿不到信息（找不到窗口/显示器）就原样返回，不钳制。
fn clamp_to_screen(app: &AppHandle, label: &str, x: i32, y: i32) -> (i32, i32) {
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
fn viewport_inset(label: &str, win_h: f64, viewport_h: f64) -> f64 {
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

/// 前端量好卡片内容高度后调用：把窗口调整到刚好包住卡片，并重新钳制位置。
/// 只改高度——宽度保持用户当前宽度（长文本可手动拉宽）。
/// 位置钳制用「自己算出的新外框尺寸」，不依赖 set_size 之后立刻回读（那是异步消息）。
/// 返回实际生效的窗口高度（可能被屏幕上限钳小）：前端用它判断「内容真的
/// 装不下了」（is-capped），没顶到上限前一律不出滚动条，避免高度动画中闪条
#[tauri::command]
fn fit_window(app: AppHandle, label: String, height: f64, viewport: Option<f64>) -> Result<f64, String> {
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
    // 高度上限用窗口所在屏的工作区（排除任务栏）；work_area_at 给的是物理像素，
    // 内容高度是逻辑像素，先除以缩放再钳制（多屏不同 DPI 时上限才不偏大）
    let (_, _, _, work_h) = crate::platform::work_area_at(pos.x + 8, pos.y + 8);
    let inner_w = inner.width as f64 / scale;
    // 带标题栏/边框的窗口（输入翻译窗）里，窗口高 ≠ webview 视口高：前端拿到的 100vh 少一条标题栏，
    // 直接按内容 set_size 会被吃掉 32px。无边框弹窗（划词）差值为 0，行为不变。
    // 差值只测一次并缓存：前端报上来的视口高可能落后于刚 set_size 的窗口，现算会把差值放大，
    // 窗口越长越离谱（实测一次长出 200px 的空白）
    let chrome_extra = viewport.map(|vh| viewport_inset(&label, inner.height as f64 / scale, vh)).unwrap_or(0.0);
    let inner_h = fit_height(
        height + chrome_extra,
        if work_h > 0 { work_h as f64 / scale } else { 0.0 },
        chrome_h,
    );
    win.set_size(tauri::LogicalSize::new(inner_w, inner_h))
        .map_err(|e| e.to_string())?;

    // 新外框尺寸 = 客户区 + 边框/标题栏，用它重新钳一次位置。
    // 逻辑尺寸必须换算成物理像素再和显示器矩形比：缩放 150% 的屏上把逻辑高当
    // 物理高钳位，卡片会伸出屏幕底 (scale−1)×高，内容被裁掉又没滚动条，只能拖窗口（issue #8）
    let new_w = to_physical(inner_w + chrome_w, scale);
    let new_h = to_physical(inner_h + chrome_h, scale);
    let (nx, ny) = match usable_rect_at(&win, pos.x + 8, pos.y + 8) {
        Some(rect) => clamp_rect_to_monitor(rect, new_w, new_h, pos.x, pos.y),
        None => (pos.x, pos.y),
    };
    win.set_position(tauri::PhysicalPosition::new(nx, ny))
        .map_err(|e| e.to_string())?;
    Ok(inner_h)
}

/// 把窗口摆到主屏工作区中央（macOS 上 Dock / 菜单栏、Windows 上任务栏不会遮住它）。
/// 不用 win.center()：它按整块屏幕居中，而且窗口还隐藏时可能取不到屏幕直接 no-op。
fn center_window(win: &tauri::WebviewWindow) {
    let Ok(Some(mon)) = win.primary_monitor() else {
        return;
    };
    let p = mon.position();
    let s = mon.size();
    let full = (p.x, p.y, s.width as i32, s.height as i32);
    let rect = clamp_rect_prefer_work(
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

/// 已存在则 show+置前；没建过就在事件循环线程按需创建为隐藏态（内容就绪后由前端自己 show）。
/// 首次创建摆到屏幕中央；已打开时只 show，尊重用户拖到的位置。
/// macOS：页面画到标题栏下面（Overlay），头部底色由前端画，交通灯浮在上面。
/// drag_drop: true 时加 disable_drag_drop_handler（设置窗里要拖拽排序，输入窗不需要）
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

/// 显示独立的设置窗口（弹窗齿轮按钮 / 托盘菜单共用）。
/// 点 ✕ 即销毁，回收它那个常驻 WebView2 renderer；一级菜单选择已存配置，
/// 重开时设置页自动回到上次停留的菜单
#[tauri::command]
fn open_settings(app: AppHandle) {
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

/// 显示输入翻译窗口（托盘菜单 / 输入翻译快捷键共用）。
/// 点 ✕ 即销毁，回收它那个常驻 WebView2 renderer（~60MB）
fn open_input_window(app: &AppHandle) {
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

/// 已缓存的可用更新（启动 / 手动检查后写入）
type UpdateState = parking_lot::Mutex<Option<Update>>;

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

/// 检查更新并把结果缓存到应用状态（async：同步命令在主线程跑，block_on 会冻结整个 UI）
async fn check_update(app: &AppHandle) -> Result<UpdateStatus, String> {
    // macOS 端还没有签名/公证的产物，latest.json 里也没有 darwin 平台，检查只会报错
    if cfg!(target_os = "macos") {
        return Err("macOS 版暂不提供自动更新，请到 GitHub Releases 手动下载".into());
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let status = status_of(app, update.as_ref());
    println!("[updater] check: {status:?}");
    *app.state::<UpdateState>().lock() = update;
    Ok(status)
}

/// 设置页打开时查询缓存的检查结果（不发网络请求）
#[tauri::command]
fn get_update_status(app: AppHandle) -> UpdateStatus {
    let cached = app.state::<UpdateState>().lock().as_ref().cloned();
    status_of(&app, cached.as_ref())
}

/// 手动检查更新（设置页「检查更新」按钮）。async 命令由 tauri 丢到异步运行时，
/// 网络等待期间主线程（事件循环）不阻塞，弱网下应用不再假死
#[tauri::command]
async fn check_update_now(app: AppHandle) -> Result<UpdateStatus, String> {
    check_update(&app).await
}

/// 重启自己。改完系统权限后必须换进程才生效，而 macOS 那个「退出并重新打开」
/// 只对 .app 包有效——从终端起的进程被关掉就没了，所以给个应用内按钮。
#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

/// 下载并静默安装已缓存的更新，完成后重启应用（后台线程执行）
#[tauri::command]
fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .state::<UpdateState>()
        .lock()
        .take()
        .ok_or_else(|| "没有已缓存的更新".to_string())?;
    std::thread::spawn(move || {
        println!("[updater] downloading & installing...");
        let install = update.download_and_install(|_chunk, _total| {}, || {});
        if let Err(e) = tauri::async_runtime::block_on(install) {
            eprintln!("[updater] install failed: {e}");
            // 失败把已下载的更新放回缓存，用户重试不用再走一遍检查更新
            *app.state::<UpdateState>().lock() = Some(update);
            return;
        }
        // NSIS 静默安装结束后旧进程通常已被安装器接管，restart 兜底
        app.restart();
    });
    Ok(())
}

/// 解析 "Alt+Z" 形式的快捷键字符串
pub(crate) fn parse_hotkey(s: &str) -> Result<Shortcut, String> {
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

/// 划词图标最近一次探测到的选中文本：点图标时直接用。
/// 点击会把焦点转给我们的窗口，那时候再去读源应用的选区已经读不到了
#[derive(Default)]
struct IconSelection(parking_lot::Mutex<Option<String>>);

/// 刚点过图标的时刻（毫秒）。点击的那一下会先收起图标，紧接着的鼠标松开事件
/// 还会触发一次探测——那时源应用的选区还在（点我们自己的窗口不会取消选区），
/// 图标就会立刻又弹回来。所以点完图标 400ms 内的探测直接跳过
static ICON_CLICKED_AT: AtomicU64 = AtomicU64::new(0);

/// 左键按下的位置与时刻（物理像素, 毫秒）：用来区分「选词手势」和普通单击
static MOUSE_DOWN: std::sync::OnceLock<parking_lot::Mutex<(i32, i32, u64)>> =
    std::sync::OnceLock::new();
/// 上一次松手的位置与时刻（同上），双击/三击判定用
static MOUSE_UP: std::sync::OnceLock<parking_lot::Mutex<(i32, i32, u64)>> =
    std::sync::OnceLock::new();
/// 本次松手是否值得探测选区；mouse-up 回调里算好，worker 直接读
static PROBE_INTENT: AtomicBool = AtomicBool::new(false);
/// 选区代次：每次按下 +1。探测在独立线程里异步做（UIA 重试要 sleep，
/// 不能堵 worker，否则后续鼠标信号会撑爆 sync_channel 被丢弃），
/// show 前对一下代次，期间有新的按下（用户已开始新选区）就放弃本次结果
static SELECTION_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 本次按下时光标是否落在划词图标矩形内（钩子回调只做廉价判断，hide 由 worker 做）
static DOWN_ON_ICON: AtomicBool = AtomicBool::new(false);
/// 划词图标最近一次 show 的屏幕矩形（物理像素 x,y,w,h）：钩子回调用它判断
/// 点下的是不是图标自己，避免在回调里跨线程查窗口位置/尺寸
static ICON_RECT: parking_lot::Mutex<Option<(i32, i32, u32, u32)>> = parking_lot::Mutex::new(None);

fn record_mouse_down() {
    let (x, y) = crate::platform::cursor_pos();
    *MOUSE_DOWN.get_or_init(Default::default).lock() = (x, y, now_ms());
}

/// 只有真的做了选词手势才探测：拖动（位移 ≥ 6 物理像素）或连击（350ms 内同一点再松手，
/// 双击选词/三击选行）。普通单击不算——很多应用会记住上一次选区，单击后
/// AXSelectedText 照样返回旧文本，不拦就会出现「随便点一下也冒图标」。
/// 位移阈值：曼哈顿距离超过 6px 视为拖拽选区（低于此值是普通点击的抖动）
const GESTURE_MOVE_PX: i32 = 6;
/// 连击判定：按下与上次松手间隔 < 500ms 且同一点（±8px）→ 双击选词/三击选行
/// （Windows 系统双击默认 500ms，取同值避免手慢的双击被判成两次单击）
const GESTURE_REPEAT_MS: u64 = 500;
const GESTURE_REPEAT_PX: i32 = 8;

/// 纯判定逻辑，供测试：本次按下/松开与上一次松开（px,py,prev_at）的坐标时间
fn gesture_selects(
    down: (i32, i32, u64),
    up: (i32, i32),
    prev_up: (i32, i32, u64),
) -> bool {
    let (dx, dy, down_at) = down;
    let (x, y) = up;
    let (px, py, prev_at) = prev_up;
    let moved = (x - dx).abs() + (y - dy).abs() >= GESTURE_MOVE_PX;
    // 本次按下紧跟上一次松手（350ms 内、同一点）= 连击：双击选词/三击选行
    let repeated = prev_at != 0
        && down_at.saturating_sub(prev_at) < GESTURE_REPEAT_MS
        && (dx - px).abs() + (dy - py).abs() < GESTURE_REPEAT_PX;
    moved || repeated
}

fn mouse_gesture_selects(x: i32, y: i32) -> bool {
    let down = { *MOUSE_DOWN.get_or_init(Default::default).lock() };
    let (px, py, prev_at) = {
        let mut up = MOUSE_UP.get_or_init(Default::default).lock();
        let prev = *up;
        *up = (x, y, now_ms());
        prev
    };
    gesture_selects(down, (x, y), (px, py, prev_at))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 光标是否落在指定窗口内（物理像素）。取不到窗口/位置按不在处理
fn cursor_on_window(app: &AppHandle, label: &str) -> bool {
    let Some(win) = app.get_webview_window(label) else {
        return false;
    };
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return false;
    };
    let (x, y) = crate::platform::cursor_pos();
    x >= pos.x && x < pos.x + size.width as i32 && y >= pos.y && y < pos.y + size.height as i32
}

fn hide_selection_icon(app: &AppHandle) {
    *ICON_RECT.lock() = None;
    if let Some(win) = app.get_webview_window("icon") {
        let _ = win.hide();
    }
}

/// 在松手位置显示图标（窗口已在 tauri.conf.json 里预建，这里只是挪位置 + show）。
/// 锚定松手位置而不是实时光标：探测是异步的（要等 120ms+），用实时位置的话
/// 用户探测期间挪开鼠标，图标会追着光标在别处冒出来
fn show_selection_icon(app: &AppHandle, text: String, ax: i32, ay: i32) {
    *app.state::<IconSelection>().0.lock() = Some(text);
    let app = app.clone();
    let inner = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = inner.get_webview_window("icon") {
            let (x, y) = clamp_to_screen(&inner, "icon", ax + 8, ay + 12);
            let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
            if let Ok(size) = win.outer_size() {
                *ICON_RECT.lock() = Some((x, y, size.width, size.height));
            }
            let _ = win.show();
        }
    });
}

/// 光标是否落在缓存的图标矩形内。只在钩子回调用：只碰 GetCursorPos + 内存读，
/// 绝不做跨线程窗口查询/ShowWindow（回调慢了全系统鼠标卡，超时还会被系统摘钩）
fn cursor_on_icon_rect() -> bool {
    let Some((x, y, w, h)) = *ICON_RECT.lock() else {
        return false;
    };
    let (cx, cy) = crate::platform::cursor_pos();
    cx >= x && cx < x + w as i32 && cy >= y && cy < y + h as i32
}

/// 划词图标：鼠标松开后读一次系统选区（不碰剪贴板、不注入按键），有选中文字就在
/// 光标旁弹按钮，点一下走热键同一套翻译链路。读不到选区（终端、Electron 等
/// 不实现 AX/UIA 文本接口的应用）就什么都不做，用户照旧用划词快捷键。
fn start_selection_icon(app: &AppHandle) {
    /// 鼠标信号：Down=按下、Up=松开（带松手位置，锚点用）、Leave=图标可见时鼠标飘远
    enum Signal {
        Down,
        Up { x: i32, y: i32 },
        Leave,
    }
    // 队列只留一个信号：钩子回调绝不做窗口操作（跨线程查询/ShowWindow 会拖慢全系统鼠标，
    // 超时还会被 Windows 静默摘钩），hide 一律在 worker 线程里做
    let (tx, rx) = std::sync::mpsc::sync_channel::<Signal>(1);
    let tx_down = tx.clone();
    let tx_move = tx.clone();
    /// 光标离开图标矩形这么远（物理像素）就收起图标：说明用户已经去别处了，
    /// 图标留在原地碍事。留这一圈余量是给「把鼠标挪过去点图标」的路径
    const ICON_LEAVE_PX: i32 = 48;
    /// 探测完成时（要等 120ms+）光标离松手点超过这么远就丢弃结果：用户已去别处，
    /// 且很多应用会一直报旧选区，人不在那就不该弹
    const ICON_NEAR_PX: i32 = 150;
    let watch = crate::platform::start_mouse_watch(
        Box::new(move || {
            // 点在图标窗口自身上时不能收起：Windows 低级钩子连自己窗口的鼠标事件也收，
            // down 就把窗口藏掉会吞掉这次 click（macOS 全局监视器不收自身应用的事件，无此问题）
            DOWN_ON_ICON.store(cursor_on_icon_rect(), Ordering::Relaxed);
            record_mouse_down();
            let _ = tx_down.try_send(Signal::Down);
        }),
        Box::new(move || {
            let (x, y) = crate::platform::cursor_pos();
            PROBE_INTENT.store(mouse_gesture_selects(x, y), Ordering::Relaxed);
            // 队列里只留一个待处理的探测：连点鼠标时不堆积，也不会每点一下就开条线程
            let _ = tx.try_send(Signal::Up { x, y });
        }),
        // 移动：图标可见且光标已飘远（要开始别的事了）→ 发 Leave 收起。
        // 只碰内存读 + 坐标比较，保持回调轻量；图标已收起时这里近乎免费
        Box::new(move |x, y| {
            if let Some((ix, iy, iw, ih)) = *ICON_RECT.lock() {
                if x < ix - ICON_LEAVE_PX
                    || x >= ix + iw as i32 + ICON_LEAVE_PX
                    || y < iy - ICON_LEAVE_PX
                    || y >= iy + ih as i32 + ICON_LEAVE_PX
                {
                    let _ = tx_move.try_send(Signal::Leave);
                }
            }
        }),
    );
    println!(
        "[icon] 鼠标监听已装，辅助功能={}",
        crate::platform::accessibility_allowed()
    );
    if !watch {
        eprintln!("[icon] 鼠标监听装不上，划词图标不可用");
        return;
    }
    let worker = app.clone();
    std::thread::spawn(move || {
        while let Ok(sig) = rx.recv() {
            match sig {
                // 按下/飘远：收起图标（开始新一轮选区）；点在图标上按下则保留，
                // 等随后的 click 命令（translate_selection_icon）接管
                Signal::Down | Signal::Leave => {
                    SELECTION_GEN.fetch_add(1, Ordering::Relaxed);
                    if matches!(sig, Signal::Leave)
                        || !DOWN_ON_ICON.load(Ordering::Relaxed)
                    {
                        hide_selection_icon(&worker);
                    }
                }
                Signal::Up { x, y } => self::probe_selection(&worker, x, y, ICON_NEAR_PX),
            }
        }
    });
}

/// 松开且是选词手势：探测选区，成功且用户还没走远就在松手位置弹图标
fn probe_selection(worker: &AppHandle, ux: i32, uy: i32, near_px: i32) {
    // 开关每次现读配置：设置里一关就立即生效，不用重装监听
    let enabled = worker
        .state::<parking_lot::Mutex<translate::AppConfig>>()
        .lock()
        .selection_icon;
    if !enabled
        || !PROBE_INTENT.load(Ordering::Relaxed)
        || now_ms().saturating_sub(ICON_CLICKED_AT.load(Ordering::Relaxed)) < 400
    {
        hide_selection_icon(worker);
        return;
    }
    // 在自家窗口里划词（卡片内选中跳查、设置页复制等）不弹图标，
    // 否则用户在自己卡片里选个词，光标旁边又冒出一个翻译图标
    if ["main", "icon", "settings", "input"]
        .iter()
        .any(|&l| cursor_on_window(worker, l))
    {
        hide_selection_icon(worker);
        return;
    }
    // 探测放进独立线程：UIA 对 Chromium 应用要重试等 120ms+，堵在这里
    // 会让 sync_channel(1) 溢出，丢掉后续鼠标信号 → 图标时灵时不灵
    let gen = SELECTION_GEN.load(Ordering::Relaxed);
    let prober = worker.clone();
    std::thread::spawn(move || match crate::platform::focused_selection() {
        // 等待期间用户又按下了鼠标（新选区开始）：本次结果已过期，丢弃
        Some(text) if SELECTION_GEN.load(Ordering::Relaxed) == gen => {
            // 光标已离松手点很远：用户去别处了，不追着报旧选区的应用弹图标
            let (cx, cy) = crate::platform::cursor_pos();
            if (cx - ux).abs() + (cy - uy).abs() > near_px {
                hide_selection_icon(&prober);
            } else {
                show_selection_icon(&prober, text, ux, uy);
            }
        }
        _ if SELECTION_GEN.load(Ordering::Relaxed) == gen => {
            hide_selection_icon(&prober)
        }
        _ => {} // 过期结果，什么都不做
    });
}

/// 点划词图标：把上次探测到的选区交给弹窗卡片（与热键同一个 selection 事件）
#[tauri::command]
fn translate_selection_icon(app: AppHandle) -> Result<(), String> {
    ICON_CLICKED_AT.store(now_ms(), Ordering::Relaxed);
    hide_selection_icon(&app);
    let text = app
        .state::<IconSelection>()
        .0
        .lock()
        .take()
        .ok_or("没有缓存的选中文本")?;
    let (cx, cy) = crate::platform::cursor_pos();
    let (x, y) = clamp_to_screen(&app, "main", cx + 12, cy + 16);
    app.emit("selection", Selection { text, x, y })
        .map_err(|e| e.to_string())
}

/// 快捷键触发：抓取选中文本并通知弹窗
fn handle_hotkey_trigger(app: &AppHandle) {
    println!("[shortcut] triggered");
    // 抓取选中文本需要模拟按键并等待剪贴板，放到独立线程，
    // 避免阻塞主线程；且此时用户可能已松开按键。
    let app = app.clone();
    std::thread::spawn(move || {
        match grab_selection(&app) {
            Some(text) => {
                let (cx, cy) = crate::platform::cursor_pos();
                println!("[grab] got text: {:?} at ({}, {})", text, cx, cy);
                // 在 Rust 端钳制到屏幕内后再发事件，前端直接使用
                let (x, y) = clamp_to_screen(&app, "main", cx + 12, cy + 16);
                hide_selection_icon(&app);
                let _ = app.emit("selection", Selection { text, x, y });
            }
            None => {
                println!("[grab] no text captured");
                // macOS 上抓词失败最常见原因是辅助功能权限未授予，
                // 直接打开设置页引导授权（Windows/已授权时恒 true，静默）
                if !crate::platform::accessibility_allowed() {
                    open_settings(app.clone());
                }
            }
        }
    });
}

/// 权限缺失时弹原生对话框，一键跳到对应系统设置面板。
/// 只在后台线程调用：blocking_show 会阻塞当前线程，主线程调用会卡住事件循环
fn alert_permission(app: &AppHandle, message: &str, settings_url: &str) {
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
        use tauri_plugin_opener::OpenerExt;
        let _ = app.opener().open_url(settings_url, None::<&str>);
    }
}

/// 截图会话状态：触发时保存全屏快照与设置窗口可见性，finish 时再裁剪
#[derive(Default)]
struct ScreenshotState(parking_lot::Mutex<ScreenshotSession>);

#[derive(Default)]
struct ScreenshotSession {
    screen: Option<crate::capture::CapturedScreen>,
    settings_was_visible: bool,
    main_was_visible: bool,
}

/// 藏起自家窗口并返回藏之前的真实可见性（供事后恢复）；窗口不存在/读不到状态都按
/// “本来就没开”处理返回 false。与 show_window 是截图会话专用的配对原语
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

/// 截图翻译触发：后台抓屏供裁剪，遮罩立即显示（透明，框选对实时桌面）
fn handle_screenshot_trigger(app: &AppHandle) {
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
        // 没权限时抓屏只会拿到桌面壁纸，必须明确告诉用户去哪开，而不是静默失败。
        // 先 request：只有它会弹系统授权框并把 Wordpeek 登记进「屏幕录制」列表
        //（只 preflight 的话列表里没有这一项，用户根本没法勾）。
        // 用户当场勾了也常常要重启进程才生效，所以仍继续走提示。
        if !crate::platform::screen_capture_allowed() {
            let granted = crate::platform::request_screen_capture();
            if !granted {
                alert_permission(
                    &app,
                    "截图翻译需要「屏幕录制」权限。\n\n系统设置 → 隐私与安全性 → 屏幕录制，勾选 Wordpeek 后重启应用。",
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                );
                return;
            }
        }
        // 抓屏前先把自家窗口藏起来：否则设置窗/上次的结果卡片会被拍进截图，
        // 而且新遮罩弹出时旧卡片还压在屏幕上（用户看到的「卡片不消失」）
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

        // 存快照供 finish/cancel 清理（was_visible 是抓屏前读到的真实值，
        // 窗口此刻已被藏起，不能再重新 is_visible）
        let state = app.state::<ScreenshotState>();
        {
            let mut session = state.0.lock();
            session.screen = Some(screen);
            session.settings_was_visible = was_visible;
            session.main_was_visible = main_was_visible;
        }

        // 遮罩内容常驻就绪（纯 CSS，无大图传输），立即显示即可框选；
        // screenshot-start 仅作轻量信号让前端重置选区
        let _ = app.emit_to(
            "screenshot",
            "screenshot-start",
            serde_json::json!({}),
        );
        let _ = win.show();
        let _ = win.set_focus();
        crate::platform::prepare_overlay(&win);

        // 双屏不同 DPI：set_position 跨屏移动会触发 WM_DPICHANGED，tao 在事件循环里
        // 异步按「保持逻辑尺寸」重设窗口，覆盖刚设的虚拟屏尺寸 → 首次遮罩只盖一块屏
        //（第二次触发 DPI 已稳定不再触发，所以正常）。显示后校验几何，被 churn 改动
        // 就重设，直到连续两次读数稳定。
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

/// 截图结果 payload：ctx 里带 sel/virtual/mouse/work 四组坐标（物理像素），
/// 前端用它们把卡片弹到框选位置附近；四个分支只差 ok/text/info/error，其余手抄三遍太脆
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
fn screenshot_finish(app: AppHandle, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let state = app.state::<ScreenshotState>();
    let mut session = state.0.lock();
    let screen = session.screen.take().ok_or("没有进行中的截图会话")?;
    let was_visible = session.settings_was_visible;
    let win = app.get_webview_window("screenshot").ok_or("截图窗口不存在")?;
    let _ = win.hide();
    // 松手瞬间的光标位置（物理像素），结果卡片弹到鼠标旁边；OCR 完再取会被用户摌走
    let (mx, my) = crate::platform::cursor_pos();
    // 光标所在屏的工作区（排除任务栏），前端锚制用
    let (wx, wy, ww, wh) = crate::platform::work_area_at(mx, my);

    // 钳制到快照范围内（capture::clamp_rect，有单测），避免越界尺寸与裁剪像素长度不符
    let rect = crate::capture::clamp_rect(x, y, w, h, screen.width, screen.height);
    let (cx, cy, cw, ch) = (rect.x, rect.y, rect.w, rect.h);

    let cropped = crate::capture::crop(&screen.rgba, screen.width, screen.height, rect)?;
    // 定位信息先提取，随后释放全屏像素（OCR 期间不再需要 ~23MB RGBA）
    let (vx, vy, vw, vh) = (screen.origin_x, screen.origin_y, screen.width, screen.height);
    drop(screen.rgba);
    // 选区/虚拟屏/鼠标/工作区坐标（物理像素）随结果发前端，用于把卡片弹到框选位置附近
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
fn screenshot_cancel(app: AppHandle) {
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

/// 复制文本到剪贴板（结果卡片复制按钮）
#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| format!("复制失败: {e}"))
}

/// 快捷键两两不能重复（重复会导致行为歧义）；大小写与首尾空白不同视为同一个键
pub(crate) fn validate_hotkeys(hotkeys: &[&str]) -> Result<(), String> {
    let mut seen: Vec<String> = Vec::new();
    for h in hotkeys {
        let key = h.trim().to_lowercase();
        if seen.contains(&key) {
            return Err(format!("快捷键重复：{h}"));
        }
        seen.push(key);
    }
    Ok(())
}

/// 注册全部全局快捷键（先注销旧的全部再注册）；返回注册失败的快捷键名（
/// 通常是键位被其他程序占用）。保存配置时把它作为 Err 返回给设置页展示
pub(crate) fn register_hotkeys(app: &AppHandle, cfg: &translate::AppConfig) -> Vec<String> {
    let mut failed = Vec::new();
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("[shortcut] 注销旧快捷键失败: {e}");
    }
    // 任意两个快捷键相同都会导致行为歧义，直接都不注册
    if let Err(e) = validate_hotkeys(&[&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey]) {
        eprintln!("[shortcut] {e}");
        return vec![e];
    }
    macro_rules! reg {
        ($hotkey:expr, $handler:expr) => {
            if let Ok(sc) = parse_hotkey(&$hotkey) {
                if let Err(e) = app.global_shortcut().on_shortcut(sc, $handler) {
                    eprintln!("[shortcut] {} 注册失败: {e}", $hotkey);
                    failed.push($hotkey.clone());
                }
            }
        };
    }
    reg!(cfg.hotkey, |app: &AppHandle, _s, event| {
        if event.state == ShortcutState::Pressed {
            handle_hotkey_trigger(app);
        }
    });
    reg!(cfg.screenshot_hotkey, |app: &AppHandle, _s, event| {
        if event.state == ShortcutState::Pressed {
            handle_screenshot_trigger(app);
        }
    });
    reg!(cfg.input_hotkey, |app: &AppHandle, _s, event| {
        if event.state == ShortcutState::Pressed {
            open_input_window(app);
        }
    });
    failed
}

/// 系统权限状态（设置页「系统权限」区块展示 + 跳系统设置；
/// Windows 恒可用，macOS 实现在 platform/macos.rs）
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

fn main() {
    // 第一件事：拆 stdout/stderr 死管道炸弹（必须在任何 println! 之前）。
    // dev 构建不拆：cargo tauri dev 的控制台日志全靠 npm 那根管道
    #[cfg(windows)]
    if cfg!(not(debug_assertions)) {
        platform::sanitize_stdio();
    }

    let builder = tauri::Builder::default();
    builder
        // 单实例：第二次启动不再起新进程，直接在已有实例里把设置窗拉到前台。
        // 必须排在其它 plugin 之前（官方要求），否则可能先拿到已注册的系统资源
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            open_settings(app.clone());
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

            app.manage(UpdateState::default());
            app.manage(ScreenshotState::default());
            app.manage(IconSelection::default());

            // Win11 会给透明弹窗再叠一层 ~8px 系统圆角，CSS 圆角外多出一道弧；
            // 弹窗类窗口全关掉，圆角只留 CSS 一层（设置/输入窗有标题栏，保留系统圆角）
            #[cfg(windows)]
            for label in ["main", "icon", "screenshot"] {
                if let Some(win) = app.get_webview_window(label) {
                    platform::disable_rounding(&win);
                }
            }

    // panic 钩子：tokio 任务里的 panic 默认只打到 stderr（release 下不可见），
    // 落一份到配置目录的 panic.log 方便事后排查
    let panic_dir = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::temp_dir());
    std::panic::set_hook(Box::new(move |info| {
        translate::log_panic(&panic_dir, &info.to_string());
    }));

    // TODO(诊断): tokio 运行时心跳——卡死时对比此日志是否仍在增长，破案后删
    tauri::async_runtime::spawn(async {
        loop {
            translate::diag("runtime alive");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

            // 划词图标：全局鼠标监听（跟设置里的开关无关，开关在探测时现读）
            start_selection_icon(&handle);

            // 启动后后台把截图遮罩窗口预铺到虚拟屏尺寸（常驻隐藏），
            // 首次截图时窗口无需 resize，避免闪动
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

            // 启动后在后台预热 OCR（仅 macOS）：进程内第一次调 Vision 要加载模型（实测 30-60s），
            // 不预热的话「第一次截图翻译」就会卡半分钟。Windows 用 Windows.Media.Ocr 无需预热
            #[cfg(target_os = "macos")]
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_secs(2));
                crate::ocr::warmup();
            });

            // 启动后延迟几秒静默检查一次更新，结果缓存供设置页查询            // （macOS 拿不到更新产物，启动时不检查，避免每次启动一条错误日志）
            if !cfg!(target_os = "macos") {
                let updater_handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(6));
                    if let Err(e) = tauri::async_runtime::block_on(check_update(&updater_handle)) {
                        eprintln!("[updater] startup check failed: {e}");
                    }
                });
            }

            // 设置窗口也改为按需创建（open_settings），点 ✕ 即销毁，
            // 不再需要关闭时隐藏的守卫

            // 系统托盘：应用常驻后台的唯一可见入口，
            // 左键/右键弹出菜单，可打开设置或退出
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
                    "input" => open_input_window(app),
                    "settings" => open_settings(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // 按配置注册全部全局快捷键（划词+截图；失败仅打日志，不阻断启动）
            let cfg = app
                .state::<parking_lot::Mutex<translate::AppConfig>>()
                .lock()
                .clone();
            register_hotkeys(&handle, &cfg);
            // 启动时注册失败只打日志：窗口还没起来，无处展示；设置页保存时会二次注册并报错

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_permissions,
            restart_app,
            open_settings,
            get_update_status,
            check_update_now,
            install_update,
            screenshot_finish,
            screenshot_cancel,
            fit_window,
            copy_text,
            translate_selection_icon,
            translate::translate,
            translate::translate_all,
            translate::get_config,
            translate::save_sources,
            translate::save_baidu,
            translate::save_tencent,
            translate::save_settings,
            translate::save_popup_width,
            translate::save_settings_section,
            translate::save_theme,
            translate::save_selection_icon
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_selects_distinguishes_drag_click_and_double_click() {
        // 拖拽（位移 ≥ 6px）→ 选中
        assert!(gesture_selects((100, 100, 1000), (130, 100), (0, 0, 0)));
        // 普通点击（位移小、无连击）→ 不弹图标
        assert!(!gesture_selects((100, 100, 1000), (102, 101), (0, 0, 0)));
        // 双击选词：500ms 内同一点再次按下 → 选中
        assert!(gesture_selects((100, 100, 1200), (101, 100), (101, 100, 1000)));
        // 间隔太久（≥500ms）不算连击
        assert!(!gesture_selects((100, 100, 1600), (101, 100), (101, 100, 1000)));
    }

    #[test]
    fn parse_common_hotkeys() {
        assert!(parse_hotkey("Ctrl+Alt+T").is_ok());
        assert!(parse_hotkey("Ctrl+Shift+D").is_ok());
        assert!(parse_hotkey("Alt+F2").is_ok());
        assert!(parse_hotkey("Ctrl+1").is_ok());
        assert!(parse_hotkey("D").is_err()); // 缺修饰键
        assert!(parse_hotkey("Ctrl+").is_err());
    }

    #[test]
    fn validate_hotkeys_rejects_duplicates() {
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+S", "Ctrl+Alt+I"]).is_ok());
        // 大小写/空白不同视为同一个键
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "ctrl+alt+t", "Ctrl+Alt+I"]).is_err());
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+S", "Ctrl+Alt+S"]).is_err());
        let err = validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+T", "Ctrl+Alt+I"]).unwrap_err();
        assert!(err.contains("重复"), "错误信息应说明原因: {err}");
    }

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
