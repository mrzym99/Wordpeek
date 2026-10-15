import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { SourceResult, WordInfo } from "../types";
import { useAutoFitWindow } from "../lib/autofit";
import SourceCompare from "./SourceCompare";
import SourceBox from "./SourceBox";

/** 输入翻译窗口（?page=input）：打字/粘贴 → 默认翻译 + 勾选的额外源逐源给出译文，目标语言可智能/手动切换 */
export default function InputPage() {
  const [text, setText] = useState("");
  // 目标语言：智能 = 自动检测（中文→英文，其它→中文），也可手动固定方向
  const [mode, setMode] = useState<"auto" | "en" | "zh">("auto");
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  // 默认源（有道/必应，各司其职）的结果，单独一行排在最前
  const [defaultInfo, setDefaultInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [copiedKey, setCopiedKey] = useState("");
  // .input-layout 上下 padding 14×2 + 1px 余量
  const bodyRef = useAutoFitWindow("input", 29, 260);
  const taRef = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    // 窗口由 Rust 按需创建为隐藏态，内容挂好了再自己显示，避免先白屏一下
    const win = getCurrentWindow();
    win.show().catch(() => {});
    win.setFocus().catch(() => {});
  }, []);

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
    setLoading(true);
    setError("");
    setRows(null);
    setDefaultInfo(null);
    try {
      // 翻译请求由 Rust 端发出（避免 CORS，且 API 密钥不进前端）。
      // 默认源（有道/必应）单独出结果，额外源并发对比；有道/必应不作为独立行出现
      const [d, rs] = await Promise.allSettled([
        invoke<WordInfo>("translate", { text: q, to: target }),
        invoke<SourceResult[]>("translate_all", { text: q, to: target, includeBuiltin: false }),
      ]);
      setDefaultInfo(d.status === "fulfilled" ? d.value : null);
      if (rs.status === "fulfilled") {
        setRows(rs.value);
      } else {
        // 额外源整体失败（如没勾任何源）：有默认结果就不报错，否则给出原因
        if (d.status === "fulfilled") setRows([]);
        else setError(String(rs.reason));
      }
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
        <SourceBox
          value={text}
          onChange={setText}
          rows={4}
          textareaRef={taRef}
          onEnter={run}
          placeholder="输入或粘贴要翻译的文本（Enter 翻译，Shift+Enter 换行，右上角可切换目标语言）"
        />
        <div className="input-actions">
          <span className="lang-toggle" role="group" aria-label="目标语言">
            <button
              className={mode === "auto" ? "active" : ""}
              title="自动检测：中文译为英文，其它译为中文"
              onClick={() => setMode("auto")}
            >
              智能
            </button>
            <button
              className={mode === "en" ? "active" : ""}
              title="固定译为英文"
              onClick={() => setMode("en")}
            >
              → 英文
            </button>
            <button
              className={mode === "zh" ? "active" : ""}
              title="固定译为中文"
              onClick={() => setMode("zh")}
            >
              → 中文
            </button>
          </span>
          <button className="save-btn" onClick={run} disabled={loading || !text.trim()}>
            {loading ? "翻译中…" : "翻译"}
          </button>
        </div>
        {error && <div className="error">{error}</div>}
        {loading && <div className="loading">翻译中…</div>}
        {(() => {
          // 默认源一行排最前（有道/必应看成一个源，各司其职），额外源跟在后面；
          // 主翻译本身就是某个额外源时跳过该行避免重复
          if (!rows) return null;
          const all: SourceResult[] = [
            ...(defaultInfo
              ? [{ source: "default", label: "默认翻译", ok: true, senses: defaultInfo.senses, error: null }]
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
