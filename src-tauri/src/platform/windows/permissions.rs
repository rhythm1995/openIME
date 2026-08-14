//! Windows 权限兼容实现。
//!
//! Windows 的授权模型与 macOS 明显不同，这里做三层兼容：
//!
//! 1. **辅助功能（Accessibility）**：Windows 上没有"辅助功能授权"这一 TCC 概念——
//!    文字注入走 enigo（SendInput / keybd_event），任何应用都可直接调用、无需授权。
//!    因此恒为 `Granted`，"授权 / 打开系统设置"按钮在 UI 上不再出现。
//!
//! 2. **麦克风（Microphone）**：系统**不允许**应用直接触发授权弹窗（首次使用麦克风时
//!    系统自动提示）。权限状态以注册表为准：
//!    `HKCU` 与 `HKLM` 的 `...\CapabilityAccessManager\ConsentStore\microphone`
//!      - `Value`             —— 全局"允许应用访问麦克风"总开关
//!      - `NonPackaged\Value` —— "允许桌面应用访问麦克风"开关（Win32 应用受此约束）
//!      - 任意层级（用户级 HKCU / 组策略级 HKLM）任意开关为 `Deny` → 已拒绝；
//!      - `NonPackaged\Value = Allow`（或键缺失）→ 已授权。
//!
//! 3. **深链**：pane 名沿用 macOS 面板名（`Privacy_Microphone` 等）做跨平台兼容，
//!    在 Windows 上映射为 `ms-settings:privacy-microphone` 打开系统设置。

use voice_core::permissions::{
    PermissionChecker, PermissionKind, PermissionState, PermissionStatus,
};

/// Windows 上文字注入无需授权 → 麦克风外恒为已授权。
pub struct MacPermissionChecker;

impl PermissionChecker for MacPermissionChecker {
    fn check(&self, kind: PermissionKind) -> PermissionStatus {
        let (state, hint) = match kind {
            PermissionKind::Accessibility => (
                PermissionState::Granted,
                "Windows 的文字注入无需辅助功能授权".to_string(),
            ),
            PermissionKind::Microphone => (
                microphone_state(),
                "Windows 设置 → 隐私和安全性 → 麦克风：打开“允许桌面应用访问你的麦克风”"
                    .to_string(),
            ),
        };
        PermissionStatus { kind, state, hint }
    }
}

/// 辅助功能：Windows 恒为已授权（无需授权提示）。
pub fn is_trusted(_prompt: bool) -> bool {
    true
}

/// 打开 Windows 设置对应隐私页。pane 沿用 macOS 面板名做跨平台兼容。
/// - `Privacy_Microphone`    → `ms-settings:privacy-microphone`
/// - `Privacy_Accessibility` → Windows 无对应设置页（无需授权），空操作。
pub fn open_settings_pane(pane: &str) -> Result<(), String> {
    let uri = match pane {
        "Privacy_Microphone" => "ms-settings:privacy-microphone",
        _ => return Ok(()), // 辅助功能在 Windows 无对应设置页
    };
    open_ms_settings(uri)
}

/// 通过 `ms-settings:` URI 协议打开设置应用。
/// explorer.exe 能解析该 URI（绑定到设置应用）；失败时回退 rundll32 的
/// FileProtocolHandler（经典 URI 打开方式）。
fn open_ms_settings(uri: &str) -> Result<(), String> {
    let err = match std::process::Command::new("explorer.exe").arg(uri).spawn() {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    std::process::Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", uri])
        .spawn()
        .map_err(|e2| format!("打开 Windows 设置失败：{err}（rundll32: {e2}）"))?;
    Ok(())
}

// ──────────────── 麦克风授权状态（注册表 ConsentStore） ────────────────

/// ConsentStore 麦克风分支（注册表根键下的路径，按 `\` 分段）。
const MIC_STORE_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
/// HKLM 下的同构路径（管理员 / 组策略级开关，企业环境用）。
const MIC_STORE_SUBKEY_HKLM: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// 读取指定注册表根键下某子键的 `Value` 字符串；键或值缺失 → None。
/// `root` 用 `winreg::HKEY`（isize 别名，常量见 `winreg::enums`）。
fn consent_value_in(root: winreg::HKEY, subpath: &str) -> Option<String> {
    use winreg::RegKey;
    RegKey::predef(root)
        .open_subkey(subpath)
        .ok()?
        .get_value("Value")
        .ok()
}

/// 纯决策：任一层级开关为 `Deny` 即拒绝（注册表读取不可控，抽出供单测）。
fn any_deny(master: Option<&str>, desktop_apps: Option<&str>) -> bool {
    master == Some("Deny") || desktop_apps == Some("Deny")
}

/// 麦克风授权状态。
/// - 任意一级开关为 `Deny` → `Denied`（全局关闭 / 桌面应用关闭）。
/// - 总开关与桌面应用开关允许（含键缺失、默认放行）→ `Granted`。
/// - HKCU（用户级）与 HKLM（组策略级）都检查：企业环境管理员在 HKLM 禁用麦克风时，
///   用户级显示 Allow 也须判为 Denied（任一层级 Deny 生效）。
///
/// 注：不存在 `NotDetermined` 之外的兜底——Win32 应用首次使用麦克风由系统自动提示，
/// 未出现拒绝即视为可用，让"授权"按钮保持隐藏、不打扰用户。
pub fn microphone_state() -> PermissionState {
    for (root, store) in [
        (winreg::enums::HKEY_CURRENT_USER, MIC_STORE_SUBKEY),
        (winreg::enums::HKEY_LOCAL_MACHINE, MIC_STORE_SUBKEY_HKLM),
    ] {
        let master = consent_value_in(root, store);
        let desktop_apps = consent_value_in(root, &format!(r"{store}\NonPackaged"));
        if any_deny(master.as_deref(), desktop_apps.as_deref()) {
            return PermissionState::Denied;
        }
    }
    PermissionState::Granted
}

/// 麦克风预检：状态已定（授权/拒绝）→ Some，无需发起请求。Windows 无请求 API，恒 Some。
pub fn microphone_preflight() -> Option<bool> {
    Some(matches!(microphone_state(), PermissionState::Granted))
}

/// Windows 无"发起授权请求"API（首次使用自动提示），保持桩：返回 false 表示
/// 未发起，由 request_microphone 命令回退到打开系统设置。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助功能在 Windows 无需授权，恒为已授权。
    #[test]
    fn accessibility_always_granted() {
        let c = MacPermissionChecker;
        let s = c.check(PermissionKind::Accessibility);
        assert_eq!(s.state, PermissionState::Granted);
        // 依赖真实注册表（本机/CI 默认放行，未显式拒绝即 Granted）。
        assert!(is_trusted(false));
    }

    /// 各级开关均为 Allow / 键缺失 → 已授权（默认放行）。
    #[test]
    fn mic_granted_when_not_denied() {
        assert_eq!(microphone_state(), PermissionState::Granted);
        assert_eq!(microphone_preflight(), Some(true));
    }

    /// 纯决策函数：Deny 判定不依赖真实注册表（HKCU/HKLM 两级共用同一逻辑）。
    #[test]
    fn any_deny_detects_both_levels() {
        assert!(!any_deny(None, None));
        assert!(!any_deny(Some("Allow"), Some("Allow")));
        assert!(any_deny(Some("Deny"), None));
        assert!(any_deny(None, Some("Deny")));
        assert!(any_deny(Some("Allow"), Some("Deny")));
    }

    /// 深链映射：麦克风 → ms-settings URI；辅助功能 → 空操作不报错。
    #[test]
    fn settings_pane_mapping() {
        // 打开 ms-settings 是副作用，这里只验证辅助功能空操作分支。
        assert!(open_settings_pane("Privacy_Accessibility").is_ok());
    }
}
