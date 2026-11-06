import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { UpdateProgress, UpdateStatus } from "../../../../types";
import { IS_MAC, REPO_URL } from "../../../../hooks/useConfigForm";
import s from "../../index.module.css";

/** 字节数 → 一位数 MB 文本 */
const mb = (bytes: number) => `${(bytes / 1048576).toFixed(1)} MB`;

/** 「关于」页：版本信息 + 更新检查/安装（下载进度由 update-progress 事件驱动） */
export default function AboutTab() {
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [updateError, setUpdateError] = useState("");
  const [checked, setChecked] = useState(false); // 是否检查过更新（用于显示"已是最新"）

  // 进入"关于"分组时读取启动时缓存的检查结果（不发网络请求）
  useEffect(() => {
    invoke<UpdateStatus>("get_update_status")
      .then((s) => {
        setUpdate(s);
        setChecked(true);
      })
      .catch(() => {});
  }, []);

  // 下载进度/失败都由后台线程经事件通知（install_update 命令立即返回）。
  // 失败时后端已把更新放回缓存，重新拉状态恢复「立即更新」按钮
  useEffect(() => {
    let disposed = false;
    const offs: Array<() => void> = [];
    listen<UpdateProgress>("update-progress", (e) => setProgress(e.payload)).then((f) => {
      if (disposed) f();
      else offs.push(f);
    });
    listen<string>("update-failed", (e) => {
      setUpdating(false);
      setProgress(null);
      setUpdateError(`更新下载失败：${e.payload}`);
      invoke<UpdateStatus>("get_update_status")
        .then(setUpdate)
        .catch(() => {});
    }).then((f) => {
      if (disposed) f();
      else offs.push(f);
    });
    return () => {
      disposed = true;
      offs.forEach((f) => f());
    };
  }, []);

  const checkUpdate = async () => {
    setChecking(true);
    setUpdateError("");
    try {
      setUpdate(await invoke<UpdateStatus>("check_update_now"));
      setChecked(true);
    } catch (err) {
      setUpdateError(String(err));
    } finally {
      setChecking(false);
    }
  };

  // 下载安装由 Rust 后台线程执行，成功后应用自动重启；
  // 进度条随事件更新，失败由 update-failed 复位
  const installUpdate = async () => {
    setUpdating(true);
    setProgress(null);
    setUpdateError("");
    try {
      await invoke("install_update");
    } catch (err) {
      setUpdateError(String(err));
      setUpdating(false);
    }
  };

  // 有总量按百分比，未知总量显示已下载量（不定态进度条）
  const pct =
    progress?.total && progress.total > 0
      ? Math.min(100, Math.floor((progress.downloaded / progress.total) * 100))
      : null;

  return (
    <>
      <div className={s.sectionHeading}>关于</div>
      <div className={s.aboutText}>
        <p>
          Wordpeek{update ? ` v${update.current_version}` : ""}
          <span className="app-beta">beta</span> — 最小划词翻译
        </p>
        <p>在任意窗口选中单词，按快捷键，鼠标旁弹出翻译卡片。</p>
        <p>
          设置保存于{" "}
          {IS_MAC
            ? "~/Library/Application Support/com.wordpeek.desktop/config.json"
            : "%APPDATA%\\com.wordpeek.desktop\\config.json"}
        </p>
      </div>
      <div className={s.updateTip}>
        {update?.available ? (
          <>
            <span className={s.tipText}>
              {IS_MAC
                ? `发现新版本 v${update.new_version}，请前往 GitHub Releases 下载安装包`
                : `发现新版本 v${update.new_version}，下载安装完成后将自动重启`}
            </span>
            <div className={s.tipActions}>
              {IS_MAC ? (
                // macOS 没有签名/公证的更新包，不能静默安装：给按钮直接跳 Release 下载页
                <button
                  className={s.updateBtn}
                  onClick={() => openUrl(`${REPO_URL}/releases/latest`).catch(() => {})}
                >
                  前往下载
                </button>
              ) : (
                <button className={s.updateBtn} onClick={installUpdate} disabled={updating}>
                  {updating ? "下载安装中…" : "立即更新"}
                </button>
              )}
            </div>
            {!IS_MAC && updating && progress && (
              <div
                className={s.progressTrack}
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={pct ?? undefined}
              >
                <div
                  className={
                    pct === null ? `${s.progressFill} ${s.progressIndeterminate}` : s.progressFill
                  }
                  style={pct === null ? undefined : { width: `${pct}%` }}
                />
              </div>
            )}
            {!IS_MAC && updating && progress && (
              <div className={s.progressText}>
                {mb(progress.downloaded)}
                {progress.total ? ` / ${mb(progress.total)}（${pct}%）` : ""}
              </div>
            )}
          </>
        ) : (
          <>
            <span className={s.tipText}>
              {checked ? "当前已是最新版本。" : "检查 GitHub Releases 获取新版本。"}
            </span>
            <div className={s.tipActions}>
              <button className={s.updateBtn} onClick={checkUpdate} disabled={checking}>
                {checking ? "检查中…" : "检查更新"}
              </button>
            </div>
          </>
        )}
      </div>
      {updateError && <div className="field-error">{updateError}</div>}
    </>
  );
}
