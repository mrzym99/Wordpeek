import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** 卡片上下 padding（16×2）+ 边框（1×2）+ 1px 取整余量（.card 已无阴影边距） */
const CARD_CHROME = 35;
/** 高度变化小于该值不上报，避免来回抖动 */
const EPSILON = 3;
/** 拖拽后暂停自动适配的静默期（ms）：适配反向 set_size 会和系统拖拽循环互相触发，陷入「拔河」死循环 */
const USER_RESIZE_QUIET = 300;
/** 自己 set_size 引发的 onResized 抑制窗口（ms） */
const RESIZE_SUPPRESS = 80;
/** 上报高度多留的余量（px）：DPI 取整（125%/150%）轻易少 1px，顶满时被裁出滚动条；余量成为卡片底部留白 */
const FIT_SLACK = 4;

/**
 * 窗口高度跟着卡片内容走：内容少收缩到刚好包住卡片，多则长到屏幕上限
 * （Rust fit_height 钳工作区）后框内滚动。返回内容容器的 ref。
 * 量自然高度 max(offsetHeight, scrollHeight)：容器须只收缩不拉伸
 * （.card-body 用 flex: 0 1 auto），否则量到被拉伸后的高度，自适应退化成「永远不变」。
 * min/max 钳制上报高度，避免查词过程中高度反复跳变；上限一般不传，交给 Rust 按屏幕钳制。
 */
export function useAutoFitWindow(
  label: string,
  chrome: number = CARD_CHROME,
  min = 0,
  max = Infinity
): { setBody: (el: HTMLDivElement | null) => void; refit: () => Promise<void> } {
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  // refit 要重置 effect 内部状态，经 ref 暴露稳定引用给调用方
  const refitRef = useRef<() => Promise<void>>(() => Promise.resolve());
  useEffect(() => {
    if (!body) return;
    let target = 0; // 已提交的目标高度（去重用它比）
    let userResizeAt = 0;
    let suppressUntil = 0;
    let trailing: ReturnType<typeof setTimeout> | undefined;

    const canApply = () => Date.now() - userResizeAt > USER_RESIZE_QUIET;
    let fitFailed = false;
    const send = (h: number): Promise<void> => {
      suppressUntil = Date.now() + RESIZE_SUPPRESS;
      return invoke("fit_window", { label, height: h, viewport: window.innerHeight })
        .then((applied) => {
          fitFailed = false;
          // 请求高 > 实际生效高 = Rust 钳到屏幕上限、内容真装不下，才允许出滚动条；CSS 据此遮条
          document.documentElement.classList.toggle(
            "is-capped",
            h - Number(applied) > 0.5
          );
        })
        .catch(() => {
          // 偶发失败后补测重试一次；必须清零 target，否则 < EPSILON 短路，重试空转、窗口卡在错误高度
          if (!fitFailed) {
            fitFailed = true;
            target = 0;
            schedule();
          }
        });
    };
    // 不做逐帧动画：每帧一次前端→Rust 的 IPC 往返，帧率稳不住，反而比直接跳变更卡
    const apply = (): Promise<void> => {
      // scrollHeight：内容被盒子截断时才是真正的自然高度
      const content = Math.max(body.offsetHeight, body.scrollHeight);
      const h = Math.min(max, Math.max(min, Math.ceil(content) + chrome + FIT_SLACK));
      if (Math.abs(h - target) < EPSILON) return Promise.resolve();
      target = h;
      return send(h);
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
    // 盒子到 max-height 封顶后 ResizeObserver 不再触发（内容再多盒子不变大），改盯 DOM 节点增删
    const mo = new MutationObserver(() => (canApply() ? apply() : schedule()));
    mo.observe(body, { childList: true, subtree: true, characterData: true });
    apply(); // 首次测量（ResizeObserver 只保证后续变化）

    // 用户拖拽：暂停适配，停 300ms 后补一次；自己 set_size 引发的 onResized 用抑制窗口跳过，
    // 否则正常适配被自己误伤
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

    // 窗口从隐藏态被 show 出来时，隐藏期间的 set_size 可能没落地：拿到焦点即强制重报一次
    // refit 返回 Promise（fit_window 已落地）：调用方可以「先量好尺寸再 show」，避免先闪一个旧高度
    const refit = (): Promise<void> => {
      // target 清零强制重报；fitFailed 后 target=0 的重试语义与此一致。
      // 显式 refit 是“现在就量”，清掉拖拽静默期，不然刚 setSize 过的话要等 300ms 才发
      target = 0;
      userResizeAt = 0;
      return apply();
    };
    window.addEventListener("focus", refit);
    refitRef.current = refit;
    return () => {
      disposed = true;
      ro.disconnect();
      mo.disconnect();
      unlistenResize?.();
      window.removeEventListener("focus", refit);
      if (trailing !== undefined) clearTimeout(trailing);
    };
  }, [body, label, chrome, min, max]);
  const refit = useCallback(() => refitRef.current(), []);
  return { setBody, refit };
}
