import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { saveSettingsSection } from "../../lib/config";
import { useConfigForm, type SettingsSection, IS_MAC } from "../../hooks/useConfigForm";
import TranslateTab from "./components/TranslateTab";
import HotkeyTab from "./components/HotkeyTab";
import ThemeIcons from "./components/ThemeIcons";
import PermTab from "./components/PermTab";
import AboutTab from "./components/AboutTab";
import mascot from "../../assets/about-mascot.webp";
import s from "./index.module.css";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "translate", label: "翻译源" },
  { id: "hotkey", label: "快捷键" },
  ...(IS_MAC ? [{ id: "perm" as const, label: "系统权限" }] : []),
  { id: "about", label: "关于" },
];

/** 独立设置窗口：左侧菜单切换分组，用户手动关闭，不会被 blur 隐藏。
 *  各分组的内容在 components/ 下各 Tab，配置状态在 useConfigForm.ts */
export default function SettingsPage() {
  const f = useConfigForm();
  const { section, setSection } = f;

  // 窗口由 Rust 按需创建为隐藏态（open_settings），内容挂好了再自己显示，避免先白屏
  useEffect(() => {
    const win = getCurrentWindow();
    win.show().catch(() => {});
    win.setFocus().catch(() => {});
  }, []);

  return (
    <div className={s.layout}>
      {/* macOS 自绘头部：与左侧菜单同色，Windows 下隐藏（原生标题栏还在） */}
      <div className="win-head" data-tauri-drag-region />
      <div className={s.nav}>
        {SECTIONS.map((it) => (
          <button
            key={it.id}
            type="button"
            className={s.navItem + (section === it.id ? " active" : "")}
            onClick={() => {
              setSection(it.id);
              // 记住一级菜单：设置窗口已改为关掉即销毁，重开要回到这页
              saveSettingsSection(it.id).catch(() => {});
            }}
          >
            {it.label}
          </button>
        ))}
        {/* 主题留在菜单底部：三档图标直接切换，不占一个一级菜单 */}
        <ThemeIcons value={f.theme} onChange={f.setTheme} />
      </div>

      <div className={s.content}>
        <div className={s.card}>
          {/* 右下角吉祥物：绝对定位在卡片底层，不吃鼠标事件，内容压在上面。
              快捷键页表单铺满整卡，放了会压住输入框，所以那页不放 */}
          {section !== "hotkey" && <img className={s.mascot} src={mascot} alt="" aria-hidden />}
          {section === "translate" && <TranslateTab f={f} />}
          {section === "hotkey" && <HotkeyTab f={f} />}
          {section === "perm" && IS_MAC && <PermTab />}
          {section === "about" && <AboutTab />}

          <div className={s.footer}>
            {f.saveError && <span className={s.saveError}>{f.saveError}</span>}
            {f.saved && !f.saveError && <span className={s.savedTip}>已保存</span>}
            {/* 源配置的保存按钮在各自手风琴项里；其他页统一走底栏 */}
            {section === "hotkey" && (
              <button className={s.saveBtn} onClick={f.saveGeneral}>
                保存
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
