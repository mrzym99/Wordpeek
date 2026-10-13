import { invoke } from "@tauri-apps/api/core";
import type { ShotResult } from "../types";
import SenseList from "./SenseList";
import { CheckIcon, CopyIcon, RefreshIcon } from "./icons";
import { useAutoFitWindow } from "../lib/autofit";

interface ShotCardProps {
  shotResult: ShotResult;
  /** 原文可编辑副本（用户修掉 OCR 错误换行） */
  draft: string;
  onDraftChange: (v: string) => void;
  /** 复制反馈："text" | "translation" | "" */
  copied: string;
  onCopy: (label: "text" | "translation", text: string) => void;
  /** 用编辑后的文本重新翻译 */
  onRetranslate: (text: string) => void;
}

/** 截图翻译结果卡片：标题行 + 可编辑原文 + 图标按钮 + 译文 */
export default function ShotCard({ shotResult, draft, onDraftChange, copied, onCopy, onRetranslate }: ShotCardProps) {
  // 与划词卡片共用 main 窗口，同样按内容高度收缩
  const bodyRef = useAutoFitWindow("main");
  // 重新翻译进行中：info/error 均空且原文存在
  const loading = !!shotResult.text && !shotResult.info && !shotResult.error;

  return (
    <div className="card">
      <div className="drag-strip" data-tauri-drag-region />
      <div className="card-body" ref={bodyRef}>
      <div className="word-head">
        <div className="word">截图识别结果</div>
        <button className="icon-btn gear" title="设置" onClick={() => invoke("open_settings")}>
          ⚙
        </button>
      </div>
      {shotResult.text && (
        <>
          <textarea
            className="shot-text"
            value={draft}
            onChange={(e) => onDraftChange(e.target.value)}
            rows={3}
            spellCheck={false}
          />
          <div className="shot-actions">
            <button
              className={"icon-btn" + (copied === "text" ? " ok" : "")}
              title="复制原文"
              onClick={() => onCopy("text", draft)}
            >
              {copied === "text" ? <CheckIcon /> : <CopyIcon />}
            </button>
            <button
              className={"icon-btn" + (loading ? " spinning" : "")}
              title="重新翻译"
              onClick={() => onRetranslate(draft)}
            >
              <RefreshIcon />
            </button>
          </div>
        </>
      )}
      {shotResult.error && <div className="error">{shotResult.error}</div>}
      {loading && <div className="shot-loading">翻译中…</div>}
      {shotResult.info && (
        <>
          <SenseList senses={shotResult.info.senses} className="shot-senses" />
          <button
            className={"icon-btn" + (copied === "translation" ? " ok" : "")}
            title="复制译文"
            onClick={() => onCopy("translation", shotResult.info!.senses.join("\n"))}
          >
            {copied === "translation" ? <CheckIcon /> : <CopyIcon />}
          </button>
        </>
      )}
      </div>
    </div>
  );
}
