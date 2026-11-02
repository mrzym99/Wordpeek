import { invoke } from "@tauri-apps/api/core";
import logo from "../../assets/logo.png";
import s from "./index.module.css";

/** 划词图标：独立的透明小窗口，选中文字后出现在光标旁。
 *  点击由 Rust 端收起自己并广播 selection 事件，弹窗卡片接着走热键那套流程 */
export default function SelectionIcon() {
  return (
    <button
      type="button"
      className={s.icon}
      title="翻译选中的文字"
      aria-label="翻译选中的文字"
      onClick={() => {
        // 失败只可能是没有缓存的选区（探测后被系统清掉），关掉就行
        invoke("translate_selection_icon").catch(() => {});
      }}
    >
      <img src={logo} alt="" draggable={false} />
    </button>
  );
}
