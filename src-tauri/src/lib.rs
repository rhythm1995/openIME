//! openIME Tauri 薄壳：只做 IPC 命令包装、插件注册、托盘/快捷键。
//! 所有业务逻辑都在 voice-core。

mod commands;
mod credentials;
mod insert_fallback;
mod logging;
mod platform;
mod qa;
mod state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use voice_core::SqliteStore;

use state::AppState;

// 日志宏由 #[macro_export] 定义在 crate 根，直接可用：
// log_debug! / log_info! / log_warn! / log_error!

/// Fn 键回调在原生块上下文执行（无捕获），需要全局拿到 AppHandle。
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

/// 默认快捷键（配置缺失/解析失败时兜底）。
const DEFAULT_HOTKEY: &str = "Alt+Shift+D";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 最先初始化日志：之后的任何启动失败（含 panic）都会落盘。
    let log_dir = logging::init();
    log_info!("openIME 启动，日志目录：{}", log_dir.display());

    let mut app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // 开机自启（macOS 用 LaunchAgent）：自启时附带 --autostart 参数，
        // setup 据此判断是开机自启（静默常驻菜单栏）还是正常启动（显示面板）。
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    on_hotkey(app, shortcut);
                })
                .build(),
        )
        .setup(|app| {
            log_info!("setup 开始");

            // 数据库 + 模型根目录。
            let data_dir = app.path().app_data_dir().map_err(|e| {
                log_error!("获取 app_data_dir 失败：{e}");
                e
            })?;
            log_info!("data_dir = {}", data_dir.display());
            std::fs::create_dir_all(&data_dir).map_err(|e| {
                log_error!("创建 data_dir 失败：{e}");
                anyhow::anyhow!("创建 data_dir 失败: {e}")
            })?;

            // 单实例：通过 unix domain socket 协调。
            // 已有实例在跑 → 新进程发 "show" 指令唤起已运行实例的主窗口，然后自己退出。
            // 这是用户「再次打开 app」时期望的行为（弹出现有实例，而非开第二个）。
            let sock_path = data_dir.join("openime.sock");
            if let Err(e) = single_instance_check(app.handle().clone(), &sock_path) {
                log_info!("单实例检查：{e}，退出本进程");
                // 唤起已运行实例后退出本进程。setup 早期阶段直接 process::exit 最干净。
                std::process::exit(0);
            }

            let db_path = data_dir.join("openime.db");
            let store = SqliteStore::open(&db_path).map_err(|e| {
                log_error!("打开数据库失败：{e}（路径：{}）", db_path.display());
                anyhow::anyhow!("打开数据库失败: {e}")
            })?;
            log_info!("数据库已打开：{}", db_path.display());

            // 本地模型根：app_data_dir/models（paraformer 子目录）+ app_data_dir/models/vad（silero）。
            let model_root = data_dir.join("models");
            let vad_root = model_root.join("vad");
            std::fs::create_dir_all(&model_root).map_err(|e| {
                log_error!("创建模型目录失败：{e}");
                anyhow::anyhow!("创建模型目录失败: {e}")
            })?;
            let sherpa_root = Some((model_root, vad_root));
            let state = AppState::new(app.handle().clone(), store, sherpa_root).map_err(|e| {
                log_error!("初始化状态失败：{e}");
                anyhow::anyhow!("初始化状态失败: {e}")
            })?;
            app.manage(state);
            log_info!("AppState 初始化完成");

            // Fn 键监听供原生回调取用（块无捕获，只能走全局句柄）。
            let _ = APP_HANDLE.set(app.handle().clone());

            // 托盘菜单（失败不阻塞启动：菜单栏 App 至少要能跑）。
            let open_main = MenuItem::with_id(app, "open_main", "打开主窗口", true, None::<&str>)?;
            let history = MenuItem::with_id(app, "history", "历史记录", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "退出 openIME", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_main, &history, &settings, &sep, &quit])?;
            log_info!("托盘菜单已创建");
            // 菜单栏图标：用单色 template image（声波剪影），macOS 会随明暗模式自动反色。
            // 失败则退回 default_window_icon（彩色 app icon），最坏退回无图标。
            let tray_icon: Option<tauri::image::Image> = match tauri::image::Image::from_bytes(
                include_bytes!("../icons/menubar-template@2x.png"),
            ) {
                Ok(img) => Some(img),
                Err(e) => {
                    log_warn!("菜单栏 template 图标加载失败，退回 app icon：{e}");
                    app.default_window_icon().cloned()
                }
            };
            log_info!(
                "托盘图标：{}",
                if tray_icon.is_some() {
                    "有（template）"
                } else {
                    "无（托盘可能不可见！）"
                }
            );
            // 左键点击托盘图标 = 直接打开主窗口（不弹菜单，最简交互）。
            // 右键仍可弹菜单（含打开/设置/历史/退出）作为兜底入口。
            let mut tray_builder = tauri::tray::TrayIconBuilder::with_id("main-tray")
                .tooltip("openIME — 点击打开")
                .icon_as_template(true)
                .show_menu_on_left_click(false)
                .menu(&menu)
                .on_menu_event(|app, event| {
                    log_info!("托盘菜单点击：{}", event.id.as_ref());
                    match event.id.as_ref() {
                        "open_main" | "show_main" => show_main_window(app),
                        "history" => {
                            show_main_window(app);
                            let _ = app.emit("nav://goto", "history");
                        }
                        "settings" => {
                            show_main_window(app);
                            let _ = app.emit("nav://goto", "settings");
                        }
                        "quit" => {
                            log_info!("收到退出指令");
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    // 左键抬起 → 直接打开主窗口。
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        log_info!("托盘左键点击 → 打开主窗口");
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(ic) = tray_icon {
                tray_builder = tray_builder.icon(ic);
            }
            match tray_builder.build(app) {
                Ok(_) => log_info!("托盘创建成功"),
                Err(e) => log_warn!("托盘创建失败（忽略，继续启动）：{e}"),
            }

            // 关闭主窗口 = 隐藏回菜单栏（不退出）；并恢复 Accessory，避免 Dock 再挂一个图标。
            if let Some(main) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(win) = app_handle.get_webview_window("main") {
                            let _ = win.hide();
                        }
                        // 回到菜单栏常驻形态，Dock 图标消失。
                        let _ =
                            app_handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                        log_info!("main 窗口关闭请求 → 隐藏并恢复 Accessory");
                    }
                });
            }

            // overlay 启动即配成 HUD（不可聚焦/鼠标穿透），避免首次 Fn 才配置时闪一下焦点。
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.set_focusable(false);
                let _ = overlay.set_ignore_cursor_events(true);
                #[cfg(target_os = "macos")]
                if let Ok(ns) = overlay.ns_window() {
                    crate::platform::current::fn_key::prepare_overlay_window(ns);
                }
            }

            // R6：QA 窗关闭请求 → close_qa_panel（禁止只 hide 留 messages，NFR-6.3）。
            if let Some(qa_win) = app.get_webview_window("qa") {
                let app_handle = app.handle().clone();
                qa_win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        qa::close_qa_panel(&app_handle);
                    }
                });
            }

            // 快捷键注册中心（PR4 收口）：录音 / 风格循环 / 翻译 / QA。
            let cfg = app.state::<AppState>().config.blocking_read().clone();
            apply_hotkey(app.handle(), &cfg);

            // 记录启动完成后的窗口状态（排查"看不到面板"的关键）。
            for label in ["main", "overlay"] {
                match app.get_webview_window(label) {
                    Some(win) => {
                        let visible = win.is_visible().ok();
                        let size = win.outer_size().ok();
                        log_info!("窗口 [{label}] 存在：visible={visible:?}, size={size:?}");
                    }
                    None => log_warn!("窗口 [{label}] 不存在"),
                }
            }

            // 正常启动（非开机自启）：主动显示主面板。
            // 开机自启（LaunchAgent 附带 --autostart）时保持隐藏、静默常驻菜单栏。
            let autostarted = std::env::args().any(|a| a == "--autostart");
            log_info!("autostarted = {autostarted}");
            match app.get_webview_window("main") {
                Some(win) => {
                    // 默认约 1200×800：够大但多数 Mac 上不会一进来就近全屏
                    // （原先 1800×1200 在 14/16" 逻辑分辨率下常被裁到几乎铺满）
                    const DEFAULT_W: f64 = 1200.0;
                    const DEFAULT_H: f64 = 800.0;
                    if let Err(e) = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                        DEFAULT_W, DEFAULT_H,
                    ))) {
                        log_warn!("设置 main 默认尺寸失败：{e}");
                    } else {
                        log_info!("main 窗口尺寸设为 {DEFAULT_W}x{DEFAULT_H}");
                    }
                    let _ = win.center();
                    if autostarted {
                        log_info!("开机自启：main 窗口保持隐藏，常驻菜单栏");
                    } else {
                        if let Err(e) = win.show() {
                            log_error!("启动时显示 main 失败：{e}");
                        }
                        let _ = win.set_focus();
                        log_info!("正常启动：main 窗口已显示");
                    }
                }
                None => log_warn!("启动时未找到 main 窗口"),
            }

            log_info!("setup 完成");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::default_config,
            commands::get_config,
            commands::save_app_config,
            commands::validate_provider,
            commands::test_cloud_connection,
            commands::test_cloud_polish,
            commands::create_session,
            commands::save_utterance,
            commands::list_sessions,
            commands::list_utterances,
            commands::search_utterances,
            commands::delete_session,
            commands::check_permission,
            commands::request_accessibility,
            commands::request_microphone,
            commands::open_permission_settings,
            commands::toggle_recording,
            commands::get_recording_state,
            commands::list_audio_devices,
            commands::test_microphone,
            commands::list_hotwords,
            commands::add_hotword,
            commands::delete_hotword,
            commands::import_hotwords_csv,
            commands::list_style_packs,
            commands::set_active_style_pack,
            commands::upsert_style_pack,
            commands::delete_style_pack,
            commands::get_selection,
            commands::transcribe_file,
            commands::export_diary,
            commands::frontend_log,
            commands::set_launch_at_login,
            commands::get_launch_at_login,
            commands::list_local_asr_models,
            commands::get_system_info,
            commands::set_active_asr_model,
            commands::delete_local_asr_model,
            commands::get_local_model_status,
            commands::install_local_model,
            commands::get_polish_model_status,
            commands::install_polish_model,
            commands::qa_refresh_selection,
            commands::qa_cancel,
            commands::qa_insert_last,
            commands::qa_clear,
            commands::qa_copy_last,
        ])
        .build(tauri::generate_context!())
        .expect("构建 Tauri 应用失败");

    log_info!("进入事件循环");

    // 默认 Accessory：不抢焦点、录音 HUD 不抢前台。
    // 若 setup 已显示主窗口（非开机自启），再切回 Regular，否则会出现
    // 「启动后主窗口在但点不进 / Dock 与菜单栏像装了两份」的错觉。
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    if let Some(main) = app.get_webview_window("main") {
        if main.is_visible().unwrap_or(false) {
            app.set_activation_policy(tauri::ActivationPolicy::Regular);
            log_info!("主窗口可见 → Regular 激活策略");
        } else {
            log_info!("主窗口隐藏 → Accessory（菜单栏常驻）");
        }
    }
    app.run(|app_handle, event| {
        // Dock 图标点击：始终拉起主面板（即使 has_visible_windows 因 overlay 为 true）。
        if let tauri::RunEvent::Reopen { .. } = event {
            log_info!("Reopen（Dock 点击）→ 打开主窗口");
            show_main_window(app_handle);
        }
    });
    log_info!("Tauri 事件循环结束，进程退出");
}

/// 全局快捷键：按注册中心分流——录音（听写/QA 录音）→ 翻译 → QA 窗开关 → 风格循环。
fn on_hotkey(app: &tauri::AppHandle, shortcut: &Shortcut) {
    // R2:润色中按 ESC → 取消润色（ESC 由润色流程动态注册，见 commands.rs）。
    // R6:QA 流式中按 ESC → 取消 QA 流（保留已输出）。
    if shortcut.key == Code::Escape && shortcut.mods.is_empty() {
        let state = app.state::<AppState>();
        state.request_cancel_polish();
        let _ = app.emit("recording://polish-cancelled", ());
        if qa::panel_visible() && qa::phase() == qa::QaPhase::Streaming {
            qa::cancel_stream(app);
        }
        log_info!("已请求取消（ESC）");
        return;
    }
    let cfg = match app.state::<AppState>().config.try_read() {
        Ok(c) => c.clone(),
        Err(_) => return,
    };
    let style_sc = cfg.style_switch_hotkey.as_deref().and_then(parse_shortcut);
    if style_sc == Some(*shortcut) {
        cycle_style_pack(app);
        return;
    }
    let translate_sc = cfg.translate_hotkey.as_deref().and_then(parse_shortcut);
    if translate_sc == Some(*shortcut) {
        on_translate_hotkey(app);
        return;
    }
    let qa_sc = cfg.qa_hotkey.as_deref().and_then(parse_shortcut);
    if qa_sc == Some(*shortcut) {
        on_qa_hotkey(app);
        return;
    }
    let record_sc = parse_shortcut(cfg.hotkey.trim());
    if record_sc == Some(*shortcut) {
        on_record_hotkey(app);
        return;
    }
    // 未注册组合键（如动态注册的 ESC 之外）忽略。
    log_info!("未匹配的快捷键：{shortcut:?}");
}

/// R4：翻译快捷键（P1 仅 Toggle）。无云端 key → 不写 intent、不录音（FR-4.5）。
fn on_translate_hotkey(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    // 互斥表：听写 / 翻译录音中 → 忽略 + toast；QA 窗可见 → 忽略。
    if state.recording_guard.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = app.emit("toast://info", "录音进行中，翻译键已忽略");
        return;
    }
    if qa::panel_visible() {
        let _ = app.emit("toast://info", "问答面板打开中，翻译键已忽略");
        return;
    }
    if !state.has_cloud_key() {
        let _ = app.emit("toast://info", "请先配置云端 LLM（润色 endpoint + API Key）");
        log_info!("翻译键：无云端 key，拒绝开始");
        return;
    }
    if let Ok(mut intent) = state.pending_intent.lock() {
        *intent = voice_core::SessionIntent::Translate;
    }
    log_info!("翻译会话开始（intent=Translate）");
    trigger_toggle(app);
}

/// R6：QA 快捷键 toggle。开窗前抓选区 + 冻结 frontmost；听写中 → 拒绝并 toast。
fn on_qa_hotkey(app: &tauri::AppHandle) {
    if qa::panel_visible() {
        qa::close_qa_panel(app);
        return;
    }
    // A6.4：听写进行中 QA 键不开窗。
    let state = app.state::<AppState>();
    if state.recording_guard.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = app.emit("toast://info", "录音进行中，问答面板暂不可开");
        return;
    }
    qa::open_qa_panel(app);
}

/// 录音键：QA 窗可见时改走 QA 录音（流式中 → 取消流）；否则听写。
fn on_record_hotkey(app: &tauri::AppHandle) {
    if qa::panel_visible() {
        match qa::phase() {
            qa::QaPhase::Streaming | qa::QaPhase::Transcribing => {
                // FR-6.10：流式中再按录音键 = 取消（保留已输出）。
                qa::cancel_stream(app);
                return;
            }
            qa::QaPhase::Recording => {
                // QA 录音中：走正常停止路径。
                trigger_toggle(app);
                return;
            }
            _ => {
                // Idle：开始 QA 录音（FR-6.9：无 key 不录）。
                let state = app.state::<AppState>();
                if !state.has_cloud_key() {
                    let _ = app.emit("toast://info", "请先配置云端 LLM（润色 endpoint + API Key）");
                    return;
                }
                if let Ok(mut intent) = state.pending_intent.lock() {
                    *intent = voice_core::SessionIntent::Qa;
                }
                qa::mark_recording(app, true);
            }
        }
    }
    trigger_toggle(app);
}

/// F1：循环切换风格包（None → 第一个 → ... → 末尾 → None/默认 Heavy）。
/// R5：只扫**无前缀**风格包（有 match_prefix 的包是角色，不进循环）。
fn cycle_style_pack(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let packs: Vec<_> = state
        .store
        .list_style_packs()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.is_prefix_role())
        .collect();
    if packs.is_empty() {
        let _ = app.emit("style://switched", "无风格包");
        return;
    }
    let current = state.config.blocking_read().active_style_pack_id.clone();
    let next = match current.as_deref() {
        None => Some(packs[0].id.clone()),
        Some(id) => match packs.iter().position(|p| p.id == id) {
            Some(idx) if idx + 1 < packs.len() => Some(packs[idx + 1].id.clone()),
            _ => None, // 末尾或未找到 → 回到默认 Heavy
        },
    };
    {
        let mut cfg = state.config.blocking_write();
        cfg.active_style_pack_id = next.clone();
        if let Err(e) = state::save_config(&state.store, &cfg) {
            log_warn!("风格包切换持久化失败：{e}");
        }
    }
    let label = next
        .as_deref()
        .and_then(|id| packs.iter().find(|p| p.id == id).map(|p| p.name.as_str()))
        .unwrap_or("默认 Heavy");
    log_info!("风格包切换：{label}");
    let _ = app.emit("style://switched", label);
}

/// 显示录音 overlay，尽量不抢走用户当前输入框的焦点/光标。
/// `frontmost`：显示**前**捕获的前台 app。
///
/// 注意：不要在这里调用 Tauri 的 set_focus / set_position / show——它们常会
/// makeKey 或激活 openIME，导致 input caret 消失。定位与显示全部走 ObjC HUD 路径。
fn show_overlay(app: &tauri::AppHandle, frontmost: Option<&str>) {
    match app.get_webview_window("overlay") {
        Some(win) => {
            // 仅设置忽略鼠标（Tauri）；失败忽略，ObjC 侧也会设 ignoresMouseEvents。
            let _ = win.set_ignore_cursor_events(true);
            let _ = win.set_focusable(false);

            #[cfg(target_os = "macos")]
            {
                // AppKit 坐标：原点在屏幕左下。算左下角偏上一点的位置。
                let (x, y) = match win.current_monitor() {
                    Ok(Some(monitor)) => {
                        let size = monitor.size();
                        let scale = monitor.scale_factor();
                        let logical_h = size.height as f64 / scale;
                        let win_h = win
                            .outer_size()
                            .map(|s| s.height as f64 / scale)
                            .unwrap_or(40.0);
                        // 物理→逻辑后，y 从底部往上 60pt。
                        let y = (logical_h - win_h - 60.0).max(0.0);
                        (16.0, y)
                    }
                    _ => (16.0, 60.0),
                };

                match win.ns_window() {
                    Ok(ns) => {
                        crate::platform::current::fn_key::show_overlay_preserving_focus(
                            ns, x, y, frontmost,
                        );
                        log_debug!(
                            "overlay HUD 显示（preserve focus），frontmost={frontmost:?} pos=({x:.0},{y:.0})"
                        );
                    }
                    Err(e) => {
                        // 最后手段：尽量不 set_focus。
                        log_warn!("获取 overlay ns_window 失败，降级 show：{e}");
                        let _ = win.set_position(tauri::Position::Logical(
                            tauri::LogicalPosition::new(x, y),
                        ));
                        if let Err(e) = win.show() {
                            log_error!("overlay show 失败：{e}");
                        }
                        if let Some(bid) = frontmost {
                            if bid != "com.openime.desktop" {
                                let ok = crate::platform::current::fn_key::activate_app(bid);
                                log_debug!("降级路径还焦 {bid}：{ok}");
                            }
                            // 同 app：禁止 set_focus(main)，会弄掉 textarea caret。
                        }
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                if let Err(e) = win.show() {
                    log_error!("overlay show 失败：{e}");
                }
            }
        }
        None => log_warn!("overlay 窗口不存在"),
    }
}

/// 显示主窗口（设置/历史）。Accessory 菜单栏 App 必须切到 Regular 并真正 activate，
/// 否则会出现「点了菜单/托盘却看不到窗口」的间歇问题。
fn show_main_window(app: &tauri::AppHandle) {
    match app.get_webview_window("main") {
        Some(win) => {
            // 1) 允许出现在 Dock 并参与激活（仅展示主面板期间）。
            if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
                log_warn!("切换 Regular 激活策略失败：{e}");
            }
            // 2) 取消最小化 / 显示 / 聚焦。
            let _ = win.unminimize();
            if let Err(e) = win.show() {
                log_error!("main show 失败：{e}");
            }
            // 若窗口跑到屏幕外（多显示器热拔等），拉回主屏中心。
            if let Ok(false) = win.is_visible() {
                let _ = win.center();
                let _ = win.show();
            }
            let _ = win.set_focus();
            // 3) 强制把本进程激活到前台（Accessory→Regular 后偶发不激活）。
            #[cfg(target_os = "macos")]
            {
                let ok = crate::platform::current::fn_key::activate_app("com.openime.desktop");
                log_info!("激活 openIME 自身：{ok}");
            }
            // 再 focus 一次，避免 activate 抢跑后焦点落空。
            let _ = win.set_focus();
            log_info!("main 窗口已显示");
        }
        None => log_warn!("main 窗口不存在"),
    }
}

/// R6：显示 QA 浮窗（可聚焦、不抢原 app）。Regular + show + set_focus，
/// 与 overlay 的 orderFront / 鼠标穿透完全不同。位置：指针所在屏右下角距边 24px，
/// 之后记住上次位置（窗口已有位置则不动）。
pub(crate) fn show_qa_window(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("qa") else {
        log_warn!("qa 窗口不存在");
        return;
    };
    if let Err(e) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
        log_warn!("QA：切换 Regular 激活策略失败：{e}");
    }
    // 首次显示：定位到指针所在屏右下角（距边 24px）。
    if let Ok(false) = win.is_visible() {
        let (scale, monitor) = match win.current_monitor() {
            Ok(Some(m)) => (m.scale_factor(), Some(m)),
            _ => (1.0, None),
        };
        let win_size = win
            .outer_size()
            .map(|s| (s.width as f64 / scale, s.height as f64 / scale))
            .unwrap_or((400.0, 520.0));
        let pos = match (monitor, app.cursor_position().ok()) {
            (Some(m), Some(cursor)) => {
                let mp = m.position();
                let ms = m.size();
                let cursor_in_monitor = cursor.x as f64 >= mp.x as f64
                    && cursor.x as f64 <= mp.x as f64 + ms.width as f64
                    && cursor.y as f64 >= mp.y as f64
                    && cursor.y as f64 <= mp.y as f64 + ms.height as f64;
                if cursor_in_monitor {
                    // 物理坐标 → 逻辑坐标（Tauri set_position 用逻辑坐标，原点左上）。
                    let x = mp.x as f64 / scale + (ms.width as f64 / scale) - win_size.0 - 24.0;
                    let y = mp.y as f64 / scale + (ms.height as f64 / scale) - win_size.1 - 24.0;
                    Some((x.max(0.0), y.max(0.0)))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some((x, y)) = pos {
            let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
                x, y,
            )));
        }
    }
    if let Err(e) = win.show() {
        log_error!("qa show 失败：{e}");
    }
    let _ = win.set_focus();
    log_info!("QA 窗口已显示");
}

/// 单实例协调：用 unix domain socket 做存在性探测。
///
/// - 若 socket 文件存在且能连上 → 已有实例在跑：发 "show" 唤起其主窗口，返回 Err 让调用方退出。
/// - 否则（无 socket / 连接失败 = 残留 socket）→ 本进程成为「主实例」，起一个监听线程
///   接收后续新进程的 "show" 指令并唤起本实例主窗口；返回 Ok 继续。
///
/// socket 走 app_data_dir，路径稳定、用户无关、无端口冲突。
/// Unix（macOS/Linux）：unix domain socket。
#[cfg(unix)]
fn single_instance_check(app: tauri::AppHandle, sock_path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::net::UnixStream;

    // 1) 先探测是否已有实例：尝试连接。
    if UnixStream::connect(sock_path).is_ok() {
        // 已连上 → 有实例在跑。发 "show" 指令（不关心响应），让对端弹主窗口。
        // 给对端一点时间接受；connect 成功即说明监听端已就绪。
        let _ = std::fs::write(sock_path.with_extension("show"), "1");
        // 也直接通过 socket 写一行，确保对端收到。
        if let Ok(mut s) = UnixStream::connect(sock_path) {
            use std::io::Write;
            let _ = s.write_all(b"show");
        }
        return Err("已有实例运行，已唤起其主窗口".into());
    }

    // 2) 残留 socket（上次崩溃未清理）：删除后绑定。
    let _ = std::fs::remove_file(sock_path);

    // 3) 本进程成为主实例：起监听线程。
    let listener = std::os::unix::net::UnixListener::bind(sock_path)
        .map_err(|e| format!("绑定单实例 socket 失败: {e}"))?;
    let app_for_listener = app.clone();
    std::thread::Builder::new()
        .name("single-instance-sock".into())
        .spawn(move || {
            use std::io::Read;
            for stream in listener.incoming() {
                if stream.is_err() {
                    continue;
                }
                let mut s = stream.unwrap();
                let mut buf = [0u8; 16];
                let _ = s.read(&mut buf);
                // 收到任意指令 → 唤起主窗口。
                log_info!("单实例：收到唤起请求，显示主窗口");
                show_main_window(&app_for_listener);
            }
        })
        .map_err(|e| format!("启动单实例监听线程失败: {e}"))?;

    Ok(())
}

/// Windows：暂用简单策略（无单实例协调），返回 Ok 继续。
/// TODO：用 Windows 命名 Mutex（CreateMutexW）实现真正的单实例。
#[cfg(not(unix))]
fn single_instance_check(
    _app: tauri::AppHandle,
    _sock_path: &std::path::Path,
) -> Result<(), String> {
    Ok(())
}

// ──────────────── 快捷键注册中心 ────────────────

/// PR4 收口：`unregister_all` 后注册 录音 / 风格循环 / 翻译 / QA。
/// 任何 hotkey 字段变化都调用本函数重新注册（save_app_config 与启动都走这里）。
fn apply_hotkey(app: &tauri::AppHandle, cfg: &voice_core::AppConfig) {
    let _ = app.global_shortcut().unregister_all();
    let trimmed = cfg.hotkey.trim();
    if trimmed.eq_ignore_ascii_case("fn") {
        // Fn 是修饰键，global-shortcut 无法注册，用原生 NSEvent 监听。
        crate::platform::current::fn_key::install_fn_monitor(on_fn_edge);
        log_info!("录音快捷键：Fn（原生监听）");
    } else {
        match parse_shortcut(trimmed) {
            Some(sc) => match app.global_shortcut().register(sc) {
                Ok(_) => log_info!("全局快捷键已注册（录音）：{trimmed}"),
                Err(e) => log_warn!("快捷键 {trimmed} 注册失败：{e}"),
            },
            None => {
                log_warn!("无法解析快捷键 {trimmed:?}，回退 {DEFAULT_HOTKEY}");
                if let Some(sc) = parse_shortcut(DEFAULT_HOTKEY) {
                    let _ = app.global_shortcut().register(sc);
                }
            }
        }
    }
    // 可选快捷键：风格循环 / 翻译 / QA（P1 仅 Toggle）。
    for (name, hk) in [
        ("风格包切换", cfg.style_switch_hotkey.as_deref()),
        ("翻译", cfg.translate_hotkey.as_deref()),
        ("问答", cfg.qa_hotkey.as_deref()),
    ] {
        let Some(s) = hk else { continue };
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        match parse_shortcut(s) {
            Some(sc) => match app.global_shortcut().register(sc) {
                Ok(_) => log_info!("全局快捷键已注册（{name}）：{s}"),
                Err(e) => log_warn!("{name}快捷键 {s} 注册失败：{e}"),
            },
            None => log_warn!("{name}快捷键无法解析：{s:?}"),
        }
    }
}

/// 解析 "Alt+Shift+D" / "Ctrl+Space" 风格字符串为 Shortcut。
fn parse_shortcut(s: &str) -> Option<Shortcut> {
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;
    for part in s.split('+') {
        let p = part.trim();
        match p.to_ascii_lowercase().as_str() {
            "" => {}
            "alt" | "option" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "cmd" | "command" | "meta" | "super" => mods |= Modifiers::SUPER,
            _ => code = parse_code(p),
        }
    }
    Some(Shortcut::new(Some(mods), code?))
}

fn parse_code(p: &str) -> Option<Code> {
    let lower = p.to_ascii_lowercase();
    match lower.as_str() {
        "space" => return Some(Code::Space),
        "enter" | "return" => return Some(Code::Enter),
        "tab" => return Some(Code::Tab),
        "backspace" => return Some(Code::Backspace),
        "esc" | "escape" => return Some(Code::Escape),
        "up" => return Some(Code::ArrowUp),
        "down" => return Some(Code::ArrowDown),
        "left" => return Some(Code::ArrowLeft),
        "right" => return Some(Code::ArrowRight),
        // PR4：QA 快捷键 Cmd+Shift+; 等标点（keyboard-types Code）。
        ";" | "semicolon" => return Some(Code::Semicolon),
        "'" | "quote" => return Some(Code::Quote),
        "[" => return Some(Code::BracketLeft),
        "]" => return Some(Code::BracketRight),
        "," => return Some(Code::Comma),
        "." => return Some(Code::Period),
        "/" => return Some(Code::Slash),
        "=" => return Some(Code::Equal),
        "-" | "minus" => return Some(Code::Minus),
        _ => {}
    }
    if let Some(c) = lower.chars().next() {
        if lower.len() == 1 && c.is_ascii_lowercase() {
            let code = match c {
                'a' => Code::KeyA,
                'b' => Code::KeyB,
                'c' => Code::KeyC,
                'd' => Code::KeyD,
                'e' => Code::KeyE,
                'f' => Code::KeyF,
                'g' => Code::KeyG,
                'h' => Code::KeyH,
                'i' => Code::KeyI,
                'j' => Code::KeyJ,
                'k' => Code::KeyK,
                'l' => Code::KeyL,
                'm' => Code::KeyM,
                'n' => Code::KeyN,
                'o' => Code::KeyO,
                'p' => Code::KeyP,
                'q' => Code::KeyQ,
                'r' => Code::KeyR,
                's' => Code::KeyS,
                't' => Code::KeyT,
                'u' => Code::KeyU,
                'v' => Code::KeyV,
                'w' => Code::KeyW,
                'x' => Code::KeyX,
                'y' => Code::KeyY,
                'z' => Code::KeyZ,
                _ => return None,
            };
            return Some(code);
        }
        if let Some(d) = c.to_digit(10) {
            if lower.len() == 1 {
                let code = match d {
                    0 => Code::Digit0,
                    1 => Code::Digit1,
                    2 => Code::Digit2,
                    3 => Code::Digit3,
                    4 => Code::Digit4,
                    5 => Code::Digit5,
                    6 => Code::Digit6,
                    7 => Code::Digit7,
                    8 => Code::Digit8,
                    _ => Code::Digit9,
                };
                return Some(code);
            }
        }
    }
    // F1-F12
    if let Some(n) = lower.strip_prefix('f') {
        let code = match n {
            "1" => Code::F1,
            "2" => Code::F2,
            "3" => Code::F3,
            "4" => Code::F4,
            "5" => Code::F5,
            "6" => Code::F6,
            "7" => Code::F7,
            "8" => Code::F8,
            "9" => Code::F9,
            "10" => Code::F10,
            "11" => Code::F11,
            "12" => Code::F12,
            _ => return None,
        };
        return Some(code);
    }
    None
}

/// Fn 键边沿回调（NSEvent monitor 线程上下文）。
/// 始终向前端推送事件（供测试模块显示）；按下 → 开始录音（防抖），松开 → 停止录音。
fn on_fn_edge(pressed: bool) {
    log_info!("Fn 键{}", if pressed { "按下" } else { "抬起" });

    // 推送事件给前端（测试模块用）。
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit("fn://edge", pressed);
    }
    let Some(app) = APP_HANDLE.get() else {
        return;
    };

    // Fn 释放的 300ms 尾部延时与按下的"继续说" отмены共用一个代数；分别声明成两个静态会导致不共享。
    static STOP_GEN: AtomicU64 = AtomicU64::new(0);
    if pressed {
        // 松开的尾部延时如果还在 sleep，说明用户想继续说 —— 取消该次待停。
        STOP_GEN.fetch_add(1, Ordering::SeqCst);
        // 300ms 防抖。
        static LAST_TRIGGER_MS: AtomicU64 = AtomicU64::new(0);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last = LAST_TRIGGER_MS.load(Ordering::SeqCst);
        if now_ms.saturating_sub(last) < 300 {
            return;
        }
        LAST_TRIGGER_MS.store(now_ms, Ordering::SeqCst);
        // A1 Hold（按住说话）模式：已在录音时，press 不重复触发（只开始，不切换停）。
        let hold = app
            .state::<AppState>()
            .config
            .try_read()
            .map(|c| c.hotkey_mode == voice_core::HotkeyMode::Hold)
            .unwrap_or(false);
        if hold && *app.state::<AppState>().recording.blocking_read() {
            return;
        }
        // 与全局录音快捷键同一条入口（QA 窗可见时走 QA 录音）。
        on_record_hotkey(app);
    } else {
        // 松开：延后 300ms 再停，保留一点尾音，避免用户刚说完的最后一个字被切掉。
        // 若期间用户又按下（想继续说），则该次待停被按下分支的 STOP_GEN 累加作废。
        let gen = STOP_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        let state = app.state::<AppState>().clone();
        let app_for_processing = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            // 期间若已再次按下，gen 已过期，不再停。
            if STOP_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let _ = app_for_processing.emit("recording://processing", "正在识别…");
            state.request_stop();
        });
    }
}

/// 切换录音 + 显示 overlay（快捷键与 Fn 共用入口）。
/// 意图读 `pending_intent`（听写 / 翻译 / QA），由 toggle_recording 在抢到 guard 后 take。
fn trigger_toggle(app: &tauri::AppHandle) {
    log_info!("录音快捷键触发");
    let intent = match app.state::<AppState>().pending_intent.lock() {
        Ok(g) => *g,
        Err(poisoned) => *poisoned.into_inner(),
    };
    // 必须在 overlay 显示前记录前台 app：随后立刻还焦，录音过程中 caret 不消失；
    // 录音结束再次还焦，保证 enigo 插入到用户输入框。
    // QA 不改 frontmost（开窗时已冻结），也无需还焦。
    let frontmost = match intent {
        voice_core::SessionIntent::Qa => None,
        _ => crate::platform::current::fn_key::frontmost_bundle_id(),
    };
    let app_clone = app.clone();
    let frontmost_for_cmd = frontmost.clone();
    // 仅在「即将开始录音」时显示 HUD；已在录音中则由松开/停止路径处理，避免闪一下。
    let already = app
        .try_state::<AppState>()
        .map(|s| *s.recording.blocking_read())
        .unwrap_or(false);
    if !already {
        show_overlay(app, frontmost.as_deref());
        let _ = app.emit("recording://started", ());
        // 意图对应的 HUD 起始文案（overlay 的 processing 通道）。
        match intent {
            voice_core::SessionIntent::Translate => {
                let _ = app.emit("recording://processing", "正在聆听（翻译）…");
            }
            voice_core::SessionIntent::Qa => {
                let _ = app.emit("recording://processing", "问答录音中…");
            }
            voice_core::SessionIntent::Dictate => {}
        }
    }
    tauri::async_runtime::spawn(async move {
        let state = app_clone.state::<AppState>();
        match commands::toggle_recording(app_clone.clone(), state.clone(), frontmost_for_cmd).await
        {
            Ok(started) => {
                log_info!("toggle_recording 结果：started={started}");
                if !started {
                    // 已在录音 → 请求停止：提示识别中，HUD 保持到文字上屏。
                    let _ = app_clone.emit("recording://processing", "正在识别…");
                }
            }
            Err(e) => {
                log_error!("toggle_recording 失败：{e}");
                // 启动失败：收起 HUD，避免残留。
                if let Some(win) = app_clone.get_webview_window("overlay") {
                    #[cfg(target_os = "macos")]
                    {
                        if let Ok(ns) = win.ns_window() {
                            crate::platform::current::fn_key::hide_window_without_activating(ns);
                        } else {
                            let _ = win.hide();
                        }
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        let _ = win.hide();
                    }
                }
                let _ = app_clone.emit("recording://error", e.to_string());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortcut_supports_semicolon_qa_hotkey() {
        // PR4：Cmd+Shift+;（QA 默认 placeholder）必须可解析。
        let sc = parse_shortcut("Cmd+Shift+;").expect("应能解析 Cmd+Shift+;");
        assert_eq!(sc.key, Code::Semicolon);
        assert!(sc.mods.contains(Modifiers::SUPER));
        assert!(sc.mods.contains(Modifiers::SHIFT));
        // 别名 semicolon 同样可解析。
        let sc2 = parse_shortcut("Cmd+Shift+semicolon").expect("semicolon 别名");
        assert_eq!(sc2.key, Code::Semicolon);
    }

    #[test]
    fn parse_shortcut_supports_punctuation_codes() {
        for code in ["'", "[", "]", ",", ".", "/", "=", "-"] {
            assert!(parse_shortcut(&format!("Cmd+Shift+{code}")).is_some(), "{code} 应可解析");
        }
    }

    #[test]
    fn parse_shortcut_rejects_unknown() {
        assert!(parse_shortcut("Cmd+Shift+不存在").is_none());
    }
}
