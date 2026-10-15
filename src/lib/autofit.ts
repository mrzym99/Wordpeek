import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** 卡片上下 padding（16×2）+ 边框（1×2）+ 1px 取整余量 */
const CARD_CHROME = 35;
/** 高度变化小于该值不上报，避免来回抖动 */
const EPSILON = 3;
/** 用户手动拖拽窗口后，暂停自动适配的静默期（ms）：拖拽中窗口尺寸由用户主导， */
/** 自动适配反向 set_size 会和系统拖拽循环互相触发，陷入「拔河」死循环 */
const USER_RESIZE_QUIET = 300;
/** 自己 set_size 引发的 onResized 抑制窗口（ms）：每发一帧续期，动画结束后很快失效 */
const RESIZE_SUPPRESS = 80;
/** 窗口高度过渡时长（ms）与缓动帧数无关，用时间轴算 */
const ANIM_MS = 140;
/** 变化小于该值直接跳，不值得动画（微抖） */
const ANIM_MIN_DELTA = 12;
/** 上报高度多留的余量（px）：逻辑高 撞上窗口 DPI 取整（125%/150%）轻易就少 1px， */
/** 内容刚好顶满时会被裁出个滚动条；这点白边在卡片 16px 内边距里看不出来 */
const FIT_SLACK = 4;

/**
 * 让窗口高度跟着卡片内容走：内容少时窗口收缩到刚好包住卡片，
 * 内容多时长到屏幕允许的上限（Rust fit_height 钳工作区），再高就框内滚动，
 * 不再留下「透明但吃点击」的空白区域。返回内容容器的 ref。
 * 量的是内容自然高度 max(offsetHeight, scrollHeight)：
 * 调用方须让容器只收缩不拉伸（.card-body 用 flex: 0 1 auto），
 * 否则量到的是被窗口拉伸后的盒子高度，自适应会退化成「永远不变」。
 * min/max：钳制上报高度，内容在两者之间时窗口不动，
 * 避免查词过程中高度反复跳变闪烁；上限一般不传，交给 Rust 按屏幕钳制。
 * 高度变化用 ~140ms 缓动逐帧 set_size（前端驱动，Rust 只是唯一写入者）。
 */
export function useAutoFitWindow(
  label: string,
  chrome: number = CARD_CHROME,
  min = 0,
  max = Infinity
): { setBody: (el: HTMLDivElement | null) => void; refit: () => void } {
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  // refit 需要重置 effect 内部的 target/cur 状态，经由 ref 暴露稳定引用给调用方
  const refitRef = useRef<() => void>(() => {});
  useEffect(() => {
    if (!body) return;
    let target = 0; // 已提交的目标高度（去重用它比，动画中途改目标不会重复发）
    let cur = 0; // 上一帧真正发出去的高度；0 = 还没发过（首次直接跳）
    let animFrom = 0;
    let animStart = 0;
    let rafId: number | undefined;
    let userResizeAt = 0;
    let suppressUntil = 0;
    let trailing: ReturnType<typeof setTimeout> | undefined;

    const canApply = () => Date.now() - userResizeAt > USER_RESIZE_QUIET;
    // 类名挂在 <html> 上：要遮的是页面里全部滚动容器（.card / .card-body / 原文框），
    // 不只是一个元素；动画落定后去掉类，真滚动不了才重新出条
    const setFitting = (on: boolean) =>
      document.documentElement.classList.toggle("is-fitting", on);
    let fitFailed = false;
    const send = (h: number) => {
      cur = h;
      suppressUntil = Date.now() + RESIZE_SUPPRESS;
      invoke("fit_window", { label, height: h })
        .then(() => {
          fitFailed = false;
        })
        .catch(() => {
          // 偶发失败（窗口暂不可写等）不再永久静默：静默期后补测重试一次
          if (!fitFailed) {
            fitFailed = true;
            schedule();
          }
        });
    };
    const tick = () => {
      const f = Math.min(1, (performance.now() - animStart) / ANIM_MS);
      const eased = 1 - (1 - f) ** 3; // ease-out cubic
      send(Math.round(animFrom + (target - animFrom) * eased));
      if (f >= 1) setFitting(false); // 落定：没真溢出就不出滚动条，溢出了才出
      rafId = f < 1 ? requestAnimationFrame(tick) : undefined;
    };
    const apply = () => {
      // scrollHeight：内容被盒子截断时才是真正的自然高度
      const content = Math.max(body.offsetHeight, body.scrollHeight);
      const h = Math.min(max, Math.max(min, Math.ceil(content) + chrome + FIT_SLACK));
      if (Math.abs(h - target) < EPSILON) return;
      target = h;
      if (rafId !== undefined) cancelAnimationFrame(rafId);
      // 窗口隐藏时 rAF 会被节流（动画停在中间帧），直接发准确目标
      if (!cur || document.hidden || Math.abs(h - cur) < ANIM_MIN_DELTA) {
        send(h);
        setFitting(false);
        return;
      }
      animFrom = cur;
      setFitting(true); // 动画期间窗口暂时比内容矮，先藏滚动条
      animStart = performance.now();
      tick();
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
    // 自己 set_size（包成动画则是一连串）引发的 onResized 用抑制时间跳过，
    // 不当作「用户在拖拽」——否则正常适配会被自己误伤
    let disposed = false;
    let unlistenResize: (() => void) | undefined;
    getCurrentWindow()
      .onResized(() => {
        if (Date.now() < suppressUntil) return;
        userResizeAt = Date.now();
        schedule();
      })
      .then((f) => {
        if (disposed) f(); // 晚到的订阅：立刻退订，不留泄漏
        else unlistenResize = f;
      });

    // 窗口从隐藏态被 show 出来时，隐藏期间的 set_size 可能没落地：
    // 拿到焦点即强制重报一次，保证显示出来的尺寸就是内容尺寸（不带动画）
    const refit = () => {
      target = 0;
      cur = 0;
      if (canApply()) apply();
      else schedule();
    };
    window.addEventListener("focus", refit);
    refitRef.current = refit;
    return () => {
      disposed = true;
      ro.disconnect();
      unlistenResize?.();
      window.removeEventListener("focus", refit);
      if (trailing !== undefined) clearTimeout(trailing);
      if (rafId !== undefined) cancelAnimationFrame(rafId);
    };
  }, [body, label, chrome, min, max]);
  const refit = useCallback(() => refitRef.current(), []);
  return { setBody, refit };
}
