//! 划词探测：鼠标松开 → 手势判定 → UIA/AX 读选区 → 弹图标。
//! 探测全在独立线程（UIA 对 Chromium 应用要重试等 120ms+，堵住会撑爆信号通道丢鼠标事件），
//! 代次防过期：期间用户开始新选区就丢弃结果。

use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager};

use super::fallback::grab_in_progress;
use super::gesture::{PROBE_INTENT, mouse_gesture_selects, record_mouse_down};
use super::icon::{cursor_on_icon_rect, cursor_on_window, hide_selection_icon, show_selection_icon};
use super::{SessionManager, now_ms};
use crate::platform;

/// 鼠标信号：Down=按下、Up=松开（带松手位置，锚点用）、Leave=图标可见时鼠标飘远
enum Signal {
    Down,
    Up { x: i32, y: i32 },
    Leave,
}

/// 光标离开图标矩形这么远（物理像素）就收起图标；留这一圈余量是给「把鼠标挪过去点图标」的路径
const ICON_LEAVE_PX: i32 = 48;
/// 探测完成时光标离松手点超过这么远就丢弃：很多应用一直报旧选区，人不在不该弹；400px 给 Cmd+C 兜底（≥300ms）留余量
const ICON_NEAR_PX: i32 = 400;
/// 刚点过图标后的探测静默窗口（毫秒）：点击后的松开事件仍会触发探测且源应用选区还在，图标会立刻弹回
pub(crate) const ICON_REPROBE_QUIET_MS: u64 = 400;

/// [DEBUG-m0u5] 临时插桩：worker 最近一次处理 Up 信号的 epoch ms（看门狗日志对比用）。诊断完删除
pub(crate) static LAST_WORKER_UP_MS: AtomicU64 = AtomicU64::new(0);

/// 划词图标：鼠标松开后读一次系统选区（不碰剪贴板、不注入按键），有选中文字就在光标旁弹按钮，
/// 点一下走热键同一套翻译链路；读不到选区（终端、Electron 等不实现 AX/UIA 文本接口）就什么都不做
pub(crate) fn start_selection_icon(app: &AppHandle) {
    // 队列只留一个信号：钩子回调绝不做窗口操作（跨线程查询/ShowWindow 拖慢全系统鼠标且会被摘钩），hide 都在 worker 里做
    let (tx, rx) = std::sync::mpsc::sync_channel::<Signal>(1);
    let tx_down = tx.clone();
    let tx_move = tx.clone();
    let watch = platform::start_mouse_watch(
        Box::new(move || {
            // 点在图标自身上不能收起：Windows 低级钩子连自家窗口的鼠标事件也收，down 就藏会吞掉这次 click
            super::icon::DOWN_ON_ICON.store(cursor_on_icon_rect(), Ordering::Relaxed);
            record_mouse_down();
            let _ = tx_down.try_send(Signal::Down);
        }),
        Box::new(move || {
            let (x, y) = platform::cursor_pos();
            PROBE_INTENT.store(mouse_gesture_selects(x, y), Ordering::Relaxed);
            let _ = tx.try_send(Signal::Up { x, y });
        }),
        // 移动：图标可见且光标已飘远 → 发 Leave 收起；只碰内存读 + 坐标比较，保持回调轻量
        Box::new(move |x, y| {
            if let Some((ix, iy, iw, ih)) = *super::icon::ICON_RECT.lock() {
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
        platform::accessibility_allowed()
    );
    crate::platform::debug_log(&format!(
        "[icon] 鼠标监听已装 watch={watch} 辅助功能={}",
        platform::accessibility_allowed()
    ));
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
                    worker.state::<SessionManager>().bump_gen();
                    if matches!(sig, Signal::Leave)
                        || !super::icon::DOWN_ON_ICON.load(Ordering::Relaxed)
                    {
                        hide_selection_icon(&worker);
                    }
                }
                Signal::Up { x, y } => {
                    LAST_WORKER_UP_MS.store(super::now_ms(), Ordering::Relaxed); // [DEBUG-m0u5]
                    crate::platform::debug_log(&format!("worker Up x={x} y={y}"));
                    probe_selection(&worker, x, y, ICON_NEAR_PX);
                }
            }
        }
    });
}

/// 松开且是选词手势：探测选区，成功且用户还没走远就在松手位置弹图标
fn probe_selection(worker: &AppHandle, ux: i32, uy: i32, near_px: i32) {
    // 未授权时什么选区都读不到（macOS 上重建 adhoc 包后最常见），连探测线程都不起。
    // 授权引导在启动时已经弹过（platform::init），这里只留一条日志说明卡在哪
    if !platform::accessibility_allowed() {
        crate::platform::debug_log("[probe] 辅助功能未授权，跳过探测");
        hide_selection_icon(worker);
        return;
    }
    // 开关每次现读配置：设置里一关就立即生效，不用重装监听
    let enabled = worker
        .state::<parking_lot::Mutex<crate::translate::AppConfig>>()
        .lock()
        .selection_icon;
    let clicked_recently = worker
        .state::<SessionManager>()
        .icon_clicked_recently(now_ms());
    if !enabled || !PROBE_INTENT.load(Ordering::Relaxed) || clicked_recently {
        println!(
            "[probe] 放弃：enabled={enabled} intent={} clicked_recently={clicked_recently}",
            PROBE_INTENT.load(Ordering::Relaxed)
        );
        crate::platform::debug_log(&format!(
            "probe 放弃 enabled={enabled} intent={} clicked_recently={clicked_recently}",
            PROBE_INTENT.load(Ordering::Relaxed)
        )); // [DEBUG-m0u5]
        hide_selection_icon(worker);
        return;
    }
    // 在自家窗口里划词（卡片内选中跳查、设置页复制等）不弹图标，否则在自己卡片里选词又冒出图标
    if ["main", "icon", "settings", "input"]
        .iter()
        .any(|&l| cursor_on_window(worker, l))
    {
        println!("[probe] 光标在自家窗口内，不弹图标");
        crate::platform::debug_log("probe 自家窗口内"); // [DEBUG-m0u5]
        hide_selection_icon(worker);
        return;
    }
    // 在探测线程读代次（发起时刻的）：worker 不堵，probe 线程对齐
    let gen = worker.state::<SessionManager>().gen();
    let prober = worker.clone();
    // 探测线程要自己开 autorelease pool：worker 线程没有主线程那个池，Obj-C 临时对象会攒到线程结束
    std::thread::spawn(move || {
        platform::with_release_pool(|| probe_and_show(&prober, ux, uy, near_px, gen))
    });
}

/// 探测选区并弹图标。`gen` 是发起时的选区代次：期间用户开始了新选区就丢弃本次结果
fn probe_and_show(worker: &AppHandle, ux: i32, uy: i32, near_px: i32, gen: u64) {
    // 上一轮的剪贴板兜底还没收尾（连点、连按快捷键）：放弃本次，但**不 hide**——上一轮可能
    // 刚把图标弹出来，hide 会让它闪一下就消失；只读标志不上锁，真撞同一毫秒少弹一次，不值
    if grab_in_progress() {
        println!("[probe] 上一轮剪贴板兜底仍在收尾，跳过本次探测");
        crate::platform::debug_log("probe grab_in_progress"); // [DEBUG-m0u5]
        return;
    }
    // 图标路径铁律：绝不碰剪贴板（也不注入复制按键）——UIA/AX 读不到选区（终端、
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
            // 光标已离松手点很远：用户去别处了，不追着报旧选区的应用弹图标
            let (cx, cy) = platform::cursor_pos();
            let far = (cx - ux).abs() + (cy - uy).abs() > near_px;
            if far {
                crate::platform::debug_log("probe 光标已远离"); // [DEBUG-m0u5]
                hide_selection_icon(worker);
            } else {
                crate::platform::debug_log(&format!("probe 弹图标 text={text:.20}")); // [DEBUG-m0u5]
                show_selection_icon(worker, text, ux, uy);
            }
        }
        _ if live => hide_selection_icon(worker),
        // 过期结果，什么都不做
        _ => crate::platform::debug_log("probe 结果已过期，丢弃"),
    }
}
