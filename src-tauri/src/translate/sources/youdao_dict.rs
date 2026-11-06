//! 有道词典（免 Key）：dict.youdao.com/jsonapi，单词词条（音标/释义/词形/同义词/考试类型）；
//! 整句依赖已下线的 fanyi 节点，故仅支持英→中方向

use super::{TranslateFut, TranslateSource, http};
use crate::translate::{AppConfig, SynoGroup, WordForm, WordInfo};

pub(crate) struct YoudaoDict;

impl TranslateSource for YoudaoDict {
    /// jsonapi 只能英→中（词典 + 已下线的整句节点），译为其它语言交给降级链
    fn supports(&self, _from: &str, to: &str) -> bool {
        to == "zh"
    }

    fn translate<'a>(
        &'a self,
        text: &'a str,
        _to: &'a str,
        _cfg: &'a AppConfig,
    ) -> TranslateFut<'a> {
        Box::pin(translate_youdao(text))
    }
}

/// tr 节点里的 "l.i" 可能是字符串，也可能是字符串数组
fn tr_texts(l: &serde_json::Value) -> Vec<String> {
    match &l["i"] {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    }
}

async fn translate_youdao(text: &str) -> Result<WordInfo, String> {
    let resp: serde_json::Value = http()
        .get("https://dict.youdao.com/jsonapi")
        .query(&[("client", "deskdict"), ("jsonversion", "2"), ("q", text)])
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;

    let w = &resp["ec"]["word"][0];
    if w.is_null() {
        // 整句/长句没有词条：回落到 fanyi.tran（有道整句翻译节点）
        let tran = resp["fanyi"]["tran"].as_str().unwrap_or_default();
        if !tran.trim().is_empty() {
            return Ok(WordInfo {
                word: text.to_string(),
                source: "youdao".into(),
                senses: vec![tran.to_string()],
                ..Default::default()
            });
        }
        // 有道免费整句接口已下线（fanyi 节点不再返回）：交由上层降级到必应/百度
        return Err("有道不支持整句翻译".into());
    }
    let mut senses = Vec::new();
    for trs in w["trs"].as_array().cloned().unwrap_or_default() {
        for tr in trs["tr"].as_array().cloned().unwrap_or_default() {
            senses.extend(tr_texts(&tr["l"]));
        }
    }
    if senses.is_empty() {
        return Err("未找到释义".into());
    }

    let word_forms = w["wfs"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| {
            Some(WordForm {
                name: x["wf"]["name"].as_str()?.to_string(),
                value: x["wf"]["value"].as_str()?.to_string(),
            })
        })
        .collect();

    // 同义词：syno.synos[].syno { pos, tran, ws[] }
    let synos = resp["syno"]["synos"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|g| &g["syno"])
        .filter_map(|s| {
            Some(SynoGroup {
                pos: s["pos"].as_str().unwrap_or_default().to_string(),
                tran: s["tran"].as_str().unwrap_or_default().to_string(),
                words: s["ws"]
                    .as_array()?
                    .iter()
                    .filter_map(|x| x["w"].as_str().map(str::to_string))
                    .collect(),
            })
        })
        .filter(|g| !g.words.is_empty())
        .collect();

    let exam_types = resp["ec"]["exam_type"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();

    Ok(WordInfo {
        word: w["return-phrase"]["l"]["i"]
            .as_str()
            .or(resp["simple"]["query"].as_str())
            .unwrap_or(text)
            .to_string(),
        source: "youdao".into(),
        usphone: w["usphone"].as_str().map(str::to_string),
        ukphone: w["ukphone"].as_str().map(str::to_string),
        senses,
        word_forms,
        synos,
        exam_types,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youdao_dict_only_supports_chinese_target() {
        // jsonapi 仅英→中：译为英文由 pipeline 的 supports 过滤走降级链
        assert!(YoudaoDict.supports("auto", "zh"));
        assert!(!YoudaoDict.supports("auto", "en"));
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn youdao_returns_entries() {
        let result = translate_youdao("error").await;
        println!("youdao result: {result:?}");
        let info = result.expect("youdao should succeed");
        assert!(!info.senses.is_empty());
        assert!(info.usphone.is_some());
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn youdao_happy_has_synos() {
        let info = translate_youdao("happy").await.expect("should succeed");
        println!("synos: {:?}", info.synos);
        assert!(!info.synos.is_empty());
        assert!(!info.word_forms.is_empty());
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn full_sentence_reports_baidu_guidance() {
        // 有道免费整句接口已下线：有道整句直接报错，由 translate_text 的降级链兜底
        let r = translate_youdao("The quick brown fox jumps over the lazy dog").await;
        let msg = r.expect_err("整句应报引导性错误");
        assert!(msg.contains("整句"), "错误信息应说明整句原因: {msg}");
    }
}
