import type { ConfigForm } from "../../../../hooks/useConfigForm";
import LangToggle from "../../../LangToggle";
import { applyTheme, type Theme } from "../../../../lib/theme";
import { saveTheme } from "../../../../lib/config";
import s from "../../index.module.css";

/** 外观主题三档（顺序即显示顺序） */
const THEMES: { id: Theme; label: string }[] = [
  { id: "system", label: "跟随系统" },
  { id: "light", label: "亮色" },
  { id: "dark", label: "暗色" },
];

/** 「外观」页：主题三档，点选即时生效（设置窗口自己先变，其它窗口下次聚焦时跟上） */
export default function AppearanceTab({ f }: { f: ConfigForm }) {
  return (
    <>
      <div className={s.sectionHeading}>外观</div>
      <div className="field">
        主题
        <LangToggle
          options={THEMES}
          value={f.theme}
          className={s.themeToggle}
          onChange={(id) => {
            f.setTheme(id);
            applyTheme(id);
            saveTheme(id).catch(() => {});
          }}
        />
      </div>
      <div className="field-hint">
        跟随系统时，系统切换深浅色后弹窗立即跟着变；手动指定则固定不变。
      </div>
    </>
  );
}
