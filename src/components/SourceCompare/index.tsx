import type { ReactNode } from "react";
import s from "./index.module.css";
import type { SourceResult } from "../../types";
import SenseList from "../SenseList";
import { CheckIcon, CopyIcon } from "../icons";

/** 对比行：额外源就是纯 SourceResult；默认翻译行可附单词音标行（仅 InputPage 用） */
export type CompareRow = SourceResult & { wordLine?: ReactNode };

interface SourceCompareProps {
  rows: CompareRow[];
  /** 复制反馈标记：命中的行显示对勾 */
  copied: string;
  onCopy: (source: string, text: string) => void;
}

/** 多源对比列表：每个已勾选源一行（顺序与设置页一致），含译文与复制 */
export default function SourceCompare({ rows, copied, onCopy }: SourceCompareProps) {
  return (
    <div className={s.list}>
      {rows.map((r) => (
        <div className={s.row} key={r.source}>
          <div className={s.head}>
            <span className={s.label}>{r.label}</span>
            <button
              className={"icon-btn" + (copied === r.source ? " ok" : "")}
              title="复制该源译文"
              disabled={!r.ok}
              onClick={() => onCopy(r.source, r.senses.join("\n"))}
            >
              {copied === r.source ? <CheckIcon /> : <CopyIcon />}
            </button>
          </div>
          {r.wordLine}
          {r.ok ? (
            <SenseList senses={r.senses} />
          ) : (
            <div className={s.error}>{r.error}</div>
          )}
        </div>
      ))}
    </div>
  );
}
