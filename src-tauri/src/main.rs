#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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

// 平台相关实现（光标/工作区/复制注入/抓屏/权限）在 platform/windows.rs 与 platform/macos.rs

#[derive(serde::Serialize, Clone)]
struct Selection {
    text: String,
    x: i32,
    y: i32,
}

/// 抓取互斥量：grab_selection 会读改写剪贴板（备份→注入复制键→恢复），并发交错会顶掉用户内容，必须串行
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

/// 模拟复制键抓取当前选中文本（热键路径的兜底；划词图标路径绝不走这里）。
/// 全程对剪贴板只有两次写入：目标应用响应复制键的一次 + 我们恢复备份的一次。
/// 不再写哨兵——注入前先取剪贴板「变更序号」基线，序号一动即有人写入，
/// 选中内容与旧内容相同也不会漏判（旧哨兵法是比内容，相同即误判），
/// 轮询窗口内用户的粘贴也再不会看到零宽垃圾。
/// 旧文本备份带锁竞争重试（旧实现的洞：目标应用瞬时持锁时备份无声失败 → 无从恢复）；
/// 备份读不到就整轮放弃——宁可这次不翻译，也不动用户剪贴板里我们看不见的内容。
/// 慢应用（Word/大网页）在轮询窗口后才写完，由 platform::spawn_second_chance 盯住打回
fn grab_selection(app: &AppHandle) -> Option<String> {
    // 热键连按：上一轮未结束就放弃本次，上一轮结果照常弹出
    if GRAB_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        println!("[grab] 上一轮抓取仍在进行，跳过本次触发");
        return None;
    }
    let _guard = FlagGuard(&GRAB_IN_PROGRESS);
    // 新一轮开始：上一轮的「二次恢复」watcher 立即收手，两轮备份互不打架
    let gen = crate::platform::bump_watch_gen();
    // 诊断打印：foreground_app 仅 Windows 实现（macOS 走 NSWorkspace，暂缺）
    #[cfg(windows)]
    println!("[grab] foreground app: {}", crate::platform::foreground_app());
    // 老控制台 Ctrl+C 是「中断」不是复制，注入会打断用户前台命令（macOS 恒 true）
    if !crate::platform::clipboard_fallback_allowed() {
        println!("[grab] 前台是老式控制台，不注入复制键，放弃本次");
        return None;
    }
    // 备份原文本（带重试）。读不到内容 = 不知道剪贴板里是什么，绝不继续；
    // Ok(None) = 确定没有文本（图片/文件/空），照常恢复（恢复到空态）
    const BACKUP_RETRIES: u32 = 8;
    let Ok(backup) = crate::platform::clipboard_read_text(BACKUP_RETRIES) else {
        println!("[grab] 剪贴板被其他程序占用，放弃本次");
        return None;
    };
    let base_seq = crate::platform::clipboard_seq();

    // 注入复制键（Windows Ctrl+C / macOS Cmd+C）
    let injected = crate::platform::inject_copy(app);
    let ax = crate::platform::accessibility_allowed();
    if !injected || !ax {
        println!("[grab] inject_copy={injected}, accessibility={ax}");
        crate::platform::clipboard_restore(backup);
        return None;
    }

    // 轮询到序号变化（慢应用写入可能超百毫秒，固定 sleep 会误判），最多 ~600ms；
    // 首轮没等到再注入一次复制键重试，再等 ~300ms
    let mut grabbed: Option<String> = None;
    for attempt in 0..2 {
        let rounds = if attempt == 0 { 30 } else { 15 };
        for _ in 0..rounds {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if crate::platform::clipboard_seq() == base_seq {
                continue;
            }
            // 序号动了 = 有应用写了剪贴板；文本还读不到（先清后写中/非文本/瞬时锁）就等下一拍
            if let Ok(Some(t)) = crate::platform::clipboard_read_text(0) {
                let trimmed = t.trim().to_string();
                if !trimmed.is_empty() {
                    grabbed = Some(trimmed);
                    break;
                }
            }
        }
        if grabbed.is_some() {
            break;
        }
        if attempt == 0 {
            println!("[grab] 首轮复制未生效，重试注入一次");
            if !crate::platform::inject_copy(app) {
                break;
            }
        }
    }

    // 把用户原内容放回去（有文本还原文本；原本无文本则清空）
    crate::platform::clipboard_restore(backup.clone());
    // 以恢复完成的这一刻为基线：之后再有写入才是待纠正的「事后背刺」
    let restored_seq = crate::platform::clipboard_seq();

    match grabbed {
        Some(text) => {
            // 选中词与旧文本逐字相同 → 恢复即原样，无需盯梢（也避免把目标应用的
            // 重复写入误判成背刺）；否则交给 watcher 纠正慢应用的事后写入
            if backup.as_deref().map(str::trim) != Some(text.as_str()) {
                crate::platform::spawn_second_chance(backup, text.clone(), restored_seq, gen);
            }
            Some(text)
        }
        None => {
            println!("[grab] 序号未变/未读到文本：目标应用没响应复制键");
            None
        }
    }
}

/// 内容高度 → 客户区逻辑高度：夹在 60px 与「工作区−标题栏−40px」间，超出才内部滚动；拿不到工作区退回 800
fn fit_height(content_h: f64, work_h: f64, chrome_h: f64) -> f64 {
    let max = if work_h > 0.0 {
        work_h - chrome_h - 40.0
    } else {
        800.0
    };
    content_h.clamp(60.0, max.max(60.0))
}

/// 逻辑像素 → 物理像素（四舍五入、至少 1px）；钳位前必须乘 scale，否则缩放 ≠100% 时钳不住
fn to_physical(logical: f64, scale: f64) -> i32 {
    (logical * scale).round().max(1.0) as i32
}

/// 把左上角在 (x, y)、尺寸 win_w×win_h 的窗口钳进显示器矩形；窗口比屏大时贴左上角
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

/// 钳制矩形优先用工作区（排除菜单栏/Dock/任务栏），全 0 才退回整屏——整屏钳会让贴底卡片钻到 Dock 下
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

/// 把弹窗位置钳进光标所在屏幕；任一步骤拿不到信息（找不到窗口/显示器）就原样返回不钳制
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

/// 前端量好内容高度后调用：只调高不调宽，按算出的新外框重新钳位（不依赖 set_size 后回读，那是异步消息）；
/// 返回实际生效高度（可能被钳小），前端据此判断 is-capped，未顶上限不出滚动条，避免高度动画中闪条
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
    // 高度上限用窗口所在屏的工作区；work_area_at 给物理像素，内容高度是逻辑像素，先除以 scale 再钳制
    let (_, _, _, work_h) = crate::platform::work_area_at(pos.x + 8, pos.y + 8);
    let inner_w = inner.width as f64 / scale;
    // 带标题栏的窗口里窗口高 ≠ webview 视口高（前端拿到的 100vh 少一条标题栏）需补差值，无边框弹窗为 0；
    // 差值只测一次并缓存：前端上报的视口高可能落后于刚 set_size 的窗口，现算会把差值越滚越大
    let chrome_extra = viewport.map(|vh| viewport_inset(&label, inner.height as f64 / scale, vh)).unwrap_or(0.0);
    let inner_h = fit_height(
        height + chrome_extra,
        if work_h > 0 { work_h as f64 / scale } else { 0.0 },
        chrome_h,
    );
    win.set_size(tauri::LogicalSize::new(inner_w, inner_h))
        .map_err(|e| e.to_string())?;

    // 新外框尺寸 = 客户区 + 边框/标题栏，换算成物理像素再钳位；否则 150% 缩放下卡片伸出屏幕底且无滚动条（issue #8）
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

/// 摆到主屏工作区中央（Dock/菜单栏/任务栏不遮挡）；不用 win.center()：按整屏居中且隐藏时可能 no-op
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

/// 显示输入翻译窗口（托盘菜单/快捷键共用）。点 ✕ 即销毁，回收常驻 WebView2 renderer（~60MB）
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
    // macOS 还没有签名/公证的产物，latest.json 里也没有 darwin 平台，检查只会报错
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

/// 手动检查更新（设置页「检查更新」按钮）。async 命令由 tauri 丢到异步运行时，网络等待不阻塞主线程
#[tauri::command]
async fn check_update_now(app: AppHandle) -> Result<UpdateStatus, String> {
    check_update(&app).await
}

/// 重启自己：改完系统权限必须换进程才生效；macOS「退出并重新打开」对终端起的进程无效，故用应用内按钮
#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

/// 更新下载进度（发给设置页画进度条；total 未知时前端显示不定态进度）
#[derive(serde::Serialize, Clone)]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// 后台线程下载并静默安装已缓存的更新，完成后重启。进度经 `update-progress` 事件按整数
/// 百分比/256KB 步进去重推送；失败发 `update-failed` 并把更新放回缓存——命令立即返回 Ok，线程内失败只能靠事件通知前端
#[tauri::command]
fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .state::<UpdateState>()
        .lock()
        .take()
        .ok_or_else(|| "没有已缓存的更新".to_string())?;
    std::thread::spawn(move || {
        println!("[updater] downloading & installing...");
        let emitter = app.clone();
        let mut downloaded: u64 = 0;
        let mut total: Option<u64> = None;
        let mut last_anchor: u64 = u64::MAX;
        let install = update.download_and_install(
            |chunk, t| {
                downloaded += chunk as u64;
                if total.is_none() {
                    total = t;
                }
                // 锚点：有总量按百分比（0-100），无总量按 256KB 步进
                let anchor = match total {
                    Some(t) if t > 0 => downloaded * 100 / t,
                    _ => downloaded / (256 * 1024),
                };
                if anchor != last_anchor {
                    last_anchor = anchor;
                    let _ = emitter.emit("update-progress", UpdateProgress { downloaded, total });
                }
            },
            || {},
        );
        if let Err(e) = tauri::async_runtime::block_on(install) {
            eprintln!("[updater] install failed: {e}");
            // 失败把已下载的更新放回缓存，用户重试不用再走一遍检查更新
            *app.state::<UpdateState>().lock() = Some(update);
            let _ = app.emit("update-failed", e.to_string());
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

/// 划词图标最近一次探测到的选中文本，点图标时直接用；点击后焦点已转走，再读源应用选区就读不到了
#[derive(Default)]
struct IconSelection(parking_lot::Mutex<Option<String>>);

/// 刚点过图标的时刻（毫秒）。点击后的松开事件仍会触发探测且源应用选区还在，图标会立刻弹回，故 400ms 内的探测跳过
static ICON_CLICKED_AT: AtomicU64 = AtomicU64::new(0);

/// 左键按下的位置与时刻（物理像素, 毫秒）：用来区分「选词手势」和普通单击
static MOUSE_DOWN: std::sync::OnceLock<parking_lot::Mutex<(i32, i32, u64)>> =
    std::sync::OnceLock::new();
/// 上一次松手的位置与时刻（同上），双击/三击判定用
static MOUSE_UP: std::sync::OnceLock<parking_lot::Mutex<(i32, i32, u64)>> =
    std::sync::OnceLock::new();
/// 本次松手是否值得探测选区；mouse-up 回调里算好，worker 直接读
static PROBE_INTENT: AtomicBool = AtomicBool::new(false);
/// 选区代次：每次按下 +1。探测异步做（不能堵 worker），show 前对代次，期间有新按下（用户已开始新选区）就放弃
static SELECTION_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 本次按下时光标是否落在划词图标矩形内（钩子回调只做廉价判断，hide 由 worker 做）
static DOWN_ON_ICON: AtomicBool = AtomicBool::new(false);
/// 划词图标最近一次 show 的屏幕矩形（物理像素 x,y,w,h）：钩子回调用它判断是否点在图标上，避免回调里跨线程查窗口
static ICON_RECT: parking_lot::Mutex<Option<(i32, i32, u32, u32)>> = parking_lot::Mutex::new(None);

fn record_mouse_down() {
    let (x, y) = crate::platform::cursor_pos();
    *MOUSE_DOWN.get_or_init(Default::default).lock() = (x, y, now_ms());
}

/// 只有真的做了选词手势才探测：拖动（位移 ≥ 6 物理像素，曼哈顿距离）或连击（双击选词/三击选行）；
/// 普通单击不算——很多应用单击后 AXSelectedText 仍返回旧选区，不拦会「随便点一下也冒图标」
const GESTURE_MOVE_PX: i32 = 6;
const GESTURE_REPEAT_PX: i32 = 8;

/// 纯判定逻辑，供测试：本次按下/松开与上一次松开（px,py,prev_at）的坐标时间
fn gesture_selects(
    down: (i32, i32, u64),
    up: (i32, i32),
    prev_up: (i32, i32, u64),
    repeat_ms: u64,
) -> bool {
    let (dx, dy, down_at) = down;
    let (x, y) = up;
    let (px, py, prev_at) = prev_up;
    let moved = (x - dx).abs() + (y - dy).abs() >= GESTURE_MOVE_PX;
    // 本次按下紧跟上一次松手（系统双击间隔内、同一点）= 连击
    let repeated = prev_at != 0
        && down_at.saturating_sub(prev_at) < repeat_ms
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
    gesture_selects(down, (x, y), (px, py, prev_at), repeat_ms())
}

/// 连击判定窗口取系统双击间隔（系统调到 900ms 时慢速双击也是双击，硬编码 500ms 会误判），极端值夹到 250–1000
fn repeat_ms() -> u64 {
    crate::platform::double_click_interval_ms().clamp(250, 1000)
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

fn hide_selection_icon(app: &AppHandle) {
    *ICON_RECT.lock() = None;
    if let Some(win) = app.get_webview_window("icon") {
        let _ = win.hide();
    }
}

/// 在松手位置显示图标（窗口已在 tauri.conf.json 预建，只挪位置 + show）。锚定松手点而非实时光标：
/// 探测要等 120ms+，追光标的话用户挪开鼠标，图标会在别处冒出来
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

/// 光标是否落在缓存的图标矩形内；只在钩子回调用，只碰 GetCursorPos + 内存读，绝不做跨线程窗口查询
fn cursor_on_icon_rect() -> bool {
    let Some((x, y, w, h)) = *ICON_RECT.lock() else {
        return false;
    };
    let (cx, cy) = crate::platform::cursor_pos();
    cx >= x && cx < x + w as i32 && cy >= y && cy < y + h as i32
}

/// 划词图标：鼠标松开后读一次系统选区（不碰剪贴板、不注入按键），有选中文字就在光标旁弹按钮，
/// 点一下走热键同一套翻译链路；读不到选区（终端、Electron 等不实现 AX/UIA 文本接口）就什么都不做
fn start_selection_icon(app: &AppHandle) {
    /// 鼠标信号：Down=按下、Up=松开（带松手位置，锚点用）、Leave=图标可见时鼠标飘远
    enum Signal {
        Down,
        Up { x: i32, y: i32 },
        Leave,
    }
    // 队列只留一个信号：钩子回调绝不做窗口操作（跨线程查询/ShowWindow 拖慢全系统鼠标且会被摘钩），hide 都在 worker 里做
    let (tx, rx) = std::sync::mpsc::sync_channel::<Signal>(1);
    let tx_down = tx.clone();
    let tx_move = tx.clone();
    /// 光标离开图标矩形这么远（物理像素）就收起图标；留这一圈余量是给「把鼠标挪过去点图标」的路径
    const ICON_LEAVE_PX: i32 = 48;
    /// 探测完成时光标离松手点超过这么远就丢弃：很多应用一直报旧选区，人不在不该弹；400px 给 Cmd+C 兜底（≥300ms）留余量
    const ICON_NEAR_PX: i32 = 400;
    let watch = crate::platform::start_mouse_watch(
        Box::new(move || {
            // 点在图标自身上不能收起：Windows 低级钩子连自家窗口的鼠标事件也收，down 就藏会吞掉这次 click
            DOWN_ON_ICON.store(cursor_on_icon_rect(), Ordering::Relaxed);
            record_mouse_down();
            let _ = tx_down.try_send(Signal::Down);
        }),
        Box::new(move || {
            let (x, y) = crate::platform::cursor_pos();
            PROBE_INTENT.store(mouse_gesture_selects(x, y), Ordering::Relaxed);
            let _ = tx.try_send(Signal::Up { x, y });
        }),
        // 移动：图标可见且光标已飘远 → 发 Leave 收起；只碰内存读 + 坐标比较，保持回调轻量
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
                // 按下/飘远收起图标（开始新一轮选区）；点在图标上则保留，等 click 命令（translate_selection_icon）接管
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
    // 在自家窗口里划词（卡片内选中跳查、设置页复制等）不弹图标，否则在自己卡片里选词又冒出图标
    if ["main", "icon", "settings", "input"]
        .iter()
        .any(|&l| cursor_on_window(worker, l))
    {
        hide_selection_icon(worker);
        return;
    }
    // 探测放独立线程：UIA 对 Chromium 应用要重试等 120ms+，堵住会撑爆 sync_channel(1) 丢鼠标信号
    let gen = SELECTION_GEN.load(Ordering::Relaxed);
    let prober = worker.clone();
    // 探测线程要自己开 autorelease pool：worker 线程没有主线程那个池，Obj-C 临时对象会攒到线程结束
    std::thread::spawn(move || {
        crate::platform::with_release_pool(|| probe_and_show(&prober, ux, uy, near_px, gen))
    });
}

/// 探测选区并弹图标。`gen` 是发起时的选区代次：期间用户开始了新选区就丢弃本次结果
fn probe_and_show(worker: &AppHandle, ux: i32, uy: i32, near_px: i32, gen: u64) {
    // 上一轮的 Cmd+C 兜底还没收尾（连点、连按快捷键）：放弃本次，但**不 hide**——上一轮可能
    // 刚把图标弹出来，hide 会让它闪一下就消失；只读标志不上锁，真撞同一毫秒少弹一次，不值
    if GRAB_IN_PROGRESS.load(Ordering::SeqCst) {
        return;
    }
    // 图标路径铁律：绝不碰剪贴板（也不注入复制按键）——UIA/AX 读不到选区（终端、
    // 部分 Electron/自绘 UI）就不弹图标，用户要翻这些应用里的词请走热键
    // （只有热键路径的 grab_selection 允许剪贴板兜底，且全程备份/恢复）
    let text = crate::platform::focused_selection();
    match text {
        // 等待期间用户又按下了鼠标（新选区开始）：本次结果已过期，丢弃
        Some(text) if SELECTION_GEN.load(Ordering::Relaxed) == gen => {
            // 光标已离松手点很远：用户去别处了，不追着报旧选区的应用弹图标
            let (cx, cy) = crate::platform::cursor_pos();
            let far = (cx - ux).abs() + (cy - uy).abs() > near_px;
            if far {
                hide_selection_icon(worker);
            } else {
                show_selection_icon(worker, text, ux, uy);
            }
        }
        _ if SELECTION_GEN.load(Ordering::Relaxed) == gen => {
            hide_selection_icon(worker)
        }
        _ => {} // 过期结果，什么都不做
    }
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
    // 模拟按键+等剪贴板耗时，放独立线程避免阻塞主线程
    let app = app.clone();
    std::thread::spawn(move || {
        match grab_selection(&app) {
            Some(text) => {
                let (cx, cy) = crate::platform::cursor_pos();
                println!("[grab] got text: {:?} at ({}, {})", text, cx, cy);
                let (x, y) = clamp_to_screen(&app, "main", cx + 12, cy + 16);
                hide_selection_icon(&app);
                let _ = app.emit("selection", Selection { text, x, y });
            }
            None => {
                println!("[grab] no text captured");
                // macOS 抓词失败最常见是辅助功能权限未授予，开设置页引导授权（Windows/已授权时恒 true，静默）
                if !crate::platform::accessibility_allowed() {
                    open_settings(app.clone());
                }
            }
        }
    });
}

/// 权限缺失时弹原生对话框，一键跳到对应系统设置面板；只能在后台线程调用（blocking_show 会阻塞当前线程）
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
        // 没权限时抓屏只会拿到桌面壁纸，必须明确引导而不是静默失败。只有 request 会弹授权框并把
        // Wordpeek 登记进「屏幕录制」列表（只 preflight 没法勾）；当场勾了也常要重启进程才生效，仍继续走提示
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
            serde_json::json!({}),
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
fn screenshot_finish(app: AppHandle, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
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

/// 注册全部全局快捷键（先注销旧的全部再注册）；返回注册失败的键名（通常是键位被其他程序占用），
/// 保存配置时作为 Err 返回给设置页展示
pub(crate) fn register_hotkeys(app: &AppHandle, cfg: &translate::AppConfig) -> Vec<String> {
    let mut failed = Vec::new();
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("[shortcut] 注销旧快捷键失败: {e}");
    }
    // 有重复直接全不注册
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

    // TODO(诊断): tokio 运行时心跳——卡死时对比此日志是否仍在增长，破案后删
    tauri::async_runtime::spawn(async {
        loop {
            translate::diag("runtime alive");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

            // 划词图标：全局鼠标监听（跟设置里的开关无关，开关在探测时现读）
            start_selection_icon(&handle);

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

            // 启动后延迟几秒静默检查一次更新，结果缓存供设置页查询（macOS 拿不到更新产物，启动时不检查）
            if !cfg!(target_os = "macos") {
                let updater_handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(6));
                    if let Err(e) = tauri::async_runtime::block_on(check_update(&updater_handle)) {
                        eprintln!("[updater] startup check failed: {e}");
                    }
                });
            }

            // 设置窗口按需创建（open_settings），点 ✕ 即销毁，无需关闭时隐藏的守卫

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
                    "input" => open_input_window(app),
                    "settings" => open_settings(app.clone()),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

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
                    use windows::Win32::Graphics::Dwm::{
                        DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE,
                    };
                    use windows::Win32::UI::WindowsAndMessaging::{
                        SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
                        SWP_NOZORDER,
                    };
                    let dark = windows::Win32::Foundation::BOOL::from(matches!(
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
                        let hwnd = windows::Win32::Foundation::HWND(hwnd.0 as *mut _);
                        unsafe {
                            let _ = DwmSetWindowAttribute(
                                hwnd,
                                DWMWA_USE_IMMERSIVE_DARK_MODE,
                                &dark as *const _ as *const core::ffi::c_void,
                                std::mem::size_of::<windows::Win32::Foundation::BOOL>() as u32,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_selects_distinguishes_drag_click_and_double_click() {
        // 拖拽（位移 ≥ 6px）→ 选中
        assert!(gesture_selects((100, 100, 1000), (130, 100), (0, 0, 0), 500));
        // 普通点击（位移小、无连击）→ 不弹图标
        assert!(!gesture_selects((100, 100, 1000), (102, 101), (0, 0, 0), 500));
        // 双击选词：双击间隔内同一点再次按下 → 选中
        assert!(gesture_selects((100, 100, 1200), (101, 100), (101, 100, 1000), 500));
        // 间隔超双击窗口不算连击
        assert!(!gesture_selects((100, 100, 1600), (101, 100), (101, 100, 1000), 500));
        // 系统把双击间隔调到 900ms 时，慢速双击仍算连击
        assert!(gesture_selects((100, 100, 1800), (101, 100), (101, 100, 1000), 900));
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
