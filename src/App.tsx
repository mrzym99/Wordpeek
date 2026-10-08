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

interface AppConfig {
  source: string;
  hotkey: string;
  baidu: BaiduConfig;
}

interface UpdateStatus {
  available: boolean;
  current_version: string;
  new_version: string | null;
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
  const [hotkey, setHotkey] = useState("Ctrl+Alt+T");
  const [recording, setRecording] = useState(false);
  const [hotkeyError, setHotkeyError] = useState("");
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
      await invoke("save_config", {
        config: {
          source,
          hotkey,
          baidu: { appid: appid.trim(), secret: secret.trim() },
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
              <p>
                Wordpeek v{update?.current_version ?? "0.1.4"} — 最小划词翻译
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
      // Rust 端抓取到选中文本后通过 "selection" 事件发过来；
      // x/y 已在 Rust 端钳制到光标所在屏幕内
      unlistenSelection = await listen<Selection>("selection", async (e) => {
        const { text, x, y } = e.payload;
        await win.setPosition(new PhysicalPosition(x, y));
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

  // 播放有道词典发音，只按单词字符串取音，与当前翻译源无关
  // type=2 美音 / type=1 英音 / type=0 通用 TTS 兜底
  const playVoice = (w: string, type: 1 | 2) => {
    const url = (t: number) =>
      `https://dict.youdao.com/dictvoice?audio=${encodeURIComponent(w)}&type=${t}`;
    const audio = new Audio(url(type));
    // 个别词条缺指定音色录音（接口返回 500），退回通用发音
    audio.onerror = () => {
      audio.onerror = null;
      audio.src = url(0);
      audio.play();
    };
    audio.play();
  };

  return (
    <div className="card">
      {/* 顶部 padding 区是隐形拖拽热区：按住可移动窗口；窗口边缘仍可拉伸 */}
      <div className="drag-strip" data-tauri-drag-region />
      <button className="icon-btn gear" title="设置" onClick={() => invoke("open_settings")}>
        ⚙
      </button>

      <div className="word-head">
        <div className="word">{word}</div>
        {(info?.usphone || info?.ukphone) && (
          <div className="phonetics">
            {/* 点击播放有道词典发音（type=2 美音 / type=1 英音），与翻译源无关 */}
            {info?.usphone && (
              <span
                className="phonetic"
                title="播放美音"
                onClick={() => playVoice(info.word, 2)}
              >
                🔊 美 {info.usphone}
              </span>
            )}
            {info?.ukphone && (
              <span
                className="phonetic"
                title="播放英音"
                onClick={() => playVoice(info.word, 1)}
              >
                🔊 英 {info.ukphone}
              </span>
            )}
          </div>
        )}
        {info && info.exam_types.length > 0 && (
          <div className="badges">
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
