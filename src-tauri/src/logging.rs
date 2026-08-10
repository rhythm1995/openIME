//! 日志模块：把启动、运行与崩溃信息持久化到文件，便于排障。
//!
//! 设计要点：
//! - 在 Tauri Builder 构造之前初始化（自行推算 app_data_dir），
//!   因此连 setup 阶段的崩溃也能记录下来。
//! - 按天滚动：`logs/openime-YYYY-MM-DD.log`，自动清理 7 天前的日志。
//! - 同时镜像到 stderr（终端运行时可直接看到）。
//! - 全局 panic hook：把 panic 位置、消息与 backtrace 写入日志（崩溃日志）。
//! - 前端通过 `frontend_log` 命令把 JS 日志/错误转发到这里。
//!
//! 日志目录（macOS）：`~/Library/Application Support/com.openime.desktop/logs/`

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Local;

/// 与 tauri.conf.json 的 identifier 保持一致。
const IDENTIFIER: &str = "com.openime.desktop";
/// 日志保留天数。
const RETENTION_DAYS: i64 = 7;
/// 日志文件前缀。
const FILE_PREFIX: &str = "openime-";

struct Inner {
    /// 当前打开的日志文件对应的日期（YYYY-MM-DD）。
    date: String,
    file: File,
}

static STATE: OnceLock<Mutex<Option<Inner>>> = OnceLock::new();
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 初始化日志系统：打开当天日志文件、安装 panic hook、清理过期日志。
///
/// 幂等：重复调用不会重复安装。返回日志目录（用于展示给用户）。
pub fn init() -> PathBuf {
    let dir = LOG_DIR.get_or_init(log_dir);
    let _ = fs::create_dir_all(dir);

    STATE.get_or_init(|| {
        install_panic_hook();
        cleanup_old_logs(dir);
        let date = Local::now().format("%Y-%m-%d").to_string();
        match open_log_file(dir, &date) {
            Ok(file) => {
                let mut inner = Inner { date, file };
                let _ = writeln!(
                    inner.file,
                    "\n===== openIME 启动 pid={} =====",
                    std::process::id()
                );
                Mutex::new(Some(inner))
            }
            Err(e) => {
                eprintln!("[openIME] 无法打开日志文件：{e}");
                Mutex::new(None)
            }
        }
    });

    dir.clone()
}

/// 日志目录：优先 app_data_dir/logs，兜底系统临时目录。
fn log_dir() -> PathBuf {
    if let Some(home) = dirs_home() {
        // macOS: ~/Library/Application Support/<identifier>/logs
        #[cfg(target_os = "macos")]
        {
            return home
                .join("Library/Application Support")
                .join(IDENTIFIER)
                .join("logs");
        }
        #[cfg(not(target_os = "macos"))]
        {
            return home.join(".openime").join("logs");
        }
    }
    std::env::temp_dir().join("openime-logs")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn open_log_file(dir: &Path, date: &str) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{FILE_PREFIX}{date}.log")))
}

/// 清理超过保留期的日志文件。失败静默（不影响主流程）。
fn cleanup_old_logs(dir: &Path) {
    let today = Local::now().date_naive();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(date_str) = name
            .strip_prefix(FILE_PREFIX)
            .and_then(|rest| rest.strip_suffix(".log"))
        else {
            continue;
        };
        let Ok(file_date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
            continue;
        };
        if (today - file_date).num_days() > RETENTION_DAYS {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// 写一条日志（level: DEBUG/INFO/WARN/ERROR）。
pub fn write(level: &str, message: &str) {
    let now = Local::now();
    let ts = now.format("%Y-%m-%d %H:%M:%S%.3f");
    let line = format!("{ts} [{level:<5}] {message}");

    // stderr 镜像：终端运行时可见。
    eprintln!("{line}");

    let Some(state) = STATE.get() else {
        return; // init() 尚未调用或失败
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    let date = now.format("%Y-%m-%d").to_string();
    // 跨天滚动：日期变化时重新打开文件。
    let need_reopen = match guard.as_ref() {
        Some(inner) => inner.date != date,
        None => true,
    };
    if need_reopen {
        if let Some(dir) = LOG_DIR.get() {
            match open_log_file(dir, &date) {
                Ok(file) => *guard = Some(Inner { date, file }),
                Err(_) => return,
            }
        }
    }
    if let Some(inner) = guard.as_mut() {
        let _ = writeln!(inner.file, "{line}");
    }
}

/// 安装全局 panic hook：panic 时记录位置、消息与 backtrace（崩溃日志）。
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".into());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let backtrace = std::backtrace::Backtrace::force_capture();
        write(
            "ERROR",
            &format!("!!! PANIC at {location}: {payload}\n{backtrace}"),
        );
        default(info);
    }));
}

/// 便捷宏：用法与 println! 一致。
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => { $crate::logging::write("DEBUG", &format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::logging::write("INFO", &format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::logging::write("WARN", &format!($($arg)*)) };
}
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::logging::write("ERROR", &format!($($arg)*)) };
}
