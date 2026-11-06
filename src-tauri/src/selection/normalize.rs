//! 选区文本清洗：划词结果的格式规整（纯函数，单测随行）。
//! PDF/终端复制常带硬换行与连字符断词，直接弹卡片体验很差；这里统一规整。

/// 划词文本清洗。返回 None =「不像真的划词」（空壳拦截）：
/// 全空白（换行/空格/零宽字符堆）不值得弹卡片
pub(crate) fn normalize_selection(raw: &str) -> Option<String> {
    let s = trim_all(raw);
    if s.is_empty() {
        return None;
    }
    let s = merge_hyphen_breaks(&s);
    let s = collapse_soft_breaks(&s);
    let s = collapse_space_runs(&s);
    Some(cap_length(&s))
}

/// 全文 trim：全角空格与零宽空格都视同空白剥掉（边上的 + 全文判断「全空白」）
fn trim_all(s: &str) -> String {
    let mapped = s.replace('\u{3000}', " ").replace('\u{200b}', "");
    mapped.trim().to_string()
}

/// 连字符断词合并（PDF 排版特征）：行尾 `-` 且下一行以字母开头 → 词接回去
/// （`some-\nthing` → `something`，连字符是排版加的，丢弃）。
/// 与真实复合词（well-known 恰好断行）存在不可消歧的歧义，按 PDF 断词优先处理。
/// 只在「字母-连字符-换行-字母」这个极窄模式动手，普通行内连字符完全不受影响
fn merge_hyphen_breaks(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let hyphen_break = chars[i] == '-'
            && i > 0
            && chars[i - 1].is_alphabetic()
            && i + 2 < chars.len()
            && chars[i + 1] == '\n'
            && chars[i + 2].is_alphabetic();
        if hyphen_break {
            i += 2; // 丢掉连字符和换行，字母由下一轮自然接上
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 折叠硬换行：单个 `\n`（PDF/终端复制特征）→ 空格；连续 2 个以上保留为段落分隔（原样保留）
fn collapse_soft_breaks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            let mut run = 1;
            while chars.peek() == Some(&'\n') {
                chars.next();
                run += 1;
            }
            if run >= 2 {
                // 段落分隔原样保留（2 个换行就是 2 个）
                for _ in 0..run {
                    out.push('\n');
                }
            } else {
                out.push(' ');
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// 折叠 3 个以上连续空格 → 1 个（对齐表格/代码缩进的复制噪声；1-2 个可能是缩进语义，保留）
fn collapse_space_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space_run = 0usize;
    let flush = |out: &mut String, run: &mut usize| {
        if *run == 0 {
            return;
        }
        if *run >= 3 {
            out.push(' ');
        } else {
            for _ in 0..*run {
                out.push(' ');
            }
        }
        *run = 0;
    };
    for c in s.chars() {
        if c == ' ' {
            space_run += 1;
        } else {
            flush(&mut out, &mut space_run);
            out.push(c);
        }
    }
    flush(&mut out, &mut space_run); // 尾部空格
    out
}

/// 长度上限：防整页拖选把卡片和翻译链路拖死
fn cap_length(s: &str) -> String {
    const MAX_CHARS: usize = 5000;
    if s.chars().count() <= MAX_CHARS {
        return s.to_string();
    }
    s.chars().take(MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_selections_are_rejected() {
        assert!(normalize_selection("").is_none());
        assert!(normalize_selection("   \n\t \u{3000}").is_none());
        assert!(normalize_selection("\u{200b}\n").is_none());
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(normalize_selection("hello world").as_deref(), Some("hello world"));
        // 中英混排与标点不被动过
        assert_eq!(
            normalize_selection("Rust 语言！").as_deref(),
            Some("Rust 语言！")
        );
    }

    #[test]
    fn pdf_hyphenated_line_breaks_merge_back() {
        assert_eq!(
            normalize_selection("some-\nthing important").as_deref(),
            Some("something important")
        );
        // 与真实复合词（well-known 恰好断行）的歧义按 PDF 断词优先：连字符丢弃
        assert_eq!(
            normalize_selection("well-\nknown brand").as_deref(),
            Some("wellknown brand")
        );
        // 行内连字符完全不受影响
        assert_eq!(
            normalize_selection("well-known brand").as_deref(),
            Some("well-known brand")
        );
    }

    #[test]
    fn single_newline_becomes_space_double_stays_paragraph() {
        // 终端/PDF 的单换行折叠成空格
        assert_eq!(
            normalize_selection("first line\nsecond line").as_deref(),
            Some("first line second line")
        );
        // 连续两个换行 = 段落分隔，保留
        assert_eq!(
            normalize_selection("para one\n\npara two").as_deref(),
            Some("para one\n\npara two")
        );
    }

    #[test]
    fn wide_space_runs_collapse_but_indent_kept() {
        // 表格复制的对齐空格（3 个以上）折叠
        assert_eq!(
            normalize_selection("name    value").as_deref(),
            Some("name value")
        );
        // 1-2 个空格可能是缩进语义，保留
        assert_eq!(normalize_selection("a  b").as_deref(), Some("a  b"));
        // 全角空格视同空格参与折叠与 trim
        assert_eq!(normalize_selection("\u{3000}x\u{3000}").as_deref(), Some("x"));
    }

    #[test]
    fn overlong_selection_is_truncated() {
        let raw = "ab".repeat(4000); // 8000 字符
        let out = normalize_selection(&raw).unwrap();
        assert_eq!(out.chars().count(), 5000);
        assert!(out.starts_with("ab"));
    }
}
