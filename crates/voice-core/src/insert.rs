//! 文本插入：把转写结果写入前台 App 的光标位置。
//!
//! 一期用 enigo 模拟键盘逐字输入（macOS CGEvent）。
//! 需 macOS 辅助功能权限（见 [`crate::permissions`]）。
//! 二期再加剪贴板 + Cmd+V 兜底。
//!
//! 为可测：核心是 [`TextInserter`] trait（在 traits.rs），本模块提供 [`EnigoInserter`]。
//! Enigo 非 Send，故用 Mutex 包裹；测试用 RecordingInserter（见 tests）。

use std::sync::Mutex;

use async_trait::async_trait;
use enigo::{Enigo, Keyboard, Settings};

use crate::traits::TextInserter;
use crate::Error;

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
}
