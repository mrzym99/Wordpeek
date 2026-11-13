import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize, PhysicalPosition } from "@tauri-apps/api/dpi";
import { savePopupWidth } from "../../lib/config";
import type { Selection, ShotResult, WordInfo } from "../../types";
import SenseList from "../SenseList";
import DictPop from "../DictPop";
import LoadingOverlay from "../LoadingOverlay";
import s from "./index.module.css";
import ContentBox from "../ContentBox";
import SourceBox from "../SourceBox";
import TranslateBar from "../TranslateBar";
import Phonetic from "../Phonetic";
import logo from "../../assets/logo.png";
import { useAutoFitWindow } from "../../hooks/useAutoFitWindow";
import { focusCard, useCardDismiss } from "../../hooks/useCardDismiss";
import { isSentenceText } from "../../lib/text";
import { sourceLabel } from "../../lib/sourceLabels";

/** 截图卡固定宽度（逻辑像素）：与输入翻译窗口同宽，原文框才装得下 */
const SHOT_CARD_WIDTH = 560;

/** 等结果再 show 的最长等待（仅单词）：比它慢就先弹加载态卡片，避免「划了没反应」；
 *  句子翻译慢，不等，立即 show 出半透明遮罩 */
const SHOW_WAIT_MS = 400;

/** 划词/截图统一弹窗：同一张卡片，标题行 = 可编辑原文 + 「对比各源」按钮 */
export default function Popup() {
  const win = getCurrentWindow();
  // chrome=67（padding+border 35 + header 32）：内容高度上报 Rust 让窗口收缩，
  // 防透明区域吞掉下层点击；min 防查词时塌成一条，上限交给 Rust 按屏幕钳制
  const { setBody, refit } = useAutoFitWindow("main", 67, 214);
  const [word, setWord] = useState("");
  // 最近一次实际查词的词：改词框只在真正变化后才重查，避免回车+失焦查两次
  const lastLooked = useRef("");
  // 查词序号：丢弃晚到的旧请求结果，避免快速连查时结果错乱
  const seqRef = useRef(0);
  const [info, setInfo] = useState<WordInfo | null>(null);
  // 主翻译在途：期间保留上一份内容不卸载，只盖一层遮罩，结果到达后一次性替换（卡片只跳一次）
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cardRef = useRef<HTMLDivElement | null>(null);
  // 小卡锚定/测量容器：溢出底部时计入 scrollHeight，窗口自适应长高包住小卡
  const cardBodyRef = useRef<HTMLDivElement | null>(null);
  // 须 useCallback 稳定引用：内联 ref 回调每次 render 换新，null/el 来回卸载重挂，
  // setBody 跟着翻转 → 无限重渲染
  const bindBody = useCallback(
    (el: HTMLDivElement | null) => {
      cardBodyRef.current = el;
      setBody(el);
    },
    [setBody]
  );
  // 截图翻译结果（screenshot-result 事件）；非空时卡片切到截图模式
  const [shotResult, setShotResult] = useState<ShotResult | null>(null);
  // 截图原文可编辑副本：修掉 OCR 错误换行后可重新翻译
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

  const inShot = !!shotResult;
  const activeText = (inShot ? draft : word).trim();
  // 遮罩 + 统一 busy：两种模式都等结果回来才换内容
  const mainLoading = busy;

  // 用编辑后的截图原文重译，结果回填卡片（不清空 draft）
  const retranslate = (text: string, to: string = shotTo) => {
    if (!text.trim()) return;
    setShotResult((r) => (r ? { ...r, error: null } : r)); // 旧译文留着给遮罩垫高
    const seq = ++seqRef.current; // 连点方向时，慢的旧请求不能覆盖新结果
    setBusy(true);
    invoke<WordInfo>("translate", { text, to })
      .then((info) => {
        if (seqRef.current !== seq) return;
        setShotResult((r) => (r ? { ...r, info, error: null } : r));
      })
      .catch((e) => {
        if (seqRef.current !== seq) return;
        setShotResult((r) => (r ? { ...r, info: null, error: String(e) } : r));
      })
      .finally(() => {
        if (seqRef.current === seq) setBusy(false);
      });
  };

  // 切方向顺手按新方向重译当前原文，省一次点「重新翻译」
  const pickTo = (to: "auto" | "en" | "zh") => {
    setShotTo(to);
    retranslate(draft, to);
  };

  useEffect(() => {
    let unlistenSelection: (() => void) | undefined;
    let unlistenShot: (() => void) | undefined;
    let unlistenResized: (() => void) | undefined;
    let widthTimer: number | undefined;
    let selSeq = 0; // selection 处理器序号（见 sel 回调内注释）
    // 截图模式：卡片固定 560 宽且屏幕居中；退出后回到用户拖出来的宽度（不把 560 写进配置）
    let inShotMode = false;
    let selWidth = 0;

    // 异步订阅竞态防护：StrictMode 会 挂载→清理→再挂载，listen 的 Promise
    // 若在清理后才 resolve，不判 disposed 的话监听器永远退不掉
    let disposed = false;

    const setup = async () => {
      selWidth = (await win.outerSize()).width / (await win.scaleFactor());
      // 只改宽不改高（高度由 refit 按内容决定）；已经是目标宽就不动，避开 resize 事件噪声。
      // 失败（ACL 未授权等）不能阻断后面的 show，否则卡片永远出不来
      const applyWidth = async (logicalW: number) => {
        try {
          const scale = await win.scaleFactor();
          const inner = await win.innerSize();
          if (Math.abs(inner.width / scale - logicalW) < 0.5) return;
          await win.setSize(new LogicalSize(logicalW, inner.height / scale));
        } catch (err) {
          console.error("设置卡片宽度失败", err);
        }
      };

      // x/y 已在 Rust 端钳制到光标所在屏幕内
      const sel = await listen<Selection>("selection", async (e) => {
        // 连续两次划词：旧处理器（跨屏 DPI 轮询最长等 500ms）醒来后
        // 不得把窗口挪回旧位置/覆盖新处理器已 show 的窗口
        const mySel = ++selSeq;
        const stale = () => mySel !== selSeq;
        const { text, x, y } = e.payload;
        setShotResult(null); // 新的划词到达，退出截图模式
        inShotMode = false;
        // 翻译请求先发出（不等定位），与下面的 setPosition/DPI 等待并行跑
        const lookupDone = lookup(text.trim());
        await applyWidth(selWidth);
        // 跨屏 setPosition 触发 WM_DPICHANGED，tao 异步按「保持逻辑尺寸」重设窗口会覆盖
        // refit 的尺寸；scale 变了就等窗口尺寸稳定再 show+refit
        const sf0 = await win.scaleFactor();
        await win.setPosition(new PhysicalPosition(x, y));
        if ((await win.scaleFactor()) !== sf0) {
          let prev = await win.outerSize();
          for (let i = 0; i < 25; i++) {
            await new Promise((r) => setTimeout(r, 20));
            if (stale()) return;
            const cur = await win.outerSize();
            if (cur.width === prev.width && cur.height === prev.height) break;
            prev = cur;
          }
        }
        if (stale()) return;
        // 单词：等结果再显示（结果在 SHOW_WAIT_MS 内回来，卡片带着完整内容 show，零闪烁），
        // 慢了先弹加载态卡片；句子翻译慢，不等待，立即 show，遮罩盖在半透明层上转
        if (!isSentenceText(text.trim())) {
          await Promise.race([lookupDone, new Promise((r) => setTimeout(r, SHOW_WAIT_MS))]);
          if (stale()) return;
        }
        // 隐藏态先把高度量定，show 出来就是最终尺寸
        await refit();
        markShown(); // 记录 show 时刻 + 启动收起轮询（不依赖 blur 送达）
        await win.show();
        await win.setFocus();
        // macOS Accessory 应用 makeKey 偶发不生效：焦点没立起来 blur 通道就整个
        // 失灵，卡片失去「点外面收起」的触发器，多试几次把焦点立起来
        if (!stale()) focusCard();
      });
      if (disposed) sel();
      else unlistenSelection = sel;

      // 截图结果回来时主窗口多半隐藏着：先定位到鼠标旁再 show+focus；
      // 用真实窗口尺寸（outerSize，物理像素）锚制，避免估算偏差导致漂移
      const shot = await listen<ShotResult>("screenshot-result", async (e) => {
        setShotResult(e.payload);
        setDraft(e.payload.text ?? ""); // 新一轮结果重置编辑副本
        setShotTo("auto"); // 新一轮回到自动检测，不沿用上轮手动方向
        const { virtual, work } = e.payload;
        inShotMode = true;
        // 居中基准取光标所在屏的工作区（排除任务栏），拿不到则退回虚拟屏
        const bx = work.width > 0 ? work.x : virtual.x;
        const by = work.height > 0 ? work.y : virtual.y;
        const bw = work.width > 0 ? work.width : virtual.width;
        const bh = work.height > 0 ? work.height : virtual.height;
        // 按当前外框尺寸算居中坐标；宽度/高度变化后再调一次就是同屏内的微调
        const centerAt = async () => {
          const size = await win.outerSize();
          await win.setPosition(
            new PhysicalPosition(
              Math.round(bx + (bw - size.width) / 2),
              Math.round(by + (bh - size.height) / 2)
            )
          );
        };
        try {
          // 整个定位过程都在隐藏态完成：任何中间态都不该露脸（mouse 只用于挑屏，落点直接算居中）
          await win.hide();
          // 先落到截图所在屏（跳屏时 Windows 按该屏缩放重新解释窗口）→ 撑到 560 宽 → 隐藏态量定高度
          await centerAt();
          await applyWidth(SHOT_CARD_WIDTH);
          await new Promise((r) => setTimeout(r, 0));
          await refit();
          // 高度变了，同一屏内再纠一次居中
          await centerAt();
        } catch (err) {
          // 定位失败（权限/取尺寸异常）也要把卡片弹出来，沿用上次位置
          console.error("定位截图卡片失败", err);
        }
        markShown(); // 记录 show 时刻 + 启动收起轮询（不依赖 blur 送达）
        await win.show();
        await win.setFocus();
        // 和选词路径同样的问题：没 await show 就 setFocus，焦点常立不起来，
        // 只能退化成「先点内部再点外部」才收得掉
        await focusCard();
      });
      if (disposed) shot();
      else unlistenShot = shot;

      // 用户拖宽弹窗后防抖落盘；高度动画引发的等宽 resize 被「与上次已存宽度比较」过滤，不频繁写盘
      let savedWidth = 0;
      const resized = await win.onResized(async (e) => {
        if (inShotMode) return; // 截图卡宽度固定 560，不写进 popup_width
        const s = await win.scaleFactor();
        const w = e.payload.width / s;
        selWidth = w;
        if (Math.abs(w - savedWidth) <= 0.5) return;
        if (widthTimer !== undefined) clearTimeout(widthTimer);
        widthTimer = window.setTimeout(() => {
          widthTimer = undefined;
          savedWidth = w;
          savePopupWidth(w).catch(() => {});
        }, 500);
      });
      if (disposed) resized();
      else unlistenResized = resized;
    };

    setup();
    return () => {
      unlistenSelection?.();
      unlistenShot?.();
      unlistenResized?.();
      if (widthTimer !== undefined) clearTimeout(widthTimer);
    };
  }, []);

  // 收起动作：清空会话状态。收起「判定」已抽进 useCardDismiss（300ms 轮询谓词，
  // 纯函数在 lib/cardDismiss.ts），这里只负责执行
  const hideAndReset = useCallback(() => {
    win.hide();
    seqRef.current++;
    lastLooked.current = "";
    setWord("");
    setInfo(null);
    setBusy(false);
    setError("");
    setShotResult(null);
    setDraft("");
    setShotTo("auto");
    setCopied("");
  }, []);

  // 收起轮询：visible ∧ ¬cursorInCard ∧ (blur 语义 ∨ 全局点击语义)，见 §5.1
  const { markShown } = useCardDismiss({ cardRef, onDismiss: hideAndReset });

  // 点击同义词/词形变化：弹窗内继续查词不重新定位；seq 计数丢弃慢的旧请求
  const lookup = async (w: string) => {
    lastLooked.current = w;
    setWord(w);
    setError("");
    const seq = ++seqRef.current;
    setBusy(true);
    try {
      const res = await invoke<WordInfo>("translate", { text: w });
      if (seqRef.current !== seq) return; // 已有更新的查询，丢弃过期结果
      setInfo(res);
    } catch (err) {
      if (seqRef.current !== seq) return;
      setInfo(null); // 失败要清掉旧内容，别让上一词的结果配着报错一起显示
      setError(String(err));
    } finally {
      if (seqRef.current === seq) setBusy(false);
    }
  };


  const shotLoading = inShot && busy;

  return (
    <div className={"card " + s.card} ref={cardRef}>
      {/* 顶部 padding 区是隐形拖拽热区：按住可移动窗口；窗口边缘仍可拉伸 */}
      <div className={s.dragStrip} data-tauri-drag-region />
      <div className={s.wordHead}>
        {/* 品牌独占标题行左侧；版本号留在设置 → 关于，弹窗里只留名字 */}
        <div className={s.appTitle}>
          <img src={logo} alt="" draggable={false} />
          Wordpeek<span className="app-beta">beta</span>
        </div>
      </div>

      <div className="card-body" ref={bindBody}>
      {error && <div className="error">{error}</div>}

      {/* 划词模式：有道词典 → 经典排版「单词 + 音标」一行；其它源（整句翻译降级）→ 保留原文框 */}
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
          <Phonetic word={info.word} usphone={info.usphone} ukphone={info.ukphone} />
        </div>
      )}
      {!inShot && info && info.source !== "youdao" && (
        <SourceBox
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
        <div className={s.badges}>
          {info.exam_types.map((t) => (
            <span key={t} className={s.badge}>
              {t}
            </span>
          ))}
        </div>
      )}

      {inShot && shotResult && (
        <>
          {shotResult.text && (
            <SourceBox
              value={draft}
              onChange={setDraft}
              rows={6}
              onEnter={() => retranslate(draft)}
            />
          )}
          {shotResult.text && (
            <TranslateBar
              to={shotTo}
              onPick={pickTo}
              onRun={() => retranslate(draft)}
              busy={shotLoading}
            />
          )}
          {shotResult.error && <div className="error">{shotResult.error}</div>}
          {shotResult.info && (
            <ContentBox
              label={sourceLabel(shotResult.info.source)}
              copied={copied === "translation"}
              onCopy={() => doCopy("translation", shotResult.info!.senses.join("\n"))}
            >
              <SenseList senses={shotResult.info.senses} className={s.shotSenses} />
            </ContentBox>
          )}
        </>
      )}

      {/* 划词模式：句子套灰框不带行首序号（多行译文里序号像行号），单词仍平铺词典卡片 */}
      {!inShot && info && (
        <>
          {isSentenceText(info.word) ? (
            <ContentBox
              label={sourceLabel(info.source)}
              copied={copied === "translation"}
              onCopy={() => doCopy("translation", info.senses.join("\n"))}
            >
              <SenseList senses={info.senses} className={s.shotSenses} />
            </ContentBox>
          ) : (
            <SenseList senses={info.senses} />
          )}

          {info.word_forms.length > 0 && (
            <div className={s.section}>
              <div className={s.sectionTitle}>词形变化</div>
              <div className={s.wordForms}>
                {info.word_forms.map((f, i) => (
                  <span key={i} className={s.wordForm}>
                    {f.name}{" "}
                    <button type="button" className={s.linkBtn} onClick={() => lookup(f.value)}>
                      {f.value}
                    </button>
                  </span>
                ))}
              </div>
            </div>
          )}

          {info.synos.map((g, i) => (
            <div key={i} className={s.section}>
              <div className={s.sectionTitle}>
                同义词 {g.pos && <span className={s.posTag}>{g.pos}</span>} {g.tran}
              </div>
              <div className={s.synos}>
                {g.words.map((w) => (
                  <button
                    key={w}
                    type="button"
                    className={s.syno}
                    onClick={() => lookup(w)}
                  >
                    {w}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </>
      )}
      {/* 翻译中遮罩：盖住 card-body，旧内容留在下面，结果回来才替换；
          首查（句子/新词）无旧内容可垫，由 pad 撑起卡片高度 */}
      <LoadingOverlay show={busy} pad={!info && !shotResult} />

      {/* 划词小释义卡：fixed 定位挂在视口上，不参与窗口自适应的 scrollHeight 测量 */}
      <DictPop
        rootRef={cardRef}
        resetKey={`${word}|${shotResult?.text ?? ""}`}
      />
      </div>
    </div>
  );
}
