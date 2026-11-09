//! Windows 平台实现：光标、工作区、复制注入、GDI 虚拟屏抓取（自原 main.rs / capture.rs 搬迁）

use enigo::{Direction, Enigo, Key, Keyboard};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Accessibility::IUIAutomation;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// 启动时拆掉 stdout/stderr 上的「死管道炸弹」，必须在任何 println! 之前调用。
///
/// stdout 若是继承来的死管道（终端已关、updater restart），之后任何 println! 都会
/// panic（os error 232）：落在异步命令任务里 invoke 永不 settle（前端永远「翻译中」），
/// 落在事件循环/快捷键回调里则 `panic in a function that cannot unwind` 直接 abort。
/// 处理：无效句柄（GUI 启动时 stdout 本就是 NULL）和管道句柄一律换成 NUL 设备——写入恒成功、
/// 日志丢弃；console（真控制台）与磁盘文件保持原样；dev 构建不调用（cargo tauri dev 的日志靠 npm 那根活管道）。
/// std 的 Stdout 首次写入才取句柄并缓存，赶在第一次 println! 前换掉即可。
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

/// 注入「复制」按键：先松开物理按着的修饰键，否则 Ctrl+C 会被应用当成 Ctrl+Alt+C 拒绝复制。
/// `app` 只为与 macOS 实现对齐（那边必须派发回主线程）。返回是否成功按下。
pub fn inject_copy(_app: &tauri::AppHandle) -> bool {
    let Ok(mut enigo) = Enigo::new(&enigo::Settings::default()) else {
        return false;
    };
    // C 用 Unicode('c') 而非 Key::C：Key::C 走 VK 路径（wScan=0），部分 App（Chromium、
    // IME 激活时）忽略无扫描码的注入键；Unicode 路径带 KEYEVENTF_SCANCODE，与真实键盘一致。
    let ok = enigo.key(Key::Alt, Direction::Release).is_ok()
        && enigo.key(Key::Control, Direction::Press).is_ok()
        && enigo.key(Key::Unicode('c'), Direction::Click).is_ok()
        && enigo.key(Key::Control, Direction::Release).is_ok();
    std::thread::sleep(std::time::Duration::from_millis(20));
    ok
}

/// 前台应用上下文一次性查询（热键兜底注入前的安全闸）。取不到信息一律按「允许」处理，
/// 绝不因查询失败拦下正常路径；elevated/password/secure 三项只做诊断与放行决策，不重试
pub fn foreground_context() -> super::ForegroundContext {
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId};

    let hwnd = unsafe { GetForegroundWindow() };
    let (pid, process_name, console_class, elevated) = unsafe {
        let mut pid: u32 = 0;
        if hwnd.0.is_null() {
            (None, String::new(), false, false)
        } else {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let pid = (pid != 0).then_some(pid as i32);
            let name = pid
                .and_then(|p| process_image_name(p as u32))
                .unwrap_or_else(|| format!("pid={}", pid.unwrap_or(0)));
            // 老式控制台（窗口类 ConsoleWindowClass）里 Ctrl+C 是「中断」不是复制，注入会打断用户命令；
            // Windows Terminal 有选区时才是复制，只排除老控制台
            let mut buf = [0u16; 64];
            let class = GetClassNameW(hwnd, &mut buf);
            let console = class > 0
                && String::from_utf16_lossy(&buf[..class as usize]) == "ConsoleWindowClass";
            let elev = pid.map(|p| process_elevated(p as u32)).unwrap_or(false);
            (pid, name, console, elev)
        }
    };
    super::ForegroundContext {
        pid,
        process_name,
        fallback_allowed: !console_class,
        elevated,
        password_focus: password_focused(),
        secure_input: false,
    }
}

/// 进程映像文件名（诊断用）：OpenProcess+QueryFullProcessImageNameW，比快照法轻
pub(crate) unsafe fn process_image_name(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(
        handle,
        PROCESS_NAME_WIN32,
        windows::core::PWSTR(buf.as_mut_ptr()),
        &mut len,
    );
    let _ = CloseHandle(handle);
    ok.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    Some(path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string())
}

/// Chromium 系浏览器：无障碍树懒建且会在客户端停止查询后自动拆除（Chrome 空闲
/// 几分钟就拆树）。树冷启动后的前几轮探测全是 NoTextPattern，60 秒内三振即被
/// hopeless 拉黑——而拉黑 = 停止敲门 = 树永远重建不起来（死循环）。这类进程不记账
pub(crate) fn is_chromium_browser(pid: i32) -> bool {
    const CHROMIUM_BROWSERS: [&str; 5] = ["chrome.exe", "msedge.exe", "chromium.exe", "brave.exe", "opera.exe"];
    let Some(name) = (unsafe { process_image_name(pid as u32) }) else {
        return false;
    };
    CHROMIUM_BROWSERS.iter().any(|b| name.eq_ignore_ascii_case(b))
}

/// 目标进程是否以管理员运行（UIPI 会拒我们这个普通完整性进程的模拟按键与 UIA 读）
fn process_elevated(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut token = HANDLE::default();
        // 0.58 的 OpenProcessToken 返回 Result<()>，句柄写在 &mut token 里
        let elevated = match OpenProcessToken(handle, TOKEN_QUERY, &mut token) {
            Ok(()) => {
                let mut elev = TOKEN_ELEVATION::default();
                let mut ret_len = 0u32;
                GetTokenInformation(
                    token,
                    TokenElevation,
                    Some(&mut elev as *mut TOKEN_ELEVATION as *mut std::ffi::c_void),
                    std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                    &mut ret_len,
                )
                .map(|_| elev.TokenIsElevated != 0)
                .unwrap_or(false)
            }
            Err(_) => false,
        };
        let _ = CloseHandle(token); // token 句柄也要关
        let _ = CloseHandle(handle);
        elevated
    }
}

/// 焦点控件是不是密码框（UIA IsPassword 属性）。查不到（COM 起不来/无焦点元素）按否处理：
/// 这只是安全闸的加固项，不能反过来因基础设施故障拦下正常翻译
fn password_focused() -> bool {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    unsafe {
        // 与 read_focused_selection 同理用 MTA（见那里的注释）；若本线程已被先到的
        // MTA 初始化钉住，这里返回 RPC_E_CHANGED_MODE 被忽略，直接沿用 MTA 即可
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let automation: IUIAutomation =
            match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                Ok(a) => a,
                Err(_) => return false,
            };
        automation
            .GetFocusedElement()
            .and_then(|el| el.CurrentIsPassword())
            .map(|b| b.as_bool())
            .unwrap_or(false)
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

/// 光标所在显示器的工作区（物理像素，排除任务栏等应用栏）；拿不到时返回 (0,0,0,0)，
/// 前端会退回用虚拟屏边界锚制。
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

/// 虚拟屏边界（SM_X/Y/CX/CYVIRTUALSCREEN，物理像素）；宽或高非正时返回 None。抓屏与 virtual_screen_bounds 共用
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
use super::{MOUSE_ON_DOWN, MOUSE_ON_MOVE, MOUSE_ON_UP, MouseCallback, MoveCallback, SelectionRead};

// 钩子活体检测：Windows 会在睡眠唤醒/长时间挂机后静默摘掉 WH_MOUSE_LL（无任何通知），
// 症状就是划词图标从此不弹。看门狗拿系统级 GetLastInputInfo（不经过我们的钩子）对比
// 钩子最后收到事件的时间，确认钩子死了就重装。
static LAST_HOOK_EVENT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static WATCHDOG_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 活钩子的所有权凭证：tid 用于投递 WM_QUIT，done 通道在线程 unhook 完成后才有响应。
/// 看门狗重装前必须先 take 旧凭证并等线程真正退出——否则旧线程与新钩子的状态互相踩踏，
/// 会留下永远 unhook 不掉的孤儿钩子，鼠标事件要过越积越长的钩子链（全系统卡）。
struct HookGuard {
    tid: u32,
    done_rx: std::sync::mpsc::Receiver<()>,
}

static LIVE_HOOK: std::sync::Mutex<Option<HookGuard>> = std::sync::Mutex::new(None);

fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// [DEBUG-m0u5] 临时诊断插桩：追加写 %TEMP%\wp-sel.log（GUI 子系统 println 不可见）。诊断完删除
pub(crate) fn debug_log(line: &str) {
    use std::io::Write;
    let Ok(tmp) = std::env::var("TEMP") else {
        return;
    };
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!(r"{tmp}\wp-sel.log"))
    else {
        return;
    };
    let _ = writeln!(f, "{} {line}", epoch_ms());
}

/// 注册 AUMID（HKCU，无需管理员）：NSIS 装的快捷键不带 AppUserModelID，WinRT toast
/// 按 HKCU\Software\Classes\AppUserModelId\<id> 找身份，找不到就静默失败（GUI 进程
/// 看不到 eprintln）。写入 DisplayName/IconUri 后通知显示「Wordpeek」名字与图标
pub fn register_aumid(app_id: &str) {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    use windows::core::PCWSTR;

    let subkey: Vec<u16> = format!(r"Software\Classes\AppUserModelId\{app_id}")
        .encode_utf16()
        .chain([0])
        .collect();
    let set_str = |hkey: HKEY, name: &str, value: String| {
        let name_w: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
        unsafe {
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(name_w.as_ptr()),
                0,
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    data.as_ptr() as *const u8,
                    data.len() * 2,
                )),
            );
        }
    };
    unsafe {
        let mut hkey = HKEY::default();
        if RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
        .is_err()
        {
            println!("[notify] AUMID 注册失败（toast 将无法以应用身份显示）");
            return;
        }
        set_str(hkey, "DisplayName", "Wordpeek".to_string());
        if let Ok(exe) = std::env::current_exe() {
            set_str(hkey, "IconUri", exe.to_string_lossy().into_owned());
        }
        let _ = RegCloseKey(hkey);
    }
}
fn store_live_hook(guard: Option<HookGuard>) {
    if let Ok(mut slot) = LIVE_HOOK.lock() {
        *slot = guard;
    }
}

/// 装低级鼠标钩子（独立线程跑消息循环）。成功返回所有权凭证，失败返回 None。
fn install_hook() -> Option<HookGuard> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || unsafe {
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx, MSG, WH_MOUSE_LL,
        };
        let tid = GetCurrentThreadId();
        let hmod = GetModuleHandleW(None).unwrap_or_default();
        let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), hmod, 0) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[mouse] 装低级鼠标钩子失败: {e}");
                let _ = ready_tx.send(None);
                return;
            }
        };
        let _ = ready_tx.send(Some(tid));
        println!("[mouse] 低级鼠标钩子已装");
        LAST_HOOK_EVENT_MS.store(epoch_ms(), std::sync::atomic::Ordering::Relaxed);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
        let _ = UnhookWindowsHookEx(hook);
        // done_tx 在 unhook 完成后才 drop：等 done 的一侧据此确认旧钩子已彻底卸下
        let _ = done_tx.send(());
    });
    match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(Some(tid)) => Some(HookGuard { tid, done_rx }),
        _ => None,
    }
}

/// 重装钩子：先让旧线程真正退出（unhook 完成）再装新的，绝不新旧并存。
fn replace_hook() -> bool {
    let old = LIVE_HOOK.lock().ok().and_then(|mut slot| slot.take());
    if let Some(g) = old {
        unsafe {
            use windows::Win32::Foundation::{LPARAM, WPARAM};
            use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
            let _ = PostThreadMessageW(
                g.tid,
                WM_QUIT,
                WPARAM(0),
                LPARAM(0),
            );
        }
        // 超时不硬上：宁可这轮没有钩子（下个周期再来），也不留一个没人管得住的孤儿钩子
        match g
            .done_rx
            .recv_timeout(std::time::Duration::from_secs(10))
        {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                eprintln!("[mouse] 旧钩子线程迟迟不退，暂缓重装");
                store_live_hook(Some(g));
                return false;
            }
        }
    }
    match install_hook() {
        Some(g) => {
            store_live_hook(Some(g));
            true
        }
        None => false,
    }
}

/// 钩子看门狗：用户最近 10s 内有过任何系统输入、但钩子连续两轮（≥10s）一个鼠标事件都没
/// 送来 = 钩子已被 Windows 静默摘除，重装。纯键盘时段会误判，但重装经过 done 通道确认、
/// 不再产生孤儿钩子，开销几毫秒、用户无感。
fn start_hook_watchdog() {
    std::thread::spawn(move || {
        let mut dead_streak = 0u32;
        let mut tick = 0u32; // [DEBUG-m0u5]
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            tick += 1; // [DEBUG-m0u5]
            // 钩子压根不在（上次重装失败）：直接补装，不必等用户有输入
            let hook_missing = LIVE_HOOK
                .lock()
                .map(|slot| slot.is_none())
                .unwrap_or(false);
            if hook_missing {
                if tick % 6 == 0 {
                    // [DEBUG-m0u5]
                    debug_log("watchdog hookMissing=true");
                }
                dead_streak = 0;
                replace_hook();
                continue;
            }
            unsafe {
                use windows::Win32::System::SystemInformation::GetTickCount;
                use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
                let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
                if !GetLastInputInfo(&mut li).as_bool() {
                    continue;
                }
                // dwTime 是 GetTickCount 时基（ms since boot，32 位会回绕），只做小差值比较
                let active_ms = GetTickCount().wrapping_sub(li.dwTime);
                if tick % 6 == 0 {
                    // [DEBUG-m0u5] 每 30s 一次：钩子年龄/输入年龄/worker 心跳年龄，拿划词失效现场
                    let hook_age = epoch_ms().saturating_sub(
                        LAST_HOOK_EVENT_MS.load(std::sync::atomic::Ordering::Relaxed),
                    );
                    let worker_age = epoch_ms().saturating_sub(
                        crate::selection::probe::LAST_WORKER_UP_MS
                            .load(std::sync::atomic::Ordering::Relaxed),
                    );
                    debug_log(&format!(
                        "watchdog hookAge={hook_age} inputAge={active_ms} workerUpAge={worker_age}"
                    ));
                }
                if active_ms > 10_000 {
                    dead_streak = 0; // 用户本就没在动，钩子没事件是正常的
                    continue;
                }
                let stale_ms = epoch_ms().saturating_sub(LAST_HOOK_EVENT_MS.load(std::sync::atomic::Ordering::Relaxed));
                if stale_ms <= 30_000 {
                    dead_streak = 0; // 钩子还活着
                    continue;
                }
                dead_streak += 1;
                if dead_streak < 2 {
                    continue; // 连续两轮都失活才动手，防单次误判
                }
                dead_streak = 0;
                println!("[mouse] 检测到低级鼠标钩子失效（静默摘除），重装");
                debug_log("watchdog reinstalling hook"); // [DEBUG-m0u5]
                replace_hook();
            }
        }
    });
}

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
            WM_LBUTTONDOWN | WM_LBUTTONUP => {
                LAST_HOOK_EVENT_MS.store(epoch_ms(), std::sync::atomic::Ordering::Relaxed);
                if wparam.0 as u32 == WM_LBUTTONDOWN {
                    if let Some(cb) = MOUSE_ON_DOWN.get() {
                        cb()
                    }
                } else if let Some(cb) = MOUSE_ON_UP.get() {
                    cb()
                }
            }
            WM_MOUSEMOVE => {
                LAST_HOOK_EVENT_MS.store(epoch_ms(), std::sync::atomic::Ordering::Relaxed);
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

/// 装全局鼠标监听：左键按下/松开各回调一次。低级钩子必须装在自带消息循环的线程上，起常驻线程跑它。
/// 另起看门狗线程：Windows 会在睡眠唤醒后静默摘掉 WH_MOUSE_LL，死了就自动重装（重装经
/// HookGuard 所有权交接，回调槽是 OnceLock，复用同一批闭包和同一个信号通道，worker 无感知）。
pub fn start_mouse_watch(
    on_down: MouseCallback,
    on_up: MouseCallback,
    on_move: MoveCallback,
) -> bool {
    let _ = MOUSE_ON_DOWN.set(on_down);
    let _ = MOUSE_ON_UP.set(on_up);
    let _ = MOUSE_ON_MOVE.set(on_move);
    // 钩子安装结果同步传回：装不上时调用方要明确知道（划词图标不可用），不能静默失败。
    // 首装偶尔被拖慢（杀软扫新 exe、系统繁忙）超过 2s 应答超时，重试两次再定生死，
    // 不能一次 recv 超时就把划词图标判死刑
    let mut watch = false;
    for _ in 0..3 {
        if let Some(g) = install_hook() {
            store_live_hook(Some(g));
            watch = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    if !watch {
        return false;
    }
    if !WATCHDOG_STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        start_hook_watchdog();
    }
    true
}

/// 读焦点控件里被选中的文本（UI Automation 文本模式，不碰剪贴板也不注入按键）。
/// 返回门面统一四态（hopeless 拉黑记忆由门面统一记账）；终端、部分 Electron 应用
/// 不实现文本模式返回 NoTextPattern——那时不弹图标，用户照旧用热键
pub fn read_focused_selection() -> super::SelectionRead {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::CUIAutomation;

    let mut text = None;
    // 本轮拿到的最后一个非 Text 状态；COM 起不来时保持 Error（绝不据此拉黑前台应用）
    let mut overall = super::SelectionRead::Error;
    unsafe {
        // UIA 客户端必须走 MTA：探测线程没有消息泵，STA 下跨进程取属性遇到需要
        // 重入的 provider 回调（Chromium 取属性时会回调客户端）会直接失败——
        // 表现就是永远拿不到 TextPattern，Chromium 的树永远敲不醒（ hopeless 黑名单
        // 死循环）。微软官方要求无消息循环的 UIA 客户端用 MTA。
        // 反复初始化只是加引用计数，无需配对 Uninitialize（线程随进程退出）
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        if let Ok(automation) = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
            // Chromium / Electron 的 UIA 树是懒建的，第一次常拿到空节点；负载高时
            // 120ms 也不够，逐次加长重试到 360ms（图标本就不弹在按下瞬间，慢几十毫秒无感）
            for attempt in 0..3 {
                if attempt > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(120 * attempt as u64));
                }
                match selection_of(&automation) {
                    SelectionRead::Text(t) => {
                        text = Some(t);
                        break;
                    }
                    SelectionRead::NoTextPattern => overall = SelectionRead::NoTextPattern,
                    // 后面的尝试哪怕只是空选区，也说明应用其实支持（不算「不支持」）；
                    // 空选区与瞬时错误在门面都不拉黑，这里合并为 NoSelection 即可
                    SelectionRead::NoSelection | SelectionRead::Error => {
                        overall = SelectionRead::NoSelection;
                    }
                }
            }
        }
    }
    match text {
        Some(t) => super::SelectionRead::Text(t),
        None => overall,
    }
}

/// 预热 UIA 客户端：进程内首次 COM/UIA 初始化慢（CoInitializeEx(MTA) 启动 + 加载
/// UIAutomationCore），不预热的话第一次划词探测被拖慢到秒级，结果因代次过期被丢弃，
/// 图标不弹。启动时后台线程做一次，之后各探测线程加入已有 MTA 即可（加引用计数，
/// 无需配对 Uninitialize）。只裸初始化，不调用 selection_of——不向焦点应用发查询，
/// 也不碰 hopeless 记账
pub fn warm_uia() {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        // 拿到即预热完成：UIA 核心已加载进进程，实例本身随作用域释放
        let _: Option<IUIAutomation> =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
    }
}

pub fn foreground_pid() -> Option<i32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some(pid as i32)
    }
}

/// 单次 UIA 选区读取，用门面的统一四态（语义与拉黑规则见 platform::SelectionRead）
unsafe fn selection_of(automation: &IUIAutomation) -> super::SelectionRead {
    let element = match automation.GetFocusedElement() {
        Ok(e) => e,
        Err(_) => return SelectionRead::Error,
    };
    if let Some(r) = unsafe { selection_via_text_pattern(&element) } {
        return r;
    }
    // 焦点元素没有 TextPattern：聊天/编辑类网页（DeepSeek、Slack…）键盘焦点常驻输入框，
    // 鼠标划选不转移焦点 → Ctrl+C 也只作用于空输入框。改试光标下的元素——选区就在那，
    // 它的 TextPattern.GetSelection 返回整个文档的选区。仍拿不到才算「不支持」
    let mut pt = POINT { x: 0, y: 0 };
    if GetCursorPos(&mut pt).is_ok() {
        if let Ok(el) = automation.ElementFromPoint(pt) {
            if let Some(r) = unsafe { selection_via_text_pattern(&el) } {
                return r;
            }
        }
    }
    SelectionRead::NoTextPattern
}

/// 元素的 TextPattern 选区文本：拿不到 TextPattern 返 None（=此元素不支持，由调用方决定
/// 是否换元素再试）；空选区返 NoSelection；其余读失败返 Error（不拉黑）
unsafe fn selection_via_text_pattern(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<super::SelectionRead> {
    use windows::Win32::UI::Accessibility::UIA_TextPatternId;
    let pattern: windows::Win32::UI::Accessibility::IUIAutomationTextPattern =
        element.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
    let ranges = match pattern.GetSelection() {
        Ok(r) => r,
        Err(_) => return Some(SelectionRead::Error),
    };
    let count = match ranges.Length() {
        Ok(c) => c,
        Err(_) => return Some(SelectionRead::Error),
    };
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
    Some(if trimmed.is_empty() {
        SelectionRead::NoSelection
    } else {
        SelectionRead::Text(trimmed.to_string())
    })
}
