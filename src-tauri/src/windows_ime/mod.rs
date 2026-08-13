//! R11：Windows TSF 集成。
//!
//! 纯协议 / 决策函数（`protocol`、`profile`）跨平台可单测（NFR-11.2）；
//! Windows 专属 FFI（命名管道 client / 会话控制）后续按 `cfg(target_os = "windows")` 落地。

pub mod profile;
pub mod protocol;
