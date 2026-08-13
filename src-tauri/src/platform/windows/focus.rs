//! R7：Windows 前台进程 exe basename 捕获 / 还焦。
//!
//! - `frontmost_exe_basename`：GetForegroundWindow → GetWindowThreadProcessId →
//!   OpenProcess + QueryFullProcessImageNameW → 文件名（小写）。
//! - `activate_by_exe_basename`：EnumWindows 找 pid→exe 命中窗口 → SetForegroundWindow。
//! - `paste_fallback_apps` 与还焦共用同一 exe basename（与 macOS bundle id 同管道）。

use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, PWSTR};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
};

/// 当前前台窗口所属进程的 exe basename（小写，如 "mstsc.exe"）。失败返回 None。
pub fn frontmost_exe_basename() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0 == 0 {
            return None;
        }
        exe_basename_of_window(hwnd)
    }
}

/// 给定 HWND，取其进程 exe basename（小写）。
fn exe_basename_of_window(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len: u32 = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
            .is_ok();
        let _ = CloseHandle(process);
        if !ok || len == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let base = path.rsplit(['\\', '/']).next()?.trim();
        if base.is_empty() {
            None
        } else {
            Some(base.to_lowercase())
        }
    }
}

/// 枚举上下文：目标 exe basename + 命中的顶层窗口。
struct FindCtx {
    target: String,
    found: HWND,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindCtx);
    if ctx.found.0 != 0 {
        return BOOL(0); // 已找到，停止枚举
    }
    if let Some(exe) = exe_basename_of_window(hwnd) {
        if exe == ctx.target {
            ctx.found = hwnd;
            return BOOL(0);
        }
    }
    BOOL(1)
}

/// 枚举顶层窗，pid→exe 命中则 SetForegroundWindow（录音结束后还焦用）。
pub fn activate_by_exe_basename(exe: &str) -> bool {
    let target = exe.trim().to_lowercase();
    if target.is_empty() {
        return false;
    }
    let mut ctx = FindCtx {
        target,
        found: HWND(0),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut FindCtx as isize));
        if ctx.found.0 == 0 {
            return false;
        }
        SetForegroundWindow(ctx.found).as_bool()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_hwnd_returns_none() {
        // 设计 R7 单元测试：假 HWND（0）→ None，不依赖真实桌面。
        assert!(exe_basename_of_window(HWND(0)).is_none());
    }

    #[test]
    fn empty_exe_never_activates() {
        assert!(!activate_by_exe_basename(""));
        assert!(!activate_by_exe_basename("   "));
    }
}
