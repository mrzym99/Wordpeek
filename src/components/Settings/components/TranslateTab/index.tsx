import { SOURCE_ITEMS, type ConfigForm } from "../../../../hooks/useConfigForm";
import { SOURCE_LABELS } from "../../../../lib/sourceLabels";
import s from "../../index.module.css";

/** 「翻译」页：单词 / 句子两个分组——有道词典必用（单词），句子主源 DeepL/必应 互斥单选 */
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
        {SOURCE_ITEMS.map(({ id, label }) => (
          <div key={id} className={s.accItem}>
            <div className={s.accHead}>
              <span className={s.accTitle + " " + s.accStatic}>{label}</span>
              <label className={s.switch}>
                {/* 单选：句子主源互斥，必应在链尾兜底，所以永远有一个亮着 */}
                <input
                  type="radio"
                  name="sentence-source"
                  checked={f.sources.includes(id)}
                  onChange={() => f.selectSource(id)}
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
