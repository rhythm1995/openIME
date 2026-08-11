//! 暴露给前端的 Tauri 命令。
//! - 健康检查 / 默认配置
//! - 配置读写（持久化到 settings 表）
//! - 历史记录：创建会话 / 保存录音 / 列会话 / 列录音 / 删会话
//! - 权限探测（辅助功能 / 麦克风）
//! - 录音控制：toggle_recording / get_recording_state

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};
use voice_core::audio::CpalAudioSource;
use voice_core::permissions::{PermissionChecker, PermissionKind, PermissionStatus};
use voice_core::pipeline::SessionMeta;
use voice_core::traits::HistoryStore;
use voice_core::{AppConfig, Hotword, ProviderConfig, SessionSummary, UtteranceRecord};

use crate::platform::current::permissions::MacPermissionChecker;
use crate::state::{save_config, AppState};
use crate::{log_debug, log_error, log_info, log_warn};

#[tauri::command]
pub fn ping() -> String {
    "openIME voice-core 在线".into()
}

// ──────────────── 开机自启 ────────────────
// 注册插件时已配置 args=["--autostart"]（见 lib.rs），开机自启时 LaunchAgent
// 会以该参数启动应用，setup 据此保持窗口隐藏。

#[tauri::command]
pub fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable().map_err(|e| e.to_string())?;
    } else {
        mgr.disable().map_err(|e| e.to_string())?;
    }
    log_info!("开机自启已{}", if enabled { "开启" } else { "关闭" });
    Ok(())
}

#[tauri::command]
pub fn get_launch_at_login(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// 前端日志转发：JS 侧的 console/error 统一落到后端日志文件。
#[tauri::command]
pub fn frontend_log(level: String, message: String) {
    let level = match level.to_ascii_uppercase().as_str() {
        "DEBUG" => "DEBUG",
        "WARN" | "WARNING" => "WARN",
        "ERROR" => "ERROR",
        _ => "INFO",
    };
    crate::logging::write(level, &format!("[frontend] {message}"));
}

#[tauri::command]
pub fn default_config() -> AppConfig {
    AppConfig::default()
}

// ──────────────── 配置 ────────────────

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> AppConfig {
    state.config.blocking_read().clone()
}

#[tauri::command]
pub async fn save_app_config(
    app: AppHandle,
    state: State<'_, AppState>,
    mut config: AppConfig,
) -> Result<(), String> {
    config.active().map_err(|e| e.to_string())?;
    // 规范化 local_asr_model，同步 local_mode 与 sherpa provider.model。
    config.sync_local_asr_fields();
    let hotkey_changed = state.config.read().await.hotkey != config.hotkey;
    save_config(&state.store, &config).map_err(|e| e.to_string())?;
    let new_hotkey = config.hotkey.clone();
    *state.config.write().await = config;
    // 润色/引擎等变更：丢弃 pipeline，下次录音按新配置重建。
    state.invalidate_pipeline().await;
    // 快捷键变化立即生效（Fn 走原生监听，组合键走 global-shortcut）。
    if hotkey_changed {
        crate::apply_hotkey(&app, &new_hotkey);
    }
    Ok(())
}

#[tauri::command]
pub fn validate_provider(provider: ProviderConfig) -> Result<(), String> {
    provider.validate().map_err(|e| e.to_string())
}

/// 测试云端引擎连接：建立 WS → 发 run-task → 等 task-started。
/// 成功返回确认消息，失败返回具体错误。
#[tauri::command]
pub async fn test_cloud_connection(provider: ProviderConfig) -> Result<String, String> {
    log_info!(
        "测试云端连接：model={} url={}",
        provider.model,
        provider.base_url
    );
    voice_core::test_connection(&provider).await.map_err(|e| {
        log_error!("云端连接测试失败：{e}");
        e.to_string()
    })
}

// ──────────────── 历史 ────────────────

#[tauri::command]
pub async fn create_session(
    state: State<'_, AppState>,
    session: SessionSummary,
) -> Result<(), String> {
    state
        .store
        .create_session(&session)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_utterance(
    state: State<'_, AppState>,
    utterance: UtteranceRecord,
) -> Result<(), String> {
    state
        .store
        .save_utterance(&utterance)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    state.store.list_sessions().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_utterances(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<UtteranceRecord>, String> {
    state
        .store
        .list_utterances(&session_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    state
        .store
        .delete_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

// ──────────────── 本地模型 ────────────────

/// 本地引擎安装状态。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalModelStatus {
    pub installed: bool,
    pub downloading: bool,
    pub total_files: usize,
    pub missing_files: Vec<String>,
    pub missing_size: u64,
    pub total_size: u64,
    pub model_root: String,
    /// 查询所用的模型 id（规范化后）。
    pub model_id: String,
}

/// 设置页 ASR 候选卡片。
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalAsrModelEntry {
    pub id: String,
    pub title: String,
    pub description: String,
    pub backend: String,
    pub recommended: bool,
    pub approx_size: u64,
    pub installed: bool,
    pub active: bool,
    pub missing_size: u64,
}

#[tauri::command]
pub fn list_local_asr_models(
    state: State<'_, AppState>,
) -> Result<Vec<LocalAsrModelEntry>, String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let active = state.config.blocking_read().resolved_local_asr_model();
    let entries = voice_core::asr_model_catalog()
        .iter()
        .map(|m| {
            let missing = voice_core::missing_files_for(&model_root, m.id);
            let missing_size: u64 = missing.iter().map(|f| f.size).sum();
            LocalAsrModelEntry {
                id: m.id.to_string(),
                title: m.title.to_string(),
                description: m.description.to_string(),
                backend: match m.backend {
                    voice_core::AsrBackend::OfflineSenseVoice => "offline_sense_voice".into(),
                    voice_core::AsrBackend::OfflineFireRed => "offline_fire_red".into(),
                    voice_core::AsrBackend::StreamingZipformer => "streaming_zipformer".into(),
                },
                recommended: m.recommended,
                approx_size: m.approx_size,
                installed: missing.is_empty(),
                // 未安装不可算「使用中」
                active: missing.is_empty() && m.id == active,
                missing_size,
            }
        })
        .collect();
    Ok(entries)
}

#[tauri::command]
pub fn get_local_model_status(
    state: State<'_, AppState>,
    mode: Option<String>,
) -> Result<LocalModelStatus, String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let mode = mode.unwrap_or_else(|| state.config.blocking_read().resolved_local_asr_model());
    let model_id = voice_core::normalize_asr_model_id(&mode).to_string();
    let files = voice_core::local_model_files_for(&model_id);
    let missing = voice_core::missing_files_for(&model_root, &model_id);
    let total_size: u64 = files.iter().map(|f| f.size).sum();
    let missing_size: u64 = missing.iter().map(|f| f.size).sum();
    Ok(LocalModelStatus {
        installed: missing.is_empty(),
        downloading: state
            .model_downloading
            .load(std::sync::atomic::Ordering::SeqCst),
        total_files: files.len(),
        missing_files: missing.iter().map(|f| f.file_name.to_string()).collect(),
        missing_size,
        total_size,
        model_root: model_root.display().to_string(),
        model_id,
    })
}

/// 本地润色 GGUF 安装状态。
#[derive(serde::Serialize)]
pub struct PolishModelStatus {
    pub installed: bool,
    pub downloading: bool,
    pub model_id: String,
    pub file_name: String,
    pub total_size: u64,
    pub model_path: String,
    pub llm_feature: bool,
}

#[tauri::command]
pub fn get_polish_model_status(state: State<'_, AppState>) -> Result<PolishModelStatus, String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let files = voice_core::model_download::polish_model_files();
    let total_size: u64 = files.iter().map(|f| f.size).sum();
    let path = voice_core::polish_model_path(&model_root);
    Ok(PolishModelStatus {
        installed: voice_core::is_polish_model_installed(&model_root),
        downloading: state
            .model_downloading
            .load(std::sync::atomic::Ordering::SeqCst),
        model_id: voice_core::POLISH_MODEL_ID.to_string(),
        file_name: voice_core::POLISH_GGUF_FILE.to_string(),
        total_size,
        model_path: path.display().to_string(),
        llm_feature: cfg!(feature = "llm"),
    })
}

/// 下载安装本地润色 GGUF（进度复用 model://download-progress）。
#[tauri::command]
pub async fn install_polish_model(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    if state
        .model_downloading
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return Err("模型正在下载中".to_string());
    }
    if voice_core::is_polish_model_installed(&model_root) {
        state
            .model_downloading
            .store(false, std::sync::atomic::Ordering::SeqCst);
        return Ok(());
    }

    let flag = state.model_downloading.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let app_for_cb = app_for_task.clone();
        let result = voice_core::install_polish_model(&model_root, &move |p| {
            let _ = app_for_cb.emit("model://download-progress", &p);
        })
        .await;
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        match result {
            Ok(()) => {
                log_info!("本地润色模型安装完成");
                let _ = app_for_task.emit("model://download-complete", "polish");
            }
            Err(e) => {
                log_error!("本地润色模型安装失败：{e}");
                let _ = app_for_task.emit("model://download-error", e.to_string());
            }
        }
    });
    Ok(())
}

#[tauri::command]
pub fn list_personas(state: State<'_, AppState>) -> Result<Vec<voice_core::Persona>, String> {
    state.store.list_personas().map_err(|e| e.to_string())
}

/// 下载安装本地引擎模型（后台进行，进度经 model://download-progress 事件推送）。
/// `mode` 为 ASR 模型 id（`zipformer-zh-2025` / `sensevoice`）或兼容旧值 offline/realtime。
#[tauri::command]
pub async fn install_local_model(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: Option<String>,
) -> Result<(), String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let mode = mode.unwrap_or_else(|| state.config.blocking_read().resolved_local_asr_model());
    let mode = voice_core::normalize_asr_model_id(&mode).to_string();

    // 防并发：已有下载在途则拒绝。
    if state
        .model_downloading
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return Err("模型正在下载中".to_string());
    }
    if voice_core::is_local_engine_installed_for(&model_root, &mode) {
        state
            .model_downloading
            .store(false, std::sync::atomic::Ordering::SeqCst);
        log_info!("本地模型（{}）已安装，无需下载", mode);
        return Ok(());
    }

    let flag = state.model_downloading.clone();
    let app_for_task = app.clone();
    let mode_for_task = mode.clone();
    tauri::async_runtime::spawn(async move {
        let app_for_cb = app_for_task.clone();
        let mode_for_emit = mode_for_task.clone();
        let result = voice_core::install_local_engine(&model_root, &mode_for_task, &move |p| {
            let _ = app_for_cb.emit("model://download-progress", &p);
        })
        .await;
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        match result {
            Ok(()) => {
                log_info!("本地模型安装完成：{mode_for_emit}");
                let _ = app_for_task.emit("model://download-complete", mode_for_emit);
            }
            Err(e) => {
                log_error!("本地模型安装失败：{e}");
                let _ = app_for_task.emit("model://download-error", e.to_string());
            }
        }
    });
    log_info!("本地模型下载任务已启动：{mode}");
    Ok(())
}

// ──────────────── 权限 ────────────────

#[tauri::command]
pub fn check_permission(kind: PermissionKind) -> PermissionStatus {
    let status = MacPermissionChecker.check(kind);
    log_debug!("check_permission({kind:?}) -> {:?}", status.state);
    status
}

/// 请求辅助功能授权：
/// 1) prompt=true 触发系统弹窗（仅首次有效）；
/// 2) 若仍未授信（已拒绝过、或重装后旧条目失效），直接深链打开系统设置对应面板。
#[tauri::command]
pub fn request_accessibility() -> bool {
    let trusted = crate::platform::current::permissions::is_trusted(true);
    log_info!("request_accessibility: is_trusted={trusted}");
    if !trusted {
        if let Err(e) =
            crate::platform::current::permissions::open_settings_pane("Privacy_Accessibility")
        {
            log_warn!("打开辅助功能设置面板失败：{e}");
        }
    }
    trusted
}

/// 请求麦克风授权：触发系统弹窗（首次）并等待用户选择。
///
/// 关键：请求必须在主线程发起——TCC 依赖运行循环弹出授权框，
/// 在后台线程发起会不弹窗且立即被拒。等待结果放在异步轮询里，不卡 UI。
#[tauri::command]
pub async fn request_microphone(app: AppHandle) -> bool {
    use crate::platform::current::permissions as perm;

    // 1) 结果已知（已授权/已拒绝）直接返回。
    if let Some(known) = perm::microphone_preflight() {
        log_info!("request_microphone: 状态已知 granted={known}");
        return known;
    }

    // 2) 主线程发起请求。
    let (tx, rx) = std::sync::mpsc::channel();
    let scheduled = app
        .run_on_main_thread(move || {
            let issued = perm::issue_microphone_request();
            let _ = tx.send(issued);
        })
        .is_ok();
    let issued = scheduled && rx.recv().unwrap_or(false);
    if !issued {
        log_warn!("request_microphone: 未能发起请求（重复点击或 FFI 失败）");
        return perm::microphone_preflight().unwrap_or(false);
    }

    // 3) 非主线程等待系统弹窗回调（弹窗由 tccd 管理，用户交互不受影响）。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if perm::microphone_request_finished() {
            let granted = perm::microphone_request_granted();
            perm::clear_microphone_request();
            log_info!("request_microphone: 弹窗回调 granted={granted}");
            return granted;
        }
        if std::time::Instant::now() > deadline {
            perm::clear_microphone_request();
            log_warn!("request_microphone: 等待授权弹窗超时（60s）");
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// 深链打开系统设置对应隐私面板（前端"打开系统设置"按钮用）。
#[tauri::command]
pub fn open_permission_settings(kind: PermissionKind) -> Result<(), String> {
    let pane = match kind {
        PermissionKind::Accessibility => "Privacy_Accessibility",
        PermissionKind::Microphone => "Privacy_Microphone",
    };
    log_info!("open_permission_settings({kind:?})");
    crate::platform::current::permissions::open_settings_pane(pane)
}

// ──────────────── 录音 ────────────────

/// 在**主线程**执行 AppKit 操作（orderOut / activate 等），禁止在 tokio worker 上调。
fn run_on_main_sync(app: &AppHandle, f: impl FnOnce() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);
    if app
        .run_on_main_thread(move || {
            f();
            let _ = tx.send(());
        })
        .is_err()
    {
        log_warn!("无法调度主线程任务");
        return;
    }
    if rx
        .recv_timeout(std::time::Duration::from_millis(500))
        .is_err()
    {
        log_warn!("主线程任务超时");
    }
}

/// 仅恢复前台 app（不隐藏 overlay）。插入文字前用，HUD 保持可见。
fn restore_frontmost_focus(app: &AppHandle, frontmost: Option<&str>) {
    let Some(bid) = frontmost else { return };
    let bid = bid.to_string();
    let app2 = app.clone();
    run_on_main_sync(app, move || {
        if bid == "com.openime.desktop" {
            if let Some(main) = app2.get_webview_window("main") {
                let _ = main.set_focus();
            }
        } else {
            let ok = crate::platform::current::fn_key::activate_app(&bid);
            log_info!("恢复前台 app {}：{}", bid, ok);
        }
    });
}

/// 仅隐藏 overlay（orderOut）。文字已上屏后再调用。
fn hide_overlay_only(app: &AppHandle) {
    let app2 = app.clone();
    run_on_main_sync(app, move || {
        if let Some(win) = app2.get_webview_window("overlay") {
            #[cfg(target_os = "macos")]
            {
                match win.ns_window() {
                    Ok(ns) => {
                        crate::platform::current::fn_key::hide_window_without_activating(ns);
                    }
                    Err(_) => {
                        let _ = win.hide();
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = win.hide();
            }
        }
    });
}

/// 切换录音：未录音→开始，录音中→停止。
/// partial 增量通过事件 `recording://partial` 推给前端；结束推 `recording://stopped`。
///
/// `frontmost`：录音前的前台 app bundle ID（由 trigger_toggle 在 overlay 显示前捕获）。
/// overlay 显示会抢走焦点，录音结束后需先激活回原 app 再插入文本，否则 enigo 输入到错误窗口。
#[tauri::command]
pub async fn toggle_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    frontmost: Option<String>,
) -> Result<bool, String> {
    // 原子 guard：false→true 抢占启动权。CAS 失败说明已有 pipeline 在跑 → 请求停止。
    // 这避免了「读 recording=false → 之后才写 true」的窗口期，两次 trigger_toggle
    // 在窗口内并发各启一个 pipeline，导致同一句话被插两遍。
    let acquired = state.recording_guard.compare_exchange(
        false,
        true,
        std::sync::atomic::Ordering::SeqCst,
        std::sync::atomic::Ordering::SeqCst,
    );
    if acquired.is_err() {
        // 已在录音中 → 请求停止。
        state.request_stop();
        return Ok(false);
    }

    // 读当前 provider 配置。
    let cfg = state.config.read().await.clone();
    let mut provider_cfg = cfg.active().map_err(|e| {
        release_recording_guard(&state);
        e.to_string()
    })?.clone();

    // 本地引擎：注入当前启用的 ASR 模型 id；未安装则回退到任一已装候选。
    if provider_cfg.kind == voice_core::ProviderKind::Sherpa {
        let mut model_id = cfg.resolved_local_asr_model();
        if let Some(root) = state.model_root() {
            if !voice_core::is_local_engine_installed_for(&root, &model_id) {
                let fallback = voice_core::asr_model_catalog().iter().find(|m| {
                    voice_core::is_local_engine_installed_for(&root, m.id)
                });
                if let Some(m) = fallback {
                    log_warn!(
                        "配置的 ASR「{}」未安装，回退到已安装的「{}」",
                        model_id,
                        m.id
                    );
                    model_id = m.id.to_string();
                } else {
                    release_recording_guard(&state);
                    return Err(format!(
                        "本地 ASR 模型「{model_id}」尚未下载，请到设置中下载并启用"
                    ));
                }
            }
        }
        provider_cfg.model = model_id;
    }
    provider_cfg.validate().map_err(|e| {
        release_recording_guard(&state);
        e.to_string()
    })?;

    // 懒初始化 pipeline（含 enigo，可能在无辅助功能权限时失败）。
    let pipeline = state.pipeline().await.map_err(|e| {
        release_recording_guard(&state);
        e.to_string()
    })?;

    // 建立音频源（优先用户选定的麦克风，否则系统默认）。
    let audio: Box<dyn voice_core::AudioSource> = Box::new(
        CpalAudioSource::new_with_device(cfg.audio_device.clone())
            .map_err(|e| {
                release_recording_guard(&state);
                e.to_string()
            })?,
    );

    state.clear_stop();
    *state.recording.write().await = true;
    // 通知 overlay：录音已开始（避免挂载时 race 读到 false 显示空白）。
    let _ = app.emit("recording://started", ());

    // frontmost 由 trigger_toggle 在 overlay 显示前捕获并传入。
    log_info!("录音前前台 app：{:?}", frontmost);

    let recording = state.recording.clone();
    let guard = state.recording_guard.clone();
    let stop_flag = state.stop_flag.clone();
    let polish_ctx = state.polish_context().await;
    let app_handle = app.clone();
    let meta = SessionMeta {
        engine: "cloud".into(),
        provider: format!("{:?}", provider_cfg.kind).to_lowercase(),
        model: provider_cfg.model.clone(),
    };

    tokio::spawn(async move {
        // partial 回调：发 Tauri 事件给 overlay。
        let app_for_cb = app_handle.clone();
        let on_partial: voice_core::pipeline::PartialCallback = Arc::new(move |text| {
            let _ = app_for_cb.emit("recording://partial", text.to_string());
        });

        // 录音 + 收集 finals（不在内部插入——插入需在焦点恢复后进行）。
        let result = pipeline
            .record_and_collect(
                audio,
                &provider_cfg,
                meta,
                Some(on_partial),
                Some(stop_flag),
            )
            .await;

        match result {
            Ok(r) => {
                // 识别结束 → 上屏前：还焦，但 HUD 仍可见，提示正在输入。
                let _ = app_handle.emit("recording://processing", "正在输入…");
                restore_frontmost_focus(&app_handle, frontmost.as_deref());
                tokio::time::sleep(std::time::Duration::from_millis(120)).await;

                if let Err(e) = pipeline
                    .insert_finals_with_polish(&r.session_id, &r.utterances, &polish_ctx)
                    .await
                {
                    log_error!("插入文本失败：{e}");
                    let _ = app_handle.emit("recording://error", e.to_string());
                }

                *recording.write().await = false;
                guard.store(false, std::sync::atomic::Ordering::SeqCst);
                // 文字已上屏后再收起 HUD。
                hide_overlay_only(&app_handle);
                let _ = app_handle.emit("recording://stopped", r.utterances.join(""));
            }
            Err(e) => {
                hide_overlay_only(&app_handle);
                if let Some(ref bid) = frontmost {
                    restore_frontmost_focus(&app_handle, Some(bid.as_str()));
                }
                *recording.write().await = false;
                guard.store(false, std::sync::atomic::Ordering::SeqCst);
                let _ = app_handle.emit("recording://error", e.to_string());
            }
        }
    });

    Ok(true)
}

/// 释放录音启动 guard（启动失败时回退，保证下次能重新触发）。
fn release_recording_guard(state: &State<'_, AppState>) {
    state
        .recording_guard
        .store(false, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub async fn get_recording_state(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(*state.recording.read().await)
}

// ──────────────── 音频设备 ────────────────

/// 列出可用输入设备名（设置页麦克风下拉）。
#[tauri::command]
pub fn list_audio_devices() -> Vec<String> {
    CpalAudioSource::list_input_devices()
}

/// 测试麦克风：采集约 0.6s 返回峰值振幅（0..1）。
#[tauri::command]
pub async fn test_microphone(device: Option<String>) -> Result<f32, String> {
    CpalAudioSource::test_input_level(device)
        .await
        .map_err(|e| e.to_string())
}

// ──────────────── 热词词典 ────────────────

#[tauri::command]
pub fn list_hotwords(state: State<'_, AppState>) -> Result<Vec<Hotword>, String> {
    state.store.list_hotwords().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_hotword(
    state: State<'_, AppState>,
    word: String,
    weight: i32,
) -> Result<Hotword, String> {
    state
        .store
        .add_hotword(&word, weight)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_hotword(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.store.delete_hotword(&id).map_err(|e| e.to_string())
}

#[allow(dead_code)]
fn _ensure_arc(_a: &Arc<()>) {}
