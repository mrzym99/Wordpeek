/** 是否像整句/短语而非单个词：含空白或常见标点，或中文超过 2 字 */
export function isSentenceText(t: string) {
  const s = t.trim();
  return /[\s，。！？；,.!?;:]/.test(s) || (s.match(/[\u4e00-\u9fff]/g)?.length ?? 0) > 2;
}
