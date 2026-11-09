//! 热键路径的选区获取：UIA/AX 直读先行，读不到才按用户授权走剪贴板兜底
//! （模拟复制键 + 全程备份/恢复）。这是唯一允许碰剪贴板的路径；划词图标路径绝不走这里。

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;

use super::icon::hide_selection_icon;
use super::{SessionManager, SOURCE_HOTKEY, SOURCE_HOTKEY_FALLBACK};
use crate::platform;

/// 热键兜底抓取进行中的互斥标志：连按热键时上一轮未结束就放弃本次；探测也借它避让
static GRAB_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub(crate) fn grab_in_progress() -> bool {
    GRAB_IN_PROGRESS.load(Ordering::SeqCst)
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
    let _guard = crate::FlagGuard(&GRAB_IN_PROGRESS);
    // 新一轮开始：上一轮的「二次恢复」watcher 立即收手，两轮备份互不打架
    let gen = platform::bump_watch_gen();
    // 注入前安全闸（一次查询拿全前台上下文）：老控制台 Ctrl+C 是「中断」不是复制、
    // 密码框/Secure Input 场景注入会被吞甚至触发安全告警——宁可不翻译，绝不在这些场景注入
    let ctx = platform::foreground_context();
    println!(
        "[grab] foreground: pid={:?} app={:?} elevated={} password={} secure={}",
        ctx.pid, ctx.process_name, ctx.elevated, ctx.password_focus, ctx.secure_input
    );
    if ctx.elevated {
        // UIPI 会拒绝普通完整性进程对管理员目标的模拟按键：多半拿不到文本，属预期降级
        println!("[grab] 目标进程以管理员运行，模拟按键大概率被 UIPI 拒绝");
    }
    if !ctx.fallback_allowed {
        println!("[grab] 前台是老式控制台，不注入复制键，放弃本次");
        return None;
    }
    if ctx.password_focus {
        println!("[grab] 焦点在密码框，放弃注入（宁可不翻译）");
        return None;
    }
    if ctx.secure_input {
        println!("[grab] 系统安全输入开启中（登录/密码窗口），放弃注入");
        return None;
    }
    // 备份原内容（带重试，多格式原始字节：图片/文件/自定义格式都能还原）。
    // Err = 被持续锁住或全是 GDI 句柄格式（内容未知/还原不了），绝不继续；
    // Ok = 完整快照，之后恢复到原样
    const BACKUP_RETRIES: u32 = 8;
    let Ok(backup) = platform::clipboard_backup(BACKUP_RETRIES) else {
        println!("[grab] 剪贴板备份失败（被占用或含无法备份的内容），放弃本次");
        return None;
    };
    let base_seq = platform::clipboard_seq();

    // 注入复制键（Windows Ctrl+C / macOS Cmd+C）
    let injected = platform::inject_copy(app);
    let ax = platform::accessibility_allowed();
    if !injected || !ax {
        println!("[grab] inject_copy={injected}, accessibility={ax}");
        platform::clipboard_restore(&backup);
        return None;
    }

    // 轮询到序号变化（慢应用写入可能超百毫秒，固定 sleep 会误判），最多 ~600ms；
    // 首轮没等到再注入一次复制键重试，再等 ~300ms
    let mut grabbed: Option<String> = None;
    for attempt in 0..2 {
        let rounds = if attempt == 0 { 30 } else { 15 };
        for _ in 0..rounds {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if platform::clipboard_seq() == base_seq {
                continue;
            }
            // 序号动了 = 有应用写了剪贴板；文本还读不到（先清后写中/非文本/瞬时锁）就等下一拍
            if let Ok(Some(t)) = platform::clipboard_read_text(0) {
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
            if !platform::inject_copy(app) {
                break;
            }
        }
    }

    // 把用户原内容放回去（多格式原样还原）
    platform::clipboard_restore(&backup);
    // 以恢复完成的这一刻为基线：之后再有写入才是待纠正的「事后背刺」
    let restored_seq = platform::clipboard_seq();

    match grabbed {
        Some(text) => {
            // 选中词与旧文本逐字相同 → 恢复即原样，无需盯梢（也避免把目标应用的
            // 重复写入误判成背刺）；否则交给 watcher 纠正慢应用的事后写入
            if backup.text().as_deref().map(str::trim) != Some(text.as_str()) {
                platform::spawn_second_chance(backup, text.clone(), restored_seq, gen);
            }
            Some(text)
        }
        None => None,
    }
}

/// 热键失败时的权限引导：macOS 抓词失败最常见是辅助功能权限未授予，开设置页引导授权
/// （Windows/已授权时恒 true，静默）
fn open_settings_if_permission_missing(app: &AppHandle) {
    if !platform::accessibility_allowed() {
        crate::windows::open_settings(app.clone());
    }
}

/// 快捷键触发：UIA/AX 直读选区先行（零剪贴板访问）；读不到就走剪贴板兜底
/// （技术安全闸见 foreground_context——老控制台/密码框/Secure Input 一律不注入）。
/// 这是唯一允许碰剪贴板的路径；划词图标路径绝不走这里
pub(crate) fn handle_hotkey_trigger(app: &AppHandle) {
    println!("[shortcut] triggered");
    #[cfg(windows)]
    crate::platform::windows::debug_log("hotkey triggered"); // [DEBUG-m0u5]
    // 直读/模拟按键都可能耗上百毫秒，放独立线程避免阻塞主线程
    let app = app.clone();
    std::thread::spawn(move || {
        // 先试系统选区接口：命中时零剪贴板污染（不注入按键、不动用户剪贴板）。
        // 用 untracked 读法：热键不查拉黑表也不记账——误拉黑的应用靠热键一次成功自愈
        if let platform::SelectionRead::Text(text) = platform::focused_selection_untracked() {
            println!("[grab] UIA/AX 直读命中");
            let (cx, cy) = platform::cursor_pos();
            hide_selection_icon(&app);
            SessionManager::show_card(&app, &text, cx + 12, cy + 16, SOURCE_HOTKEY);
            return;
        }
        // 读不到选区：模拟复制键借剪贴板取词（备份/恢复全程，备份读不到就整轮放弃）
        match grab_selection(&app) {
            Some(text) => {
                let (cx, cy) = platform::cursor_pos();
                println!("[grab] got text: {:?} at ({}, {})", text, cx, cy);
                #[cfg(windows)]
                crate::platform::windows::debug_log(&format!("grab got {:.20}", text)); // [DEBUG-m0u5]
                hide_selection_icon(&app);
                SessionManager::show_card(&app, &text, cx + 12, cy + 16, SOURCE_HOTKEY_FALLBACK);
            }
            None => {
                println!("[grab] no text captured");
                #[cfg(windows)]
                crate::platform::windows::debug_log("grab no text"); // [DEBUG-m0u5]
                open_settings_if_permission_missing(&app);
            }
        }
    });
}
