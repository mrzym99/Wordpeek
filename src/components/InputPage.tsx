import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SourceResult } from "../types";
import { useAutoFitWindow } from "../lib/autofit";
import SourceCompare from "./SourceCompare";

/** 目标语言：目前固定译为英文（issue #2 第 1 条确认），不做选择器 */
const TARGET = "en";

/** 输入翻译窗口（?page=input）：打字/粘贴 → 按勾选的翻译源逐源给出英文译文 */
export default function InputPage() {
  const [text, setText] = useState("");
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [copiedKey, setCopiedKey] = useState("");
  // .input-layout 上下 padding 14×2 + 1px 余量
  const bodyRef = useAutoFitWindow("input", 29);
  const taRef = useRef<HTMLTextAreaElement | null>(null);

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
    setLoading(true);
    setError("");
    setRows(null);
    try {
      // 翻译请求由 Rust 端发出（避免 CORS，且 API 密钥不进前端）；
      // 参与对比的就是设置里勾选的源，顺序也一致
      setRows(await invoke<SourceResult[]>("translate_all", { text: q, to: TARGET }));
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
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
    <div className="input-layout">
      <div className="card-body" ref={bodyRef}>
        <textarea
          ref={taRef}
          className="input-text"
          value={text}
          rows={4}
          spellCheck={false}
          placeholder="输入或粘贴要翻译的文本，Ctrl+Enter 翻译（译为英文）"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              run();
            }
          }}
        />
        <div className="input-actions">
          <span className="field-hint">译为英文；逐个展示设置里勾选的翻译源结果</span>
          <button className="save-btn" onClick={run} disabled={loading || !text.trim()}>
            {loading ? "翻译中…" : "翻译"}
          </button>
        </div>
        {error && <div className="error">{error}</div>}
        {loading && <div className="loading">翻译中…</div>}
        {rows && rows.length === 0 && (
          <div className="error">没有可用的翻译源，请在设置 → 翻译里勾选</div>
        )}
        {rows && rows.length > 0 && (
          <SourceCompare rows={rows} copied={copiedKey} onCopy={copyRow} />
        )}
      </div>
    </div>
  );
}
