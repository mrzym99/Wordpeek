//! 翻译管线：按勾选顺序逐源尝试，首个成功即返回、全失败才报错；单源 15s 超时兜底保证永远能降级。
//! 能力过滤（supports）与就绪过滤（ready）统一在这里落地，源内不再散落方向判断。

use crate::translate::{AppConfig, WordInfo};
use super::sources::find_source;

/// 仅作降级用的免 Key 源：不进「大模型翻译」等按需入口，避免白名单外多跑几个开放 demo 端点
pub(crate) const FALLBACK_SOURCES: [&str; 1] = ["youdaoai"];

/// 翻译顺序：有道词典（单词，必用）在前；句子源二选一——选了 GLM 先大模型，
/// 选 DeepL（默认）走 DeepL→必应；免 Key 的有道 AI 永远垫底。
/// GLM 没 key/请求失败时 capable 过滤与逐源尝试自动落回 DeepL/必应，翻译永远可用
pub(crate) fn build_order(cfg: &AppConfig) -> Vec<String> {
    let mut order = vec!["youdao".into()];
    // 勾选的句子源按固定顺序进链；两个全没勾（异常配置）兜底必应
    let has = |id: &str| cfg.sources.iter().any(|s| s == id);
    if has("deepl") {
        order.push("deepl".into());
    }
    if has("bing") || !has("deepl") {
        order.push("bing".into());
    }
    for s in FALLBACK_SOURCES {
        order.push(s.into());
    }
    order
}

/// 能力过滤后的执行顺序：不声明支持该方向的源（如有道词典仅英→中）不进链
fn capable_order(cfg: &AppConfig, to: &str) -> Vec<String> {
    build_order(cfg)
        .into_iter()
        .filter(|s| find_source(s).is_some_and(|src| src.supports("auto", to)))
        .collect()
}

async fn try_translate(
    source: &str,
    text: &str,
    cfg: &AppConfig,
    to: &str,
) -> Result<WordInfo, String> {
    let src = find_source(source).ok_or_else(|| format!("未知翻译源: {source}"))?;
    if !src.ready(cfg) {
        return Err("未配置密钥".into());
    }
    src.translate(text, to, cfg).await
}

/// 翻译核心：按顺序逐源尝试，首个成功即返回、全失败才报错；to: "zh" | "en"。
pub(crate) const SOURCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub async fn translate_text(cfg: &AppConfig, text: &str, to: &str) -> Result<WordInfo, String> {
    let to = if to == "en" { "en" } else { "zh" };
    let order = capable_order(cfg, to);
    if order.is_empty() {
        return Err("没有可用的翻译源：内置源已关闭且未勾选（或未配密钥的）额外源".into());
    }
    let mut errors: Vec<String> = Vec::new();
    for src in order {
        // 每源独立异步超时：底层任务偶发停止被 poll 时请求级超时不会触发，
        // 用 tokio 计时器强制断开，保证永远能降级到下一个源
        let attempt = tokio::time::timeout(SOURCE_TIMEOUT, try_translate(&src, text, cfg, to));
        match attempt.await {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(e)) => {
                errors.push(format!("[{src}] {e}"));
            }
            Err(_) => {
                errors.push(format!("[{src}] 请求超时（{:#?}，已跳过）", SOURCE_TIMEOUT));
            }
        }
    }
    Err(errors.join("；"))
}

#[cfg(test)]
mod tests {
    use crate::translate::AppConfig;

    use super::*;

    #[test]
    fn youdao_is_always_first() {
        // 默认勾选 DeepL+必应：有道 → DeepL → 必应 → 有道AI 垫底
        let cfg = AppConfig::default();
        assert_eq!(build_order(&cfg), vec!["youdao", "deepl", "bing", "youdaoai"]);
        // 只勾必应：DeepL 不进链
        let mut cfg = AppConfig::default();
        cfg.sources = vec!["bing".into()];
        assert_eq!(build_order(&cfg), vec!["youdao", "bing", "youdaoai"]);
        // 两个全没勾（异常配置）：兜底必应
        let mut cfg = AppConfig::default();
        cfg.sources.clear();
        assert_eq!(build_order(&cfg), vec!["youdao", "bing", "youdaoai"]);
    }

    #[test]
    fn capable_order_filters_by_supports_and_ready() {
        let mut cfg = AppConfig::default();
        // 译为英文：有道词典（仅英→中）被 supports 过滤，DeepL 仍是第一主力
        assert_eq!(capable_order(&cfg, "en"), vec!["deepl", "bing", "youdaoai"]);
        // 只勾必应：链里只剩必应 + 兜底
        cfg.sources = vec!["bing".into()];
        assert_eq!(capable_order(&cfg, "zh"), vec!["youdao", "bing", "youdaoai"]);
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn chinese_input_translates_to_english_via_bing() {
        // 中→英只能走必应（有道/DeepL 中文目标方向不可用）
        let cfg = AppConfig::default();
        let info = translate_text(&cfg, "今天天气很好", "en")
            .await
            .expect("中译英应有必应结果");
        assert_eq!(info.source, "bing");
        assert!(!info.senses.is_empty());
        println!("en result: {:?}", info.senses);
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn default_strategy_falls_back_to_bing() {
        // 默认（DeepL/必应链）：整句走 有道→DeepL→必应，最终应拿到必应的译文
        let cfg = AppConfig::default();
        let info = translate_text(&cfg, "The rain in Spain falls mainly on the plain", "zh")
            .await
            .expect("整句应有必应兜底结果");
        assert_eq!(info.source, "bing");
    }
}
