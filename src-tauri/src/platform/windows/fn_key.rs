//! Windows 版 fn_key 兼容层：macOS 薄壳的调用面在 Windows 上转发为 exe basename。
//! Fn（Globe）键监听 / overlay 无激活显示 / AX 选区是 macOS 专属，Windows 为桩。

use super::focus;

/// Fn 键监听：Windows 不实现（P1 用全局快捷键）。
pub fn install_fn_monitor(_on_edge: fn(pressed: bool)) {}

/// R9：Fn 吞键下发——macOS 专属，Windows 无 Fn tap。
pub fn set_fn_tap_consume(_consume: bool) {}

/// R9：Fn 短按补发——macOS 专属，Windows 无 Fn tap。
pub fn schedule_repost_fn() {}

pub fn is_fn_registered() -> bool {
    false
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

pub fn show_window_without_activating(_ns_window: *mut std::ffi::c_void) {}

pub fn show_overlay_preserving_focus(
    _ns_window: *mut std::ffi::c_void,
    _x: f64,
    _y: f64,
    _restore_bundle_id: Option<&str>,
) {
}

pub fn hide_window_without_activating(_ns_window: *mut std::ffi::c_void) {}

/// AX 选区直读是 macOS 专属；Windows 读选区不在 P1。
pub fn get_selection() -> Option<String> {
    None
}
