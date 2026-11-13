import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { AppConfig, SourceResult, WordInfo } from "../../types";
import { useAutoFitWindow } from "../../hooks/useAutoFitWindow";
import SourceCompare from "../SourceCompare";
import type { CompareRow } from "../SourceCompare";
import SourceBox from "../SourceBox";
import TranslateBar from "../TranslateBar";
import Phonetic from "../Phonetic";
import s from "./index.module.css";

/** 输入翻译窗口（?page=input）：打字/粘贴 → 默认翻译；勾选源按「对比各源」按需逐个补 */
export default function InputPage() {
  const [text, setText] = useState("");
  const [mode, setMode] = useState<"auto" | "en" | "zh">("auto");
  // 默认源（有道/必应）的结果，单独一行排在最前
  const [defaultInfo, setDefaultInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [copiedKey, setCopiedKey] = useState("");
  // 对比各源：懒加载，展开才逐个源请求（不再一次性把所有源都翻一遍）
  const [compareOpen, setCompareOpen] = useState(false);
  const [extrasCount, setExtrasCount] = useState(-1); // -1 = 未查到，避免按钮闪现
  const [compareRows, setCompareRows] = useState<SourceResult[] | null>(null);
  const [compareText, setCompareText] = useState(""); // compareRows 对应的原文，没变就复用
  const [compareLoading, setCompareLoading] = useState(false);
  const [compareError, setCompareError] = useState("");
  // chrome=33（.input-layout 上下 padding 16×2 + 1px 余量，与 .card 的 16/18 一致）；
  // min 只防塌，空态窗口贴着内容走；macOS 比 Windows 多一条 28px 自绘头部（Overlay 标题栏）
  const headHeight = document.documentElement.dataset.os === "mac" ? 28 : 0;
  const { setBody, refit } = useAutoFitWindow("input", 33 + headHeight, 180);
  const taRef = useRef<HTMLTextAreaElement | null>(null);
  // 请求序号：慢的旧结果返回时丢弃，避免覆盖新查询（同 Popup.tsx 的 seqRef）
  const seqRef = useRef(0);
  const compareSeq = useRef(0);

  const to = mode === "auto" ? "auto" : mode;

  useEffect(() => {
    // 窗口由 Rust 按需创建为隐藏态，内容挂好了再自己显示，避免先白屏一下
    const win = getCurrentWindow();
    win.show().catch(() => {});
    win.setFocus().catch(() => {});
    refit(); // 首次按当前内容纠正窗口高度
  }, [refit]);

  useEffect(() => {
    // 窗口每次被唤到前台都聚焦输入框，省一次点击
    const focus = () => taRef.current?.focus();
    focus();
    window.addEventListener("focus", focus);
    return () => window.removeEventListener("focus", focus);
  }, []);

  // 勾了额外源才显示「对比各源」（persist 已保证只含密钥就绪的源）；窗口是常驻隐藏的，
  // 设置里改了勾选要能立刻反映，不能等重建
  const compareState = useRef({ open: false, text: "", to: "auto" });
  compareState.current = { open: compareOpen, text, to };
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const refreshExtras = () => {
      invoke<AppConfig>("get_config")
        .then((cfg) => setExtrasCount((cfg.sources ?? []).length))
        .catch(() => {});
    };
    listen("config-changed", () => {
      refreshExtras();
      const { open, text: t, to: target } = compareState.current;
      if (open) runCompare(t.trim(), target);
      else setCompareRows(null);
    }).then((f) => {
      if (disposed) f(); // 晚到的订阅：立刻退订，不留泄漏
      else unlisten = f;
    });
    refreshExtras();
    return () => {
      disposed = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runCompare = async (q: string, target: string) => {
    if (!q) return;
    const seq = ++compareSeq.current;
    setCompareText(q);
    setCompareLoading(true);
    setCompareRows(null);
    setCompareError("");
    try {
      const rs = await invoke<SourceResult[]>("translate_all", {
        text: q,
        to: target,
        includeBuiltin: false,
      });
      if (compareSeq.current !== seq) return;
      setCompareRows(rs);
    } catch (err) {
      if (compareSeq.current !== seq) return;
      setCompareError(String(err));
    } finally {
      if (compareSeq.current === seq) setCompareLoading(false);
    }
  };

  // 只在第一次（或原文变了）时请求，之后切换是纯显示/隐藏，结果不销毁
  const toggleCompare = () => {
    if (compareOpen) {
      setCompareOpen(false);
      return;
    }
    setCompareOpen(true);
    const q = text.trim();
    if (!compareRows || compareText !== q) runCompare(q, to);
  };

  const run = async () => {
    const q = text.trim();
    if (!q) return;
    const seq = ++seqRef.current;
    setLoading(true);
    setError("");
    try {
      // 翻译请求由 Rust 端发出（避免 CORS，API 密钥不进前端）；有道/必应不作为独立行出现
      const info = await invoke<WordInfo>("translate", { text: q, to });
      if (seqRef.current !== seq) return;
      setDefaultInfo(info);
      // 对比区开着就顺手按新原文重拉；关着不动，省掉一轮没人看的请求
      if (compareOpen) runCompare(q, to);
    } catch (err) {
      if (seqRef.current !== seq) return;
      // 失败清掉旧内容，别让上一段文本的译文配着报错一起显示
      setDefaultInfo(null);
      setCompareRows(null);
      setError(String(err));
    } finally {
      if (seqRef.current === seq) setLoading(false);
    }
  };

  const copyRow = (source: string, value: string) => {
    invoke("copy_text", { text: value })
      .then(() => {
        setCopiedKey(source);
        setTimeout(() => setCopiedKey(""), 1500);
      })
      .catch((e) => setError(String(e)));
  };

  return (
    <div className={s.layout}>
      {/* macOS 把页面画到标题栏下面了，这条是自绘的头部（拖拽 + 底色），Windows 下隐藏 */}
      <div className="win-head" data-tauri-drag-region />
      <div className="card-body" ref={setBody}>
        <SourceBox
          value={text}
          onChange={setText}
          rows={6}
          textareaRef={taRef}
          onEnter={run}
          placeholder="输入或粘贴要翻译的文本（Enter 翻译，Shift+Enter 换行）"
        />
        <TranslateBar
          to={mode}
          onPick={setMode}
          onRun={run}
          busy={loading}
          disabled={!text.trim()}
          // 对比各源从框头挪到底部按钮排，紧跟翻译按钮（2026-11-13 用户定案：去掉「原文」标签）
          extra={
            text.trim() && extrasCount > 0 ? (
              <button
                className="text-btn"
                disabled={loading}
                title={
                  loading
                    ? "等默认翻译完成后再对比"
                    : compareOpen
                      ? "收起各源译文对比"
                      : "在下方追加各已勾选源的译文"
                }
                onClick={toggleCompare}
              >
                {compareOpen ? "收起对比" : "对比各源"}
              </button>
            ) : undefined
          }
        />
        {error && <div className="error">{error}</div>}
        {(() => {
          // 默认源一行排最前，额外源跟在后面；主翻译本身就是某个额外源时跳过该行避免重复
          const extras = (compareOpen ? compareRows ?? [] : []).filter(
            (r) => r.source !== defaultInfo?.source
          );
          const all: CompareRow[] = [
            ...(defaultInfo
              ? [{
                  source: "default",
                  label: "默认翻译",
                  ok: true,
                  senses: defaultInfo.senses,
                  error: null,
                  // 查到音标＝有道单词条目：在默认翻译行内补上与划词同款的「单词 + 音标」行，句子无音标不显示
                  wordLine:
                    defaultInfo.usphone || defaultInfo.ukphone ? (
                      <div className="word-line">
                        <span className="word-big">{defaultInfo.word}</span>
                        <Phonetic
                          word={defaultInfo.word}
                          usphone={defaultInfo.usphone}
                          ukphone={defaultInfo.ukphone}
                        />
                      </div>
                    ) : undefined,
                }]
              : []),
            ...extras,
          ];
          if (all.length === 0) return null;
          return <SourceCompare rows={all} copied={copiedKey} onCopy={copyRow} />;
        })()}
        {compareOpen && compareLoading && <div className="loading">对比中…</div>}
        {compareOpen && compareError && <div className="error">{compareError}</div>}
        {compareOpen && compareRows?.length === 0 && !compareLoading && (
          <div className="error">没有可对比的额外源，可在设置 → 翻译里勾选</div>
        )}

        {/* 翻译中遮罩：旧内容留在下面，结果回来才替换，卡片只跳一次 */}
        {loading && (
          <div className={s.busy} role="status">
            翻译中…
          </div>
        )}
      </div>
    </div>
  );
}