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
    /// 录音键是否为「单键」（macOS Fn / Windows CapsLock、best-effort Fn）。
    /// 单键才有 Hold delay-start / 短按补发语义（组合键无可靠 key-up）。
    pub is_single_key: bool,
    /// `fn_repost_enabled`。
    pub fn_repost_enabled: bool,
}

/// 单键监听目标（Windows 低阶键盘钩子 / macOS Fn 原生监听共用判定）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchKey {
    /// macOS 🌐 Fn（原生 NSEvent / CGEventTap）；Windows 上为 best-effort 厂商扫描码。
    Fn,
    /// Windows 的「Fn 等价单键」：所有键盘都可靠上报，可吞键 / 补发。
    CapsLock,
    /// 非单键（组合键 / 未知）→ 走 global-shortcut 注册。
    None,
}

/// 规范化 hotkey 字符串 → 单键监听目标。
/// 接受 "Fn"/"CapsLock"（大小写 / 首尾空格 / 内部空格与 `-`、`_` 变体）。
pub fn parse_watch_key(hotkey: &str) -> WatchKey {
    let normalized = hotkey
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "");
    match normalized.as_str() {
        "fn" | "globe" => WatchKey::Fn,
        "capslock" | "caps" => WatchKey::CapsLock,
        _ => WatchKey::None,
    }
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
        if c.hold && c.is_single_key {
            return FnEdgeAction::ArmHoldTimer;
        }
        return FnEdgeAction::StartRecord;
    }
    if !c.hold {
        return FnEdgeAction::IgnoreRelease;
    }
    if !c.is_single_key {
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

/// R9：单键且 Hold 才吞键（delay-start 生效）。
/// - Fn：仅 Hold 吞（Toggle 不吞，macOS 与 Windows 厂商 Fn 同策略）。
/// - Windows CapsLock：**两种模式都吞**——否则 Toggle 每次触发都会翻转大小写锁定
///   （Hold 也会在按下/抬起各翻一次），对文本输入是实打实的破坏。
///
/// 只改 hotkey_mode（不改 hotkey 字符串）时 Fn 的判定会随之翻转，供 `store_fn_tap_consume` 下发。
pub fn fn_tap_can_consume(hotkey: &str, hold: bool) -> bool {
    match parse_watch_key(hotkey) {
        WatchKey::Fn => hold,
        WatchKey::CapsLock => true,
        WatchKey::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(
        pressed: bool,
        hold: bool,
        rec: bool,
        own: bool,
        is_fn: bool,
        repost: bool,
    ) -> FnEdgeContext {
        FnEdgeContext {
            pressed,
            hold,
            already_recording: rec,
            this_press_started_recording: own,
            duration_ms: None,
            is_single_key: is_fn,
            fn_repost_enabled: repost,
        }
    }

    #[test]
    fn classify_table_driven() {
        // (pressed, hold, rec, own, is_fn, repost, expected)
        let cases: Vec<(bool, bool, bool, bool, bool, bool, FnEdgeAction)> = vec![
            (
                true,
                true,
                false,
                false,
                true,
                true,
                FnEdgeAction::ArmHoldTimer,
            ),
            (
                true,
                true,
                true,
                false,
                true,
                true,
                FnEdgeAction::IgnorePress,
            ),
            (
                true,
                false,
                false,
                false,
                true,
                true,
                FnEdgeAction::StartRecord,
            ),
            (
                true,
                false,
                true,
                false,
                true,
                true,
                FnEdgeAction::ToggleStop,
            ),
            (
                false,
                true,
                false,
                false,
                true,
                true,
                FnEdgeAction::RepostOnly,
            ),
            (
                false,
                true,
                false,
                false,
                true,
                false,
                FnEdgeAction::IgnoreRelease,
            ),
            (
                false,
                true,
                true,
                true,
                true,
                true,
                FnEdgeAction::StopAfterTail,
            ),
            // 翻译/UI 已在录 + Hold Fn 短触：own=false → 停并插入（不 abort）。
            (
                false,
                true,
                true,
                false,
                true,
                true,
                FnEdgeAction::StopAfterTail,
            ),
            (
                false,
                false,
                true,
                false,
                true,
                true,
                FnEdgeAction::IgnoreRelease,
            ),
            // 组合键（is_fn=false）松开：无可靠 key-up → 忽略。
            (
                false,
                true,
                true,
                false,
                false,
                true,
                FnEdgeAction::IgnoreRelease,
            ),
            // 组合键按下：无 delay-start。
            (
                true,
                true,
                false,
                false,
                false,
                true,
                FnEdgeAction::StartRecord,
            ),
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
        // 只改 hotkey_mode（不改 hotkey 字符串）→ Fn 的吞键判定翻转（R9 清单）。
        assert!(fn_tap_can_consume("Fn", true));
        assert!(fn_tap_can_consume("fn", true));
        assert!(fn_tap_can_consume(" Fn ", true));
        assert!(!fn_tap_can_consume("Fn", false)); // Toggle 不吞
                                                   // Windows CapsLock：两种模式都吞（Toggle 也要防止翻转大小写锁定）。
        assert!(fn_tap_can_consume("CapsLock", true));
        assert!(fn_tap_can_consume("caps lock", false));
        assert!(fn_tap_can_consume("CapsLock", false));
        assert!(!fn_tap_can_consume("Alt+Shift+D", true)); // 组合键不吞
        assert!(!fn_tap_can_consume("", true));
    }

    #[test]
    fn parse_watch_key_variants() {
        assert_eq!(parse_watch_key("Fn"), WatchKey::Fn);
        assert_eq!(parse_watch_key(" fn "), WatchKey::Fn);
        assert_eq!(parse_watch_key("Globe"), WatchKey::Fn);
        assert_eq!(parse_watch_key("CapsLock"), WatchKey::CapsLock);
        assert_eq!(parse_watch_key("caps lock"), WatchKey::CapsLock);
        assert_eq!(parse_watch_key("CAPS_LOCK"), WatchKey::CapsLock);
        assert_eq!(parse_watch_key("caps"), WatchKey::CapsLock);
        assert_eq!(parse_watch_key("Ctrl+Shift+D"), WatchKey::None);
        assert_eq!(parse_watch_key(""), WatchKey::None);
        // "Fn" 出现在组合键里不算单键。
        assert_eq!(parse_watch_key("Fn+D"), WatchKey::None);
    }
}
