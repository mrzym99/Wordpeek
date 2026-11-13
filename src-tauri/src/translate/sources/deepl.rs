//! DeepL 翻译（免 Key）：模拟 DeepL 官方 iOS App 的 oneshot 无状态端点。
//! 该端点是官方 App / Chrome 扩展自己走的匿名翻译路径，比网页版 jsonrpc（已被 WAF 封杀）稳定。
//! 参考实现：DeepLX（github.com/OwO-Network/DeepLX）translate/translate.go。
//!
//! 为什么走系统 curl 子进程而不是 reqwest：DeepL 风控认 TLS 指纹，实测
//! reqwest 的 rustls 和 schannel(native-tls) 指纹一律 429，只有系统 curl
//! （Windows 是 Schannel 指纹的 curl.exe，macOS 是 SecureTransport 的 /usr/bin/curl）能过。
//! 子进程开销约 100ms，划词场景无感。
//! 风险点：DeepL App Store 换版本后 app_version 过期会全 IP 429，届时更新下面三个常量。

use std::process::Stdio;

use super::{TranslateFut, TranslateSource};
use crate::translate::{AppConfig, WordInfo};

const ENDPOINT: &str = "https://oneshot-free.www.deepl.com/v1/translate";
/// 与 iOS 客户端协议保持一致的一组版本常量（UA、x-app-* 头、body 里 app_information 必须互相自洽，
/// 不自洽（如出现未发布的 OS 版本）会 429 且可能临时封 IP）
const APP_VERSION: &str = "26.52";
const APP_BUILD: &str = "5443737";
const OS_VERSION: &str = "26.0";
const USER_AGENT: &str = "DeepL/26.52 CFNetwork/3826.600.41 Darwin/25.0.0";

pub(crate) struct Deepl;

impl TranslateSource for Deepl {
    fn translate<'a>(
        &'a self,
        text: &'a str,
        to: &'a str,
        _cfg: &'a AppConfig,
    ) -> TranslateFut<'a> {
        Box::pin(translate_deepl(text, to))
    }
}

async fn translate_deepl(text: &str, to: &str) -> Result<WordInfo, String> {
    let target_lang = if to == "en" { "en-US" } else { "zh-Hans" };
    let body = serde_json::json!({
        "text": [text],
        "target_lang": target_lang,
        "usage_type": "translate",
        "app_information": {
            "os": "ios",
            "os_version": OS_VERSION,
            "app_version": APP_VERSION,
            "app_build": APP_BUILD,
            "instance_id": pseudo_uuid(),
        }
    });

    // Authorization: None 是字面量——匿名档的认证标记，缺了走另一条服务端分支
    // Windows 的可执行名叫 curl.exe（Win10 1803+ 自带）；macOS 叫 curl，硬写 curl.exe 会 spawn ENOENT
    let mut cmd = tokio::process::Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    cmd.args([
        "-s",
        "--max-time",
        "15",
        "--ssl-no-revoke", // Windows schannel 撤销检查对不可达 CRL 会直接报错
        "-X",
        "POST",
        ENDPOINT,
        "-H",
        "Content-Type: application/json",
        "-H",
        "Authorization: None",
        "-H",
        &format!("User-Agent: {USER_AGENT}"),
        "-H",
        &format!("x-app-os-version: {OS_VERSION}"),
        "-H",
        &format!("x-app-instance-id: {}", pseudo_uuid()),
        "-H",
        &format!("x-app-session-id: {}", pseudo_uuid()),
        "-H",
        "Accept: */*",
        "-H",
        "Accept-Language: en-US,en;q=0.9",
        "-w",
        "\n%{http_code}",
        "-d",
        &body.to_string(),
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null());

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("curl 启动失败: {e}"))?;
    let raw = String::from_utf8_lossy(&output.stdout);
    // 末行是 -w 写的状态码
    let (resp_body, status) = match raw.rsplit_once('\n') {
        Some((b, s)) if s.trim().parse::<u16>().is_ok() => (b, s.trim().parse::<u16>().unwrap()),
        _ => (raw.as_ref(), 0),
    };
    let resp: serde_json::Value = serde_json::from_str(resp_body)
        .map_err(|e| format!("响应解析失败(HTTP {status}): {e}: {resp_body}"))?;

    let sense = resp["translations"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    if sense.is_empty() {
        let msg = resp["message"].as_str().unwrap_or(resp_body);
        return Err(format!("DeepL HTTP {status}: {msg}"));
    }
    Ok(WordInfo {
        word: text.to_string(),
        source: "deepl".into(),
        senses: vec![sense],
        ..Default::default()
    })
}

/// 无 uuid/rand 依赖，用时间戳+pid 喂 xorshift 手搓 v4 格式 UUID（服务端只校验格式；
/// 官方 App 复用同一 instance_id，但子进程每次新起，就地取材即可）
fn pseudo_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ ((std::process::id() as u64) << 32)
        ^ 0x9e3779b97f4a7c15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut b = [0u8; 16];
    for chunk in b.chunks_mut(8) {
        let v = next().to_le_bytes();
        chunk.copy_from_slice(&v[..chunk.len()]);
    }
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 10
    let hex: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        hex[0..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..16].concat()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudo_uuid_is_v4_shaped() {
        let id = pseudo_uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4"); // version nibble
        let c = id.chars().nth(19).unwrap();
        assert!(c == '8' || c == '9' || c == 'a' || c == 'b'); // variant nibble
    }

    // 真网络请求：默认忽略，跑法 cargo test --release -- --ignored deepl
    #[tokio::test]
    #[ignore]
    async fn deepl_oneshot_translates() {
        let r = translate_deepl("The quick brown fox jumps over the lazy dog.", "zh")
            .await
            .expect("deepl oneshot 失败");
        println!("deepl: {:?}", r.senses);
        assert!(!r.senses[0].is_empty());
    }
}
