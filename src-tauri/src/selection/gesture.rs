//! 选词手势判定：从鼠标按下/松开序列判「这次值不值得探测」。纯函数核心（单测随行）；
//! 钩子回调写入的原始鼠标数据放在本模块静态变量——回调拿不到 AppHandle，
//! 且平台层铁律要求回调内只做内存读，绝不做窗口/系统查询。

use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use parking_lot::Mutex;

use super::now_ms;

/// 左键按下的位置与时刻（物理像素, 毫秒）：用来区分「选词手势」和普通单击
pub(crate) static MOUSE_DOWN: OnceLock<Mutex<(i32, i32, u64)>> = OnceLock::new();
/// 上一次松手的位置与时刻（同上），双击/三击判定用
pub(crate) static MOUSE_UP: OnceLock<Mutex<(i32, i32, u64)>> = OnceLock::new();
/// 本次松手是否值得探测选区；mouse-up 回调里算好，worker 直接读
pub(crate) static PROBE_INTENT: AtomicBool = AtomicBool::new(false);

/// 只有真的做了选词手势才探测：拖动（位移 ≥ 6 物理像素，曼哈顿距离）或连击（双击选词/三击选行）；
/// 普通单击不算——很多应用单击后 AXSelectedText 仍返回旧选区，不拦会「随便点一下也冒图标」
const GESTURE_MOVE_PX: i32 = 6;
const GESTURE_REPEAT_PX: i32 = 8;

/// 纯判定逻辑，供测试：本次按下/松开与上一次松开（px,py,prev_at）的坐标时间
pub(crate) fn gesture_selects(
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

/// 钩子 down 回调：记下按下位置（探测手势判定用）。只碰内存读 + GetCursorPos，保持回调轻量
pub(crate) fn record_mouse_down() {
    let (x, y) = crate::platform::cursor_pos();
    *MOUSE_DOWN.get_or_init(Default::default).lock() = (x, y, now_ms());
}

/// 钩子 up 回调：判手势 + 记松手位置，结果给 worker 的探测节点用
pub(crate) fn mouse_gesture_selects(x: i32, y: i32) -> bool {
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

/// 全局最近一次鼠标按下（供弹窗卡片在 macOS makeKey 失灵时判「点过卡片外」）
pub(crate) fn last_mouse_down() -> Option<(i32, i32, u64)> {
    let d = *MOUSE_DOWN.get_or_init(Default::default).lock();
    (d.2 != 0).then_some(d)
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
}
