import s from "./index.module.css";

/** 翻译中遮罩：绝对定位盖满最近的定位祖先；pad 在首查无旧内容时垫高卡片 */
export default function LoadingOverlay({
  show,
  pad,
}: {
  show: boolean;
  /** 容器没有旧内容垫高时传 true，撑出最小高度 */
  pad?: boolean;
}) {
  if (!show) return null;
  return (
    <>
      {pad && <div className={s.pad} />}
      <div className={s.overlay} role="status">
        翻译中…
      </div>
    </>
  );
}
