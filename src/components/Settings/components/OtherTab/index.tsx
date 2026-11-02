import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CheckIcon, StarIcon } from "../../../icons";
import { REPO_URL, type ConfigForm } from "../../../../hooks/useConfigForm";
import s from "../../index.module.css";

/** 「其他」页：推荐语 / Star / Issue 反馈 */
export default function OtherTab({ f }: { f: ConfigForm }) {
  // 复制反馈标记："text" 推荐语 | "link" 链接 | "" 无
  const [copied, setCopied] = useState<"" | "text" | "link">("");

  // 复制推荐语/链接；成功后按钮短暂显示"已复制 ✓"
  const copyShare = (kind: Exclude<"" | "text" | "link", "">, text: string) => {
    invoke("copy_text", { text })
      .then(() => {
        setCopied(kind);
        setTimeout(() => setCopied(""), 1500);
      })
      .catch((e) => f.setSaveError(String(e)));
  };

  return (
    <>
      <div className={s.sectionHeading}>向我推荐</div>
      <div className={s.aboutText}>
        <p>如果 Wordpeek 对你有帮助，欢迎推荐给身边的同学和同事。</p>
        <p>项目完全开源、免费，GitHub 搜索 mrzym99/Wordpeek：</p>
        <p>{REPO_URL}</p>
        <p>去 GitHub 点个 Star，或分享下面的推荐语：</p>
      </div>
      <div className={s.updateTip}>
        <span className={s.tipText}>
          推荐试试 Wordpeek——划词/截图即按即译，免 Key 开箱即用。
        </span>
        <div className={s.tipActions}>
          <button
            className={s.updateBtn}
            onClick={() =>
              copyShare(
                "text",
                "推荐试试 Wordpeek——划词/截图即按即译，免 Key 开箱即用。开源地址：" + REPO_URL,
              )
            }
          >
            {copied === "text" ? (
              <span className="i-txt">
                已复制 <CheckIcon />
              </span>
            ) : (
              "复制推荐语"
            )}
          </button>
          <button className={s.updateBtn} onClick={() => copyShare("link", REPO_URL)}>
            {copied === "link" ? (
              <span className="i-txt">
                已复制 <CheckIcon />
              </span>
            ) : (
              "复制链接"
            )}
          </button>
        </div>
      </div>
      <div className={s.updateTip}>
        <div className={s.tipActions}>
          <button
            className={s.updateBtn}
            onClick={() => openUrl(REPO_URL).catch(() => {})}
          >
            <span className="i-txt">
              去 GitHub 点 Star <StarIcon />
            </span>
          </button>
          <button className={s.updateBtn} onClick={() => openUrl(REPO_URL + "/issues").catch(() => {})}>
            提交 Issue 反馈
          </button>
        </div>
      </div>
    </>
  );
}
