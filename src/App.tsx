import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PhysicalPosition } from "@tauri-apps/api/dpi";

interface Selection {
  text: string;
  x: number;
  y: number;
}

interface BaiduConfig {
  appid: string;
  secret: string;
}

interface MSTranslateConfig {
  key: string;
  region: string;
}

interface AppConfig {
  source: string;
  hotkey: string;
  baidu: BaiduConfig;
  microsoft: MSTranslateConfig;
}

interface WordForm {
  name: string;
  value: string;
}

interface SynoGroup {
  pos: string;
  tran: string;
  words: string[];
}

interface WordInfo {
  word: string;
  source: string;
  usphone: string | null;
  ukphone: string | null;
  senses: string[];
  word_forms: WordForm[];
  synos: SynoGroup[];
  exam_types: string[];
}

/** 把 "n. 错误，差错" 拆成词性和释义两部分 */
function splitSense(sense: string): { pos: string; text: string } {
  const m = sense.match(/^([a-z]+\.\s*)?(.*)$/s);
  return { pos: m?.[1]?.trim() ?? "", text: m?.[2] ?? sense };
}

type SettingsSection = "translate" | "hotkey" | "about";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "translate", label: "翻译" },
  { id: "hotkey", label: "快捷键" },
  { id: "about", label: "关于" },
];

/** 独立设置窗口：左侧菜单切换分组，用户手动关闭，不会被 blur 隐藏 */
function SettingsPage() {
  const [section, setSection] = useState<SettingsSection>("translate");
  const [source, setSource] = useState("youdao");
  const [appid, setAppid] = useState("");
  const [secret, setSecret] = useState("");
  const [msKey, setMsKey] = useState("");
  const [msRegion, setMsRegion] = useState("global");
  const [hotkey, setHotkey] = useState("Ctrl+Alt+T");
  const [recording, setRecording] = useState(false);
  const [hotkeyError, setHotkeyError] = useState("");
  const [saveError, setSaveError] = useState("");
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    invoke<AppConfig>("get_config")
      .then((cfg) => {
        setSource(cfg.source);
        setHotkey(cfg.hotkey || "Ctrl+Alt+T");
        setAppid(cfg.baidu.appid);
        setSecret(cfg.baidu.secret);
        setMsKey(cfg.microsoft?.key ?? "");
        setMsRegion(cfg.microsoft?.region || "global");
      })
      .catch(() => {
        // 读失败就用默认值，不至于打不开设置
      });
  }, []);

  const save = async () => {
    setSaved(false);
    setSaveError("");
    try {
      await invoke("save_config", {
        config: {
          source,
          hotkey,
          baidu: { appid: appid.trim(), secret: secret.trim() },
          microsoft: { key: msKey.trim(), region: msRegion.trim() || "global" },
        },
      });
      setSaved(true);
      setTimeout(() => setSaved(false), 1500);
    } catch (err) {
      setSaveError(String(err));
    }
  };

  // 快捷键录制：录制态下在 document 上捕获按键。
  // 用 e.code（物理按键）而非 e.key，避免中文输入法把按键吞成 "Process"
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.code === "Escape") {
        setRecording(false);
        setHotkeyError("");
        return;
      }
      // 修饰键单独按下时先等待完整组合
      if (/^(Control|Alt|Shift|Meta)(Left|Right)?$/.test(e.code)) return;

      const parts: string[] = [];
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.altKey) parts.push("Alt");
      if (e.shiftKey) parts.push("Shift");
      if (e.metaKey) parts.push("Super");

      let keyName: string | null = null;
      if (/^Key[A-Z]$/.test(e.code)) keyName = e.code.slice(3);
      else if (/^Digit[0-9]$/.test(e.code)) keyName = e.code.slice(5);
      else if (/^F([1-9]|1[0-2])$/.test(e.code)) keyName = e.code;

      if (!keyName) {
        setHotkeyError("不支持的按键，可用字母、数字、F1-F12");
        return;
      }
      if (parts.length === 0) {
        setHotkeyError("需要至少一个修饰键（Ctrl / Alt / Shift）");
        return;
      }
      setHotkeyError("");
      setHotkey([...parts, keyName].join("+"));
      setRecording(false);
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [recording]);

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
                <option value="microsoft">微软翻译（Azure，需 Key）</option>
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
            {source === "microsoft" && (
              <>
                <label className="field">
                  订阅密钥
                  <input
                    type="password"
                    value={msKey}
                    placeholder="Azure 翻译器资源的 KEY"
                    onChange={(e) => setMsKey(e.target.value)}
                  />
                </label>
                <label className="field">
                  区域
                  <input
                    value={msRegion}
                    placeholder="global"
                    onChange={(e) => setMsRegion(e.target.value)}
                  />
                </label>
                <div className="field-hint">
                  在 portal.azure.com 创建"翻译器"资源后获取密钥和区域；免费 F0 档每月 200 万字符
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
              <input
                className={recording ? "recording" : ""}
                readOnly
                value={recording ? "" : hotkey}
                placeholder={recording ? "按下新的组合键（Esc 取消）" : ""}
                onClick={() => setRecording(true)}
              />
            </label>
            {hotkeyError && <div className="field-error">{hotkeyError}</div>}
            <div className="field-hint">点击输入框后按下组合键即可更换，需包含 Ctrl / Alt / Shift 之一</div>
          </>
        )}

        {section === "about" && (
          <>
            <div className="section-heading">关于</div>
            <div className="about-text">
              <p>Wordpeek v0.1.0 — 最小划词翻译</p>
              <p>在任意窗口选中单词，按快捷键，鼠标旁弹出翻译卡片。</p>
              <p>设置保存于 %APPDATA%\com.wordpeek.app\config.json</p>
            </div>
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

/** 划词翻译弹窗 */
function Popup() {
  const [word, setWord] = useState("");
  const [info, setInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    const win = getCurrentWindow();
    let unlistenSelection: (() => void) | undefined;
    let unlistenBlur: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;

    const lookup = async (w: string) => {
      setWord(w);
      setInfo(null);
      setError("");
      try {
        // 翻译请求由 Rust 端发出（避免 CORS，且 API 密钥不进前端）
        setInfo(await invoke<WordInfo>("translate", { text: w }));
      } catch (err) {
        setError(String(err));
      }
    };

    // 点击窗外自动隐藏。不能在 blur 时直接 hide：
    // 拖拽/缩放窗口（系统模态循环）开始时也会触发一次 blur，
    // 立即隐藏会中断操作。所以 blur 后延迟复核 isFocused，
    // 窗口实际仍有焦点（拖拽中）就不隐藏。
    let hideCheck: number | undefined;
    const stopHideCheck = () => {
      if (hideCheck !== undefined) {
        clearInterval(hideCheck);
        hideCheck = undefined;
      }
    };
    const startHideCheck = () => {
      if (hideCheck !== undefined) return;
      hideCheck = setInterval(async () => {
        stopHideCheck();
        if (!(await win.isFocused())) {
          win.hide();
        }
      }, 150);
    };

    const setup = async () => {
      // Rust 端抓取到选中文本后通过 "selection" 事件发过来
      unlistenSelection = await listen<Selection>("selection", async (e) => {
        const { text, x, y } = e.payload;
        await win.setPosition(new PhysicalPosition(x + 12, y + 16));
        await win.show();
        await win.setFocus();
        lookup(text.trim());
      });

      unlistenBlur = await win.listen("tauri://blur", startHideCheck);
      unlistenFocus = await win.listen("tauri://focus", stopHideCheck);
    };

    setup();
    return () => {
      unlistenSelection?.();
      unlistenBlur?.();
      unlistenFocus?.();
      stopHideCheck();
    };
  }, []);

  // 点击同义词 / 词形变化：在弹窗内继续查词，不重新定位窗口
  const lookup = async (w: string) => {
    setWord(w);
    setInfo(null);
    setError("");
    try {
      setInfo(await invoke<WordInfo>("translate", { text: w }));
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="card">
      <button className="icon-btn gear" title="设置" onClick={() => invoke("open_settings")}>
        ⚙
      </button>

      {/* 标题区可按住拖动窗口 */}
      <div className="word-head" data-tauri-drag-region>
        <div className="word" data-tauri-drag-region>{word}</div>
        {(info?.usphone || info?.ukphone) && (
          <div className="phonetics" data-tauri-drag-region>
            {info?.usphone && <span>美 {info.usphone}</span>}
            {info?.ukphone && <span>英 {info.ukphone}</span>}
          </div>
        )}
        {info && info.exam_types.length > 0 && (
          <div className="badges" data-tauri-drag-region>
            {info.exam_types.map((t) => (
              <span key={t} className="badge">
                {t}
              </span>
            ))}
          </div>
        )}
      </div>

      {error && <div className="error">{error}</div>}
      {!info && !error && <div className="loading">翻译中...</div>}

      {info && (
        <>
          <ul className="senses">
            {info.senses.map((s, i) => {
              const { pos, text } = splitSense(s);
              return (
                <li key={i}>
                  {pos && <b className="pos">{pos}</b>}
                  {text}
                </li>
              );
            })}
          </ul>

          {info.word_forms.length > 0 && (
            <div className="section">
              <div className="section-title">词形变化</div>
              <div className="word-forms">
                {info.word_forms.map((f, i) => (
                  <span key={i} className="word-form">
                    {f.name} <b onClick={() => lookup(f.value)}>{f.value}</b>
                  </span>
                ))}
              </div>
            </div>
          )}

          {info.synos.map((g, i) => (
            <div key={i} className="section">
              <div className="section-title">
                同义词 {g.pos && <span className="pos-tag">{g.pos}</span>} {g.tran}
              </div>
              <div className="synos">
                {g.words.map((w) => (
                  <span key={w} className="syno" onClick={() => lookup(w)}>
                    {w}
                  </span>
                ))}
              </div>
            </div>
          ))}
        </>
      )}
    </div>
  );
}

export default function App() {
  // 设置窗口加载同一前端页面，用 URL 参数区分
  const isSettingsPage = window.location.search.includes("page=settings");
  return isSettingsPage ? <SettingsPage /> : <Popup />;
}
