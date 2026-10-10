import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** 全屏遮罩框选页：遮罩窗口透明，框选直接对着实时桌面（选区外变暗）；
 *  快照由后端在触发瞬间抓取，仅供松手后裁剪，前端不显示大图 */
export default function ScreenshotPage() {
  // 选区：起点 + 当前点（CSS 像素，窗口内坐标）
  const [sel, setSel] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  const dragging = useRef(false);

  // screenshot-start 仅作轻量信号：重置上一轮选区（遮罩内容常驻就绪，无需加载）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen("screenshot-start", () => {
      setSel(null);
      dragging.current = false;
    }).then((fn) => (unlisten = fn));
    return () => unlisten?.();
  }, []);

  // ESC / 窗口失焦 → 取消本次截图会话（窗口隐藏后事件不会再触发）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") invoke("screenshot_cancel");
    };
    const onBlur = () => invoke("screenshot_cancel");
    document.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onBlur);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  const dpr = window.devicePixelRatio || 1;

  const onMouseDown = (e: React.MouseEvent) => {
    dragging.current = true;
    setSel({ x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY });
  };
  const onMouseMove = (e: React.MouseEvent) => {
    if (!dragging.current) return;
    setSel((s) => (s ? { ...s, x1: e.clientX, y1: e.clientY } : s));
  };
  const onMouseUp = () => {
    if (!dragging.current || !sel) return;
    dragging.current = false;
    const s = sel;
    setSel(null);
    // 物理像素 = CSS 像素 × devicePixelRatio，并归一化到左上原点
    const x = Math.round(Math.min(s.x0, s.x1) * dpr);
    const y = Math.round(Math.min(s.y0, s.y1) * dpr);
    const w = Math.round(Math.abs(s.x1 - s.x0) * dpr);
    const h = Math.round(Math.abs(s.y1 - s.y0) * dpr);
    // 太小的选区视为误触，直接取消；否则交给后端裁剪（后端会隐藏本窗口）
    if (w < 8 || h < 8) {
      invoke("screenshot_cancel");
      return;
    }
    invoke("screenshot_finish", { x, y, w, h });
  };

  return (
    <div className="shot-mask" onMouseDown={onMouseDown} onMouseMove={onMouseMove} onMouseUp={onMouseUp}>
      {sel ? (
        <div
          className="shot-selection"
          style={{
            left: Math.min(sel.x0, sel.x1),
            top: Math.min(sel.y0, sel.y1),
            width: Math.abs(sel.x1 - sel.x0),
            height: Math.abs(sel.y1 - sel.y0),
          }}
        />
      ) : (
        // 未开始拖拽时整屏变暗，提示已进入截图模式（选区内亮、外暗由 box-shadow 实现）
        <div className="shot-selection shot-idle" />
      )}
    </div>
  );
}
