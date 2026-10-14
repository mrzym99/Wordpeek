import { invoke } from "@tauri-apps/api/core";

/** 各设置模块独立落盘：每处只写自己的字段，互不覆盖（后端按模块合成写盘） */

/** 翻译源勾选/排序（源列表 tab） */
export function saveSources(sources: string[]) {
  return invoke("save_sources", { sources });
}

/** 百度密钥（源配置 tab） */
export function saveBaidu(appid: string, secret: string) {
  return invoke("save_baidu", { appid: appid.trim(), secret: secret.trim() });
}

/** 快捷键 + OCR 语言（快捷键/截图翻译 tab 的「保存」按钮） */
export function saveSettings(fields: {
  hotkey: string;
  screenshot_hotkey: string;
  input_hotkey: string;
  ocr_lang: string;
}) {
  return invoke("save_settings", fields);
}
