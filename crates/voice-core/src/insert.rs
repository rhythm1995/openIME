//! 文本插入：把转写结果写入前台 App 的光标位置。
//!
//! 一期用 enigo 模拟键盘逐字输入（macOS CGEvent）。
//! 需 macOS 辅助功能权限（见 [`crate::permissions`]）。
//! R7 增加四态（Typed / Pasted / CopiedFallback / Failed）与剪贴板恢复纯逻辑；
//! 剪贴板本体 + 平台粘贴和弦在 Tauri 薄壳（insert_fallback.rs），voice-core 不依赖 arboard。
//!
//! 为可测：核心是 [`TextInserter`] trait（在 traits.rs），本模块提供 [`EnigoInserter`]。
//! Enigo 非 Send，故用 Mutex 包裹；测试用 RecordingInserter（见 tests）。

use std::sync::Mutex;

use async_trait::async_trait;
use enigo::{Enigo, Keyboard, Settings};

use crate::config::{AppConfig, InsertStrategy};
use crate::traits::TextInserter;
use crate::Error;

/// R7：一次插入的选项（由薄壳从 AppConfig + 前台 app 组装）。
/// P2 R11：`tsf_enabled` / `tsf_fallback` 由 [`InsertOpts::from_config`] 组装。
#[derive(Debug, Clone, Default)]
pub struct InsertOpts {
    pub strategy: InsertStrategy,
    /// 前台 app 标识命中任一条时视同 Paste（macOS bundle id / Windows exe basename）。
    pub paste_fallback_apps: Vec<String>,
    /// 粘贴后 750ms 恢复原剪贴板。
    pub restore_clipboard: bool,
    /// 本次插入时的前台 app 标识（粘贴策略命中判断用）。
    pub frontmost: Option<String>,
    /// P2 R11：优先走 TSF CommitText（是否已安装由 insert_ex 再查一次 status）。
    pub tsf_enabled: bool,
    /// P2 R11：TSF 提交失败回退 P1 R7 粘贴。
    pub tsf_fallback: bool,
}

impl InsertOpts {
    /// P2 R11：唯一业务构造器（`toggle_recording` 与 `qa::insert_last_answer` 都必须走它）。
    /// `tsf_enabled = windows && cfg.windows_tsf_enabled && !streaming`。
    pub fn from_config(cfg: &AppConfig, frontmost: Option<String>, streaming: bool) -> Self {
        let tsf = cfg!(windows) && cfg.windows_tsf_enabled && !streaming;
        Self {
            strategy: cfg.insert_strategy,
            paste_fallback_apps: cfg.paste_fallback_apps.clone(),
            restore_clipboard: cfg.restore_clipboard,
            frontmost,
            tsf_enabled: tsf,
            tsf_fallback: cfg.windows_tsf_fallback,
        }
    }
}

/// enigo 实现的文本插入器。逐字（Unicode）输入到当前键盘焦点。
pub struct EnigoInserter {
    enigo: Mutex<Enigo>,
}

impl EnigoInserter {
    pub fn new() -> crate::Result<Self> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| Error::Insert(format!("初始化 enigo 失败: {e}")))?;
        Ok(Self {
            enigo: Mutex::new(enigo),
        })
    }
}

#[async_trait]
impl TextInserter for EnigoInserter {
    async fn insert(&self, text: &str) -> crate::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let mut enigo = self
            .enigo
            .lock()
            .map_err(|e| Error::Insert(format!("enigo 锁中毒: {e}")))?;
        match enigo.text(text) {
            Ok(_) => {
                tracing::info!("enigo.insert 成功：{} 字", text.chars().count());
                Ok(())
            }
            Err(e) => {
                tracing::error!("enigo.insert 失败: {e}");
                Err(Error::Insert(format!("键盘输入失败: {e}")))
            }
        }
    }
}

/// 把字符串按"已插入前缀"去重，返回应新增输入的部分。
/// 用于 pipeline 在 partial 变化时只输入增量（一期可不用，保留工具）。
pub fn diff_prefix<'a>(previous: &'a str, current: &'a str) -> &'a str {
    let common = previous
        .chars()
        .zip(current.chars())
        .take_while(|(a, b)| a == b)
        .count();
    // start 是 current 中第 common 个字符的字节偏移。
    let start = current
        .char_indices()
        .nth(common)
        .map(|(i, _)| i)
        .unwrap_or(current.len());
    &current[start..]
}

/// R7：插入结果四态（Type-then-Paste 兜底）。
/// P2 R11：增加 `Committed`（TSF CommitText 成功，HUD 与 Typed 一样静默）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// enigo 模拟按键成功。
    Typed,
    /// 剪贴板粘贴（macOS Cmd+V / Windows Ctrl+V）成功。
    Pasted,
    /// 已写入剪贴板但粘贴和弦失败（用户需手动粘贴）。
    CopiedFallback,
    /// 打字与粘贴都失败。
    Failed,
    /// P2 R11：TSF CommitText 成功。
    Committed,
}

/// R7：是否恢复原剪贴板——仅当当前剪贴板内容仍是上次插入的文字时才恢复
/// （用户中途复制了别的则不覆盖；设计 FR-7.6）。纯函数，可单测。
pub fn should_restore_clipboard(
    current_clipboard_text: Option<&str>,
    last_inserted: &str,
) -> bool {
    match current_clipboard_text {
        Some(t) => !last_inserted.is_empty() && t == last_inserted,
        None => false,
    }
}

// ── R7：PendingRestore 状态机纯逻辑（薄壳持 Mutex，这里只做无锁决策）──

/// 一次待恢复的粘贴快照（进程级单例在薄壳，见 `src-tauri/insert_fallback.rs`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRestore {
    pub id: u64,
    /// 恢复目标（粘贴前的内容）。连续粘贴期间沿用第一次的 original。
    pub original: Option<String>,
    /// 本次写入剪贴板的文字（恢复前与剪贴板比对用）。
    pub last_inserted: String,
}

/// 粘贴成功后登记恢复。纯函数：
/// - original 链：沿用未完成的上次 original，否则用「本次粘贴前剪贴板内容」；
/// - 连续两句 Paste 的最终 original 是第一次之前的内容（A7.4）。
pub fn remember_pending(
    pending: Option<&PendingRestore>,
    id: u64,
    clipboard_before_overwrite: Option<&str>,
    inserted_text: &str,
) -> PendingRestore {
    PendingRestore {
        id,
        original: pending
            .and_then(|p| p.original.clone())
            .or_else(|| clipboard_before_overwrite.map(str::to_string)),
        last_inserted: inserted_text.to_string(),
    }
}

/// 750ms 恢复点到达时的决策。纯函数：
/// - id 不匹配（已有更新的粘贴）→ 不动作；
/// - 剪贴板仍是 last_inserted → 写回 original（None 只清空）；
/// - 剪贴板被用户改过（或非文本）→ 不覆盖，只清空登记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreDecision {
    DoNothing,
    Restore(String),
    Clear,
}

pub fn decide_restore(
    pending: Option<&PendingRestore>,
    my_id: u64,
    clipboard_now: Option<&str>,
) -> RestoreDecision {
    let Some(p) = pending else {
        return RestoreDecision::DoNothing;
    };
    if p.id != my_id {
        return RestoreDecision::DoNothing;
    }
    if should_restore_clipboard(clipboard_now, &p.last_inserted) {
        match &p.original {
            Some(original) => RestoreDecision::Restore(original.clone()),
            None => RestoreDecision::Clear,
        }
    } else {
        RestoreDecision::Clear
    }
}

/// 前台 app 标识是否命中粘贴兜底列表（FR-7.4）。
/// - macOS：bundle id 包含关键字（与 punct_half_width_apps 一致）。
/// - Windows：exe basename（小写）== kw、== kw+".exe" 或包含 kw。
///
/// 实现取并集（== / +".exe" / contains），跨平台可单测（A7.7）。
pub fn matches_paste_fallback(frontmost: Option<&str>, apps: &[String]) -> bool {
    let Some(f) = frontmost else {
        return false;
    };
    let f = f.trim();
    if f.is_empty() {
        return false;
    }
    let f_lower = f.to_lowercase();
    apps.iter().any(|kw| {
        let kw = kw.trim().to_lowercase();
        if kw.is_empty() {
            return false;
        }
        f_lower == kw || f_lower == format!("{kw}.exe") || f_lower.contains(&kw)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_prefix_returns_increment() {
        assert_eq!(diff_prefix("你好", "你好世界"), "世界");
        assert_eq!(diff_prefix("", "你好"), "你好");
        assert_eq!(diff_prefix("你好", "你好"), "");
        assert_eq!(diff_prefix("abc", "abd"), "d");
    }

    #[test]
    fn should_restore_when_unchanged() {
        assert!(should_restore_clipboard(Some("HELLO"), "HELLO"));
    }

    #[test]
    fn should_not_restore_when_user_copied_other() {
        assert!(!should_restore_clipboard(Some("OTHER"), "HELLO"));
    }

    #[test]
    fn should_not_restore_when_clipboard_non_text() {
        assert!(!should_restore_clipboard(None, "HELLO"));
    }

    #[test]
    fn should_not_restore_when_inserted_empty() {
        assert!(!should_restore_clipboard(Some(""), ""));
    }

    #[test]
    fn enigo_inserter_handles_empty() {
        // 在无头/CI 上构造 Enigo 可能失败；这里只测空串短路逻辑。
        if let Ok(ins) = EnigoInserter::new() {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let _ = rt.block_on(ins.insert(""));
        }
    }

    // ── R7：PendingRestore 状态机 ──

    #[test]
    fn remember_pending_first_paste_takes_clipboard_before() {
        let p = remember_pending(None, 1, Some("SECRET"), "HELLO");
        assert_eq!(p.original.as_deref(), Some("SECRET"));
        assert_eq!(p.last_inserted, "HELLO");
    }

    #[test]
    fn remember_pending_chains_original_across_pastes() {
        // A7.4：连续两句 Paste，最终 original 是第一次之前的内容。
        let p1 = remember_pending(None, 1, Some("SECRET"), "HELLO");
        let p2 = remember_pending(Some(&p1), 2, Some("HELLO"), "WORLD");
        assert_eq!(p2.original.as_deref(), Some("SECRET"));
        assert_eq!(p2.last_inserted, "WORLD");
    }

    #[test]
    fn remember_pending_without_prior_or_clipboard_text_has_no_original() {
        // 剪贴板非文本（图片）→ previous=None，恢复不写回。
        let p = remember_pending(None, 1, None, "HELLO");
        assert_eq!(p.original, None);
    }

    #[test]
    fn decide_restore_restores_when_unchanged() {
        let p = remember_pending(None, 1, Some("SECRET"), "HELLO");
        assert_eq!(
            decide_restore(Some(&p), 1, Some("HELLO")),
            RestoreDecision::Restore("SECRET".into())
        );
    }

    #[test]
    fn decide_restore_skips_when_user_copied_other() {
        // A7.3：200ms 内用户复制 OTHER → 不覆盖。
        let p = remember_pending(None, 1, Some("SECRET"), "HELLO");
        assert_eq!(
            decide_restore(Some(&p), 1, Some("OTHER")),
            RestoreDecision::Clear
        );
    }

    #[test]
    fn decide_restore_skips_when_stale_id() {
        // 750ms 前又发生了一次新粘贴：旧 id 的恢复任务不动作。
        let p1 = remember_pending(None, 1, Some("SECRET"), "HELLO");
        let p2 = remember_pending(Some(&p1), 2, Some("HELLO"), "WORLD");
        assert_eq!(
            decide_restore(Some(&p2), 1, Some("HELLO")),
            RestoreDecision::DoNothing
        );
        // 新 id 到期且未变 → 恢复第一次之前的内容。
        assert_eq!(
            decide_restore(Some(&p2), 2, Some("WORLD")),
            RestoreDecision::Restore("SECRET".into())
        );
    }

    #[test]
    fn decide_restore_non_text_clipboard_clears() {
        let p = remember_pending(None, 1, Some("SECRET"), "HELLO");
        assert_eq!(
            decide_restore(Some(&p), 1, None),
            RestoreDecision::Clear
        );
    }

    #[test]
    fn decide_restore_no_pending_is_noop() {
        assert_eq!(decide_restore(None, 1, Some("HELLO")), RestoreDecision::DoNothing);
    }

    #[test]
    fn paste_fallback_matching() {
        // A7.7：mstsc.exe 命中 mstsc。
        assert!(matches_paste_fallback(Some("mstsc.exe"), &["mstsc".into()]));
        // macOS bundle id contains。
        assert!(matches_paste_fallback(
            Some("com.microsoft.rdc.macos"),
            &["rdc".into()]
        ));
        assert!(matches_paste_fallback(Some("notepad.exe"), &["notepad".into()]));
        // 不区分大小写。
        assert!(matches_paste_fallback(Some("MSTSC.EXE"), &["mstsc".into()]));
        // 不命中 / 空列表 / 无前台。
        assert!(!matches_paste_fallback(Some("com.apple.notes"), &["mstsc".into()]));
        assert!(!matches_paste_fallback(Some("mstsc.exe"), &[]));
        assert!(!matches_paste_fallback(None, &["mstsc".into()]));
        assert!(!matches_paste_fallback(Some("mstsc.exe"), &["".into()]));
    }

    // ── P2 R11：InsertOpts::from_config ──

    #[test]
    fn from_config_mirrors_cfg_and_frontmost() {
        let cfg = AppConfig {
            insert_strategy: InsertStrategy::Paste,
            paste_fallback_apps: vec!["mstsc".into()],
            restore_clipboard: false,
            windows_tsf_enabled: true,
            windows_tsf_fallback: true,
            ..AppConfig::default()
        };
        let opts = InsertOpts::from_config(&cfg, Some("com.apple.notes".into()), false);
        assert_eq!(opts.strategy, InsertStrategy::Paste);
        assert_eq!(opts.paste_fallback_apps, vec!["mstsc".to_string()]);
        assert!(!opts.restore_clipboard);
        assert_eq!(opts.frontmost.as_deref(), Some("com.apple.notes"));
        assert!(opts.tsf_fallback);
    }

    #[test]
    fn from_config_tsf_disabled_on_non_windows_and_streaming() {
        // A11.8b：非 Windows 平台 tsf_enabled 恒 false；streaming=true 也 false。
        let cfg = AppConfig::default();
        // R11：TSF FFI 落地前 windows_tsf_enabled 默认 false，因此 Windows 默认也不启用；
        // FFI 落地、默认值改回 true 后，此断言需恢复为 #[cfg(not(windows))] 门控。
        assert!(!InsertOpts::from_config(&cfg, None, false).tsf_enabled);
        assert!(!InsertOpts::from_config(&cfg, None, true).tsf_enabled);
        // Default 漏填 = 静默降级。
        let d = InsertOpts::default();
        assert!(!d.tsf_enabled);
        assert!(!d.tsf_fallback);
    }
}
