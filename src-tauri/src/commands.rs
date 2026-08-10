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
pub fn save_app_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<(), String> {
    config.active().map_err(|e| e.to_string())?;
    let hotkey_changed = state.config.blocking_read().hotkey != config.hotkey;
    save_config(&state.store, &config).map_err(|e| e.to_string())?;
    let new_hotkey = config.hotkey.clone();
    *state.config.blocking_write() = config;
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
}

#[tauri::command]
pub fn get_local_model_status(
    state: State<'_, AppState>,
    mode: Option<String>,
) -> Result<LocalModelStatus, String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let mode = mode.unwrap_or_else(|| state.config.blocking_read().local_mode.clone());
    let files = voice_core::local_model_files_for(&mode);
    let missing = voice_core::missing_files_for(&model_root, &mode);
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
    })
}

/// 下载安装本地引擎模型（后台进行，进度经 model://download-progress 事件推送）。
#[tauri::command]
pub async fn install_local_model(
    app: AppHandle,
    state: State<'_, AppState>,
    mode: Option<String>,
) -> Result<(), String> {
    let Some(model_root) = state.model_root() else {
        return Err("未配置本地模型目录".to_string());
    };
    let mode = mode.unwrap_or_else(|| state.config.blocking_read().local_mode.clone());

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
    tauri::async_runtime::spawn(async move {
        let app_for_cb = app_for_task.clone();
        let result = voice_core::install_local_engine(&model_root, &mode, &move |p| {
            let _ = app_for_cb.emit("model://download-progress", &p);
        })
        .await;
        flag.store(false, std::sync::atomic::Ordering::SeqCst);
        match result {
            Ok(()) => {
                log_info!("本地模型安装完成");
                let _ = app_for_task.emit("model://download-complete", ());
            }
            Err(e) => {
                log_error!("本地模型安装失败：{e}");
                let _ = app_for_task.emit("model://download-error", e.to_string());
            }
        }
    });
    log_info!("本地模型下载任务已启动");
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
    // 正在录音 → 请求停止。
    {
        let rec = state.recording.read().await;
        if *rec {
            drop(rec);
            state.request_stop();
            return Ok(false);
        }
    }

    // 读当前 provider 配置。
    let cfg = state.config.read().await.clone();
    let mut provider_cfg = cfg.active().map_err(|e| e.to_string())?.clone();
    provider_cfg.validate().map_err(|e| e.to_string())?;

    // 离线模式：给 model 名加 "offline:" 前缀，让 SherpaProvider 走 OfflineRecognizer 路径。
    if provider_cfg.kind == voice_core::ProviderKind::Sherpa && cfg.local_mode == "offline" {
        provider_cfg.model = format!("offline:{}", provider_cfg.model);
    }

    // 懒初始化 pipeline（含 enigo，可能在无辅助功能权限时失败）。
    let pipeline = state.pipeline().await.map_err(|e| e.to_string())?;

    // 建立音频源（优先用户选定的麦克风，否则系统默认）。
    let audio: Box<dyn voice_core::AudioSource> = Box::new(
        CpalAudioSource::new_with_device(cfg.audio_device.clone())
            .map_err(|e| e.to_string())?,
    );

    state.clear_stop();
    *state.recording.write().await = true;

    // frontmost 由 trigger_toggle 在 overlay 显示前捕获并传入（overlay 抢焦点后再
    // 捕获会拿到 openIME 自己）。录音结束先激活回原 app，再插入文本。
    log_info!("录音前前台 app：{:?}", frontmost);

    let recording = state.recording.clone();
    let stop_flag = state.stop_flag.clone();
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
                // 1) 隐藏 overlay；2) 激活回原前台 app；3) 再插入文本。
                if let Some(win) = app_handle.get_webview_window("overlay") {
                    let _ = win.hide();
                }
                if let Some(ref bid) = frontmost {
                    // 短暂延迟让 overlay hide 生效。
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    let activated =
                        crate::platform::current::fn_key::activate_app(bid);
                    log_info!("录音后激活原前台 app {}：{}", bid, activated);
                    // 激活后短暂等待，让目标窗口获得焦点。
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }

                // 焦点就绪后插入文本并落库。
                if let Err(e) = pipeline.insert_finals(&r.session_id, &r.utterances).await {
                    log_error!("插入文本失败：{e}");
                    let _ = app_handle.emit("recording://error", e.to_string());
                }

                *recording.write().await = false;
                let _ = app_handle.emit("recording://stopped", r.utterances.join(""));
            }
            Err(e) => {
                *recording.write().await = false;
                let _ = app_handle.emit("recording://error", e.to_string());
            }
        }
    });

    Ok(true)
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
