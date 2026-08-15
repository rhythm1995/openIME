//! R11：命名管道 client（宿主侧，FFI 部分）。
//!
//! 角色固定：TIP = server（目标进程内），宿主 = client（设计 KD-11：宿主必须校验
//! `GetNamedPipeServerProcessId == 目标 pid`，防仿冒/连错）。连接时序（L756-768）：
//! 800ms 内 `WaitNamedPipe` + `CreateFile` 重试 → 读 `clientReady` → 提交/收结果。

use super::protocol::{
    ime_pipe_name_for_target, ImeErrorCode, ImeProtocolMessage, ImeSubmitStatus,
    OPENIME_IME_PROTOCOL_VERSION,
};

/// 连接 + clientReady 阶段的失败分类（映射 ImeErrorCode）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcConnectError {
    /// 800ms 内管道未出现 / 连接失败（DLL 未被激活或加载失败）。
    Timeout,
    /// server pid 与目标不符（仿冒/连错），立即断开。
    ServerPidMismatch,
    /// clientReady 格式/版本不符。
    Protocol,
    /// clientReady 里 processId 与目标不符。
    ReadyPidMismatch,
}

/// 一次已建立的会话连接（含句柄）。Drop 关闭。
pub struct IpcClient {
    handle: windows::Win32::Foundation::HANDLE,
}

/// 与 `ime_pipe_name_for_target(pid, tid)` 的管道建立连接并读取 clientReady。
/// 成功标准（KD-10）：读到 `clientReady` 且其 processId == pid（belt），
/// 且 `GetNamedPipeServerProcessId(handle) == pid`（suspenders）。
#[cfg(target_os = "windows")]
pub fn connect_and_wait_ready(
    pid: u32,
    tid: u32,
    deadline_ms: u64,
) -> Result<IpcClient, IpcConnectError> {
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    };
    use windows::Win32::System::Pipes::{GetNamedPipeServerProcessId, WaitNamedPipeW};

    let name: Vec<u16> = ime_pipe_name_for_target(pid, tid)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let started = Instant::now();
    loop {
        let pipe_ready = unsafe { WaitNamedPipeW(windows::core::PCWSTR(name.as_ptr()), 50) };
        let opened: Option<windows::Win32::Foundation::HANDLE> = pipe_ready
            .as_bool()
            .then(|| unsafe {
                CreateFileW(
                    windows::core::PCWSTR(name.as_ptr()),
                    (GENERIC_READ | GENERIC_WRITE).0,
                    Default::default(),
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    windows::Win32::Foundation::HANDLE(std::ptr::null_mut()),
                )
            })
            .and_then(|r| r.ok());
        if let Some(h) = opened {
            let mut server_pid = 0u32;
            if unsafe { GetNamedPipeServerProcessId(h, &mut server_pid) }.is_err() {
                unsafe { let _ = CloseHandle(h); };
                return Err(IpcConnectError::Protocol);
            }
            if server_pid != pid {
                unsafe { let _ = CloseHandle(h); };
                return Err(IpcConnectError::ServerPidMismatch);
            }
            let client = IpcClient { handle: h };
            // 等 clientReady（DLL 连接建立后立即写）。
            match client.read_message(deadline_ms.saturating_sub(
                started.elapsed().as_millis() as u64,
            )) {
                Some(ImeProtocolMessage::ClientReady { process_id, .. }) if process_id == pid => {
                    return Ok(client)
                }
                Some(ImeProtocolMessage::ClientReady { .. }) => {
                    return Err(IpcConnectError::ReadyPidMismatch)
                }
                _ => return Err(IpcConnectError::Protocol),
            }
        }
        if started.elapsed() >= Duration::from_millis(deadline_ms) {
            return Err(IpcConnectError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

impl IpcClient {
    /// 读一行 JSONL 并解码（带超时；EOF/超时/坏帧 → None）。
    #[cfg(target_os = "windows")]
    pub fn read_message(&self, timeout_ms: u64) -> Option<ImeProtocolMessage> {
        use std::time::{Duration, Instant};

        use windows::Win32::System::Pipes::PeekNamedPipe;
        use windows::Win32::Storage::FileSystem::ReadFile;

        let started = Instant::now();
        let mut pending = Vec::<u8>::new();
        let mut buf = [0u8; 8192];
        loop {
            // Peek + Read：Peek 判定「有数据」再 Read，避免永久阻塞。
            let mut avail = 0u32;
            let peek_ok = unsafe {
                PeekNamedPipe(self.handle, None, 0, None, Some(&mut avail), None)
            }
            .is_ok();
            if !peek_ok || avail == 0 {
                if started.elapsed() >= Duration::from_millis(timeout_ms) {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            let mut read = 0u32;
            let r = unsafe { ReadFile(self.handle, Some(&mut buf), Some(&mut read), None) };
            if r.is_err() || read == 0 {
                return None; // EOF / 断开
            }
            pending.extend_from_slice(&buf[..read as usize]);
            if let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = pending.drain(..pos).collect();
                let line = String::from_utf8_lossy(&line).into_owned();
                return serde_json::from_str(&line).ok();
            }
            if pending.len() > 64 * 1024 + 4096 {
                return None;
            }
        }
    }

    /// 写一行 JSONL（同步写，小帧直接完成）。
    #[cfg(target_os = "windows")]
    pub fn write_message(&self, msg: &ImeProtocolMessage) -> bool {
        use windows::Win32::Storage::FileSystem::WriteFile;
        let mut line = serde_json::to_string(msg).unwrap_or_default();
        line.push('\n');
        let mut written = 0u32;
        unsafe {
            WriteFile(
                self.handle,
                Some(line.as_bytes()),
                Some(&mut written),
                None,
            )
            .is_ok()
                && written as usize == line.len()
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for IpcClient {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

// ── 纯逻辑（跨平台单测）──

/// 提交并等待匹配 sessionId 的 SubmitResult；stale 帧由调用方忽略（NFR-11.2 注）。
/// 这里的纯函数只做「结果匹配 + 回退判定」的组合，供 session 层单测。
pub fn interpret_result(
    got: Option<ImeProtocolMessage>,
    session_id: &str,
) -> Result<ImeSubmitStatus, ImeErrorCode> {
    match got {
        Some(ImeProtocolMessage::SubmitResult {
            session_id: sid,
            status,
            error_code,
            ..
        }) => {
            if sid != session_id {
                return Err(ImeErrorCode::Protocol);
            }
            if status == ImeSubmitStatus::Committed {
                Ok(status)
            } else {
                Err(error_code.unwrap_or(ImeErrorCode::Protocol))
            }
        }
        _ => Err(ImeErrorCode::Protocol),
    }
}

/// SubmitText 帧（session 层用；单测锁 schema）。
pub fn submit_text_frame(session_id: &str, text: &str) -> ImeProtocolMessage {
    ImeProtocolMessage::SubmitText {
        protocol_version: OPENIME_IME_PROTOCOL_VERSION,
        session_id: session_id.to_string(),
        text: text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_result_maps_ok() {
        let ok = ImeProtocolMessage::SubmitResult {
            protocol_version: 1,
            session_id: "s1".into(),
            status: ImeSubmitStatus::Committed,
            error_code: None,
        };
        assert_eq!(interpret_result(Some(ok), "s1"), Ok(ImeSubmitStatus::Committed));
    }

    #[test]
    fn failed_result_maps_error_code() {
        let bad = ImeProtocolMessage::SubmitResult {
            protocol_version: 1,
            session_id: "s1".into(),
            status: ImeSubmitStatus::Failed,
            error_code: Some(ImeErrorCode::TooLarge),
        };
        assert_eq!(interpret_result(Some(bad), "s1"), Err(ImeErrorCode::TooLarge));
    }

    #[test]
    fn stale_or_wrong_session_is_protocol_error() {
        let stale = ImeProtocolMessage::SubmitResult {
            protocol_version: 1,
            session_id: "other".into(),
            status: ImeSubmitStatus::Committed,
            error_code: None,
        };
        assert_eq!(interpret_result(Some(stale), "s1"), Err(ImeErrorCode::Protocol));
        assert_eq!(interpret_result(None, "s1"), Err(ImeErrorCode::Protocol));
        assert_eq!(
            interpret_result(Some(ImeProtocolMessage::Ping { protocol_version: 1 }), "s1"),
            Err(ImeErrorCode::Protocol)
        );
    }

    #[test]
    fn submit_frame_is_wire_compatible() {
        let f = submit_text_frame("s1", "你好");
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains(r#""type":"submitText""#));
        assert!(json.contains(r#""sessionId":"s1""#));
        assert!(json.contains("你好"));
    }
}
