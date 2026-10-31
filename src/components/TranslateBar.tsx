import { ArrowRightIcon } from "./icons";

export type TranslateTo = "auto" | "en" | "zh";

interface Props {
  to: TranslateTo;
  onPick: (to: TranslateTo) => void;
  onRun: () => void;
  /** 后端在跑：按钮文案换「翻译中…」并禁用 */
  busy?: boolean;
  /** 只是没内容可翻（如输入框为空） */
  disabled?: boolean;
}

/** 目标语言切换 + 执行按钮。输入窗口与截图卡片共用，左边切换、右边执行 */
export default function TranslateBar({ to, onPick, onRun, busy, disabled }: Props) {
  return (
    <div className="action-bar">
      <span className="lang-toggle" role="group" aria-label="目标语言">
        <button
          className={to === "auto" ? "active" : ""}
          title="自动检测：中文译为英文，其它译为中文"
          disabled={busy}
          onClick={() => onPick("auto")}
        >
          智能
        </button>
        <button
          className={to === "en" ? "active" : ""}
          title="固定译为英文"
          disabled={busy}
          onClick={() => onPick("en")}
        >
          <ArrowRightIcon /> 英文
        </button>
        <button
          className={to === "zh" ? "active" : ""}
          title="固定译为中文"
          disabled={busy}
          onClick={() => onPick("zh")}
        >
          <ArrowRightIcon /> 中文
        </button>
      </span>
      <button className="text-btn" disabled={busy || disabled} onClick={onRun}>
        {busy ? "翻译中…" : "翻译"}
      </button>
    </div>
  );
}
