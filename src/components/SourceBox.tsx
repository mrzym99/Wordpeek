import type { LegacyRef } from "react";

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
  /** 需要外部聚焦输入框时传入 ref（输入翻译窗口唤起时聚焦） */
  textareaRef?: LegacyRef<HTMLTextAreaElement>;
}) {
  return (
    <div className="source-box">
      {label && <span className="source-box-label">{label}</span>}
      <textarea
        ref={textareaRef}
        className="shot-text"
        value={value}
        rows={rows}
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
