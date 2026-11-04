//! Windows 剪贴板原语：CF_UNICODETEXT 读/写（带锁重试）、变更序号、归属判断。
//! 只服务热键路径的 grab_selection（划词图标路径绝不碰剪贴板）。
//!
//! 旧实现经 arboard 走「备份→写哨兵→轮询→恢复」，有两个毁用户内容的洞：
//! 1. 目标应用瞬时锁着剪贴板时备份读写双双失败 → 旧内容无从恢复；
//! 2. 哨兵本身就是一次破坏性写，轮询窗口 ~900ms 内用户 Ctrl+V 粘出零宽垃圾。
//! 现设计**零多余写入**：grab 期间只有目标应用响应复制键的一次写入 + 我们的一次恢复；
//! 「复制是否生效」用 GetClipboardSequenceNumber（无需打开剪贴板，一次查询）判定，
//! 序号一动即有人写入，内容相同也不会漏判（哨兵法靠比内容，选中与旧内容相同时会误判）。

use windows::Win32::Foundation::HWND;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner, GetClipboardSequenceNumber,
    OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// 恢复备份时的 OpenClipboard 重试次数与间隔：抢瞬时锁（目标应用正在写/读剪贴板）
const RESTORE_RETRIES: u32 = 8;
const RETRY_MS: u64 = 15;

/// 剪贴板变更序号：任何进程写剪贴板都会 +1。只读查询、不碰剪贴板锁，轮询零成本。
pub fn clipboard_seq() -> u64 {
    unsafe { GetClipboardSequenceNumber() as u64 }
}

/// 读剪贴板当前文本。
/// `Ok(Some(t))` 有文本；`Ok(None)` **确定**没有文本格式（剪贴板是图片/文件/空）；
/// `Err(())` 重试 `retries` 次后仍打不开（别的进程持锁，内容未知）——调用方据此
/// 区分「确认无文本（可以放心恢复/清空）」和「不知道里面是什么（绝不能乱动）」。
pub fn clipboard_read_text(retries: u32) -> Result<Option<String>, ()> {
    unsafe {
        for attempt in 0..=retries {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(RETRY_MS));
            }
            if OpenClipboard(HWND::default()).is_err() {
                continue;
            }
            let text = clipboard_text_locked();
            let _ = CloseClipboard();
            // 内层 Some/None 都有意义（有文本/确定没文本）；外层 None=句柄读失败，按锁竞争继续重试
            if let Some(t) = text {
                return Ok(t);
            }
        }
        Err(())
    }
}

/// 恢复用户原内容：Some(t) 写回文本；None 清空（原本就没有文本）。全程一次开关剪贴板。
pub fn clipboard_restore(backup: Option<String>) -> bool {
    unsafe {
        for attempt in 0..=RESTORE_RETRIES {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(RETRY_MS));
            }
            if OpenClipboard(HWND::default()).is_err() {
                continue;
            }
            let ok = EmptyClipboard().is_ok()
                && match &backup {
                    // 原本无文本：EmptyClipboard 成功即还原到空态；空串文本同理（Empty 已清）
                    None => true,
                    Some(t) => t.is_empty() || set_cf_unicode_text_locked(t),
                };
            let _ = CloseClipboard();
            if ok {
                return true;
            }
        }
        println!("[clip] 恢复剪贴板失败（重试 {} 次后被持续锁住）", RESTORE_RETRIES);
        false
    }
}

/// 当前剪贴板是不是我们进程写的（watcher 排除自己恢复动作的回声）
pub fn clipboard_owner_is_self() -> bool {
    unsafe {
        let Ok(owner) = GetClipboardOwner() else {
            return false;
        };
        if owner.0.is_null() {
            return false;
        }
        let mut pid: u32 = 0;
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(owner, Some(&mut pid));
        pid == std::process::id()
    }
}

// ---------- 内部 ----------

/// 须在 OpenClipboard 之后调用。返回外层 None = 暂读不了（按锁竞争重试）；
/// 内层 Some=文本 / None=确定没有文本格式。
/// 0.58 绑定的 is_invalid() 把 NULL 也判无效 → 「格式不存在」走 Err 分支，光看 Ok/Err
/// 分不清有无。MSDN 正解：调用前把 last-error 清零，返回 NULL 后错误码 0 = 格式确实
/// 不存在，非 0 = 瞬时失败（延迟渲染物化失败、内存不足等）归重试。
unsafe fn clipboard_text_locked() -> Option<Option<String>> {
    windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
    let h = match GetClipboardData(CF_UNICODETEXT.0 as u32) {
        Ok(h) => h,
        Err(_) => {
            return if windows::Win32::Foundation::GetLastError().0 == 0 {
                Some(None)
            } else {
                None
            };
        }
    };
    let hglobal = windows::Win32::Foundation::HGLOBAL(h.0);
    let size = GlobalSize(hglobal);
    let ptr = GlobalLock(hglobal) as *const u16;
    if ptr.is_null() {
        return None; // 锁句柄失败按瞬时故障，外层走重试
    }
    let units = std::slice::from_raw_parts(ptr, size / 2);
    // UTF-16 以 NUL 结尾；个别应用给的双终止/无终止都按截断读
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    let text = String::from_utf16_lossy(&units[..end]);
    let _ = GlobalUnlock(hglobal);
    Some(Some(text))
}

/// 须在 OpenClipboard + EmptyClipboard 之后调用。SetClipboardData 成功后这块 HGLOBAL
/// 归系统所有（不能再 GlobalFree）；只有失败路径才回收。
unsafe fn set_cf_unicode_text_locked(text: &str) -> bool {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    let bytes = units.len() * 2;
    let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
        return false;
    };
    let ptr = GlobalLock(hmem);
    if ptr.is_null() {
        let _ = windows::Win32::Foundation::GlobalFree(hmem);
        return false;
    }
    std::ptr::copy_nonoverlapping(units.as_ptr() as *const u8, ptr as *mut u8, bytes);
    let _ = GlobalUnlock(hmem);
    if SetClipboardData(CF_UNICODETEXT.0 as u32, windows::Win32::Foundation::HANDLE(hmem.0)).is_err() {
        let _ = windows::Win32::Foundation::GlobalFree(hmem);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真操作系统剪贴板的往返自检（动的是用户剪贴板，默认 ignore；手动 `cargo test -- --ignored` 跑）
    #[test]
    #[ignore]
    fn write_read_restore_roundtrip() {
        // 先备份用户现有内容，测完原样放回
        let backup = clipboard_read_text(8).expect("打不开剪贴板");
        let marker = "__wordpeek_clipboard_roundtrip__";
        let s0 = clipboard_seq();
        assert!(clipboard_restore(Some(marker.to_string())), "写入标记失败");
        assert_ne!(clipboard_seq(), s0, "写入后序号必须变化");
        assert_eq!(clipboard_read_text(0), Ok(Some(marker.to_string())), "标记回读不一致");
        // 恢复到备份（Some 文本或空态）后再验证读回与备份一致
        assert!(clipboard_restore(backup.clone()), "恢复失败");
        assert_eq!(clipboard_read_text(0), Ok(backup));
    }
}
