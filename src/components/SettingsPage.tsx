import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AppConfig, UpdateStatus } from "../types";
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

/** 复制反馈标记："text" 推荐语 | "link" 链接 | "" 无 */
type CopiedKind = "" | "text" | "link";

/** 独立设置窗口：左侧菜单切换分组，用户手动关闭，不会被 blur 隐藏 */
export default function SettingsPage() {
  const [section, setSection] = useState<SettingsSection>("translate");
  const [source, setSource] = useState("youdao");
  const [appid, setAppid] = useState("");
  const [secret, setSecret] = useState("");
  const [hotkey, setHotkey] = useState("Ctrl+Alt+T");
  const [screenshotHotkey, setScreenshotHotkey] = useState("Ctrl+Alt+S");
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
    invoke<AppConfig>("get_config")
      .then((cfg) => {
        setSource(cfg.source);
        setHotkey(cfg.hotkey || "Ctrl+Alt+T");
        setScreenshotHotkey(cfg.screenshot_hotkey || "Ctrl+Alt+S");
        setOcrLang(cfg.ocr_lang || "auto");
        setAppid(cfg.baidu.appid);
        setSecret(cfg.baidu.secret);
      })
      .catch(() => {
        // 读失败就用默认值，不至于打不开设置
      });
  }, []);

  // 保存密钥（不切换主源，配置后仍作为兜底源生效）
  const saveBaiduKeys = async () => {
    setSaveError("");
    try {
      await patchConfig({ baidu: { appid: appid.trim(), secret: secret.trim() } });
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  // 以后端当前配置为基底，只改指定字段后整体写回——避免把表单里未保存的编辑一并落盘
  const patchConfig = async (patch: Partial<AppConfig>) => {
    const cfg = await invoke<AppConfig>("get_config");
    await invoke("save_config", { config: { ...cfg, ...patch } });
  };

  // 免 Key 源是默认行为，百度是否作主源由 baiduEnabled 表达（source 字段仅后端路由用）
  const baiduEnabled = source === "baidu";

  // 启用百度：保存密钥并设为主源（需密钥非空）
  const enableBaidu = async () => {
    setSaveError("");
    try {
      await patchConfig({
        source: "baidu",
        baidu: { appid: appid.trim(), secret: secret.trim() },
      });
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  // 关闭百度：仅切回默认免 Key 源，密钥保留作兜底
  const disableBaidu = async () => {
    setSaveError("");
    try {
      await patchConfig({ source: "youdao" });
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  const save = async () => {
    setSaved(false);
    setSaveError("");
    try {
      if (hotkey.trim().toLowerCase() === screenshotHotkey.trim().toLowerCase()) {
        setSaveError("划词与截图快捷键不能相同");
        return;
      }
      await invoke("save_config", {
        config: {
          source,
          hotkey,
          screenshot_hotkey: screenshotHotkey,
          ocr_lang: ocrLang,
          baidu: { appid: appid.trim(), secret: secret.trim() },
        },
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
            <div className="section-heading">翻译</div>
            <div className="field-hint">
              默认源：有道词典（单词）+ 必应翻译（整句），免 Key 开箱即用。百度翻译为可选项：启用后全部走百度；仅配置未启用时作为兜底源；未配置则不使用。
            </div>
            <div className="group-heading">百度翻译（可选）</div>
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
              <label className={"switch" + (baiduEnabled ? " on" : "")}>
                <input
                  type="checkbox"
                  checked={baiduEnabled}
                  disabled={!appid.trim() || !secret.trim()}
                  onChange={(e) =>
                    e.target.checked ? enableBaidu() : disableBaidu()
                  }
                />
                启用为主源
              </label>
            </div>
            <div className="field-hint">
              {appid.trim() && secret.trim()
                ? "已配置：即使不作主源，也会作为免 Key 源失败后的降级"
                : "密钥可留空；留空时仅使用免 Key 源"}
            </div>
          </>
        )}

        {section === "hotkey" && (
          <>
            <div className="section-heading">快捷键</div>
            <label className="field">
              划词快捷键
              <HotkeyRecorder value={hotkey} onChange={setHotkey} />
            </label>
            <div className="field-hint">点击输入框后按下组合键即可更换，需包含 Ctrl / Alt / Shift 之一</div>
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
