import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { UpdateStatus } from "../../../../types";
import { IS_MAC } from "../../../../hooks/useConfigForm";
import s from "../../index.module.css";

/** 「关于」页：版本信息 + 更新检查/安装 */
export default function AboutTab() {
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [updating, setUpdating] = useState(false);
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

  // 下载安装由 Rust 后台线程执行，成功后应用自动重启
  const installUpdate = async () => {
    setUpdating(true);
    setUpdateError("");
    try {
      await invoke("install_update");
    } catch (err) {
      setUpdateError(String(err));
      setUpdating(false);
    }
  };

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
              发现新版本 v{update.new_version}，下载安装完成后将自动重启
            </span>
            <div className={s.tipActions}>
              <button className={s.updateBtn} onClick={installUpdate} disabled={updating}>
                {updating ? "下载安装中…" : "立即更新"}
              </button>
            </div>
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
