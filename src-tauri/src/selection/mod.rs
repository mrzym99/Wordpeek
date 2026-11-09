//! 划词会话域：手势判定（gesture）、图标显隐（icon）、探测（probe）、
//! 热键兜底（fallback）与选区文本清洗（normalize）。
//!
//! 会话状态收敛在 [`SessionManager`]（Tauri managed state）——划词会话唯一所有者；
//! 钩子回调触达的鼠标原始数据留在 gesture.rs / icon.rs 的静态变量（回调拿不到
//! AppHandle，且平台层铁律要求回调内只做内存读）。
//!
//! 所有进卡片的路径统一走 [`SessionManager::show_card`] 合流点：格式清洗 →
//! 同文本去重 → 位置钳制 → emit `selection`。

mod fallback;
mod normalize;
pub(crate) mod probe; // windows.rs 看门狗读 LAST_WORKER_UP_MS（[DEBUG-m0u5] 插桩，诊断完改回 mod probe）
// gesture / icon 的个别符号被 main.rs 命令与 invoke_handler 直接引用，公开到 crate 内
pub(crate) mod gesture;
pub(crate) mod icon;

use tauri::{AppHandle, Emitter, Manager};

pub(crate) use fallback::handle_hotkey_trigger;
pub(crate) use probe::start_selection_icon;

/// 下发卡片弹窗的来源（随 `selection` 事件带给前端/诊断日志）
pub(crate) const SOURCE_ICON: &str = "icon";
pub(crate) const SOURCE_HOTKEY: &str = "hotkey";
pub(crate) const SOURCE_HOTKEY_FALLBACK: &str = "hotkey-fallback";

/// 下发卡片弹窗的 payload。`source` 是新增字段，旧前端会忽略
#[derive(serde::Serialize, Clone)]
pub(crate) struct Selection {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub source: &'static str,
}

/// 同文本去重窗口（毫秒）：连点/抖动在窗口内不重复弹同一内容
const DEDUP_WINDOW_MS: u64 = 1500;

/// 划词会话唯一所有者（managed state）。钩子回调不能触碰（拿不到 AppHandle），
/// 其余划词路径从这里读写
pub(crate) struct SessionManager(parking_lot::Mutex<Session>);

impl Default for SessionManager {
    fn default() -> Self {
        Self(parking_lot::Mutex::new(Session::default()))
    }
}

#[derive(Default)]
struct Session {
    /// 选区代次：每次鼠标按下 +1。探测异步做（不能堵 worker），show 前对代次，
    /// 期间有新按下（用户已开始新选区）就放弃
    gen: u64,
    /// 划词图标最近一次探测到的选中文本，点图标时直接用；
    /// 点击后焦点已转走，再读源应用选区就读不到了
    icon_text: Option<String>,
    /// 刚点过图标的时刻（毫秒）。点击后的松开事件仍会触发探测且源应用选区还在，
    /// 图标会立刻弹回，故静默窗口内的探测跳过
    icon_clicked_at: u64,
    /// 上一次下发卡片的内容与时刻：同文本去重用（连点/抖动不重复弹）
    last_text: Option<String>,
    last_shown_ms: u64,
}

impl SessionManager {
    fn lock(&self) -> parking_lot::MutexGuard<'_, Session> {
        self.0.lock()
    }

    /// 新一轮鼠标按下：代次 +1（旧的异步探测结果全部过期）
    pub(crate) fn bump_gen(&self) {
        self.lock().gen += 1;
    }

    pub(crate) fn gen(&self) -> u64 {
        self.lock().gen
    }

    /// 记录探测到的选中文本（弹图标前）
    pub(crate) fn set_icon_text(&self, text: Option<String>) {
        self.lock().icon_text = text;
    }

    /// 点图标：取走缓存的文本并记录点击时刻
    pub(crate) fn take_icon_text(&self, now: u64) -> Option<String> {
        let mut s = self.lock();
        s.icon_clicked_at = now;
        s.icon_text.take()
    }

    /// 刚点过图标不久（探测静默窗口内，防「点完图标图标立刻弹回」）
    pub(crate) fn icon_clicked_recently(&self, now: u64) -> bool {
        let s = self.lock();
        now.saturating_sub(s.icon_clicked_at) < probe::ICON_REPROBE_QUIET_MS
    }

    /// 卡片下发合流点：图标点击与热键路径都走这里。统一做格式清洗（拦截空壳）、
    /// 同文本去重（窗口内且卡片仍可见）、位置钳制，再发 `selection` 事件
    pub(crate) fn show_card(app: &AppHandle, raw: &str, x: i32, y: i32, source: &'static str) {
        let Some(text) = normalize::normalize_selection(raw) else {
            println!("[card] 选区清洗后为空，不下发（{source}）");
            return;
        };
        {
            let state = app.state::<SessionManager>();
            let mut s = state.lock();
            let now = now_ms();
            let card_visible = app
                .get_webview_window("main")
                .is_some_and(|w| w.is_visible().unwrap_or(false));
            if s.last_text.as_deref() == Some(text.as_str())
                && now.saturating_sub(s.last_shown_ms) < DEDUP_WINDOW_MS
                && card_visible
            {
                println!("[card] 同文本 {DEDUP_WINDOW_MS}ms 内已下发，跳过（{source}）");
                return;
            }
            s.last_text = Some(text.clone());
            s.last_shown_ms = now;
        }
        let (cx, cy) = crate::geometry::clamp_to_screen(app, "main", x, y);
        let _ = app.emit(
            "selection",
            Selection {
                text,
                x: cx,
                y: cy,
                source,
            },
        );
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
