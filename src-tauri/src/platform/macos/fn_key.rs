//! Fn（🌐 Globe）键监听 + 前台 app 焦点管理 + overlay 无激活显示。

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};

extern "C" {
    fn openime_install_fn_monitor_objc();
    fn openime_frontmost_bundle_id() -> *const c_char;
    fn openime_activate_app(bundle_id: *const c_char) -> i32;
    fn openime_prepare_overlay_window(ns_window: *mut c_void);
    fn openime_show_overlay_preserving_focus(
        ns_window: *mut c_void,
        x: f64,
        y: f64,
        restore_bundle_id: *const c_char,
    );
    fn openime_hide_window_without_activating(ns_window: *mut c_void);
    fn openime_get_selection() -> *const c_char;
    fn openime_paste_cmd_v() -> i32;
    // R9：吞键下发 + 补发 🌐（HID flagsChanged）。
    fn openime_set_fn_tap_consume(consume: bool);
    fn openime_schedule_repost_fn();
    fn openime_repost_fn() -> i32;
}

static mut EDGE_CALLBACK: Option<fn(pressed: bool)> = None;

/// ObjC 文件调用此函数（C ABI）：flagsChanged 边沿检测后推送。
#[no_mangle]
pub extern "C" fn openime_fn_edge(pressed: bool) {
    let _ = std::panic::catch_unwind(|| unsafe {
        let cb = EDGE_CALLBACK;
        if let Some(cb) = cb {
            cb(pressed);
        }
    });
}

/// 安装 Fn 键监听。必须在主线程调用。
pub fn install_fn_monitor(on_edge: fn(pressed: bool)) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        EDGE_CALLBACK = Some(on_edge);
    }
    unsafe {
        openime_install_fn_monitor_objc();
    }
}

/// F4：读系统当前选中文字（macOS AX，不碰剪贴板）。无选中返回 None。
pub fn get_selection() -> Option<String> {
    unsafe {
        let ptr = openime_get_selection();
        if ptr.is_null() {
            return None;
        }
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        libc_free(ptr as *mut _);
        Some(s)
    }
}

/// 获取当前前台 app 的 bundle ID（录音前调用，录音后激活回去）。
pub fn frontmost_bundle_id() -> Option<String> {
    unsafe {
        let ptr = openime_frontmost_bundle_id();
        if ptr.is_null() {
            return None;
        }
        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        libc_free(ptr as *mut _);
        Some(s)
    }
}

/// 按 bundle ID 激活 app（录音结束后、enigo 输入前调用；也可在 overlay 显示后立即还焦）。
pub fn activate_app(bundle_id: &str) -> bool {
    let c = match CString::new(bundle_id) {
        Ok(c) => c,
        Err(_) => return false,
    };
    unsafe { openime_activate_app(c.as_ptr()) != 0 }
}

/// 配置 overlay 为 HUD 风格（鼠标穿透、不进窗口循环）。
pub fn prepare_overlay_window(ns_window: *mut c_void) {
    if ns_window.is_null() {
        return;
    }
    unsafe { openime_prepare_overlay_window(ns_window) };
}

/// 显示录音 overlay，并尽量保留调用前的焦点/光标。
///
/// - `x`/`y`：AppKit 坐标（左下角原点）下的窗口原点
/// - `restore_bundle_id`：显示前的前台 app；用于误抢激活时还焦
pub fn show_overlay_preserving_focus(
    ns_window: *mut c_void,
    x: f64,
    y: f64,
    restore_bundle_id: Option<&str>,
) {
    if ns_window.is_null() {
        return;
    }
    let c_bid = restore_bundle_id.and_then(|s| CString::new(s).ok());
    let ptr = c_bid
        .as_ref()
        .map(|c| c.as_ptr())
        .unwrap_or(std::ptr::null());
    unsafe {
        openime_show_overlay_preserving_focus(ns_window, x, y, ptr);
    }
}

/// 隐藏窗口且不触发激活切换。
pub fn hide_window_without_activating(ns_window: *mut c_void) {
    if ns_window.is_null() {
        return;
    }
    unsafe { openime_hide_window_without_activating(ns_window) };
}

/// R7：发送 Cmd+V 粘贴和弦（CGEvent）。成功 true。
pub fn paste_cmd_v() -> bool {
    unsafe { openime_paste_cmd_v() != 0 }
}

/// R9：下发「是否吞 Fn 键」到 ObjC tap（hotkey==Fn && Hold 才吞）。
pub fn set_fn_tap_consume(consume: bool) {
    unsafe { openime_set_fn_tap_consume(consume) };
}

/// R9：先写 ignore deadline，下一圈 main runloop 再补发一对 flagsChanged。
pub fn schedule_repost_fn() {
    unsafe { openime_schedule_repost_fn() };
}

/// R9：立即补发一对 flagsChanged（供测试/回退路径）。
#[allow(dead_code)]
pub fn repost_fn() -> bool {
    unsafe { openime_repost_fn() != 0 }
}

extern "C" {
    fn free(ptr: *mut std::ffi::c_void);
}

fn libc_free(ptr: *mut std::ffi::c_void) {
    unsafe { free(ptr) }
}
