import { splitSense } from "../lib/senses";

/** 释义列表：Popup / InputPage 共用，避免样式各写一套 */
export default function SenseList({
  senses,
  className = "",
}: {
  senses: string[];
  className?: string;
}) {
  return (
    <ul className={("senses " + className).trim()}>
      {senses.map((s, i) => {
        const { pos, text } = splitSense(s);
        return (
          <li key={i}>
            {pos && <b className="pos">{pos}</b>}
            {text}
          </li>
        );
      })}
    </ul>
  );
}
