//! Windows 单键监听（`WH_KEYBOARD_LL` 低阶键盘钩子 + Raw Input 观测兜底）——
//! macOS Fn 原生监听的等价物。
//!
//! 硬件事实（决定设计）：绝大多数笔记本键盘的 Fn 键由**键盘固件/EC** 消费，
//! 根本不会上报给 Windows（raw input / 低阶钩子都看不到）；个别键盘（部分
//! Dell/Lenovo 等）会把 Fn 作为厂商扫描码上报（`E0 63`），这里 best-effort 捕获。
//! 因此 Windows 上「类 Fn 单键」的一等公民是 **CapsLock**：
//! - 所有键盘都可靠上报，按住说话的体验与 Mac Fn 一致；
//! - 可吞键（Hold/Toggle 都吞，避免每次触发翻转大小写锁定）；
//! - 短按可补发（SendInput 重发一对 CapsLock，恢复原按键行为）。
//!
//! 双通道：
//! 1. **LL 钩子**（主）：可吞键；但 openIME 自有 WebView 窗口聚焦时被系统屏蔽
//!    （Tauri #14770，Chromium 输入管线接管键盘）。
//! 2. **Raw Input 观测**（兜底）：`RIDEV_INPUTSINK` 注册隐藏窗口，不受 #14770 影响，
//!    自己窗口聚焦时也能收到按键（设置页功能测试 / QA 面板录音可用）。
//!    不能吞键（事件照常到达系统）：Hold 按下+抬起各翻一次大小写 = 净零；
//!    短按翻一次 = CapsLock 原功能，语义自洽。
//!    两条通道对同一次按键的去重由 `classify_hook_event` 的按下状态机天然完成
//!    （钩子先到置位，raw 后到判为 repeat / 孤立 up → 不触发）。
//!
//! 结构：`classify_hook_event` 为纯函数（吞键 / 边沿 / auto-repeat 去重决策，
//! 可跨平台单测）；钩子线程 + 隐藏窗口 + SendInput 为薄封装。
//! 自捕获防护：补发前写 `ignore_deadline`，窗口内的注入事件放行给系统但不再触发边沿
//! （与 macOS magic userdata / ignore window 同思路，见 `fn_policy::should_ignore_fn_edge`）。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::fn_policy::{parse_watch_key, WatchKey};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_CAPITAL,
};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
    RAWINPUTHEADER, RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEKEYBOARD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLKHF_UP, RegisterClassW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WINDOW_EX_STYLE, WNDCLASSW,
    WNDCLASS_STYLES, WH_KEYBOARD_LL, WM_INPUT, RI_KEY_BREAK, RI_KEY_E0,
};

/// 补发 CapsLock 后忽略自捕获注入事件的窗口（毫秒）。SendInput 事件同步派发，250ms 余量足够。
const REPOST_IGNORE_MS: u64 = 250;
/// 部分键盘把 Fn 上报为厂商扫描码：E0 前缀（extended）+ 0x63。仅接受该形态，避免误吞其它 OEM 键。
const VENDOR_FN_SCAN: u32 = 0x63;
/// CapsLock 的 VK 码（VK_CAPITAL.0）。
const VK_CAPITAL_CODE: u32 = VK_CAPITAL.0 as u32;

// ──────────────── 纯函数决策（TDD 核心） ────────────────

/// 低阶钩子事件的平台无关快照（供 `classify_hook_event` 纯逻辑测试）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookEvent {
    pub vk: u32,
    pub scan: u32,
    pub extended: bool,
    pub injected: bool,
    pub up: bool,
}

/// 事件是否命中监听目标。
pub fn matches_watch(ev: &HookEvent, watch: WatchKey) -> bool {
    match watch {
        WatchKey::CapsLock => ev.vk == VK_CAPITAL_CODE,
        WatchKey::Fn => ev.extended && ev.scan == VENDOR_FN_SCAN,
        WatchKey::None => false,
    }
}

/// 对一个钩子事件做完整决策。
///
/// 返回 `(是否吞键, 边沿回调(None/Some(pressed)), watch 键新的按下状态)`：
/// - 吞键（consume）→ 钩子返回 1，事件不再到达系统/其它应用；
/// - 补发窗口内的注入事件**放行**（补发的意义就是让系统收到）但不触发边沿；
/// - auto-repeat 的重复 key-down 不重复触发边沿；无按下状态的 key-up 同样忽略。
#[allow(clippy::too_many_arguments)]
pub fn classify_hook_event(
    ev: &HookEvent,
    watch: WatchKey,
    consume: bool,
    ignore_injected_until_ms: u64,
    now_ms: u64,
    key_is_down: bool,
) -> (bool, Option<bool>, bool) {
    if watch == WatchKey::None || !matches_watch(ev, watch) {
        return (false, None, key_is_down);
    }
    if ev.injected && now_ms < ignore_injected_until_ms {
        return (false, None, key_is_down);
    }
    if ev.up {
        let edge = if key_is_down { Some(false) } else { None };
        return (consume, edge, false);
    }
    let edge = if key_is_down { None } else { Some(true) };
    (consume, edge, true)
}

/// 当前时间（UNIX_EPOCH 起毫秒），与 lib.rs `on_fn_edge` 同源。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ──────────────── 钩子线程与全局状态 ────────────────

/// watch 目标（AtomicU8 存储）。判别值与 `WatchKey` 枚举顺序解耦（显式映射，
/// 避免枚举重排悄悄改变存储语义）：0=None / 1=Fn / 2=CapsLock。
static WATCH: AtomicU8 = AtomicU8::new(0);
static CONSUME: AtomicBool = AtomicBool::new(false);
static IGNORE_INJECTED_UNTIL_MS: AtomicU64 = AtomicU64::new(0);
static KEY_IS_DOWN: AtomicBool = AtomicBool::new(false);
static EDGE_CB: OnceLock<fn(pressed: bool)> = OnceLock::new();
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
/// 最近一次边沿是否来自可吞键通道（LL 钩子）。Raw Input 兜底通道不吞键——
/// 原按键已直达系统，短按补发会造成「双翻转」，必须跳过（见 repost_capslock）。
static LAST_EDGE_FROM_HOOK: AtomicBool = AtomicBool::new(true);
static HOOK_THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
/// install / uninstall 串行锁。
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

fn watch_to_u8(w: WatchKey) -> u8 {
    match w {
        WatchKey::None => 0,
        WatchKey::Fn => 1,
        WatchKey::CapsLock => 2,
    }
}

fn watch() -> WatchKey {
    match WATCH.load(Ordering::SeqCst) {
        1 => WatchKey::Fn,
        2 => WatchKey::CapsLock,
        _ => WatchKey::None,
    }
}

/// 两条通道（LL 钩子 / Raw Input）共用的按键派发：分类 → 更新按下状态 → 触发边沿回调。
/// 返回是否应吞键（`allow_swallow=false` 的 Raw Input 通道恒不吞——它只是观测者）。
///
/// 去重：钩子先处理一次按键（状态机置位），Raw Input 随后到达同一按键时被判为
/// auto-repeat / 孤立 up → 不重复触发；#14770 场景（钩子被屏蔽）则只有 Raw Input
/// 到达，正常触发。补发（repost）忽略窗口内的 Raw 事件按注入处理（防自捕获）。
unsafe fn dispatch_key_event(ev: &HookEvent, allow_swallow: bool) -> bool {
    let now = now_ms();
    let deadline = IGNORE_INJECTED_UNTIL_MS.load(Ordering::SeqCst);
    // Raw Input 无法区分注入事件：处于补发忽略窗口内则视为注入（不触发边沿、不吞）。
    let ev = if !allow_swallow && !ev.injected && now < deadline {
        &HookEvent {
            injected: true,
            ..*ev
        }
    } else {
        ev
    };
    let (swallow, edge, is_down) = classify_hook_event(
        ev,
        watch(),
        CONSUME.load(Ordering::SeqCst),
        deadline,
        now,
        KEY_IS_DOWN.load(Ordering::SeqCst),
    );
    KEY_IS_DOWN.store(is_down, Ordering::SeqCst);
    if edge.is_some() {
        LAST_EDGE_FROM_HOOK.store(allow_swallow, Ordering::SeqCst);
    }
    if let Some(pressed) = edge {
        if let Some(cb) = EDGE_CB.get() {
            // 回调进 Tauri/业务逻辑，不能让 panic 毁掉钩子线程。
            let _ = std::panic::catch_unwind(|| cb(pressed));
        }
    }
    swallow && allow_swallow
}

/// 钩子回调（系统在钩子线程上下文调用，无捕获）。必须快：决策 → 边沿回调 → 转发。
unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        // HC_ACTION 之外（理论不会发生）：必须原样转发。
        return CallNextHookEx(None, code, wparam, lparam);
    }
    let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
    let ev = HookEvent {
        vk: kb.vkCode,
        scan: kb.scanCode,
        extended: kb.flags.contains(LLKHF_EXTENDED),
        injected: kb.flags.contains(LLKHF_INJECTED),
        up: kb.flags.contains(LLKHF_UP),
    };
    if dispatch_key_event(&ev, true) {
        LRESULT(1)
    } else {
        CallNextHookEx(None, code, wparam, lparam)
    }
}

// ──────────────── Raw Input 观测兜底（Tauri #14770） ────────────────

/// raw input 观测窗口类（进程级注册一次；线程可反复建/销毁窗口）。
const RAW_WND_CLASS: &str = "OpenImeFnMonitorRawInput";

unsafe extern "system" fn raw_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        handle_raw_input(lparam);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 解析 WM_INPUT 的 RAWKEYBOARD：命中监听目标 → 走共享派发（不吞键）。
unsafe fn handle_raw_input(lparam: LPARAM) {
    let mut size: u32 = 0;
    let hr = HRAWINPUT(lparam.0 as *mut core::ffi::c_void);
    // 第一次调用取所需大小（pdata=None）。
    if GetRawInputData(hr, RID_INPUT, None, &mut size, std::mem::size_of::<RAWINPUTHEADER>() as u32)
        == u32::MAX
    {
        return;
    }
    if size == 0 || size as usize > 1024 {
        return;
    }
    let mut buf = vec![0u8; size as usize];
    let copied = GetRawInputData(
        hr,
        RID_INPUT,
        Some(buf.as_mut_ptr().cast()),
        &mut size,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if copied == 0 || copied == u32::MAX {
        return;
    }
    let raw = &*(buf.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEKEYBOARD.0 {
        return;
    }
    let kb = &raw.data.keyboard;
    let ev = HookEvent {
        vk: kb.VKey as u32,
        scan: kb.MakeCode as u32,
        extended: (kb.Flags as u32 & RI_KEY_E0) != 0,
        injected: false, // Raw Input 无注入标记；补发窗口内在 dispatch 内按注入处理
        up: (kb.Flags as u32 & RI_KEY_BREAK) != 0,
    };
    if matches_watch(&ev, watch()) {
        dispatch_key_event(&ev, false);
    }
}

/// 创建 raw input 观测用的隐藏顶层窗口（不可见、不进任务栏，仅收 WM_INPUT）。
unsafe fn create_raw_input_window() -> HWND {
    use std::sync::OnceLock;
    static CLASS_ONCE: OnceLock<()> = OnceLock::new();
    CLASS_ONCE.get_or_init(|| {
        // DefWindowProcW 在 windows-rs 是泛型，包一层具体签名。
        unsafe extern "system" fn wnd_proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            raw_wnd_proc(hwnd, msg, wparam, lparam)
        }
        unsafe {
            let class: Vec<u16> = RAW_WND_CLASS
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let module = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .unwrap_or_default();
            let wc = WNDCLASSW {
                style: WNDCLASS_STYLES::default(),
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: windows::Win32::Foundation::HINSTANCE(module.0),
                hIcon: Default::default(),
                hCursor: Default::default(),
                hbrBackground: Default::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: PCWSTR(class.as_ptr()),
            };
            let atom = RegisterClassW(&wc);
            if atom == 0 {
                crate::log_warn!("raw input 窗口类注册失败（可能已注册，忽略）");
            }
        }
    });
    let class: Vec<u16> = RAW_WND_CLASS
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR::null(),
        Default::default(), // 无样式 = 隐藏窗口
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .unwrap_or_default();
    if hwnd.0.is_null() {
        crate::log_warn!("raw input 观测窗口创建失败");
    }
    hwnd
}

/// 安装（或复用）全局钩子线程，并更新监听目标。幂等；目标可热切换（改配置时复用同一钩子）。
pub fn install(on_edge: fn(pressed: bool), hotkey: &str) {
    let _ = EDGE_CB.set(on_edge);
    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    {
        let mut thread = HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner());
        if thread.is_none() {
            let handle = std::thread::Builder::new()
                .name("fn-keyboard-hook".into())
                .spawn(hook_thread_main)
                .expect("启动 fn-keyboard-hook 线程失败");
            *thread = Some(handle);
        }
    }
    // 等钩子线程完成 SetWindowsHookExW（HOOK_THREAD_ID 置位）再返回，
    // 保证 install 返回后事件不丢。
    let deadline = now_ms() + 2_000;
    while HOOK_THREAD_ID.load(Ordering::SeqCst) == 0 && now_ms() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    set_watch_key(parse_watch_key(hotkey));
}

/// 钩子线程主体：创建 raw input 观测窗口（#14770 兜底）→ 安装 WH_KEYBOARD_LL →
/// 消息泵（LL 钩子回调与 WM_INPUT 都依赖本线程取消息；WM_INPUT 需 Dispatch）→
/// WM_QUIT 时销毁窗口并卸载钩子。
fn hook_thread_main() {
    unsafe {
        // 1) Raw Input 观测兜底：RIDEV_INPUTSINK 允许无焦点接收（自己窗口聚焦时
        //    LL 钩子被 WebView 屏蔽，此通道仍可用）。
        let raw_hwnd = create_raw_input_window();
        if !raw_hwnd.0.is_null() {
            let rid = RAWINPUTDEVICE {
                usUsagePage: 0x01, // Generic Desktop
                usUsage: 0x06,     // Keyboard
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: raw_hwnd,
            };
            match RegisterRawInputDevices(
                &[rid],
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            ) {
                Ok(()) => crate::log_info!("raw input 观测已注册（#14770 兜底）"),
                Err(e) => crate::log_warn!("raw input 注册失败（仅 #14770 兜底降级）：{e}"),
            }
        }
        // 2) LL 钩子（主通道，可吞键）。
        let hook: Result<_, _> = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0);
        match hook {
            Ok(hhook) => {
                HOOK_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
                crate::log_info!("WH_KEYBOARD_LL 钩子已安装");
                let mut msg = Default::default();
                // GetMessageW：>0 收到消息；0 = WM_QUIT；-1 = 错误。
                // WM_INPUT 必须经 DispatchMessageW 才到 raw_wnd_proc。
                loop {
                    let r = GetMessageW(&mut msg, None, 0, 0);
                    if r.0 <= 0 {
                        break;
                    }
                    let _ = TranslateMessage(&msg);
                    let _ = DispatchMessageW(&msg);
                }
                let _ = UnhookWindowsHookEx(hhook);
            }
            Err(e) => {
                crate::log_error!("WH_KEYBOARD_LL 安装失败：{e}");
            }
        }
        if !raw_hwnd.0.is_null() {
            let _ = DestroyWindow(raw_hwnd);
        }
        HOOK_THREAD_ID.store(0, Ordering::SeqCst);
        KEY_IS_DOWN.store(false, Ordering::SeqCst);
    }
}

/// 更新监听目标（None = 钩子保留但全部放行，等价于不干预）。
pub fn set_watch_key(watch_key: WatchKey) {
    WATCH.store(watch_to_u8(watch_key), Ordering::SeqCst);
    // 目标切换后旧的按下状态无意义（比如 Fn→CapsLock 改配置的瞬间正按着键）。
    KEY_IS_DOWN.store(false, Ordering::SeqCst);
}

/// 下发「是否吞键」（策略见 `fn_policy::fn_tap_can_consume`）。
pub fn set_consume(consume: bool) {
    CONSUME.store(consume, Ordering::SeqCst);
}

/// 钩子是否在跑（供状态查询）。
pub fn is_installed() -> bool {
    HOOK_THREAD_ID.load(Ordering::SeqCst) != 0
}

/// 短按补发：仅 CapsLock 目标有意义（固件 Fn 键无法合成）。
/// 仅当原按键被钩子吞掉（未直达系统）时才补发；Raw Input 兜底通道不吞键，
/// 原按键已直达系统（caps 已翻转），再补发会双翻转。
/// 先写忽略窗口再 SendInput，防止补发的一对事件被自己再捕获。
pub fn repost_capslock() {
    if watch() != WatchKey::CapsLock {
        return;
    }
    if !LAST_EDGE_FROM_HOOK.load(Ordering::SeqCst) {
        crate::log_info!("短按补发跳过：本次边沿来自 raw input（原按键已直达系统）");
        return;
    }
    IGNORE_INJECTED_UNTIL_MS.store(now_ms() + REPOST_IGNORE_MS, Ordering::SeqCst);
    let down = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CAPITAL,
                wScan: 0,
                dwFlags: KEYBD_EVENT_FLAGS(0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let up = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_CAPITAL,
                wScan: 0,
                dwFlags: KEYEVENTF_KEYUP,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32) };
    if sent != 2 {
        crate::log_warn!("CapsLock 补发 SendInput 未完全成功：{sent}/2");
    }
}

/// 测试专用：卸载钩子线程并复位全局状态（生产路径常驻到进程退出）。
#[cfg(test)]
pub(crate) fn uninstall_for_test() {
    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let handle = {
        let mut thread = HOOK_THREAD.lock().unwrap_or_else(|p| p.into_inner());
        thread.take()
    };
    if let Some(handle) = handle {
        let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        let _ = handle.join();
    }
    WATCH.store(0, Ordering::SeqCst);
    CONSUME.store(false, Ordering::SeqCst);
    IGNORE_INJECTED_UNTIL_MS.store(0, Ordering::SeqCst);
    KEY_IS_DOWN.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tauri #14770 的同侧效应：本进程自己的窗口在前台时，本进程的 LL 钩子会被屏蔽。
    /// 全量测试里 focus 测试创建过可见窗口，可能残留前台 → 注入前把前台让给桌面（Progman）。
    fn force_foreground_off_self() {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
        };
        unsafe {
            let fg = GetForegroundWindow();
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            if pid != std::process::id() {
                return; // 前台本就不是本进程窗口，无需处理
            }
            let class: Vec<u16> = "Progman".encode_utf16().chain(std::iter::once(0)).collect();
            let progman = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()).unwrap_or_default();
            if progman.0.is_null() {
                return;
            }
            // ALT 按下绕过前台锁定，切前台后抬起。
            keybd_event(VK_MENU.0 as u8, 0x38, Default::default(), 0);
            let _ = SetForegroundWindow(progman);
            keybd_event(VK_MENU.0 as u8, 0x38, KEYEVENTF_KEYUP, 0);
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    fn ev(vk: u32, scan: u32, extended: bool, injected: bool, up: bool) -> HookEvent {
        HookEvent { vk, scan, extended, injected, up }
    }

    #[test]
    fn classify_matches_watch_targets_only() {
        // CapsLock 目标：只认 VK_CAPITAL。
        let caps = ev(VK_CAPITAL_CODE, 0x3A, false, false, false);
        assert!(matches_watch(&caps, WatchKey::CapsLock));
        assert!(!matches_watch(&ev(0x41, 0x1E, false, false, false), WatchKey::CapsLock));
        // Fn 目标：只认 extended 0x63（厂商上报形态）。
        assert!(matches_watch(&ev(0, 0x63, true, false, false), WatchKey::Fn));
        // 匹配只看扫描码形态（厂商 Fn 的 vkCode 无标准，可能是 0/0xFF/厂商值）。
        assert!(matches_watch(&ev(0xFF, 0x63, true, false, false), WatchKey::Fn));
        assert!(!matches_watch(&ev(0, 0x63, false, false, false), WatchKey::Fn));
        assert!(!matches_watch(&ev(0, 0x64, true, false, false), WatchKey::Fn));
        // None 目标：全部放行。
        assert!(!matches_watch(&caps, WatchKey::None));
    }

    #[test]
    fn classify_down_up_edges_and_repeat_dedupe() {
        let caps_down = ev(VK_CAPITAL_CODE, 0x3A, false, false, false);
        // 按下：触发 down 边沿；consume=true 时吞。
        assert_eq!(classify_hook_event(&caps_down, WatchKey::CapsLock, true, 0, 1_000, false), (true, Some(true), true));
        // auto-repeat 的重复 down：吞但不重复触发。
        assert_eq!(classify_hook_event(&caps_down, WatchKey::CapsLock, true, 0, 1_050, true), (true, None, true));
        // 抬起：触发 up 边沿。
        let caps_up = ev(VK_CAPITAL_CODE, 0x3A, false, false, true);
        assert_eq!(classify_hook_event(&caps_up, WatchKey::CapsLock, true, 0, 1_100, true), (true, Some(false), false));
        // 无按下状态的孤立 up：忽略。
        assert_eq!(classify_hook_event(&caps_up, WatchKey::CapsLock, true, 0, 1_200, false), (true, None, false));
        // consume=false：不吞但边沿照发。
        assert_eq!(classify_hook_event(&caps_down, WatchKey::CapsLock, false, 0, 1_300, false), (false, Some(true), true));
    }

    #[test]
    fn classify_non_target_passes_through() {
        let a_down = ev(0x41, 0x1E, false, false, false);
        // 非目标键：即便 consume=true 也不吞、不触发。
        assert_eq!(classify_hook_event(&a_down, WatchKey::CapsLock, true, 0, 1_000, false), (false, None, false));
        // watch=None（配置了组合键）：全放行。
        let caps_down = ev(VK_CAPITAL_CODE, 0x3A, false, false, false);
        assert_eq!(classify_hook_event(&caps_down, WatchKey::None, true, 0, 1_000, false), (false, None, false));
    }

    #[test]
    fn classify_ignores_own_repost_inside_window_but_passes_it() {
        let injected_down = ev(VK_CAPITAL_CODE, 0x3A, false, true, false);
        // 补发窗口内：放行（系统要收到这对 CapsLock）、不吞、不触发边沿。
        assert_eq!(
            classify_hook_event(&injected_down, WatchKey::CapsLock, true, 2_000, 1_800, false),
            (false, None, false)
        );
        // 窗口外（比如其它程序注入）：按真实按键处理。
        assert_eq!(
            classify_hook_event(&injected_down, WatchKey::CapsLock, true, 1_000, 1_800, false),
            (true, Some(true), true)
        );
    }

    /// 跨进程注入变体：由外部 PowerShell keybd_event 注入 CapsLock。
    /// 依赖真实桌面输入路径，交互式桌面（用户正在操作）下不稳定：
    /// 手动运行 `cargo test --lib real_hook -- --ignored --nocapture`。
    #[test]
    #[ignore = "真机金丝雀：依赖桌面输入环境，交互会话下不稳定，手动运行"]
    fn real_hook_captures_cross_process_injected_capslock() {
        use std::sync::mpsc;
        let _guard = crate::platform::windows::focus::test_util::SERIAL
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        static TX2: OnceLock<Mutex<Option<mpsc::Sender<bool>>>> = OnceLock::new();
        let tx_slot = TX2.get_or_init(|| Mutex::new(None));
        let (tx, rx) = mpsc::channel();
        *tx_slot.lock().unwrap() = Some(tx);

        install(
            |pressed: bool| {
                if let Some(tx) = TX2.get().and_then(|s| s.lock().ok()) {
                    if let Some(tx) = tx.as_ref() {
                        let _ = tx.send(pressed);
                    }
                }
            },
            "CapsLock",
        );
        assert!(is_installed());
        set_consume(true);

        let ps = r#"
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class KBT { [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra); }
'@
[KBT]::keybd_event(0x14, 0x3A, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 150
[KBT]::keybd_event(0x14, 0x3A, 2, [UIntPtr]::Zero)
"#;
        // 本进程窗口若在前台会屏蔽自己的 LL 钩子（Tauri #14770）→ 先把前台让出去，
        // 再由子进程 PowerShell 注入。前台状态受环境影响偶发抖动，最多重试 3 次。
        let mut first = Err(());
        for _ in 0..2 {
            force_foreground_off_self();
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", ps])
                .status()
                .expect("powershell 应能启动");
            assert!(out.success());
            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(v) => {
                    first = Ok(v);
                    break;
                }
                Err(_) => {
                    // 同进程首次装钩子偶发不投递（第二次稳定工作）：卸载重装再试。
                    uninstall_for_test();
                    install(
                        |pressed: bool| {
                            if let Some(tx) = TX2.get().and_then(|s| s.lock().ok()) {
                                if let Some(tx) = tx.as_ref() {
                                    let _ = tx.send(pressed);
                                }
                            }
                        },
                        "CapsLock",
                    );
                    set_consume(true);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            }
        }
        let first = first.expect("跨进程注入的按下边沿应到达（含重装重试）");
        let second = rx.recv_timeout(std::time::Duration::from_secs(3));
        assert!(first, "跨进程注入的按下边沿应为 true");
        assert_eq!(second, Ok(false), "跨进程注入的抬起边沿应到达");

        uninstall_for_test();
        *tx_slot.lock().unwrap() = None;
    }

    /// 真机端到端：装钩子 → SendInput 合成 CapsLock → 钩子必须捕获到一对边沿。
    /// consume=true，事件被吞 → 不改变本机真实的大小写锁定状态。
    /// 与真实窗口测试共用串行锁（全局钩子 + 真实输入事件，避免与其它输入测试交错）。
    /// 依赖真实桌面输入路径：手动运行 `cargo test --lib real_hook -- --ignored --nocapture`。
    #[test]
    #[ignore = "真机金丝雀：依赖桌面输入环境，交互会话下不稳定，手动运行"]
    fn real_hook_captures_injected_capslock() {
        use std::sync::mpsc;
        let _guard = crate::platform::windows::focus::test_util::SERIAL
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        static TX: OnceLock<Mutex<Option<mpsc::Sender<bool>>>> = OnceLock::new();
        let tx_slot = TX.get_or_init(|| Mutex::new(None));
        let (tx, rx) = mpsc::channel();
        *tx_slot.lock().unwrap() = Some(tx);

        install(
            |pressed: bool| {
                if let Some(tx) = TX.get().and_then(|s| s.lock().ok()) {
                    if let Some(tx) = tx.as_ref() {
                        let _ = tx.send(pressed);
                    }
                }
            },
            "CapsLock",
        );
        assert!(is_installed(), "钩子线程应已安装");
        set_consume(true);
        // 本进程窗口若在前台会屏蔽自己的 LL 钩子（Tauri #14770）→ 先把前台让出去。
        force_foreground_off_self();

        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CAPITAL,
                    wScan: 0,
                    dwFlags: KEYBD_EVENT_FLAGS(0),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_CAPITAL,
                    wScan: 0,
                    dwFlags: KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        // SendInput 同步走过低阶钩子后才返回。全量测试里前台状态受环境影响偶发抖动，
        // 最多重试 3 次（每次重试前再让一次前台）。
        let mut first = Err(());
        for _ in 0..2 {
            let sent = unsafe { SendInput(&[down, up], std::mem::size_of::<INPUT>() as i32) };
            assert_eq!(sent, 2, "SendInput 应送出两个事件");
            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(v) => {
                    first = Ok(v);
                    break;
                }
                Err(_) => {
                    // 同进程首次装钩子偶发不投递（第二次稳定工作）：卸载重装再试。
                    uninstall_for_test();
                    install(
                        |pressed: bool| {
                            if let Some(tx) = TX.get().and_then(|s| s.lock().ok()) {
                                if let Some(tx) = tx.as_ref() {
                                    let _ = tx.send(pressed);
                                }
                            }
                        },
                        "CapsLock",
                    );
                    set_consume(true);
                    force_foreground_off_self();
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            }
        }
        let first = first.expect("应收到按下边沿（含重装重试）");
        let second = rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("应收到抬起边沿");
        assert!(first, "第一个边沿应为按下");
        assert!(!second, "第二个边沿应为抬起");

        uninstall_for_test();
        assert!(!is_installed());
        *tx_slot.lock().unwrap() = None;
    }
}
