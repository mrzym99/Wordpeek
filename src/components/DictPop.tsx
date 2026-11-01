import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import Phonetic from "./Phonetic";
import type { WordInfo } from "../types";

/** 小卡离视口边缘的最小间距 / 离选中词的距离 */
const GAP = 6;
const EDGE = 4;
/** 小卡宽度上限：够放下一行「美 + 英」音标，再宽就压在卡片上了，长释义交给两行截断 */
const MAX_W = 220;
/** 落位时按小卡的最大高度估算：加载中先矮后高，用同一高度判断上下方向，卡片才不会先落下方再翻到上方 */
const PLACE_H = 140;

/**
 * 划词小释义卡（类似百度词典划词）：监听 rootRef 容器内松手，
 * 选中单个单词（拖动选词或双击选词）时在单词下方弹迷你词典卡，
 * 主内容不变；点「详细释义」把词交给宿主（onOpenDetail）做整卡跳转。
 *
 * - resetKey：宿主主内容变化（换词/关窗清空）时收起，避免残留旧锚点
 * - 小卡用 fixed 定位挂视口坐标：不在文档流里，不参与父容器 scrollHeight，
 *   宿主的窗口自适应完全不受它影响（窗口尺寸不会因为划过单词而变）
 * - 下方放不下就翻到单词上方，横向按实测宽度钳进视口（窗口可被拖得比小卡还窄）
 */
export default function DictPop({
  rootRef,
  resetKey,
  onOpenDetail,
}: {
  rootRef: RefObject<HTMLElement | null>;
  resetKey: unknown;
  onOpenDetail: (w: string) => void;
}) {
  // at：选中词在视口里的矩形（小卡落位以它为锚）
  const [pop, setPop] = useState<{
    word: string;
    at: { left: number; top: number; bottom: number };
  } | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [info, setInfo] = useState<WordInfo | null>(null);
  const popRef = useRef<HTMLDivElement | null>(null);
  // 记住已算好的落位：内容变化只重算宽度，位置保持不动
  const posRef = useRef<{ left: number; top: number } | null>(null);
  // 小卡查词独立序号/词缓存，不干扰宿主主卡的请求竞态与去重
  const seqRef = useRef(0);
  const wordRef = useRef("");

  useEffect(() => {
    posRef.current = null;
    setPop(null);
    setPos(null);
  }, [resetKey]);

  // 宽度必须跟着内容重算：加载中只有「查询中…」，比出结果后窄得多，只量一次会把音标和释义切掉。
  // 位置只在弹出时算一次（resize 例外），否则出结果变高又会翻到上方，看起来跳一下。
  // useLayoutEffect 在绘制前跑，不会闪一帧错误位置。
  useLayoutEffect(() => {
    if (!pop) return;
    const place = (reposition: boolean) => {
      const el = popRef.current;
      if (!el) return;
      // 宽度随内容收缩（短词不铺满 MAX_W），且不超过视口
      // 先量内容宽再定宽：窗口能被拖得比小卡还窄，横向钳位才有解
      el.style.width = "max-content";
      const vw = window.innerWidth;
      const w = Math.min(el.offsetWidth, MAX_W, vw - EDGE * 2);
      el.style.width = `${w}px`;
      if (!reposition && posRef.current) return;
      // 高度按上限算：加载中的矮卡和出结果后的高卡落在同一位置，不会跳一下
      const h = Math.max(el.offsetHeight, PLACE_H);
      const vh = window.innerHeight;
      let top = pop.at.bottom + GAP;
      if (top + h > vh - EDGE) top = pop.at.top - h - GAP;
      const next = {
        left: Math.min(Math.max(EDGE, pop.at.left), vw - w - EDGE),
        top: Math.min(Math.max(EDGE, top), vh - h - EDGE),
      };
      posRef.current = next;
      setPos(next);
    };
    place(false);
    const onResize = () => place(true);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [pop, info]);

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
      // 鼠标锚点兼容 textarea（它没有 range rect）
      const vp = rect ?? { left: e.clientX, top: e.clientY, bottom: e.clientY + 4 };
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
      posRef.current = null;
      setPos(null);
      setPop({ word: w, at: { left: vp.left, top: vp.top, bottom: vp.bottom } });
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
    <div
      className="pop-card"
      ref={popRef}
      style={pos ?? { left: pop.at.left, top: pop.at.bottom + GAP }}
    >
      <div className="pop-head">
        <span className="pop-word">{pop.word}</span>
      </div>
      {(info?.usphone || info?.ukphone) && (
        <div className="pop-phon">
          <Phonetic word={pop.word} usphone={info.usphone} ukphone={info.ukphone} />
        </div>
      )}
      {/* 只留第一条释义一行：小卡要够矮，否则会盖住整张卡片 */}
      <div className="pop-senses">
        {info ? info.senses[0] : <span className="pop-loading">查询中…</span>}
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
