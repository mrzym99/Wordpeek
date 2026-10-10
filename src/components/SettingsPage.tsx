import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, UpdateStatus } from "../types";
import HotkeyRecorder from "./HotkeyRecorder";

type SettingsSection = "translate" | "hotkey" | "shot" | "about";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "translate", label: "翻译" },
  { id: "hotkey", label: "快捷键" },
  { id: "shot", label: "截图翻译" },
  { id: "about", label: "关于" },
];

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
            <label className="field">
              翻译源
              <select value={source} onChange={(e) => setSource(e.target.value)}>
                <option value="youdao">有道词典（免 Key）</option>
                <option value="baidu">百度翻译</option>
              </select>
            </label>
            {source === "baidu" && (
              <>
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
            <div className="field-hint">按截图快捷键框选屏幕区域即可识别并翻译；整句翻译需配置百度密钥</div>
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
                  <span>
                    发现新版本 v{update.new_version}，下载安装完成后将自动重启
                  </span>
                  <button
                    className="update-btn"
                    onClick={installUpdate}
                    disabled={updating}
                  >
                    {updating ? "下载安装中…" : "立即更新"}
                  </button>
                </>
              ) : (
                <>
                  <span>
                    {checked ? "当前已是最新版本。" : ""}
                  </span>
                  <button
                    className="update-btn"
                    onClick={checkUpdate}
                    disabled={checking}
                  >
                    {checking ? "检查中…" : "检查更新"}
                  </button>
                </>
              )}
            </div>
            {updateError && <div className="field-error">{updateError}</div>}
          </>
        )}

        <div className="settings-footer">
          {saveError && <span className="save-error">{saveError}</span>}
          {saved && !saveError && <span className="saved-tip">已保存</span>}
          <button className="save-btn" onClick={save}>
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
