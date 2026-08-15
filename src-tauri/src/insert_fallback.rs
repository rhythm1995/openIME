//! R7：粘贴兜底 + 剪贴板恢复（进程级状态机，macOS / Windows 共用）。
//!
//! 插入路径四态：
//! - `Auto`（默认）：enigo 逐字成功 → `Typed`（不碰剪贴板）；失败 → 粘贴。
//! - `Type`：只 enigo，失败 → `Failed`。
//! - `Paste`（或前台 app 命中 `paste_fallback_apps`）：只粘贴。
//! - 粘贴和弦失败 → `CopiedFallback`（文字留在剪贴板，HUD 提示手动粘贴）；不恢复。
//!
//! 剪贴板恢复（设计规范，两端同一实现）：
//! - 粘贴成功后登记 [`PendingRestore`]，750ms 后若剪贴板仍是插入文字则写回原内容；
//!   用户中途复制别的则不覆盖（相等才恢复）。
//! - 连续粘贴的 original 链取第一次之前的剪贴板内容。
//! - `Type` 成功不碰 PENDING（不取消进行中的上次恢复）。
//! - 剪贴板 get/set 与恢复串在 `CLIPBOARD_MU` 后，避免交叠。
//!
//! 平台粘贴和弦：macOS `Cmd+V`（CGEvent kVK_ANSI_V + Command，见 app_focus.m）；
//! Windows `Ctrl+V`（enigo Key::Control + Key::V 成对 Press/Click/Release）。
//! Linux 不实现和弦 → 粘贴直接失败（`Failed`）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use tauri::AppHandle;
use voice_core::insert::{
    decide_restore, remember_pending, InsertOpts, InsertOutcome, PendingRestore, RestoreDecision,
};
use voice_core::{EnigoInserter, InsertStrategy, TextInserter};

use crate::log_info;

/// 待恢复登记（进程级单例）。
static PENDING: Mutex<Option<PendingRestore>> = Mutex::new(None);
/// get/set 与 restore 互斥：避免 restore_1.set 插在 insert_2.get/set 之间。
static CLIPBOARD_MU: Mutex<()> = Mutex::new(());
static NEXT_RESTORE_ID: AtomicU64 = AtomicU64::new(0);

/// 恢复延迟（两端一致）。
pub const RESTORE_DELAY: std::time::Duration = std::time::Duration::from_millis(750);

/// 薄壳组合插入器：Type-then-Paste + PendingRestore 状态机。
pub struct CompositeInserter {
    enigo: EnigoInserter,
    app: AppHandle,
}

impl CompositeInserter {
    pub fn new(app: AppHandle) -> voice_core::Result<Self> {
        Ok(Self {
            enigo: EnigoInserter::new()?,
            app,
        })
    }
}

#[async_trait]
impl TextInserter for CompositeInserter {
    async fn insert(&self, text: &str) -> voice_core::Result<()> {
        // 流式 chunk / 其它旧路径：保持尽力打字（不碰剪贴板）。
        self.enigo.insert(text).await
    }

    /// 四态插入（R7）+ TSF 优先通道（R11，设计 L801）。
    async fn insert_ex(&self, text: &str, opts: &InsertOpts) -> InsertOutcome {
        if text.is_empty() {
            return InsertOutcome::Typed;
        }
        // R11：TSF CommitText 优先（目标进程内上屏，不抢焦点不碰剪贴板）。
        // 门控不过（未装/非 AMD64/超限/流式）→ 静默走原路径；失败按 tsf_fallback 回退。
        #[cfg(target_os = "windows")]
        if opts.tsf_enabled {
            if let Some(outcome) = self.try_tsf_insert(text, opts) {
                return outcome;
            }
        }
        // Auto + 前台 app 命中兜底列表 → 视同 Paste（FR-7.4）。
        let strategy = if opts.strategy == InsertStrategy::Auto
            && voice_core::matches_paste_fallback(
                opts.frontmost.as_deref(),
                &opts.paste_fallback_apps,
            ) {
            InsertStrategy::Paste
        } else {
            opts.strategy
        };
        match strategy {
            InsertStrategy::Auto => match self.enigo.insert(text).await {
                Ok(_) => {
                    log_info!("插入成功（Typed）：{} 字", text.chars().count());
                    InsertOutcome::Typed
                }
                Err(e) => {
                    log_info!("enigo 失败，回退粘贴：{e}");
                    self.paste(text, opts).await
                }
            },
            InsertStrategy::Type => match self.enigo.insert(text).await {
                Ok(_) => InsertOutcome::Typed,
                Err(e) => {
                    log_info!("Type 策略 enigo 失败：{e}");
                    InsertOutcome::Failed
                }
            },
            InsertStrategy::Paste => self.paste(text, opts).await,
        }
    }
}

#[cfg(target_os = "windows")]
impl CompositeInserter {
    /// TSF 通道（R11）：Some(Committed/Failed) = 终态；None = 不适用或失败可回退，
    /// 调用方继续走 enigo/粘贴（设计：Type 策略回退后仍只打字，由原 match 语义保证）。
    fn try_tsf_insert(&self, text: &str, opts: &InsertOpts) -> Option<InsertOutcome> {
        use crate::windows_ime::install::{detect_status, ImeInstallStatus};
        use crate::windows_ime::session::{prepare_session, tsf_gate};
        use voice_core::InsertOutcome;

        let info = crate::platform::windows::focus::frontmost_process_info()?;
        let installed = matches!(
            detect_status(Some(&self.app)),
            ImeInstallStatus::Installed { .. }
        );
        if tsf_gate(opts.tsf_enabled, text.len(), info.machine, installed).is_err() {
            return None; // 门控不过 → 原路径
        }
        let fallback = |err: &str| -> Option<InsertOutcome> {
            if opts.tsf_fallback {
                log_info!("TSF 失败（{err}），回退 R7 插入");
                None
            } else {
                log_info!("TSF 失败（{err}），tsf_fallback=false → Failed");
                Some(InsertOutcome::Failed)
            }
        };
        match prepare_session(Some(&self.app)) {
            Ok(mut s) => match s.submit(text) {
                Ok(status) => {
                    s.restore_session();
                    if matches!(
                        status,
                        crate::windows_ime::protocol::ImeSubmitStatus::Committed
                    ) {
                        log_info!("插入成功（TSF Committed）：{} 字", text.chars().count());
                        Some(InsertOutcome::Committed)
                    } else {
                        fallback(&format!("{status:?}"))
                    }
                }
                Err(e) => fallback(&format!("{e:?}")),
            },
            // prepare 失败路径内部已做 restore（含激活失败）。
            Err(e) => fallback(&format!("{e:?}")),
        }
    }
}

impl CompositeInserter {
    /// 粘贴：持锁读原剪贴板 → 写插入文字 → 平台和弦 → 登记恢复。
    async fn paste(&self, text: &str, opts: &InsertOpts) -> InsertOutcome {
        // 1) 剪贴板互斥区内读原值 + 写新值。
        let before = {
            let _g = match CLIPBOARD_MU.lock() {
                Ok(g) => g,
                Err(_) => return InsertOutcome::Failed,
            };
            let before = match clipboard_get_text(&self.app) {
                Ok(b) => b,
                Err(e) => {
                    log_info!("读剪贴板失败：{e}");
                    return InsertOutcome::Failed;
                }
            };
            if let Err(e) = clipboard_set_text(&self.app, text) {
                log_info!("写剪贴板失败：{e}");
                return InsertOutcome::Failed;
            }
            before
        };

        // 2) 平台粘贴和弦。
        if let Err(e) = simulate_paste() {
            // FR-7.8：文字留在剪贴板，不恢复；提示手动粘贴。
            log_info!("粘贴和弦失败（CopiedFallback）：{e}");
            return InsertOutcome::CopiedFallback;
        }

        // 3) 登记恢复（FR-7.5/7.7：Type 成功不碰 PENDING；这里只有 Paste 路径）。
        if opts.restore_clipboard {
            let id = NEXT_RESTORE_ID.fetch_add(1, Ordering::SeqCst) + 1;
            {
                let mut pending = match PENDING.lock() {
                    Ok(p) => p,
                    Err(_) => return InsertOutcome::Pasted,
                };
                let snap = remember_pending(pending.as_ref(), id, before.as_deref(), text);
                *pending = Some(snap);
            }
            spawn_restore(self.app.clone(), id);
        }
        log_info!("插入成功（Pasted）：{} 字", text.chars().count());
        InsertOutcome::Pasted
    }
}

/// 750ms 后执行恢复。线程 panic 不影响录音（NFR-7.2）。
fn spawn_restore(app: AppHandle, id: u64) {
    let _ = std::thread::Builder::new()
        .name("clipboard-restore".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                std::thread::sleep(RESTORE_DELAY);
                // 剪贴板读与决策在同一把锁下，避免与另一次 insert 交叠。
                let action = {
                    let _g = match CLIPBOARD_MU.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    let now = clipboard_get_text(&app).ok().flatten();
                    let pending = PENDING.lock().unwrap();
                    decide_restore(pending.as_ref(), id, now.as_deref())
                };
                match action {
                    RestoreDecision::Restore(original) => {
                        if let Err(e) = clipboard_set_text(&app, &original) {
                            log_info!("剪贴板恢复失败：{e}");
                        } else {
                            log_info!("剪贴板已恢复（原内容 {} 字）", original.chars().count());
                        }
                        clear_pending(id);
                    }
                    RestoreDecision::Clear => {
                        log_info!("剪贴板被用户改动，跳过恢复");
                        clear_pending(id);
                    }
                    RestoreDecision::DoNothing => {
                        // 已有更新的粘贴接管（clipboard_restore{skipped}）。
                        log_info!("恢复跳过：存在更新的粘贴");
                    }
                }
            }));
            if result.is_err() {
                log_info!("剪贴板恢复线程 panic（不影响录音）");
            }
        });
}

fn clear_pending(id: u64) {
    if let Ok(mut p) = PENDING.lock() {
        if p.as_ref().map(|s| s.id) == Some(id) {
            *p = None;
        }
    }
}

// ──────────────── 剪贴板（平台分发）────────────────

/// 读剪贴板文本。macOS 走主线程（NSPasteboard 历史要求）；Windows arboard 直接调用。
// `app` 仅 macOS 主线程调度用到；Windows 分支不用 → 允许未用形参告警。
#[allow(unused_variables)]
pub fn clipboard_get_text(app: &AppHandle) -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        run_on_main_blocking(app, || {
            arboard::Clipboard::new()
                .ok()
                .and_then(|mut c| c.get_text().ok())
        })
        .ok_or_else(|| "无法调度主线程读取剪贴板".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(arboard::Clipboard::new()
            .ok()
            .and_then(|mut c| c.get_text().ok()))
    }
}

/// 写剪贴板文本。macOS 走主线程；Windows arboard 直接调用（OpenClipboard 短重试）。
// `app`/`owned` 仅 macOS 主线程闭包用到；Windows 分支不用 → 允许未用变量告警。
#[allow(unused_variables)]
pub fn clipboard_set_text(app: &AppHandle, text: &str) -> Result<(), String> {
    let owned = text.to_string();
    #[cfg(target_os = "macos")]
    {
        run_on_main_blocking(app, move || {
            arboard::Clipboard::new()
                .ok()
                .and_then(|mut c| c.set_text(owned).ok())
        })
        .ok_or_else(|| "无法调度主线程写剪贴板".to_string())?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows OpenClipboard 被占用时短重试（1～2 次、间隔 ~20ms）。
        let mut last_err = None;
        for _ in 0..3 {
            match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e.to_string());
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| "写剪贴板失败".into()))
    }
}

/// 在主线程执行并同步取回结果（macOS 剪贴板用）。
#[cfg(target_os = "macos")]
fn run_on_main_blocking<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .is_err()
    {
        return None;
    }
    rx.recv_timeout(std::time::Duration::from_millis(1000)).ok()
}

// ──────────────── 平台粘贴和弦 ────────────────

/// 和弦规格（纯函数，单测断言平台差异：macOS=Cmd+V，Windows=Ctrl+V）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordModifier {
    #[allow(dead_code)] // 非 macOS 平台仅在 cfg(macos) 分支构造
    Cmd,
    #[allow(dead_code)] // 非 Windows 平台仅在 cfg(windows) 分支构造
    Ctrl,
}

#[allow(dead_code)] // 单测用（NFR-7.3）；生产走 simulate_paste
pub fn paste_chord_spec() -> (ChordModifier, char) {
    #[cfg(target_os = "macos")]
    {
        (ChordModifier::Cmd, 'v')
    }
    #[cfg(target_os = "windows")]
    {
        (ChordModifier::Ctrl, 'v')
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        (ChordModifier::Ctrl, 'v')
    }
}

/// 平台粘贴和弦。失败返回 Err → 薄壳走 CopiedFallback / Failed。
fn simulate_paste() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if crate::platform::current::fn_key::paste_cmd_v() {
            Ok(())
        } else {
            Err("macOS Cmd+V 发送失败".into())
        }
    }
    #[cfg(target_os = "windows")]
    {
        windows_ctrl_v()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Err("unsupported platform".into())
    }
}

/// Windows：enigo Key::Control + Key::V（Press/Click/Release 成对，失败反向释放防卡键）。
#[cfg(target_os = "windows")]
fn windows_ctrl_v() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| format!("enigo 初始化失败: {e}"))?;
    let down = |e: &mut Enigo| -> Result<(), String> {
        e.key(Key::Control, Direction::Press)
            .map_err(|e| e.to_string())?;
        e.key(Key::V, Direction::Click).map_err(|e| e.to_string())?;
        Ok(())
    };
    if let Err(e) = down(&mut enigo) {
        // 失败时反向释放，防止 Ctrl 卡住。
        let _ = enigo.key(Key::Control, Direction::Release);
        return Err(format!("Ctrl+V 发送失败: {e}"));
    }
    enigo
        .key(Key::Control, Direction::Release)
        .map_err(|e| format!("Ctrl 释放失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::{decide_restore, remember_pending, PendingRestore, RestoreDecision};

    #[test]
    fn paste_chord_is_cmd_v_on_macos() {
        // A7.7：单元断言平台和弦。
        #[cfg(target_os = "macos")]
        assert_eq!(paste_chord_spec(), (ChordModifier::Cmd, 'v'));
    }

    #[test]
    fn paste_chord_is_ctrl_v_on_windows() {
        #[cfg(target_os = "windows")]
        assert_eq!(paste_chord_spec(), (ChordModifier::Ctrl, 'v'));
    }

    #[test]
    fn paste_fallback_hits_mstsc() {
        // A7.7：paste_fallback_apps 对 mstsc.exe 命中 mstsc。
        assert!(voice_core::matches_paste_fallback(
            Some("mstsc.exe"),
            &["mstsc".into()]
        ));
        assert!(voice_core::matches_paste_fallback(
            Some("com.microsoft.rdc.macos"),
            &["rdc".into()]
        ));
        assert!(!voice_core::matches_paste_fallback(
            Some("com.apple.notes"),
            &["mstsc".into()]
        ));
    }

    #[test]
    fn restore_state_machine_chains_original() {
        // A7.4：连续两次 Paste，最终 original 是第一次之前的内容。
        let p1: PendingRestore = remember_pending(None, 1, Some("SECRET"), "HELLO");
        let p2 = remember_pending(Some(&p1), 2, Some("HELLO"), "WORLD");
        // 旧 id 到期 → 不动作（已有更新的粘贴）。
        assert_eq!(
            decide_restore(Some(&p2), 1, Some("HELLO")),
            RestoreDecision::DoNothing
        );
        // 新 id 到期且未变 → 写回第一次之前的内容。
        assert_eq!(
            decide_restore(Some(&p2), 2, Some("WORLD")),
            RestoreDecision::Restore("SECRET".into())
        );
    }

    #[test]
    fn restore_state_machine_skips_when_user_copied() {
        // A7.3：用户中途复制 → 不覆盖。
        let p = remember_pending(None, 1, Some("SECRET"), "HELLO");
        assert_eq!(
            decide_restore(Some(&p), 1, Some("OTHER")),
            RestoreDecision::Clear
        );
        assert_eq!(decide_restore(Some(&p), 1, None), RestoreDecision::Clear);
    }

    #[test]
    fn restore_delay_is_750ms() {
        assert_eq!(RESTORE_DELAY, std::time::Duration::from_millis(750));
    }

    #[test]
    fn pending_singleton_starts_empty() {
        assert!(PENDING.lock().unwrap().is_none());
    }
}
