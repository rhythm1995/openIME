//! R11：IME profile 快照与恢复决策（纯函数，跨平台可单测）。
//! 快照/决策类型由 Windows FFI（阶段 B）消费；纯函数层暂未引用，故允许未使用。

/// 激活前快照的输入法 profile。
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeProfileSnapshot {
    KeyboardLayout {
        lang: u16,
        hkl: u64,
    },
    TextService {
        lang: u16,
        clsid: String,
        profile_guid: String,
    },
}

/// 恢复决策：RestoreSavedProfile（还有快照且仍停在 openIME / 激活失败）｜KeepCurrentProfile。
#[allow(dead_code)]
pub enum ProfileRestoreDecision {
    RestoreSavedProfile,
    KeepCurrentProfile,
}

/// A11.3：有快照且（openIME 仍当前 或 激活失败）→ Restore；否则 Keep。
/// 用户在 restore 前手切走（openime_is_current=false 且未失败）→ 不强行切回。
#[allow(dead_code)]
pub fn restore_decision(
    saved: Option<&ImeProfileSnapshot>,
    openime_is_current: bool,
    activation_failed: bool,
) -> ProfileRestoreDecision {
    if saved.is_some() && (openime_is_current || activation_failed) {
        ProfileRestoreDecision::RestoreSavedProfile
    } else {
        ProfileRestoreDecision::KeepCurrentProfile
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> ImeProfileSnapshot {
        ImeProfileSnapshot::TextService {
            lang: 0x0804,
            clsid: "{...}".into(),
            profile_guid: "{...}".into(),
        }
    }

    #[test]
    fn restore_when_still_openime() {
        let s = snap();
        assert!(matches!(
            restore_decision(Some(&s), true, false),
            ProfileRestoreDecision::RestoreSavedProfile
        ));
    }

    #[test]
    fn restore_when_activation_failed() {
        let s = snap();
        assert!(matches!(
            restore_decision(Some(&s), false, true),
            ProfileRestoreDecision::RestoreSavedProfile
        ));
    }

    #[test]
    fn keep_when_user_switched_away() {
        let s = snap();
        assert!(matches!(
            restore_decision(Some(&s), false, false),
            ProfileRestoreDecision::KeepCurrentProfile
        ));
    }

    #[test]
    fn keep_when_no_snapshot() {
        assert!(matches!(
            restore_decision(None, true, true),
            ProfileRestoreDecision::KeepCurrentProfile
        ));
    }
}
