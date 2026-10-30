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
