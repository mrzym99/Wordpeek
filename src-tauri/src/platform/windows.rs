//! Windows 平台实现：光标、工作区、复制注入、GDI 虚拟屏抓取。
//! 自原 main.rs / capture.rs 原样搬迁，逻辑零改动。

use enigo::{Direction, Enigo, Key, Keyboard};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Accessibility::IUIAutomation;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// 启动时拆掉 stdout/stderr 上的「死管道炸弹」，必须在任何 println! 之前调用。
///
/// 背景（panic.log 实锤）：进程的 stdout 若是继承来的管道（从终端启动后终端关了、
/// updater restart 继承旧进程的管道），读者一死，之后**任何一次 println! 都会
/// panic**（os error 232「管道正在被关闭」）。panic 落在异步命令任务里，invoke 的
/// promise 永不 settle（前端永远「翻译中」）；落在事件循环/快捷键回调里则是
/// `panic in a function that cannot unwind`，整个进程直接 abort。
///
/// 处理：无效句柄（GUI 方式启动时 stdout 本就是 NULL）和管道句柄一律换成 NUL
/// 设备——写入永远成功，日志丢弃。release 的 GUI 窗口本就看不到 stdout，不亏；
/// 从终端启动想看日志的场景用 dev 构建。console（真控制台）和磁盘文件
/// （重定向到日志文件）保持原样。dev 构建不调用本函数：cargo tauri dev
/// 的日志全靠 npm 那根活管道。
///
/// std 的 Stdout 在首次写入时才取句柄并缓存，所以只要赶在第一次 println! 前
/// 换掉即可，此后所有 println! 写 NUL 恒成功、永不 panic。
pub fn sanitize_stdio() {
    use windows::Win32::Foundation::GENERIC_WRITE;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileType, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_TYPE_PIPE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    // NUL 设备：打开一次，两个标准流共用（打不开就放弃，维持现状总比没有强）
    let nul = unsafe {
        CreateFileW(
            windows::core::w!("NUL"),
            GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    let Ok(nul) = nul else { return };
    for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        unsafe {
            let Ok(h) = GetStdHandle(which) else { continue };
            if h.is_invalid() || GetFileType(h) == FILE_TYPE_PIPE {
                let _ = SetStdHandle(which, nul);
            }
        }
    }
}

/// 注入「复制」按键。快捷键触发时物理按着的修饰键要先松开：
/// 直接模拟 Ctrl+C 会被应用当成 Ctrl+Alt+C 而拒绝复制。
/// `app` 参数只为与 macOS 实现对齐（那边必须派发回主线程）。返回是否成功按下。
pub fn inject_copy(_app: &tauri::AppHandle) -> bool {
    let Ok(mut enigo) = Enigo::new(&enigo::Settings::default()) else {
        return false;
    };
    // C 用 Unicode('c') 而不是 Key::C：Key::C 走 VK 路径（wScan=0），
    // 部分 App（Chromium、IME 激活时）会忽略无扫描码的注入键导致复制失效；
    // Unicode 路径带 KEYEVENTF_SCANCODE，与真实键盘事件一致。
    let ok = enigo.key(Key::Alt, Direction::Release).is_ok()
        && enigo.key(Key::Control, Direction::Press).is_ok()
        && enigo.key(Key::Unicode('c'), Direction::Click).is_ok()
        && enigo.key(Key::Control, Direction::Release).is_ok();
    std::thread::sleep(std::time::Duration::from_millis(20));
    ok
}

/// 前台窗口所属进程名（诊断用，取不到返回空串）
pub fn foreground_app() -> String {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return String::new();
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return String::new();
        }
        // 取进程名：快照法太重，用 OpenProcess+QueryFullProcessImageNameW
        let handle = windows::Win32::System::Threading::OpenProcess(
            windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        );
        if let Ok(h) = handle {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            let ok = windows::Win32::System::Threading::QueryFullProcessImageNameW(h, windows::Win32::System::Threading::PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = windows::Win32::Foundation::CloseHandle(h);
            if ok.is_ok() {
                let path = String::from_utf16_lossy(&buf[..len as usize]);
                return path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
            }
        }
        format!("pid={pid}")
    }
}

pub fn cursor_pos() -> (i32, i32) {
    unsafe {
        let mut p = POINT { x: 0, y: 0 };
        let _ = GetCursorPos(&mut p);
        (p.x, p.y)
    }
}

/// 关掉 Windows 11 给顶层窗口自动加的系统圆角（~8px）：透明弹窗的圆角由 CSS 画，
/// 系统再叠一层弧就成了「两层圆角」。Win10 不认这个属性，返回错误静默忽略
pub fn disable_rounding(win: &tauri::WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_DONOTROUND,
    };
    let Ok(hwnd) = win.hwnd() else {
        return;
    };
    let pref = DWM_WINDOW_CORNER_PREFERENCE(DWMWCP_DONOTROUND.0);
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(hwnd.0 as *mut core::ffi::c_void),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}

/// 光标所在显示器的工作区（物理像素，排除任务栏等应用栏）。
/// 拿不到信息时返回 (0,0,0,0)，前端会退回用虚拟屏边界錨制。
pub fn work_area_at(x: i32, y: i32) -> (i32, i32, i32, i32) {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let hmon = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    if unsafe { GetMonitorInfoW(hmon, &mut info) }.as_bool() {
        let rc = info.rcWork;
        return (rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top);
    }
    (0, 0, 0, 0)
}

/// 虚拟屏边界（SM_X/Y/CX/CYVIRTUALSCREEN，物理像素）；宽或高非正时返回 None。
/// 抓屏与 virtual_screen_bounds 共用这一份读数
fn virtual_screen_metrics() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        let (ox, oy) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        let (w, h) = (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        );
        (w > 0 && h > 0).then_some((ox, oy, w, h))
    }
}

/// 抓取整个虚拟屏幕（所有显示器覆盖的范围）
pub fn capture_virtual_screen() -> Result<crate::capture::CapturedScreen, String> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, SRCCOPY,
    };

    let (ox, oy, w, h) = virtual_screen_metrics().ok_or("获取虚拟屏幕尺寸失败")?;
    unsafe {
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(screen_dc);
        let bitmap = CreateCompatibleBitmap(screen_dc, w, h);
        let old = SelectObject(mem_dc, bitmap);

        let blit = BitBlt(mem_dc, 0, 0, w, h, screen_dc, ox, oy, SRCCOPY);

        // MSDN：GetDIBits 调用时位图不能被选入任何 DC —— 必须先取下位图再读像素
        SelectObject(mem_dc, old);

        // 负高度 = 自上而下行序；32bpp BI_RGB 输出为 BGRA。
        // biSize 必须显式设置：windows crate 的 Default() 是全零，缺了它 GetDIBits 直接失败
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB
            ..Default::default()
        };
        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
        let lines = GetDIBits(
            screen_dc,
            bitmap,
            0,
            h as u32,
            Some(rgba.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        let _ = DeleteObject(bitmap);
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        if let Err(e) = blit {
            return Err(format!("BitBlt 抓屏失败: {e}"));
        }
        if lines == 0 {
            return Err("GetDIBits 读取像素失败".into());
        }
        crate::capture::bgra_to_rgba(&mut rgba);
        Ok(crate::capture::CapturedScreen {
            width: w,
            height: h,
            origin_x: ox,
            origin_y: oy,
            rgba,
        })
    }
}

/// 虚拟屏边界（物理像素）
pub fn virtual_screen_bounds() -> Option<(i32, i32, i32, i32)> {
    virtual_screen_metrics()
}

// ---------- 划词图标：全局鼠标监听 + 系统选区 ----------

// 回调类型与回调槽（MOUSE_ON_*）在 super（platform/mod.rs），两个平台共用
use super::{MOUSE_ON_DOWN, MOUSE_ON_MOVE, MOUSE_ON_UP, MouseCallback, MoveCallback};

/// 低级鼠标钩子：回调必须立刻返回（它在系统的输入线程上跑，慢了整个系统的鼠标都卡）
unsafe extern "system" fn mouse_hook(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, MSLLHOOKSTRUCT,
    };
    if code >= 0 {
        match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                if let Some(cb) = MOUSE_ON_DOWN.get() {
                    cb()
                }
            }
            WM_LBUTTONUP => {
                if let Some(cb) = MOUSE_ON_UP.get() {
                    cb()
                }
            }
            WM_MOUSEMOVE => {
                if let Some(cb) = MOUSE_ON_MOVE.get() {
                    let pt = (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt;
                    cb(pt.x as i32, pt.y as i32);
                }
            }
            _ => {}
        }
    }
    windows::Win32::UI::WindowsAndMessaging::CallNextHookEx(None, code, wparam, lparam)
}

/// 装全局鼠标监听：左键按下/松开各回调一次。低级钩子必须装在自带消息循环的线程上，
/// 所以这里起一个常驻线程把它跑起来。
pub fn start_mouse_watch(
    on_down: MouseCallback,
    on_up: MouseCallback,
    on_move: MoveCallback,
) -> bool {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, SetWindowsHookExW, MSG, WH_MOUSE_LL,
    };

    let _ = MOUSE_ON_DOWN.set(on_down);
    let _ = MOUSE_ON_UP.set(on_up);
    let _ = MOUSE_ON_MOVE.set(on_move);
    // 钩子安装结果同步传回：装不上时调用方要明确知道（划词图标不可用），
    // 而不是永远返回 true 静默失败
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        let hmod = GetModuleHandleW(None).unwrap_or_default();
        let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hmod, 0) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[mouse] 装低级鼠标钩子失败: {e}");
                let _ = tx.send(false);
                return;
            }
        };
        let _ = tx.send(true);
        println!("[mouse] 低级鼠标钩子已装");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
        let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
    });
    rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or(false)
}

/// 读焦点控件里被选中的文本（UI Automation 文本模式，不碰剪贴板也不注入按键）。
/// 终端、部分 Electron 应用不实现文本模式，返回 None——那时不弹图标，用户照旧用热键。
pub fn focused_selection() -> Option<String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::CUIAutomation;

    unsafe {
        // UIA 客户端用 STA；反复初始化只是加引用计数，无需配对 Uninitialize（线程随进程退出）
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        // Chromium / Electron 的 UIA 树是懒建的，第一次常拿到空节点；负载高时
        // 120ms 也不够，逐次加长重试到 360ms（选词后图标本就不弹在鼠标按下瞬间，
        // 这里慢几十毫秒无感）
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(120 * attempt as u64));
            }
            if let Some(text) = selection_of(&automation) {
                return Some(text);
            }
        }
        None
    }
}

unsafe fn selection_of(automation: &IUIAutomation) -> Option<String> {
    use windows::Win32::UI::Accessibility::{IUIAutomationTextPattern, UIA_TextPatternId};

    let element = automation.GetFocusedElement().ok()?;
    let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
    let ranges = pattern.GetSelection().ok()?;
    let count = ranges.Length().ok()?;
    let mut text = String::new();
    for i in 0..count {
        let Ok(range) = ranges.GetElement(i) else {
            continue;
        };
        // -1 = 取全部长度
        if let Ok(bstr) = range.GetText(-1) {
            text.push_str(&bstr.to_string());
        }
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
