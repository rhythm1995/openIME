//! R7：Windows 前台进程 exe basename 捕获 / 还焦。
//!
//! - `frontmost_exe_basename`：GetForegroundWindow → GetWindowThreadProcessId →
//!   OpenProcess + QueryFullProcessImageNameW → 文件名（小写）。
//! - `activate_by_exe_basename`：EnumWindows 找 pid→exe 命中窗口 → SetForegroundWindow。
//! - `paste_fallback_apps` 与还焦共用同一 exe basename（与 macOS bundle id 同管道）。

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM};
// windows 0.58：`PWSTR` 不再位于 `Win32::Foundation`，而是 `windows::core::PWSTR`
// （来自 windows-strings：`pub struct PWSTR(pub *mut u16)`，非类型别名）。
// `QueryFullProcessImageNameW` 形参即该结构体，需用 `PWSTR(ptr)` 构造。
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetLastActivePopup, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SetForegroundWindow,
};

/// 当前前台窗口所属进程的 exe basename（小写，如 "mstsc.exe"）。失败返回 None。
pub fn frontmost_exe_basename() -> Option<String> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        exe_basename_of_window(hwnd)
    }
}

/// R11（FR-11.10）：前台窗口的 pid / tid / 架构。管道名按 pid+tid 反查目标进程，
/// `machine` 用于 TSF 门控（仅 AMD64 走 TSF，其余直接 R7）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontmostProcessInfo {
    pub pid: u32,
    pub tid: u32,
    /// IMAGE_FILE_MACHINE_*（IsWow64Process2 的 process machine）。
    pub machine: u16,
}

pub fn frontmost_process_info() -> Option<FrontmostProcessInfo> {
    use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
    use windows::Win32::System::Threading::{IsWow64Process2, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        let tid = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if tid == 0 || pid == 0 {
            return None;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        // 原生 x64 进程：process machine == IMAGE_FILE_MACHINE_AMD64(0x8664)；
        // WOW64 的 32 位进程返回 0x014c，ARM64 进程 0xaa64 → 门控回退 R7。
        let mut process_machine = IMAGE_FILE_MACHINE::default();
        let mut native_machine = IMAGE_FILE_MACHINE::default();
        let ok = IsWow64Process2(process, &mut process_machine, Some(&mut native_machine)).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return None;
        }
        // 未运行在 WOW64 下时 process machine == UNKNOWN(0)，实际架构 = native。
        let machine = if process_machine.0 == 0 {
            native_machine.0
        } else {
            process_machine.0
        };
        Some(FrontmostProcessInfo { pid, tid, machine })
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
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
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
    if !ctx.found.0.is_null() {
        return BOOL(0); // 已找到，停止枚举
    }
    // 只激活可见、未最小化的窗口：跳过隐藏消息窗 / 托盘窗 / 最小化窗，
    // 否则可能唤醒错误窗口（EnumWindows 会枚举到这些）。
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return BOOL(1);
    }
    if let Some(exe) = exe_basename_of_window(hwnd) {
        if exe == ctx.target {
            // 有最近活跃的弹出子窗（对话框等）则优先激活它，否则激活 hwnd 本身。
            let popup = GetLastActivePopup(hwnd);
            ctx.found = if popup.0.is_null() { hwnd } else { popup };
            return BOOL(0);
        }
    }
    BOOL(1)
}

/// 枚举顶层窗找可激活窗口：exe basename 命中、可见、未最小化。
/// EnumWindows 按 Z-order 自顶向下，首个命中即最靠前；命中后优先取其最近活跃的
/// 弹出子窗（GetLastActivePopup，对话框等）。不执行激活（与 SetForegroundWindow 解耦，便于真机单测）。
pub(crate) fn find_activatable_window(exe: &str) -> Option<HWND> {
    let target = exe.trim().to_lowercase();
    if target.is_empty() {
        return None;
    }
    let mut ctx = FindCtx {
        target,
        found: HWND(std::ptr::null_mut()),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut FindCtx as isize));
        if ctx.found.0.is_null() {
            None
        } else {
            Some(ctx.found)
        }
    }
}

/// 枚举顶层窗，pid→exe 命中则 SetForegroundWindow（录音结束后还焦用）。
/// 只考虑可见、未最小化的窗口；命中后优先激活其最近活跃的弹出子窗（GetLastActivePopup）。
pub fn activate_by_exe_basename(exe: &str) -> bool {
    match find_activatable_window(exe) {
        Some(hwnd) => unsafe { SetForegroundWindow(hwnd).as_bool() },
        None => false,
    }
}

/// 真机行为测试共享工具：创建真实顶层窗口 + 跨测试串行锁。
/// 窗口枚举 / 前台断言受全局桌面状态影响，涉及窗口的测试之间不能并行。
#[cfg(test)]
pub(crate) mod test_util {
    use std::sync::{Mutex, OnceLock};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, ShowWindow, SW_MINIMIZE,
        WINDOW_EX_STYLE, WNDCLASSW, WNDCLASS_STYLES, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    const TEST_CLASS: &str = "OpenImeTestWindowClass";
    const TEST_TITLE: &str = "openime-test";

    /// 串行锁：窗口枚举 / 前台断言受全局桌面影响，相关测试不得并行。
    pub static SERIAL: Mutex<()> = Mutex::new(());

    /// DefWindowProcW 在 windows-rs 是泛型函数，不能直接 coerce 成 fn 指针；
    /// 包一层具体签名供 WNDCLASSW 使用。
    unsafe extern "system" fn test_wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    /// 注册一次测试窗口类（进程级，wndproc 用 DefWindowProcW）。
    fn register_test_class() {
        static REGISTERED: OnceLock<()> = OnceLock::new();
        REGISTERED.get_or_init(|| unsafe {
            let class: Vec<u16> = TEST_CLASS
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let module = GetModuleHandleW(None).ok().unwrap_or_default();
            let wc = WNDCLASSW {
                style: WNDCLASS_STYLES::default(),
                lpfnWndProc: Some(test_wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: HINSTANCE(module.0),
                hIcon: Default::default(),
                hCursor: Default::default(),
                hbrBackground: Default::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: PCWSTR(class.as_ptr()),
            };
            let atom = RegisterClassW(&wc);
            assert_ne!(atom, 0, "RegisterClassW 失败");
        });
    }

    /// 创建本进程的真实顶层测试窗口（同进程 = 同 exe，供按 exe 查找的测试用）。
    /// - `visible`：创建时带 WS_VISIBLE。
    /// - `iconic`：创建后立即最小化（IsWindowVisible 仍为 true，IsIconic 为 true）。
    pub fn create_test_window(visible: bool, iconic: bool) -> HWND {
        register_test_class();
        let class: Vec<u16> = TEST_CLASS
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let title: Vec<u16> = TEST_TITLE
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut style = WS_OVERLAPPEDWINDOW;
        if visible {
            style |= WS_VISIBLE;
        }
        unsafe {
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                style,
                0,
                0,
                300,
                200,
                None,
                None,
                None,
                None,
            )
            .expect("CreateWindowExW 失败");
            if iconic {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            }
            hwnd
        }
    }

    /// 销毁测试窗口（测试结束清理）。
    pub fn destroy_test_window(hwnd: HWND) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::{create_test_window, destroy_test_window, SERIAL};
    use super::*;

    #[test]
    fn invalid_hwnd_returns_none() {
        // 设计 R7 单元测试：假 HWND（null）→ None，不依赖真实桌面。
        assert!(exe_basename_of_window(HWND(std::ptr::null_mut())).is_none());
    }

    #[test]
    fn empty_exe_never_activates() {
        assert!(!activate_by_exe_basename(""));
        assert!(!activate_by_exe_basename("   "));
    }

    /// 真机行为验证（3.3）：本进程创建 可见 / 最小化 / 隐藏 三种真实窗口（同 exe），
    /// 查找必须命中「可见且未最小化」的那个；仅剩隐藏/最小化窗时返回 None。
    #[test]
    fn find_activatable_skips_hidden_and_iconic() {
        let _guard = SERIAL.lock().unwrap();
        let w_visible = create_test_window(true, false);
        let w_iconic = create_test_window(true, true);
        let w_hidden = create_test_window(false, false);
        let exe = std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let found = find_activatable_window(&exe);
        assert_eq!(
            found,
            Some(w_visible),
            "必须选中可见且未最小化的窗口（got {found:?}）"
        );

        // 销毁可见窗后：仅剩最小化/隐藏窗 → 无可激活候选。
        destroy_test_window(w_visible);
        assert_eq!(
            find_activatable_window(&exe),
            None,
            "仅剩隐藏/最小化窗口时必须返回 None"
        );

        destroy_test_window(w_iconic);
        destroy_test_window(w_hidden);
    }
}
