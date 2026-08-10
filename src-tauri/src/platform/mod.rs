//! 平台相关实现。一期仅 macOS；其他平台提供桩实现保证编译。

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub use macos as current;

// 非 macOS 平台：桩实现，保证跨平台编译（权限功能仅 macOS 生效）。
#[cfg(not(target_os = "macos"))]
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
        pub fn is_fn_registered() -> bool {
            false
        }
    }
}
