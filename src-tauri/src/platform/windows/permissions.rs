//! Windows 权限：桩。麦克风/辅助功能授权模型与 macOS 不同，P1 不做自动检测
//! （Windows 上 enigo/Ctrl+V 不需要 Accessibility 授权；麦克风由系统提示）。

use voice_core::permissions::{
    PermissionChecker, PermissionKind, PermissionState, PermissionStatus,
};

pub struct MacPermissionChecker;

impl PermissionChecker for MacPermissionChecker {
    fn check(&self, kind: PermissionKind) -> PermissionStatus {
        PermissionStatus {
            kind,
            state: PermissionState::NotDetermined,
            hint: "Windows 暂不支持自动检测权限".to_string(),
        }
    }
}

pub fn is_trusted(_prompt: bool) -> bool {
    false
}

pub fn open_settings_pane(_pane: &str) -> Result<(), String> {
    Err("Windows 暂不支持".to_string())
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
