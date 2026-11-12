import type { ConfigForm } from "../../../../hooks/useConfigForm";
import { IS_MAC } from "../../../../hooks/useConfigForm";
import HotkeyRecorder from "../../../HotkeyRecorder";
import s from "../../index.module.css";

/** 「快捷键」页：划词/输入翻译/划词图标开关快捷键（图标开关只由快捷键控制，设置页无复选框） */
export default function HotkeyTab({ f }: { f: ConfigForm }) {
  return (
    <>
      <div className={s.sectionHeading}>快捷键</div>
      <label className="field">
        划词快捷键
        <HotkeyRecorder value={f.hotkey} onChange={f.setHotkey} />
      </label>
      <label className="field" style={{ marginTop: 12 }}>
        划词图标开关快捷键
        <HotkeyRecorder value={f.iconHotkey} onChange={f.setIconHotkey} />
      </label>
      <div className="field-hint">
        按下切换光标旁翻译按钮的显示
      </div>
      <label className="field" style={{ marginTop: 12 }}>
        输入翻译快捷键
        <HotkeyRecorder value={f.inputHotkey} onChange={f.setInputHotkey} />
      </label>
      <div className="field-hint">
        点击输入框录入新组合键；需包含 {IS_MAC ? "Control / ⌥ Option / Shift / ⌘ Cmd" : "Ctrl / Alt / Shift"} 之一，四个快捷键不能相同
        {/* 存的是 Alt / Super，mac 上分别对应 ⌥ Option 和 ⌘ Cmd */}
      </div>
    </>
  );
}
