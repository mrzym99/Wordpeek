//! macOS 平台实现：系统权限检测、屏幕信息缓存、光标/工作区（CGEvent 坐标系）、
//! Cmd+C 复制注入（需辅助功能权限）、CGDisplay 抓屏合成。

use std::sync::OnceLock;

use core_graphics::display::CGDisplay;
use objc2::rc::Retained;

use crate::capture::{bgra_to_rgba, CapturedScreen};

// ---------- 系统权限 ----------

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const std::ffi::c_void;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// 辅助功能权限：模拟 Cmd+C 抓词必需
pub fn accessibility_allowed() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// 未授权时弹系统授权对话框（把当前进程/它的父应用加进「辅助功能」列表）。
/// 没有这个提示，用户只能自己去系统设置里手动加，而 dev 模式下真正需要授权的是
/// 启动应用的终端，不是 Wordpeek 自己——很容易白折腾一轮。
pub fn request_accessibility() -> bool {
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};

    unsafe {
        let key: &NSString = &*(kAXTrustedCheckOptionPrompt as *const NSString);
        let yes = NSNumber::new_bool(true);
        let dict = NSDictionary::from_slices(&[key], &[&*yes as &AnyObject]);
        AXIsProcessTrustedWithOptions(Retained::as_ptr(&dict) as *const std::ffi::c_void)
    }
}

/// 屏幕录制权限：截图翻译必需
pub fn screen_capture_allowed() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// 未授权时弹系统授权框，并把 Wordpeek 登记进「屏幕录制」列表。
/// 只 preflight 不 request 的话，列表里根本没有 Wordpeek 这一项，用户无从勾选。
pub fn request_screen_capture() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}

// ---------- 启动初始化 ----------

/// macOS 托盘常驻：不在 Dock 显示图标；主线程缓存屏幕信息
/// （NSScreen 只能主线程访问，之后各线程只读缓存）
pub fn init(app: &tauri::AppHandle) {
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    init_mac_screens();
    // 首次启动直接把系统授权对话框弹出来，别让用户去猜要给谁勾权限
    if !accessibility_allowed() {
        let trusted = request_accessibility();
        println!("[perm] 辅助功能未授权（trusted={trusted}）：请在系统设置 → 隐私与安全性 → 辅助功能中授权") ;
    }
}

/// 截图遮罩：显示在全部 Space（含全屏应用所在空间）上，否则切到别的桌面就看不见遮罩
pub fn prepare_overlay(win: &tauri::WebviewWindow) {
    let _ = win.set_visible_on_all_workspaces(true);
}

/// macOS 屏幕信息（点坐标，左上原点，与 CGEvent 同一坐标系；
/// 混合 DPI 多屏尽力而为）
struct MacScreen {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    wx: f64,
    wy: f64,
    ww: f64,
    wh: f64,
    scale: f64,
}

static MAC_SCREENS: OnceLock<Vec<MacScreen>> = OnceLock::new();

fn init_mac_screens() {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[mac] init_mac_screens 不在主线程，跳过");
        return;
    };
    let arr = NSScreen::screens(mtm);
    // NS 坐标系左下原点且主屏左下角为 (0,0)；CG/CGEvent 左上原点：y_cg = 主屏高 - y_ns
    let primary_h = arr
        .firstObject()
        .map(|s| s.frame().size.height)
        .unwrap_or(0.0);
    let mut out = Vec::new();
    for s in arr.iter() {
        let f = s.frame();
        let vf = s.visibleFrame();
        let scale = s.backingScaleFactor();
        out.push(MacScreen {
            x: f.origin.x,
            y: primary_h - f.origin.y - f.size.height,
            w: f.size.width,
            h: f.size.height,
            wx: vf.origin.x,
            wy: primary_h - vf.origin.y - vf.size.height,
            ww: vf.size.width,
            wh: vf.size.height,
            scale,
        });
    }
    println!("[mac] screens: {} 个, 缓存工作区", out.len());
    let _ = MAC_SCREENS.set(out);
}

// ---------- 光标与工作区 ----------

pub fn cursor_pos() -> (i32, i32) {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    // 造一个合成事件只为读光标位置（等价 CGEventCreate(None)）
    let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
        return (100, 100);
    };
    let Ok(ev) = CGEvent::new(source) else {
        return (100, 100);
    };
    let p = ev.location();
    // CGEvent location 是点（左上原点），物理像素 = 点 × 所在屏的缩放
    let scale = MAC_SCREENS
        .get()
        .and_then(|screens| {
            screens
                .iter()
                .find(|s| p.x >= s.x && p.x < s.x + s.w && p.y >= s.y && p.y < s.y + s.h)
        })
        .map(|s| s.scale)
        .unwrap_or(2.0);
    ((p.x * scale).round() as i32, (p.y * scale).round() as i32)
}

/// 光标所在显示器的工作区（物理像素，排除菜单栏/Dock）。
/// 拿不到信息时返回 (0,0,0,0)，前端会退回用虚拟屏边界锚制。
pub fn work_area_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    if let Some(screens) = MAC_SCREENS.get() {
        for s in screens {
            let sx = (s.x * s.scale).round() as i32;
            let sy = (s.y * s.scale).round() as i32;
            let sw = (s.w * s.scale).round() as i32;
            let sh = (s.h * s.scale).round() as i32;
            if x >= sx && x < sx + sw && y >= sy && y < sy + sh {
                return (
                    (s.wx * s.scale).round() as i32,
                    (s.wy * s.scale).round() as i32,
                    (s.ww * s.scale).round() as i32,
                    (s.wh * s.scale).round() as i32,
                );
            }
        }
    }
    (0, 0, 0, 0)
}

// ---------- 复制注入 ----------

/// 注入 Cmd+C。
///
/// 直接用 CGEvent 发键盘事件：enigo 的 Unicode 路径只设置字符、不设置物理键位，
/// AppKit 应用匹配不到「复制」菜单命令（实测 TextEdit 里选中文本后无任何反应），
/// 而 Cmd+C 是快捷键匹配，与字符无关。
/// 事件自带 flags，目标应用看到的就是这里设的修饰键，不必先物理松开用户按着的 Ctrl/Alt。
pub fn inject_copy(_app: &tauri::AppHandle) -> bool {
    use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    // 8 = 物理键位 'c'（US 布局）
    const C_KEY: u16 = 8;
    let Ok(source) = CGEventSource::new(CGEventSourceStateID::CombinedSessionState) else {
        return false;
    };
    let (Ok(down), Ok(up)) = (
        CGEvent::new_keyboard_event(source.clone(), C_KEY, true),
        CGEvent::new_keyboard_event(source, C_KEY, false),
    ) else {
        return false;
    };
    down.set_flags(CGEventFlags::CGEventFlagCommand);
    up.set_flags(CGEventFlags::CGEventFlagCommand);
    down.post(CGEventTapLocation::HID);
    up.post(CGEventTapLocation::HID);
    true
}

// ---------- 虚拟屏抓取 ----------

/// 屏幕录制权限未授权时 CGDisplayCreateImage 会返回内容无效的占位图，提前拦截并报错
pub fn capture_virtual_screen() -> Result<CapturedScreen, String> {
    if !screen_capture_allowed() {
        return Err(
            "屏幕录制权限未授权：请在 系统设置 → 隐私与安全性 → 屏幕录制 中勾选 Wordpeek 后重启应用"
                .into(),
        );
    }

    let displays = CGDisplay::active_displays().map_err(|e| format!("获取显示器列表失败: {e}"))?;
    if displays.is_empty() {
        return Err("没有可用的显示器".into());
    }

    // 虚拟屏边界（点，左上原点）与每屏像素缩放（Retina 为 2.0）；混合 DPI 时取最大缩放，尽力而为
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut scale = 1.0f64;
    for &id in &displays {
        let d = CGDisplay::new(id);
        let b = d.bounds();
        min_x = min_x.min(b.origin.x);
        min_y = min_y.min(b.origin.y);
        max_x = max_x.max(b.origin.x + b.size.width);
        max_y = max_y.max(b.origin.y + b.size.height);
        let s = display_scale(&d);
        if s > scale {
            scale = s;
        }
    }
    let vw = max_x - min_x;
    let vh = max_y - min_y;
    if vw <= 0.0 || vh <= 0.0 {
        return Err("获取虚拟屏幕尺寸失败".into());
    }

    let cw = (vw * scale).round() as usize;
    let ch = (vh * scale).round() as usize;
    let mut rgba = vec![0u8; cw * ch * 4];

    for &id in &displays {
        let d = CGDisplay::new(id);
        let b = d.bounds();
        let Some(img) = d.image() else {
            return Err(format!("抓取显示器 {} 失败", id));
        };
        let (iw, ih) = (img.width() as i32, img.height() as i32);
        // CGImage 每行可能有对齐填充（bytesPerRow > width*4），按行重排成紧凑 RGBA，
        // 否则整幅图会斜着错位、OCR 一行都认不出
        let bpr = img.bytes_per_row();
        let data = img.data();
        let raw = data.bytes();
        let stride = (iw as usize) * 4;
        if bpr < stride || raw.len() < bpr * ih as usize {
            return Err("抓屏像素不完整".into());
        }
        let mut disp = Vec::with_capacity(stride * ih as usize);
        for row in 0..ih as usize {
            disp.extend_from_slice(&raw[row * bpr..row * bpr + stride]);
        }
        bgra_to_rgba(&mut disp);

        // 该屏在画布上的位置与目标尺寸（物理像素，统一缩放到画布 scale）
        let dw = ((b.size.width * scale).round() as i32).max(1);
        let dh = ((b.size.height * scale).round() as i32).max(1);
        let dx = ((b.origin.x - min_x) * scale).round() as i32;
        let dy = ((b.origin.y - min_y) * scale).round() as i32;
        let piece = scale_rgba(&disp, iw, ih, dw, dh);
        // 逐行拷入画布，越界部分裁掉
        for row in 0..dh {
            let cy = dy + row;
            if cy < 0 || cy >= ch as i32 {
                continue;
            }
            let copy_w = dw.min(cw as i32 - dx);
            if copy_w <= 0 {
                continue;
            }
            let src = (row * dw * 4) as usize;
            let dst = (cy as usize * cw + dx as usize) * 4;
            rgba[dst..dst + copy_w as usize * 4]
                .copy_from_slice(&piece[src..src + copy_w as usize * 4]);
        }
    }

    Ok(CapturedScreen {
        width: cw as i32,
        height: ch as i32,
        origin_x: (min_x * scale).round() as i32,
        origin_y: (min_y * scale).round() as i32,
        rgba,
    })
}

/// 显示器物理像素 / 逻辑点（Retina 通常 2.0）。
/// 不能用 CGDisplayPixelsWide：「更多空间」这类缩放模式下它返回的是点数（本机 1512），
/// 会让遮罩窗口只铺满屏幕四分之一，裁剪坐标也跟着差一倍
fn display_scale(d: &CGDisplay) -> f64 {
    let w = d.bounds().size.width;
    if w <= 0.0 {
        return 1.0;
    }
    d.display_mode()
        .map(|m| m.pixel_width() as f64 / w)
        .filter(|s| *s > 0.0)
        .unwrap_or(1.0)
}

/// 虚拟屏边界（物理像素）
pub fn virtual_screen_bounds() -> Option<(i32, i32, i32, i32)> {
    let displays = CGDisplay::active_displays().ok()?;
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut scale = 1.0f64;
    for &id in &displays {
        let d = CGDisplay::new(id);
        let b = d.bounds();
        min_x = min_x.min(b.origin.x);
        min_y = min_y.min(b.origin.y);
        max_x = max_x.max(b.origin.x + b.size.width);
        max_y = max_y.max(b.origin.y + b.size.height);
        let s = display_scale(&d);
        if s > scale {
            scale = s;
        }
    }
    ((max_x > min_x) && (max_y > min_y)).then(|| {
        (
            (min_x * scale).round() as i32,
            (min_y * scale).round() as i32,
            ((max_x - min_x) * scale).round() as i32,
            ((max_y - min_y) * scale).round() as i32,
        )
    })
}

/// 最近邻缩放（macOS 多屏合成用；混合 DPI 尽力而为）
fn scale_rgba(src: &[u8], sw: i32, sh: i32, dw: i32, dh: i32) -> Vec<u8> {
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 4];
    for dy in 0..dh {
        let sy = (dy as i64 * sh as i64 / dh as i64) as i32;
        for dx in 0..dw {
            let sx = (dx as i64 * sw as i64 / dw as i64) as i32;
            let s = ((sy * sw + sx) * 4) as usize;
            let d = ((dy * dw + dx) * 4) as usize;
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}
