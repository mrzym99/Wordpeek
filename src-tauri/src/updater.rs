//! 更新器：检查/下载/静默安装 + 应用重启。从 main.rs 整体迁出（纯移动不改逻辑）。
//!
//! Windows 走 Tauri updater（下载 + 静默安装）；macOS 没有签名/公证的更新包，
//! latest.json 里也没有 darwin 平台，所以只做「查版本号」，安装交给用户在
//! GitHub Releases 手动下载（设置 → 关于里的「前往下载」按钮）。

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// 更新清单（与 tauri.conf.json 里 updater.endpoints 同一个地址）。
/// 顶层 `version` 字段跟平台无关，macOS 只读它来比较版本号。
const LATEST_JSON_URL: &str =
    "https://github.com/mrzym99/Wordpeek/releases/latest/download/latest.json";

/// 已缓存的可用更新（启动 / 手动检查后写入）
pub(crate) type UpdateState = parking_lot::Mutex<Option<Update>>;

/// macOS 手动更新：最近一次检查到的远端最新版本号（启动 / 手动检查后写入）
pub(crate) type LatestVersionState = parking_lot::Mutex<Option<String>>;

#[derive(serde::Serialize, Debug)]
pub(crate) struct UpdateStatus {
    available: bool,
    current_version: String,
    new_version: Option<String>,
}

fn status_of(app: &AppHandle, update: Option<&Update>) -> UpdateStatus {
    UpdateStatus {
        available: update.is_some(),
        current_version: app.package_info().version.to_string(),
        new_version: update.map(|u| u.version.clone()),
    }
}

/// macOS 版的状态：只比较版本号，不缓存 Update（拿不到可安装的更新包）
fn manual_status_of(app: &AppHandle, latest: Option<&str>) -> UpdateStatus {
    let current_version = app.package_info().version.to_string();
    let newer = latest.filter(|v| is_newer(v, &current_version));
    UpdateStatus {
        available: newer.is_some(),
        current_version,
        new_version: newer.map(str::to_string),
    }
}

/// 解析 "2.2.0" / "v2.2.0" / "2.2.0-beta.1" → [2, 2, 0]。
/// 只比较数字段（预发布后缀不参与），解析失败返回 None
fn parse_version(v: &str) -> Option<Vec<u64>> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

/// remote 是否比 current 新（逐段比较，缺省段按 0 处理）
fn is_newer(remote: &str, current: &str) -> bool {
    let (Some(r), Some(c)) = (parse_version(remote), parse_version(current)) else {
        return false;
    };
    for i in 0..r.len().max(c.len()) {
        let rv = r.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if rv != cv {
            return rv > cv;
        }
    }
    false
}

/// macOS：读 GitHub Release 清单里的版本号，只判断是否可更新
async fn check_update_manual(app: &AppHandle) -> Result<UpdateStatus, String> {
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| e.to_string())?
        .get(LATEST_JSON_URL)
        .header("User-Agent", "Wordpeek-Updater")
        .send()
        .await
        .map_err(|e| format!("检查更新失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("检查更新失败：HTTP {}", resp.status()));
    }
    let manifest: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析版本信息失败：{e}"))?;
    let latest = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "版本信息缺少 version 字段".to_string())?
        .to_string();
    let status = manual_status_of(app, Some(&latest));
    println!("[updater] manual check: {status:?}");
    *app.state::<LatestVersionState>().lock() = Some(latest);
    Ok(status)
}

/// 检查更新并把结果缓存到应用状态（async：同步命令在主线程跑，block_on 会冻结整个 UI）
pub(crate) async fn check_update(app: &AppHandle) -> Result<UpdateStatus, String> {
    // macOS 没有可安装的更新包，只查版本号（设置页改提供跳转 Releases 的下载按钮）
    if cfg!(target_os = "macos") {
        return check_update_manual(app).await;
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let status = status_of(app, update.as_ref());
    println!("[updater] check: {status:?}");
    *app.state::<UpdateState>().lock() = update;
    Ok(status)
}

/// 设置页打开时查询缓存的检查结果（不发网络请求）
#[tauri::command]
pub(crate) fn get_update_status(app: AppHandle) -> UpdateStatus {
    if cfg!(target_os = "macos") {
        let latest = app.state::<LatestVersionState>().lock().clone();
        return manual_status_of(&app, latest.as_deref());
    }
    let cached = app.state::<UpdateState>().lock().as_ref().cloned();
    status_of(&app, cached.as_ref())
}

/// 手动检查更新（设置页「检查更新」按钮）。async 命令由 tauri 丢到异步运行时，网络等待不阻塞主线程
#[tauri::command]
pub(crate) async fn check_update_now(app: AppHandle) -> Result<UpdateStatus, String> {
    check_update(&app).await
}

/// 重启自己：改完系统权限必须换进程才生效；macOS「退出并重新打开」对终端起的进程无效，故用应用内按钮
#[tauri::command]
pub(crate) fn restart_app(app: AppHandle) {
    app.restart();
}

/// 更新下载进度（发给设置页画进度条；total 未知时前端显示不定态进度）
#[derive(serde::Serialize, Clone)]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

/// 后台线程下载并静默安装已缓存的更新，完成后重启。进度经 `update-progress` 事件按整数
/// 百分比/256KB 步进去重推送；失败发 `update-failed` 并把更新放回缓存——命令立即返回 Ok，线程内失败只能靠事件通知前端
#[tauri::command]
pub(crate) fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .state::<UpdateState>()
        .lock()
        .take()
        .ok_or_else(|| "没有已缓存的更新".to_string())?;
    std::thread::spawn(move || {
        println!("[updater] downloading & installing...");
        let emitter = app.clone();
        let mut downloaded: u64 = 0;
        let mut total: Option<u64> = None;
        let mut last_anchor: u64 = u64::MAX;
        let install = update.download_and_install(
            |chunk, t| {
                downloaded += chunk as u64;
                if total.is_none() {
                    total = t;
                }
                // 锚点：有总量按百分比（0-100），无总量按 256KB 步进
                let anchor = match total {
                    Some(t) if t > 0 => downloaded * 100 / t,
                    _ => downloaded / (256 * 1024),
                };
                if anchor != last_anchor {
                    last_anchor = anchor;
                    let _ = emitter.emit("update-progress", UpdateProgress { downloaded, total });
                }
            },
            || {},
        );
        if let Err(e) = tauri::async_runtime::block_on(install) {
            eprintln!("[updater] install failed: {e}");
            // 失败把已下载的更新放回缓存，用户重试不用再走一遍检查更新
            *app.state::<UpdateState>().lock() = Some(update);
            let _ = app.emit("update-failed", e.to_string());
            return;
        }
        // NSIS 静默安装结束后旧进程通常已被安装器接管，restart 兜底
        app.restart();
    });
    Ok(())
}
