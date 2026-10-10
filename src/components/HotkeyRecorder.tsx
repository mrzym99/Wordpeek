import { useEffect, useState } from "react";

/** 快捷键录制输入框：点击后按下组合键录制，Esc 取消。
 *  用 e.code（物理按键）而非 e.key，避免中文输入法把按键吞成 "Process" */
export default function HotkeyRecorder({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setRecording(false);
        setError("");
        return;
      }
      // 修饰键单独按下时先等待完整组合
      if (/^(Control|Alt|Shift|Meta)(Left|Right)?$/.test(e.code)) return;

      const parts: string[] = [];
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");
      if (e.metaKey) parts.push("Super");

      let keyName: string | null = null;
      if (/^Key[A-Z]$/.test(e.code)) keyName = e.code.slice(3);
      else if (/^Digit[0-9]$/.test(e.code)) keyName = e.code.slice(5);
      else if (/^F([1-9]|1[0-2])$/.test(e.code)) keyName = e.code;

      if (!keyName) {
        setError("不支持的按键，可用字母、数字、F1-F12");
        return;
      }
      if (parts.length === 0) {
        setError("需要至少一个修饰键（Ctrl / Alt / Shift）");
        return;
      }
      setError("");
      onChange([...parts, keyName].join("+"));
      setRecording(false);
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [recording]);

  return (
    <>
      <input
        className={recording ? "recording" : ""}
        readOnly
        value={recording ? "" : value}
        placeholder={recording ? "按下新的组合键（Esc 取消）" : ""}
        onClick={() => setRecording(true)}
      />
      {error && <div className="field-error">{error}</div>}
    </>
  );
}
