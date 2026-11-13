import { SOURCE_ITEMS, type ConfigForm } from "../../../../hooks/useConfigForm";
import s from "../../index.module.css";

/** 「翻译」页：单词 / 句子两个分组——有道词典必用（单词），句子源 DeepL/必应 自由勾选 */
export default function TranslateTab({ f }: { f: ConfigForm }) {
  return (
    <>
      <div className={s.groupTitle}>单词</div>
      <div className={s.accList}>
        {/* 有道词典：单词翻译必用，不可关闭 */}
        <div className={s.accItem}>
          <div className={s.accHead}>
            <span className={s.accTitle + " " + s.accStatic}>有道词典（必用）</span>
            <span className={s.accChevronEmpty} />
          </div>
        </div>
      </div>

      <div className={s.groupTitle}>句子</div>
      <div className={s.accList}>
        {SOURCE_ITEMS.map(({ id, label }) => (
          <div key={id} className={s.accItem}>
            <div className={s.accHead}>
              <span className={s.accTitle + " " + s.accStatic}>{label}</span>
              <label className={s.switch}>
                <input
                  type="checkbox"
                  checked={f.sources.includes(id)}
                  onChange={() => f.toggleSource(id)}
                />
              </label>
              <span className={s.accChevronEmpty} />
            </div>
          </div>
        ))}
      </div>
    </>
  );
}
