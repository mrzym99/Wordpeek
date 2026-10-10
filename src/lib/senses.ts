/** 把 "n. 错误，差错" 拆成词性和释义两部分 */
export function splitSense(sense: string): { pos: string; text: string } {
  const m = sense.match(/^([a-z]+\.\s*)?(.*)$/s);
  return { pos: m?.[1]?.trim() ?? "", text: m?.[2] ?? sense };
}
