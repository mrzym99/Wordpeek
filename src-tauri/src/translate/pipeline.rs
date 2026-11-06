//! 翻译管线：按勾选顺序逐源尝试，首个成功即返回、全失败才报错；单源 15s 超时兜底保证永远能降级。
//! 能力过滤（supports）与就绪过滤（ready）统一在这里落地，源内不再散落方向判断。

use crate::translate::{AppConfig, WordInfo};
use super::sources::{find_source, source_label};

/// 仅作降级用的免 Key 源：不进「对比各源」列表，避免白名单外多跑几个开放 demo 端点
pub(crate) const FALLBACK_SOURCES: [&str; 1] = ["youdaoai"];

/// 翻译/对比顺序：内置默认源（有道→必应→免 Key 降级源）在前，勾选的额外源在后；内置关闭后只剩额外源
pub(crate) fn build_order(cfg: &AppConfig) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    if cfg.builtin_sources {
        order.push("youdao".into());
        order.push("bing".into());
        for s in FALLBACK_SOURCES {
            order.push(s.into());
        }
    }
    for s in crate::translate::enabled_sources(cfg) {
        if !order.contains(&s) {
            order.push(s);
        }
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

/// 翻译核心：按勾选顺序逐源尝试，首个成功即返回、全失败才报错；to: "zh" | "en"。
/// 供截图翻译管线在无 State 的上下文直接调用。SOURCE_TIMEOUT：单源超时上限，请求层失灵时由运行时层兜底
const SOURCE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

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

/// 单源对比结果：失败的源也返回，由前端逐行展示错误
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SourceResult {
    pub source: String,
    pub label: String,
    pub ok: bool,
    pub senses: Vec<String>,
    pub error: Option<String>,
}

/// 参与对比的源：默认翻译的源排第一，勾选的额外源跟在后面
pub(crate) fn compare_sources(cfg: &AppConfig, to: &str) -> Vec<String> {
    capable_order(cfg, to)
        .into_iter()
        .filter(|s| !FALLBACK_SOURCES.contains(&s.as_str()))
        .collect()
}

/// 并发请求各源并逐源收集结果：整体不失败，单个源出错只影响它自己那一行。
/// include_builtin = false 时只请求勾选的额外源（默认翻译的结果由前端复用主卡片数据）
pub async fn translate_all_sources(
    cfg: &AppConfig,
    text: &str,
    to: &str,
    include_builtin: bool,
) -> Vec<SourceResult> {
    let to = if to == "en" { "en" } else { "zh" };
    // 每源一个异步任务：reqwest 请求并行发出，总耗时 ≈ 最慢的源
    let mut sources = compare_sources(cfg, to);
    if !include_builtin {
        sources.retain(|s| s != "youdao" && s != "bing");
    }
    let handles: Vec<(
        String,
        tauri::async_runtime::JoinHandle<Result<WordInfo, String>>,
    )> = sources
        .into_iter()
        .map(|source| {
            let src = source.clone();
            let query = text.to_string();
            let cfg = cfg.clone(); // AppConfig 很小（几个字符串），克隆进异步任务避免生命周期问题
            let to = to.to_string();
            let handle = tauri::async_runtime::spawn(async move {
                // 密钥缺失不必发请求：try_translate 的 ready 检查直接给出可读原因
                try_translate(&src, &query, &cfg, &to).await
            });
            (source, handle)
        })
        .collect();

    let mut out = Vec::with_capacity(handles.len());
    for (source, handle) in handles {
        // 任务 panic / 被取消也算该源失败，不影响其它行
        let result = match handle.await {
            Ok(r) => r,
            Err(e) => Err(format!("请求异常: {e}")),
        };
        out.push(match result {
            Ok(info) => SourceResult {
                label: source_label(&source).to_string(),
                source,
                ok: true,
                senses: info.senses,
                error: None,
            },
            Err(e) => SourceResult {
                label: source_label(&source).to_string(),
                source,
                ok: false,
                senses: Vec::new(),
                error: Some(e),
            },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use crate::translate::{AppConfig, BaiduConfig, TencentConfig};

    use super::*;

    #[test]
    fn build_order_puts_builtin_first() {
        let mut cfg = AppConfig::default();
        assert_eq!(build_order(&cfg), vec!["youdao", "bing", "youdaoai"]);
        cfg.sources = vec!["baidu".into()];
        assert_eq!(build_order(&cfg), vec!["youdao", "bing", "youdaoai"]);
        cfg.baidu = BaiduConfig {
            appid: "a".into(),
            secret: "b".into(),
        };
        assert_eq!(build_order(&cfg), vec!["youdao", "bing", "youdaoai", "baidu"]);
        cfg.builtin_sources = false;
        assert_eq!(build_order(&cfg), vec!["baidu"]);
        // 内置源关闭且无额外源：空队列（persist 层会拦住这种配置落盘）
        let mut off = AppConfig::default();
        off.builtin_sources = false;
        assert!(build_order(&off).is_empty());
    }

    #[test]
    fn pipeline_filters_by_supports() {
        // 有道词典 jsonapi 仅英→中：译为英文时被 supports 过滤掉，不占降级链
        let cfg = AppConfig::default();
        assert_eq!(capable_order(&cfg, "en"), vec!["bing", "youdaoai"]);
        assert_eq!(compare_sources(&cfg, "en"), vec!["bing"]);
        assert_eq!(capable_order(&cfg, "zh"), vec!["youdao", "bing", "youdaoai"]);
    }

    #[test]
    fn compare_sources_puts_default_translation_first() {
        let mut cfg = AppConfig::default();
        cfg.tencent = TencentConfig {
            secret_id: "id".into(),
            secret_key: "key".into(),
        };
        cfg.sources = vec!["tencent".into()];
        assert_eq!(compare_sources(&cfg, "zh"), vec!["youdao", "bing", "tencent"]);
        cfg.builtin_sources = false;
        assert_eq!(compare_sources(&cfg, "zh"), vec!["tencent"]);
    }

    #[tokio::test]
    async fn compare_reports_each_source_separately() {
        // 内置源关闭且无额外源：不允许出现（persist 会拦），这里直接验证空队列行为
        let mut cfg = AppConfig::default();
        cfg.builtin_sources = false;
        assert!(translate_all_sources(&cfg, "hello", "zh", true).await.is_empty());

        // 内置源关闭、只勾百度但密钥无效：只有百度这一行，且行内报错
        cfg.sources = vec!["baidu".into()];
        cfg.baidu = BaiduConfig {
            appid: "fake".into(),
            secret: "fake".into(),
        };
        let rows = translate_all_sources(
            &cfg,
            "The quick brown fox jumps over the lazy dog",
            "zh",
            true,
        )
        .await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "baidu");
        assert!(!rows[0].ok);
        assert!(rows[0].error.is_some());
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn chinese_input_translates_to_english_via_bing() {
        // 未配置百度：中→英只能走必应（有道不支持该方向）
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
    async fn default_strategy_falls_back_to_bing_without_baidu_keys() {
        // 未配置百度：整句走 有道→必应，最终应拿到非有道来源的译文
        let cfg = AppConfig::default();
        let info = translate_text(&cfg, "The rain in Spain falls mainly on the plain", "zh")
            .await
            .expect("整句应有必应兜底结果");
        assert_eq!(info.source, "bing");
    }
}
