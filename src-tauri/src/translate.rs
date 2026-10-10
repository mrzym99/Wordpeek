use std::path::PathBuf;
use std::sync::Mutex;
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
    state.lock().unwrap().clone()
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
    *state.lock().unwrap() = config.clone();
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
        // 有道免费整句接口已下线（fanyi 节点不再返回）：交由上层降级到百度
        return Err("有道不支持整句翻译，请配置百度密钥后使用整句翻译".into());
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
        _ => translate_youdao(text).await,
    }
}

/// 翻译核心：主源优先，失败按 有道→百度 顺序降级（百度需已配置 key）。
/// 从 translate 命令抽出，供截图翻译管线在无 State 的上下文直接调用。
pub async fn translate_text(cfg: &AppConfig, text: &str) -> Result<WordInfo, String> {
    // 主源排最前，其余作为降级候选
    let mut order: Vec<String> = vec![cfg.source.clone()];
    for s in ["youdao", "baidu"] {
        if !order.contains(&s.to_string()) {
            order.push(s.to_string());
        }
    }

    let mut errors: Vec<String> = Vec::new();
    for src in order {
        if src == "baidu" && (cfg.baidu.appid.is_empty() || cfg.baidu.secret.is_empty()) {
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
    let cfg = state.lock().unwrap().clone();
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
}
