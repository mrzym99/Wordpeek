//! Windows 剪贴板原语：多格式原始字节备份/还原（grab 期间的快照）、CF_UNICODETEXT 读/写、
//! 变更序号、归属判断。只服务热键路径的 grab_selection（划词图标路径绝不碰剪贴板）。
//!
//! 旧实现经 arboard 走「备份→写哨兵→轮询→恢复」，有两个毁用户内容的洞：
//! 1. 目标应用瞬时锁着剪贴板时备份读写双双失败 → 旧内容无从恢复；
//! 2. 哨兵本身就是一次破坏性写，轮询窗口 ~900ms 内用户 Ctrl+V 粘出零宽垃圾。
//! 现设计**零多余写入**：grab 期间只有目标应用响应复制键的一次写入 + 我们的一次恢复；
//! 「复制是否生效」用 GetClipboardSequenceNumber（无需打开剪贴板，一次查询）判定，
//! 序号一动即有人写入，内容相同也不会漏判（哨兵法靠比内容，选中与旧内容相同时会误判）。

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, CountClipboardFormats, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardOwner, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

use super::{ClipboardBackup, RawClipFormat};

pub(crate) const CF_UNICODETEXT_ID: u32 = 13;
/// GDI 句柄类格式：GetClipboardData 返回句柄非指针，GlobalLock 不适用，字节备份跳过
const CF_BITMAP_ID: u32 = 2;
const CF_METAFILEPICT_ID: u32 = 3;
const CF_PALETTE_ID: u32 = 9;
const CF_ENHMETAFILE_ID: u32 = 14;

/// 恢复备份时的 OpenClipboard 重试次数与间隔：抢瞬时锁（目标应用正在写/读剪贴板）
const RESTORE_RETRIES: u32 = 8;
const RETRY_MS: u64 = 15;

/// 剪贴板变更序号：任何进程写剪贴板都会 +1。只读查询、不碰剪贴板锁，轮询零成本。
pub fn clipboard_seq() -> u64 {
    unsafe { GetClipboardSequenceNumber() as u64 }
}

/// 读剪贴板当前文本（grab 轮询和 watcher 判定用；备份走 clipboard_backup）。
/// `Ok(Some(t))` 有文本；`Ok(None)` **确定是空剪贴板**（任何格式都没有）；
/// `Err(())` 打不开（别的进程持锁，内容未知）、句柄读失败、或**没有文本格式**——调用方据此
/// 区分「可以放心恢复/清空」和「不知道里面是什么/还原不了（绝不能乱动）」。
pub fn clipboard_read_text(retries: u32) -> Result<Option<String>, ()> {
    unsafe {
        for attempt in 0..=retries {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(RETRY_MS));
            }
            if OpenClipboard(HWND::default()).is_err() {
                if attempt == retries {
                    println!("[clip] 剪贴板持续锁着（OpenClipboard 重试耗尽）");
                }
                continue;
            }
            let text = clipboard_text_locked();
            match text {
                Some(Some(s)) => {
                    let _ = CloseClipboard();
                    return Ok(Some(s));
                }
                Some(None) => {
                    // 确定没文本格式：再分两种——空剪贴板（一种格式都没有）可放心
                    // 按「原本为空」备份/恢复；有非文本内容（图片/文件）无法备份，
                    // 照常恢复会把用户内容清掉，只能整轮放弃。两者都是确定状态，
                    // 不是锁竞争，无需重试（重试到超时只会误报「被占用」）
                    let empty = CountClipboardFormats() == 0;
                    let _ = CloseClipboard();
                    if empty {
                        return Ok(None);
                    }
                    println!("[clip] 剪贴板有非文本内容（图片/文件），无法备份，放弃本轮");
                    return Err(());
                }
                None => {
                    // 外层 None = 句柄读失败（GetClipboardData 报错/GlobalLock 失败），
                    // 按锁竞争继续重试；末次打点——这是唯一静默的 Err 来源，
                    // 「备份失败」没有 [clip] 行就查不到出处
                    if attempt == retries {
                        println!("[clip] 剪贴板开着但读不出内容（句柄读失败，重试耗尽）");
                    }
                    let _ = CloseClipboard();
                }
            }
        }
        Err(())
    }
}

/// 备份剪贴板当前全部内容（多格式原始字节）。Ok = 拿到完整快照，之后可放心还原；
/// Err = 打不开（被持续锁住）或一种格式都拷不出来（纯 GDI 句柄内容，如裸 CF_BITMAP）——
/// 还原不了就绝不动它，调用方整轮放弃。文本外的图片（DIB/DIBV5/PNG）、文件（HDROP）、
/// 自定义注册格式都在可备份范围内；混有 GDI 句柄格式时只备份可拷的（DIB 已携带同等内容）。
pub fn clipboard_backup(retries: u32) -> Result<ClipboardBackup, ()> {
    unsafe {
        for attempt in 0..=retries {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(RETRY_MS));
            }
            if OpenClipboard(HWND::default()).is_err() {
                if attempt == retries {
                    println!("[clip] 剪贴板持续锁着（OpenClipboard 重试耗尽）");
                }
                continue;
            }
            if CountClipboardFormats() == 0 {
                let _ = CloseClipboard();
                return Ok(ClipboardBackup::Empty);
            }
            let mut fmts: Vec<RawClipFormat> = Vec::new();
            let mut fmt = 0u32;
            loop {
                fmt = EnumClipboardFormats(fmt);
                if fmt == 0 {
                    break;
                }
                if let Some(bytes) = raw_format_bytes(fmt) {
                    fmts.push(RawClipFormat { fmt, name: registered_format_name(fmt), bytes });
                }
            }
            let _ = CloseClipboard();
            if !fmts.is_empty() {
                return Ok(ClipboardBackup::Raw(fmts));
            }
            // 有格式但一种都拷不出来：内容未知不可还原，绝不继续（恢复到空态 = 毁掉用户内容）
            println!("[clip] 剪贴板内容无法按字节备份（GDI 句柄类格式），放弃本轮");
            return Err(());
        }
        Err(())
    }
}

/// 恢复用户原内容：Empty 清空；Text 写回文本（空串≈清空）；Raw 逐格式写回原始字节。
/// 全程一次开关剪贴板（Raw 多格式也在同一次开闭里写完，中途别人抢不走）。
pub fn clipboard_restore(backup: &ClipboardBackup) -> bool {
    unsafe {
        // 注册格式预先解析：RegisterClipboardFormatW 不需要打开剪贴板，注册失败在
        // EmptyClipboard 之前就退出——绝不半写一个「已清空」的快照（评审 P1）
        let raw_writes = match backup {
            ClipboardBackup::Raw(fmts) => {
                let Some(writes) = fmts
                    .iter()
                    .map(|f| {
                        let id = match &f.name {
                            Some(n) => register_format(n),
                            None => Some(f.fmt),
                        };
                        id.map(|id| (id, f.bytes.as_slice()))
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    return false;
                };
                writes
            }
            _ => Vec::new(),
        };
        for attempt in 0..=RESTORE_RETRIES {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(RETRY_MS));
            }
            if OpenClipboard(HWND::default()).is_err() {
                continue;
            }
            let ok = EmptyClipboard().is_ok()
                && match backup {
                    // 原本无内容/无文本：EmptyClipboard 成功即还原到空态
                    ClipboardBackup::Empty => true,
                    ClipboardBackup::Text(t) => t.is_empty() || set_cf_unicode_text_locked(t),
                    ClipboardBackup::Raw(_) => raw_writes
                        .iter()
                        .all(|(fmt, bytes)| set_raw_format_locked(*fmt, bytes)),
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

/// 读一种格式的全局内存字节。None = GDI 句柄类格式（非全局内存）或读失败。
/// 纯 CF_BITMAP 场景：Windows 一般会给 CF_DIB/CF_DIBV5 派生格式，字节备份走它们，
/// 恢复后粘贴目标仍能拿到图；只丢「还原后立即存在同款 GDI 句柄」这一表现
unsafe fn raw_format_bytes(fmt: u32) -> Option<Vec<u8>> {
    if matches!(
        fmt,
        CF_BITMAP_ID | CF_METAFILEPICT_ID | CF_PALETTE_ID | CF_ENHMETAFILE_ID
    ) {
        return None;
    }
    let hglobal = HGLOBAL(GetClipboardData(fmt).ok()?.0);
    let size = GlobalSize(hglobal);
    if size == 0 {
        return None;
    }
    let ptr = GlobalLock(hglobal) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
    let _ = GlobalUnlock(hglobal);
    Some(bytes)
}

/// 注册格式（>=0xC000）的名字；标准格式返回 None（恢复时直接用数字 ID）
unsafe fn registered_format_name(fmt: u32) -> Option<String> {
    if fmt < 0xC000 {
        return None;
    }
    let mut buf = [0u16; 256];
    let n = GetClipboardFormatNameW(fmt, &mut buf);
    if n == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..n as usize]))
}

fn register_format(name: &str) -> Option<u32> {
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    wide.push(0);
    let id = unsafe { RegisterClipboardFormatW(windows::core::PCWSTR(wide.as_ptr())) };
    (id != 0).then_some(id)
}

/// 须在 OpenClipboard + EmptyClipboard 之后调用。SetClipboardData 成功后这块 HGLOBAL
/// 归系统所有（不能再 GlobalFree）；只有失败路径才回收。
unsafe fn set_raw_format_locked(fmt: u32, bytes: &[u8]) -> bool {
    let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
        return false;
    };
    let ptr = GlobalLock(hmem);
    if ptr.is_null() {
        let _ = windows::Win32::Foundation::GlobalFree(hmem);
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    let _ = GlobalUnlock(hmem);
    if SetClipboardData(fmt, HANDLE(hmem.0)).is_err() {
        let _ = windows::Win32::Foundation::GlobalFree(hmem);
        return false;
    }
    true
}

/// UTF-16LE（NUL 结尾）字节转字符串；给 mod.rs 的 ClipboardBackup::text() 用
pub(crate) fn utf16_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

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

/// 须在 OpenClipboard + EmptyClipboard 之后调用（文本 = CF_UNICODETEXT 原始字节的特例）
unsafe fn set_cf_unicode_text_locked(text: &str) -> bool {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    set_raw_format_locked(CF_UNICODETEXT_ID, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
    };

    /// 两个测试共用真剪贴板，必须串行
    static CLIP_LOCK: Mutex<()> = Mutex::new(());
    const CF_DIB_ID: u32 = 8;

    fn write_clipboard(setup: impl FnOnce()) -> (u64, u64) {
        let before = clipboard_seq();
        unsafe {
            let mut tries = 0;
            loop {
                if OpenClipboard(HWND::default()).is_ok() {
                    break;
                }
                tries += 1;
                assert!(tries < 100, "打不开剪贴板");
                std::thread::sleep(std::time::Duration::from_millis(RETRY_MS));
            }
            EmptyClipboard().expect("EmptyClipboard");
            setup();
            let _ = CloseClipboard();
        }
        (before, clipboard_seq())
    }

    /// 读某格式当前字节（验证用）
    fn peek_raw(fmt: u32) -> Option<Vec<u8>> {
        unsafe {
            if OpenClipboard(HWND::default()).is_err() {
                return None;
            }
            let r = super::raw_format_bytes(fmt);
            let _ = CloseClipboard();
            r
        }
    }

    /// 真操作系统剪贴板的往返自检（动的是用户剪贴板，默认 ignore；手动 `cargo test -- --ignored` 跑）
    #[test]
    #[ignore]
    fn write_read_restore_roundtrip() {
        let _g = CLIP_LOCK.lock().unwrap();
        // 先备份用户现有内容，测完原样放回
        let backup = clipboard_backup(8).expect("打不开剪贴板");
        let marker = "__wordpeek_clipboard_roundtrip__";
        let (s0, s1) = write_clipboard(|| unsafe { assert!(set_cf_unicode_text_locked(marker)) });
        assert_ne!(s1, s0, "写入后序号必须变化");
        assert_eq!(clipboard_read_text(0), Ok(Some(marker.to_string())), "标记回读不一致");
        // 恢复到备份（文本/空态）后再验证读回与备份一致
        assert!(clipboard_restore(&backup), "恢复失败");
        let read = clipboard_read_text(0);
        assert_eq!(read, Ok(backup.text()), "read={read:?} backup_text={:?}", backup.text());
    }

    /// 非文本内容（自定义注册格式 + 伪 DIB）也能按字节备份/还原——
    /// 这是「剪贴板有图片/文件时 Alt+Z 照常工作」的核心保证
    #[test]
    #[ignore]
    fn raw_backup_restore_roundtrip() {
        let _g = CLIP_LOCK.lock().unwrap();
        let user = clipboard_backup(8).expect("备份用户内容失败");
        const FMT_NAME: &str = "Wordpeek.Test.Format";
        const PAYLOAD: &[u8] = b"hello-raw-payload";
        let fake_dib: Vec<u8> = {
            let mut v = vec![0u8; 44]; // 40B BITMAPINFOHEADER + 1 像素占位，restore 不校验内容
            v[0] = 40; // biSize
            v
        };
        let (s0, s1) = write_clipboard(|| unsafe {
            assert!(set_cf_unicode_text_locked("__wp_raw_roundtrip__"));
            let fmt = RegisterClipboardFormatW(windows::core::w!("Wordpeek.Test.Format"));
            assert_ne!(fmt, 0, "注册格式失败");
            assert!(set_raw_format_locked(fmt, PAYLOAD));
            assert!(set_raw_format_locked(CF_DIB_ID, &fake_dib));
        });
        let fmt = unsafe { RegisterClipboardFormatW(windows::core::w!("Wordpeek.Test.Format")) };

        let backup = clipboard_backup(8).expect("非文本内容也应可按字节备份");
        let ClipboardBackup::Raw(fmts) = &backup else {
            panic!("含非文本内容的快照应是 Raw，实际 {:?}", backup.text());
        };
        assert!(fmts.iter().any(|f| f.fmt == fmt && f.bytes == PAYLOAD));
        assert!(fmts.iter().any(|f| f.fmt == CF_DIB_ID && f.bytes == fake_dib));
        assert!(backup.text().as_deref() == Some("__wp_raw_roundtrip__"));

        // 抹掉 → 恢复 → 三种格式全部回来、字节逐位一致
        assert!(clipboard_restore(&ClipboardBackup::Empty));
        assert_ne!(clipboard_seq(), s1, "恢复后序号必须变化");
        assert!(clipboard_restore(&backup));
        assert_eq!(peek_raw(fmt).as_deref(), Some(PAYLOAD));
        assert_eq!(peek_raw(CF_DIB_ID).as_deref(), Some(&fake_dib[..]));
        assert_eq!(clipboard_read_text(0), Ok(Some("__wp_raw_roundtrip__".to_string())));

        clipboard_restore(&user);
    }
}
