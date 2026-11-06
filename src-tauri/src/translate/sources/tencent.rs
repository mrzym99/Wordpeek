//! 腾讯云机器翻译（tmt.tencentcloudapi.com TextTranslate，TC3-HMAC-SHA256 签名），单词整句通吃

use std::time::{SystemTime, UNIX_EPOCH};

use hmac::Mac;
use sha2::{Digest, Sha256};

use super::{TranslateFut, TranslateSource, http};
use crate::translate::{AppConfig, TencentConfig, WordInfo};

type HmacSha256 = hmac::Hmac<Sha256>;

pub(crate) struct Tencent;

impl TranslateSource for Tencent {
    /// 密钥就绪才可用：enabled_sources 据此剥掉没配密钥的勾选
    fn ready(&self, cfg: &AppConfig) -> bool {
        !cfg.tencent.secret_id.is_empty() && !cfg.tencent.secret_key.is_empty()
    }

    fn translate<'a>(
        &'a self,
        text: &'a str,
        to: &'a str,
        cfg: &'a AppConfig,
    ) -> TranslateFut<'a> {
        Box::pin(translate_tencent(text, to, &cfg.tencent))
    }
}

async fn translate_tencent(
    text: &str,
    to: &str,
    tc: &TencentConfig,
) -> Result<WordInfo, String> {
    let service = "tmt";
    let host = "tmt.tencentcloudapi.com";
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let date = utc_date(timestamp);

    let payload = serde_json::json!({
        "SourceText": text,
        "Source": "auto",
        "Target": if to == "en" { "en" } else { "zh" },
        "ProjectId": 0,
    })
    .to_string();

    // TC3 签名规范：CanonicalRequest → StringToSign → 派生密钥 → HMAC
    let hashed_payload = hex_str(Sha256::digest(payload.as_bytes()).as_slice());
    let canonical_request = format!(
        "POST\n/\n\ncontent-type:application/json; charset=utf-8\nhost:{host}\nx-tc-action:texttranslate\n\ncontent-type;host;x-tc-action\n{hashed_payload}"
    );
    let hashed_canonical = hex_str(Sha256::digest(canonical_request.as_bytes()).as_slice());
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{timestamp}\n{date}/{service}/tc3_request\n{hashed_canonical}"
    );
    let mac = |key: &[u8], data: &[u8]| -> Vec<u8> {
        let mut m = HmacSha256::new_from_slice(key).expect("HMAC 接受任意长度密钥");
        m.update(data);
        m.finalize().into_bytes().to_vec()
    };
    let k_date = mac(format!("TC3{}", tc.secret_key).as_bytes(), date.as_bytes());
    let k_service = mac(&k_date, service.as_bytes());
    let k_signing = mac(&k_service, b"tc3_request");
    let signature = hex_str(&mac(&k_signing, string_to_sign.as_bytes()));

    let resp: serde_json::Value = http()
        .post(format!("https://{host}/"))
        .header("content-type", "application/json; charset=utf-8")
        .header("host", host)
        .header("x-tc-action", "TextTranslate")
        .header("x-tc-version", "2018-03-21")
        .header("x-tc-region", "ap-guangzhou")
        .header("x-tc-timestamp", timestamp.to_string())
        .header(
            "authorization",
            format!(
                "TC3-HMAC-SHA256 Credential={}/{}/{}/tc3_request, SignedHeaders=content-type;host;x-tc-action, Signature={signature}",
                tc.secret_id, date, service
            ),
        )
        .body(payload)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;
    if let Some(code) = resp["Response"]["Error"]["Code"].as_str() {
        let msg = resp["Response"]["Error"]["Message"].as_str().unwrap_or("");
        return Err(format!("腾讯接口错误 {code}: {msg}"));
    }
    let sense = resp["Response"]["TargetText"].as_str().unwrap_or_default();
    if sense.trim().is_empty() {
        return Err("腾讯未返回译文".into());
    }
    Ok(WordInfo {
        word: text.to_string(),
        source: "tencent".into(),
        senses: vec![sense.to_string()],
        ..Default::default()
    })
}

/// 时间戳（秒）→ UTC 日期 YYYY-MM-DD（TC3 签名按日期分区密钥，不引 chrono 手算）
fn utc_date(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

fn hex_str(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tencent_needs_both_keys() {
        let mut cfg = AppConfig::default();
        assert!(!Tencent.ready(&cfg));
        cfg.tencent.secret_id = "id".into();
        assert!(!Tencent.ready(&cfg));
        cfg.tencent.secret_key = "key".into();
        assert!(Tencent.ready(&cfg));
    }

    #[test]
    fn utc_date_matches_epoch() {
        assert_eq!(utc_date(0), "1970-01-01");
        // 2026-01-01 00:00:00 UTC = 1767225600
        assert_eq!(utc_date(1767225600), "2026-01-01");
    }
}
