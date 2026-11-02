import type { ReactNode } from "react";
import s from "./index.module.css";
import { CheckIcon, CopyIcon } from "../icons";

/**
 * 通用内容框：浅灰圆角框 + 框内右上角复制按钮（对齐多源对比行的样式）。
 * Popup 译文/截图原文与译文、输入翻译原文共用，复制体验全应用一致
 */
export default function ContentBox({
  label,
  copied = false,
  onCopy,
  className = "",
  children,
}: {
  /** 框头标签（如「原文」「译文」），不传则只显示按钮 */
  label?: string;
  copied?: boolean;
  onCopy?: () => void;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={s.box + " " + className}>
      {(label || onCopy) && (
        <div className={s.head}>
          {label && <span className={s.label}>{label}</span>}
          <span className={s.actions}>
            {onCopy && (
              <button
                className={s.copy + " icon-btn" + (copied ? " ok" : "")}
                title="复制"
                onClick={onCopy}
              >
                {copied ? <CheckIcon /> : <CopyIcon />}
              </button>
            )}
          </span>
        </div>
      )}
      {children}
    </div>
  );
}
