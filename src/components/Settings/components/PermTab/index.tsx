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
      <div className="field-hint" style={{ marginBottom: 12 }}>
        按顺序授权，从系统设置回到本页会自动刷新状态。
      </div>
      <div className={s.accList}>
        <PermRow
          label="辅助功能"
          hint="读取选中文本（Cmd+C）"
          kind="accessibility"
          ok={perms?.accessibility}
          next={next === "accessibility"}
        />
        <PermRow
          label="屏幕录制"
          hint="截取屏幕像素（截图翻译）"
          kind="screen_capture"
          ok={perms?.screen_capture}
          next={next === "screen_capture"}
        />
      </div>
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
  hint,
  kind,
  ok,
  next,
}: {
  label: string;
  hint: string;
  kind: PermKind;
  ok: boolean | undefined;
  next: boolean;
}) {
  return (
    <div className={`${s.accItem} ${s.permItem}`}>
      <div className={s.accHead}>
        <span className={`${s.accTitle} ${s.accStatic}`}>
          {label}
          <span className={s.permHint}>{hint}</span>
        </span>
        {/* 状态就写在按钮里：已授权 = 不可点的按钮，未授权 = 去授权（下一步那项用强调色） */}
        <button
          className={s.updateBtn + (next ? " " + s.updateBtnNext : "")}
          disabled={!!ok}
          onClick={() => invoke("open_permission_settings", { kind }).catch(() => {})}
        >
          {ok ? (
            <span className="i-txt">
              已授权 <CheckIcon />
            </span>
          ) : (
            "去授权"
          )}
        </button>
      </div>
    </div>
  );
}
