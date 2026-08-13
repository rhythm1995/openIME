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
use voice_core::traits::{HistoryStore, TextPolishProvider};
use voice_core::{AppConfig, Hotword, ProviderConfig, SessionSummary, UtteranceRecord};

use crate::platform::current::permissions::MacPermissionChecker;
use crate::state::{save_config, AppState};
use crate::{log_debug, log_error, log_info, log_warn};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Shortcut};

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
    // R3：保存期校验所有非空用户 endpoint（不强制 api_key；不合法整单不落盘）。
    validate_all_endpoints(&config).map_err(|e| e.to_string())?;
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

/// R3：保存期校验所有非空用户 endpoint（provider base_url + polish_cloud_endpoint）。
/// 百炼验归一化 wss；REST 验原文。Sherpa 无 URL 跳过。不强制 api_key。
fn validate_all_endpoints(cfg: &AppConfig) -> Result<(), String> {
    use voice_core::ProviderKind;
    for p in &cfg.providers {
        let url = p.base_url.trim();
        if url.is_empty() {
            continue;
        }
        let target = match p.kind {
            ProviderKind::Bailian => voice_core::providers::bailian::normalize_ws_url(url),
            ProviderKind::OpenAiAsr | ProviderKind::MultimodalAsr => url.to_string(),
            ProviderKind::Sherpa => continue,
        };
        voice_core::endpoint::validate_endpoint(&target)
            .map_err(|e| format!("endpoint「{}」校验失败：{e}", p.base_url))?;
    }
    if !cfg.polish_cloud_endpoint.trim().is_empty() {
        voice_core::endpoint::validate_endpoint(cfg.polish_cloud_endpoint.trim())
            .map_err(|e| format!("润色 endpoint 校验失败：{e}"))?;
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

/// 测试云端润色 LLM 连接（按 polish_cloud_protocol 发一个 "ping" 看返回）。
#[tauri::command]
pub async fn test_cloud_polish(state: State<'_, AppState>) -> Result<String, String> {
    let cfg = state.config.read().await.clone();
    let (base, key, model, protocol) = {
        if !cfg.polish_cloud_endpoint.trim().is_empty()
            && !cfg.polish_cloud_api_key.trim().is_empty()
        {
            (
                cfg.polish_cloud_endpoint.clone(),
                cfg.polish_cloud_api_key.clone(),
                cfg.polish_cloud_model.clone(),
                cfg.polish_cloud_protocol,
            )
        } else {
            // 回退 bailian provider
            let p = cfg
                .providers
                .iter()
                .find(|p| {
                    p.kind == voice_core::ProviderKind::Bailian && !p.api_key.trim().is_empty()
                });
            let Some(p) = p else {
                // 没配云端润色 key 属正常状态（本地优先运行，不影响使用），不算错误。
                return Ok("未配置云端润色 API Key。本地优先运行，不影响使用；如需云端润色，请填 polish 独立配置或百炼 provider key。".into());
            };
            (
                voice_core::BailianChatPolish::default_chat_base(),
                p.api_key.clone(),
                cfg.polish_cloud_model.clone(),
                voice_core::PolishCloudProtocol::OpenAiChat,
            )
        }
    };
    let provider = voice_core::BailianChatPolish::new_with_protocol(key, base, model, protocol);
    log_info!("测试云端润色连接：protocol={:?}", protocol);
    let req = voice_core::PolishRequest {
        text: "ping".into(),
        mode: voice_core::PolishMode::Light,
        style_prompt: None,
        hotwords: vec![],
        timeout: std::time::Duration::from_secs(20),
    };
    match provider.polish(req).await {
        Ok(r) => {
            log_info!("云端润色连接成功：{} 字返回", r.text.chars().count());
            Ok(format!(
                "连接成功！模型已就绪（返回 {}）",
                r.text.chars().take(30).collect::<String>()
            ))
        }
        Err(e) => {
            log_error!("云端润色连接测试失败：{e}");
            Err(e.to_string())
        }
    }
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

/// D2：跨会话搜索录音文本（LIKE 模糊匹配）。
#[tauri::command]
pub fn search_utterances(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<UtteranceRecord>, String> {
    state
        .store
        .search_utterances(&query)
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
    /// 本机适配度标签（由 system.rs 打标）。首次无缓存时为空，前端应容忍 None。
    pub perf_tag: Option<voice_core::ModelPerfTag>,
}

const SYSTEM_INFO_KEY: &str = "system_info";

fn system_info_cached(store: &voice_core::SqliteStore) -> Option<voice_core::SystemInfo> {
    store
        .get_setting(SYSTEM_INFO_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn system_info_ensure(state: &State<'_, AppState>) -> Option<voice_core::SystemInfo> {
    if let Some(cached) = system_info_cached(&state.store) {
        return Some(cached);
    }
    let fresh = voice_core::collect_system_info();
    let json = serde_json::to_string(&fresh).unwrap_or_default();
    let _ = state.store.set_setting(SYSTEM_INFO_KEY, &json);
    Some(fresh)
}

#[tauri::command]
pub fn get_system_info(
    state: State<'_, AppState>,
    refresh: bool,
) -> Result<voice_core::SystemInfo, String> {
    if !refresh {
        if let Some(cached) = system_info_cached(&state.store) {
            return Ok(cached);
        }
    }
    let fresh = voice_core::collect_system_info();
    let json = serde_json::to_string(&fresh).map_err(|e| e.to_string())?;
    state
        .store
        .set_setting(SYSTEM_INFO_KEY, &json)
        .map_err(|e| e.to_string())?;
    Ok(fresh)
}

#[tauri::command]
pub fn list_local_asr_models(
    state: State<'_, AppState>,
) -> Result<Vec<LocalAsrModelEntry>, String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let active = state.config.blocking_read().resolved_local_asr_model();
    // 本机信息：读缓存，若无则采集一次并写回（极简持久化）。
    let sys_opt = system_info_ensure(&state);
    let entries = voice_core::asr_model_catalog()
        .iter()
        .map(|m| {
            let missing = voice_core::missing_files_for(&model_root, m.id);
            let missing_size: u64 = missing.iter().map(|f| f.size).sum();
            let perf_tag = sys_opt
                .as_ref()
                .map(|sys| voice_core::compute_model_tag(m.approx_size, sys));
            LocalAsrModelEntry {
                id: m.id.to_string(),
                title: m.title.to_string(),
                description: m.description.to_string(),
                backend: match m.backend {
                    voice_core::AsrBackend::OfflineSenseVoice => "offline_sense_voice".into(),
                    voice_core::AsrBackend::OfflineFireRed => "offline_fire_red".into(),
                    voice_core::AsrBackend::StreamingParaformer => "streaming_paraformer".into(),
                    voice_core::AsrBackend::OfflineFunAsrNano => "offline_funasr_nano".into(),
                },
                recommended: m.recommended,
                approx_size: m.approx_size,
                installed: missing.is_empty(),
                // 未安装不可算「使用中」
                active: missing.is_empty() && m.id == active,
                missing_size,
                perf_tag,
            }
        })
        .collect();
    Ok(entries)
}

/// 启用某个已安装的本地 ASR 模型：写回 config 并立即生效（无需手动点「保存设置」）。
/// 同步 local_asr_model / local_mode / sherpa provider.model。
#[tauri::command]
pub fn set_active_asr_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    let id = voice_core::normalize_asr_model_id(&model_id).to_string();
    // 校验：必须是目录里的已知模型。
    if voice_core::asr_model_by_id(&id).is_none() {
        return Err(format!("未知的 ASR 模型 id：{model_id}"));
    }
    {
        let mut cfg = state.config.blocking_write();
        cfg.local_asr_model = id.clone();
        cfg.sync_local_asr_fields();
        // 持久化（store 已在 AppState 持有）。
        if let Err(e) = crate::state::save_config(&state.store, &cfg) {
            return Err(format!("保存配置失败：{e}"));
        }
    }
    log_info!("已启用本地 ASR 模型：{id}");
    let _ = app.emit("asr://active-changed", &id);
    Ok(())
}

/// 删除某个已安装本地 ASR 模型的全部文件（不影响共享 VAD）。
#[tauri::command]
pub fn delete_local_asr_model(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let id = voice_core::normalize_asr_model_id(&model_id).to_string();
    let Some(info) = voice_core::asr_model_by_id(&id) else {
        return Err(format!("未知的 ASR 模型 id：{model_id}"));
    };
    // 删除模型主体目录（不动共享的 vad/）。
    let dir = model_root.join(info.dir_name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("删除模型目录失败 {}: {e}", dir.display()))?;
        log_info!("已删除本地 ASR 模型目录：{}", dir.display());
    }
    // 若删的是当前启用模型，回退到任一已装候选；都删了则保留配置（录音时会引导下载）。
    {
        let cfg = state.config.blocking_read();
        if cfg.resolved_local_asr_model() == id {
            drop(cfg);
            let fallback = voice_core::asr_model_catalog()
                .iter()
                .find(|m| m.id != id && voice_core::is_asr_model_installed(&model_root, m.id));
            if let Some(fb) = fallback {
                let mut cfg = state.config.blocking_write();
                cfg.local_asr_model = fb.id.to_string();
                cfg.sync_local_asr_fields();
                let _ = crate::state::save_config(&state.store, &cfg);
                log_info!("删后回退启用模型：{}", fb.id);
            }
        }
    }
    Ok(())
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
pub async fn install_polish_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
    let mut provider_cfg = cfg
        .active()
        .map_err(|e| {
            release_recording_guard(&state);
            e.to_string()
        })?
        .clone();

    // 本地引擎：注入当前启用的 ASR 模型 id；未安装则回退到任一已装候选。
    if provider_cfg.kind == voice_core::ProviderKind::Sherpa {
        let mut model_id = cfg.resolved_local_asr_model();
        if let Some(root) = state.model_root() {
            if !voice_core::is_local_engine_installed_for(&root, &model_id) {
                let fallback = voice_core::asr_model_catalog()
                    .iter()
                    .find(|m| voice_core::is_local_engine_installed_for(&root, m.id));
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
        CpalAudioSource::new_with_device(cfg.audio_device.clone()).map_err(|e| {
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
        // partial 回调：overlay 显示实时识别（不上屏；统一在 final 后一次性插入）。
        let app_for_cb = app_handle.clone();
        let on_partial: voice_core::pipeline::PartialCallback = Arc::new(move |text| {
            let _ = app_for_cb.emit("recording://partial", text.to_string());
        });

        // C1：流式引擎（百炼）边说边逐字上屏；离线引擎录完一次性插入。
        let streaming = provider_cfg.kind == voice_core::ProviderKind::Bailian;
        // 流式模式：录音期间就上屏，先还焦。
        if streaming {
            restore_frontmost_focus(&app_handle, frontmost.as_deref());
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }

        // 录音 + 收集 finals（流式模式下 finals 已在内部逐字上屏）。
        let result = pipeline
            .record_and_collect(
                audio,
                &provider_cfg,
                meta,
                Some(on_partial),
                Some(stop_flag),
                streaming,
            )
            .await;

        match result {
            Ok(r) => {
                let _ = app_handle.emit("recording://processing", "正在输入…");
                if streaming {
                    // C1：已逐字上屏，只落库（不重复插入、不润色——流式模式优先实时性）。
                    if let Err(e) = pipeline.persist_finals(&r.session_id, &r.utterances).await {
                        log_error!("流式结果落库失败：{e}");
                    }
                } else {
                    // 离线：录完还焦 + 一次性润色上屏。
                    restore_frontmost_focus(&app_handle, frontmost.as_deref());
                    tokio::time::sleep(std::time::Duration::from_millis(120)).await;

                    // B5+B6：按前台 app 半角标点偏好 + 繁简偏好转换 finals。
                    let finals: Vec<String> = {
                        let app_state = app_handle.state::<AppState>();
                        let cfg = app_state.config.read().await;
                        let half = cfg.punct_half_width_apps.iter().any(|kw| {
                            frontmost
                                .as_deref()
                                .map(|f| f.contains(kw.as_str()))
                                .unwrap_or(false)
                        });
                        let script = cfg.chinese_script_preference;
                        r.utterances
                            .iter()
                            .map(|t| {
                                let mut s = voice_core::polish::convert_script(t, script);
                                if half {
                                    s = voice_core::polish::full_to_half_punct(&s);
                                }
                                s
                            })
                            .collect()
                    };
                    // R2:润色前清掉取消标志 + 动态注册 ESC 中断快捷键（润色结束注销）。
                    app_handle
                        .state::<AppState>()
                        .clear_cancel_polish();
                    let esc = Shortcut::new(None, Code::Escape);
                    let _ = app_handle.global_shortcut().register(esc);
                    let insert_res = pipeline
                        .insert_finals_with_polish(&r.session_id, &finals, &polish_ctx)
                        .await;
                    // 润色结束（完成 / 取消 / 超时）注销 ESC，避免长期占用。
                    let _ = app_handle.global_shortcut().unregister(esc);
                    if let Err(e) = insert_res {
                        log_error!("插入文本失败：{e}");
                        let _ = app_handle.emit("recording://error", e.to_string());
                    }
                } // 闭合 else（非流式分支）

                *recording.write().await = false;
                guard.store(false, std::sync::atomic::Ordering::SeqCst);
                hide_overlay_only(&app_handle);
                let deduped = voice_core::polish::dedupe_consecutive_finals(&r.utterances);
                let _ = app_handle.emit("recording://stopped", deduped.join(""));
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

#[derive(serde::Serialize)]
pub struct HotwordImportResult {
    pub imported: usize,
    pub total: usize,
}

/// 从 CSV 文本批量导入热词（每行一个词；支持「词,权重」取首列；忽略空行与重复）。
/// 热词用于：L0 同音/模糊音纠错 + 润色 prompt 保留专有名词（本地 ASR 无解码层热词偏置）。
#[tauri::command]
pub fn import_hotwords_csv(
    state: State<'_, AppState>,
    content: String,
) -> Result<HotwordImportResult, String> {
    let words: Vec<String> = content
        .lines()
        .map(|l| l.split(',').next().unwrap_or("").trim().to_string())
        .collect();
    let imported = state
        .store
        .add_hotwords_batch(&words)
        .map_err(|e| e.to_string())?;
    let total = state.store.list_hotwords().map(|v| v.len()).unwrap_or(0);
    Ok(HotwordImportResult { imported, total })
}

// ── 风格包（F1）──

#[tauri::command]
pub fn list_style_packs(state: State<'_, AppState>) -> Result<Vec<voice_core::StylePack>, String> {
    state.store.list_style_packs().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_active_style_pack(state: State<'_, AppState>, id: Option<String>) -> Result<(), String> {
    log_info!("切换风格包：{:?}", id);
    {
        let mut cfg = state.config.blocking_write();
        cfg.active_style_pack_id = id;
        if let Err(e) = crate::state::save_config(&state.store, &cfg) {
            return Err(format!("保存配置失败：{e}"));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn upsert_style_pack(
    state: State<'_, AppState>,
    pack: voice_core::StylePack,
) -> Result<voice_core::StylePack, String> {
    state
        .store
        .upsert_style_pack(&pack)
        .map_err(|e| e.to_string())?;
    Ok(pack)
}

#[tauri::command]
pub fn delete_style_pack(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .store
        .delete_style_pack(&id)
        .map_err(|e| e.to_string())
}

/// F4：读前台 app 当前选中的文字（macOS AX 直读，不碰剪贴板）。
#[tauri::command]
pub fn get_selection() -> Result<Option<String>, String> {
    #[cfg(target_os = "macos")]
    {
        Ok(crate::platform::current::fn_key::get_selection())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(None)
    }
}

/// D3：文件转录结果（文本 + srt 字幕）。
#[derive(serde::Serialize)]
pub struct TranscribeResult {
    pub text: String,
    pub srt: String,
    /// 音频文件名（前端展示用）。
    pub file_name: String,
}

/// D3：转录音频文件 → (文本, srt)。用当前选中的本地 ASR 模型。
#[tauri::command]
pub async fn transcribe_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<TranscribeResult, String> {
    let (model_id, lang, model_root) = {
        let cfg = state.config.read().await;
        (
            cfg.resolved_local_asr_model(),
            cfg.local_language.clone(),
            state.model_root(),
        )
    };
    let root = model_root.ok_or("未配置本地模型路径，请先下载模型")?;
    let file_name = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio")
        .to_string();
    log_info!("开始转录文件：{path}（模型 {model_id}）");
    let path_clone = path.clone();
    let result = tokio::task::spawn_blocking(move || {
        voice_core::transcribe::transcribe_file_full(
            std::path::Path::new(&path_clone),
            &root,
            &model_id,
            &lang,
        )
    })
    .await
    .map_err(|e| format!("转录任务失败：{e}"))?
    .map_err(|e| e.to_string())?;
    log_info!("转录完成：{path}（{} 字）", result.0.chars().count());
    Ok(TranscribeResult {
        text: result.0,
        srt: result.1,
        file_name,
    })
}

/// D1：导出所有录音为 Markdown 日记（按日期分组）。
#[tauri::command]
pub fn export_diary(state: State<'_, AppState>) -> Result<String, String> {
    state
        .store
        .export_diary_markdown()
        .map_err(|e| e.to_string())
}
