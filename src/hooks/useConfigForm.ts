import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AppConfig } from "../types";
import {
  saveBaidu,
  saveTencent,
  saveSources,
  saveSettings,
} from "../lib/config";
import type { Theme } from "../lib/theme";

export type SettingsSection = "translate" | "hotkey" | "shot" | "appearance" | "perm" | "about" | "other";

/** macOS 需要在系统设置里手动授权；Windows 无此概念，菜单也不显示 */
export const IS_MAC = navigator.userAgent.includes("Macintosh");

/** 项目仓库地址，推荐/反馈共用 */
export const REPO_URL = "https://github.com/mrzym99/Wordpeek";

/** 可配置的额外翻译源：主翻译内置（单词→有道、句子→必应），这里只列额外源 */
export const SOURCE_ITEMS: { id: string; label: string; needKey: boolean }[] = [
  { id: "baidu", label: "百度翻译（需密钥）", needKey: true },
  { id: "tencent", label: "腾讯翻译（需密钥）", needKey: true },
];

/** 密钥字段是否都已填写（勾选需密钥源的前提） */
const keysReady = (a: string, b: string) => a.trim() !== "" && b.trim() !== "";

/** 设置窗口各页共用的配置表单状态与动作。
 *  状态全在这一处（切页不丢），各 Tab 组件只管渲染自己那一块 */
export function useConfigForm() {
  const [builtin, setBuiltin] = useState(true);
  const [sources, setSources] = useState<string[]>([]);
  // 翻译源区域子 tab：「list」勾选+排序列表 /「config」源配置（密钥）
  const [srcTab, setSrcTab] = useState<"list" | "config">("list");
  const [dragId, setDragId] = useState("");
  const [appid, setAppid] = useState("");
  const [secret, setSecret] = useState("");
  const [tcId, setTcId] = useState("");
  const [tcKey, setTcKey] = useState("");
  const [cfgSrc, setCfgSrc] = useState<"baidu" | "tencent">("baidu");
  const [hotkey, setHotkey] = useState("Alt+Z");
  const [screenshotHotkey, setScreenshotHotkey] = useState("Alt+S");
  const [inputHotkey, setInputHotkey] = useState("Alt+X");
  // 切换划词图标的快捷键（默认 Alt+W）
  const [iconHotkey, setIconHotkey] = useState("Alt+W");
  const [ocrLang, setOcrLang] = useState("auto");
  const [theme, setTheme] = useState<Theme>("system");
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
          setBuiltin(cfg.builtin_sources ?? true);
          setAppid(cfg.baidu.appid);
          setSecret(cfg.baidu.secret);
          setTcId(cfg.tencent?.secret_id ?? "");
          setTcKey(cfg.tencent?.secret_key ?? "");
          // 配置里勾着需密钥的源但密钥为空：本地显示为未勾选（下次保存会真正剥掉）
          setSources((cfg.sources ?? []).filter((s) => sourceReady(s, cfg)));
          setHotkey(cfg.hotkey || "Alt+Z");
          setScreenshotHotkey(cfg.screenshot_hotkey || "Alt+S");
          setInputHotkey(cfg.input_hotkey || "Alt+X");
          setIconHotkey(cfg.icon_hotkey || "Alt+W");
          setOcrLang(cfg.ocr_lang || "auto");
          setTheme(cfg.theme ?? "system");
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

  // 某个源的密钥是否就绪：配置加载后表单即真实值
  const ready = (id: string) =>
    id === "baidu" ? keysReady(appid, secret) : id === "tencent" ? keysReady(tcId, tcKey) : true;

  const flashSaved = () => {
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  };

  // 保存密钥后同步本地勾选（密钥清空时后端自动剥掉对应源）；
  // 失败必须上抖，否则界面显示「已保存」而密钥没落盘
  const syncAfterKeySave = async (id: string, isReady: boolean, save: () => Promise<unknown>) => {
    await save();
    if (!isReady && sources.includes(id)) {
      const next = sources.filter((s) => s !== id);
      setSources(next);
      await saveSources(next, builtin);
    }
  };

  const saveBaiduKeys = async () => {
    setSaveError("");
    try {
      await syncAfterKeySave("baidu", keysReady(appid, secret), () => saveBaidu(appid, secret));
      flashSaved();
    } catch (err) {
      setSaveError(String(err));
    }
  };

  const saveTencentKeys = async () => {
    setSaveError("");
    try {
      await syncAfterKeySave("tencent", keysReady(tcId, tcKey), () => saveTencent(tcId, tcKey));
      flashSaved();
    } catch (err) {
      setSaveError(String(err));
    }
  };

  // 约束提示：至少保留一个源；3s 后自动清除
  const denyToggle = (msg: string) => {
    setSaveError(msg);
    setTimeout(() => setSaveError(""), 3000);
  };

  // 内置默认源开关：关掉它是唯一源时禁止
  const toggleBuiltin = () => {
    if (builtin && sources.length === 0) {
      denyToggle("至少保留一个翻译源：请先勾选额外源，再关闭默认源");
      return;
    }
    const next = !builtin;
    setBuiltin(next);
    saveSources(sources, next).catch((e) => setSaveError(String(e)));
  };

  // 额外源勾选/取消：默认源已关闭时，不能取消最后一个额外源
  const toggleSource = (id: string) => {
    if (sources.includes(id) && !builtin && sources.length === 1) {
      denyToggle("至少保留一个翻译源：默认源已关闭，不能取消最后一个源");
      return;
    }
    const next = sources.includes(id) ? sources.filter((s) => s !== id) : [...sources, id];
    setSources(next);
    saveSources(next, builtin).catch((e) => setSaveError(String(e)));
  };

  // 拖拽排序：把 from 插到 to 之前
  const moveSource = (from: string, to: string) => {
    if (!from || from === to) return;
    const next = sources.filter((s) => s !== from);
    const at = next.indexOf(to);
    next.splice(at < 0 ? next.length : at, 0, from);
    setSources(next);
    saveSources(next, builtin).catch((e) => setSaveError(String(e)));
  };

  // 快捷键 + OCR 语言的统一保存；密钥/翻译源各由自己的模块命令落盘
  const saveGeneral = async () => {
    setSaved(false);
    setSaveError("");
    try {
      await saveSettings({ hotkey, screenshotHotkey, inputHotkey, iconHotkey, ocrLang });
      flashSaved();
    } catch (err) {
      setSaveError(String(err));
    }
  };

  return {
    builtin, sources, srcTab, setSrcTab, dragId, setDragId,
    appid, setAppid, secret, setSecret, tcId, setTcId, tcKey, setTcKey,
    cfgSrc, setCfgSrc, hotkey, setHotkey, screenshotHotkey, setScreenshotHotkey,
    inputHotkey, setInputHotkey, iconHotkey, setIconHotkey, ocrLang, setOcrLang, theme, setTheme,
    saveError, setSaveError, loadFailed, saved,
    ready, toggleBuiltin, toggleSource, moveSource,
    saveBaiduKeys, saveTencentKeys, saveGeneral,
  };
}

export type ConfigForm = ReturnType<typeof useConfigForm>;

/** 某个源的密钥是否就绪；不依赖组件状态的纯函数（加载时也用） */
function sourceReady(id: string, cfg: AppConfig) {
  switch (id) {
    case "baidu":
      return keysReady(cfg.baidu?.appid ?? "", cfg.baidu?.secret ?? "");
    case "tencent":
      return keysReady(cfg.tencent?.secret_id ?? "", cfg.tencent?.secret_key ?? "");
    default:
      return true;
  }
}
