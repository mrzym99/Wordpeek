//! 必应翻译（免 Key）：Edge 的 translate/translatetext 端点；老的 cn.bing.com/ttranslatev3 对非浏览器一律回 200 + 空 body，已不可用

use super::{TranslateFut, TranslateSource, WEB_UA, http};
use crate::translate::{AppConfig, WordInfo};

pub(crate) struct Bing;

impl TranslateSource for Bing {
    fn translate<'a>(
        &'a self,
        text: &'a str,
        to: &'a str,
        _cfg: &'a AppConfig,
    ) -> TranslateFut<'a> {
        Box::pin(translate_bing(text, to))
    }
}

async fn translate_bing(text: &str, to: &str) -> Result<WordInfo, String> {
    let url = format!(
        "https://edge.microsoft.com/translate/translatetext?from=&to={}&isEnterpriseClient=false",
        bing_target(to)
    );
    // body 是纯字符串数组，一次可译多条；这里一次一条，返回也是同序数组
    let resp: serde_json::Value = http()
        .post(url)
        .header("content-type", "application/json")
        .header("user-agent", WEB_UA)
        .body(serde_json::json!([text]).to_string())
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;

    let senses: Vec<String> = resp
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["translations"][0]["text"].as_str().map(str::to_string))
        .collect();
    if senses.is_empty() {
        return Err("微软未返回译文".into());
    }
    Ok(WordInfo {
        word: text.to_string(),
        source: "bing".into(),
        senses: vec![senses.join(" ")],
        ..Default::default()
    })
}

/// 目标语言 → 各接口的参数写法；非法值一律按中文处理
pub(crate) fn bing_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh-Hans" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bing_target_maps_each_api() {
        assert_eq!(bing_target("zh"), "zh-Hans");
        assert_eq!(bing_target("en"), "en");
        assert_eq!(bing_target("fr"), "zh-Hans");
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn bing_translates_full_sentence() {
        let info = translate_bing("The quick brown fox jumps over the lazy dog", "zh")
            .await
            .expect("必应整句应能翻译");
        assert_eq!(info.source, "bing");
        assert!(!info.senses.is_empty());
        println!("bing result: {:?}", info.senses);
    }
}
