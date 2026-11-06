//! 百度翻译开放平台：需 appid/secret，MD5(appid + q + salt + secret) 签名

use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};

use super::{TranslateFut, TranslateSource, http};
use crate::translate::{AppConfig, WordInfo};

pub(crate) struct Baidu;

impl TranslateSource for Baidu {
    /// 密钥就绪才可用：enabled_sources 据此剥掉没配密钥的勾选
    fn ready(&self, cfg: &AppConfig) -> bool {
        !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty()
    }

    fn translate<'a>(
        &'a self,
        text: &'a str,
        to: &'a str,
        cfg: &'a AppConfig,
    ) -> TranslateFut<'a> {
        Box::pin(translate_baidu(text, &cfg.baidu.appid, &cfg.baidu.secret, to))
    }
}

async fn translate_baidu(
    text: &str,
    appid: &str,
    secret: &str,
    to: &str,
) -> Result<WordInfo, String> {
    // 签名规则：MD5(appid + q + salt + secret)，见百度翻译开放平台文档
    let salt = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string();
    let mut hasher = Md5::new();
    hasher.update(format!("{appid}{text}{salt}{secret}"));
    let sign = format!("{:x}", hasher.finalize());

    // POST 表单而非 GET：整页文本塞 URL 会撞长度限制（百度 q 上限 ~6000 字节）；该端点两种都收
    let resp: serde_json::Value = http()
        .post("https://fanyi-api.baidu.com/api/trans/vip/translate")
        .form(&[
            ("q", text),
            ("from", "auto"),
            ("to", baidu_target(to)),
            ("appid", appid),
            ("salt", salt.as_str()),
            ("sign", sign.as_str()),
        ])
        .send()
        .await
        // 请求体里有 appid+sign，reqwest 错误 Display 默认带完整 URL，必须脱掉
        .map_err(|e| format!("请求失败: {}", e.without_url()))?
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;

    if let Some(code) = resp["error_code"].as_str() {
        let msg = resp["error_msg"].as_str().unwrap_or("");
        return Err(format!("API 错误 {code}: {msg}"));
    }
    let results = resp["trans_result"].as_array().cloned().unwrap_or_default();
    if results.is_empty() {
        return Err("未找到释义".into());
    }
    let senses: Vec<String> = results
        .iter()
        .filter_map(|i| i["dst"].as_str().map(str::to_string))
        .collect();

    Ok(WordInfo {
        word: text.to_string(),
        source: "baidu".into(),
        senses,
        ..Default::default()
    })
}

/// 目标语言 → 各接口的参数写法；非法值一律按中文处理
pub(crate) fn baidu_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baidu_target_maps_each_api() {
        assert_eq!(baidu_target("zh"), "zh");
        assert_eq!(baidu_target("en"), "en");
        assert_eq!(baidu_target("fr"), "zh");
    }

    #[test]
    fn baidu_needs_both_keys() {
        let mut cfg = AppConfig::default();
        assert!(!Baidu.ready(&cfg));
        cfg.baidu.appid = "a".into();
        assert!(!Baidu.ready(&cfg));
        cfg.baidu.secret = "b".into();
        assert!(Baidu.ready(&cfg));
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn baidu_rejects_bad_key() {
        // 错误 key 应返回带信息的 Err（能到达 API 且错误处理正常）
        let r = translate_baidu("error", "dummy-appid", "dummy-secret", "zh").await;
        assert!(r.is_err());
        println!("baidu err: {:?}", r.err());
    }
}
