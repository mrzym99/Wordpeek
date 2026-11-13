import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { WordInfo } from "../../types";
import { useAutoFitWindow } from "../../hooks/useAutoFitWindow";
import SourceCompare from "../SourceCompare";
import type { CompareRow } from "../SourceCompare";
import SourceBox from "../SourceBox";
import LoadingOverlay from "../LoadingOverlay";
import TranslateBar from "../TranslateBar";
import Phonetic from "../Phonetic";
import { sourceLabel } from "../../lib/sourceLabels";
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
  // chrome=33（.input-layout 上下 padding 16×2 + 1px 余量，与 .card 的 16/18 一致）；
  // min 只防塌，空态窗口贴着内容走；macOS 比 Windows 多一条 28px 自绘头部（Overlay 标题栏）
  const headHeight = document.documentElement.dataset.os === "mac" ? 28 : 0;
  const { setBody, refit } = useAutoFitWindow("input", 33 + headHeight, 180);
  const taRef = useRef<HTMLTextAreaElement | null>(null);
  // 请求序号：慢的旧结果返回时丢弃，避免覆盖新查询（同 Popup.tsx 的 seqRef）
  const seqRef = useRef(0);

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
    } catch (err) {
      if (seqRef.current !== seq) return;
      // 失败清掉旧内容，别让上一段文本的译文配着报错一起显示
      setDefaultInfo(null);
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
        />
        {error && <div className="error">{error}</div>}
        {(() => {
          // 默认翻译一行（单词带音标行，句子灰框）：沿用 SourceCompare 行样式
          const rows: CompareRow[] = [
            ...(defaultInfo
              ? [{
                  source: defaultInfo.source,
                  label: sourceLabel(defaultInfo.source, "默认翻译"),
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
          ];
          if (rows.length === 0) return null;
          return <SourceCompare rows={rows} copied={copiedKey} onCopy={copyRow} />;
        })()}

        {/* 翻译中遮罩：旧内容留在下面，结果回来才替换，卡片只跳一次 */}
        <LoadingOverlay show={loading} />
      </div>
    </div>
  );
}