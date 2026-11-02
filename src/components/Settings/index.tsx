import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { saveSettingsSection } from "../../lib/config";
import { useConfigForm, type SettingsSection, IS_MAC } from "../../hooks/useConfigForm";
import TranslateTab from "./components/TranslateTab";
import HotkeyTab from "./components/HotkeyTab";
import ShotTab from "./components/ShotTab";
import AppearanceTab from "./components/AppearanceTab";
import PermTab from "./components/PermTab";
import AboutTab from "./components/AboutTab";
import OtherTab from "./components/OtherTab";
import s from "./index.module.css";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "translate", label: "翻译" },
  { id: "hotkey", label: "快捷键" },
  { id: "shot", label: "截图翻译" },
  { id: "appearance", label: "外观" },
  ...(IS_MAC ? [{ id: "perm" as const, label: "系统权限" }] : []),
  { id: "about", label: "关于" },
  { id: "other", label: "其他" },
];

/** 独立设置窗口：左侧菜单切换分组，用户手动关闭，不会被 blur 隐藏。
 *  各分组的内容在 components/ 下各 Tab，配置状态在 useConfigForm.ts */
export default function SettingsPage() {
  const f = useConfigForm();
  const [section, setSection] = useState<SettingsSection>("translate");

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
      </div>

      <div className={s.content}>
        {section === "translate" && <TranslateTab f={f} />}
        {section === "hotkey" && <HotkeyTab f={f} />}
        {section === "shot" && <ShotTab f={f} />}
        {section === "appearance" && <AppearanceTab f={f} />}
        {section === "perm" && IS_MAC && <PermTab />}
        {section === "about" && <AboutTab />}
        {section === "other" && <OtherTab f={f} />}

        <div className={s.footer}>
          {f.saveError && <span className={s.saveError}>{f.saveError}</span>}
          {f.saved && !f.saveError && <span className={s.savedTip}>已保存</span>}
          {/* 源配置：密钥表单的保存按钮也走底栏，与其他页一样落在右下角 */}
          {section === "translate" && f.srcTab === "config" ? (
            <button
              className={s.saveBtn}
              onClick={f.cfgSrc === "baidu" ? f.saveBaiduKeys : f.saveTencentKeys}
              disabled={f.loadFailed}
            >
              保存
            </button>
          ) : section === "hotkey" || section === "shot" || section === "appearance" ? (
            <button className={s.saveBtn} onClick={f.saveGeneral}>
              保存
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
