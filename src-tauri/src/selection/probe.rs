//! 划词探测：鼠标松开 → 手势判定 → UIA/AX 读选区 → 弹小点。
//! 探测全在独立线程（UIA 对 Chromium 应用要重试等 120ms+，堵住会撑爆信号通道丢鼠标事件），
//! 代次防过期：期间用户开始新选区就丢弃结果。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager};

use super::fallback::grab_in_progress;
use super::gesture::{PROBE_INTENT, mouse_gesture_selects, record_mouse_down};
use super::dot::{cursor_on_dot_rect, cursor_on_window, hide_selection_dot, show_selection_dot};
use super::{SessionManager, now_ms};
use crate::platform;

/// 鼠标信号：Down=按下、Up=松开（带松手位置）、Near/Far=小点 proximity 边沿
enum Signal {
    Down,
    Up { x: i32, y: i32 },
    Near,
    Far,
}

/// 松手后鼠标停留不动满这么久（且选区未被新操作改变）才弹小点：静默轻提示的核心阈值
const SETTLE_MS: u64 = 350;
/// 静置期间光标漂移超过这么远（曼哈顿距离，物理像素）就算「挪走了」，不出点
const SETTLE_DRIFT_PX: i32 = 10;
/// 光标进入小点窗口外扩这么远的矩形即「靠近」，展开「翻译」胶囊（far→near 边沿触发）
const DOT_PROXIMITY_PX: i32 = 36;
/// 小点无交互这么久自动收起（展开态无超时；收回小点时重新计时）
const DOT_TIMEOUT_MS: u64 = 2500;
/// 探测完成时光标离松手点超过这么远就丢弃：很多应用一直报旧选区，人不在不该弹；400px 给 Cmd+C 兜底（≥300ms）留余量
const ICON_NEAR_PX: i32 = 400;
/// 刚点过小点后的探测静默窗口（毫秒）：点击后的松开事件仍会触发探测且源应用选区还在，小点会立刻弹回
pub(crate) const DOT_REPROBE_QUIET_MS: u64 = 400;

/// 静置判定进行中（Settling 态）：move 回调据此检查漂移
static SETTLE_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 静置期间光标是否漂移超限（只置不清；start_settle 进入时重置）
static SETTLE_DIRTY: AtomicBool = AtomicBool::new(false);
/// 静置原点 = 松手点：漂移比较基准
static SETTLE_ORIGIN: Mutex<(i32, i32)> = Mutex::new((0, 0));

/// [DEBUG-m0u5] 临时插桩：worker 最近一次处理 Up 信号的 epoch ms（看门狗日志对比用）。诊断完删除
pub(crate) static LAST_WORKER_UP_MS: AtomicU64 = AtomicU64::new(0);

/// 纯判定：静置到点后是否还成立（漂移脏标记未置位 ∧ 代次未变 ∧ 开关仍开）
fn settle_passes(dirty: bool, gen_start: u64, gen_now: u64, enabled: bool) -> bool {
    !dirty && gen_start == gen_now && enabled
}

/// 纯判定：光标是否落在矩形外扩 margin 的盒内（与旧 Leave 判定同构取反）；
/// 盒状而非欧氏距离，判定只做四次比较，钩子回调可用
fn within_margin(x: i32, y: i32, rect: (i32, i32, u32, u32), margin: i32) -> bool {
    let (rx, ry, rw, rh) = rect;
    x >= rx - margin
        && x < rx + rw as i32 + margin
        && y >= ry - margin
        && y < ry + rh as i32 + margin
}

/// 纯判定：小点是否已超时无交互
fn dot_expired(now_ms: u64, shown_at_ms: u64, timeout_ms: u64) -> bool {
    now_ms.saturating_sub(shown_at_ms) >= timeout_ms
}

/// 划词轻提示：鼠标松开后读一次系统选区（不碰剪贴板、不注入按键），有选中文字就在光标旁弹小点，
/// 靠近/点一下走热键同一套翻译链路；读不到选区（终端、Electron 等不实现 AX/UIA 文本接口）就什么都不做
pub(crate) fn start_selection_dot(app: &AppHandle) {
    // 队列只留一个信号：钩子回调绝不做窗口操作（跨线程查询/ShowWindow 拖慢全系统鼠标且会被摘钩），hide 都在 worker 里做
    let (tx, rx) = std::sync::mpsc::sync_channel::<Signal>(1);
    let tx_down = tx.clone();
    let tx_move = tx.clone();
    let watch = platform::start_mouse_watch(
        Box::new(move || {
            // 点在小点自身上不能收起：Windows 低级钩子连自家窗口的鼠标事件也收，down 就藏会吞掉这次 click
            super::dot::DOWN_ON_DOT.store(cursor_on_dot_rect(), Ordering::Relaxed);
            record_mouse_down();
            let _ = tx_down.try_send(Signal::Down);
        }),
        Box::new(move || {
            let (x, y) = platform::cursor_pos();
            PROBE_INTENT.store(mouse_gesture_selects(x, y), Ordering::Relaxed);
            let _ = tx.try_send(Signal::Up { x, y });
        }),
        // 移动：静置漂移置脏 + 小点 proximity 边沿检测。只碰内存读 + 坐标比较，保持回调轻量
        Box::new(move |x, y| {
            if SETTLE_ACTIVE.load(Ordering::Relaxed) {
                let (ox, oy) = *SETTLE_ORIGIN.lock();
                if (x - ox).abs() + (y - oy).abs() > SETTLE_DRIFT_PX {
                    SETTLE_DIRTY.store(true, Ordering::Relaxed);
                }
            }
            let near = if let Some(rect) = *super::dot::DOT_RECT.lock() {
                within_margin(x, y, rect, DOT_PROXIMITY_PX)
            } else {
                false
            };
            if near != super::dot::CURSOR_NEAR.swap(near, Ordering::Relaxed) {
                let _ = tx_move.try_send(if near { Signal::Near } else { Signal::Far });
            }
        }),
    );
    println!(
        "[dot] 鼠标监听已装，辅助功能={}",
        platform::accessibility_allowed()
    );
    crate::platform::debug_log(&format!(
        "[dot] 鼠标监听已装 watch={watch} 辅助功能={}",
        platform::accessibility_allowed()
    ));
    if !watch {
        eprintln!("[dot] 鼠标监听装不上，划词轻提示不可用");
        return;
    }
    let worker = app.clone();
    std::thread::spawn(move || {
        while let Ok(sig) = rx.recv() {
            match sig {
                // 按下：代次 +1 废掉在途探测/静置/超时；点在小点上则保留，等 click 命令（translate_selection_dot）接管
                Signal::Down => {
                    worker.state::<SessionManager>().bump_gen();
                    SETTLE_ACTIVE.store(false, Ordering::Relaxed);
                    super::dot::DOT_EXPANDED.store(false, Ordering::Relaxed);
                    if !super::dot::DOWN_ON_DOT.load(Ordering::Relaxed) {
                        super::dot::hide_selection_dot(&worker);
                    }
                }
                // 靠近：展开「翻译」胶囊（超时定时器见 DOT_EXPANDED 自动让位）
                Signal::Near => {
                    super::dot::DOT_EXPANDED.store(true, Ordering::Relaxed);
                    let _ = worker.emit(
                        "dot-state",
                        super::dot::DotState { near: true, guide: false },
                    );
                }
                // 移开：收回小点，重启无交互计时
                Signal::Far => {
                    super::dot::DOT_EXPANDED.store(false, Ordering::Relaxed);
                    let _ = worker.emit(
                        "dot-state",
                        super::dot::DotState { near: false, guide: false },
                    );
                    arm_dot_timeout(&worker);
                }
                Signal::Up { x, y } => {
                    let t_up = now_ms();
                    LAST_WORKER_UP_MS.store(t_up, Ordering::Relaxed); // [DEBUG-m0u5]
                    crate::platform::debug_log(&format!("worker Up x={x} y={y}"));
                    probe_selection(&worker, x, y, ICON_NEAR_PX, t_up);
                }
            }
        }
    });
}

/// 松开且是选词手势：探测选区，成功且用户还没走远就在松手位置弹小点
fn probe_selection(worker: &AppHandle, ux: i32, uy: i32, near_px: i32, t_up: u64) {
    // 未授权时什么选区都读不到（macOS 上重建 adhoc 包后最常见），连探测线程都不起。
    // 授权引导在启动时已经弹过（platform::init），这里只留一条日志说明卡在哪
    if !platform::accessibility_allowed() {
        crate::platform::debug_log("[probe] 辅助功能未授权，跳过探测");
        hide_selection_dot(worker);
        return;
    }
    // 开关每次现读配置：设置里一关就立即生效，不用重装监听
    let enabled = worker
        .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
        .lock()
        .selection_dot;
    let clicked_recently = worker
        .state::<SessionManager>()
        .dot_clicked_recently(now_ms());
    if !enabled || !PROBE_INTENT.load(Ordering::Relaxed) || clicked_recently {
        println!(
            "[probe] 放弃：enabled={enabled} intent={} clicked_recently={clicked_recently}",
            PROBE_INTENT.load(Ordering::Relaxed)
        );
        crate::platform::debug_log(&format!(
            "probe 放弃 enabled={enabled} intent={} clicked_recently={clicked_recently}",
            PROBE_INTENT.load(Ordering::Relaxed)
        )); // [DEBUG-m0u5]
        hide_selection_dot(worker);
        return;
    }
    // 在自家窗口里划词（卡片内选中跳查、设置页复制等）不弹小点，否则在自己卡片里选词又冒出小点
    if ["main", "icon", "settings", "input"]
        .iter()
        .any(|&l| cursor_on_window(worker, l))
    {
        println!("[probe] 光标在自家窗口内，不弹小点");
        crate::platform::debug_log("probe 自家窗口内"); // [DEBUG-m0u5]
        hide_selection_dot(worker);
        return;
    }
    // 在探测线程读代次（发起时刻的）：worker 不堵，probe 线程对齐
    let gen = worker.state::<SessionManager>().gen();
    let prober = worker.clone();
    // 探测线程要自己开 autorelease pool：worker 线程没有主线程那个池，Obj-C 临时对象会攒到线程结束
    std::thread::spawn(move || {
        platform::with_release_pool(|| probe_and_show(&prober, ux, uy, near_px, gen, t_up))
    });
}

/// 探测选区并弹小点。`gen` 是发起时的选区代次：期间用户开始了新选区就丢弃本次结果
fn probe_and_show(worker: &AppHandle, ux: i32, uy: i32, near_px: i32, gen: u64, t_up: u64) {
    // 上一轮的剪贴板兜底还没收尾（连点、连按快捷键）：放弃本次，但**不 hide**——上一轮可能
    // 刚把小点弹出来，hide 会让它闪一下就消失；只读标志不上锁，真撞同一毫秒少弹一次，不值
    if grab_in_progress() {
        println!("[probe] 上一轮剪贴板兜底仍在收尾，跳过本次探测");
        crate::platform::debug_log("probe grab_in_progress"); // [DEBUG-m0u5]
        return;
    }
    // 轻提示路径铁律：绝不碰剪贴板（也不注入复制按键）——UIA/AX 读不到选区（终端、
    // 部分 Electron/自绘 UI）就不弹图标，用户要翻这些应用里的词请走热键
    // （只有热键路径的 grab_selection 允许剪贴板兜底，且全程备份/恢复）
    let text = match platform::focused_selection() {
        platform::SelectionRead::Text(t) => {
            crate::platform::debug_log(&format!("probe 读到选区 {:.20}", t));
            Some(t)
        }
        other => {
            println!("[probe] UIA/AX 未读到选区：{other:?}");
            crate::platform::debug_log(&format!("probe UIA 未读到：{other:?}")); // [DEBUG-m0u5]
            None
        }
    };
    let live = worker.state::<SessionManager>().gen() == gen;
    match text {
        // 等待期间用户又按下了鼠标（新选区开始）：本次结果已过期，丢弃
        Some(text) if live => {
            // 光标已离松手点很远：用户去别处了，不追着报旧选区的应用弹小点
            let (cx, cy) = platform::cursor_pos();
            let far = (cx - ux).abs() + (cy - uy).abs() > near_px;
            if far {
                crate::platform::debug_log("probe 光标已远离");
                hide_selection_dot(worker);
            } else if dot_gated() {
                // 可编辑焦点：轻提示降噪，不出点（有诊断日志）
                hide_selection_dot(worker);
            } else {
                crate::platform::debug_log(&format!("probe 静置判定 text={text:.20}"));
                start_settle(worker, text, t_up, ux, uy, gen);
            }
        }
        _ if live => hide_selection_dot(worker),
        // 过期结果，什么都不做
        _ => crate::platform::debug_log("probe 结果已过期，丢弃"),
    }
}

/// 轻提示闸门：焦点控件可编辑/敏感（输入框、IDE、终端）→ 不出小点。
/// 在探测线程调用（与 focused_selection 同线程，UIA 会话安全）
fn dot_gated() -> bool {
    if platform::focused_editable() {
        println!("[probe] 焦点控件可编辑/敏感，不出小点");
        return true;
    }
    false
}

/// 静置判定：松手起算 SETTLE_MS，到点复查三条件（漂移脏标记 / gen / 开关），
/// 全过才弹小点。单次定时器非轮询——探测本身耗 120~360ms，多数情况到点即弹
fn start_settle(worker: &AppHandle, text: String, t_up: u64, ux: i32, uy: i32, gen: u64) {
    SETTLE_DIRTY.store(false, Ordering::Relaxed);
    *SETTLE_ORIGIN.lock() = (ux, uy);
    SETTLE_ACTIVE.store(true, Ordering::Relaxed);
    let worker = worker.clone();
    std::thread::spawn(move || {
        let elapsed = now_ms().saturating_sub(t_up);
        let remain = SETTLE_MS.saturating_sub(elapsed).max(20);
        std::thread::sleep(std::time::Duration::from_millis(remain));
        SETTLE_ACTIVE.store(false, Ordering::Relaxed);
        let enabled = worker
            .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
            .lock()
            .selection_dot;
        let gen_now = worker.state::<SessionManager>().gen();
        if !settle_passes(SETTLE_DIRTY.load(Ordering::Relaxed), gen, gen_now, enabled) {
            crate::platform::debug_log("[settle] 静置判定未通过，不出点");
            return;
        }
        show_selection_dot(&worker, text, ux, uy);
        arm_dot_timeout(&worker);
    });
}

/// 小点无交互超时：单次定时器，到期复查（展开态不收）。Far 收回时重新武装并递增
/// 纪元，旧定时器醒来发现纪元已变自动让位——否则 show 时的旧定时器会在
/// 「展开→移开」重启计时后按旧时刻把小点提前收掉
fn arm_dot_timeout(worker: &AppHandle) {
    let worker = worker.clone();
    let epoch = super::dot::DOT_TIMER_EPOCH.fetch_add(1, Ordering::Relaxed) + 1;
    let shown_at = now_ms();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(DOT_TIMEOUT_MS));
        if super::dot::DOT_TIMER_EPOCH.load(Ordering::Relaxed) != epoch {
            return; // 已有更新的武装（Far 重启计时），本轮让位
        }
        if !dot_expired(now_ms(), shown_at, DOT_TIMEOUT_MS)
            || super::dot::DOT_EXPANDED.load(Ordering::Relaxed)
        {
            return;
        }
        crate::platform::debug_log("[dot] 无交互超时，收起");
        hide_selection_dot(&worker);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: (i32, i32, u32, u32) = (100, 100, 40, 40);

    #[test]
    fn within_margin_is_box_expanded_rect() {
        // 矩形内 → 近
        assert!(within_margin(120, 120, RECT, DOT_PROXIMITY_PX));
        // 外扩 margin 的边界内（含边）→ 近：x 最大 100+40+36-1=175
        assert!(within_margin(175, 120, RECT, DOT_PROXIMITY_PX));
        // 边界外 1px → 远
        assert!(!within_margin(176, 120, RECT, DOT_PROXIMITY_PX));
        assert!(!within_margin(63, 120, RECT, DOT_PROXIMITY_PX)); // 100-36-1
        assert!(!within_margin(120, 176, RECT, DOT_PROXIMITY_PX));
    }

    #[test]
    fn settle_passes_requires_still_fresh_and_enabled() {
        assert!(settle_passes(false, 7, 7, true));
        // 期间鼠标挪过（漂移 > 10px 置脏）
        assert!(!settle_passes(true, 7, 7, true));
        // 期间开始了新选区（gen 变）
        assert!(!settle_passes(false, 7, 8, true));
        // 开关中途被关（Alt+W / 托盘）
        assert!(!settle_passes(false, 7, 7, false));
    }

    #[test]
    fn dot_expired_boundary() {
        assert!(!dot_expired(1000, 1000, DOT_TIMEOUT_MS)); // 刚展示
        assert!(!dot_expired(3499, 1000, DOT_TIMEOUT_MS));
        assert!(dot_expired(3500, 1000, DOT_TIMEOUT_MS)); // 恰好到期
    }
}
