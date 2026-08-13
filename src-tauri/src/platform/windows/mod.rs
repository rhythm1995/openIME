//! Windows 平台实现（R7 P1）：
//! - `focus`：前台进程 exe basename 捕获 + 按 exe 还焦（粘贴兜底 / 插入目标判断）。
//! - `fn_key`：兼容 macOS 薄壳调用面（frontmost_bundle_id 转发为 exe basename）。
//! - `permissions`：桩（Windows 授权模型不同，P1 不做自动检测）。
//!
//! 注意：本模块在 macOS 上不参与编译（cfg(target_os = "windows")），
//! Windows 构建由 CI 验证（GitHub Actions windows-latest）。

pub mod focus;
pub mod fn_key;
pub mod permissions;
