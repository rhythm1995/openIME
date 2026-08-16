//! 平台相关实现。macOS 全功能；Windows 前台 exe 捕获 / 还焦 / Ctrl+V（R7 P1）；
//! 其它平台提供桩实现保证编译。

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub use macos as current;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "windows")]
pub use windows as current;

// 非 macOS / Windows 平台：桩实现，保证跨平台编译（权限功能仅 macOS 生效）。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub mod current {
    pub mod permissions {
        use voice_core::permissions::{
            PermissionChecker, PermissionKind, PermissionState, PermissionStatus,
        };

        pub struct MacPermissionChecker;

        impl PermissionChecker for MacPermissionChecker {
            fn check(&self, kind: PermissionKind) -> PermissionStatus {
                PermissionStatus {
                    kind,
                    state: PermissionState::NotDetermined,
                    hint: "当前平台暂不支持自动检测权限".to_string(),
                }
            }
        }

        pub fn is_trusted(_prompt: bool) -> bool {
            false
        }

        pub fn open_settings_pane(_pane: &str) -> Result<(), String> {
            Err("当前平台暂不支持".to_string())
        }

        pub fn microphone_preflight() -> Option<bool> {
            Some(false)
        }

        pub fn issue_microphone_request() -> bool {
            false
        }

        pub fn microphone_request_finished() -> bool {
            true
        }

        pub fn microphone_request_granted() -> bool {
            false
        }

        pub fn clear_microphone_request() {}
    }

    pub mod fn_key {
        pub fn install_fn_monitor(_on_edge: fn(pressed: bool)) {}
        pub fn set_fn_tap_consume(_consume: bool) {}
        pub fn schedule_repost_fn() {}
        pub fn is_fn_registered() -> bool {
            false
        }
        pub fn frontmost_bundle_id() -> Option<String> {
            None
        }
        pub fn activate_app(_bundle_id: &str) -> bool {
            false
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
    }
}
