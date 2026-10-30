use std::path::PathBuf;
use std::sync::OnceLock;
use parking_lot::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

/// 百度翻译 API 的鉴权配置
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct BaiduConfig {
    #[serde(default)]
    pub appid: String,
    #[serde(default)]
    pub secret: String,
}

/// 腾讯云机器翻译的鉴权配置（SecretId + SecretKey）
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct TencentConfig {
    #[serde(default)]
    pub secret_id: String,
    #[serde(default)]
    pub secret_key: String,
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
    /// 内置默认源（有道词典/必应翻译）是否启用；关闭后仅用勾选的额外源
    #[serde(default = "default_builtin_sources")]
    pub builtin_sources: bool,
    /// 输入翻译窗口的唤出快捷键，如 "Ctrl+Alt+I"
    #[serde(default = "default_input_hotkey")]
    pub input_hotkey: String,
    /// 划词弹窗宽度（逻辑像素）：用户拖拽后落盘，重启恢复；高度自适应不存
    #[serde(default = "default_popup_width")]
    pub popup_width: f64,
    /// 设置窗口上次停留的一级菜单：切页即落盘，重开设置窗口时回到上次页
    #[serde(default = "default_settings_section")]
    pub settings_section: String,
    /// 外观主题："system"（跟随系统）| "light" | "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub baidu: BaiduConfig,
    #[serde(default)]
    pub tencent: TencentConfig,
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

fn default_builtin_sources() -> bool {
    true
}

fn default_input_hotkey() -> String {
    "Ctrl+Alt+I".into()
}

fn default_popup_width() -> f64 {
    400.0
}

fn default_settings_section() -> String {
    "translate".into()
}

fn default_theme() -> String {
    "system".into()
}

/// 默认无额外源：主翻译内置（单词→有道、句子→必应），额外源由用户自己勾选
fn default_sources() -> Vec<String> {
    Vec::new()
}

/// 只保留合法的额外源并去重。主翻译的两个源（有道/必应）已内置化，不再作为
/// 可配置项：老配置里残留的 youdao/bing 在这里被剥掉，否则会作为额外源
/// 重复出现在对比区。需要密钥的源（百度）没配密钥时同样剥掉——保存密钥时
/// 删掉密钥，勾选状态会随本次落盘自动移除。空列表合法——主翻译不依赖额外源。
pub(crate) fn enabled_sources(cfg: &AppConfig) -> Vec<String> {
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    let tencent_ready =
        !cfg.tencent.secret_id.is_empty() && !cfg.tencent.secret_key.is_empty();
    let mut out: Vec<String> = Vec::new();
    for s in &cfg.sources {
        let usable = match s.as_str() {
            // 需密钥的源：没配密钥时自动剥掉
            "baidu" => baidu_ready,
            "tencent" => tencent_ready,
            _ => false,
        };
        if usable && !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

/// 老配置迁移：曾用 source = "baidu" 表达「百度作主源」。
/// sources 为空时不补默认——额外源是可选项，主翻译不依赖它
fn migrate_sources(cfg: &mut AppConfig) {
    if !cfg.sources.is_empty() {
        return;
    }
    if cfg.source == "baidu" {
        cfg.sources.push("baidu".into());
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
            builtin_sources: default_builtin_sources(),
            popup_width: default_popup_width(),
            settings_section: default_settings_section(),
            theme: default_theme(),
            baidu: BaiduConfig::default(),
            tencent: TencentConfig::default(),
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

/// 全局共享的 HTTP 客户端：连接池 + TLS 配置只建一次。
/// 之前每次请求都 Client::new()，翻译/对比高频调用时池子来不及回收，内存缓慢上涨
fn http() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    HTTP.get_or_init(|| {
        // 整体超时：服务端无响应时不能让弹窗永远停在“翻译中...”
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest client")
    })
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
    let html = http()
        .get("https://cn.bing.com/translator")
        .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?
        .text()
        .await
        .map_err(|e| format!("响应解析失败: {e}"))?;

    let (ig, key, token) = parse_bing_token(&html)?;
    let tok = BingToken {
        ig,
        key,
        token,
        expires_at: now + 3_600_000 - 300_000,
    };
    *BING_TOKEN.lock() = Some(tok.clone());
    Ok(tok)
}

/// 从必应翻译首页 HTML 抽出凭证（IG / key / token）。
/// 纯函数：格式是必应说了算，随时会变，这里必须用 checked 切片，
/// 格式异常时返回 Err（弹窗显示错误），绝不能 panic。
fn parse_bing_token(html: &str) -> Result<(String, String, String), String> {
    let err = |m: &str| format!("必应页面格式异常：{m}（可能已改版）");
    let i = html.find("IG:\"").ok_or_else(|| err("缺少 IG"))?;
    let rest = html.get(i + 4..).ok_or_else(|| err("缺少 IG"))?;
    // IG 固定 32 位且紧跟收尾引号：不校验的话会把后面的标签一起吞进凭证
    if rest.as_bytes().get(32) != Some(&b'"') {
        return Err(err("IG 长度不足"));
    }
    let ig = rest.get(..32).ok_or_else(|| err("IG 长度不足"))?.to_string();
    let ap = html
        .find("params_AbusePreventionHelper")
        .ok_or_else(|| err("缺少防滥用参数"))?;
    let rest = &html[ap..];
    let open = rest.find('[').ok_or_else(|| err("防滥用参数格式异常"))?;
    let rest = &rest[open + 1..];
    let comma = rest.find(',').ok_or_else(|| err("防滥用参数格式异常"))?;
    let key = rest[..comma].to_string();
    // 逗号后预期是 ", "，但别对空格做硬假设：找到下一个引号内的内容当 token
    let tail = rest.get(comma + 1..).ok_or_else(|| err("防滥用参数格式异常"))?;
    let tail = tail.trim_start();
    let tail = tail.strip_prefix('"').ok_or_else(|| err("防滥用参数格式异常"))?;
    let token = tail.split('"').next().unwrap_or_default().to_string();
    if ig.is_empty() || key.is_empty() || token.is_empty() {
        return Err(err("凭证为空"));
    }
    Ok((ig, key, token))
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
    let resp: serde_json::Value = http()
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

/// identifier 曾为 com.wordpeek.app（macOS 包扩展名冲突被迫改名），
/// app_config_dir 随之变化；仅此一次性把旧目录的 config.json 搬过来，
/// 且仅在新配置不存在时——绝不覆盖用户当前配置。
fn migrate_identifier_rename(app: &AppHandle) {
    let Some(path) = config_path(app) else { return };
    if path.exists() {
        return;
    }
    let legacy = app
        .path()
        .app_config_dir()
        .ok()
        .and_then(|d| d.parent().map(|p| p.join("com.wordpeek.app").join("config.json")));
    if let (Some(legacy), Some(dir)) = (legacy, path.parent()) {
        if std::fs::create_dir_all(dir).is_ok() && std::fs::copy(&legacy, &path).is_ok() {
            println!("[config] migrated legacy config from com.wordpeek.app");
        }
    }
}

/// 启动时调用：读配置文件，不存在或损坏则用默认值，并补齐老配置缺少的字段。
/// 卸载重装后不做旧目录迁移：把远古配置搬回来反而会覆盖用户的当前设置
/// （如翻译源被复旧为 baidu）。
/// 原子落盘：先写 .tmp 再 rename 覆盖，断电/崩溃不会留下截断的 config.json
/// （直接 fs::write 覆盖原文件，中途失败 = 配置丢失）
fn write_config(path: &std::path::Path, cfg: &AppConfig) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("写入配置文件失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("写入配置文件失败: {e}"))
}

pub fn load_config(app: &AppHandle) -> AppConfig {
    migrate_identifier_rename(app);
    let path = config_path(app);
    let raw = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
    let mut cfg: AppConfig = match raw {
        Some(s) => match serde_json::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                // 损坏的配置绝不静默丢弃：备份后用默认值，用户至少能找回来
                if let Some(p) = path {
                    let bak = p.with_extension("json.bak");
                    let _ = std::fs::rename(&p, &bak);
                    eprintln!("[config] 配置文件解析失败（{e}），已备份到 {} 并改用默认配置", bak.display());
                }
                AppConfig::default()
            }
        },
        None => {
            // 只在文件真的存在时才报：新建安装时文件本就不存在，不必吓人
            if path.as_ref().is_some_and(|p| p.exists()) {
                // 文件存在但读不了（如杀软占用）：打日志提醒，避免后面一次保存把真配置盖成默认值
                eprintln!("[config] 配置文件存在但读取失败，本次使用默认值");
            }
            AppConfig::default()
        }
    };
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
    // 全程持锁：读-改-写期间不能放锁，否则两个并发保存会互相覆盖字段。
    // 克隆出来改，失败时不污染内存态
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    let cfg = {
        let mut guard = state.lock();
        let mut cfg = guard.clone();
        mutate(&mut cfg);
        // 快捷键重复会被后端拦下，设置页直接显示这条错误
        crate::validate_hotkeys(&[&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey])?;
        // 快捷键格式不对会在注册时静默失败，保存时就拦住
        for h in [&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey] {
            crate::parse_hotkey(h)?;
        }
        // 勾选列表去重并剔除非法项，避免空列表导致没有可用源
        cfg.sources = enabled_sources(&cfg);
        write_config(&path, &cfg)?;
        *guard = cfg.clone();
        cfg
    };
    println!("[config] saved");

    // 快捷键可能被修改：按新配置重新注册全部快捷键（失败仅日志，不阻断保存）
    crate::register_hotkeys(&app, &cfg);
    // 广播配置变化：已打开的窗口（如划词卡片）据此作废按旧配置算出的结果，
    // 典型场景是设置里拖拽排序后，对比区的顺序要跟上新配置
    let _ = app.emit("config-changed", ());
    Ok(())
}

/// 翻译源：内置源开关 + 额外源勾选（翻译源 tab）。
/// 至少保留一个源的约束只在这里拦——删密钥导致的自动剥离允许清空到零
#[tauri::command]
pub fn save_sources(
    app: AppHandle,
    state: State<SharedConfig>,
    sources: Vec<String>,
    builtin_sources: bool,
) -> Result<(), String> {
    if !builtin_sources && sources.is_empty() {
        return Err("至少保留一个翻译源：请先勾选额外源，再关闭默认源".into());
    }
    persist(app, state, |c| {
        c.sources = sources;
        c.builtin_sources = builtin_sources;
    })
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

/// 腾讯云密钥（源配置 tab）
#[tauri::command]
pub fn save_tencent(
    app: AppHandle,
    state: State<SharedConfig>,
    secret_id: String,
    secret_key: String,
) -> Result<(), String> {
    persist(app, state, |c| {
        c.tencent = TencentConfig {
            secret_id: secret_id.trim().into(),
            secret_key: secret_key.trim().into(),
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

/// 弹窗宽度（用户拖拽后落盘）：只写这一个字段并直接写盘。
/// 刻意不走 persist()：宽度保存不需要重注册快捷键，也不该发 config-changed
/// （会无谓作废划词卡片里的对比区缓存）
#[tauri::command]
pub fn save_popup_width(app: AppHandle, state: State<SharedConfig>, width: f64) -> Result<(), String> {
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    let mut guard = state.lock();
    // 高度动画也会触发 resize 事件：宽度没变就不写盘
    if (guard.popup_width - width).abs() <= 0.5 {
        return Ok(());
    }
    guard.popup_width = width;
    write_config(&path, &guard)
}

/// 设置窗口当前停留的一级菜单：切页即落盘，重开设置窗口时回到上次页。
/// 直接写盘不走 persist：不重挂快捷键、不发 config-changed（表单状态没变）
#[tauri::command]
pub fn save_settings_section(
    app: AppHandle,
    state: State<SharedConfig>,
    section: String,
) -> Result<(), String> {
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    let mut guard = state.lock();
    if guard.settings_section == section {
        return Ok(());
    }
    guard.settings_section = section;
    write_config(&path, &guard)
}

/// 外观主题：system / light / dark。直接写盘不走 persist（不碰快捷键，不发 config-changed），
/// 各窗口下次加载/获得焦点时自己按 get_config 重新落地
#[tauri::command]
pub fn save_theme(app: AppHandle, state: State<SharedConfig>, theme: String) -> Result<(), String> {
    if !matches!(theme.as_str(), "system" | "light" | "dark") {
        return Err(format!("未知主题: {theme}"));
    }
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    let mut guard = state.lock();
    if guard.theme == theme {
        return Ok(());
    }
    guard.theme = theme;
    write_config(&path, &guard)
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

    let resp: serde_json::Value = http()
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
        // URL 里有 appid+sign，reqwest 错误 Display 默认带完整 URL，必须脱掉
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

/// 腾讯云机器翻译（tmt.tencentcloudapi.com TextTranslate，TC3-HMAC-SHA256 签名）
/// 单词、整句通吃；密钥在腾讯云控制台「访问管理 → API 密钥」申请
async fn translate_tencent(text: &str, to: &str, tc: &TencentConfig) -> Result<WordInfo, String> {
    use hmac::Mac;
    use sha2::{Digest, Sha256};
    type HmacSha256 = hmac::Hmac<Sha256>;

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

/// 目标语言 → 各接口的参数写法；非法值一律按中文处理
pub(crate) fn bing_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh-Hans" }
}

pub(crate) fn baidu_target(to: &str) -> &'static str {
    if to == "en" { "en" } else { "zh" }
}

/// 翻译/对比顺序：内置默认源（单词→有道，句子自动落到必应）在前，
/// 用户勾选的额外源排在其后。内置源关闭后只剩额外源。
pub(crate) fn build_order(cfg: &AppConfig, to: &str) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    if cfg.builtin_sources {
        if to == "zh" {
            order.push("youdao".into()); // 有道 jsonapi 只支持英→中
        }
        order.push("bing".into());
    }
    for s in enabled_sources(cfg) {
        if !order.contains(&s) {
            order.push(s);
        }
    }
    order
}

async fn try_translate(
    source: &str,
    text: &str,
    cfg: &AppConfig,
    to: &str,
) -> Result<WordInfo, String> {
    match source {
        "baidu" => {
            translate_baidu(text, &cfg.baidu.appid, &cfg.baidu.secret, to)
                .await
        }
        "bing" => translate_bing(text, to).await,
        // 腾讯云机器翻译双向都支持
        "tencent" => translate_tencent(text, to, &cfg.tencent).await,
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
    let order = build_order(cfg, to);
    if order.is_empty() {
        return Err("没有可用的翻译源：内置源已关闭且未勾选（或未配密钥的）额外源".into());
    }
    let mut errors: Vec<String> = Vec::new();
    for src in order {
        match try_translate(&src, text, cfg, to).await {
            Ok(result) => return Ok(result),
            Err(e) => errors.push(format!("[{src}] {e}")),
        }
    }
    Err(errors.join("；"))
}

/// 语言方向检测：按「中文词数 vs 英文词数」加权判定主体语言，译为对方语言。
/// 不直接比字符数：一个中文词平均 ≈ 2 个汉字，而一个英文词平均 5~6 个字母，
/// 比字母会把「请检查 API timeout 的返回值」这类中文句误判成英文为主。
/// 纯中/纯英/空串结果和旧的「见 CJK 就 →en」一致，只有中英混排会变
pub(crate) fn detect_to(text: &str) -> &'static str {
    let mut cjk = 0usize;
    let mut words = 0usize;
    let mut in_word = false;
    for c in text.chars() {
        if matches!(c,
            '\u{3400}'..='\u{4dbf}'   // CJK 扩展 A
            | '\u{4e00}'..='\u{9fff}' // 基本汉字
            | '\u{3040}'..='\u{30ff}' // 日文假名
            | '\u{ac00}'..='\u{d7af}' // 韩文
        ) {
            cjk += 1;
            in_word = false;
        } else if c.is_alphabetic() {
            // 字母连段计为一个词（其它字母文字也当英文词算）
            if !in_word {
                words += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    // 两个汉字算一个中文词；打平算中文为主（保住旧的「有汉字就 →en」），没汉字一律 →zh
    let zh_words = (cjk + 1) / 2;
    if cjk > 0 && zh_words >= words { "en" } else { "zh" }
}

/// 解析目标语言：调用方没传 / 传了 "auto" → 自动检测
fn resolve_to(text: &str, to: Option<&str>) -> &'static str {
    match to {
        Some(t) if !t.is_empty() && t != "auto" => {
            if t == "en" { "en" } else { "zh" }
        }
        _ => detect_to(text),
    }
}

#[tauri::command]
pub async fn translate(
    state: State<'_, SharedConfig>,
    text: String,
    to: Option<String>,
) -> Result<WordInfo, String> {
    let cfg = state.lock().clone();
    translate_text(&cfg, &text, resolve_to(&text, to.as_deref())).await
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
        "tencent" => "腾讯翻译",
        _ => "未知源",
    }
}

/// 参与对比的源：默认翻译的源排第一，勾选的额外源跟在后面
pub(crate) fn compare_sources(cfg: &AppConfig, to: &str) -> Vec<String> {
    build_order(cfg, to)
}

/// 并发请求各源并逐源收集结果：整体不失败，单个源出错只影响它自己那一行。
/// include_builtin = false 时只请求勾选的额外源（默认翻译的结果由前端复用主卡片数据）
pub async fn translate_all_sources(
    cfg: &AppConfig,
    text: &str,
    to: &str,
    include_builtin: bool,
) -> Vec<SourceResult> {
    let to = if to == "en" { "en" } else { "zh" };
    let baidu_ready = !cfg.baidu.appid.is_empty() && !cfg.baidu.secret.is_empty();
    // 每源一个异步任务：reqwest 请求并行发出，总耗时 ≈ 最慢的源
    let mut sources = compare_sources(cfg, to);
    if !include_builtin {
        sources.retain(|s| s != "youdao" && s != "bing");
    }
    let handles: Vec<(
        String,
        tauri::async_runtime::JoinHandle<Result<WordInfo, String>>,
    )> = sources
        .into_iter()
        .map(|source| {
            let src = source.clone();
            let query = text.to_string();
            let cfg = cfg.clone(); // AppConfig 很小（几个字符串），克隆进异步任务避免生命周期问题
            let to = to.to_string();
            let ready = baidu_ready;
            let handle = tauri::async_runtime::spawn(async move {
                // 百度密钥缺失不必发请求：直接给出与单源路径一致的可读原因
                if src == "baidu" && !ready {
                    return Err("未配置 appid/secret".to_string());
                }
                try_translate(&src, &query, &cfg, &to).await
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
    // false = 只请求额外源（默认翻译的结果前端复用主卡片数据，不重复请求）
    include_builtin: Option<bool>,
) -> Result<Vec<SourceResult>, String> {
    let cfg = state.lock().clone();
    let include_builtin = include_builtin.unwrap_or(true);
    Ok(
        translate_all_sources(&cfg, &text, resolve_to(&text, to.as_deref()), include_builtin)
            .await,
    )
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
    fn migrate_sources_keeps_empty_for_old_configs() {
        // 老配置没有 sources 字段 → 空 Vec 保持为空（额外源是可选项）
        let mut cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.sources.is_empty());
        migrate_sources(&mut cfg);
        assert!(cfg.sources.is_empty());
        // 迁移幂等：已有值时不再动
        cfg.sources = vec!["baidu".into()];
        let mut again = cfg.clone();
        migrate_sources(&mut again);
        assert_eq!(again.sources, vec!["baidu"]);
    }

    #[test]
    fn legacy_baidu_primary_migrates_to_extra_sources() {
        // 旧「启用为主源」= source: "baidu"，现在等价于勾选百度作额外源
        let mut cfg: AppConfig = serde_json::from_str(
            r#"{"source":"baidu","baidu":{"appid":"a","secret":"b"}}"#,
        )
        .unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["baidu"]);
    }

    #[test]
    fn enabled_sources_keeps_only_extra_sources() {
        // 默认无额外源
        let cfg = AppConfig::default();
        assert!(enabled_sources(&cfg).is_empty());
        // 有道/必应已内置化：老配置残留的会被剥掉，避免对比区重复主翻译
        let mut cfg = AppConfig::default();
        cfg.sources = vec!["youdao".into(), "baidu".into(), "bing".into(), "baidu".into()];
        // 百度需密钥：没配密钥时同样剥掉
        assert!(enabled_sources(&cfg).is_empty());
        cfg.baidu = BaiduConfig {
            appid: "a".into(),
            secret: "b".into(),
        };
        assert_eq!(enabled_sources(&cfg), vec!["baidu"]);
        // 腾讯同样需密钥（腾讯云 SecretId/SecretKey）：没配时剥掉，配了才可用
        cfg.sources = vec!["tencent".into(), "nope".into()];
        assert!(enabled_sources(&cfg).is_empty());
        cfg.tencent = TencentConfig {
            secret_id: "id".into(),
            secret_key: "key".into(),
        };
        assert_eq!(enabled_sources(&cfg), vec!["tencent"]);
        // 全非法 → 空列表合法（主翻译不受影响）
        cfg.sources = vec!["nope".into()];
        assert!(enabled_sources(&cfg).is_empty());
    }

    #[test]
    fn build_order_puts_builtin_first() {
        let mut cfg = AppConfig::default();
        // 内置源开启：默认翻译在前，额外源在后
        assert_eq!(build_order(&cfg, "zh"), vec!["youdao", "bing"]);
        assert_eq!(build_order(&cfg, "en"), vec!["bing"]);
        cfg.sources = vec!["baidu".into()];
        // 百度需密钥：没配密钥时进不了队列
        assert_eq!(build_order(&cfg, "zh"), vec!["youdao", "bing"]);
        cfg.baidu = BaiduConfig {
            appid: "a".into(),
            secret: "b".into(),
        };
        assert_eq!(build_order(&cfg, "zh"), vec!["youdao", "bing", "baidu"]);
        // 内置源关闭：只剩额外源
        cfg.builtin_sources = false;
        assert_eq!(build_order(&cfg, "zh"), vec!["baidu"]);
        // 内置源关闭且无额外源：空队列（persist 层会拦住这种配置落盘）
        let mut off = AppConfig::default();
        off.builtin_sources = false;
        assert!(build_order(&off, "zh").is_empty());
    }

    #[test]
    fn compare_sources_puts_default_translation_first() {
        let mut cfg = AppConfig::default();
        cfg.tencent = TencentConfig {
            secret_id: "id".into(),
            secret_key: "key".into(),
        };
        cfg.sources = vec!["tencent".into()];
        // 对比区：默认翻译的源排第一，额外源跟在后面
        assert_eq!(compare_sources(&cfg, "zh"), vec!["youdao", "bing", "tencent"]);
        cfg.builtin_sources = false;
        assert_eq!(compare_sources(&cfg, "zh"), vec!["tencent"]);
    }

    #[test]
    fn parse_bing_token_happy_path() {
        let ig = "A1B2C3D4E5F6G7H8I9J0K1L2M3N4O5P6"; // 32 位
        let html = format!(
            "<script>var params_AbusePreventionHelper = [1234567890,\"TOKENABC\",3600000];</script> IG:\"{ig}\";"
        );
        let (g, key, token) = parse_bing_token(&html).expect("完整页面应能抽出凭证");
        assert_eq!(g, ig);
        assert_eq!(key, "1234567890");
        assert_eq!(token, "TOKENABC");
    }

    #[test]
    fn parse_bing_token_rejects_malformed_pages() {
        // 缺 IG
        assert!(parse_bing_token("params_AbusePreventionHelper=[1,\"t\"]").is_err());
        // 缺防滥用参数
        assert!(parse_bing_token("IG:\"A1B2C3D4E5F6G7H8I9J0K1L2M3N4O5P6\"").is_err());
        // IG 不足 32 位
        assert!(parse_bing_token("IG:\"short\"params_AbusePreventionHelper=[1,\"t\"]").is_err());
        // 防滥用参数无逗号
        assert!(parse_bing_token("IG:\"A1B2C3D4E5F6G7H8I9J0K1L2M3N4O5P6\"params_AbusePreventionHelper=[1").is_err());
        // 空页面 / 纯乱文本不 panic，返回 Err
        assert!(parse_bing_token("").is_err());
        assert!(parse_bing_token("随机网页内容").is_err());
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
        // 内置源关闭且无额外源：不允许出现（persist 会拦），这里直接验证空队列行为
        let mut cfg = AppConfig::default();
        cfg.builtin_sources = false;
        assert!(translate_all_sources(&cfg, "hello", "zh", true).await.is_empty());

        // 内置源关闭、只勾百度但密钥无效：只有百度这一行，且行内报错
        cfg.sources = vec!["baidu".into()];
        cfg.baidu = BaiduConfig {
            appid: "fake".into(),
            secret: "fake".into(),
        };
        let rows = translate_all_sources(
            &cfg,
            "The quick brown fox jumps over the lazy dog",
            "zh",
            true,
        )
        .await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "baidu");
        assert!(!rows[0].ok);
        assert!(rows[0].error.is_some());
    }

    #[test]
    fn config_defaults_fill_input_hotkey() {
        // 旧配置文件没有这个字段，serde 默认值必须兜底
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.input_hotkey, "Ctrl+Alt+I");
        assert_eq!(cfg.popup_width, 400.0);
        assert_eq!(cfg.settings_section, "translate");
    }

    #[test]
    fn detect_to_by_script() {
        assert_eq!(detect_to("那只敏捷的棕色狐狸"), "en");
        assert_eq!(detect_to("hello world"), "zh");
        assert_eq!(detect_to("mixed 中文 text"), "zh"); // 英文句里只夹一个中文词：主体是英文
        assert_eq!(detect_to(""), "zh");
        // 中英混排：按中文词 vs 英文词加权，不再“一个汉字就算中文”
        assert_eq!(detect_to("The 型号 说明 is wrong"), "zh");
        assert_eq!(detect_to("Click the OK button to 确认"), "zh");
        assert_eq!(detect_to("React 组件"), "en"); // 打平仍按旧的「有汉字就 →en」
        // 中文为主只夹几个术语 → 仍译为英文
        assert_eq!(detect_to("请检查 API timeout 的返回值"), "en");
    }

    #[test]
    fn utc_date_matches_epoch() {
        assert_eq!(utc_date(0), "1970-01-01");
        // 2026-01-01 00:00:00 UTC = 1767225600
        assert_eq!(utc_date(1767225600), "2026-01-01");
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

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn youdao_rejects_non_chinese_target() {
        let cfg = AppConfig::default();
        let err = try_translate("youdao", "hello", &cfg, "en")
            .await
            .expect_err("译为英文应拒绝有道");
        assert!(err.contains("仅支持译为中文"), "错误信息应说明原因: {err}");
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
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
        // 有道免费整句接口已下线：整句应报引导配置百度的错误（translate_text 会自动降级百度）
        let r = translate_youdao("The quick brown fox jumps over the lazy dog").await;
        let msg = r.expect_err("整句应报引导性错误");
        assert!(msg.contains("整句"), "错误信息应说明整句原因: {msg}");
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

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn bing_translates_full_sentence() {
        // 必应免 Key 兜底：整句应返回译文
        let info = translate_bing("The quick brown fox jumps over the lazy dog", "zh")
            .await
            .expect("必应整句应能翻译");
        assert_eq!(info.source, "bing");
        assert!(!info.senses.is_empty());
        println!("bing result: {:?}", info.senses);
    }

    // 真网络请求：默认忽略，跑法 cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn default_strategy_falls_back_to_bing_without_baidu_keys() {
        // 未配置百度：整句走 有道→必应，最终应拿到非有道来源的译文
        let cfg = AppConfig::default();
        let info = translate_text(&cfg, "The rain in Spain falls mainly on the plain", "zh")
            .await
            .expect("整句应有必应兜底结果");
        assert_eq!(info.source, "bing");
    }
}

