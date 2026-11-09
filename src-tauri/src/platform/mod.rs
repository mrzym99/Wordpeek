//! 平台层：Windows / macOS 各自实现同一组接口，cfg 门控保证互不影响；
//! 共享纯函数留在 main.rs / translate.rs / ocr.rs / capture.rs，不掺平台细节

// ---------- 划词图标：全局鼠标监听回调槽（两个平台共用，钩子/监视器从这里取） ----------

/// 鼠标事件回调（进程内只装一次，钩子函数从静态里取）
pub(crate) type MouseCallback = Box<dyn Fn() + Send + Sync>;
/// 鼠标移动回调：参数是钩子/事件自带的光标物理像素坐标（不额外调 GetCursorPos）。
/// macOS 未用 move：监视器加 mouseMoved 会每帧唤醒主线程，不值得
pub(crate) type MoveCallback = Box<dyn Fn(i32, i32) + Send + Sync>;
pub(crate) static MOUSE_ON_DOWN: std::sync::OnceLock<MouseCallback> = std::sync::OnceLock::new();
pub(crate) static MOUSE_ON_UP: std::sync::OnceLock<MouseCallback> = std::sync::OnceLock::new();
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) static MOUSE_ON_MOVE: std::sync::OnceLock<MoveCallback> = std::sync::OnceLock::new();

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub mod win_clipboard;
#[cfg(target_os = "macos")]
pub mod macos;

/// 复制注入（Windows: Ctrl+C；macOS: Cmd+C，需辅助功能权限）
#[cfg(windows)]
pub use windows::inject_copy;
#[cfg(target_os = "macos")]
pub use macos::inject_copy;

#[cfg(windows)]
pub use windows::cursor_pos;
#[cfg(windows)]
#[cfg(windows)]
pub use windows::sanitize_stdio;
#[cfg(target_os = "macos")]
pub use macos::cursor_pos;
/// 光标所在显示器的工作区（物理像素；拿不到时 (0,0,0,0)）
#[cfg(windows)]
pub use windows::work_area_at;
#[cfg(target_os = "macos")]
pub use macos::work_area_at;

/// 虚拟屏截取（所有显示器合成一张）
#[cfg(windows)]
pub use windows::capture_virtual_screen;
#[cfg(target_os = "macos")]
pub use macos::capture_virtual_screen;

#[cfg(windows)]
pub use windows::virtual_screen_bounds;
#[cfg(target_os = "macos")]
pub use macos::virtual_screen_bounds;

/// 辅助功能权限（Windows 无此概念，恒可用）
#[cfg(windows)]
pub fn accessibility_allowed() -> bool {
    true
}
#[cfg(target_os = "macos")]
pub fn accessibility_allowed() -> bool {
    macos::accessibility_allowed()
}

/// 屏幕录制权限（Windows 无此概念，恒可用）
#[cfg(windows)]
pub fn screen_capture_allowed() -> bool {
    true
}
#[cfg(target_os = "macos")]
pub fn screen_capture_allowed() -> bool {
    macos::screen_capture_allowed()
}

/// 申请屏幕录制权限（弹系统授权框；Windows 无此概念，恒成功）
#[cfg(windows)]
pub fn request_screen_capture() -> bool {
    true
}
#[cfg(target_os = "macos")]
pub fn request_screen_capture() -> bool {
    macos::request_screen_capture()
}

/// 屏幕录制系统授权框本机是否已弹过（Windows 无此概念，恒 true）
#[cfg(windows)]
pub fn screen_capture_prompted(_app: &tauri::AppHandle) -> bool {
    true
}
#[cfg(target_os = "macos")]
pub fn screen_capture_prompted(app: &tauri::AppHandle) -> bool {
    prompt_marker(app).is_some_and(|p| p.exists())
}

/// 记下「系统授权框已弹过」。标记必须跨进程：TCC 拒绝后系统不会再弹框，
/// 只靠进程内标志的话重启后又会被当成首次，两个提示仍会撞在一起
#[cfg(windows)]
pub fn mark_screen_capture_prompted(_app: &tauri::AppHandle) {}
#[cfg(target_os = "macos")]
pub fn mark_screen_capture_prompted(app: &tauri::AppHandle) {
    if let Some(path) = prompt_marker(app) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // 写失败只打日志：最坏结果是下次再多弹一次系统框，不影响功能
        if let Err(e) = std::fs::write(&path, b"1") {
            eprintln!("[perm] 写入系统授权框标记失败: {e}");
        }
    }
}

#[cfg(target_os = "macos")]
fn prompt_marker(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join(".screen-capture-prompted"))
}

/// 需要跳系统设置授权的权限项
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PermissionKind {
    Accessibility,
    ScreenCapture,
}

impl PermissionKind {
    /// 前端传参名 → 权限项（未知返回 None）
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "accessibility" => Some(Self::Accessibility),
            "screen_capture" => Some(Self::ScreenCapture),
            _ => None,
        }
    }

    /// 系统设置隐私扩展里的锚点名（Privacy_*）；仅 macOS 调用
    #[cfg(target_os = "macos")]
    pub(crate) fn anchor(self) -> &'static str {
        match self {
            Self::Accessibility => "Privacy_Accessibility",
            Self::ScreenCapture => "Privacy_ScreenCapture",
        }
    }
}

/// 打开对应权限的系统设置面板（Windows 无此概念，返回错误）
#[cfg(windows)]
pub fn open_permission_settings(_app: &tauri::AppHandle, _kind: PermissionKind) -> Result<(), String> {
    Err("仅 macOS 支持".into())
}
#[cfg(target_os = "macos")]
pub fn open_permission_settings(app: &tauri::AppHandle, kind: PermissionKind) -> Result<(), String> {
    macos::open_permission_settings(app, kind)
}

/// 应用启动时初始化平台状态（macOS：Dock 策略 + 主线程缓存屏幕信息）
#[cfg(windows)]
pub fn init(_app: &tauri::AppHandle) {}

#[cfg(target_os = "macos")]
pub fn init(app: &tauri::AppHandle) {
    macos::init(app);
}

/// 截图遮罩窗口的额外平台处理（Windows 无需）
#[cfg(windows)]
pub fn prepare_overlay(_win: &tauri::WebviewWindow) {}

#[cfg(target_os = "macos")]
pub fn prepare_overlay(win: &tauri::WebviewWindow) {
    macos::prepare_overlay(win);
}

/// 全局鼠标监听：左键按下 → on_down，左键松开 → on_up，鼠标移动 → on_move（带物理像素坐标；
/// macOS 端未用 move）。回调可能很频繁，必须立刻返回（耗时工作自己丢线程）。
#[cfg(windows)]
pub use windows::start_mouse_watch;
#[cfg(target_os = "macos")]
pub use macos::start_mouse_watch;

#[cfg(windows)]
pub use windows::disable_rounding;

/// 焦点控件里被选中区域的单次读取结果（两平台统一语义，是探测节点的前提）。
/// 必须区分「这次没选中」和「应用不支持」：后者才值得拉黑 hopeless 记忆，
/// 前者是日常操作（拖滚动条、空白处拖动），据它拉黑会错杀整个应用
#[derive(Debug)]
pub enum SelectionRead {
    /// 读到了选中文本（已 trim）
    Text(String),
    /// 焦点控件不提供文本/选区接口（终端、自绘 UI、无障碍树未建）
    NoTextPattern,
    /// 支持但当前没有选中内容（空选区）
    NoSelection,
    /// COM/IPC 瞬时错误，下次再试
    Error,
}

#[cfg(windows)]
pub use windows::{foreground_pid, read_focused_selection, warm_uia};
#[cfg(windows)]
pub(crate) use windows::is_chromium_browser;
#[cfg(target_os = "macos")]
pub use macos::{foreground_pid, read_focused_selection};
// macOS 侧暂不豁免（本修复只针对 Windows Chrome 拆树问题），返回 false 保持原行为
#[cfg(target_os = "macos")]
pub(crate) fn is_chromium_browser(_pid: i32) -> bool {
    false
}

/// 读焦点控件里被选中的文本（系统 AX/UIA 接口，不碰剪贴板也不注入按键）。
/// hopeless 记忆（按 pid 拉黑 + TTL）在门面统一做：两平台的拉黑条件同为「本轮
/// 恰好是 NoTextPattern」——空选区和瞬时错误都绝不进记忆
pub fn focused_selection() -> SelectionRead {
    let Some(pid) = foreground_pid() else {
        return SelectionRead::Error;
    };
    // 已确认「压根不提供文本/选区」的 app（终端、自绘 UI）不再试：
    // Windows UIA 重试要白等 360ms，macOS 光唤醒轮询就要 640ms
    if selection_hopeless(pid) {
        println!("[probe] pid={pid} 处于 hopeless 拉黑中，跳过 UIA 直读");
        return SelectionRead::NoTextPattern;
    }
    let read = read_focused_selection();
    println!("[probe] pid={pid} UIA 直读结果：{read:?}");
    match &read {
        SelectionRead::Text(_) => clear_selection_hopeless(pid),
        SelectionRead::NoTextPattern => {
            // Chromium 系（Chrome/Edge）的无障碍树空闲后会被浏览器拆掉，恢复使用时
            // 前几轮探测必然 NoTextPattern，照常三振会误拉黑并停止敲门，树永远重建
            // 不起来（划词图标死循环，TTL 10 分钟一次）。这类进程不记账：代价只是
            // 每轮探测多白等 ~360ms（见 read_focused_selection 注释），无感
            if !is_chromium_browser(pid) {
                mark_selection_hopeless(pid);
            }
        }
        _ => {}
    }
    read
}

/// 热键路径专用读法：不查拉黑表、也不记账。黑名单是图标路径的延迟优化，热键自己
/// 有剪贴板兜底——焦点恰好落在 Chrome 工具栏/地址栏这类非文本控件时 UIA 返回
/// NoTextPattern，热键照记三振会把图标路径拖下水（Chrome 被误拉黑 10 分钟，划词
/// 图标死循环）。代价：真不支持的 app（终端）热键多白等一次 UIA，几百毫秒无感。
/// 读到 Text 顺手清掉误拉黑：热键是用户手动触发的高置信信号，一次成功即可自愈
pub fn focused_selection_untracked() -> SelectionRead {
    let read = read_focused_selection();
    if let (SelectionRead::Text(_), Some(pid)) = (&read, foreground_pid()) {
        clear_selection_hopeless(pid);
    }
    read
}

/// 前台应用上下文：热键兜底注入前的安全闸。一次查询拿全，替代 grab 路径
/// 各自零散的前台查询（进程名/窗口类/权限）；图标路径只消费 secure_input
#[derive(Debug)]
pub struct ForegroundContext {
    pub pid: Option<i32>,
    /// 前台进程名（诊断日志用，取不到为空串）
    pub process_name: String,
    /// 剪贴板兜底注入是否允许（Win 排除老控制台：Ctrl+C 是「中断」不是复制；mac 恒允许）
    pub fallback_allowed: bool,
    /// 目标进程以管理员运行（Win UIPI 会拒模拟按键和 UIA 读；仅诊断，不自动弹 UAC）
    pub elevated: bool,
    /// 焦点控件是密码框——宁可不翻译，绝不在密码场景注入
    pub password_focus: bool,
    /// 系统安全输入开启中（mac 登录窗/密码框；注入会被系统吞且可能触发安全告警）
    pub secure_input: bool,
}

#[cfg(windows)]
pub use windows::foreground_context;
#[cfg(target_os = "macos")]
pub use macos::foreground_context;

// ---------- 剪贴板原语（只有热键路径 grab_selection 用；划词图标路径绝不碰） ----------

/// 剪贴板变更序号：任何进程写入都会变（Windows 是 GetClipboardSequenceNumber，
/// macOS 是 NSPasteboard changeCount）。只读查询不加锁，轮询零成本
#[cfg(windows)]
pub use win_clipboard::clipboard_seq;
#[cfg(target_os = "macos")]
pub use macos::clipboard_seq;

/// 备份剪贴板当前内容（grab 用；Err = 内容未知或还原不了，绝不能动它）
#[cfg(windows)]
pub use win_clipboard::clipboard_backup;
#[cfg(target_os = "macos")]
pub use macos::clipboard_backup;

/// 读剪贴板当前文本。Err = 重试后仍打不开，或剪贴板全是非文本内容（图片/文件，
/// 内容未知，调用方绝不能盲目恢复/清空）；Ok(None) = 确定的空剪贴板
#[cfg(windows)]
pub use win_clipboard::clipboard_read_text;
#[cfg(target_os = "macos")]
pub use macos::clipboard_read_text;

/// 恢复用户原内容：Empty 清空；Text 写回文本（空串≈清空）；Raw 逐格式写回字节
#[cfg(windows)]
pub use win_clipboard::clipboard_restore;
#[cfg(target_os = "macos")]
pub use macos::clipboard_restore;

/// 剪贴板是不是我们进程写的（watcher 排除自己恢复动作的回声；macOS 没有廉价归属查询，恒 false）
#[cfg(windows)]
pub use win_clipboard::clipboard_owner_is_self;
#[cfg(target_os = "macos")]
pub use macos::clipboard_owner_is_self;

// ---------- 剪贴板快照（grab 备份/恢复的通用形态，两个平台共用） ----------

/// grab 期间的剪贴板快照。Empty=确认空剪贴板；Text=纯文本；Raw=多格式原始字节
/// （Windows：文本外的图片 DIB/PNG、文件 HDROP、自定义注册格式等，见 win_clipboard.rs）
pub enum ClipboardBackup {
    Empty,
    /// macOS 侧专用（Windows 侧快照一律 Raw；text() 兼容读）
    #[cfg_attr(windows, allow(dead_code))]
    Text(String),
    #[cfg(windows)]
    Raw(Vec<RawClipFormat>),
}

/// Raw 快照里的一种格式：fmt=标准格式 ID（CF_*）或注册格式（>=0xC000，带 name）
#[cfg(windows)]
pub struct RawClipFormat {
    pub fmt: u32,
    pub name: Option<String>,
    pub bytes: Vec<u8>,
}

impl ClipboardBackup {
    /// 快照里的文本（grab 判断「划词与原内容相同」用；Raw 取 CF_UNICODETEXT）
    pub fn text(&self) -> Option<String> {
        match self {
            Self::Empty => None,
            Self::Text(t) => Some(t.clone()),
            #[cfg(windows)]
            Self::Raw(fmts) => fmts
                .iter()
                .find(|f| f.fmt == win_clipboard::CF_UNICODETEXT_ID)
                .map(|f| win_clipboard::utf16_to_string(&f.bytes)),
        }
    }
}

// ---------- 慢应用「背刺」看门狗（两个平台共用） ----------

use std::sync::atomic::{AtomicU64, Ordering};

/// 「二次恢复」watcher 的代次：每轮新 grab 进门递增，旧 watcher 见代次不符立刻收手，
/// 防止两轮的备份互相打来打去
static WATCH_GEN: AtomicU64 = AtomicU64::new(0);

/// 新一轮 grab 开始，返回本轮代次（传给 spawn_second_chance）
pub fn bump_watch_gen() -> u64 {
    WATCH_GEN.fetch_add(1, Ordering::SeqCst) + 1
}

/// watcher 决策：恢复备份之后看到剪贴板又变了，该不该打回去
#[derive(Debug, PartialEq, Eq)]
pub enum WatchDecision {
    /// 新文本 == 本次划词 → 慢应用恢复后才写完（Word/大网页可迟 ~秒级），把备份打回去
    RestoreSnapshot,
    /// 新文本是别的 → 用户自己复制了内容，从此收手，绝不再动
    StandDown,
    /// 读不到文本（瞬时锁/写的是非文本/先清后写）→ 继续看到超时
    KeepWatching,
}

/// 纯函数决策，供单测。只有「恰好等于本次划词文本」才动手——用户窗口期内自己
/// 复制的内容不会被顶回去（误伤面收窄到「2.5s 内手动复制了与选区逐字相同的内容」）
pub fn watch_decision(
    new_text: Result<Option<String>, ()>,
    grabbed: &str,
    owner_is_self: bool,
) -> WatchDecision {
    if owner_is_self {
        return WatchDecision::StandDown;
    }
    match new_text {
        Err(()) => WatchDecision::KeepWatching,
        Ok(None) => WatchDecision::KeepWatching,
        Ok(Some(t)) => {
            let t = t.trim();
            if t.is_empty() {
                WatchDecision::KeepWatching
            } else if t == grabbed.trim() {
                WatchDecision::RestoreSnapshot
            } else {
                WatchDecision::StandDown
            }
        }
    }
}

/// grab_selection 恢复备份后的一次性监视线程：~2.5s（60ms × 42 拍）盯「目标应用迟到的
/// 写入」——轮询窗口（~900ms）结束后 Word/大网页才把选中文本写进剪贴板的场景，
/// 由这里纠正回来。只在内容逐字匹配本次划词时动手，且只执行一次
pub fn spawn_second_chance(backup: ClipboardBackup, grabbed: String, base_seq: u64, gen: u64) {
    std::thread::spawn(move || {
        for _ in 0..42 {
            std::thread::sleep(std::time::Duration::from_millis(60));
            // 新一轮 grab 已开始：它的备份/恢复自己管，我们的快照已过时
            if WATCH_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            if clipboard_seq() == base_seq {
                continue;
            }
            match watch_decision(clipboard_read_text(0), &grabbed, clipboard_owner_is_self()) {
                WatchDecision::RestoreSnapshot => {
                    println!("[grab] 慢应用事后写入，二次恢复备份");
                    clipboard_restore(&backup);
                    return;
                }
                WatchDecision::StandDown => return,
                WatchDecision::KeepWatching => {}
            }
        }
    });
}

// ---------- 划词图标：「这个 app 读不到选区」的记忆（两个平台共用） ----------

/// 「这个 app 读不到选区」的记忆（两个平台共用）：**连续** N 轮探测都确认「焦点控件
/// 压根不提供文本模式」才拉黑，之后跳过 UIA 直读不再白等（macOS 唤醒轮询 ~640ms /
/// Windows UIA 重试 ~360ms）；读到过一次就清掉。
///
/// 为什么不能一次失败就拉黑：Chromium 系（Chrome/Edge/Electron）的 UIA 无障碍树是
/// 懒建的——客户端查询（WM_GETOBJECT）本身就是「敲门」，敲一两下树才建起来。一次
/// 失败就拉黑 = 停止敲门 = 树永远建不起来 = 图标在这个浏览器里永远不弹（死循环）。
/// 三振给足了唤醒窗口；真正的「不支持」应用（终端/自绘 UI）连输三局也就白等三次，
/// 各几百毫秒且都在探测线程，无感。
/// TTL 仍在：pid 会被系统复用，过期自动失效。
static SELECTION_HOPELESS: std::sync::OnceLock<
    parking_lot::Mutex<std::collections::HashMap<i32, (u64, u32)>>,
> = std::sync::OnceLock::new();

/// 连续这么多次 NoTextPattern 才拉黑
const HOPELESS_STRIKES: u32 = 3;
/// 三振的计数窗口：距上次失败超过这么久视为「不连续」，重新计数（用户隔天再划一次
/// 不该背着上次的失败记录）
const HOPELESS_STRIKE_WINDOW_MS: u64 = 60 * 1000;
/// 拉黑记忆的保质期：到期自动失效，就算 pid 复用撞车也最多错 10 分钟
const HOPELESS_TTL_MS: u64 = 10 * 60 * 1000;

fn hopeless_set() -> &'static parking_lot::Mutex<std::collections::HashMap<i32, (u64, u32)>> {
    SELECTION_HOPELESS.get_or_init(|| parking_lot::Mutex::new(std::collections::HashMap::new()))
}

fn hopeless_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn selection_hopeless(pid: i32) -> bool {
    let mut set = hopeless_set().lock();
    match set.get(&pid) {
        Some(&(at, strikes)) if strikes >= HOPELESS_STRIKES => {
            if hopeless_now_ms().saturating_sub(at) < HOPELESS_TTL_MS {
                true
            } else {
                set.remove(&pid); // 过期：顺手清掉
                false
            }
        }
        _ => false,
    }
}

pub(crate) fn mark_selection_hopeless(pid: i32) {
    let now = hopeless_now_ms();
    let mut set = hopeless_set().lock();
    let strikes = match set.get(&pid) {
        Some(&(last, n)) if now.saturating_sub(last) < HOPELESS_STRIKE_WINDOW_MS => n + 1,
        _ => 1,
    };
    set.insert(pid, (now, strikes));
}

pub(crate) fn clear_selection_hopeless(pid: i32) {
    hopeless_set().lock().remove(&pid);
}

/// 系统设置里的「双击间隔」（毫秒）。Windows 侧不动：`#[cfg(windows)]` 代码在 macOS
/// 上不参与编译，加 GetDoubleClickTime 只能盲写，保持 500ms 默认值
#[cfg(target_os = "macos")]
pub use macos::double_click_interval_ms;
#[cfg(windows)]
pub fn double_click_interval_ms() -> u64 {
    500
}

/// 后台线程执行 Objective-C/AppKit 代码前要自己开 autorelease pool：worker 线程
/// 没有主线程那个池，临时对象会攒到线程结束。Windows 无此概念，直通
#[cfg(target_os = "macos")]
pub use macos::with_release_pool;
#[cfg(windows)]
pub fn with_release_pool<R>(f: impl FnOnce() -> R) -> R {
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Result<Option<String>, ()> {
        Ok(Some(s.to_string()))
    }

    #[test]
    fn slow_app_writeback_restores_snapshot() {
        // 慢应用恢复后才写完：内容逐字（trim 后）等于本次划词 → 打回备份
        assert_eq!(watch_decision(t(" hello "), "hello", false), WatchDecision::RestoreSnapshot);
        assert_eq!(watch_decision(Ok(Some("hello".into())), " hello", false), WatchDecision::RestoreSnapshot);
    }

    #[test]
    fn user_copied_other_text_stands_down() {
        // 窗口期内用户自己复制了别的内容 → 绝不顶回去
        assert_eq!(watch_decision(t("something else"), "hello", false), WatchDecision::StandDown);
    }

    #[test]
    fn our_own_write_stands_down() {
        assert_eq!(watch_decision(t("hello"), "hello", true), WatchDecision::StandDown);
    }

    #[test]
    fn unreadable_or_non_text_keeps_watching() {
        // 瞬时锁 / 写的是非文本 / 先清后写中 → 继续盯，不动剪贴板
        assert_eq!(watch_decision(Err(()), "hello", false), WatchDecision::KeepWatching);
        assert_eq!(watch_decision(Ok(None), "hello", false), WatchDecision::KeepWatching);
        assert_eq!(watch_decision(t("  "), "hello", false), WatchDecision::KeepWatching);
    }

    #[test]
    fn watch_gen_bump_is_monotonic_and_visible() {
        let a = bump_watch_gen();
        let b = bump_watch_gen();
        assert!(b > a, "代次必须严格递增");
    }
}
