//! 翻译模块：配置类型与持久化、命令层、语言检测。各翻译源实现在 sources/，执行链在 pipeline.rs。

mod pipeline;
mod sources;

pub use pipeline::translate_text;

use std::path::{Path, PathBuf};
use parking_lot::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};


/// 应用配置，持久化在 app_config_dir/config.json
#[derive(Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// 旧字段：曾用于「百度启用为主源」，现仅作迁移依据（见 migrate_sources）
    #[serde(default = "default_source")]
    pub source: String,
    /// 句子翻译主源（单选，设置页互斥）："deepl" | "bing"；必应永远在链尾兜底
    #[serde(default = "default_sources")]
    pub sources: Vec<String>,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_screenshot_hotkey")]
    pub screenshot_hotkey: String,
    /// 截图 OCR 识别语言："auto" | "zh" | "en"
    #[serde(default = "default_ocr_lang")]
    pub ocr_lang: String,
    #[serde(default = "default_input_hotkey")]
    pub input_hotkey: String,
    /// 划词图标开关的快捷键：按下翻转 selection_icon，不用进取设置页
    #[serde(default = "default_icon_hotkey")]
    pub icon_hotkey: String,
    /// 划词弹窗宽度（逻辑像素）：用户拖拽后落盘，重启恢复；高度自适应不存
    #[serde(default = "default_popup_width")]
    pub popup_width: f64,
    /// 设置窗口上次停留的一级菜单：切页即落盘，重开设置窗口时回到上次页
    #[serde(default = "default_settings_section")]
    pub settings_section: String,
    /// 外观主题："system"（跟随系统）| "light" | "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 选中文字后在光标旁弹翻译按钮（默认开）
    #[serde(default = "default_selection_icon")]
    pub selection_icon: bool,
}

fn default_source() -> String {
    "youdao".into()
}

fn default_hotkey() -> String {
    "Alt+Z".into()
}

fn default_screenshot_hotkey() -> String {
    "Alt+S".into()
}

fn default_ocr_lang() -> String {
    "auto".into()
}

fn default_input_hotkey() -> String {
    "Alt+X".into()
}

fn default_icon_hotkey() -> String {
    "Alt+W".into()
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

fn default_selection_icon() -> bool {
    true
}

fn default_sources() -> Vec<String> {
    // 句子主源单选（设置页互斥）；必应永远在链尾兜底，不靠这个字段
    vec!["deepl".into()]
}

/// 老配置迁移：句子源多选勾选归一化——只留 deepl/bing，
/// 按固定顺序去重，空则回默认；旧 sentence_source 字段由 serde 忽略
fn migrate_sources(cfg: &mut AppConfig) {
    let has = |id: &str| cfg.sources.iter().any(|s| s == id);
    // 句子源已改成单选：老配置里 deepl+bing 同时勾的收敛到 deepl（必应永远在链尾兜底）
    cfg.sources = if has("deepl") {
        vec!["deepl".into()]
    } else if has("bing") {
        vec!["bing".into()]
    } else {
        default_sources()
    };
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
            icon_hotkey: default_icon_hotkey(),
            popup_width: default_popup_width(),
            settings_section: default_settings_section(),
            selection_icon: default_selection_icon(),
            theme: default_theme(),
        }
    }
}

type SharedConfig = Mutex<AppConfig>;

/// 落盘串行锁：写盘顺序与内存修改顺序一致；配置锁内绝不做磁盘 I/O（杀软/系统卡住写盘会冻住所有读配置路径）
static CONFIG_WRITE: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct WordForm {
    pub name: String,
    pub value: String,
}

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
    pub exam_types: Vec<String>,
}

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("config.json"))
}

/// identifier 曾为 com.wordpeek.app，app_config_dir 随之变化；仅在新配置不存在时搬旧目录的 config.json，绝不覆盖当前配置
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

/// 原子落盘：先写 .tmp 再 rename，中途失败不会留下截断的 config.json（fs::write 直写 = 配置丢失）。
/// 卸载重装不做旧目录迁移：搬回远古配置会覆盖用户当前设置（如翻译源复旧为 baidu）
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

/// 模块化落盘：以后端当前配置为基底，只改调用方负责的字段后整体写回，杜绝「一处保存覆盖另一处」
fn persist<F: FnOnce(&mut AppConfig)>(
    app: AppHandle,
    state: State<SharedConfig>,
    mutate: F,
) -> Result<(), String> {
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    // 落盘串行锁：写盘顺序与内存修改顺序一致，否则并发保存时旧快照可能覆盖新字段（I/O 只在这把锁内）
    let _write = CONFIG_WRITE.lock();
    let cfg = {
        let mut guard = state.lock();
        let mut cfg = guard.clone();
        mutate(&mut cfg);
        crate::hotkeys::validate_hotkeys(&[&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey, &cfg.icon_hotkey])?;
        // 快捷键格式不对会在注册时静默失败，保存时就拦住
        for h in [&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey, &cfg.icon_hotkey] {
            crate::hotkeys::parse_hotkey(h)?;
        }
        *guard = cfg.clone();
        cfg
    };
    write_config(&path, &cfg)?;
    println!("[config] saved");

    let failed = crate::hotkeys::register_hotkeys(&app, &cfg);
    if !failed.is_empty() {
        return Err(format!(
            "配置已保存，但快捷键注册失败：{}（可能被其他程序占用）",
            failed.join("、")
        ));
    }
    // 广播配置变化：已打开的窗口据此作废按旧配置算出的结果（如对比区的排序要跟上新配置）
    let _ = app.emit("config-changed", ());
    Ok(())
}

/// 句子翻译源勾选（翻译源 tab）：deepl/bing 可自由勾选，至少保留一个
#[tauri::command]
pub fn save_sources(
    app: AppHandle,
    state: State<SharedConfig>,
    sources: Vec<String>,
) -> Result<(), String> {
    let has = |id: &str| sources.iter().any(|s| s == id);
    if !has("deepl") && !has("bing") {
        return Err("句子翻译至少保留一个源：DeepL 或 必应".into());
    }
    persist(app, state, |c| {
        // 归一化：单选，只认 deepl/bing（老调用方可能传两个，deepl 优先）
        c.sources = if has("deepl") {
            vec!["deepl".into()]
        } else {
            vec!["bing".into()]
        };
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
    icon_hotkey: String,
    ocr_lang: String,
) -> Result<(), String> {
    persist(app, state, |c| {
        c.hotkey = hotkey;
        c.screenshot_hotkey = screenshot_hotkey;
        c.input_hotkey = input_hotkey;
        c.icon_hotkey = icon_hotkey;
        c.ocr_lang = ocr_lang;
    })
}

/// 弹窗宽度（拖拽后落盘）：不走 persist——不重注册快捷键、不发 config-changed（不无谓作废对比区缓存）
#[tauri::command]
pub fn save_popup_width(app: AppHandle, state: State<SharedConfig>, width: f64) -> Result<(), String> {
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    let _write = CONFIG_WRITE.lock();
    let snapshot = {
        let mut guard = state.lock();
        // 高度动画也会触发 resize 事件：宽度没变就不写盘
        if (guard.popup_width - width).abs() <= 0.5 {
            return Ok(());
        }
        guard.popup_width = width;
        guard.clone()
    };
    write_config(&path, &snapshot)
}

/// 设置页当前一级菜单：切页即落盘，重开时回到上次页；不走 persist（不碰快捷键、不发 config-changed）
#[tauri::command]
pub fn save_settings_section(
    app: AppHandle,
    state: State<SharedConfig>,
    section: String,
) -> Result<(), String> {
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    let _write = CONFIG_WRITE.lock();
    let snapshot = {
        let mut guard = state.lock();
        if guard.settings_section == section {
            return Ok(());
        }
        guard.settings_section = section;
        guard.clone()
    };
    write_config(&path, &snapshot)
}

/// 外观主题：不走 persist（不碰快捷键、不发 config-changed），各窗口下次加载/获焦时自己重新落地
#[tauri::command]
pub fn save_theme(app: AppHandle, state: State<SharedConfig>, theme: String) -> Result<(), String> {
    if !matches!(theme.as_str(), "system" | "light" | "dark") {
        return Err(format!("未知主题: {theme}"));
    }
    let path = config_path(&app).ok_or("无法定位配置目录")?;
    let _write = CONFIG_WRITE.lock();
    let snapshot = {
        let mut guard = state.lock();
        if guard.theme == theme {
            return Ok(());
        }
        guard.theme = theme;
        guard.clone()
    };
    write_config(&path, &snapshot)
}

/// 划词图标开关（全局快捷键 icon_hotkey 触发）：翻转 selection_icon 并落盘；
/// 鼠标监听后台探测时每次现读该字段，改完立即生效。切换后弹系统通知反馈新状态
/// （mac 未授权通知时静默失败，功能不受影响）
pub(crate) fn toggle_selection_icon(app: &AppHandle) {
    let Some(path) = config_path(app) else {
        eprintln!("[icon] 无法定位配置目录，切换未落盘");
        return;
    };
    // 锁内只翻字段取快照，磁盘 I/O 在锁外（同其余 save_* 命令）
    let (enabled, snapshot) = {
        let shared = app.state::<SharedConfig>();
        let mut guard = shared.lock();
        guard.selection_icon = !guard.selection_icon;
        (guard.selection_icon, guard.clone())
    };
    let _write = CONFIG_WRITE.lock();
    if let Err(e) = write_config(&path, &snapshot) {
        eprintln!("[icon] 切换落盘失败: {e}");
    }
    // 关闭时顺手把可能还显着的图标收掉
    if !enabled {
        if let Some(win) = app.get_webview_window("icon") {
            let _ = win.hide();
        }
    }
    // 反馈用屏上 HUD（系统 toast 显示太久且连按排队，看不出当前状态；mac 无 toast 也能统一）
    crate::windows::show_hud(
        app,
        if enabled { "划词图标已开启" } else { "划词图标已关闭" },
    );
}

/// 语言方向检测：按「中文词数 vs 英文词数」加权判定主体语言（中文词 ≈2 汉字、英文词 5~6 字母，
/// 比字符数会把「请检查 API timeout 的返回值」这类中文句误判成英文），译为对方语言；打平算中文为主
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

/// 崩溃日志落配置目录：tokio 任务里的 panic 默认只打到看不见的 stderr（追加写，不轮转）
pub(crate) fn log_panic(dir: &Path, msg: &str) {
    use std::io::Write;
    if std::fs::create_dir_all(dir).is_ok() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("panic.log"))
        {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = writeln!(f, "[unix {ms}ms] {msg}");
        }
    }
}

#[tauri::command]
pub async fn translate(
    state: State<'_, SharedConfig>,
    text: String,
    to: Option<String>,
) -> Result<WordInfo, String> {
    let cfg = state.lock().clone();
    let to = resolve_to(&text, to.as_deref());
    translate_text(&cfg, &text, to).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_fill_screenshot_fields() {
        // 旧配置文件没有这两个新字段，serde 默认值必须兜底
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.screenshot_hotkey, "Alt+S");
        assert_eq!(cfg.ocr_lang, "auto");
    }

    #[test]
    fn migrate_sources_normalizes_to_single_source() {
        // 新配置无勾选：默认 DeepL（必应永远在链尾兜底）
        let mut cfg: AppConfig = serde_json::from_str("{}").unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["deepl"]);
        // 老配置同时勾了 deepl+bing（还带未知 id）：收敛到 deepl
        let mut cfg: AppConfig =
            serde_json::from_str(r#"{"sources":["bing","glm","deepl"]}"#).unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["deepl"]);
        // 只勾必应：保留单选
        let mut cfg: AppConfig = serde_json::from_str(r#"{"sources":["bing"]}"#).unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["bing"]);
        // 勾选为空/全不可用：回默认
        let mut cfg: AppConfig = serde_json::from_str(r#"{"sources":["glm"]}"#).unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["deepl"]);
    }

    #[test]
    fn legacy_baidu_primary_migrates_to_default_sources() {
        // 旧「启用为主源」= source: "baidu"；百度源已下线（Depl 可能已勾过）→ 句子主源 Deepl
        let mut cfg: AppConfig = serde_json::from_str(
            r#"{"source":"baidu","baidu":{"appid":"a","secret":"b"}}"#,
        )
        .unwrap();
        migrate_sources(&mut cfg);
        assert_eq!(cfg.sources, vec!["deepl"]);
    }

    #[test]
    fn config_defaults_fill_input_hotkey() {
        let cfg: AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg.input_hotkey, "Alt+X");
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
}
