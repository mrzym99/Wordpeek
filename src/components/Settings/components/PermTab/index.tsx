import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { PermissionStatus } from "../../../../types";
import { CheckIcon } from "../../../icons";
import s from "../../index.module.css";

/** 「系统权限」页（仅 macOS）：辅助功能 + 屏幕录制授权入口 */
export default function PermTab() {
  // macOS 系统权限（Windows 恒为已授，本页不渲染）
  const [perms, setPerms] = useState<PermissionStatus | null>(null);

  // 从系统设置授权回来（窗口重新获得焦点）时再读一次
  useEffect(() => {
    const refresh = () =>
      invoke<PermissionStatus>("check_permissions").then(setPerms).catch(() => {});
    refresh();
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, []);

  return (
    <>
      <div className={s.sectionHeading}>系统权限</div>
      <div className={s.aboutText}>
        <p>
          Wordpeek 需要两项系统权限：划词翻译靠<b>辅助功能</b>模拟 Cmd+C 读取选中文本，
          截图翻译靠<b>屏幕录制</b>截取屏幕像素。
        </p>
        <p>授权后请重启应用（修改权限后系统需要重启进程才生效）。</p>
      </div>
      <PermRow
        label="辅助功能"
        ok={perms?.accessibility}
        url="x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
      />
      <PermRow
        label="屏幕录制"
        ok={perms?.screen_capture}
        url="x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
      />
      <div className={s.updateTip}>
        <span className={s.tipText}>勾选完成后重启应用，新权限才会生效</span>
        <div className={s.tipActions}>
          <button className={s.updateBtn} onClick={() => invoke("restart_app").catch(() => {})}>
            重启应用
          </button>
        </div>
      </div>
    </>
  );
}

function PermRow({ label, ok, url }: { label: string; ok: boolean | undefined; url: string }) {
  return (
    <div className={s.updateTip}>
      <span className={s.tipText}>
        {label}：{ok ? (
          <span className="i-txt">
            已授权 <CheckIcon />
          </span>
        ) : (
          "未授权"
        )}
      </span>
      <div className={s.tipActions}>
        <button className={s.updateBtn} onClick={() => openUrl(url).catch(() => {})}>
          打开设置
        </button>
      </div>
    </div>
  );
}
