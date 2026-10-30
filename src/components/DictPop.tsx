import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WordInfo } from "../types";

const POP_W = 262; // 小卡宽 + 边距：横向钳位用
const POP_H = 220; // 小卡估算高：下方放不下时翻到选区上方

/**
 * 划词小释义卡（类似百度词典划词）：监听 rootRef 容器内松手，
 * 选中单个单词（拖动选词或双击选词）时在选区旁浮动迷你词典卡，
 * 主内容不变；点「详细释义」把词交给宿主（onOpenDetail）做整卡跳转。
 *
 * - onPlay：发音交给宿主（复用宿主的 Audio 实例）
 * - resetKey：宿主主内容变化（换词/关窗清空）时收起，避免残留旧锚点
 * - 小卡用 position:fixed 视口坐标，不参与宿主内容高度测量（窗口自适应不受影响）；
 *   下方放不下自动翻到选区上方，横向钳位不出窗；任意滚动即收起
 */
export default function DictPop({
  rootRef,
  resetKey,
  onOpenDetail,
  onPlay,
}: {
  rootRef: RefObject<HTMLElement | null>;
  resetKey: unknown;
  onOpenDetail: (w: string) => void;
  onPlay: (w: string, type: 1 | 2) => void;
}) {
  const [pop, setPop] = useState<{ word: string; x: number; y: number } | null>(null);
  const [info, setInfo] = useState<WordInfo | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  // 小卡查词独立序号/词缓存，不干扰宿主主卡的请求竞态与去重
  const seqRef = useRef(0);
  const wordRef = useRef("");

  useEffect(() => {
    setPop(null);
  }, [resetKey]);

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const onMouseUp = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (popRef.current?.contains(t)) return; // 小卡内部的选择/点击不重置小卡
      if (t.closest("input, textarea, button, .phonetic")) return;
      const sel = window.getSelection();
      const w = (sel?.toString() ?? "").trim().replace(/^[\p{P}\p{S}]+|[\p{P}\p{S}]+$/gu, "");
      // 只认单个词：空白（普通单击）或含空白的整句选中 → 收起小卡
      if (!w || w.length > 30 || /\s/.test(w)) {
        setPop(null);
        return;
      }
      const rect = sel && sel.rangeCount ? sel.getRangeAt(0).getBoundingClientRect() : null;
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const x = Math.min(Math.max(8, rect?.left ?? 8), vw - POP_W - 8);
      let y = (rect?.bottom ?? vh) + 6;
      if (y + POP_H > vh - 8) y = Math.max(8, (rect?.top ?? vh) - POP_H - 6);
      if (w !== wordRef.current) {
        wordRef.current = w;
        const seq = ++seqRef.current;
        setInfo(null);
        invoke<WordInfo>("translate", { text: w })
          .then((res) => {
            if (seqRef.current === seq) setInfo(res);
          })
          .catch(() => {
            // 查不到就静默收起（非词文本、网络错都走这里）
            if (seqRef.current === seq) setPop(null);
          });
      }
      setPop({ word: w, x, y });
    };
    // scroll 不冒泡：捕获阶段挂 document，一个监听覆盖所有滚动容器
    const onScroll = () => setPop(null);
    root.addEventListener("mouseup", onMouseUp);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      root.removeEventListener("mouseup", onMouseUp);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [rootRef]);

  if (!pop) return null;
  return (
    <div className="pop-card" ref={popRef} style={{ left: pop.x, top: pop.y }}>
      <div className="pop-head">
        <span className="pop-word">{pop.word}</span>
        <button className="pop-close" title="关闭" onClick={() => setPop(null)}>
          ×
        </button>
      </div>
      {info?.usphone && (
        <span className="phonetic" title="播放美音" onClick={() => onPlay(pop.word, 2)}>
          🔊 {info.usphone}
        </span>
      )}
      <div className="pop-senses">
        {info ? (
          info.senses.slice(0, 3).map((s, i) => <div key={i}>{s}</div>)
        ) : (
          <div className="pop-loading">查询中…</div>
        )}
      </div>
      <button
        className="text-btn pop-more"
        onClick={() => {
          setPop(null);
          onOpenDetail(pop.word);
        }}
      >
        详细释义
      </button>
    </div>
  );
}
