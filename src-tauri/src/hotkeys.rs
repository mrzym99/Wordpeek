//! 全局快捷键：解析/校验/注册。从 main.rs 整体迁出（纯移动不改逻辑）。

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

/// 解析 "Alt+Z" 形式的快捷键字符串
pub(crate) fn parse_hotkey(s: &str) -> Result<Shortcut, String> {
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;
    for part in s.split('+') {
        let p = part.trim().to_lowercase();
        if p.is_empty() {
            continue;
        }
        match p.as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "super" | "win" | "meta" | "cmd" => mods |= Modifiers::SUPER,
            other => {
                if key.is_some() {
                    return Err(format!("无效的快捷键: {s}"));
                }
                key = Some(parse_code(other).ok_or_else(|| format!("不支持的按键: {part}"))?);
            }
        }
    }
    let key = key.ok_or_else(|| format!("无效的快捷键: {s}"))?;
    if mods.is_empty() {
        return Err(format!("快捷键必须包含 Ctrl/Alt/Shift 修饰键: {s}"));
    }
    Ok(Shortcut::new(Some(mods), key))
}

/// 支持单个字母、数字、F1-F12
fn parse_code(s: &str) -> Option<Code> {
    use std::str::FromStr;
    let up = s.to_uppercase();
    if up.len() == 1 {
        let c = up.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return Code::from_str(&format!("Key{c}")).ok();
        }
        if c.is_ascii_digit() {
            return Code::from_str(&format!("Digit{c}")).ok();
        }
        return None;
    }
    if up.starts_with('F') && up[1..].chars().all(|c| c.is_ascii_digit()) {
        return Code::from_str(&up).ok();
    }
    None
}

/// 快捷键两两不能重复（重复会导致行为歧义）；大小写与首尾空白不同视为同一个键
pub(crate) fn validate_hotkeys(hotkeys: &[&str]) -> Result<(), String> {
    let mut seen: Vec<String> = Vec::new();
    for h in hotkeys {
        let key = h.trim().to_lowercase();
        if seen.contains(&key) {
            return Err(format!("快捷键重复：{h}"));
        }
        seen.push(key);
    }
    Ok(())
}

/// 注册全部全局快捷键（先注销旧的全部再注册）；返回注册失败的键名（通常是键位被其他程序占用），
/// 保存配置时作为 Err 返回给设置页展示
pub(crate) fn register_hotkeys(app: &AppHandle, cfg: &crate::translate::AppConfig) -> Vec<String> {
    let mut failed = Vec::new();
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("[shortcut] 注销旧快捷键失败: {e}");
    }
    // 有重复直接全不注册
    if let Err(e) = validate_hotkeys(&[&cfg.hotkey, &cfg.screenshot_hotkey, &cfg.input_hotkey]) {
        eprintln!("[shortcut] {e}");
        return vec![e];
    }
    macro_rules! reg {
        ($hotkey:expr, $handler:expr) => {
            if let Ok(sc) = parse_hotkey(&$hotkey) {
                if let Err(e) = app.global_shortcut().on_shortcut(sc, $handler) {
                    eprintln!("[shortcut] {} 注册失败: {e}", $hotkey);
                    failed.push($hotkey.clone());
                }
            }
        };
    }
    reg!(cfg.hotkey, |app: &AppHandle, _s, event| {
        if event.state == ShortcutState::Pressed {
            crate::selection::handle_hotkey_trigger(app);
        }
    });
    reg!(cfg.screenshot_hotkey, |app: &AppHandle, _s, event| {
        if event.state == ShortcutState::Pressed {
            crate::screenshot::handle_screenshot_trigger(app);
        }
    });
    reg!(cfg.input_hotkey, |app: &AppHandle, _s, event| {
        if event.state == ShortcutState::Pressed {
            crate::windows::open_input_window(app);
        }
    });
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_common_hotkeys() {
        assert!(parse_hotkey("Ctrl+Alt+T").is_ok());
        assert!(parse_hotkey("Ctrl+Shift+D").is_ok());
        assert!(parse_hotkey("Alt+F2").is_ok());
        assert!(parse_hotkey("Ctrl+1").is_ok());
        assert!(parse_hotkey("D").is_err()); // 缺修饰键
        assert!(parse_hotkey("Ctrl+").is_err());
    }

    #[test]
    fn validate_hotkeys_rejects_duplicates() {
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+S", "Ctrl+Alt+I"]).is_ok());
        // 大小写/空白不同视为同一个键
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "ctrl+alt+t", "Ctrl+Alt+I"]).is_err());
        assert!(validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+S", "Ctrl+Alt+S"]).is_err());
        let err = validate_hotkeys(&["Ctrl+Alt+T", "Ctrl+Alt+T", "Ctrl+Alt+I"]).unwrap_err();
        assert!(err.contains("重复"), "错误信息应说明原因: {err}");
    }
}
