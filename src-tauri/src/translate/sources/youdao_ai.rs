//! 有道翻译（免 Key）：有道智云公开 demo 端点，有频率限制，只当降级用

use super::{FALLBACK_TIMEOUT, TranslateFut, TranslateSource, WEB_UA, http};
use crate::translate::{AppConfig, WordInfo};

pub(crate) struct YouDaoAi;

impl TranslateSource for YouDaoAi {
    fn translate<'a>(
        &'a self,
        text: &'a str,
        to: &'a str,
        _cfg: &'a AppConfig,
    ) -> TranslateFut<'a> {
        Box::pin(translate_youdao_ai(text, to))
    }
}

async fn translate_youdao_ai(text: &str, to: &str) -> Result<WordInfo, String> {
    let to = if to == "en" { "en" } else { "zh-CHS" };
    let resp: serde_json::Value = http()
        .post("https://aidemo.youdao.com/trans")
        .header("user-agent", WEB_UA)
        .timeout(FALLBACK_TIMEOUT)
        .form(&[("q", text), ("from", "Auto"), ("to", to)])
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;
    let sense = resp["translation"][0].as_str().unwrap_or_default();
    if sense.trim().is_empty() {
        let code = resp["errorCode"].as_str().unwrap_or("未知");
        return Err(format!("有道接口错误 {code}"));
    }
    Ok(WordInfo {
        word: text.to_string(),
        source: "youdaoai".into(),
        senses: vec![sense.to_string()],
        ..Default::default()
    })
}
