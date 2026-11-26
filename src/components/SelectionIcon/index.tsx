import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { resolveState } from "../../lib/dotState";
import logo from "../../assets/logo.png";
import s from "./index.module.css";

/** 划词轻提示：透明小窗口，Rust 静置判定通过后 show 并发 dot-state。
 *  一颗应用图标小点：静止态 16px 圆形裁切的小 logo，near=平滑放大成 28px 圆角方块
 *  （CSS 过渡形变，风格与托盘/弹窗的应用图标同源）。点击由 Rust 收起自己并广播
 *  selection 事件，弹窗卡片接着走热键那套流程；guide=一次性引导气泡 */
export default function SelectionIcon() {
  const [near, setNear] = useState(false);
  const [guide, setGuide] = useState(false);
  useEffect(() => {
    const un = listen<{ near: boolean; guide: boolean }>("dot-state", (e) => {
      setNear(e.payload.near);
      setGuide(e.payload.guide);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
  const expanded = resolveState(near) === "expanded";
  return (
    <>
      <button
        type="button"
        className={expanded ? `${s.chip} ${s.expanded}` : s.chip}
        title="翻译选中的文字"
        aria-label="翻译选中的文字"
        onClick={() => {
          // 失败只可能是没有缓存的选区（探测后被系统清掉），关掉就行
          invoke("translate_selection_dot").catch(() => {});
        }}
      >
        <img src={logo} className={s.logo} alt="" draggable={false} />
      </button>
      {guide && <div className={s.guide}>靠近小点翻译，或按快捷键</div>}
    </>
  );
}
