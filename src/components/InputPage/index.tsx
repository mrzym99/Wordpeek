import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { SourceResult, WordInfo } from "../../types";
import { useAutoFitWindow } from "../../hooks/useAutoFitWindow";
import SourceCompare from "../SourceCompare";
import type { CompareRow } from "../SourceCompare";
import SourceBox from "../SourceBox";
import TranslateBar from "../TranslateBar";
import Phonetic from "../Phonetic";
import s from "./index.module.css";

/** 输入翻译窗口（?page=input）：打字/粘贴 → 默认翻译 + 勾选的额外源逐源给出译文，目标语言可智能/手动切换 */
export default function InputPage() {
  const [text, setText] = useState("");
  const [mode, setMode] = useState<"auto" | "en" | "zh">("auto");
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  // 默认源（有道/必应）的结果，单独一行排在最前
  const [defaultInfo, setDefaultInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [copiedKey, setCopiedKey] = useState("");
  // chrome=29（.input-layout 上下 padding 14×2 + 1px 余量）；min 只防塌，空态窗口贴着内容走；
  // macOS 比 Windows 多一条 28px 自绘头部（Overlay 标题栏），窗口高度要一起加
  const headHeight = document.documentElement.dataset.os === "mac" ? 28 : 0;
  const { setBody, refit } = useAutoFitWindow("input", 29 + headHeight, 180);
  const taRef = useRef<HTMLTextAreaElement | null>(null);
  // 请求序号：慢的旧结果返回时丢弃，避免覆盖新查询（同 Popup.tsx 的 seqRef）
  const seqRef = useRef(0);

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
    // 智能 = 交给后端检测（含 CJK → 译英文，否则 → 译中文）
    const target = mode === "auto" ? "auto" : mode;
    const seq = ++seqRef.current;
    setLoading(true);
    setError("");
    setRows(null);
    setDefaultInfo(null);
    try {
      // 翻译请求由 Rust 端发出（避免 CORS，API 密钥不进前端）；有道/必应不作为独立行出现
      const [d, rs] = await Promise.allSettled([
        invoke<WordInfo>("translate", { text: q, to: target }),
        invoke<SourceResult[]>("translate_all", { text: q, to: target, includeBuiltin: false }),
      ]);
      // 已发起新查询：旧结果全部丢弃（loading 状态归新一轮管）
      if (seqRef.current !== seq) return;
      if (d.status === "fulfilled") {
        setDefaultInfo(d.value);
      } else {
        // 默认源失败也要把原因露出来，否则界面上只剩「没有可用的翻译源」，看不出是谁挂了
        setError(String(d.reason));
      }
      if (rs.status === "fulfilled") {
        setRows(rs.value);
      } else if (d.status === "fulfilled") {
        // 额外源整体失败（如没勾任何源）但有默认结果：不报错
        setRows([]);
      }
    } catch (err) {
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
          placeholder="输入或粘贴要翻译的文本（Enter 翻译，Shift+Enter 换行，右上角可切换目标语言）"
        />
        <TranslateBar
          to={mode}
          onPick={setMode}
          onRun={run}
          busy={loading}
          disabled={!text.trim()}
        />
        {error && <div className="error">{error}</div>}
        {loading && <div className="loading">翻译中…</div>}
        {(() => {
          // 默认源一行排最前，额外源跟在后面；主翻译本身就是某个额外源时跳过该行避免重复
          if (!rows) return null;
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
            ...rows.filter((r) => r.source !== defaultInfo?.source),
          ];
          if (all.length === 0 && !loading) {
            return <div className="error">没有可用的翻译源，请在设置 → 翻译里勾选</div>;
          }
          return <SourceCompare rows={all} copied={copiedKey} onCopy={copyRow} />;
        })()}
      </div>
    </div>
  );
}
