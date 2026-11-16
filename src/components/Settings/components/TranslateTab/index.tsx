import { type ConfigForm } from "../../../../hooks/useConfigForm";
import { SOURCE_LABELS } from "../../../../lib/sourceLabels";
import s from "../../index.module.css";

/** 「翻译」页：单词 / 句子两个分组——有道词典必用（单词），句子默认必应，DeepL 为可勾选增强 */
export default function TranslateTab({ f }: { f: ConfigForm }) {
  return (
    <>
      <div className={s.groupTitle}>单词</div>
      <div className={s.accList}>
        {/* 有道词典：单词翻译必用，不可关闭 */}
        <div className={s.accItem}>
          <div className={s.accHead}>
            <span className={s.accTitle + " " + s.accStatic}>{SOURCE_LABELS.youdao}</span>
            <span className={s.accChevronEmpty} />
          </div>
        </div>
      </div>

      <div className={s.groupTitle}>句子</div>
      <div className={s.accList}>
        <div className={s.accItem}>
          <div className={s.accHead}>
            <span className={s.accTitle + " " + s.accStatic}>
              {SOURCE_LABELS.deepl}
              <span className={s.permHint}>免 Key · 有 IP 频控</span>
            </span>
            <label className={s.switch}>
              {/* 勾选 = 句子优先走 DeepL（必应仍在链尾兜底）；不勾 = 只走必应 */}
              <input
                type="checkbox"
                checked={f.sources.includes("deepl")}
                onChange={(e) => f.toggleDeepl(e.target.checked)}
              />
            </label>
            <span className={s.accChevronEmpty} />
          </div>
        </div>
        {/* 必应：默认句子源，同时是整条链的兜底，不可关 */}
        <div className={s.accItem}>
          <div className={s.accHead}>
            <span className={s.accTitle + " " + s.accStatic}>
              {SOURCE_LABELS.bing}
              <span className={s.permHint}>默认 · 兜底</span>
            </span>
            <span className={s.accChevronEmpty} />
          </div>
        </div>
      </div>
    </>
  );
}
