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

    /// 功能回路（真机，Stage B 宿主侧）：本进程内起一个 mock TIP 管道 server，
    /// 验证 IpcClient 全链路——WaitNamedPipe/CreateFile 连接、server pid 校验、
    /// clientReady 读取、SubmitText 写入、SubmitResult 匹配。
    /// mock server 与 client 同进程 → GetNamedPipeServerProcessId == 本 pid ✓。
    #[cfg(target_os = "windows")]
    #[test]
    fn ipc_client_loopback_with_mock_pipe_server() {
        // 管道读写走 Win32 API，无需 std::io trait。

        let server_tid_holder = std::sync::Arc::new(std::sync::Mutex::new(0u32));
        let ready = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server = {
            let tid_holder = server_tid_holder.clone();
            let ready = ready.clone();
            std::thread::Builder::new()
                .name("mock-ime-pipe".into())
                .spawn(move || {
                    use windows::Win32::Storage::FileSystem::ReadFile;
                    use windows::Win32::Storage::FileSystem::WriteFile;
                    use windows::Win32::System::Pipes::CreateNamedPipeW;

                    let tid = unsafe {
                        windows::Win32::System::Threading::GetCurrentThreadId()
                    };
                    *tid_holder.lock().unwrap() = tid;
                    let name = ime_pipe_name_for_target(std::process::id(), tid);
                    let name16: Vec<u16> = name
                        .encode_utf16()
                        .chain(std::iter::once(0))
                        .collect();
                    let pipe = unsafe {
                        CreateNamedPipeW(
                            windows::core::PCWSTR(name16.as_ptr()),
                            windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX,
                            windows::Win32::System::Pipes::PIPE_TYPE_BYTE
                                | windows::Win32::System::Pipes::PIPE_READMODE_BYTE,
                            1,
                            64 * 1024,
                            64 * 1024,
                            0,
                            None,
                        )
                    }
                    ;  // HANDLE（INVALID 由后续调用失败暴露）
                    ready.store(true, std::sync::atomic::Ordering::SeqCst);
                    // 等宿主连接（同步 ConnectNamedPipe）。
                    unsafe {
                        use windows::Win32::System::Pipes::ConnectNamedPipe;
                        let _ = ConnectNamedPipe(pipe, None);
                    }
                    let write_line = |line: &str| unsafe {
                        let mut written = 0u32;
                        WriteFile(pipe, Some(line.as_bytes()), Some(&mut written), None)
                            .is_ok()
                            && written as usize == line.len()
                    };
                    // 1) 连接即宣告 clientReady（processId = 本 pid）。
                    let ready_line = format!(
                        "{{\"type\":\"clientReady\",\"protocolVersion\":1,\"processId\":{},\"threadId\":{}}}\n",
                        std::process::id(),
                        tid
                    );
                    assert!(write_line(&ready_line), "clientReady 写入失败");
                    // 2) 读 SubmitText（同步 8KiB 一轮，找换行）。
                    let mut buf = [0u8; 8192];
                    let mut pending = Vec::new();
                    let submit_text = loop {
                        let mut got = 0u32;
                        assert!(
                            unsafe {
                                ReadFile(pipe, Some(&mut buf), Some(&mut got), None)
                            }
                            .is_ok()
                        );
                        pending.extend_from_slice(&buf[..got as usize]);
                        if let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                            let line: Vec<u8> = pending.drain(..pos).collect();
                            let line = String::from_utf8_lossy(&line).into_owned();
                            let msg: ImeProtocolMessage =
                                serde_json::from_str(&line).expect("宿主帧应可解析");
                            if matches!(msg, ImeProtocolMessage::SubmitText { .. }) {
                                break msg;
                            }
                        }
                    };
                    // 3) 回 committed（原 sessionId 透传）。
                    let ImeProtocolMessage::SubmitText { session_id, text, .. } =
                        submit_text
                    else {
                        unreachable!()
                    };
                    let result = ImeProtocolMessage::SubmitResult {
                        protocol_version: 1,
                        session_id,
                        status: ImeSubmitStatus::Committed,
                        error_code: None,
                    };
                    let mut line = serde_json::to_string(&result).unwrap();
                    line.push('\n');
                    assert!(write_line(&line), "submitResult 写入失败");
                    let _ = text; // mock 不消费文本
                    unsafe {
                        let _ = windows::Win32::Storage::FileSystem::FlushFileBuffers(pipe);
                        let _ =
                            windows::Win32::System::Pipes::DisconnectNamedPipe(pipe);
                        let _ = windows::Win32::Foundation::CloseHandle(pipe);
                    }
                })
                .expect("mock server 线程启动失败")
        };

        // 等 mock server 建管并记录 tid。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !ready.load(std::sync::atomic::Ordering::SeqCst) {
            assert!(std::time::Instant::now() < deadline, "mock server 未就绪");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let server_tid = *server_tid_holder.lock().unwrap();

        // 宿主侧：连接 + clientReady（pid=本 pid → 校验通过）。
        let client = connect_and_wait_ready(std::process::id(), server_tid, 2000)
            .expect("连接与 clientReady 应成功");
        // SubmitText → 读回 Committed。
        let sid = uuid::Uuid::new_v4().to_string();
        assert!(client.write_message(&submit_text_frame(&sid, "回路测试你好")));
        let mut got = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while got.is_none() && std::time::Instant::now() < deadline {
            if let Some(frame) = client.read_message(200) {
                got = Some(frame);
            }
        }
        let frame = got.expect("应读到 submitResult");
        assert_eq!(
            interpret_result(Some(frame), &sid),
            Ok(ImeSubmitStatus::Committed)
        );
        server.join().unwrap();
    }

    /// 错误 pid（server=本进程，谎报目标 pid）→ 连上后必须被 pid 校验拒绝。
    #[cfg(target_os = "windows")]
    #[test]
    fn ipc_client_rejects_server_pid_mismatch() {
        use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW};

        // 场景：仿冒者在「目标进程的管道名」下建管（fake_pid），但 server 实际是
        // 另一个进程（本测试进程）。宿主连上后 GetNamedPipeServerProcessId 不符 → 拒绝。
        let fake_pid = std::process::id().wrapping_add(1);
        let name = ime_pipe_name_for_target(fake_pid, 0xdead);
        let name16: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let pipe = unsafe {
            CreateNamedPipeW(
                windows::core::PCWSTR(name16.as_ptr()),
                windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX,
                windows::Win32::System::Pipes::PIPE_TYPE_BYTE,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        // HANDLE 非 Send：按 usize 桥接进线程。
        let pipe_as_usize = pipe.0 as usize;
        let acceptor = std::thread::spawn(move || unsafe {
            let pipe = windows::Win32::Foundation::HANDLE(pipe_as_usize as *mut _);
            let _ = ConnectNamedPipe(pipe, None);
            let _ = windows::Win32::Foundation::CloseHandle(pipe);
        });
        let r = connect_and_wait_ready(fake_pid, 0xdead, 1500);
        assert!(matches!(r, Err(IpcConnectError::ServerPidMismatch)));
        acceptor.join().unwrap();
    }

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
