import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AppConfig, PermissionStatus, UpdateStatus } from "../types";
import { saveBaidu, saveTencent, saveSources, saveSettings, saveSettingsSection } from "../lib/config";
import HotkeyRecorder from "./HotkeyRecorder";
import { CheckIcon, StarIcon } from "./icons";

type SettingsSection = "translate" | "hotkey" | "shot" | "perm" | "about" | "other";

/** macOS 需要在系统设置里手动授权；Windows 无此概念，菜单也不显示 */
const IS_MAC = navigator.userAgent.includes("Macintosh");

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "translate", label: "翻译" },
  { id: "hotkey", label: "快捷键" },
  { id: "shot", label: "截图翻译" },
  ...(IS_MAC ? [{ id: "perm" as const, label: "系统权限" }] : []),
  { id: "about", label: "关于" },
  { id: "other", label: "其他" },
];

/** 项目仓库地址，推荐/反馈共用 */
const REPO_URL = "https://github.com/mrzym99/Wordpeek";

/** 可配置的额外翻译源：主翻译内置（单词→有道、句子→必应），这里只列额外源 */
const SOURCE_ITEMS: { id: string; label: string; needKey: boolean }[] = [
  { id: "baidu", label: "百度翻译（需密钥）", needKey: true },
  { id: "tencent", label: "腾讯翻译（需密钥）", needKey: true },
];

/** 密钥字段是否都已填写（勾选需密钥源的前提） */
const keysReady = (a: string, b: string) => a.trim() !== "" && b.trim() !== "";

/** 某个源的密钥是否就绪；不依赖组件状态的纯函数（加载时也用） */
const sourceReady = (id: string, cfg: AppConfig) => {
  switch (id) {
    case "baidu":
      return keysReady(cfg.baidu?.appid ?? "", cfg.baidu?.secret ?? "");
    case "tencent":
      return keysReady(cfg.tencent?.secret_id ?? "", cfg.tencent?.secret_key ?? "");
    default:
      return true;
  }
};

/** 复制反馈标记："text" 推荐语 | "link" 链接 | "" 无 */
type CopiedKind = "" | "text" | "link";

/** 独立设置窗口：左侧菜单切换分组，用户手动关闭，不会被 blur 隐藏 */
export default function SettingsPage() {
  const [section, setSection] = useState<SettingsSection>("translate");
  // 内置默认源（有道/必应）开关；关闭后仅用勾选的额外源
  const [builtin, setBuiltin] = useState(true);
  // 勾选的额外源
  const [sources, setSources] = useState<string[]>([]);
  // 翻译源区域子 tab：「list」勾选+排序列表 /「config」源配置（密钥）
  const [srcTab, setSrcTab] = useState<"list" | "config">("list");
  const [dragId, setDragId] = useState("");
  const [appid, setAppid] = useState("");
  const [secret, setSecret] = useState("");
  const [tcId, setTcId] = useState("");
  const [tcKey, setTcKey] = useState("");
  // 源配置 tab 当前在编辑哪个源的密钥
  const [cfgSrc, setCfgSrc] = useState<"baidu" | "tencent">("baidu");
  const [hotkey, setHotkey] = useState("Ctrl+Alt+T");
  const [screenshotHotkey, setScreenshotHotkey] = useState("Ctrl+Alt+S");
  const [inputHotkey, setInputHotkey] = useState("Ctrl+Alt+I");
  const [ocrLang, setOcrLang] = useState("auto");
  const [saveError, setSaveError] = useState("");
  // get_config 重试耗尽：表单停在默认值，此时绝不能允许保存（会把默认值覆盖到真配置上）
  const [loadFailed, setLoadFailed] = useState(false);
  const [saved, setSaved] = useState(false);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [updateError, setUpdateError] = useState("");
  const [checked, setChecked] = useState(false); // 是否检查过更新（用于显示"已是最新"）
  const [copied, setCopied] = useState<CopiedKind>(""); // 复制成功反馈，1.5s 后复位
  // macOS 系统权限（Windows 恒为已授）
  const [perms, setPerms] = useState<PermissionStatus | null>(null);

  // 读权限状态；从系统设置授权回来（窗口重新获得焦点）时再读一次
  useEffect(() => {
    if (!IS_MAC) return;
    const refresh = () =>
      invoke<PermissionStatus>("check_permissions").then(setPerms).catch(() => {});
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);

  // 窗口由 Rust 按需创建为隐藏态（open_settings），内容挂好了再自己显示，避免先白屏
  useEffect(() => {
    const win = getCurrentWindow();
    win.show().catch(() => {});
    win.setFocus().catch(() => {});
  }, []);

  // 复制推荐语/链接；成功后按钮短暂显示"已复制 ✓"
  const copyShare = (kind: Exclude<CopiedKind, "">, text: string) => {
    invoke("copy_text", { text })
      .then(() => {
        setCopied(kind);
        setTimeout(() => setCopied(""), 1500);
      })
      .catch((e) => setSaveError(String(e)));
  };

  // 进入"关于"分组时读取启动时缓存的检查结果（不发网络请求）
  useEffect(() => {
    if (section !== "about") return;
    invoke<UpdateStatus>("get_update_status")
      .then((s) => {
        setUpdate(s);
        setChecked(true);
      })
      .catch(() => {});
  }, [section]);

  const checkUpdate = async () => {
    setChecking(true);
    setUpdateError("");
    try {
      setUpdate(await invoke<UpdateStatus>("check_update_now"));
      setChecked(true);
    } catch (err) {
      setUpdateError(String(err));
    } finally {
      setChecking(false);
    }
  };

  // 下载安装由 Rust 后台线程执行，成功后应用自动重启
  const installUpdate = async () => {
    setUpdating(true);
    setUpdateError("");
    try {
      await invoke("install_update");
    } catch (err) {
      setUpdateError(String(err));
      setUpdating(false);
    }
  };

  useEffect(() => {
    // 设置窗口页面在应用启动时就会加载，可能早于后端状态就绪：
    // get_config 失败不能静默放弃，否则表单停在默认值，
    // 之后任何一次保存都会把真配置（密钥/快捷键）覆盖掉
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
          setHotkey(cfg.hotkey || "Ctrl+Alt+T");
          setScreenshotHotkey(cfg.screenshot_hotkey || "Ctrl+Alt+S");
          setInputHotkey(cfg.input_hotkey || "Ctrl+Alt+I");
          setOcrLang(cfg.ocr_lang || "auto");
          // 回到上次停留的一级菜单（切页时已落盘；配置里没有/非法就留在默认）
          const saved = SECTIONS.find((s) => s.id === cfg.settings_section)?.id;
          if (saved) setSection(saved);
        })
        .catch(() => {
          if (cancelled) return;
          if (tries > 0) {
            setTimeout(() => load(tries - 1), 300);
          } else {
            // 表单此刻停在默认值：禁用保存，避免一次保存把真配置（密钥/快捷键）抹成默认
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

  // 用当前表单里的密钥判断某源是否就绪（配置加载后表单即真实值）
  const ready = (id: string) =>
    id === "baidu"
      ? keysReady(appid, secret)
      : id === "tencent"
        ? keysReady(tcId, tcKey)
        : true;

  // 保存密钥后同步勾选：密钥被清空时后端会自动剥掉对应源，这里同步本地状态。
  // 失败必须上抖——静默失败会让界面显示「已保存」而密钥其实没落盘
  const syncAfterKeySave = async (
    id: string,
    isReady: boolean,
    save: () => Promise<unknown>
  ) => {
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
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  const saveTencentKeys = async () => {
    setSaveError("");
    try {
      await syncAfterKeySave("tencent", keysReady(tcId, tcKey), () => saveTencent(tcId, tcKey));
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
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
    const next = sources.includes(id)
      ? sources.filter((s) => s !== id)
      : [...sources, id];
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

  const save = async () => {
    setSaved(false);
    setSaveError("");
    try {
      // 只写快捷键 + OCR 语言；密钥/翻译源各由自己的模块命令落盘
      await saveSettings({
        hotkey,
        screenshotHotkey,
        inputHotkey,
        ocrLang,
      });
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  return (
    <div className="settings-layout">
      <div className="settings-nav">
        {SECTIONS.map((s) => (
          <div
            key={s.id}
            className={`nav-item ${section === s.id ? "active" : ""}`}
            onClick={() => {
              setSection(s.id);
              // 记住一级菜单：设置窗口已改为关掉即销毁，重开要回到这页
              saveSettingsSection(s.id).catch(() => {});
            }}
          >
            {s.label}
          </div>
        ))}
      </div>

      <div className="settings-content">
        {section === "translate" && (
          <>
            {/* 翻译源两个子页：列表（勾选+拖拽排序）/ 源配置，横向 tab 切换 */}
            <div className="sub-tabs">
              <button className={"sub-tab" + (srcTab === "list" ? " active" : "")} onClick={() => setSrcTab("list")}>
                翻译源
              </button>
              <button className={"sub-tab" + (srcTab === "config" ? " active" : "")} onClick={() => setSrcTab("config")}>
                源配置
              </button>
            </div>
            {srcTab === "list" ? (
              <>
                <div className="group-heading">翻译源（至少保留一个）</div>
                <div className="source-list">
                  <div className={"source-item" + (builtin ? "" : " off")}>
                    <label className="switch">
                      <input type="checkbox" checked={builtin} onChange={toggleBuiltin} />
                      默认源：有道词典 / 必应翻译（内置，免 Key）
                    </label>
                  </div>
                  {sources.map((id) => {
                    const item = SOURCE_ITEMS.find((s) => s.id === id);
                    if (!item) return null;
                    return (
                      <div
                        key={id}
                        className="source-item"
                        draggable
                        onDragStart={(e) => {
                          // 拖拽失效两个根因：① 窗口 dragDropEnabled 默认开启会吞掉
                          // HTML5 拖拽（已在 tauri.conf.json 关掉）；② setData 必须调，
                          // 否则 WebView2 里 drop 不触发
                          e.dataTransfer.setData("text/plain", id);
                          e.dataTransfer.effectAllowed = "move";
                          setDragId(id);
                        }}
                        onDragOver={(e) => e.preventDefault()}
                        onDrop={(e) => {
                          e.preventDefault();
                          moveSource(dragId, id);
                          setDragId("");
                        }}
                        onDragEnd={() => setDragId("")}
                      >
                        <span className="drag-handle" title="拖拽调整顺序">
                          ⠿
                        </span>
                        <label className="switch">
                          <input
                            type="checkbox"
                            checked
                            disabled={item.needKey && !ready(item.id)}
                            title={item.needKey && !ready(item.id) ? "密钥未配置，请先在「源配置」里填写" : undefined}
                            onChange={() => toggleSource(id)}
                          />
                          {item.label}
                        </label>
                      </div>
                    );
                  })}
                  {SOURCE_ITEMS.filter((s) => !sources.includes(s.id)).map((s) => (
                    <div key={s.id} className="source-item off">
                      <span className="drag-handle" />
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={false}
                          disabled={s.needKey && !ready(s.id)}
                          title={s.needKey && !ready(s.id) ? "密钥未配置，请先在「源配置」里填写" : undefined}
                          onChange={() => toggleSource(s.id)}
                        />
                        {s.label}
                      </label>
                    </div>
                  ))}
                </div>
                <div className="field-hint">
                  默认源：单词用有道、句子用必应。其他源配置后可选择。
                </div>
              </>
            ) : (
              // 源配置：左侧菜单选源，右侧编辑对应密钥
              <div className="config-split">
                <div className="config-menu">
                  {SOURCE_ITEMS.map((s) => (
                    <div
                      key={s.id}
                      className={"config-menu-item" + (cfgSrc === s.id ? " active" : "")}
                      onClick={() => setCfgSrc(s.id as "baidu" | "tencent")}
                    >
                      {s.label.replace("（需密钥）", "")}
                      {ready(s.id) && <span className="config-dot" title="已配置" />}
                    </div>
                  ))}
                </div>
                <div className="config-form">
                  {cfgSrc === "baidu" ? (
                    <>
                      <div className="group-heading">百度翻译密钥（可选）</div>
                      <label className="field">
                        APP ID
                        <input
                          value={appid}
                          placeholder="百度翻译开放平台 appid"
                          onChange={(e) => setAppid(e.target.value)}
                        />
                      </label>
                      <label className="field">
                        密钥
                        <input
                          type="password"
                          value={secret}
                          placeholder="百度翻译开放平台密钥"
                          onChange={(e) => setSecret(e.target.value)}
                        />
                      </label>
                      <div className="source-actions">
                        <button className="save-btn" onClick={saveBaiduKeys} disabled={loadFailed}>
                          保存密钥
                        </button>
                      </div>
                    </>
                  ) : (
                    <>
                      <div className="group-heading">腾讯云密钥（可选）</div>
                      <label className="field">
                        SecretId
                        <input
                          value={tcId}
                          placeholder="腾讯云访问管理 API 密钥 SecretId"
                          onChange={(e) => setTcId(e.target.value)}
                        />
                      </label>
                      <label className="field">
                        SecretKey
                        <input
                          type="password"
                          value={tcKey}
                          placeholder="腾讯云访问管理 API 密钥 SecretKey"
                          onChange={(e) => setTcKey(e.target.value)}
                        />
                      </label>
                      <div className="source-actions">
                        <button className="save-btn" onClick={saveTencentKeys} disabled={loadFailed}>
                          保存密钥
                        </button>
                      </div>
                    </>
                  )}
                </div>
              </div>
            )}
          </>
        )}

        {section === "hotkey" && (
          <>
            <div className="section-heading">快捷键</div>
            <label className="field">
              划词快捷键
              <HotkeyRecorder value={hotkey} onChange={setHotkey} />
            </label>
            <label className="field">
              输入翻译快捷键
              <HotkeyRecorder value={inputHotkey} onChange={setInputHotkey} />
            </label>
            <div className="field-hint">
              点击输入框后按下组合键即可更换，需包含 {IS_MAC ? "Control / ⌥ Option / Shift / ⌘ Cmd" : "Ctrl / Alt / Shift"} 之一；划词、截图、输入翻译三个快捷键不能相同
              {/* 存的是 Alt / Super，mac 上分别对应 ⌥ Option 和 ⌘ Cmd */}
            </div>
          </>
        )}

        {section === "shot" && (
          <>
            <div className="section-heading">截图翻译</div>
            <label className="field">
              截图快捷键
              <HotkeyRecorder value={screenshotHotkey} onChange={setScreenshotHotkey} />
            </label>
            <label className="field">
              OCR 识别语言
              <select value={ocrLang} onChange={(e) => setOcrLang(e.target.value)}>
                <option value="auto">跟随系统语言</option>
                <option value="zh">中文优先</option>
                <option value="en">英文优先</option>
              </select>
            </label>
            <div className="field-hint">按截图快捷键框选屏幕区域即可识别并翻译；整句翻译自动使用必应（免 Key），无需额外配置</div>
          </>
        )}

        {section === "perm" && IS_MAC && (
          <>
            <div className="section-heading">系统权限</div>
            <div className="about-text">
              <p>
                Wordpeek 需要两项系统权限：划词翻译靠<b>辅助功能</b>模拟 Cmd+C 读取选中文本，
                截图翻译靠<b>屏幕录制</b>截取屏幕像素。
              </p>
              <p>授权后请重启应用（修改权限后系统需要重启进程才生效）。</p>
            </div>
            <div className="update-tip">
              <span className="tip-text">
                辅助功能：{perms?.accessibility ? <span className="i-txt">已授权 <CheckIcon /></span> : "未授权"}
              </span>
              <div className="tip-actions">
                <button
                  className="update-btn"
                  onClick={() =>
                    openUrl(
                      "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                    ).catch(() => {})
                  }
                >
                  打开设置
                </button>
              </div>
            </div>
            <div className="update-tip">
              <span className="tip-text">
                屏幕录制：{perms?.screen_capture ? <span className="i-txt">已授权 <CheckIcon /></span> : "未授权"}
              </span>
              <div className="tip-actions">
                <button
                  className="update-btn"
                  onClick={() =>
                    openUrl(
                      "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                    ).catch(() => {})
                  }
                >
                  打开设置
                </button>
              </div>
            </div>
          </>
        )}

        {section === "about" && (
          <>
            <div className="section-heading">关于</div>
            <div className="about-text">
              <p>
                Wordpeek{update ? ` v${update.current_version}` : ""}
                <span className="app-beta">beta</span> — 最小划词翻译
              </p>
              <p>在任意窗口选中单词，按快捷键，鼠标旁弹出翻译卡片。</p>
              <p>
                设置保存于{" "}
                {IS_MAC
                  ? "~/Library/Application Support/com.wordpeek.desktop/config.json"
                  : "%APPDATA%\\com.wordpeek.desktop\\config.json"}
              </p>
            </div>
            <div className="update-tip">
              {update?.available ? (
                <>
                  <span className="tip-text">
                    发现新版本 v{update.new_version}，下载安装完成后将自动重启
                  </span>
                  <div className="tip-actions">
                    <button
                      className="update-btn"
                      onClick={installUpdate}
                      disabled={updating}
                    >
                      {updating ? "下载安装中…" : "立即更新"}
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <span className="tip-text">
                    {checked ? "当前已是最新版本。" : "检查 GitHub Releases 获取新版本。"}
                  </span>
                  <div className="tip-actions">
                    <button
                      className="update-btn"
                      onClick={checkUpdate}
                      disabled={checking}
                    >
                      {checking ? "检查中…" : "检查更新"}
                    </button>
                  </div>
                </>
              )}
            </div>
            {updateError && <div className="field-error">{updateError}</div>}
          </>
        )}

        {section === "other" && (
          <>
            <div className="section-heading">向我推荐</div>
            <div className="about-text">
              <p>如果 Wordpeek 对你有帮助，欢迎推荐给身边的同学和同事。</p>
              <p>项目完全开源、免费，GitHub 搜索 mrzym99/Wordpeek：</p>
              <p>{REPO_URL}</p>
              <p>去 GitHub 点个 Star，或分享下面的推荐语：</p>
            </div>
            <div className="update-tip">
              <span className="tip-text">
                推荐试试 Wordpeek——划词/截图即按即译，免 Key 开箱即用。
              </span>
              <div className="tip-actions">
                <button
                  className="update-btn"
                  onClick={() =>
                    copyShare(
                      "text",
                      "推荐试试 Wordpeek——划词/截图即按即译，免 Key 开箱即用。开源地址：" + REPO_URL,
                    )
                  }
                >
                  {copied === "text" ? <span className="i-txt">已复制 <CheckIcon /></span> : "复制推荐语"}
                </button>
                <button
                  className="update-btn"
                  onClick={() => copyShare("link", REPO_URL)}
                >
                  {copied === "link" ? <span className="i-txt">已复制 <CheckIcon /></span> : "复制链接"}
                </button>
              </div>
            </div>
            <div className="update-tip">
              <div className="tip-actions">
              <button
                className="update-btn"
                onClick={() => openUrl(REPO_URL).catch((e) => setSaveError(String(e)))}
              >
                <span className="i-txt">去 GitHub 点 Star <StarIcon /></span>
              </button>
              <button
                className="update-btn"
                onClick={() =>
                  openUrl(REPO_URL + "/issues").catch((e) => setSaveError(String(e)))
                }
              >
                提交 Issue 反馈
              </button>
              </div>
            </div>
          </>
        )}

        <div className="settings-footer">
          {saveError && <span className="save-error">{saveError}</span>}
          {saved && !saveError && <span className="saved-tip">已保存</span>}
          {/* 翻译/关于/其他：无表单或即时生效，无需全局保存 */}
          {section !== "translate" && section !== "about" && section !== "other" && (
            <button className="save-btn" onClick={save}>
              保存
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
