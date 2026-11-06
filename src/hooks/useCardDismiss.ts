import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { externalClickAfterShow, pointInRect, shouldDismiss } from "../lib/cardDismiss";

/**
 * 卡片收起轮询（设计文档 §5.1）：可见期间每 300ms 求值一次 shouldDismiss 谓词。
 *
 * 为什么轮询而不是等 blur 事件：macOS Accessory 应用 makeKey 偶发不生效，焦点从来没
 * 立起来就永远不会有 blur，只靠 blur 启动轮询的方案卡片会整个滞留。轮询从 show 就启动，
 * 每次求值幂等（isFocused/last_mouse_down 均无副作用查询），丢了下一轮补上。
 */
export function useCardDismiss(opts: {
  /** 卡片根节点：卡片外的点击由 DOM onDocMouseDown 判定 */
  cardRef: React.RefObject<HTMLDivElement | null>;
  /** 收起动作：组件负责清空会话状态（hideAndReset） */
  onDismiss: () => void;
}) {
  const onDismissRef = useRef(opts.onDismiss);
  onDismissRef.current = opts.onDismiss;
  // show 后启动收起轮询的入口（startHideCheck 定义在 effect 闭包内，经 ref 暴露）
  const startRef = useRef<() => void>(() => {});
  // 本场 show 的时刻（epoch ms）：只认这之后的全局点击；入场那次按下（选词手势/图标点击）不算
  const shownAtRef = useRef(0);

  useEffect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    let hideCheck: number | undefined;
    const stopHideCheck = () => {
      if (hideCheck !== undefined) {
        clearInterval(hideCheck);
        hideCheck = undefined;
      }
    };
    const startHideCheck = () => {
      if (hideCheck !== undefined) return;
      // 显式走 window.setInterval：DOM 重载返回 number，避免 Node 的 Timeout 类型混入
      hideCheck = window.setInterval(maybeHide, 300);
    };
    startRef.current = startHideCheck;

    // 本场卡片真正拿到过焦点：判别收起走 blur 语义还是全局鼠标语义。
    // mac Accessory 应用 makeKey 偶发不生效，焦点从未立起来，blur 通道整个失灵——
    // 这种状态收起不看焦点，改看全局鼠标数据
    let everFocused = false;

    // 收起判定：事实逐项收集（IPC 失败一律按「事实未知，不收」处理，下一轮补上）
    const maybeHide = async () => {
      // 鼠标在卡片内：不收，也省掉本轮 IPC
      if (document.documentElement.matches(":hover")) return;
      let focused: boolean;
      try {
        focused = await win.isFocused();
      } catch {
        return;
      }
      let externalClick = false;
      if (!everFocused && !focused) {
        // 焦点从未立起来：用全局鼠标按下记录判「show 之后是否点过卡片外」——
        // 点了外部就收，没点（鼠标停在原地读卡片）继续留着
        let md: [number, number, number] | null = null;
        try {
          md = await invoke<[number, number, number] | null>("last_mouse_down");
        } catch {
          return;
        }
        if (!md) return; // 没有任何按下记录（防御）：按「还没点过外部」处理
        let pos: { x: number; y: number } | undefined;
        let size: { width: number; height: number } | undefined;
        try {
          [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
        } catch {
          return;
        }
        externalClick = externalClickAfterShow(
          md,
          shownAtRef.current,
          pos !== undefined && size !== undefined && pointInRect(md[0], md[1], pos, size),
        );
      }
      if (
        shouldDismiss({
          cursorInCard: false, // 开头已排除（:hover 即 return）
          everFocused,
          focused,
          externalClick,
        })
      ) {
        dismiss();
      }
    };
    // 收起 = 停轮询 + 执行组件的清状态动作（原版 hideAndReset 就以 stopHideCheck 开头；
    // 不停的话藏起后每 300ms 空转一轮，还可能误清新划词刚立起的会话）
    const dismiss = () => {
      stopHideCheck();
      onDismissRef.current();
    };
    // blur 后鼠标在卡片里被豁免，离开卡片后补一次失焦复查
    const onLeave = () => maybeHide();

    // 点卡片外即收起（卡片已贴窗口边缘，窗口内即卡片）：这类点击不产生
    // blur，macOS 失焦后无新 blur，卡片会滞留；dragStrip/DictPop 都在 .card 内不受影响
    const onDocMouseDown = (e: MouseEvent) => {
      if (!opts.cardRef.current?.contains(e.target as Node)) dismiss();
    };

    const unlistenBlur = win.listen("tauri://blur", startHideCheck);
    // focus 只做「拿到过焦点」标记：轮询必须一直跑（mac 失焦后无新 blur，靠 blur
    // 重启轮询会漏），焦点状态由不变式每轮现查
    const unlistenFocus = win.listen("tauri://focus", () => {
      everFocused = true;
    });

    document.documentElement.addEventListener("mouseleave", onLeave);
    document.addEventListener("mousedown", onDocMouseDown, true);

    Promise.all([unlistenBlur, unlistenFocus]).then(([b, f]) => {
      if (disposed) {
        b();
        f();
      }
    });

    return () => {
      disposed = true;
      unlistenBlur.then((f) => f());
      unlistenFocus.then((f) => f());
      document.documentElement.removeEventListener("mouseleave", onLeave);
      document.removeEventListener("mousedown", onDocMouseDown, true);
      stopHideCheck();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** show 时调用：记录 show 时刻并启动收起轮询（selection / screenshot-result 两条路径） */
  return {
    markShown: () => {
      startRef.current();
      shownAtRef.current = Date.now(); // 收起判定只认这之后的全局点击
    },
  };
}
