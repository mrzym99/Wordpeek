import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { PermissionStatus } from "../../../../types";
import { CheckIcon } from "../../../icons";
import s from "../../index.module.css";

/** 跳系统设置面板时用的权限标识（与 Rust 侧 PermissionKind::parse 对应） */
type PermKind = "accessibility" | "screen_capture";

/** 「系统权限」页（仅 macOS）：辅助功能 + 屏幕录制授权入口 */
export default function PermTab() {
  // macOS 系统权限（Windows 恒为已授，本页不渲染）
  const [perms, setPerms] = useState<PermissionStatus | null>(null);
  // 本次打开设置页期间是否有权限从「未授权」变成「已授权」：变了才提示重启
  const [grantedNow, setGrantedNow] = useState(false);
  const prevRef = useRef<PermissionStatus | null>(null);

  // 从系统设置授权回来（窗口重新获得焦点）时再读一次
  useEffect(() => {
    const refresh = () =>
      invoke<PermissionStatus>("check_permissions")
        .then((next) => {
          const prev = prevRef.current;
          if (
            prev &&
            ((!prev.accessibility && next.accessibility) ||
              (!prev.screen_capture && next.screen_capture))
          ) {
            setGrantedNow(true);
          }
          prevRef.current = next;
          setPerms(next);
        })
        .catch(() => {});
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);

  // 按顺序引导：先补第一个缺失项，再补下一个
  const next: PermKind | null = !perms?.accessibility
    ? "accessibility"
    : !perms?.screen_capture
      ? "screen_capture"
      : null;

  return (
    <>
      <div className={s.sectionHeading}>系统权限</div>
      <div className={s.aboutText}>
        <p>
          Wordpeek 需要两项系统权限：划词翻译靠<b>辅助功能</b>模拟 Cmd+C 读取选中文本，
          截图翻译靠<b>屏幕录制</b>截取屏幕像素。
        </p>
        <p>
          请按顺序授权：先<b>辅助功能</b>，再<b>屏幕录制</b>。从系统设置回到本页会自动刷新状态。
        </p>
      </div>
      <PermRow
        label="辅助功能"
        kind="accessibility"
        ok={perms?.accessibility}
        next={next === "accessibility"}
      />
      <PermRow
        label="屏幕录制"
        kind="screen_capture"
        ok={perms?.screen_capture}
        next={next === "screen_capture"}
      />
      {grantedNow && (
        <div className={s.updateTip}>
          <span className={s.tipText}>权限已更新，重启应用后生效（屏幕录制必须重启才生效）</span>
          <div className={s.tipActions}>
            <button className={s.updateBtn} onClick={() => invoke("restart_app").catch(() => {})}>
              重启应用
            </button>
          </div>
        </div>
      )}
    </>
  );
}

function PermRow({
  label,
  kind,
  ok,
  next,
}: {
  label: string;
  kind: PermKind;
  ok: boolean | undefined;
  next: boolean;
}) {
  return (
    <div className={s.updateTip}>
      <span className={s.tipText}>
        {label}：
        {ok ? (
          <span className="i-txt">
            已授权 <CheckIcon />
          </span>
        ) : next ? (
          "未授权（下一步）"
        ) : (
          "未授权"
        )}
      </span>
      {!ok && (
        <div className={s.tipActions}>
          <button
            className={s.updateBtn}
            onClick={() => invoke("open_permission_settings", { kind }).catch(() => {})}
          >
            去授权
          </button>
        </div>
      )}
    </div>
  );
}
