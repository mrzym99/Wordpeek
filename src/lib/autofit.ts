import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** 卡片上下 padding（16×2）+ 边框（1×2）+ 1px 取整余量 */
const CARD_CHROME = 35;
/** 高度变化小于该值不上报，避免来回抖动 */
const EPSILON = 3;
/** 用户手动拖拽窗口后，暂停自动适配的静默期（ms）：拖拽中窗口尺寸由用户主导， */
/** 自动适配反向 set_size 会和系统拖拽循环互相触发，陷入「拔河」死循环 */
const USER_RESIZE_QUIET = 300;

/**
 * 让窗口高度跟着卡片内容走：内容少时窗口收缩到刚好包住卡片，
 * 不再留下「透明但吃点击」的空白区域。返回内容容器的 ref——
 * 该容器高度即内容自然高度（不受 .card 的 height:100% 影响）。
 * min/max：钳制上报高度，内容在两者之间时窗口不动，
 * 避免查词过程中高度反复跳变闪烁；超出 max 时容器自己出滚动条
 * （配合调用方给容器设 max-height + overflow-y）。
 */
export function useAutoFitWindow(
  label: string,
  chrome: number = CARD_CHROME,
  min = 0,
  max = Infinity
) {
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!body) return;
    let last = 0;
    let userResizeAt = 0;
    let programmatic = false;
    let trailing: ReturnType<typeof setTimeout> | undefined;

    const canApply = () => Date.now() - userResizeAt > USER_RESIZE_QUIET;
    const apply = () => {
      const h = Math.min(max, Math.max(min, Math.ceil(body.offsetHeight) + chrome));
      if (Math.abs(h - last) < EPSILON) return;
      last = h;
      programmatic = true;
      invoke("fit_window", { label, height: h }).catch(() => {});
    };
    // 静默期内的变化不丢：停一场 trailing 补测一次
    const schedule = () => {
      if (trailing !== undefined) return;
      trailing = setTimeout(() => {
        trailing = undefined;
        if (canApply()) apply();
      }, USER_RESIZE_QUIET + 20);
    };

    // 不用 requestAnimationFrame：窗口显示前是隐藏状态，rAF 会被节流甚至不触发
    const ro = new ResizeObserver(() => (canApply() ? apply() : schedule()));
    ro.observe(body);
    apply(); // 首次测量（ResizeObserver 只保证后续变化）

    // 用户拖拽窗口边缘：暂停自动适配，拖完 300ms 后补一次。
    // 自己 set_size 引发的 onResized 用 programmatic 标记跳过，
    // 不当作「用户在拖拽」——否则正常适配会被自己误伤
    let unlistenResize: (() => void) | undefined;
    getCurrentWindow()
      .onResized(() => {
        if (programmatic) {
          programmatic = false;
          return;
        }
        userResizeAt = Date.now();
        schedule();
      })
      .then((f) => (unlistenResize = f));

    // 窗口从隐藏态被 show 出来时，隐藏期间的 set_size 可能没落地：
    // 拿到焦点即强制重报一次，保证显示出来的尺寸就是内容尺寸
    const refit = () => {
      last = 0;
      if (canApply()) apply();
      else schedule();
    };
    window.addEventListener("focus", refit);
    return () => {
      ro.disconnect();
      unlistenResize?.();
      window.removeEventListener("focus", refit);
      if (trailing !== undefined) clearTimeout(trailing);
    };
  }, [body, label, chrome, min, max]);
  return setBody;
}
