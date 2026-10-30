import { useEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WordInfo } from "../types";

const POP_W = 252; // 小卡宽：横向钳位用

/**
 * 划词小释义卡（类似百度词典划词）：监听 rootRef 容器内松手，
 * 选中单个单词（拖动选词或双击选词）时在单词下方弹迷你词典卡，
 * 主内容不变；点「详细释义」把词交给宿主（onOpenDetail）做整卡跳转。
 *
 * - onPlay：发音交给宿主（复用宿主的 Audio 实例）
 * - resetKey：宿主主内容变化（换词/关窗清空）时收起，避免残留旧锚点
 * - 小卡绝对定位在 anchorRef（滚动内容容器）内：溢出容器底部时会计入
 *   scrollHeight，宿主的窗口自适应随之长高包住小卡 —— 不被窗口边框裁切，
 *   也不需要把小卡扣回窗口内盖住单词；收起/展开由 childList 变动自动触发重测
 */
export default function DictPop({
  rootRef,
  anchorRef,
  resetKey,
  onOpenDetail,
  onPlay,
}: {
  rootRef: RefObject<HTMLElement | null>;
  /** 锚定容器（绝对定位坐标系 + 参与窗口自适应测量的滚动容器） */
  anchorRef: RefObject<HTMLElement | null>;
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
      // 单词框（单行 input）、按钮、音标不触发；原文 textarea 要触发（原文框划词也是常见查词手势）
      if (t.closest("input, button, .phonetic")) return;
      // textarea 的选区不进 window.getSelection，得读 selectionStart/End；
      // 也没有选区 range rect，改用鼠标坐标锚定小卡位置
      const ta = t.closest("textarea");
      const sel = window.getSelection();
      const raw = ta ? ta.value.slice(ta.selectionStart, ta.selectionEnd) : (sel?.toString() ?? "");
      const w = raw.trim().replace(/^[\p{P}\p{S}]+|[\p{P}\p{S}]+$/gu, "");
      // 只认单个词：空白（普通单击）或含空白的整句选中 → 收起小卡
      if (!w || w.length > 30 || /\s/.test(w)) {
        setPop(null);
        return;
      }
      const rect = !ta && sel && sel.rangeCount ? sel.getRangeAt(0).getBoundingClientRect() : null;
      // 鼠标锚点兼容 textarea（无 range rect）；坐标换成 anchorRef 容器内相对坐标
      const vp = rect ?? { left: e.clientX, top: e.clientY, bottom: e.clientY + 4 };
      const anchor = anchorRef.current;
      const aRect = anchor?.getBoundingClientRect();
      if (!anchor || !aRect) return;
      const x = Math.min(Math.max(4, vp.left - aRect.left), aRect.width - POP_W);
      const y = vp.bottom - aRect.top + anchor.scrollTop + 6;
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
      // shortcut: 同一词在不同位置重复选中时只变 style 属性，MutationObserver(childList) 不触发重测；
      // 需新位置比旧位置更靠下才会裁到，概率极低；升级方案：暴露 refit 给本组件在 setPop 后调用
    };
    // scroll 不冒泡：捕获阶段挂 document，一个监听覆盖所有滚动容器
    const onScroll = () => setPop(null);
    root.addEventListener("mouseup", onMouseUp);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      root.removeEventListener("mouseup", onMouseUp);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, [rootRef, anchorRef]);

  if (!pop) return null;
  return (
    <div className="pop-card" ref={popRef} style={{ left: pop.x, top: pop.y }}>
      <div className="pop-head">
        <span className="pop-word">{pop.word}</span>
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
