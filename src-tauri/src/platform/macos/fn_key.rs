//! Fn（🌐 Globe）键监听 + 前台 app 焦点管理。

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, Ordering};

extern "C" {
    fn openime_install_fn_monitor_objc();
    fn openime_frontmost_bundle_id() -> *const c_char;
    fn openime_activate_app(bundle_id: *const c_char) -> i32;
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

/// 按 bundle ID 激活 app（录音结束后、enigo 输入前调用）。
pub fn activate_app(bundle_id: &str) -> bool {
    let c = match CString::new(bundle_id) {
        Ok(c) => c,
        Err(_) => return false,
    };
    unsafe { openime_activate_app(c.as_ptr()) != 0 }
}

extern "C" {
    fn free(ptr: *mut std::ffi::c_void);
}

fn libc_free(ptr: *mut std::ffi::c_void) {
    unsafe { free(ptr) }
}
