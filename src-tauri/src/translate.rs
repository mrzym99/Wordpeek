use std::path::PathBuf;
use parking_lot::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

/// 百度翻译 API 的鉴权配置
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct BaiduConfig {
    #[serde(default)]
    pub appid: String,
    #[serde(default)]
    pub secret: String,
}

/// 应用配置，持久化在 app_config_dir/config.json
#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// 当前翻译源："youdao" | "baidu"
    #[serde(default = "default_source")]
    pub source: String,
    /// 划词快捷键，如 "Ctrl+Alt+T"
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// 截图翻译快捷键，如 "Ctrl+Alt+S"
    #[serde(default = "default_screenshot_hotkey")]
    pub screenshot_hotkey: String,
    /// 截图 OCR 识别语言："auto" | "zh" | "en"
    #[serde(default = "default_ocr_lang")]
    pub ocr_lang: String,
    #[serde(default)]
    pub baidu: BaiduConfig,
}

fn default_source() -> String {
    "youdao".into()
}

fn default_hotkey() -> String {
    "Ctrl+Alt+T".into()
}

fn default_screenshot_hotkey() -> String {
    "Ctrl+Alt+S".into()
}

fn default_ocr_lang() -> String {
    "auto".into()
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            source: default_source(),
            hotkey: default_hotkey(),
            screenshot_hotkey: default_screenshot_hotkey(),
            ocr_lang: default_ocr_lang(),
            baidu: BaiduConfig::default(),
        }
    }
}

type SharedConfig = Mutex<AppConfig>;

/// 必应翻译的缓存凭证：从 cn.bing.com/translator 首页解析，有效期约 1 小时
#[derive(Clone)]
struct BingToken {
    /// 首页 _G.IG，32 位十六进制
    ig: String,
    /// params_AbusePreventionHelper 的时间戳（key 字段）
    key: String,
    /// params_AbusePreventionHelper 的 token 字段
    token: String,
    /// 过期时刻（毫秒），留 5 分钟余量提前刷新
    expires_at: u64,
}

static BING_TOKEN: parking_lot::Mutex<Option<BingToken>> = parking_lot::Mutex::new(None);

/// 拉取/复用必应翻译凭证；过期前 5 分钟刷新，失败时丢弃旧缓存强制重取
async fn bing_token() -> Result<BingToken, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    if let Some(t) = BING_TOKEN.lock().as_ref() {
        if now < t.expires_at {
            return Ok(BingToken {
                ig: t.ig.clone(),
                key: t.key.clone(),
                token: t.token.clone(),
                expires_at: t.expires_at,
            });
        }
    }

    // 首页 HTML 里包含 IG 与 AbusePrevention 参数，无需正则，直接字符串切片
    let html = reqwest::Client::new()
        .get("https://cn.bing.com/translator")
        .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;

    let ig = html
        .find("IG:\"")
        .map(|i| html[i + 4..i + 36].to_string())
        .ok_or("必应页面缺少 IG")?;
    let ap = html
        .find("params_AbusePreventionHelper")
        .ok_or("必应页面缺少防滥用参数")?;
    let rest = &html[ap..];
    let open = rest.find('[').ok_or("必应防滥用参数格式异常")?;
    let rest = &rest[open + 1..];
    let comma = rest.find(',').ok_or("必应防滥用参数格式异常")?;
    let key = rest[..comma].to_string();
    let q = rest[comma + 2..].find('"').ok_or("必应防滥用参数格式异常")?;
    let token = rest[comma + 2..comma + 2 + q].to_string();

    let tok = BingToken {
        ig,
        key,
        token,
        expires_at: now + 3_600_000 - 300_000,
    };
    *BING_TOKEN.lock() = Some(tok.clone());
    Ok(tok)
}

/// 必应翻译（免 Key）：从 cn.bing.com 取临时凭证后调 ttranslatev3
async fn translate_bing(text: &str) -> Result<WordInfo, String> {
    let t = bing_token().await?;
    let form = [
        ("fromLang", "auto-detect".to_string()),
        ("to", "zh-Hans".to_string()),
        ("text", text.to_string()),
        ("token", t.token),
        ("key", t.key),
    ];
    let url = format!(
        "https://cn.bing.com/ttranslatev3?IG={}&IID=translator.5023",
        t.ig
    );
    let resp: serde_json::Value = reqwest::Client::new()
        .post(url)
        .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .header("referer", "https://cn.bing.com/translator")
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .json()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;

    // 凭证失效（statusCode 非零）时清缓存，让下次调用重取
    if let Some(code) = resp["statusCode"].as_i64() {
        *BING_TOKEN.lock() = None;
        return Err(format!("必应接口错误 {code}"));
    }
    // 长文本会被分段，返回数组每段各含一个 translations
    let items = resp.as_array().cloned().unwrap_or_default();
    let senses: Vec<String> = items
        .iter()
        .filter_map(|i| i["translations"][0]["text"].as_str().map(str::to_string))
        .collect();
    if senses.is_empty() {
        *BING_TOKEN.lock() = None;
        return Err("必应未返回译文".into());
    }
    Ok(WordInfo {
        word: text.to_string(),
        source: "bing".into(),
        senses: vec![senses.join(" ")],
        ..Default::default()
    })
}

/// 词形变化，如 { name: "复数", value: "errors" }
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct WordForm {
    pub name: String,
    pub value: String,
}

/// 按词性分组的同义词
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct SynoGroup {
    pub pos: String,
    pub tran: String,
    pub words: Vec<String>,
}

/// 翻译结果：youdao 填充全部字段，baidu 只填 senses
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct WordInfo {
    pub word: String,
    pub source: String,
    pub usphone: Option<String>,
    pub ukphone: Option<String>,
    /// 含词性的释义，如 "n. 错误，差错"
    pub senses: Vec<String>,
    pub word_forms: Vec<WordForm>,
    pub synos: Vec<SynoGroup>,
    /// 适用考试，如 CET4 / 考研 / IELTS
    pub exam_types: Vec<String>,
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("config.json"))
}

/// 启动时调用：读配置文件，不存在或损坏则用默认值。
/// 不做旧目录迁移：卸载重装后若配置丢失，把远古配置搬回来
/// 反而会覆盖用户的当前设置（如翻译源被复旧为 baidu）。
pub fn load_config(app: &AppHandle) -> AppConfig {
    let raw = config_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok());
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_config(state: State<SharedConfig>) -> AppConfig {
    state.lock().clone()
}

#[tauri::command]
pub fn save_config(
    app: AppHandle,
    state: State<SharedConfig>,
    config: AppConfig,
) -> Result<(), String> {
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写入配置文件失败: {e}"))?;
    *state.lock() = config.clone();
    println!("[config] saved");

    // 快捷键可能被修改：按新配置重新注册全部快捷键（失败仅日志，不阻断保存）
    crate::register_hotkeys(&app, &config);
    Ok(())
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
    let resp: serde_json::Value = reqwest::Client::new()
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

async fn translate_baidu(text: &str, appid: &str, secret: &str) -> Result<WordInfo, String> {
    // 签名规则：MD5(appid + q + salt + secret)，见百度翻译开放平台文档
    let salt = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string();
    let mut hasher = Md5::new();
    hasher.update(format!("{appid}{text}{salt}{secret}"));
    let sign = format!("{:x}", hasher.finalize());

    let resp: serde_json::Value = reqwest::Client::new()
        .get("https://fanyi-api.baidu.com/api/trans/vip/translate")
        .query(&[
            ("q", text),
            ("from", "auto"),
            ("to", "zh"),
            ("appid", appid),
            ("salt", salt.as_str()),
            ("sign", sign.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
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

async fn try_translate(source: &str, text: &str, baidu: &BaiduConfig) -> Result<WordInfo, String> {
    match source {
        "baidu" => translate_baidu(text, &baidu.appid, &baidu.secret).await,
        "bing" => translate_bing(text).await,
        _ => translate_youdao(text).await,
    }
}

/// 翻译核心：默认策略为单词走有道、整句回落必应；仅当配置了百度密钥才优先/降级走百度。
/// 从 translate 命令抽出，供截图翻译管线在无 State 的上下文直接调用。
pub async fn translate_text(cfg: &AppConfig, text: &str) -> Result<WordInfo, String> {
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    // 配置了百度 → 主源为百度（保留用户手动选择），否则默认有道开头
    let mut order: Vec<String> = vec![if baidu_ready {
        cfg.source.clone()
    } else {
        "youdao".into()
    }];
    // 降级顺序：有道 → 必应（免 Key 兜底）→ 百度（需已配置）
    for s in ["youdao", "bing", "baidu"] {
        if !order.contains(&s.to_string()) {
            order.push(s.to_string());
        }
    }

    let mut errors: Vec<String> = Vec::new();
    for src in order {
        if src == "baidu" && !baidu_ready {
            errors.push("[百度] 未配置 appid/secret".into());
            continue;
        }
        match try_translate(&src, text, &cfg.baidu).await {
            Ok(result) => return Ok(result),
            Err(e) => errors.push(format!("[{src}] {e}")),
        }
    }
    Err(errors.join("；"))
}

#[tauri::command]
pub async fn translate(state: State<'_, SharedConfig>, text: String) -> Result<WordInfo, String> {
    let cfg = state.lock().clone();
    translate_text(&cfg, &text).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_fill_screenshot_fields() {
        // 旧配置文件没有这两个新字段，serde 默认值必须兜底
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.screenshot_hotkey, "Ctrl+Alt+S");
        assert_eq!(cfg.ocr_lang, "auto");
    }

    #[tokio::test]
    async fn youdao_returns_entries() {
        let result = translate_youdao("error").await;
        println!("youdao result: {result:?}");
        let info = result.expect("youdao should succeed");
        assert!(!info.senses.is_empty());
        assert!(info.usphone.is_some());
    }

    #[tokio::test]
    async fn youdao_happy_has_synos() {
        let info = translate_youdao("happy").await.expect("should succeed");
        println!("synos: {:?}", info.synos);
        assert!(!info.synos.is_empty());
        assert!(!info.word_forms.is_empty());
    }

    #[tokio::test]
    async fn full_sentence_reports_baidu_guidance() {
        // 有道免费整句接口已下线：整句应报引导配置百度的错误（translate_text 会自动降级百度）
        let r = translate_youdao("The quick brown fox jumps over the lazy dog").await;
        let msg = r.expect_err("整句应报引导性错误");
        assert!(msg.contains("整句"), "错误信息应说明整句原因: {msg}");
    }

    #[tokio::test]
    async fn baidu_rejects_bad_key() {
        // 错误 key 应返回带信息的 Err（能到达 API 且错误处理正常）
        let r = translate_baidu("error", "dummy-appid", "dummy-secret").await;
        assert!(r.is_err());
        println!("baidu err: {:?}", r.err());
    }

    #[tokio::test]
    async fn bing_translates_full_sentence() {
        // 必应免 Key 兜底：整句应返回译文
        let info = translate_bing("The quick brown fox jumps over the lazy dog")
            .await
            .expect("必应整句应能翻译");
        assert_eq!(info.source, "bing");
        assert!(!info.senses.is_empty());
        println!("bing result: {:?}", info.senses);
    }

    #[tokio::test]
    async fn default_strategy_falls_back_to_bing_without_baidu_keys() {
        // 未配置百度：整句走 有道→必应，最终应拿到非有道来源的译文
        let cfg = AppConfig::default();
        let info = translate_text(&cfg, "The rain in Spain falls mainly on the plain")
            .await
            .expect("整句应有必应兜底结果");
        assert_eq!(info.source, "bing");
    }
}
