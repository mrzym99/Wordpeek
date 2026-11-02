import type { ReactNode } from "react";
import s from "./index.module.css";

export interface LangOption {
  id: string;
  label: string;
  /** 悬停提示 */
  title?: string;
  /** 文案前的图标（如翻译方向的 →） */
  icon?: ReactNode;
}

/** 分段切换控件（胶囊形多段按钮）：TranslateBar 的目标语言、设置页的主题选择共用。
 *  泛型让宿主的取值类型（TranslateTo / Theme）原样穿过，回调不用再断言 */
export default function LangToggle<T extends string>({
  options,
  value,
  onChange,
  disabled,
  ariaLabel,
  className = "",
}: {
  options: LangOption[];
  value: T;
  onChange: (id: T) => void;
  /** 全部禁用（如翻译进行中） */
  disabled?: boolean;
  ariaLabel?: string;
  /** 宿主额外样式（如设置页里贴到 .field 下方） */
  className?: string;
}) {
  return (
    <span className={s.toggle + " " + className} role="group" aria-label={ariaLabel}>
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          className={value === o.id ? "active" : ""}
          title={o.title}
          disabled={disabled}
          onClick={() => onChange(o.id as T)}
        >
          {o.icon}
          {o.label}
        </button>
      ))}
    </span>
  );
}
