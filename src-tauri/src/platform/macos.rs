//! macOS 平台实现：系统权限检测、屏幕信息缓存、光标/工作区（CGEvent 坐标系）、
//! Cmd+C 复制注入（需辅助功能权限）、CGDisplay 抓屏合成。

use std::sync::OnceLock;

use core_graphics::display::CGDisplay;
use objc2::rc::Retained;

use crate::capture::{bgra_to_rgba, CapturedScreen};

use super::{ClipboardBackup, PermissionKind, SelectionRead};

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

/// 未授权时弹系统授权对话框；没有这个提示，dev 模式下用户常白折腾——真正需要授权的往往是启动应用的终端
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

// ---------- 跳系统设置面板 ----------

/// macOS 13 Ventura 起隐私设置换成了新扩展；旧 scheme 的 ?Privacy_* 锚点在新系统上
/// 会被丢弃（两个权限都落到第一个面板），所以 13+ 必须用扩展 ID。
const PRIVACY_EXT: &str = "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension";
/// macOS 11/12 只有旧版安全面板
const LEGACY_SECURITY: &str = "x-apple.systempreferences:com.apple.preference.security";

/// 系统主版本号（27.0.1 → 27）
fn macos_major_version() -> isize {
    use objc2_foundation::NSProcessInfo;

    NSProcessInfo::processInfo().operatingSystemVersion().majorVersion
}

/// 权限面板深链（纯函数，版本作参数便于单测）
pub(crate) fn settings_url(kind: PermissionKind, macos_major: isize) -> String {
    let prefix = if macos_major >= 13 {
        PRIVACY_EXT
    } else {
        LEGACY_SECURITY
    };
    format!("{prefix}?{}", kind.anchor())
}

/// 打开对应权限的系统设置面板
pub fn open_permission_settings(
    app: &tauri::AppHandle,
    kind: PermissionKind,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(settings_url(kind, macos_major_version()), None::<&str>)
        .map_err(|e| format!("打开系统设置失败：{e}"))
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

/// 光标所在显示器的工作区（物理像素，排除菜单栏/Dock）；拿不到时返回 (0,0,0,0)，
/// 前端会退回用虚拟屏边界锚制。
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

/// 注入 Cmd+C。用 CGEvent 发键盘事件：enigo 的 Unicode 路径只设字符、不设物理键位，
/// AppKit 应用匹配不到「复制」菜单命令（实测 TextEdit 无反应）；Cmd+C 是快捷键匹配，与字符无关。
/// 事件自带 flags，不必先物理松开用户按着的修饰键
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

// ---------- 剪贴板原语（热键路径 grab_selection 用；与 win_clipboard.rs 同名同签名） ----------

/// 粘贴板变更序号（NSPasteboard changeCount）：任何人写入都会变，等价 Windows 的
/// GetClipboardSequenceNumber——「复制是否生效」的信号，读取不加锁、轮询零成本
pub fn clipboard_seq() -> u64 {
    use objc2_app_kit::NSPasteboard;
    NSPasteboard::generalPasteboard().changeCount() as u64
}

/// 读粘贴板当前文本。
/// `Ok(Some(t))` 有文本；`Ok(None)` **确定是空粘贴板**（types() 为空）；
/// `Err(())` 重试后仍失败（内容未知），或确定有非文本内容（图片/文件——无法备份，
/// 照常恢复到空态会毁掉用户内容，只能放弃）
pub fn clipboard_read_text(retries: u32) -> Result<Option<String>, ()> {
    for attempt in 0..=retries {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let Ok(mut cb) = arboard::Clipboard::new() else {
            continue;
        };
        match cb.get_text() {
            Ok(t) => return Ok(Some(t)),
            // 「格式不可用」要再分：空粘贴板（types() 为空/None）可放心按「原本为空」
            // 备份/恢复；有非文本内容（图片/文件）则无法备份，只能放弃本轮。
            // arboard 的 ContentNotAvailable 分不清这两种，用 NSPasteboard types() 补判
            Err(arboard::Error::ContentNotAvailable) => {
                use objc2_app_kit::NSPasteboard;
                let count = NSPasteboard::generalPasteboard()
                    .types()
                    .map(|t| t.len())
                    .unwrap_or(0);
                if count == 0 {
                    return Ok(None);
                }
                println!("[clip] 粘贴板有非文本内容（图片/文件），无法备份，放弃本轮");
                return Err(());
            }
            Err(_) => {
                // 其余错误（被占用等）按瞬时故障继续重试；末次打点，避免静默 Err 无从排查
                if attempt == retries {
                    println!("[clip] 粘贴板读不出内容（arboard 错误，重试耗尽）");
                }
                continue;
            }
        }
    }
    Err(())
}

/// 备份剪贴板当前内容（grab 用；Err = 内容未知或还原不了，绝不能动它）；macOS 只区分文本/空
pub fn clipboard_backup(retries: u32) -> Result<ClipboardBackup, ()> {
    match clipboard_read_text(retries) {
        Ok(Some(t)) => Ok(ClipboardBackup::Text(t)),
        Ok(None) => Ok(ClipboardBackup::Empty),
        Err(()) => Err(()),
    }
}

/// 恢复用户原内容：Empty 清空，Text 写回文本（空串≈清空）；Raw 只在 Windows 产生
pub fn clipboard_restore(backup: &ClipboardBackup) -> bool {
    for attempt in 0..3u32 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let Ok(mut cb) = arboard::Clipboard::new() else {
            continue;
        };
        let ok = match backup {
            ClipboardBackup::Text(t) => cb.set_text(t.as_str()).is_ok(),
            // Raw 仅 Windows 侧产生，macOS 不可达；真收到则退化为清空
            #[cfg(windows)]
            ClipboardBackup::Raw(_) => cb.clear().is_ok(),
            ClipboardBackup::Empty => cb.clear().is_ok(),
        };
        if ok {
            return true;
        }
    }
    false
}

/// macOS 粘贴板没有廉价的「谁写入」归属查询；watcher 靠「新文本逐字等于本次划词」
/// 已足够保守，恒 false（Windows 侧的自查只是多一层防御）
pub fn clipboard_owner_is_self() -> bool {
    false
}

// ---------- 划词图标：全局鼠标监听 + 系统选区 ----------

// 回调槽（MOUSE_ON_*）在 super（platform/mod.rs）共用；MOUSE_ON_MOVE 在 macOS 未用（见 super 的 MoveCallback 注释）
use super::{MOUSE_ON_DOWN, MOUSE_ON_UP, MouseCallback, MoveCallback};

/// 装全局鼠标监听：左键按下/松开各回调一次。用 AppKit 全局监视器而非 CGEventTap：鼠标事件
/// 不需要额外授权，也不会让全系统鼠标事件排队等回调。监视器必须主线程注册，token 一释放
/// 监视器就失效——泄漏掉最省心（进程活多久它就活多久）。
pub fn start_mouse_watch(
    on_down: MouseCallback,
    on_up: MouseCallback,
    _on_move: MoveCallback,
) -> bool {
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
    fn AXUIElementSetMessagingTimeout(element: *const std::ffi::c_void, timeout: f32) -> i32;
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

/// 后台线程执行 AppKit 代码前开 autorelease pool（worker 线程没有主线程那个池）
pub fn with_release_pool<R>(f: impl FnOnce() -> R) -> R {
    objc2::rc::autoreleasepool(|_| f())
}

/// 系统设置里的「双击间隔」（毫秒）
pub fn double_click_interval_ms() -> u64 {
    use objc2_app_kit::NSEvent;
    (NSEvent::doubleClickInterval() * 1000.0).round().max(0.0) as u64
}

/// 前台 app 的 pid（系统级 AX 元素不可用，一律按它建元素）
pub fn foreground_pid() -> Option<i32> {
    use objc2_app_kit::NSWorkspace;
    Some(NSWorkspace::sharedWorkspace().frontmostApplication()?.processIdentifier())
}

// macOS 的 Cmd+C 在任何 app 里都是「复制」（不像 Windows 控制台的 Ctrl+C 是中断），兜底一律允许
// ——该判断已并入 foreground_context，这里不再单独导出

// 系统安全输入是否开启中（登录窗/密码框/带安全横幅的输入）：AX 读取被系统拦，
// 模拟 Cmd+C 会被吞掉且可能触发安全告警。
// 注意：SecIsSecureInputProcess 在 arm64 的 ApplicationServices 里**没有导出符号**
// （只有 x86_64 有），链接期引用直接 undefined symbol，只能运行时 dlsym 解析；
// 查不到按「未开启」降级——这只是安全闸加固项，不能反过来拦下正常翻译
fn secure_input_active() -> bool {
    // dlopen/dlsym 在 libSystem 里，macOS 恒链接，无需 #[link]
    extern "C" {
        fn dlopen(filename: *const std::ffi::c_char, flag: i32) -> *mut std::ffi::c_void;
        fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::ffi::c_char)
            -> *mut std::ffi::c_void;
    }

    unsafe {
        let path =
            b"/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices\0";
        let handle = dlopen(path.as_ptr() as *const std::ffi::c_char, 1); // 1 = RTLD_LAZY
        if handle.is_null() {
            return false;
        }
        let sym = dlsym(handle, b"SecIsSecureInputProcess\0".as_ptr() as *const std::ffi::c_char);
        if sym.is_null() {
            return false;
        }
        // 参数名沿用系统头文件里的 isSecure（函数指针类型的形参名只作文档，不参与链接）
        #[allow(non_snake_case)]
        let is_secure_input: extern "C" fn(isSecure: *mut bool) -> i32 =
            std::mem::transmute(sym);
        let mut secure = false;
        is_secure_input(&mut secure) == 0 && secure
    }
}

/// 前台应用上下文一次性查询（热键兜底注入前的安全闸）。macOS 无老控制台/管理员
/// 完整性问题（恒允许/恒否），密码框由 Secure Input 覆盖
pub fn foreground_context() -> super::ForegroundContext {
    use objc2_app_kit::NSWorkspace;

    let app = NSWorkspace::sharedWorkspace().frontmostApplication();
    // as_ref 借用：下面 process_name 还要用一次 app（Option<Retained> 不可 Copy）
    let pid = app.as_ref().map(|a| a.processIdentifier());
    let process_name = app
        .and_then(|a| a.localizedName().map(|n| n.to_string()))
        .unwrap_or_default();
    super::ForegroundContext {
        pid,
        process_name,
        fallback_allowed: true,
        elevated: false,
        password_focus: false,
        secure_input: secure_input_active(),
    }
}

/// 读焦点控件里被选中的文本（辅助功能 API，不碰剪贴板也不注入按键），返回门面统一四态
/// （hopeless 拉黑记账由门面统一做）。三步越往后越慢，前面的不中才走后面：1. 前台 app 的
/// AXFocusedUIElement 直接读 AXSelectedText；2. 先唤醒无障碍树（Chromium / Electron 靠
/// 这个才建树）再重试；3. 按窗口往下翻找带选区的元素（有的焦点元素是空壳容器）。
/// 系统级元素（AXUIElementCreateSystemWide）在非 GUI 进程稳定返回 -25204
/// CannotComplete，一律按前台 pid 建元素。
pub fn read_focused_selection() -> super::SelectionRead {
    if !accessibility_allowed() {
        return super::SelectionRead::Error;
    }
    let Some(pid) = foreground_pid() else {
        return super::SelectionRead::Error;
    };
    unsafe {
        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            return super::SelectionRead::Error;
        }
        // 默认 6 秒：目标 app 卡住时能把探测线程钉死，图标就得等 6 秒才弹
        AXUIElementSetMessagingTimeout(app, 0.1);
        let mut text: Option<String> = None;
        // 焦点元素自己回答过「我就是没有选区」：树必然已建好，唤醒纯属白等，跳过
        let mut answered_empty = false;
        // 焦点元素连选区属性都没有（非文本控件/树没建）——三步全落空时据此拉黑
        let mut unsupported_focus = false;
        match read_focused(app) {
            SelectionRead::Text(t) => text = Some(t),
            SelectionRead::NoSelection => answered_empty = true,
            SelectionRead::NoTextPattern => unsupported_focus = true,
            SelectionRead::Error => {}
        }
        if text.is_none() && !answered_empty && should_wake(pid) {
            wake_accessibility(app);
            // Chromium 建树是异步的（实测 100ms 常常不够）：每 80ms 试一次、最多 ~640ms。
            // 成功就不 restore，让树保持醒着，否则下次又得从零唤醒
            for _ in 0..8 {
                std::thread::sleep(std::time::Duration::from_millis(80));
                match read_focused(app) {
                    SelectionRead::Text(t) => {
                        text = Some(t);
                        break;
                    }
                    SelectionRead::NoSelection => {
                        answered_empty = true;
                        break;
                    }
                    _ => {}
                }
            }
            if text.is_some() {
                mark_wake_ok(pid);
            } else {
                restore_accessibility(app);
            }
        }
        if text.is_none() {
            text = scan_window(app, 4, 48);
        }
        CFRelease(app);
        // 四态收敛：Text 优先；「焦点元素回答过没有选区」= 支持、只是没选中；
        // 「连选区属性都没有」且哪儿都翻不到 = 不支持（门面据此拉黑）；其余算瞬时错误
        match text {
            Some(t) => super::SelectionRead::Text(t),
            None if answered_empty => super::SelectionRead::NoSelection,
            None if unsupported_focus => super::SelectionRead::NoTextPattern,
            None => super::SelectionRead::Error,
        }
    }
}

unsafe fn read_focused(app: *const std::ffi::c_void) -> super::SelectionRead {
    let focused = match ax_attr(app, "AXFocusedUIElement") {
        Some(f) => f,
        None => return super::SelectionRead::NoTextPattern,
    };
    let read = selection_in(focused);
    CFRelease(focused);
    read
}

/// 读一个元素自己的选区：AXSelectedText 优先，没有就用「选区范围 + AXStringForRange」。
/// 「属性读到了但是空」= 元素支持选区、只是这次没选中，绝不能跟「属性根本没有」混为一谈
unsafe fn selection_in(el: *const std::ffi::c_void) -> super::SelectionRead {
    use core_foundation::base::TCFType;
    use core_foundation::string::{CFString, CFStringRef};

    match ax_string_attr(el, "AXSelectedText") {
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                SelectionRead::NoSelection
            } else {
                SelectionRead::Text(t.to_string())
            }
        }
        None => {
            let range_value = match ax_attr(el, "AXSelectedTextRange") {
                Some(v) => v,
                None => return SelectionRead::NoTextPattern,
            };
            let mut range = CFRange { location: 0, length: 0 };
            let ok = AXValueGetValue(
                range_value,
                AX_VALUE_CFRANGE,
                &mut range as *mut CFRange as *mut std::ffi::c_void,
            );
            CFRelease(range_value);
            if !ok {
                return SelectionRead::Error;
            }
            if range.length <= 0 {
                return SelectionRead::NoSelection;
            }
            let param =
                AXValueCreate(AX_VALUE_CFRANGE, &range as *const CFRange as *const std::ffi::c_void);
            if param.is_null() {
                return SelectionRead::Error;
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
                return SelectionRead::Error;
            }
            if CFGetTypeID(out) != CFStringGetTypeID() {
                CFRelease(out);
                return SelectionRead::Error;
            }
            let s = CFString::wrap_under_create_rule(out as CFStringRef).to_string();
            let t = s.trim();
            if t.is_empty() {
                SelectionRead::NoSelection
            } else {
                SelectionRead::Text(t.to_string())
            }
        }
    }
}

/// 从焦点窗口往下翻，找第一个带选区的元素；depth/budget 卡开销——每次属性读都是一趟 IPC，翻太宽得不偿失。
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
        if let SelectionRead::Text(text) = selection_in(*kid) {
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

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

/// 唤醒成功：清掉该 app 的重试计数，下次需要时还能继续唤醒
fn mark_wake_ok(pid: i32) {
    wake_tries().lock().remove(&pid);
}

/// 唤醒尝试次数：每 app 最多 3 次。终端这类永不暴露选区的 app 唤醒也没用，
/// 不封顶就会每次划词白等 640ms。
fn should_wake(pid: i32) -> bool {
    let mut tries = wake_tries().lock();
    let n = tries.entry(pid).or_insert(0);
    if *n >= 3 {
        return false;
    }
    *n += 1;
    true
}

static WAKE_TRIES: std::sync::OnceLock<parking_lot::Mutex<std::collections::HashMap<i32, u8>>> =
    std::sync::OnceLock::new();

fn wake_tries() -> &'static parking_lot::Mutex<std::collections::HashMap<i32, u8>> {
    WAKE_TRIES.get_or_init(Default::default)
}

/// 叫醒目标 app 的无障碍树。Chromium / Electron 平时不建树，看到
/// AXEnhancedUserInterface（Chromium 认的还有 AXManualAccessibility）才开工。
unsafe fn wake_accessibility(app: *const std::ffi::c_void) {
    set_ax_bool(app, "AXEnhancedUserInterface", true);
    set_ax_bool(app, "AXManualAccessibility", true);
}

/// 读完把增强界面关回去，别把别的 app 长期留在无障碍模式里（AXManualAccessibility 没有「关」，留着不碍事）。
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

    // 虚拟屏边界与缩放（点，左上原点）；与 virtual_screen_bounds 共用一套计算
    let (min_x, min_y, max_x, max_y, scale) = virtual_screen_extents(&displays)
        .ok_or_else(|| "获取虚拟屏幕尺寸失败".to_string())?;
    let vw = max_x - min_x;
    let vh = max_y - min_y;

    let cw = (vw * scale).round() as usize;
    let ch = (vh * scale).round() as usize;
    let mut rgba = vec![0u8; cw * ch * 4];

    for &id in &displays {
        let d = CGDisplay::new(id);
        let b = d.bounds();
        // CGDisplayCreateImage 返回 autorelease 对象：worker 线程不自己圈 pool 会攒到线程结束，连拍能把 RSS 顶到几百 MB
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

            // 常见情况（单屏 / 各屏缩放一致）：直接逐行转 BGRA→RGBA 写进画布，
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

/// 虚拟屏边界（点，左上原点）与每屏像素缩放（Retina 为 2.0）；混合 DPI 时取最大缩放，尽力而为。
fn virtual_screen_extents(displays: &[u32]) -> Option<(f64, f64, f64, f64, f64)> {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut scale = 1.0f64;
    for &id in displays {
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
    (max_x > min_x && max_y > min_y).then_some((min_x, min_y, max_x, max_y, scale))
}

/// 虚拟屏边界（物理像素）
pub fn virtual_screen_bounds() -> Option<(i32, i32, i32, i32)> {
    let displays = CGDisplay::active_displays().ok()?;
    virtual_screen_extents(&displays).map(|(min_x, min_y, max_x, max_y, scale)| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_url_uses_privacy_extension_on_ventura_and_later() {
        assert_eq!(
            settings_url(PermissionKind::Accessibility, 27),
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility"
        );
        assert_eq!(
            settings_url(PermissionKind::ScreenCapture, 27),
            "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture"
        );
    }

    #[test]
    fn settings_url_keeps_legacy_scheme_before_ventura() {
        assert_eq!(
            settings_url(PermissionKind::Accessibility, 12),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        );
        assert_eq!(
            settings_url(PermissionKind::ScreenCapture, 12),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        );
    }

    #[test]
    fn permission_kind_parses_frontend_names() {
        assert_eq!(PermissionKind::parse("accessibility"), Some(PermissionKind::Accessibility));
        assert_eq!(PermissionKind::parse("screen_capture"), Some(PermissionKind::ScreenCapture));
        assert_eq!(PermissionKind::parse("microphone"), None);
    }
}
