import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AppConfig } from "../types";
import {
  saveSources,
  saveSettings,
} from "../lib/config";
import type { Theme } from "../lib/theme";

export type SettingsSection = "translate" | "hotkey" | "perm" | "about";

/** macOS 需要在系统设置里手动授权；Windows 无此概念，菜单也不显示 */
export const IS_MAC = navigator.userAgent.includes("Macintosh");

/** 项目仓库地址，推荐/反馈共用 */
export const REPO_URL = "https://github.com/mrzym99/Wordpeek";

/** 可配置的句子翻译源：有道词典（单词）内置必用；deepl/bing 自由勾选，至少留一个 */
export const SOURCE_ITEMS: { id: string; label: string }[] = [
  { id: "deepl", label: "DeepL 翻译（免 Key）" },
  { id: "bing", label: "必应翻译（免 Key）" },
];

/** 设置窗口各页共用的配置表单状态与动作。
 *  状态全在这一处（切页不丢），各 Tab 组件只管渲染自己那一块 */
export function useConfigForm() {
  // 句子翻译源勾选（deepl/bing，至少一个）
  const [sources, setSources] = useState<string[]>(["deepl", "bing"]);
  const [hotkey, setHotkey] = useState("Alt+Z");
  const [screenshotHotkey, setScreenshotHotkey] = useState("Alt+S");
  const [inputHotkey, setInputHotkey] = useState("Alt+X");
  // 切换划词图标的快捷键（默认 Alt+W）
  const [iconHotkey, setIconHotkey] = useState("Alt+W");
  const [theme, setTheme] = useState<Theme>("system");
  // 一级菜单：上次停留的页面（老配置里可能存着已删掉的 shot/appearance/other，按白名单回落）
  const [section, setSection] = useState<SettingsSection>("translate");
  const [saveError, setSaveError] = useState("");
  // get_config 重试耗尽：表单停在默认值，此时绝不能允许保存（会把默认值覆盖到真配置上）
  const [loadFailed, setLoadFailed] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    // 设置窗口加载可能早于后端就绪：get_config 失败不能静默放弃，
    // 否则表单停在默认值，之后任何一次保存都会把真配置（密钥/快捷键）覆盖掉
    let cancelled = false;
    const load = (tries: number) => {
      invoke<AppConfig>("get_config")
        .then((cfg) => {
          if (cancelled) return;
          setSources(
            Array.isArray(cfg.sources) && cfg.sources.length > 0
              ? cfg.sources
              : ["deepl", "bing"]
          );
          setHotkey(cfg.hotkey || "Alt+Z");
          setScreenshotHotkey(cfg.screenshot_hotkey || "Alt+S");
          setInputHotkey(cfg.input_hotkey || "Alt+X");
          setIconHotkey(cfg.icon_hotkey || "Alt+W");
          setTheme(cfg.theme ?? "system");
          const sec = cfg.settings_section;
          if (sec === "translate" || sec === "hotkey" || sec === "about" || (sec === "perm" && IS_MAC)) {
            setSection(sec);
          }
        })
        .catch(() => {
          if (cancelled) return;
          if (tries > 0) {
            setTimeout(() => load(tries - 1), 300);
          } else {
            setSaveError("配置加载失败，保存已停用（重启应用后再试）");
            setLoadFailed(true);
          }
        });
    };
    load(10);
    return () => {
      cancelled = true;
    };
  }, []);

  const flashSaved = () => {
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  };

  // 约束提示：3s 后自动清除
  const denyToggle = (msg: string) => {
    setSaveError(msg);
    setTimeout(() => setSaveError(""), 3000);
  };

  // 句子源勾选：取消最后一个句子源时拦下（deepl/bing 至少留一个）
  const toggleSource = (id: string) => {
    const next = sources.includes(id)
      ? sources.filter((s) => s !== id)
      : [...sources, id];
    if (next.length === 0) {
      denyToggle("句子翻译至少保留一个源：DeepL 或 必应");
      return;
    }
    setSources(next);
    saveSources(next).catch((e) => setSaveError(String(e)));
  };

  // 快捷键 + OCR 语言的统一保存；密钥/句子源各由自己的模块命令落盘
  const saveGeneral = async () => {
    setSaved(false);
    setSaveError("");
    try {
      // OCR 语言不再单独设置，固定跟随系统（后端 save_settings 仍带这个字段）
      await saveSettings({ hotkey, screenshotHotkey, inputHotkey, iconHotkey, ocrLang: "auto" });
      flashSaved();
    } catch (err) {
      setSaveError(String(err));
    }
  };

  return {
    sources,
    hotkey, setHotkey, screenshotHotkey, setScreenshotHotkey,
    inputHotkey, setInputHotkey, iconHotkey, setIconHotkey, theme, setTheme, section, setSection,
    saveError, setSaveError, loadFailed, saved,
    toggleSource,
    saveGeneral,
  };
}

export type ConfigForm = ReturnType<typeof useConfigForm>;
