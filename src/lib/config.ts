import { invoke } from "@tauri-apps/api/core";

/** 各设置模块独立落盘：每处只写自己的字段，互不覆盖（后端按模块合成写盘） */

/** 翻译源：内置默认源开关 + 额外源勾选（翻译源 tab） */
export function saveSources(sources: string[], builtinSources: boolean) {
  return invoke("save_sources", { sources, builtinSources });
}

/** 百度密钥（源配置 tab） */
export function saveBaidu(appid: string, secret: string) {
  return invoke("save_baidu", { appid: appid.trim(), secret: secret.trim() });
}

/** 腾讯云密钥（源配置 tab） */
export function saveTencent(secretId: string, secretKey: string) {
  return invoke("save_tencent", { secretId: secretId.trim(), secretKey: secretKey.trim() });
}

/** 快捷键 + OCR 语言（快捷键/截图翻译 tab 的「保存」按钮）；注意 Tauri 参数是 camelCase */
export function saveSettings(fields: {
  hotkey: string;
  screenshotHotkey: string;
  inputHotkey: string;
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
