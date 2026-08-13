//! R9：Fn 边沿策略纯函数（delay-start / 短按补发 / Toggle 松开不停）。
//! 与 macOS ObjC 监听、Tauri 全局状态解耦，可跨平台单测（A9.1b / 状态机表）。

/// Fn 边沿分类动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnEdgeAction {
    /// 忽略这次按下（Hold 且已在录音：不重复触发）。
    IgnorePress,
    /// 忽略这次松开（Toggle 松开不停；组合键松开不处理）。
    IgnoreRelease,
    /// 立即开始录音（Toggle 按下 / 组合键 / 翻译键）。
    StartRecord,
    /// 停止录音（Toggle 且已在录音：第二次按下停）。
    ToggleStop,
    /// Hold+Fn：武装 delay-start 计时器（阈值前不开录）。
    ArmHoldTimer,
    /// Hold+Fn 阈值前松开：只补发 🌐，不进 pipeline。
    RepostOnly,
    /// 已过阈值（或已在录）：松开走 300ms 尾音 → request_stop（识别+插入）。
    StopAfterTail,
}

/// `classify_fn_edge` 输入上下文（由调用方从 AppState 读取后传入）。
#[derive(Debug, Clone, Copy)]
pub struct FnEdgeContext {
    pub pressed: bool,
    /// `hotkey_mode == Hold`。
    pub hold: bool,
    /// 当前是否已在录音（recording_guard）。
    pub already_recording: bool,
    /// 本次按下是否真正 Start 过（过了阈值）。
    pub this_press_started_recording: bool,
    /// 本次按住的时长（毫秒，松开时提供；仅观测用）。
    pub duration_ms: Option<u64>,
    /// 录音键是否为 "Fn"（忽略大小写）。
    pub is_fn_hotkey: bool,
    /// `fn_repost_enabled`。
    pub fn_repost_enabled: bool,
}

/// 按 p2-design「R9 状态机表」分类一次 Fn 边沿。
///
/// - 短按补发只服务「录音键 == Fn 且 Hold」；Toggle 主手势是短触，不 delay。
/// - 松开分类看 `this_press_started_recording`：仅当**这一次** down 在 !already_recording
///   时武装、并在阈值后真正 Start 过，松开才按「自己的」会话处理。
/// - `classify` 永不产生 abort（R9 主路径不 request_abort）。
pub fn classify_fn_edge(c: &FnEdgeContext) -> FnEdgeAction {
    let _ = c.duration_ms; // 时长仅观测，不参与分类（阈值判断在 on_fn_edge 计时器里）。
    if c.pressed {
        if c.hold && c.already_recording {
            return FnEdgeAction::IgnorePress;
        }
        if !c.hold && c.already_recording {
            return FnEdgeAction::ToggleStop;
        }
        if c.hold && c.is_fn_hotkey {
            return FnEdgeAction::ArmHoldTimer;
        }
        return FnEdgeAction::StartRecord;
    }
    if !c.hold {
        return FnEdgeAction::IgnoreRelease;
    }
    if !c.is_fn_hotkey {
        return FnEdgeAction::IgnoreRelease;
    }
    if c.this_press_started_recording || c.already_recording {
        return FnEdgeAction::StopAfterTail;
    }
    if c.fn_repost_enabled {
        FnEdgeAction::RepostOnly
    } else {
        FnEdgeAction::IgnoreRelease
    }
}

/// 自捕获主过滤器：补发事件带 magic userdata 或仍在 ignore 窗口内 → 忽略。
/// deadline = post 前写入的 `now + REPOST_IGNORE_MS`。
/// 主过滤在 ObjC `cg_callback` / `handle_event`（已滤 magic + ignore window）；
/// 此处为纯函数防御/单测，故非测试构建允许未使用。
#[allow(dead_code)]
pub fn should_ignore_fn_edge(now_ms: u64, ignore_until_ms: u64, is_magic_userdata: bool) -> bool {
    is_magic_userdata || now_ms < ignore_until_ms
}

/// R9：`hotkey == "Fn"（忽略大小写）&& hotkey_mode == Hold` 才吞 Fn 键（delay-start 生效）。
/// 只改 hotkey_mode（不改 hotkey 字符串）时本判定会随之翻转，供 `store_fn_tap_consume` 下发。
pub fn fn_tap_can_consume(hotkey: &str, hold: bool) -> bool {
    hotkey.trim().eq_ignore_ascii_case("fn") && hold
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(pressed: bool, hold: bool, rec: bool, own: bool, is_fn: bool, repost: bool) -> FnEdgeContext {
        FnEdgeContext {
            pressed,
            hold,
            already_recording: rec,
            this_press_started_recording: own,
            duration_ms: None,
            is_fn_hotkey: is_fn,
            fn_repost_enabled: repost,
        }
    }

    #[test]
    fn classify_table_driven() {
        // (pressed, hold, rec, own, is_fn, repost, expected)
        let cases: Vec<(bool, bool, bool, bool, bool, bool, FnEdgeAction)> = vec![
            (true, true, false, false, true, true, FnEdgeAction::ArmHoldTimer),
            (true, true, true, false, true, true, FnEdgeAction::IgnorePress),
            (true, false, false, false, true, true, FnEdgeAction::StartRecord),
            (true, false, true, false, true, true, FnEdgeAction::ToggleStop),
            (false, true, false, false, true, true, FnEdgeAction::RepostOnly),
            (false, true, false, false, true, false, FnEdgeAction::IgnoreRelease),
            (false, true, true, true, true, true, FnEdgeAction::StopAfterTail),
            // 翻译/UI 已在录 + Hold Fn 短触：own=false → 停并插入（不 abort）。
            (false, true, true, false, true, true, FnEdgeAction::StopAfterTail),
            (false, false, true, false, true, true, FnEdgeAction::IgnoreRelease),
            // 组合键（is_fn=false）松开：无可靠 key-up → 忽略。
            (false, true, true, false, false, true, FnEdgeAction::IgnoreRelease),
            // 组合键按下：无 delay-start。
            (true, true, false, false, false, true, FnEdgeAction::StartRecord),
        ];
        for (pressed, hold, rec, own, is_fn, repost, expected) in cases {
            assert_eq!(
                classify_fn_edge(&ctx(pressed, hold, rec, own, is_fn, repost)),
                expected,
                "case pressed={pressed} hold={hold} rec={rec} own={own} is_fn={is_fn} repost={repost}"
            );
        }
    }

    #[test]
    fn should_ignore_fn_edge_window_and_magic() {
        // magic userdata 恒忽略（ObjC 已滤 magic，这里防御）。
        assert!(should_ignore_fn_edge(1000, 0, true));
        // 仍在 ignore 窗口内忽略。
        assert!(should_ignore_fn_edge(1000, 2000, false));
        // 过窗口且非 magic → 放行。
        assert!(!should_ignore_fn_edge(2000, 1000, false));
        // 边界：now == ignore_until → 放行。
        assert!(!should_ignore_fn_edge(1000, 1000, false));
    }

    #[test]
    fn fn_tap_can_consume_flips_with_hotkey_mode() {
        // 只改 hotkey_mode（不改 hotkey 字符串）→ 吞键判定翻转（R9 清单）。
        assert!(fn_tap_can_consume("Fn", true));
        assert!(fn_tap_can_consume("fn", true));
        assert!(fn_tap_can_consume(" Fn ", true));
        assert!(!fn_tap_can_consume("Fn", false)); // Toggle 不吞
        assert!(!fn_tap_can_consume("Alt+Shift+D", true)); // 组合键不吞
        assert!(!fn_tap_can_consume("", true));
    }
}
