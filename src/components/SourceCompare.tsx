import type { SourceResult } from "../types";
import SenseList from "./SenseList";
import { CheckIcon, CopyIcon } from "./icons";

interface SourceCompareProps {
  rows: SourceResult[];
  /** 复制反馈标记：命中的行显示对勾 */
  copied: string;
  onCopy: (source: string, text: string) => void;
}

/** 多源对比列表：每个已勾选源一行（顺序与设置页一致），含译文与复制 */
export default function SourceCompare({ rows, copied, onCopy }: SourceCompareProps) {
  return (
    <div className="compare">
      {rows.map((r) => (
        <div className="compare-row" key={r.source}>
          <div className="compare-head">
            <span className="compare-label">{r.label}</span>
            <button
              className={"icon-btn" + (copied === r.source ? " ok" : "")}
              title="复制该源译文"
              disabled={!r.ok}
              onClick={() => onCopy(r.source, r.senses.join("\n"))}
            >
              {copied === r.source ? <CheckIcon /> : <CopyIcon />}
            </button>
          </div>
          {r.ok ? (
            <SenseList senses={r.senses} />
          ) : (
            <div className="compare-error">{r.error}</div>
          )}
        </div>
      ))}
    </div>
  );
}
