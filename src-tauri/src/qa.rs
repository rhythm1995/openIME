//! R6：划词语音问答状态机（QA 面板）。
//!
//! - 开窗时抓选区 + 冻结 frontmost（之后不再覆盖）。
//! - 录音键在窗可见时改走 QA 录音（`intent=Qa`，不插入光标）。
//! - 问答走 `LlmClient::chat_stream`（SSE），多轮，关窗清空。
//! - `qa_save_history` 时写 sessions/utterances。
//!
//! 状态全局单例（进程内仅一个 QA 窗）；窗口显示/焦点由 lib.rs 操作。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::{AppHandle, Emitter, Manager};
use voice_core::polish::{build_qa_system, wrap_selected_text};
use voice_core::TextInserter;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use voice_core::ChatRequest;

use crate::log_info;
use crate::state::AppState;

/// QA 阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QaPhase {
    Hidden,
    Idle,
    Recording,
    Transcribing,
    Streaming,
}

/// 一轮消息。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct QaMessage {
    pub role: String,
    pub text: String,
}

/// QA 会话状态（仅 `open_qa_panel` 写 frontmost）。
struct QaSessionState {
    phase: QaPhase,
    panel_visible: bool,
    selection: Option<String>,
    frontmost: Option<String>,
    messages: Vec<QaMessage>,
    stream_cancel: Arc<AtomicBool>,
    session_gen: u64,
    /// 刷新选区后，下一轮问题重新携带选区信封（FR-6.7）。
    refresh_pending: bool,
    /// FR-6.11：qa_save_history 时面板每次打开建一条 sessions；每轮问答写两条。
    history_session_id: Option<String>,
    history_seq: u32,
}

static QA: OnceLock<Mutex<QaSessionState>> = OnceLock::new();

fn state() -> &'static Mutex<QaSessionState> {
    QA.get_or_init(|| {
        Mutex::new(QaSessionState {
            phase: QaPhase::Hidden,
            panel_visible: false,
            selection: None,
            frontmost: None,
            messages: Vec::new(),
            stream_cancel: Arc::new(AtomicBool::new(false)),
            session_gen: 0,
            refresh_pending: false,
            history_session_id: None,
            history_seq: 0,
        })
    })
}

// ──────────────── 查询 ────────────────

pub fn panel_visible() -> bool {
    state().lock().map(|s| s.panel_visible).unwrap_or(false)
}

pub fn phase() -> QaPhase {
    state().lock().map(|s| s.phase).unwrap_or(QaPhase::Hidden)
}

/// 开窗时冻结的前台标识（插入按钮还焦用；只读）。
#[allow(dead_code)]
pub fn frozen_frontmost() -> Option<String> {
    state().lock().ok().and_then(|s| s.frontmost.clone())
}

/// 读前台选区（开窗前；macOS AX 直读，其它平台 None）。
pub fn read_selection() -> Option<String> {
    crate::platform::current::fn_key::get_selection()
}

/// 选区截断 + 信封（同一截断结果，FR-6.2）。
pub fn envelope_for(selection: &str) -> String {
    wrap_selected_text(&voice_core::polish::truncate_selection(selection))
}

// ──────────────── 窗口生命周期 ────────────────

/// 打开 QA 面板（显示**前**抓选区 + frontmost）。幂等：已可见则只聚焦。
pub fn open_qa_panel(app: &AppHandle) {
    let selection = read_selection();
    {
        let mut s = match state().lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        if s.panel_visible {
            // 已可见：仅重新聚焦，不重置会话。
            drop(s);
            crate::show_qa_window(app);
            return;
        }
        s.selection = selection.clone();
        s.frontmost = crate::platform::current::fn_key::frontmost_bundle_id();
        s.phase = QaPhase::Idle;
        s.panel_visible = true;
        s.messages.clear();
        s.refresh_pending = true;
        s.stream_cancel.store(false, Ordering::SeqCst);
        log_info!("QA 开窗：selection={:?} frontmost={:?}", s.selection.is_some(), s.frontmost);
    }
    crate::show_qa_window(app);
    // FR-6.11：qa_save_history → 面板每次打开建一条 sessions（engine=qa）。
    let save_history = app
        .state::<AppState>()
        .config
        .blocking_read()
        .qa_save_history;
    if save_history {
        let app_for_hist = app.clone();
        let store = app.state::<AppState>().store.clone();
        tauri::async_runtime::spawn(async move {
            use voice_core::traits::HistoryStore;
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now();
            let created = store
                .create_session(&voice_core::SessionSummary {
                    id: id.clone(),
                    title: "QA 会话".into(),
                    started_at: now,
                    ended_at: None,
                    engine: "qa".into(),
                    provider: "cloud".into(),
                    model: "qa".into(),
                })
                .await;
            if created.is_ok() {
                if let Ok(mut s) = state().lock() {
                    s.history_session_id = Some(id);
                    s.history_seq = 0;
                }
            }
            let _ = app_for_hist; // 保持句柄存活
        });
    }
    emit_state(app, "open");
}

/// 关闭 QA 面板：清空消息、取消流、隐藏窗口；main 不可见则回 Accessory。
pub fn close_qa_panel(app: &AppHandle) {
    {
        let mut s = match state().lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        s.messages.clear();
        s.phase = QaPhase::Hidden;
        s.panel_visible = false;
        s.stream_cancel.store(true, Ordering::SeqCst);
        s.session_gen += 1;
        s.history_session_id = None;
        s.history_seq = 0;
        log_info!("QA 关窗：清空会话");
    }
    if let Some(win) = app.get_webview_window("qa") {
        let _ = win.hide();
    }
    // 若 main 不可见则恢复 Accessory（菜单栏常驻形态）。
    let main_visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if !main_visible {
        // macOS：回到菜单栏常驻形态（Accessory）；Windows 无此概念，跳过。
        #[cfg(target_os = "macos")]
        {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
    }
    emit_state(app, "close");
}

/// 取消进行中的流（ESC / 再按录音键 / QA 键）：bump gen，已输出保留（FR-6.10）。
pub fn cancel_stream(app: &AppHandle) {
    let mut s = match state().lock() {
        Ok(s) => s,
        Err(_) => return,
    };
    s.stream_cancel.store(true, Ordering::SeqCst);
    s.session_gen += 1;
    log_info!("QA 取消：gen={}", s.session_gen);
    emit_state(app, "cancel");
}

/// 刷新选区（不重置对话；下一轮首条 user 消息用新选区）。
pub fn refresh_selection(app: &AppHandle) -> Option<String> {
    let selection = read_selection();
    if let Ok(mut s) = state().lock() {
        s.selection = selection.clone();
        s.refresh_pending = true;
    }
    emit_state(app, "refresh");
    selection
}

/// 清空当前对话（保持窗口打开；与关窗的 close_qa_panel 不同）。
pub fn clear_messages(app: &AppHandle) {
    if let Ok(mut s) = state().lock() {
        s.messages.clear();
        s.phase = QaPhase::Idle;
    }
    emit_state(app, "clear");
}

/// 复制最后一条回答到剪贴板。
pub fn copy_last_answer(app: &AppHandle) -> Result<Option<String>, String> {
    let answer = state()
        .lock()
        .ok()
        .and_then(|s| {
            s.messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .map(|m| m.text.clone())
        });
    if let Some(text) = &answer {
        crate::insert_fallback::clipboard_set_text(app, text)?;
    }
    Ok(answer)
}

/// 把最后一条回答插入到开窗时的前台 app（R7 四态）。
pub async fn insert_last_answer(app: &AppHandle) -> Result<Option<voice_core::InsertOutcome>, String> {
    let (answer, frontmost) = {
        let s = state().lock().map_err(|_| "QA 状态不可用".to_string())?;
        let answer = s
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.text.clone());
        (answer, s.frontmost.clone())
    };
    let Some(text) = answer else {
        return Ok(None);
    };
    let app_state = app.state::<AppState>();
    let inserter = app_state.composite_inserter().map_err(|e| e.to_string())?;
    // async 上下文必须用 read().await：blocking_read 在 tokio worker 上会 panic。
    let cfg = app_state.config.read().await.clone();
    // R11：QA 插入光标也走唯一业务构造（非流式）。
    let opts = voice_core::InsertOpts::from_config(&cfg, frontmost.clone(), false);
    // 先还焦到开窗时的前台（QA 窗此刻在前台，直接插会进 webview）。
    crate::commands::restore_frontmost(app, frontmost.as_deref());
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    Ok(Some(inserter.insert_ex(&text, &opts).await))
}

// ──────────────── 问答流 ────────────────

/// 录音结束 → 转写完成，准备发流。
pub fn begin_streaming() {
    if let Ok(mut s) = state().lock() {
        s.phase = QaPhase::Transcribing;
    }
}

/// 把问题发给云端 LLM 并流式回传。问题为空 → 发错误事件。
pub async fn ask_and_stream(app: &AppHandle, question: &str) {
    let question = question.trim().to_string();
    if question.is_empty() {
        let _ = app.emit("qa://error", "未识别到问题，请重试");
        if let Ok(mut s) = state().lock() {
            s.phase = QaPhase::Idle;
        }
        emit_state(app, "empty");
        return;
    }

    // 组装消息：system + 历史（截断）+ 本轮问题（首轮带选区信封）。
    let (messages, gen, cancel) = {
        let mut s = match state().lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        s.phase = QaPhase::Streaming;
        s.stream_cancel.store(false, Ordering::SeqCst);
        let first_round = s.messages.is_empty();
        let use_envelope = first_round || s.refresh_pending;
        s.refresh_pending = false;
        let envelope = if use_envelope {
            s.selection.as_deref().map(envelope_for)
        } else {
            None
        };
        s.messages.push(QaMessage {
            role: "user".into(),
            text: question.clone(),
        });
        let trimmed = trim_messages(s.messages.clone());
        let mut msgs: Vec<(String, String)> =
            vec![("system".into(), build_qa_system())];
        for m in trimmed {
            msgs.push((m.role, m.text));
        }
        let user_msg = if let Some(env) = envelope {
            format!("{env}\n我的问题：{question}")
        } else {
            question.clone()
        };
        // 用信封版替换最后一条 user 消息。
        if let Some(last) = msgs.last_mut() {
            if last.0 == "user" {
                last.1 = user_msg;
            }
        }
        s.session_gen += 1;
        let gen = s.session_gen;
        (msgs, gen, s.stream_cancel.clone())
    };
    let save_history = app
        .state::<AppState>()
        .config
        .read()
        .await
        .qa_save_history;

    let cloud = match app.state::<AppState>().cloud_llm().await {
        Some(c) => c,
        None => {
            let _ = app.emit("qa://error", "请先配置云端 LLM（润色 endpoint + key）");
            if let Ok(mut s) = state().lock() {
                s.phase = QaPhase::Idle;
            }
            return;
        }
    };

    // ESC 动态注册：流式中按 ESC 走 on_hotkey → qa::cancel_stream（结束后注销）。
    let esc = tauri_plugin_global_shortcut::Shortcut::new(
        None,
        tauri_plugin_global_shortcut::Code::Escape,
    );
    let _ = app
        .global_shortcut()
        .register(esc);
    let app_for_unreg = app.clone();

    // 累积流式输出（取消时保留已输出内容）。
    let acc: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let acc_cb = acc.clone();
    let app_cb = app.clone();
    let on_delta = Box::new(move |delta: &str| {
        acc_cb.lock().unwrap().push_str(delta);
        let _ = app_cb.emit("qa://delta", serde_json::json!({ "gen": gen, "delta": delta }));
    });

    let req = ChatRequest {
        messages,
        timeout: std::time::Duration::from_secs(60),
        max_tokens: 2048,
        cancel,
        gen,
        on_delta,
    };

    let result = cloud.chat_stream(req).await;
    let _ = app_for_unreg.global_shortcut().unregister(esc);
    let partial = acc.lock().unwrap().clone();
    let answer = match result {
        Ok(full) if !full.trim().is_empty() => full,
        Ok(_) if !partial.trim().is_empty() => partial,
        Ok(_) => {
            let _ = app.emit("qa://error", "模型没有返回内容");
            if let Ok(mut s) = state().lock() {
                s.phase = QaPhase::Idle;
            }
            emit_state(app, "error");
            return;
        }
        Err(e) if e.to_string().contains("已取消") => {
            // 取消：保留已输出（partial）。
            partial
        }
        Err(e) => {
            let _ = app.emit("qa://error", format!("问答失败：{e}"));
            if let Ok(mut s) = state().lock() {
                s.phase = QaPhase::Idle;
            }
            emit_state(app, "error");
            return;
        }
    };

    // 落库（可选）+ 消息入列。
    {
        let mut s = match state().lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        s.phase = QaPhase::Idle;
        s.messages.push(QaMessage {
            role: "assistant".into(),
            text: answer.clone(),
        });
        s.messages = trim_messages(s.messages.clone());
    }
    if save_history {
        if let Err(e) = save_qa_history(app, &question, &answer).await {
            log_info!("QA 历史落库失败：{e}");
        }
    }
    emit_state(app, "answer");
}

/// 发送前截断：只保留最近 8 轮（16 条），或累计字符 ≤ 8000（先到为准，FR-6.6）。
pub fn trim_messages(messages: Vec<QaMessage>) -> Vec<QaMessage> {
    const MAX_ROUNDS: usize = 8;
    const MAX_CHARS: usize = 8000;
    let mut kept: Vec<QaMessage> = Vec::new();
    let mut chars = 0usize;
    for m in messages.into_iter().rev() {
        if kept.len() >= MAX_ROUNDS * 2 {
            break;
        }
        if chars + m.text.chars().count() > MAX_CHARS && !kept.is_empty() {
            break;
        }
        chars += m.text.chars().count();
        kept.push(m);
    }
    kept.reverse();
    kept
}

/// qa_save_history 时写库：复用开窗时建的 sessions（engine=qa），
/// 每轮问答写两条 utterances（Q: … / A: …），seq 递增（FR-6.11）。
async fn save_qa_history(app: &AppHandle, question: &str, answer: &str) -> Result<(), String> {
    let app_state = app.state::<AppState>();
    let store = app_state.store.clone();
    let (session_id, seq) = {
        let mut s = state().lock().map_err(|_| "QA 状态不可用".to_string())?;
        let id = match &s.history_session_id {
            Some(id) => id.clone(),
            None => {
                // 兜底：开窗时的异步建会话还没完成。
                let id = uuid::Uuid::new_v4().to_string();
                s.history_session_id = Some(id.clone());
                id
            }
        };
        let seq = s.history_seq;
        s.history_seq += 2;
        (id, seq)
    };
    use voice_core::traits::HistoryStore;
    for (i, text) in [format!("Q: {question}"), format!("A: {answer}")].iter().enumerate() {
        store
            .save_utterance(&voice_core::UtteranceRecord {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.clone(),
                seq: seq + i as u32,
                final_text: text.clone(),
                audio_path: None,
                created_at: chrono::Utc::now(),
            })
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ──────────────── 事件 ────────────────

/// 推送 qa://state（phase / panel_visible / selection 预览 / messages）。
fn emit_state(app: &AppHandle, action: &str) {
    let payload = {
        let s = match state().lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let selection_preview = s.selection.as_deref().map(|sel| {
            let sel = voice_core::polish::truncate_selection(sel);
            let preview: String = sel.chars().take(80).collect();
            if sel.chars().count() > 80 {
                format!("{preview}…")
            } else {
                preview
            }
        });
        serde_json::json!({
            "action": action,
            "phase": phase_str(s.phase),
            "panel_visible": s.panel_visible,
            "selection": selection_preview,
            "messages": s.messages,
            // FR-6.9：无云端 key 时面板显示横幅。
            "has_cloud_key": app.state::<AppState>().has_cloud_key(),
        })
    };
    let _ = app.emit("qa://state", payload);
}

fn phase_str(p: QaPhase) -> &'static str {
    match p {
        QaPhase::Hidden => "hidden",
        QaPhase::Idle => "idle",
        QaPhase::Recording => "recording",
        QaPhase::Transcribing => "transcribing",
        QaPhase::Streaming => "streaming",
    }
}

/// QA 录音开始/结束标记（HUD 文案 + 面板徽章用）。
pub fn mark_recording(app: &AppHandle, started: bool) {
    if let Ok(mut s) = state().lock() {
        s.phase = if started {
            QaPhase::Recording
        } else {
            QaPhase::Idle
        };
    }
    emit_state(app, if started { "recording" } else { "recording-stopped" });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_recent_rounds() {
        let mut msgs = Vec::new();
        for i in 0..10 {
            msgs.push(QaMessage { role: "user".into(), text: format!("Q{i}") });
            msgs.push(QaMessage { role: "assistant".into(), text: format!("A{i}") });
        }
        let out = trim_messages(msgs);
        // 10 轮 → 保留最近 8 轮（16 条）。
        assert_eq!(out.len(), 16);
        assert_eq!(out[0].text, "Q2");
        assert_eq!(out[15].text, "A9");
    }

    #[test]
    fn trim_respects_char_budget() {
        // 8000 字上限：先到为准。
        let mut msgs = Vec::new();
        for _ in 0..4 {
            msgs.push(QaMessage { role: "user".into(), text: "x".repeat(3000) });
            msgs.push(QaMessage { role: "assistant".into(), text: "y".repeat(3000) });
        }
        let out = trim_messages(msgs);
        let chars: usize = out.iter().map(|m| m.text.chars().count()).sum();
        assert!(chars <= 8000 + 3000, "至少保留最后一轮完整问答，得到 {chars}");
        assert!(out.iter().any(|m| m.text.contains("xxx")));
    }

    #[test]
    fn second_round_keeps_first_round() {
        // A6.2：第二轮 messages 含第一轮。
        let msgs = vec![
            QaMessage { role: "user".into(), text: "第一问".into() },
            QaMessage { role: "assistant".into(), text: "第一答".into() },
            QaMessage { role: "user".into(), text: "第二问".into() },
        ];
        let out = trim_messages(msgs);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].text, "第一问");
        assert_eq!(out[2].text, "第二问");
    }

    #[test]
    fn envelope_wraps_and_truncates_same_result() {
        // FR-6.2：信封与截断用同一结果（truncate 一次）。
        let long: String = "a".repeat(5000);
        let env = envelope_for(&long);
        assert!(env.contains("<selected_text>"));
        assert!(env.contains("中间内容已省略"));
    }
}
