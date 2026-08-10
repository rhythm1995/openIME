//! 权限模型：跨平台抽象 + 状态枚举。
//!
//! 平台 FFI（macOS Accessibility / 麦克风）放在 src-tauri 的 platform 模块，
//! 本模块只定义跨平台的值类型与 trait，便于在不依赖平台 API 的情况下测试。

use serde::{Deserialize, Serialize};

/// 权限类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Microphone,
    /// macOS 辅助功能（CGEvent 注入文本需要）。
    Accessibility,
}

/// 权限状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    /// 尚未询问。
    NotDetermined,
    /// 已授权。
    #[allow(dead_code)]
    Granted,
    /// 用户拒绝。
    Denied,
    /// 受限（家长控制等）。
    Restricted,
}

impl PermissionState {
    pub fn is_granted(self) -> bool {
        matches!(self, PermissionState::Granted)
    }
}

/// 一次权限查询的快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub kind: PermissionKind,
    pub state: PermissionState,
    /// 引导用户前往系统设置的说明文案（可本地化）。
    pub hint: String,
}

/// 权限检查器。真实现见 src-tauri/platform；测试用 mock。
pub trait PermissionChecker: Send + Sync {
    fn check(&self, kind: PermissionKind) -> PermissionStatus;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeChecker(MicrophoneState);
    #[derive(Clone, Copy)]
    #[allow(dead_code)]
    enum MicrophoneState {
        Granted,
        Denied,
    }
    impl PermissionChecker for FakeChecker {
        fn check(&self, kind: PermissionKind) -> PermissionStatus {
            let state = match (kind, self.0) {
                (PermissionKind::Microphone, MicrophoneState::Granted) => PermissionState::Granted,
                (PermissionKind::Microphone, MicrophoneState::Denied) => PermissionState::Denied,
                (PermissionKind::Accessibility, _) => PermissionState::Granted,
            };
            PermissionStatus {
                kind,
                state,
                hint: "前往系统设置".into(),
            }
        }
    }

    #[test]
    fn checker_reports_microphone_state() {
        let c = FakeChecker(MicrophoneState::Denied);
        let s = c.check(PermissionKind::Microphone);
        assert_eq!(s.state, PermissionState::Denied);
        assert!(!s.state.is_granted());
    }

    #[test]
    fn granted_is_granted() {
        assert!(PermissionState::Granted.is_granted());
        assert!(!PermissionState::NotDetermined.is_granted());
    }

    #[test]
    fn status_serializes() {
        let s = PermissionStatus {
            kind: PermissionKind::Microphone,
            state: PermissionState::Granted,
            hint: "x".into(),
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"kind\":\"microphone\""));
        assert!(j.contains("\"state\":\"granted\""));
    }
}
