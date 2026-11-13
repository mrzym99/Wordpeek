//! 智谱 GLM 免费模型翻译：glm-4.7-flash（OpenAI 兼容端点，国内直连，免费用）。
//! 模型若再换代（官方说 GLM-4.5-Flash 下线后请求自动路由到 GLM-4.7-Flash），只需改 MODEL。

use super::{TranslateFut, TranslateSource, http};
use crate::translate::{AppConfig, WordInfo};

const MODEL: &str = "glm-4.7-flash";

pub(crate) struct Glm;

impl TranslateSource for Glm {
    /// API key 就绪才可用：没配 key 时不进翻译链
    fn ready(&self, cfg: &AppConfig) -> bool {
        !cfg.glm.apikey.is_empty()
    }

    fn translate<'a>(
        &'a self,
        text: &'a str,
        to: &'a str,
        cfg: &'a AppConfig,
    ) -> TranslateFut<'a> {
        Box::pin(translate_glm(text, &cfg.glm.apikey, to))
    }
}

async fn translate_glm(text: &str, apikey: &str, to: &str) -> Result<WordInfo, String> {
    let dir = if to == "en" { "英语" } else { "中文" };
    // thinking 关闭：GLM-4.7-Flash 默认开思考模式，翻译要的是快不是推理
    let body = serde_json::json!({
        "model": MODEL,
        "messages": [
            {
                "role": "system",
                "content": "你是翻译引擎。把用户输入翻译成目标语言，只输出译文本身，不要解释、不要加引号、不要输出任何其他内容。"
            },
            {
                "role": "user",
                "content": format!("翻译成{dir}：\n{text}")
            }
        ],
        "temperature": 0.1,
        "thinking": { "type": "disabled" }
    });

    // 免费档并发/频率限得很低，连续划词容易撞「访问繁忙」：退避后重试一次
    let mut last_err = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        }
        let resp: serde_json::Value = match http()
            .post("https://open.bigmodel.cn/api/paas/v4/chat/completions")
            .bearer_auth(apikey)
            .json(&body)
            .send()
            .await
        {
            Ok(r) => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    last_err = format!("响应解析失败: {e}");
                    continue;
                }
            },
            Err(e) => {
                last_err = format!("请求失败: {e}");
                continue;
            }
        };
        if let Some(msg) = resp["error"]["message"].as_str() {
            last_err = format!("智谱接口错误: {msg}");
            // 限流类错误才重试；key 无效等没意义，直接放弃
            if msg.contains("繁忙") || msg.contains("频率") || msg.contains("并发")
                || msg.contains("稍后再试") || msg.contains("1302") || msg.contains("429") {
                continue;
            }
            return Err(last_err);
        }
        let sense = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .trim()
            // 模型偶发用 ``` 或引号包译文，剥掉包裹
            .trim_matches(|c: char| c == '`' || c == '"' || c == '\'' )
            .to_string();
        if sense.is_empty() {
            return Err("智谱未返回译文".into());
        }
        return Ok(WordInfo {
            word: text.to_string(),
            source: "glm".into(),
            senses: vec![sense],
            ..Default::default()
        });
    }
    Err(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glm_needs_apikey() {
        let mut cfg = AppConfig::default();
        assert!(!Glm.ready(&cfg));
        cfg.glm.apikey = "k".into();
        assert!(Glm.ready(&cfg));
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored（需在环境变量里给 key）
    #[tokio::test]
    #[ignore]
    async fn glm_rejects_bad_key() {
        let r = translate_glm("hello", "dummy-key", "zh").await;
        assert!(r.is_err());
        println!("glm err: {:?}", r.err());
    }
}
