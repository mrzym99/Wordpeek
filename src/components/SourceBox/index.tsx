import type { LegacyRef, ReactNode } from "react";
import s from "./index.module.css";

/**
 * 多行原文输入框：输入翻译 / 划词句子 / 截图共用同一个组件与外观。
 * 标签在灰框外，灰框就是 textarea 本身（padding/底色/聚焦线一套样式）。
 * Enter 翻译（onEnter），Shift+Enter 换行；label 不传则不显示标签行
 */
export default function SourceBox({
  value,
  onChange,
  onEnter,
  onBlur,
  rows = 3,
  placeholder,
  label,
  action,
  textareaRef,
}: {
  value: string;
  onChange: (v: string) => void;
  /** Enter（无 Shift）触发；不传则 Enter 也换行 */
  onEnter?: () => void;
  onBlur?: () => void;
  rows?: number;
  placeholder?: string;
  /** 框头标签（如「原文」），不传则不显示 */
  label?: string;
  /** 标签行右侧的附加按钮（如「对比各源」）：与标签同一行，不另占高度 */
  action?: ReactNode;
  /** 需要外部聚焦输入框时传入 ref（输入翻译窗口唤起时聚焦） */
  textareaRef?: LegacyRef<HTMLTextAreaElement>;
}) {
  return (
    <div className={s.box}>
      {(label || action) && (
        <div className={s.labelRow}>
          {label && <span className={s.label}>{label}</span>}
          {action}
        </div>
      )}
      <textarea
        ref={textareaRef}
        className={s.text}
        value={value}
        rows={rows}
        // rows 在新 WebView2 里会被 field-sizing: content 盖掉（空框只有 min-height 56px），
        // 这里按行数补一个下限，新旧内核都拿到同样高的框
        style={{ minHeight: `calc(${rows} * 1.65em + 16px)` }}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (onEnter && e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onEnter();
          }
        }}
        onBlur={onBlur}
      />
    </div>
  );
}
