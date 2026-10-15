import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getVersion } from "@tauri-apps/api/app";
import { PhysicalPosition } from "@tauri-apps/api/dpi";
import type { AppConfig, Selection, ShotResult, SourceResult, WordInfo } from "../types";
import SenseList from "./SenseList";
import SourceCompare from "./SourceCompare";
import ContentBox from "./ContentBox";
import SourceBox from "./SourceBox";
import { CheckIcon, CopyIcon } from "./icons";
import { useAutoFitWindow } from "../lib/autofit";

/** 划词/截图统一弹窗：同一张卡片，标题行 = 可编辑原文 + 「对比各源」/设置按钮 */
export default function Popup() {
  // 卡片内容高度上报给 Rust，窗口按内容收缩（防止透明区域吞掉下层窗口的点击）
  const bodyRef = useAutoFitWindow("main", 67, 190); // padding+border 35 + header 32；最小高度防查词时塌成一条，上限交给 Rust 按屏幕钳制
  const [appVersion, setAppVersion] = useState("");
  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);
  const [word, setWord] = useState("");
  // 最近一次实际发起查词的词：改词框失焦时只在真正变化后才重查，避免回车+失焦查两次
  const lastLooked = useRef("");
  // 查词序号：lookup 用它丢弃晚到的旧请求结果，避免快速连查时结果错乱
  const seqRef = useRef(0);
  const [info, setInfo] = useState<WordInfo | null>(null);
  const [error, setError] = useState("");
  // 多源对比：开启时在卡片底部追加各源译文（不隐藏原有内容）
  const [compareOpen, setCompareOpen] = useState(false);
  // 已勾选的额外源个数：0 时隐藏「对比各源」按钮（persist 保证 sources 里
  // 只留密钥就绪的合法源，所以这里不用再判密钥）
  const [extrasCount, setExtrasCount] = useState(-1);
  const [rows, setRows] = useState<SourceResult[] | null>(null);
  // rows 对应的原文：展开对比时原文没变就直接复用缓存，不重新翻译
  const [compareText, setCompareText] = useState("");
  const [compareLoading, setCompareLoading] = useState(false);
  const [compareError, setCompareError] = useState("");
  // 截图翻译结果（screenshot-result 事件）；非空时卡片切到截图模式
  const [shotResult, setShotResult] = useState<ShotResult | null>(null);
  // 截图原文可编辑副本：用户修掉 OCR 错误换行后可重新翻译
  const [draft, setDraft] = useState("");
  // 截图重译的目标语言：auto = 交给后端按汉字/英文词加权检测
  const [shotTo, setShotTo] = useState<"auto" | "en" | "zh">("auto");
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
  // 默认翻译进行中：划词模式 = 还没结果也没报错；截图模式 = shotLoading
  const mainLoading = inShot
    ? !!shotResult!.text && !shotResult!.info && !shotResult!.error
    : !info && !error && activeText !== "";

  // 列出勾选的额外源译文（默认翻译的结果直接复用主卡片数据，不重复请求）
  const runCompare = async (text: string) => {
    if (!text.trim()) return;
    setCompareText(text);
    setCompareLoading(true);
    setRows(null);
    setCompareError("");
    try {
      setRows(
        await invoke<SourceResult[]>("translate_all", { text, to: "auto", includeBuiltin: false })
      );
    } catch (err) {
      setCompareError(String(err));
    } finally {
      setCompareLoading(false);
    }
  };

  // 对比各源：展开/收起卡片底部的多源对比区。
  // 只在第一次（或原文变了）时请求，之后切换是纯显示/隐藏，不销毁结果
  const toggleCompare = () => {
    if (compareOpen) {
      setCompareOpen(false);
      return;
    }
    setCompareOpen(true);
    if (rows === null || compareText !== activeText) runCompare(activeText);
  };

  // 用 ref 持有最新状态：监听器只注册一次，避免每次查词都重建；
  // listen() 是异步的，若 cleanup 先于 resolve 执行，旧监听器会泄漏
  // （泄漏的监听器还持有旧 runCompare 闭包，配置一变就触发僵尸请求）
  const compareState = useRef({ open: compareOpen, text: activeText });
  compareState.current = { open: compareOpen, text: activeText };

  // 设置里改了配置（勾选/排序/密钥）：旧对比结果作废、额外源个数刷新；
  // 对比区正开着就立刻按新配置重拉，收着就只清缓存（下次展开再拉）
  useEffect(() => {
    let disposed = false;
    const refreshExtras = () => {
      invoke<AppConfig>("get_config")
        .then((cfg) => setExtrasCount((cfg.sources ?? []).length))
        .catch(() => {});
    };
    let unlisten: (() => void) | undefined;
    listen("config-changed", () => {
      refreshExtras();
      const { open, text } = compareState.current;
      if (open) runCompare(text);
      else {
        setRows(null);
        setCompareText("");
      }
    }).then((f) => {
      if (disposed) f(); // 晚到的订阅：立刻退订，不留泄漏
      else unlisten = f;
    });
    // 初始加载一次：决定「对比各源」按钮显示与否
    refreshExtras();
    return () => {
      disposed = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 用编辑后的截图原文重新翻译，结果回填卡片（不清空 draft）
  const retranslate = (text: string, to: string = shotTo) => {
    if (!text.trim()) return;
    setCompareOpen(false); // 原文变了，旧对比结果作废
    setShotResult((r) => (r ? { ...r, info: null, error: null } : r));
    const seq = ++seqRef.current; // 连点方向时，慢的旧请求不能覆盖新结果
    invoke<WordInfo>("translate", { text, to })
      .then((info) => {
        if (seqRef.current !== seq) return;
        setShotResult((r) => (r ? { ...r, info, error: null } : r));
      })
      .catch((e) => {
        if (seqRef.current !== seq) return;
        setShotResult((r) => (r ? { ...r, error: String(e) } : r));
      });
  };

  // 切方向顺手按新方向重译当前原文，省一次点「重新翻译」
  const pickTo = (to: "auto" | "en" | "zh") => {
    setShotTo(to);
    retranslate(draft, to);
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
      const seq = ++seqRef.current;
      try {
        // 翻译请求由 Rust 端发出（避免 CORS，且 API 密钥不进前端）
        const res = await invoke<WordInfo>("translate", { text: w });
        if (seqRef.current !== seq) return; // 已有更新的查询，丢弃过期结果
        setInfo(res);
      } catch (err) {
        if (seqRef.current !== seq) return;
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

    // 异步订阅竞态防护：StrictMode 会 挂载→清理→再挂载，
    // listen 的 Promise 若在清理后才 resolve，不判 disposed 的话监听器永远退不掉
    let disposed = false;

    const setup = async () => {
      // Rust 端抓取到选中文本后通过 "selection" 事件发过来；
      // x/y 已在 Rust 端钳制到光标所在屏幕内
      const sel = await listen<Selection>("selection", async (e) => {
        const { text, x, y } = e.payload;
        setShotResult(null); // 新的划词到达，退出截图模式
        setCompareOpen(false);
        await win.setPosition(new PhysicalPosition(x, y));
        await win.show();
        await win.setFocus();
        lookup(text.trim());
      });
      if (disposed) sel();
      else unlistenSelection = sel;

      // 截图结果回来时主窗口多半隐藏着：先定位到框选区附近再 show+focus。
      // 卡片弹在选区下方，空间不足放上方，并錨制在虚拟屏内。
      // 用真实窗口尺寸（outerSize，物理像素）錨制，避免估算偏差导致位置漂移
      const shot = await listen<ShotResult>("screenshot-result", async (e) => {
        setShotResult(e.payload);
        setCompareOpen(false); // 新一轮结果收起旧对比
        setDraft(e.payload.text ?? ""); // 新一轮结果重置编辑副本
        setShotTo("auto"); // 新一轮回到自动检测，不沿用上轮手动方向
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
      if (disposed) shot();
      else unlistenShot = shot;

      const blur = await win.listen("tauri://blur", startHideCheck);
      if (disposed) blur();
      else unlistenBlur = blur;
      const focus = await win.listen("tauri://focus", stopHideCheck);
      if (disposed) focus();
      else unlistenFocus = focus;
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
  // seq 计数保证慢的旧请求不会覆盖新查询的结果
  const lookup = async (w: string) => {
    lastLooked.current = w;
    setWord(w);
    setInfo(null);
    setError("");
    setCompareOpen(false); // 同义词跳查收起旧对比
    const seq = ++seqRef.current;
    try {
      const res = await invoke<WordInfo>("translate", { text: w });
      if (seqRef.current !== seq) return; // 已有更新的查询，丢弃过期结果
      setInfo(res);
    } catch (err) {
      if (seqRef.current !== seq) return;
      setError(String(err));
    }
  };

  // 播放有道词典发音，只按单词字符串取音，与当前翻译源无关。
  // 复用同一个 Audio：每次 new 会各留一份解码器等 GC，高频点发音会堆内存
  // type=2 美音 / type=1 英音 / type=0 通用 TTS 兜底
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const playVoice = (w: string, type: 1 | 2) => {
    const url = (t: number) =>
      `https://dict.youdao.com/dictvoice?audio=${encodeURIComponent(w)}&type=${t}`;
    const audio = audioRef.current ?? new Audio();
    audioRef.current = audio;
    audio.src = url(type);
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

  // 对比区只列勾选的额外源：默认翻译就是上方卡片内容，不重复展示；
  // 主翻译本身就是某个额外源（如默认源关闭时百度兜底）时跳过该行避免重复
  const mainSource = inShot ? shotResult?.info?.source : info?.source;
  const compareRows: SourceResult[] | null = rows?.filter(
    (r) => r.source !== mainSource
  ) ?? null;

  return (
    <div className="card">
      {/* 顶部 padding 区是隐形拖拽热区：按住可移动窗口；窗口边缘仍可拉伸 */}
      <div className="drag-strip" data-tauri-drag-region />
      {/* header 固定不随内容滚动；body 才是滚动区 */}
      <div className="word-head">
        <div className="app-title">
          Wordpeek<span className="app-beta">beta</span>
          {appVersion && <span className="app-version">v{appVersion}</span>}
        </div>
        {/* 对比各源 + 设置：固定标题行右上角，两种模式都有 */}
        <div className="head-actions">
          {activeText && extrasCount !== 0 && (
            <button
              className="text-btn"
              disabled={mainLoading}
              title={
                mainLoading
                  ? "等默认翻译完成后再对比"
                  : compareOpen
                    ? "收起各源译文对比"
                    : "在卡片底部追加各已勾选源的译文"
              }
              onClick={toggleCompare}
            >
              {compareOpen ? "收起对比" : "对比各源"}
            </button>
          )}
          <button className="icon-btn gear" title="设置" onClick={() => invoke("open_settings")}>
            ⚙
          </button>
        </div>
      </div>

      <div className="card-body" ref={bodyRef}>
      {error && <div className="error">{error}</div>}
      {!inShot && !info && !error && <div className="loading">翻译中...</div>}

      {/* 划词模式：有道词典 → 经典排版「单词 + 音标/发音」一行，无原文框；
          其它源（整句翻译降级）→ 保留原文框 */}
      {!inShot && info && info.source === "youdao" && (
        <div className="word-line">
          <input
            className="word-big"
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
          {/* 点击播放有道词典发音（type=2 美音 / type=1 英音），与翻译源无关 */}
          {info.usphone && (
            <span className="phonetic" title="播放美音" onClick={() => playVoice(info.word, 2)}>
              🔊 美 {info.usphone}
            </span>
          )}
          {info.ukphone && (
            <span className="phonetic" title="播放英音" onClick={() => playVoice(info.word, 1)}>
              🔊 英 {info.ukphone}
            </span>
          )}
        </div>
      )}
      {!inShot && info && info.source !== "youdao" && (
        <SourceBox
          label="原文"
          value={word}
          onChange={setWord}
          rows={2}
          onEnter={() => (document.activeElement as HTMLElement | null)?.blur()}
          onBlur={() => {
            const w = word.trim();
            if (w && w !== lastLooked.current) lookup(w);
          }}
        />
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

      {/* 截图模式：可编辑原文 + 复制/重译 + 译文 */}
      {inShot && shotResult && (
        <>
          {shotResult.text && (
            <SourceBox
              label="原文"
              value={draft}
              onChange={setDraft}
              rows={3}
              onEnter={() => retranslate(draft)}
            />
          )}
          {shotResult.text && (
            <div className="box-actions">
              <span className="lang-toggle" role="group" aria-label="目标语言">
                <button
                  className={shotTo === "auto" ? "active" : ""}
                  disabled={shotLoading}
                  title="自动检测：按汉字与英文词比例判定方向"
                  onClick={() => pickTo("auto")}
                >
                  智能
                </button>
                <button
                  className={shotTo === "en" ? "active" : ""}
                  disabled={shotLoading}
                  title="固定译为英文"
                  onClick={() => pickTo("en")}
                >
                  → 英文
                </button>
                <button
                  className={shotTo === "zh" ? "active" : ""}
                  disabled={shotLoading}
                  title="固定译为中文"
                  onClick={() => pickTo("zh")}
                >
                  → 中文
                </button>
              </span>
              <button className="text-btn" disabled={shotLoading} onClick={() => retranslate(draft)}>
                {shotLoading ? "翻译中…" : "重新翻译"}
              </button>
            </div>
          )}
          {shotLoading && <div className="shot-loading">翻译中…</div>}
          {shotResult.info && (
            <ContentBox
              label="译文"
              copied={copied === "translation"}
              onCopy={() => doCopy("translation", shotResult.info!.senses.join("\n"))}
            >
              <SenseList senses={shotResult.info.senses} className="shot-senses" />
            </ContentBox>
          )}
        </>
      )}

      {/* 划词模式：词典卡片 */}
      {/* 划词模式是完整词典卡片（音标/词形/同义词），译文平铺不套灰底框，
          套框反而显得孤立（截图模式只有裸译文才用框） */}
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
      {/* 对比区：放在卡片末尾（额外源跟在词典卡片/截图译文后面） */}
      {compareOpen && (
        <>
          {compareLoading && <div className="loading">对比中…</div>}
          {compareError && <div className="error">{compareError}</div>}
          {compareRows && compareRows.length > 0 && (
            <SourceCompare rows={compareRows} copied={copied} onCopy={doCopy} />
          )}
          {compareRows && compareRows.length === 0 && !compareLoading && (
            <div className="error">没有可对比的额外源，可在设置 → 翻译里勾选</div>
          )}
        </>
      )}

      </div>
    </div>
  );
}
