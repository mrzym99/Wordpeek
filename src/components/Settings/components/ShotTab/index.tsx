import type { ConfigForm } from "../../../../hooks/useConfigForm";
import HotkeyRecorder from "../../../HotkeyRecorder";
import s from "../../index.module.css";

/** 「截图翻译」页：截图快捷键 + OCR 语言 */
export default function ShotTab({ f }: { f: ConfigForm }) {
  return (
    <>
      <div className={s.sectionHeading}>截图翻译</div>
      <label className="field">
        截图快捷键
        <HotkeyRecorder value={f.screenshotHotkey} onChange={f.setScreenshotHotkey} />
      </label>
      <label className="field">
        OCR 识别语言
        <select value={f.ocrLang} onChange={(e) => f.setOcrLang(e.target.value)}>
          <option value="auto">跟随系统语言</option>
          <option value="zh">中文优先</option>
          <option value="en">英文优先</option>
        </select>
      </label>
      <div className="field-hint">按截图快捷键框选屏幕区域即可识别并翻译；整句翻译自动使用必应（免 Key，失败自动降级到有道），无需额外配置</div>
    </>
  );
}
