import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import s from "./index.module.css";

/** 屏上回执 HUD：屏幕中央胶囊，1.2s 自动关窗销毁（划词图标切换、启动提示共用）。
 *  重复触发走 hud_text 事件盖文案重计时，最后一条必是最新事件；
 *  新建窗口的首帧文案由挂载后 hud_take_text 领取（Rust 的 emit 早于监听注册） */
export default function Hud() {
  const [text, setText] = useState<string | null>(null);

  useEffect(() => {
    const win = getCurrentWindow();
    let timer = 0;
    const show = (next: string) => {
      setText(next);
      win.show().catch(() => {});
      clearTimeout(timer);
      timer = window.setTimeout(() => {
        win.close().catch(() => {});
      }, 1200);
    };
    invoke<string | null>("hud_take_text")
      .then((t) => {
        if (t !== null) show(t);
      })
      .catch(() => {});
    const un = listen<string>("hud_text", (e) => show(e.payload));
    return () => {
      clearTimeout(timer);
      un.then((f) => f());
    };
  }, []);

  if (text === null) return null;
  return <div className={s.hud}>{text}</div>;
}
