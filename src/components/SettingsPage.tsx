import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AppConfig, UpdateStatus } from "../types";
import { saveSources, saveBaidu, saveSettings } from "../lib/config";
import HotkeyRecorder from "./HotkeyRecorder";

type SettingsSection = "translate" | "hotkey" | "shot" | "about" | "other";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "translate", label: "翻译" },
  { id: "hotkey", label: "快捷键" },
  { id: "shot", label: "截图翻译" },
  { id: "about", label: "关于" },
  { id: "other", label: "其他" },
];

/** 项目仓库地址，推荐/反馈共用 */
const REPO_URL = "https://github.com/mrzym99/Wordpeek";

/** 可选翻译源（列表顺序由用户拖拽决定 = 优先级） */
const SOURCE_ITEMS: { id: string; label: string }[] = [
  { id: "youdao", label: "有道词典（免 Key，单词）" },
  { id: "bing", label: "必应翻译（免 Key，整句）" },
  { id: "baidu", label: "百度翻译（需密钥）" },
];

/** 复制反馈标记："text" 推荐语 | "link" 链接 | "" 无 */
type CopiedKind = "" | "text" | "link";

/** 独立设置窗口：左侧菜单切换分组，用户手动关闭，不会被 blur 隐藏 */
export default function SettingsPage() {
  const [section, setSection] = useState<SettingsSection>("translate");
  // 勾选的源，顺序即优先级（排序 tab 里拖拽调整）
  const [sources, setSources] = useState<string[]>(["youdao", "bing"]);
  // 翻译源区域子 tab：「list」勾选+排序列表 /「config」源配置（密钥）
  const [srcTab, setSrcTab] = useState<"list" | "config">("list");
  const [dragId, setDragId] = useState("");
  const [appid, setAppid] = useState("");
  const [secret, setSecret] = useState("");
  const [hotkey, setHotkey] = useState("Ctrl+Alt+T");
  const [screenshotHotkey, setScreenshotHotkey] = useState("Ctrl+Alt+S");
  const [inputHotkey, setInputHotkey] = useState("Ctrl+Alt+I");
  const [ocrLang, setOcrLang] = useState("auto");
  const [saveError, setSaveError] = useState("");
  const [saved, setSaved] = useState(false);
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [updateError, setUpdateError] = useState("");
  const [checked, setChecked] = useState(false); // 是否检查过更新（用于显示"已是最新"）
  const [copied, setCopied] = useState<CopiedKind>(""); // 复制成功反馈，1.5s 后复位

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
          setSources(cfg.sources?.length ? cfg.sources : ["youdao", "bing"]);
          setHotkey(cfg.hotkey || "Ctrl+Alt+T");
          setScreenshotHotkey(cfg.screenshot_hotkey || "Ctrl+Alt+S");
          setInputHotkey(cfg.input_hotkey || "Ctrl+Alt+I");
          setOcrLang(cfg.ocr_lang || "auto");
          setAppid(cfg.baidu.appid);
          setSecret(cfg.baidu.secret);
        })
        .catch(() => {
          if (!cancelled && tries > 0) setTimeout(() => load(tries - 1), 300);
        });
    };
    load(10);
    return () => {
      cancelled = true;
    };
  }, []);

  // 保存密钥（独立落盘，只写 baidu 字段）
  const saveBaiduKeys = async () => {
    setSaveError("");
    try {
      await saveBaidu(appid, secret);
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  // 勾选/取消：即时落盘（只写 sources 字段）；至少保留一个源（全不勾会导致翻译没有可用源）
  const toggleSource = (id: string) => {
    const next = sources.includes(id)
      ? sources.filter((s) => s !== id)
      : [...sources, id];
    if (next.length === 0) return;
    setSources(next);
    saveSources(next).catch((e) => setSaveError(String(e)));
  };

  // 拖拽排序：把 from 插到 to 之前
  const moveSource = (from: string, to: string) => {
    if (!from || from === to) return;
    const next = sources.filter((s) => s !== from);
    const at = next.indexOf(to);
    next.splice(at < 0 ? next.length : at, 0, from);
    setSources(next);
    saveSources(next).catch((e) => setSaveError(String(e)));
  };

  const save = async () => {
    setSaved(false);
    setSaveError("");
    try {
      // 只写快捷键 + OCR 语言；密钥/翻译源各由自己的模块命令落盘
      await saveSettings({
        hotkey,
        screenshot_hotkey: screenshotHotkey,
        input_hotkey: inputHotkey,
        ocr_lang: ocrLang,
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
            onClick={() => setSection(s.id)}
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
                勾选与排序
              </button>
              <button className={"sub-tab" + (srcTab === "config" ? " active" : "")} onClick={() => setSrcTab("config")}>
                源配置
              </button>
            </div>
            {srcTab === "list" ? (
              <>
                <div className="group-heading">翻译源（勾选参与翻译的源，拖拽调整优先级）</div>
                <div className="source-list">
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
                          <input type="checkbox" checked onChange={() => toggleSource(id)} />
                          {item.label}
                        </label>
                      </div>
                    );
                  })}
                  {SOURCE_ITEMS.filter((s) => !sources.includes(s.id)).map((s) => (
                    <div key={s.id} className="source-item off">
                      <span className="drag-handle" />
                      <label className="switch">
                        <input type="checkbox" checked={false} onChange={() => toggleSource(s.id)} />
                        {s.label}
                      </label>
                    </div>
                  ))}
                </div>
                <div className="field-hint">
                  顺序即优先级：第一个源也是单源快速翻译的首选，对比页按同一顺序展示；至少保留一个源。勾选百度需先在「源配置」里填密钥。
                </div>
              </>
            ) : (
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
                  <button className="save-btn" onClick={saveBaiduKeys}>
                    保存密钥
                  </button>
                </div>
                <div className="field-hint">
                  {appid.trim() && secret.trim()
                    ? "已配置：在「勾选与排序」里勾选「百度翻译」后即参与翻译与对比"
                    : "密钥可留空；留空时百度翻译不可用"}
                </div>
              </>
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
            <div className="field-hint">点击输入框后按下组合键即可更换，需包含 Ctrl / Alt / Shift 之一；划词、截图、输入翻译三个快捷键不能相同</div>
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

        {section === "about" && (
          <>
            <div className="section-heading">关于</div>
            <div className="about-text">
              <p>
                Wordpeek v{update?.current_version ?? "0.1.5"} — 最小划词翻译
              </p>
              <p>在任意窗口选中单词，按快捷键，鼠标旁弹出翻译卡片。</p>
              <p>设置保存于 %APPDATA%\com.wordpeek.app\config.json</p>
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
                  {copied === "text" ? "已复制 ✓" : "复制推荐语"}
                </button>
                <button
                  className="update-btn"
                  onClick={() => copyShare("link", REPO_URL)}
                >
                  {copied === "link" ? "已复制 ✓" : "复制链接"}
                </button>
              </div>
            </div>
            <div className="update-tip">
              <div className="tip-actions">
              <button
                className="update-btn"
                onClick={() => openUrl(REPO_URL).catch((e) => setSaveError(String(e)))}
              >
                去 GitHub 点 Star ⭐
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
