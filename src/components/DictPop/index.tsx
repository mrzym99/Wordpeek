import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import Phonetic from "../Phonetic";
import type { WordInfo } from "../../types";
import s from "./index.module.css";

/** 小卡离视口边缘的最小间距 / 离选中词的距离 */
const GAP = 6;
const EDGE = 4;
/** 小卡宽度上限：够放下一行「美 + 英」音标，再宽就压在卡片上了，长释义交给两行截断 */
const MAX_W = 220;

/**
 * 划词小释义卡：rootRef 容器内松手选中单个单词（拖选/双击）时在词下方弹迷你词典卡，主内容不变。
 * - resetKey：宿主主内容变化（换词/关窗清空）时收起，避免残留旧锚点
 * - fixed 定位挂视口坐标，不在文档流里，不参与父容器 scrollHeight（窗口自适应不受影响）
 * - 下方放不下就翻到词上方，横向按实测宽度钳进视口（窗口可被拖得比小卡还窄）
 */
export default function DictPop({
  rootRef,
  resetKey,
}: {
  rootRef: RefObject<HTMLElement | null>;
  resetKey: unknown;
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
    seqRef.current++; // 在途请求作废：宿主内容已换，晚到的结果不许再弹
    posRef.current = null;
    setPop(null);
    setPos(null);
  }, [resetKey]);

  // 宽度跟内容重算、位置弹出时算一次（resize 例外重算）：落位用的就是最终尺寸，
  // 一次到位不瞬移；useLayoutEffect 在绘制前跑，不闪一帧错误位置
  useLayoutEffect(() => {
    if (!pop) return;
    const place = (reposition: boolean) => {
      const el = popRef.current;
      if (!el) return;
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      // 先量内容宽再定宽：窗口能被拖得比小卡还窄，横向钳位才有解；短词不铺满 MAX_W
      el.style.width = "max-content";
      const w = Math.min(el.offsetWidth, MAX_W, vw - EDGE * 2);
      el.style.width = `${w}px`;
      if (!reposition && posRef.current) {
        // 出结果后卡片宽高都会变：不翻面、不大跳，但要补碰撞复查——
        // 横向：右侧锚定的卡变宽会长出窗口被裁；纵向：重新贴回锚点词，消掉大空隙
        const h = el.offsetHeight;
        const above = posRef.current.top < pop.at.top;
        const anchored = above ? pop.at.top - h - GAP : pop.at.bottom + GAP;
        const top = Math.min(Math.max(EDGE, anchored), Math.max(EDGE, vh - h - EDGE));
        const left = Math.min(Math.max(EDGE, posRef.current.left), vw - w - EDGE);
        const next = { left, top };
        if (next.left !== posRef.current.left || next.top !== posRef.current.top) {
          posRef.current = next;
          setPos(next);
        }
        return;
      }
      // 查到结果才弹卡：量到的就是最终高度，直接定上下方向和落点，落位一次到位
      const h = el.offsetHeight;
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
      // 单词框（单行 input）、按钮、音标不触发；原文 textarea 要触发（划词也是常见查词手势）
      if (t.closest("input, button, .phonetic")) return;
      // textarea 的选区不进 window.getSelection，得读 selectionStart/End；
      // 也没有选区 range rect，改用鼠标坐标锚定小卡位置
      const ta = t.closest("textarea");
      const sel = window.getSelection();
      const raw = ta ? ta.value.slice(ta.selectionStart, ta.selectionEnd) : (sel?.toString() ?? "");
      const w = raw.trim().replace(/^[\p{P}\p{S}]+|[\p{P}\p{S}]+$/gu, "");
      // 只认单个词：空白（普通单击）或含空白的整句选中 → 收起小卡
      if (!w || w.length > 30 || /\s/.test(w)) {
        seqRef.current++; // 在途请求作废：用户已收卡，晚到的结果不许再弹
        setPop(null);
        return;
      }
      const rect = !ta && sel && sel.rangeCount ? sel.getRangeAt(0).getBoundingClientRect() : null;
      const vp = rect ?? { left: e.clientX, top: e.clientY, bottom: e.clientY + 4 };
      posRef.current = null;
      setPos(null);
      // 查到结果才弹卡（无加载占位，落位时量到的就是最终宽高，不瞬移）；查不到就保持收起
      if (w !== wordRef.current || !popRef.current) {
        wordRef.current = w;
        const seq = ++seqRef.current;
        setInfo(null);
        setPop(null); // 换词重查先收旧卡：不挂着旧词的音标/释义等新结果
        invoke<WordInfo>("translate", { text: w })
          .then((res) => {
            if (seqRef.current !== seq) return;
            setInfo(res);
            setPop({ word: w, at: { left: vp.left, top: vp.top, bottom: vp.bottom } });
          })
          .catch(() => {
            // 非词文本、网络错都走这里：静默收起
            if (seqRef.current === seq) setPop(null);
          });
      } else {
        // 同词且卡开着：不重查，只按新选区位置重新锚定
        setPop({ word: w, at: { left: vp.left, top: vp.top, bottom: vp.bottom } });
      }
    };
    // scroll 不冒泡：捕获阶段挂 document，一个监听覆盖所有滚动容器
    const onScroll = () => {
      seqRef.current++; // 在途请求作废：卡已随滚动收起，晚到的结果不许再弹
      setPop(null);
    };
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
      className={s.card}
      ref={popRef}
      style={pos ?? { left: pop.at.left, top: pop.at.bottom + GAP }}
    >
      <div className={s.head}>
        <span className={s.word}>{pop.word}</span>
      </div>
      {(info?.usphone || info?.ukphone) && (
        <div className={s.phon}>
          <Phonetic word={pop.word} usphone={info.usphone} ukphone={info.ukphone} />
        </div>
      )}
      {/* 只留第一条释义：小卡要够矮，否则会盖住整张卡片 */}
      <div className={s.senses}>{info ? info.senses[0] : null}</div>
    </div>
  );
}
