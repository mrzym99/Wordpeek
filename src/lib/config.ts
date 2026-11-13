import { invoke } from "@tauri-apps/api/core";

/** 各设置模块独立落盘：每处只写自己的字段，互不覆盖（后端按模块合成写盘） */

/** 翻译源：句子源 deepl/bing 勾选（翻译源 tab）；有道词典单词用，内置必用 */
export function saveSources(sources: string[]) {
  return invoke("save_sources", { sources });
}

/** 快捷键（快捷键 tab 的「保存」按钮）；注意 Tauri 参数是 camelCase。
 *  ocrLang 已无 UI，固定传 "auto"：后端 save_settings 仍要求该参数 */
export function saveSettings(fields: {
  hotkey: string;
  screenshotHotkey: string;
  inputHotkey: string;
  iconHotkey: string;
  ocrLang: string;
}) {
  return invoke("save_settings", fields);
}

/** 弹窗宽度（用户拖拽后防抖落盘，重启恢复）；高度自适应不存 */
export function savePopupWidth(width: number) {
  return invoke("save_popup_width", { width });
}

/** 设置窗口当前一级菜单：切页即落盘，重开设置窗口时回到上次页 */
export function saveSettingsSection(section: string) {
  return invoke("save_settings_section", { section });
}

/** 外观主题（外观 tab）：system | light | dark */
export function saveTheme(theme: "system" | "light" | "dark") {
  return invoke("save_theme", { theme });
}
