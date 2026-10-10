import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import type { Selection, ShotResult, WordInfo } from "../types";
import { splitSense } from "../lib/senses";
import ShotCard from "./ShotCard";

/** 划词翻译弹窗：主窗口复用，划词结果与截图结果都在这里展示 */
export default function Popup() {
  const [word, setWord] = useState("");
  const [info, setInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");
  // 截图翻译结果卡片（screenshot-result 事件）；非空时优先展示
  const [shotResult, setShotResult] = useState<ShotResult | null>(null);
  // 复制反馈："text" | "translation" | ""，1.5s 后自动复位
  const [copied, setCopied] = useState("");
  const doCopy = (label: "text" | "translation", text: string) => {
    invoke("copy_text", { text })
      .then(() => {
        setCopied(label);
        setTimeout(() => setCopied(""), 1500);
      })
      .catch(() => {});
  };
  // 原文可编辑副本：用户修掉 OCR 错误换行后可重新翻译
  const [draft, setDraft] = useState("");
  // 用编辑后的文本重新翻译，结果回填卡片（不清空 draft）
  const retranslate = (text: string) => {
    if (!text.trim()) return;
    setShotResult((r) => (r ? { ...r, info: null, error: null } : r));
    invoke<WordInfo>("translate", { text })
      .then((info) => setShotResult((r) => (r ? { ...r, info, error: null } : r)))
      .catch((e) => setShotResult((r) => (r ? { ...r, error: String(e) } : r)));
  };

  useEffect(() => {
    const win = getCurrentWindow();
    let unlistenSelection: (() => void) | undefined;
    let unlistenBlur: (() => void) | undefined;
    let unlistenFocus: (() => void) | undefined;
    let unlistenShot: (() => void) | undefined;

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
        setShotResult(null); // 新的划词到达，退出截图卡片视图
        await win.setPosition(new PhysicalPosition(x, y));
        await win.show();
        await win.setFocus();
        lookup(text.trim());
      });

      // 截图结果回来时主窗口多半隐藏着，show+focus 弹卡片（位置沿用上次划词位置）
      unlistenShot = await listen<ShotResult>("screenshot-result", (e) => {
        setShotResult(e.payload);
        setDraft(e.payload.text ?? ""); // 新一轮结果重置编辑副本
        win.show();
        win.setFocus();
      });

      unlistenBlur = await win.listen("tauri://blur", startHideCheck);
      unlistenFocus = await win.listen("tauri://focus", stopHideCheck);
    };

    setup();
    return () => {
      unlistenSelection?.();
      unlistenBlur?.();
      unlistenFocus?.();
      unlistenShot?.();
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

  // 截图翻译结果优先展示
  if (shotResult) {
    return (
      <ShotCard
        shotResult={shotResult}
        draft={draft}
        onDraftChange={setDraft}
        copied={copied}
        onCopy={doCopy}
        onRetranslate={retranslate}
      />
    );
  }

  return (
    <div className="card">
      {/* 顶部 padding 区是隐形拖拽热区：按住可移动窗口；窗口边缘仍可拉伸 */}
      <div className="drag-strip" data-tauri-drag-region />
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
        <button className="icon-btn gear" title="设置" onClick={() => invoke("open_settings")}>
          ⚙
        </button>
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
