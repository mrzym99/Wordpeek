import type { ConfigForm } from "../../../../hooks/useConfigForm";
import { IS_MAC } from "../../../../hooks/useConfigForm";
import HotkeyRecorder from "../../../HotkeyRecorder";

/** 「快捷键」页：划词/输入/截图翻译快捷键 + 划词轻提示开关（开关只由快捷键/托盘控制，设置页无复选框） */
export default function HotkeyTab({ f }: { f: ConfigForm }) {
  return (
    <>
      <label className="field">
        划词
        <HotkeyRecorder value={f.hotkey} onChange={f.setHotkey} />
      </label>
      <label className="field" style={{ marginTop: 12 }}>
        划词轻提示是否开启
        <HotkeyRecorder value={f.iconHotkey} onChange={f.setIconHotkey} />
      </label>
      <label className="field" style={{ marginTop: 12 }}>
        输入翻译
        <HotkeyRecorder value={f.inputHotkey} onChange={f.setInputHotkey} />
      </label>
      <label className="field" style={{ marginTop: 12 }}>
        截图翻译
        <HotkeyRecorder value={f.screenshotHotkey} onChange={f.setScreenshotHotkey} />
      </label>
      <div className="field-hint">
        点击输入框录入新组合键；需包含 {IS_MAC ? "Control / ⌥ Option / Shift / ⌘ Cmd" : "Ctrl / Alt / Shift"} 之一，四个快捷键不能相同
        {/* 存的是 Alt / Super，mac 上分别对应 ⌥ Option 和 ⌘ Cmd */}
      </div>
    </>
  );
}