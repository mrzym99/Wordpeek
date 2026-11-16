/** 翻译源唯一登记处：id 与 Rust 侧 translate/sources/ 一致。
 *  结果行标签、设置页勾选项都从这里取，别再各处硬编码「译文」或源名。
 *  句子源默认必应（单词源有道词典内置必用，两者都不可关）；DeepL 是设置页可勾选的增强 */
export const SOURCE_LABELS: Record<string, string> = {
  youdao: "有道词典",
  deepl: "DeepL 翻译",
  bing: "必应翻译",
};

/** 结果行标签：显示这次真正出结果的源；未知源（后端新增但前端没登记）退回 fallback */
export function sourceLabel(source: string, fallback = "译文"): string {
  return SOURCE_LABELS[source] ?? fallback;
}