import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import type { Selection, ShotResult, SourceResult, WordInfo } from "../types";
import SenseList from "./SenseList";
import SourceCompare from "./SourceCompare";
import { CheckIcon, CopyIcon, RefreshIcon } from "./icons";
import { useAutoFitWindow } from "../lib/autofit";

/** 划词/截图统一弹窗：同一张卡片，标题行 = 可编辑原文 + 「对比各源」/设置按钮 */
export default function Popup() {
  // 卡片内容高度上报给 Rust，窗口按内容收缩（防止透明区域吞掉下层窗口的点击）
  const bodyRef = useAutoFitWindow("main");
  const [word, setWord] = useState("");
  // 最近一次实际发起查词的词：改词框失焦时只在真正变化后才重查，避免回车+失焦查两次
  const lastLooked = useRef("");
  const [info, setInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");
  // 多源对比：开启时在卡片底部追加各源译文（不隐藏原有内容）
  const [compareOpen, setCompareOpen] = useState(false);
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  const [compareLoading, setCompareLoading] = useState(false);
  const [compareError, setCompareError] = useState("");
  // 截图翻译结果（screenshot-result 事件）；非空时卡片切到截图模式
  const [shotResult, setShotResult] = useState<ShotResult | null>(null);
  // 截图原文可编辑副本：用户修掉 OCR 错误换行后可重新翻译
  const [draft, setDraft] = useState("");
  // 复制反馈 key：可能是 "text"/"translation" 或某个翻译源名，1.5s 后自动复位
  const [copied, setCopied] = useState("");
  const doCopy = (key: string, text: string) => {
    invoke("copy_text", { text })
      .then(() => {
        setCopied(key);
        setTimeout(() => setCopied(""), 1500);
      })
      .catch(() => {});
  };

  // 划词/截图模式的对比原文：截图用编辑后的 draft，划词用单词
  const inShot = !!shotResult;
  const activeText = (inShot ? draft : word).trim();

  // 一次列出各勾选源的译文（顺序与设置页一致）；不影响卡片原有内容
  const runCompare = async (text: string) => {
    if (!text.trim()) return;
    setCompareLoading(true);
    setRows(null);
    setCompareError("");
    try {
      setRows(await invoke<SourceResult[]>("translate_all", { text, to: "zh" }));
    } catch (err) {
      setCompareError(String(err));
    } finally {
      setCompareLoading(false);
    }
  };

  // 对比各源：展开/收起卡片底部的多源对比区
  const toggleCompare = () => {
    if (compareOpen) {
      setCompareOpen(false);
      return;
    }
    setCompareOpen(true);
    runCompare(activeText);
  };

  // 用编辑后的截图原文重新翻译，结果回填卡片（不清空 draft）
  const retranslate = (text: string) => {
    if (!text.trim()) return;
    setCompareOpen(false); // 原文变了，旧对比结果作废
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
      lastLooked.current = w;
      setWord(w);
      setInfo(null);
      setError("");
      setCompareOpen(false); // 新查词收起旧对比
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
        setShotResult(null); // 新的划词到达，退出截图模式
        setCompareOpen(false);
        await win.setPosition(new PhysicalPosition(x, y));
        await win.show();
        await win.setFocus();
        lookup(text.trim());
      });

      // 截图结果回来时主窗口多半隐藏着：先定位到框选区附近再 show+focus。
      // 卡片弹在选区下方，空间不足放上方，并錨制在虚拟屏内。
      // 用真实窗口尺寸（outerSize，物理像素）錨制，避免估算偏差导致位置漂移
      unlistenShot = await listen<ShotResult>("screenshot-result", async (e) => {
        setShotResult(e.payload);
        setCompareOpen(false); // 新一轮结果收起旧对比
        setDraft(e.payload.text ?? ""); // 新一轮结果重置编辑副本
        const { virtual, mouse, work } = e.payload;
        try {
          // 两步定位：先粗移到鼠标旁——窗口落到目标屏后，Windows 会按该屏缩放
          // 重新解释窗口尺寸，此时再量真实尺寸（outerSize）做钳制，
          // 避免跨屏 DPI 尺寸漂移导致卡片超出边界
          await win.setPosition(new PhysicalPosition(mouse.x + 4, mouse.y + 4));
          const size = await win.outerSize();
          // 钳制在光标所在屏的工作区内（排除任务栏）；拿不到则退回虚拟屏
          const bx = work.width > 0 ? work.x : virtual.x;
          const by = work.height > 0 ? work.y : virtual.y;
          const bw = work.width > 0 ? work.width : virtual.width;
          const bh = work.height > 0 ? work.height : virtual.height;
          // 水平：鼠标居中；垂直：鼠标下方 4px；靠边时整体收进工作区
          const px = Math.max(
            bx + 4,
            Math.min(mouse.x - size.width / 2, bx + bw - size.width - 4)
          );
          const py = Math.max(
            by + 4,
            Math.min(mouse.y + 4, by + bh - size.height - 4)
          );
          await win.setPosition(new PhysicalPosition(Math.round(px), Math.round(py)));
        } catch (err) {
          // 定位失败（权限/取尺寸异常）也要把卡片弹出来，沿用上次位置
          console.error("定位截图卡片失败", err);
        }
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
    lastLooked.current = w;
    setWord(w);
    setInfo(null);
    setError("");
    setCompareOpen(false); // 同义词跳查收起旧对比
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

  // 截图重新翻译进行中：info/error 均空且原文存在
  const shotLoading = inShot && !!shotResult!.text && !shotResult!.info && !shotResult!.error;

  return (
    <div className="card">
      {/* 顶部 padding 区是隐形拖拽热区：按住可移动窗口；窗口边缘仍可拉伸 */}
      <div className="drag-strip" data-tauri-drag-region />
      <div className="card-body" ref={bodyRef}>
      <div className="word-head">
        {inShot ? (
          <div className="word">截图识别结果</div>
        ) : (
          /* 单词可直接编辑：回车或失焦且内容变化时重查 */
          <input
            className="word-input"
            value={word}
            spellCheck={false}
            title="点击可修改单词，回车重新查词"
            onChange={(e) => setWord(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") (e.target as HTMLInputElement).blur();
            }}
            onBlur={() => {
              const w = word.trim();
              if (w && w !== lastLooked.current) lookup(w);
            }}
          />
        )}
        {/* 对比各源 + 设置：固定标题行右上角，两种模式都有 */}
        <div className="head-actions">
          {activeText && (
            <button
              className="text-btn"
              title={compareOpen ? "收起各源译文对比" : "在卡片底部追加各已勾选源的译文"}
              onClick={toggleCompare}
            >
              {compareOpen ? "收起对比" : "对比各源"}
            </button>
          )}
          <button className="icon-btn gear" title="设置" onClick={() => invoke("open_settings")}>
            ⚙
          </button>
        </div>
        {!inShot && (info?.usphone || info?.ukphone) && (
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
        {!inShot && info && info.exam_types.length > 0 && (
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
      {!inShot && !info && !error && <div className="loading">翻译中...</div>}

      {/* 多源对比区：追加在原有内容下方，不遮盖原文/译文 */}
      {compareOpen && (
        <>
          {compareLoading && <div className="loading">对比中…</div>}
          {compareError && <div className="error">{compareError}</div>}
          {rows && rows.length > 0 && (
            <SourceCompare rows={rows} copied={copied} onCopy={doCopy} />
          )}
          {rows && rows.length === 0 && !compareLoading && (
            <div className="error">没有可用的翻译源，请在设置 → 翻译里勾选</div>
          )}
        </>
      )}

      {/* 截图模式：可编辑原文 + 复制/重译 + 译文 */}
      {inShot && shotResult && (
        <>
          {shotResult.text && (
            <>
              <textarea
                className="shot-text"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                rows={3}
                spellCheck={false}
              />
              <div className="shot-actions">
                <button
                  className={"icon-btn" + (copied === "text" ? " ok" : "")}
                  title="复制原文"
                  onClick={() => doCopy("text", draft)}
                >
                  {copied === "text" ? <CheckIcon /> : <CopyIcon />}
                </button>
                <button
                  className={"icon-btn" + (shotLoading ? " spinning" : "")}
                  title="重新翻译"
                  onClick={() => retranslate(draft)}
                >
                  <RefreshIcon />
                </button>
              </div>
            </>
          )}
          {shotLoading && <div className="shot-loading">翻译中…</div>}
          {shotResult.info && (
            <>
              <SenseList senses={shotResult.info.senses} className="shot-senses" />
              <button
                className={"icon-btn" + (copied === "translation" ? " ok" : "")}
                title="复制译文"
                onClick={() => doCopy("translation", shotResult.info!.senses.join("\n"))}
              >
                {copied === "translation" ? <CheckIcon /> : <CopyIcon />}
              </button>
            </>
          )}
        </>
      )}

      {/* 划词模式：词典卡片 */}
      {!inShot && info && (
        <>
          <SenseList senses={info.senses} />

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
    </div>
  );
}
