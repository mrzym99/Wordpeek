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
    /// 旧字段：曾用于「百度启用为主源」，现仅作迁移依据（见 migrate_sources）
    #[serde(default = "default_source")]
    pub source: String,
    /// 参与翻译的源，按优先级排序（空 = 未设置过，加载时迁移补默认）
    #[serde(default)]
    pub sources: Vec<String>,
    /// 划词快捷键，如 "Ctrl+Alt+T"
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// 截图翻译快捷键，如 "Ctrl+Alt+S"
    #[serde(default = "default_screenshot_hotkey")]
    pub screenshot_hotkey: String,
    /// 截图 OCR 识别语言："auto" | "zh" | "en"
    #[serde(default = "default_ocr_lang")]
    pub ocr_lang: String,
    /// 输入翻译窗口的唤出快捷键，如 "Ctrl+Alt+I"
    #[serde(default = "default_input_hotkey")]
    pub input_hotkey: String,
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

fn default_input_hotkey() -> String {
    "Ctrl+Alt+I".into()
}

/// 默认参与翻译的源：单词走有道、整句走必应（与改造前顺序一致）
fn default_sources() -> Vec<String> {
    vec!["youdao".into(), "bing".into()]
}

/// 剔除非法源名与重复项；空结果回退默认——任何情况下都不能返回空列表
pub(crate) fn enabled_sources(cfg: &AppConfig) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in &cfg.sources {
        if matches!(s.as_str(), "youdao" | "bing" | "baidu") && !out.contains(s) {
            out.push(s.clone());
        }
    }
    if out.is_empty() {
        return default_sources();
    }
    out
}

/// 老配置迁移：曾用 source = "baidu" 表达「百度作主源」，
/// 等价于把 baidu 排到列表最前（旧顺序即 [baidu, youdao, bing]）
fn migrate_sources(cfg: &mut AppConfig) {
    if !cfg.sources.is_empty() {
        return;
    }
    cfg.sources = default_sources();
    if cfg.source == "baidu" {
        cfg.sources.insert(0, "baidu".into());
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            source: default_source(),
            sources: default_sources(),
            hotkey: default_hotkey(),
            screenshot_hotkey: default_screenshot_hotkey(),
            ocr_lang: default_ocr_lang(),
            input_hotkey: default_input_hotkey(),
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
async fn translate_bing(text: &str, to: &str) -> Result<WordInfo, String> {
    let t = bing_token().await?;
    let form = [
        ("fromLang", "auto-detect".to_string()),
        ("to", bing_target(to).to_string()),
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

/// 启动时调用：读配置文件，不存在或损坏则用默认值，并补齐老配置缺少的字段。
/// 不做旧目录迁移：卸载重装后若配置丢失，把远古配置搬回来
/// 反而会覆盖用户的当前设置（如翻译源被复旧为 baidu）。
pub fn load_config(app: &AppHandle) -> AppConfig {
    let raw = config_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok());
    let mut cfg: AppConfig = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    migrate_sources(&mut cfg);
    cfg
}

#[tauri::command]
pub fn get_config(state: State<SharedConfig>) -> AppConfig {
    state.lock().clone()
}

/// 模块化落盘：以后端当前配置为基底，只改调用方负责的字段后整体写回。
/// 设置页每个区域只调自己的命令，互不可见对方的字段，
/// 从结构上杜绝「一处保存覆盖另一处配置」的问题。
fn persist<F: FnOnce(&mut AppConfig)>(
    app: AppHandle,
    state: State<SharedConfig>,
    mutate: F,
) -> Result<(), String> {
    let mut cfg = state.lock().clone();
    mutate(&mut cfg);
    // 快捷键重复会被后端拦下，设置页直接显示这条错误
    crate::validate_hotkeys(&[&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey])?;
    // 勾选列表去重并剔除非法项，避免空列表导致没有可用源
    cfg.sources = enabled_sources(&cfg);
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写入配置文件失败: {e}"))?;
    *state.lock() = cfg.clone();
    println!("[config] saved");

    // 快捷键可能被修改：按新配置重新注册全部快捷键（失败仅日志，不阻断保存）
    crate::register_hotkeys(&app, &cfg);
    Ok(())
}

/// 翻译源：勾选/排序（源列表 tab）
#[tauri::command]
pub fn save_sources(
    app: AppHandle,
    state: State<SharedConfig>,
    sources: Vec<String>,
) -> Result<(), String> {
    persist(app, state, |c| c.sources = sources)
}

/// 百度密钥（源配置 tab）
#[tauri::command]
pub fn save_baidu(
    app: AppHandle,
    state: State<SharedConfig>,
    appid: String,
    secret: String,
) -> Result<(), String> {
    persist(app, state, |c| {
        c.baidu = BaiduConfig {
            appid: appid.trim().into(),
            secret: secret.trim().into(),
        }
    })
}

/// 快捷键 + OCR 语言（快捷键/截图翻译 tab 的「保存」按钮）
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<SharedConfig>,
    hotkey: String,
    screenshot_hotkey: String,
    input_hotkey: String,
    ocr_lang: String,
) -> Result<(), String> {
    persist(app, state, |c| {
        c.hotkey = hotkey;
        c.screenshot_hotkey = screenshot_hotkey;
        c.input_hotkey = input_hotkey;
        c.ocr_lang = ocr_lang;
    })
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

    let resp: serde_json::Value = reqwest::Client::new()
        .get("https://fanyi-api.baidu.com/api/trans/vip/translate")
        .query(&[
            ("q", text),
            ("from", "auto"),
            ("to", baidu_target(to)),
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

/// 目标语言 → 各接口的参数写法；非法值一律按中文处理
pub(crate) fn bing_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh-Hans" }
}

pub(crate) fn baidu_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh" }
}

/// 把一个源按目标语言可用性追加进降级队列（去重）。
/// 未勾选的源不会进来：勾选即取舍。百度密钥缺失留给 translate_text 报可读错误。
fn push_source(order: &mut Vec<String>, source: &str, to: &str) {
    let usable = match source {
        // 有道 jsonapi 只能英→中
        "youdao" => to == "zh",
        "bing" => true,
        "baidu" => true,
        _ => false,
    };
    if usable && !order.iter().any(|s| s == source) {
        order.push(source.to_string());
    }
}

/// 降级顺序 = 用户勾选顺序（译为英文时自动跳过有道）
pub(crate) fn build_order(cfg: &AppConfig, to: &str) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    for s in enabled_sources(cfg) {
        push_source(&mut order, &s, to);
    }
    order
}

async fn try_translate(
    source: &str,
    text: &str,
    baidu: &BaiduConfig,
    to: &str,
) -> Result<WordInfo, String> {
    match source {
        "baidu" => translate_baidu(text, &baidu.appid, &baidu.secret, to).await,
        "bing" => translate_bing(text, to).await,
        // 有道 jsonapi 只能英→中（词典 + 已下线的整句节点），译为其它语言交给降级链
        _ if to == "zh" => translate_youdao(text).await,
        _ => Err("有道词典仅支持译为中文".into()),
    }
}

/// 翻译核心：按用户勾选顺序逐个尝试源（首个成功即返回）；勾选内的源全失败才报错。
/// to: "zh" | "en"（其它值按 "zh" 处理）。
/// 从 translate 命令抽出，供截图翻译管线在无 State 的上下文直接调用。
pub async fn translate_text(cfg: &AppConfig, text: &str, to: &str) -> Result<WordInfo, String> {
    let to = if to == "en" { "en" } else { "zh" };
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    let mut errors: Vec<String> = Vec::new();
    for src in build_order(cfg, to) {
        if src == "baidu" && !baidu_ready {
            errors.push("[百度] 未配置 appid/secret".into());
            continue;
        }
        match try_translate(&src, text, &cfg.baidu, to).await {
            Ok(result) => return Ok(result),
            Err(e) => errors.push(format!("[{src}] {e}")),
        }
    }
    Err(errors.join("；"))
}

#[tauri::command]
pub async fn translate(
    state: State<'_, SharedConfig>,
    text: String,
    to: Option<String>,
) -> Result<WordInfo, String> {
    let cfg = state.lock().clone();
    translate_text(&cfg, &text, to.as_deref().unwrap_or("zh")).await
}

/// 单源对比结果：失败的源也返回，由前端逐行展示错误
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SourceResult {
    pub source: String,
    pub label: String,
    pub ok: bool,
    pub senses: Vec<String>,
    pub error: Option<String>,
}

/// 源展示名（对比行标题）
pub(crate) fn source_label(source: &str) -> &'static str {
    match source {
        "youdao" => "有道词典",
        "bing" => "必应翻译",
        "baidu" => "百度翻译",
        _ => "未知源",
    }
}

/// 参与对比的源 = 用户勾选的源，按勾选顺序（译为英文时跳过有道）
pub(crate) fn compare_sources(cfg: &AppConfig, to: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in enabled_sources(cfg) {
        if s == "youdao" && to != "zh" {
            continue;
        }
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// 并发请求全部勾选源并逐源收集结果：整体不失败，单个源出错只影响它自己那一行
pub async fn translate_all_sources(cfg: &AppConfig, text: &str, to: &str) -> Vec<SourceResult> {
    let to = if to == "en" { "en" } else { "zh" };
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    // 每源一个异步任务：reqwest 请求并行发出，总耗时 ≈ 最慢的源
    let handles: Vec<(
        String,
        tauri::async_runtime::JoinHandle<Result<WordInfo, String>>,
    )> = compare_sources(cfg, to)
        .into_iter()
        .map(|source| {
            let src = source.clone();
            let query = text.to_string();
            let baidu = cfg.baidu.clone();
            let to = to.to_string();
            let ready = baidu_ready;
            let handle = tauri::async_runtime::spawn(async move {
                // 百度密钥缺失不必发请求：直接给出与单源路径一致的可读原因
                if src == "baidu" && !ready {
                    return Err("未配置 appid/secret".to_string());
                }
                try_translate(&src, &query, &baidu, &to).await
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

/// 对比命令：一次列出各勾选源译文（输入翻译窗口 / 划词卡片「对比各源」按钮）。
/// Tauri 要求带引用参数（State）的 async 命令必须返回 Result，这里恒为 Ok：
/// 单个源的失败已经在各自行里呈现，不该让整次对比失败。
#[tauri::command]
pub async fn translate_all(
    state: State<'_, SharedConfig>,
    text: String,
    to: Option<String>,
) -> Result<Vec<SourceResult>, String> {
    let cfg = state.lock().clone();
    Ok(translate_all_sources(&cfg, &text, to.as_deref().unwrap_or("zh")).await)
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

    #[test]
    fn migrate_sources_fills_defaults_for_old_configs() {
        // 老配置没有 sources 字段 → 空 Vec → 迁移补默认
        let mut cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.sources.is_empty());
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["youdao", "bing"]);
        // 迁移幂等：已有值时不再动
        let mut again = cfg.clone();
        migrate_sources(&mut again);
        assert_eq!(again.sources, vec!["youdao", "bing"]);
    }

    #[test]
    fn legacy_baidu_primary_migrates_to_front() {
        // 旧「启用为主源」= source: "baidu"，等价于把百度排到最前
        let mut cfg: AppConfig = serde_json::from_str(
            r#"{"source":"baidu","baidu":{"appid":"a","secret":"b"}}"#,
        )
        .unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["baidu", "youdao", "bing"]);
    }

    #[test]
    fn enabled_sources_drops_unknown_and_duplicates() {
        let mut cfg = AppConfig::default();
        assert_eq!(enabled_sources(&cfg), vec!["youdao", "bing"]);
        cfg.sources = vec!["bing".into(), "nope".into(), "bing".into()];
        assert_eq!(enabled_sources(&cfg), vec!["bing"]);
        // 全非法 → 回退默认，绝不能返回空（否则翻译没有可用源）
        cfg.sources = vec!["nope".into()];
        assert_eq!(enabled_sources(&cfg), vec!["youdao", "bing"]);
    }

    #[test]
    fn build_order_follows_checked_order() {
        let mut cfg = AppConfig::default();
        cfg.sources = vec!["bing".into(), "youdao".into()];
        assert_eq!(build_order(&cfg, "zh"), vec!["bing", "youdao"]);
        // 译为英文时跳过有道（它只支持英→中）
        assert_eq!(build_order(&cfg, "en"), vec!["bing"]);
    }

    #[test]
    fn build_order_never_uses_unchecked_sources() {
        // 只勾必应：即使必应失败也不会回落到未勾选的有道
        let mut cfg = AppConfig::default();
        cfg.sources = vec!["bing".into()];
        assert_eq!(build_order(&cfg, "zh"), vec!["bing"]);
        // 勾了未配置密钥的百度：仍在队列里，由 translate_text 给出可读错误
        cfg.sources = vec!["baidu".into(), "bing".into()];
        assert_eq!(build_order(&cfg, "zh"), vec!["baidu", "bing"]);
    }

    #[test]
    fn compare_sources_follows_checked_order_and_target() {
        let mut cfg = AppConfig::default();
        assert_eq!(compare_sources(&cfg, "zh"), vec!["youdao", "bing"]);
        // 译为英文：对比页也不该出现有道
        assert_eq!(compare_sources(&cfg, "en"), vec!["bing"]);
        // 顺序即用户拖拽出来的优先级
        cfg.sources = vec!["bing".into(), "youdao".into()];
        assert_eq!(compare_sources(&cfg, "zh"), vec!["bing", "youdao"]);
        // 勾了未配置密钥的百度：仍要出这一行（行内展示「未配置」原因）
        cfg.sources = vec!["bing".into(), "baidu".into()];
        assert_eq!(compare_sources(&cfg, "zh"), vec!["bing", "baidu"]);
    }

    #[test]
    fn source_labels_are_human_readable() {
        assert_eq!(source_label("youdao"), "有道词典");
        assert_eq!(source_label("bing"), "必应翻译");
        assert_eq!(source_label("baidu"), "百度翻译");
        assert_eq!(source_label("nope"), "未知源");
    }

    #[tokio::test]
    async fn compare_reports_each_source_separately() {
        // 整句：有道没有词条（错误行），必应出译文（成功行）——整体不失败
        let cfg = AppConfig::default();
        let rows = translate_all_sources(
            &cfg,
            "The quick brown fox jumps over the lazy dog",
            "zh",
        )
        .await;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].source, "youdao");
        assert!(!rows[0].ok);
        assert!(rows[0].error.is_some());
        assert_eq!(rows[1].source, "bing");
        assert!(rows[1].ok);
        assert!(rows[1].error.is_none());
        assert!(!rows[1].senses.is_empty());
        println!("compare rows: {rows:?}");
    }

    #[test]
    fn config_defaults_fill_input_hotkey() {
        // 旧配置文件没有这个字段，serde 默认值必须兜底
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.input_hotkey, "Ctrl+Alt+I");
    }

    #[test]
    fn targets_map_to_each_api() {
        assert_eq!(bing_target("zh"), "zh-Hans");
        assert_eq!(bing_target("en"), "en");
        assert_eq!(baidu_target("zh"), "zh");
        assert_eq!(baidu_target("en"), "en");
        // 非法值按中文处理
        assert_eq!(bing_target("fr"), "zh-Hans");
        assert_eq!(baidu_target("fr"), "zh");
    }

    #[tokio::test]
    async fn youdao_rejects_non_chinese_target() {
        let cfg = AppConfig::default();
        let err = try_translate("youdao", "hello", &cfg.baidu, "en")
            .await
            .expect_err("译为英文应拒绝有道");
        assert!(err.contains("仅支持译为中文"), "错误信息应说明原因: {err}");
    }

    #[tokio::test]
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
        let r = translate_baidu("error", "dummy-appid", "dummy-secret", "zh").await;
        assert!(r.is_err());
        println!("baidu err: {:?}", r.err());
    }

    #[tokio::test]
    async fn bing_translates_full_sentence() {
        // 必应免 Key 兜底：整句应返回译文
        let info = translate_bing("The quick brown fox jumps over the lazy dog", "zh")
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
        let info = translate_text(&cfg, "The rain in Spain falls mainly on the plain", "zh")
            .await
            .expect("整句应有必应兜底结果");
        assert_eq!(info.source, "bing");
    }
}
