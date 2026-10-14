import type { ReactNode } from "react";
import { CheckIcon, CopyIcon } from "./icons";

/**
 * 通用内容框：浅灰圆角框 + 框内右上角复制按钮（对齐多源对比行的样式）。
 * Popup 译文/截图原文与译文、输入翻译原文共用，复制体验全应用一致
 */
export default function ContentBox({
  label,
  copied = false,
  onCopy,
  actions,
  className = "",
  children,
}: {
  /** 框头标签（如「原文」「译文」），不传则只显示按钮 */
  label?: string;
  copied?: boolean;
  onCopy?: () => void;
  /** 框头右侧的其它按钮（如重新翻译），排在复制按钮前 */
  actions?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={"content-box " + className}>
      {(label || onCopy || actions) && (
        <div className="content-box-head">
          {label && <span className="content-box-label">{label}</span>}
          <span className="content-box-actions">
            {actions}
            {onCopy && (
              <button
                className={"icon-btn" + (copied ? " ok" : "")}
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
