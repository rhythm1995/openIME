//! openIME Tauri 薄壳：只做 IPC 命令包装、插件注册、托盘/快捷键。
//! 所有业务逻辑都在 voice-core。

mod commands;
mod logging;
mod platform;
mod state;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{Menu, MenuItem};
use tauri::{Emitter, Manager};
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
            let state = AppState::new(store, sherpa_root).map_err(|e| {
                log_error!("初始化状态失败：{e}");
                anyhow::anyhow!("初始化状态失败: {e}")
            })?;
            let hotkey = state.config.blocking_read().hotkey.clone();
            app.manage(state);
            log_info!("AppState 初始化完成");

            // Fn 键监听供原生回调取用（块无捕获，只能走全局句柄）。
            let _ = APP_HANDLE.set(app.handle().clone());

            // 托盘菜单（失败不阻塞启动：菜单栏 App 至少要能跑）。
            let show_main = MenuItem::with_id(app, "show_main", "设置/历史", true, None::<&str>)?;
            let history = MenuItem::with_id(app, "history", "历史记录", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 openIME", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_main, &history, &quit])?;
            log_info!("托盘菜单已创建");
            // 菜单栏图标：用单色 template image（声波剪影），macOS 会随明暗模式自动反色。
            // 失败则退回 default_window_icon（彩色 app icon），最坏退回无图标。
            let tray_icon: Option<tauri::image::Image> =
                match tauri::image::Image::from_bytes(include_bytes!("../icons/menubar-template@2x.png")) {
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
            let mut tray_builder = tauri::tray::TrayIconBuilder::with_id("main-tray")
                .tooltip("openIME")
                // template image：macOS 自动按状态栏明暗反色（白底显黑、黑底显白）。
                .icon_as_template(true)
                .show_menu_on_left_click(true)
                .menu(&menu)
                .on_menu_event(|app, event| {
                    log_info!("托盘菜单点击：{}", event.id.as_ref());
                    match event.id.as_ref() {
                        "show_main" => show_main_window(app),
                        "history" => {
                            show_main_window(app);
                            let _ = app.emit("nav://goto", "history");
                        }
                        "quit" => {
                            log_info!("收到退出指令");
                            app.exit(0);
                        }
                        _ => {}
                    }
                });
            if let Some(ic) = tray_icon {
                tray_builder = tray_builder.icon(ic);
            }
            match tray_builder.build(app) {
                Ok(_) => log_info!("托盘创建成功"),
                Err(e) => log_warn!("托盘创建失败（忽略，继续启动）：{e}"),
            }

            // 录音快捷键（默认 Fn；设置里可改，保存后立即生效）。
            apply_hotkey(app.handle(), &hotkey);

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
            commands::create_session,
            commands::save_utterance,
            commands::list_sessions,
            commands::list_utterances,
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
            commands::frontend_log,
            commands::set_launch_at_login,
            commands::get_launch_at_login,
            commands::get_local_model_status,
            commands::install_local_model,
        ])
        .build(tauri::generate_context!())
        .expect("构建 Tauri 应用失败");

    log_info!("进入事件循环");

    // 设为 Accessory：不显示 Dock 图标、不抢前台焦点。
    // 这样 overlay show 不会把 openIME 激活到前台，用户的目标输入框保持焦点。
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    log_info!("已设置为 Accessory 激活策略");
    app.run(|app_handle, event| {
        // Dock 图标点击：若当前没有可见窗口，重新显示主面板。
        if let tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } = event
        {
            log_info!("Reopen（Dock 点击）：has_visible_windows={has_visible_windows}");
            if !has_visible_windows {
                show_main_window(app_handle);
            }
        }
    });
    log_info!("Tauri 事件循环结束，进程退出");
}

/// 全局快捷键（插件路径）：切换录音 + 显示 overlay。
fn on_hotkey(app: &tauri::AppHandle, _shortcut: &Shortcut) {
    trigger_toggle(app);
}

fn show_overlay(app: &tauri::AppHandle) {
    match app.get_webview_window("overlay") {
        Some(win) => {
            // 不抢焦点：overlay 仅作显示，不激活应用。
            // macOS 上 show() 默认会激活窗口；用 set_ignore_cursor_events 让窗口
            // 不接受鼠标事件（纯展示），减少对前台应用的干扰。
            let _ = win.set_ignore_cursor_events(true);

            // 定位到屏幕左下角（不引人注意）。
            if let Ok(monitor) = win.current_monitor() {
                if let Some(monitor) = monitor {
                    let size = monitor.size();
                    let scale = monitor.scale_factor();
                    let logical_h = size.height as f64 / scale;
                    let win_h = win.outer_size().map(|s| s.height as f64).unwrap_or(36.0);
                    let y = logical_h - win_h - 60.0;
                    let _ = win.set_position(tauri::Position::Logical(
                        tauri::LogicalPosition::new(16.0, y.max(0.0)),
                    ));
                }
            }
            if let Err(e) = win.show() {
                log_error!("overlay show 失败：{e}");
            }
            log_debug!("overlay 已显示（左下角，不抢焦点）");
        }
        None => log_warn!("overlay 窗口不存在"),
    }
}

fn show_main_window(app: &tauri::AppHandle) {
    match app.get_webview_window("main") {
        Some(win) => {
            // Accessory 模式下显示主窗口需临时切回 Regular 才能激活。
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
            if let Err(e) = win.show() {
                log_error!("main show 失败：{e}");
            }
            let _ = win.set_focus();
            log_info!("main 窗口已显示");
        }
        None => log_warn!("main 窗口不存在"),
    }
}

// ──────────────── 录音快捷键 ────────────────

/// 应用快捷键配置："Fn" 走原生监听；其他走 global-shortcut 插件。
/// 先清掉旧的插件快捷键，避免改键后残留。
fn apply_hotkey(app: &tauri::AppHandle, hotkey: &str) {
    let trimmed = hotkey.trim();
    if trimmed.eq_ignore_ascii_case("fn") {
        // Fn 是修饰键，global-shortcut 无法注册，用原生 NSEvent 监听。
        if let Err(e) = app.global_shortcut().unregister_all() {
            log_debug!("unregister_all 失败（忽略）：{e}");
        }
        crate::platform::current::fn_key::install_fn_monitor(on_fn_edge);
        log_info!("录音快捷键：Fn（原生监听）");
        return;
    }
    match parse_shortcut(trimmed) {
        Some(sc) => match app.global_shortcut().register(sc) {
            Ok(_) => log_info!("全局快捷键已注册：{trimmed}"),
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

    if pressed {
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
        trigger_toggle(app);
    } else {
        // 松开：立即停止录音（离线模式在此刻触发解码；实时模式结束流式识别）。
        let state = app.state::<AppState>().clone();
        tauri::async_runtime::spawn(async move {
            state.request_stop();
        });
    }
}

/// 切换录音 + 显示 overlay（快捷键与 Fn 共用入口）。
fn trigger_toggle(app: &tauri::AppHandle) {
    log_info!("录音快捷键触发");
    // 必须在 overlay 显示前记录前台 app：overlay 一显示就会抢走键盘焦点，
    // 录音结束后要把焦点还回原 app，enigo 才能插入到用户输入框。
    let frontmost = crate::platform::current::fn_key::frontmost_bundle_id();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app_clone.state::<AppState>();
        match commands::toggle_recording(app_clone.clone(), state.clone(), frontmost).await {
            Ok(started) => log_info!("toggle_recording 结果：started={started}"),
            Err(e) => log_error!("toggle_recording 失败：{e}"),
        }
    });
    show_overlay(app);
}
