//! Windows 平台实现：光标、工作区、复制注入、GDI 虚拟屏抓取。
//! 自原 main.rs / capture.rs 原样搬迁，逻辑零改动。

use enigo::{Direction, Enigo, Key, Keyboard};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

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

/// 抓取整个虚拟屏幕（所有显示器覆盖的范围）
pub fn capture_virtual_screen() -> Result<crate::capture::CapturedScreen, String> {
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    unsafe {
        let ox = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let oy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if w <= 0 || h <= 0 {
            return Err("获取虚拟屏幕尺寸失败".into());
        }

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

// ---------- 划词图标：全局鼠标监听 + 系统选区 ----------

/// 鼠标事件回调（进程内只装一次，钩子函数从静态里取）
type MouseCallback = Box<dyn Fn() + Send + Sync>;
static MOUSE_ON_DOWN: std::sync::OnceLock<MouseCallback> = std::sync::OnceLock::new();
static MOUSE_ON_UP: std::sync::OnceLock<MouseCallback> = std::sync::OnceLock::new();

/// 低级鼠标钩子：回调必须立刻返回（它在系统的输入线程上跑，慢了整个系统的鼠标都卡）
unsafe extern "system" fn mouse_hook(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
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
            _ => {}
        }
    }
    windows::Win32::UI::WindowsAndMessaging::CallNextHookEx(None, code, wparam, lparam)
}

/// 装全局鼠标监听：左键按下/松开各回调一次。低级钩子必须装在自带消息循环的线程上，
/// 所以这里起一个常驻线程把它跑起来。
pub fn start_mouse_watch(on_down: MouseCallback, on_up: MouseCallback) -> bool {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetMessageW, SetWindowsHookExW, MSG, WH_MOUSE_LL,
    };

    let _ = MOUSE_ON_DOWN.set(on_down);
    let _ = MOUSE_ON_UP.set(on_up);
    std::thread::spawn(move || unsafe {
        let hmod = GetModuleHandleW(None).unwrap_or_default();
        let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hmod, 0) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[mouse] 装低级鼠标钩子失败: {e}");
                return;
            }
        };
        println!("[mouse] 低级鼠标钩子已装");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
        let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
    });
    true
}

/// 读焦点控件里被选中的文本（UI Automation 文本模式，不碰剪贴板也不注入按键）。
/// 终端、部分 Electron 应用不实现文本模式，返回 None——那时不弹图标，用户照旧用热键。
pub fn focused_selection() -> Option<String> {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    unsafe {
        // UIA 客户端用 STA；反复初始化只是加引用计数，无需配对 Uninitialize（线程随进程退出）
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
        // Chromium / Electron 的 UIA 树是懒建的，第一次常拿到空节点，隔一小会儿重试一次
        for attempt in 0..2 {
            if attempt == 1 {
                std::thread::sleep(std::time::Duration::from_millis(120));
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
