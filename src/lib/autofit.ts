import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/** 卡片上下 padding（16×2）+ 边框（1×2）+ 1px 取整余量 */
const CARD_CHROME = 35;
/** 高度变化小于该值不上报，避免来回抖动 */
const EPSILON = 3;

/**
 * 让窗口高度跟着卡片内容走：内容少时窗口收缩到刚好包住卡片，
 * 不再留下「透明但吃点击」的空白区域。返回内容容器的 ref——
 * 该容器高度即内容自然高度（不受 .card 的 height:100% 影响）。
 */
export function useAutoFitWindow(label: string, chrome: number = CARD_CHROME) {
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!body) return;
    let last = 0;
    // 不用 requestAnimationFrame：窗口显示前是隐藏状态，rAF 会被节流甚至不触发
    const apply = () => {
      const h = Math.ceil(body.offsetHeight) + chrome;
      if (Math.abs(h - last) < EPSILON) return;
      last = h;
      invoke("fit_window", { label, height: h }).catch(() => {});
    };
    const ro = new ResizeObserver(apply);
    ro.observe(body);
    apply(); // 首次测量（ResizeObserver 只保证后续变化）
    return () => ro.disconnect();
  }, [body, label, chrome]);
  return setBody;
}
