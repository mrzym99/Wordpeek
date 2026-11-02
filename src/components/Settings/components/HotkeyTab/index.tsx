import type { ConfigForm } from "../../../../hooks/useConfigForm";
import { IS_MAC } from "../../../../hooks/useConfigForm";
import HotkeyRecorder from "../../../HotkeyRecorder";
import s from "../../index.module.css";

/** 「快捷键」页：划词/输入翻译快捷键 + 划词图标开关 */
export default function HotkeyTab({ f }: { f: ConfigForm }) {
  return (
    <>
      <div className={s.sectionHeading}>快捷键</div>
      <label className="field">
        划词快捷键
        <HotkeyRecorder value={f.hotkey} onChange={f.setHotkey} />
      </label>
      <div className={s.sourceItem}>
        <label className={s.switch}>
          <input type="checkbox" checked={f.selectionIcon} onChange={f.toggleSelectionIcon} />
          选中文字后在光标旁显示翻译按钮
        </label>
      </div>
      <div className="field-hint">
        鼠标选中文字后，光标旁会出现一个小按钮，点一下直接翻译；终端、VS Code 这类不向系统暴露选区文本的应用里不会出现
      </div>
      <label className="field" style={{ marginTop: 12 }}>
        输入翻译快捷键
        <HotkeyRecorder value={f.inputHotkey} onChange={f.setInputHotkey} />
      </label>
      <div className="field-hint">
        点击输入框后按下组合键即可更换，需包含 {IS_MAC ? "Control / ⌥ Option / Shift / ⌘ Cmd" : "Ctrl / Alt / Shift"} 之一；划词、截图、输入翻译三个快捷键不能相同
        {/* 存的是 Alt / Super，mac 上分别对应 ⌥ Option 和 ⌘ Cmd */}
      </div>
    </>
  );
}
