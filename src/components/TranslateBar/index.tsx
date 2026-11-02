import { ArrowRightIcon } from "../icons";
import LangToggle, { type LangOption } from "../LangToggle";
import s from "./index.module.css";

export type TranslateTo = "auto" | "en" | "zh";

const OPTIONS: LangOption[] = [
  { id: "auto", label: "智能", title: "自动检测：中文译为英文，其它译为中文" },
  { id: "en", label: "英文", title: "固定译为英文", icon: <ArrowRightIcon /> },
  { id: "zh", label: "中文", title: "固定译为中文", icon: <ArrowRightIcon /> },
];

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
    <div className={s.bar}>
      <LangToggle
        options={OPTIONS}
        value={to}
        onChange={onPick}
        disabled={busy}
        ariaLabel="目标语言"
      />
      <button className="text-btn" disabled={busy || disabled} onClick={onRun}>
        {busy ? "翻译中…" : "翻译"}
      </button>
    </div>
  );
}
