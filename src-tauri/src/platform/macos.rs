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

// ---------- 划词图标：全局鼠标监听 + 系统选区 ----------

/// 鼠标事件回调（进程内只装一次，装在静态里给 block 用）
type MouseCallback = Box<dyn Fn() + Send + Sync>;
static MOUSE_ON_DOWN: OnceLock<MouseCallback> = OnceLock::new();
static MOUSE_ON_UP: OnceLock<MouseCallback> = OnceLock::new();

/// 装全局鼠标监听：左键按下/松开各回调一次。
///
/// 用 AppKit 的全局监视器，不用 CGEventTap：鼠标事件不需要额外授权，也不会
/// 让全系统的鼠标事件排队等我们的回调。监视器必须在主线程注册，返回的 token
/// 一旦释放监视器就失效——泄漏掉最省心（进程活多久它就活多久）。
pub fn start_mouse_watch(on_down: MouseCallback, on_up: MouseCallback) -> bool {
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;

    let _ = MOUSE_ON_DOWN.set(on_down);
    let _ = MOUSE_ON_UP.set(on_up);
    {
        let down = block2::RcBlock::new(|_e: NonNull<NSEvent>| {
            if let Some(cb) = MOUSE_ON_DOWN.get() {
                cb()
            }
        });
        let up = block2::RcBlock::new(|_e: NonNull<NSEvent>| {
            if let Some(cb) = MOUSE_ON_UP.get() {
                cb()
            }
        });
        let d = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::LeftMouseDown, &down);
        let u = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::LeftMouseUp, &up);
        let ok = d.is_some() && u.is_some();
        if let Some(t) = d {
            std::mem::forget(t);
        }
        if let Some(t) = u {
            std::mem::forget(t);
        }
        std::mem::forget(down);
        std::mem::forget(up);
        ok
    }
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFGetTypeID(cf: *const std::ffi::c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFRelease(cf: *const std::ffi::c_void);
    fn CFRetain(cf: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn CFArrayGetCount(arr: *const std::ffi::c_void) -> isize;
    fn CFArrayGetValueAtIndex(arr: *const std::ffi::c_void, idx: isize) -> *const std::ffi::c_void;
    fn CFArrayGetTypeID() -> usize;
    static kCFBooleanTrue: *const std::ffi::c_void;
    static kCFBooleanFalse: *const std::ffi::c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> *const std::ffi::c_void;
    fn AXUIElementCopyAttributeValue(
        element: *const std::ffi::c_void,
        attribute: *const std::ffi::c_void,
        value: *mut *const std::ffi::c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *const std::ffi::c_void,
        attribute: *const std::ffi::c_void,
        value: *const std::ffi::c_void,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: *const std::ffi::c_void,
        attribute: *const std::ffi::c_void,
        parameter: *const std::ffi::c_void,
        value: *mut *const std::ffi::c_void,
    ) -> i32;
    fn AXValueCreate(the_type: u32, value: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn AXValueGetValue(value: *const std::ffi::c_void, the_type: u32, out: *mut std::ffi::c_void) -> bool;
}

/// AXValueType::CFRange
const AX_VALUE_CFRANGE: u32 = 4;

/// 读焦点控件里被选中的文本（辅助功能 API，不碰剪贴板也不注入按键）。
///
/// 三步，越往后越慢，所以前面的不中才走后面：
/// 1. 当前前台 app 的 AXFocusedUIElement 直接读 AXSelectedText——原生 app 一次就中；
/// 2. 中不了就先叫醒它的无障碍树（Chromium / Electron 靠这个才建树），等一小会儿再读；
/// 3. 还不行就按窗口往下翻几层找带选区的元素（有些 app 焦点元素是个空壳容器）。
///
/// 系统级元素（AXUIElementCreateSystemWide）在非 GUI 进程里稳定返回 -25204
/// CannotComplete，所以一律按前台 app 的 pid 建元素。终端、conhost 这类根本没有
/// 选区概念的应用三步都读不到，返回 None——那时不弹图标，用户照常用划词快捷键。
pub fn focused_selection() -> Option<String> {
    use objc2_app_kit::NSWorkspace;

    if !accessibility_allowed() {
        return None;
    }
    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .processIdentifier();
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return None;
        }
        let mut text = read_focused(app);
        if text.is_none() && mark_woken(pid) {
            wake_accessibility(app);
            std::thread::sleep(std::time::Duration::from_millis(100));
            text = read_focused(app);
            restore_accessibility(app);
        }
        if text.is_none() {
            text = scan_window(app, 4, 48);
        }
        CFRelease(app);
        text
    }
}

/// 读前台 app 焦点元素里的选区。
unsafe fn read_focused(app: *const std::ffi::c_void) -> Option<String> {
    let focused = ax_attr(app, "AXFocusedUIElement")?;
    let text = selection_in(focused);
    CFRelease(focused);
    text
}

/// 读一个元素自己的选区：AXSelectedText 优先，没有就用「选区范围 + AXStringForRange」。
unsafe fn selection_in(el: *const std::ffi::c_void) -> Option<String> {
    let text = ax_string_attr(el, "AXSelectedText")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| ax_string_for_selection(el));
    text.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// 从焦点窗口往下翻，找第一个带选区的元素。`depth`/`budget` 卡住开销：
/// 每次属性读都是一趟进程间通信，翻太宽会比不弹图标还糟。
unsafe fn scan_window(app: *const std::ffi::c_void, depth: usize, budget: usize) -> Option<String> {
    let win = ax_attr(app, "AXFocusedWindow")?;
    let mut left = budget;
    let found = scan_children(win, depth, &mut left);
    CFRelease(win);
    found
}

unsafe fn scan_children(
    el: *const std::ffi::c_void,
    depth: usize,
    left: &mut usize,
) -> Option<String> {
    if depth == 0 || *left == 0 {
        return None;
    }
    let mut kids = ax_children(el, 16);
    for kid in kids.iter() {
        if *left == 0 {
            break;
        }
        *left -= 1;
        if let Some(text) = selection_in(*kid) {
            release_all(&mut kids);
            return Some(text);
        }
        if let Some(text) = scan_children(*kid, depth - 1, left) {
            release_all(&mut kids);
            return Some(text);
        }
    }
    release_all(&mut kids);
    None
}

unsafe fn ax_children(el: *const std::ffi::c_void, limit: isize) -> Vec<*const std::ffi::c_void> {
    let mut kids = Vec::new();
    if let Some(arr) = ax_attr(el, "AXChildren") {
        if CFGetTypeID(arr) == CFArrayGetTypeID() {
            for i in 0..CFArrayGetCount(arr).min(limit) {
                let child = CFArrayGetValueAtIndex(arr, i);
                if !child.is_null() {
                    kids.push(CFRetain(child));
                }
            }
        }
        CFRelease(arr);
    }
    kids
}

unsafe fn release_all(items: &mut Vec<*const std::ffi::c_void>) {
    for item in items.drain(..) {
        CFRelease(item);
    }
}

/// AXSelectedTextRange + AXStringForRange：部分 app（PDF 阅读器、老编辑器）
/// 不实现 AXSelectedText，但实现了这套参数化属性。
unsafe fn ax_string_for_selection(el: *const std::ffi::c_void) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    let range_value = ax_attr(el, "AXSelectedTextRange")?;
    let mut range = CFRange { location: 0, length: 0 };
    let ok = AXValueGetValue(
        range_value,
        AX_VALUE_CFRANGE,
        &mut range as *mut CFRange as *mut std::ffi::c_void,
    );
    CFRelease(range_value);
    if !ok || range.length <= 0 {
        return None;
    }
    let param = AXValueCreate(AX_VALUE_CFRANGE, &range as *const CFRange as *const std::ffi::c_void);
    if param.is_null() {
        return None;
    }
    let attr = CFString::new("AXStringForRange");
    let mut out: *const std::ffi::c_void = std::ptr::null();
    let err = AXUIElementCopyParameterizedAttributeValue(
        el,
        attr.as_concrete_TypeRef() as CFStringRef as *const std::ffi::c_void,
        param,
        &mut out,
    );
    CFRelease(param);
    if err != 0 || out.is_null() {
        return None;
    }
    if CFGetTypeID(out) != CFStringGetTypeID() {
        CFRelease(out);
        return None;
    }
    Some(CFString::wrap_under_create_rule(out as CFStringRef).to_string())
}

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

/// 每个进程只叫醒一次：设置 AXEnhancedUserInterface 会触发 Chromium / Electron
/// 的「检测到读屏软件」提示（VS Code 会弹一次「是否启用无障碍支持」），反复设置
/// 就是在骚扰用户。树建起来之后本来也不会再塌，所以一次够用。
fn mark_woken(pid: i32) -> bool {
    static WOKEN: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashSet<i32>>> =
        std::sync::OnceLock::new();
    WOKEN
        .get_or_init(Default::default)
        .lock()
        .insert(pid)
}

/// 叫醒目标 app 的无障碍树。Chromium / Electron 平时不建树，看到
/// AXEnhancedUserInterface（Chromium 认的还有 AXManualAccessibility）才开工。
unsafe fn wake_accessibility(app: *const std::ffi::c_void) {
    set_ax_bool(app, "AXEnhancedUserInterface", true);
    set_ax_bool(app, "AXManualAccessibility", true);
}

/// 读完把增强界面关回去，别把别的 app 长期留在无障碍模式里。
/// AXManualAccessibility 没有「关」的说法，留着不碍事。
unsafe fn restore_accessibility(app: *const std::ffi::c_void) {
    set_ax_bool(app, "AXEnhancedUserInterface", false);
}

unsafe fn set_ax_bool(el: *const std::ffi::c_void, name: &str, value: bool) {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    let attr = CFString::new(name);
    let flag = if value { kCFBooleanTrue } else { kCFBooleanFalse };
    AXUIElementSetAttributeValue(
        el,
        attr.as_concrete_TypeRef() as CFStringRef as *const std::ffi::c_void,
        flag,
    );
}

/// 读一个 AX 属性，返回 create rule 的 CF 引用（调用方负责 CFRelease）
unsafe fn ax_attr(el: *const std::ffi::c_void, name: &str) -> Option<*const std::ffi::c_void> {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    let attr = CFString::new(name);
    let mut value: *const std::ffi::c_void = std::ptr::null();
    let err = AXUIElementCopyAttributeValue(
        el,
        attr.as_concrete_TypeRef() as CFStringRef as *const std::ffi::c_void,
        &mut value,
    );
    if err != 0 || value.is_null() {
        None
    } else {
        Some(value)
    }
}

/// 读一个 AX 字符串属性（不是 CFString 的属性一律当没有）
unsafe fn ax_string_attr(el: *const std::ffi::c_void, name: &str) -> Option<String> {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    let value = ax_attr(el, name)?;
    if CFGetTypeID(value) != CFStringGetTypeID() {
        CFRelease(value);
        return None;
    }
    let s = CFString::wrap_under_create_rule(value as CFStringRef).to_string();
    Some(s)
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
        // CGDisplayCreateImage 返回的是 autorelease 对象：worker 线程没有 autorelease pool，
        // 不自己圈一块就会攒到线程结束，连拍十几张能把 RSS 顶到几百 MB
        objc2::rc::autoreleasepool(|_| {
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

            // 该屏在画布上的位置与目标尺寸（物理像素，统一缩放到画布 scale）
            let dw = ((b.size.width * scale).round() as i32).max(1);
            let dh = ((b.size.height * scale).round() as i32).max(1);
            let dx = ((b.origin.x - min_x) * scale).round() as i32;
            let dy = ((b.origin.y - min_y) * scale).round() as i32;

            // 常见情况（单屏 / 各屏缩放一致）：不需要重采样，直接逐行转 BGRA→RGBA 写进画布，
            // 省掉 disp 与 scale_rgba 两份全屏缓冲（3024x1964 各约 24MB）
            if dw == iw && dh == ih {
                for row in 0..dh {
                    let cy = dy + row;
                    if cy < 0 || cy >= ch as i32 {
                        continue;
                    }
                    let copy_w = dw.min(cw as i32 - dx).max(0) as usize;
                    if copy_w == 0 {
                        continue;
                    }
                    let src_row = (row as usize) * bpr;
                    let dst = (cy as usize * cw + dx as usize) * 4;
                    for col in 0..copy_w {
                        let s = src_row + col * 4;
                        let o = dst + col * 4;
                        rgba[o] = raw[s + 2];
                        rgba[o + 1] = raw[s + 1];
                        rgba[o + 2] = raw[s];
                        rgba[o + 3] = 255;
                    }
                }
                return Ok(());
            }

            let mut disp = Vec::with_capacity(stride * ih as usize);
            for row in 0..ih as usize {
                disp.extend_from_slice(&raw[row * bpr..row * bpr + stride]);
            }
            bgra_to_rgba(&mut disp);
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
            Ok(())
        })?;
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
