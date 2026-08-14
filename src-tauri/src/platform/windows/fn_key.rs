//! Windows 版 fn_key 兼容层：macOS 薄壳的调用面在 Windows 上转发为 exe basename。
//! Fn（Globe）键监听由 `super::fn_monitor`（WH_KEYBOARD_LL 低阶键盘钩子）实现：
//! CapsLock 为「Fn 等价单键」（全键盘可靠可见），厂商 Fn 扫描码 best-effort；
//! overlay 无激活显示用 SW_SHOWNOACTIVATE 实现，选区读取由 `super::uia`（UI Automation）实现。
// 仅 macOS 调用路径使用的桩（prepare_overlay_window 等）在 Windows 上为预期死代码。
#![allow(dead_code)]

use super::focus;

/// 安装/更新单键钩子监听（hotkey 为 "Fn" / "CapsLock" 时有目标，其它组合键 → None 全放行）。
/// 返回是否真的有监听目标（Fn 在多数键盘上固件消费、系统不可见，调用方据此决定是否注册兜底组合键）。
pub fn install_fn_monitor_for(hotkey: &str) -> bool {
    let watch = crate::fn_policy::parse_watch_key(hotkey);
    super::fn_monitor::install(crate::on_fn_edge, hotkey);
    watch != crate::fn_policy::WatchKey::None
}

/// 吞键下发：Fn 仅 Hold 吞；CapsLock 两模式都吞（防大小写锁定被翻转）。见 `fn_policy::fn_tap_can_consume`。
pub fn set_fn_tap_consume(consume: bool) {
    super::fn_monitor::set_consume(consume);
}

/// 短按补发：CapsLock 目标重发一对按键恢复原功能；固件 Fn 无法合成（no-op）。
pub fn schedule_repost_fn() {
    super::fn_monitor::repost_capslock();
}

pub fn is_fn_registered() -> bool {
    super::fn_monitor::is_installed()
}

/// 前台 app 标识：Windows 用进程 exe basename（小写），与 macOS bundle id 同管道。
pub fn frontmost_bundle_id() -> Option<String> {
    focus::frontmost_exe_basename()
}

/// 按 exe basename 还焦（录音结束插入前）。
pub fn activate_app(bundle_id: &str) -> bool {
    focus::activate_by_exe_basename(bundle_id)
}

pub fn prepare_overlay_window(_ns_window: *mut std::ffi::c_void) {}

/// 不激活显示窗口（SW_SHOWNOACTIVATE）：overlay HUD 显示时不抢用户当前输入焦点。
/// `hwnd` 为 overlay 窗口的裸 HWND 指针（由 Tauri `WebviewWindow::hwnd()` 桥接）。
pub fn show_window_without_activating(hwnd: *mut std::ffi::c_void) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
    // ShowWindow 返回「此前是否可见」，非成败；失败场景（无效句柄）静默忽略。
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_SHOWNOACTIVATE);
    }
}

pub fn show_overlay_preserving_focus(
    _ns_window: *mut std::ffi::c_void,
    _x: f64,
    _y: f64,
    _restore_bundle_id: Option<&str>,
) {
}

pub fn hide_window_without_activating(_ns_window: *mut std::ffi::c_void) {}

/// 隐藏窗口（SW_HIDE 直调 HWND）。与 `show_window_without_activating` 对称：
/// 不走 Tauri 主线程调度（`run_on_main_sync` 的 1s 超时在主线程繁忙时——如 enigo
/// 插入文字后——会延迟/丢失 HUD 收起，导致 overlay 残留「正在聆听/…」）。
pub fn hide_window_raw(hwnd: *mut std::ffi::c_void) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    unsafe {
        let _ = ShowWindow(HWND(hwnd as _), SW_HIDE);
    }
}

/// 读焦点元素选中文本：UIA TextPattern（见 `super::uia`）。不支持 TextPattern 的应用返回 None。
pub fn get_selection() -> Option<String> {
    super::uia::get_selected_text()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::windows::focus::test_util::{create_test_window, destroy_test_window, SERIAL};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindowVisible};

    /// 真机行为验证（3.2）：SW_SHOWNOACTIVATE 显示窗口但不改变前台窗口。
    /// 本机交互会话会验证「真实前台窗口未被抢」；CI 服务会话前台为空（before==after==null）也成立。
    #[test]
    fn show_without_activating_keeps_foreground() {
        let _guard = SERIAL.lock().unwrap();
        let hwnd = create_test_window(false, false);
        unsafe {
            let fg_before = GetForegroundWindow();
            // windows 0.58 的 HWND.0 已是 *mut c_void，直接传。
            show_window_without_activating(hwnd.0);
            assert!(IsWindowVisible(hwnd).as_bool(), "窗口应已显示");
            let fg_after = GetForegroundWindow();
            assert_eq!(fg_before, fg_after, "前台窗口不应被测试窗口抢走");
            if !fg_after.0.is_null() {
                assert_ne!(fg_after, hwnd, "被激活的不应是测试窗口");
            }
            // 隐藏对称性：SW_HIDE 直调后必须立即不可见（overlay 残留修复的回归测试）。
            hide_window_raw(hwnd.0);
            assert!(!IsWindowVisible(hwnd).as_bool(), "窗口应已隐藏（直调 SW_HIDE）");
        }
        destroy_test_window(hwnd);
    }
}
