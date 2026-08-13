//! R11：TSF 命名管道协议（纯函数，跨平台可单测）。
//!
//! JSONL 一行一条，UTF-8 无 BOM；`type` 驼峰，`status` 小写。
//! Rust：`tag=type` + `rename_all=camelCase` + `rename_all_fields=camelCase`。
//!
//! 本模块的类型/常量由 Windows FFI（阶段 B）消费；纯函数层在 macOS 构建中不引用，
//! 故模块级允许 dead_code（测试仍完整覆盖协议 roundtrip）。
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// openIME TSF profile 语言（简体中文）。
/// 以下 GUID / 常量由 Windows FFI（阶段 A/B）消费；纯协议层暂未引用，故允许未使用。
#[allow(dead_code)]
pub const OPENIME_TSF_LANG_ID: u16 = 0x0804;
#[allow(dead_code)]
pub const OPENIME_TEXT_SERVICE_CLSID: &str = "{3F8A1C2E-9B47-4D61-8E2A-71C0F4D59B13}";
#[allow(dead_code)]
pub const OPENIME_PROFILE_GUID: &str = "{B6D24E91-0C53-4A8F-9E17-2A5D8C3F1B40}";
pub const OPENIME_IME_PIPE_PREFIX: &str = r"\\.\pipe\OpenImeCommit";
#[allow(dead_code)]
pub const OPENIME_IME_PROTOCOL_VERSION: u32 = 1;

/// 提交结果（status 小写）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImeSubmitStatus {
    Committed,
    Rejected,
    Failed,
}

/// 提交错误码闭集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImeErrorCode {
    Timeout,
    NoDocument,
    Rejected,
    TooLarge,
    Protocol,
}

/// 协议消息（黄金 fixture 四类）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ImeProtocolMessage {
    ClientReady {
        protocol_version: u32,
        process_id: u32,
        thread_id: u32,
    },
    SubmitText {
        protocol_version: u32,
        session_id: String,
        text: String,
    },
    SubmitResult {
        protocol_version: u32,
        session_id: String,
        status: ImeSubmitStatus,
        error_code: Option<ImeErrorCode>,
    },
    Ping {
        protocol_version: u32,
    },
}

/// `Committed` 才不回退；`Rejected` / `Failed` 都回退 P1 R7。
pub fn should_fallback_after_ime(status: ImeSubmitStatus) -> bool {
    !matches!(status, ImeSubmitStatus::Committed)
}

/// 目标进程管道名：`OpenImeCommit-{pid}-{tid}`（精确 tid 优先）。
pub fn ime_pipe_name_for_target(pid: u32, tid: u32) -> String {
    format!("{OPENIME_IME_PIPE_PREFIX}-{pid}-{tid}")
}

/// `IMAGE_FILE_MACHINE_AMD64`。
pub const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;

/// A11.10：只有 AMD64 前台进程走 TSF；I386 / ARM64 / 未知（None）→ R7。
pub fn tsf_supported_for_machine(machine: Option<u16>) -> bool {
    matches!(machine, Some(IMAGE_FILE_MACHINE_AMD64))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/windows-ime/fixtures");

    #[test]
    fn golden_fixtures_roundtrip() {
        // A11.9：黄金 fixture 4 条 roundtrip。
        let cases: Vec<(&str, ImeProtocolMessage)> = vec![
            (
                "client_ready.json",
                ImeProtocolMessage::ClientReady {
                    protocol_version: 1,
                    process_id: 1234,
                    thread_id: 5678,
                },
            ),
            (
                "submit_text.json",
                ImeProtocolMessage::SubmitText {
                    protocol_version: 1,
                    session_id: "s1".into(),
                    text: "你好".into(),
                },
            ),
            (
                "submit_result.json",
                ImeProtocolMessage::SubmitResult {
                    protocol_version: 1,
                    session_id: "s1".into(),
                    status: ImeSubmitStatus::Committed,
                    error_code: None,
                },
            ),
            (
                "ping.json",
                ImeProtocolMessage::Ping { protocol_version: 1 },
            ),
        ];
        for (file, expected) in cases {
            let json = std::fs::read_to_string(format!("{FIXTURE_DIR}/{file}"))
                .unwrap_or_else(|e| panic!("读 fixture {file} 失败：{e}"));
            let parsed: ImeProtocolMessage =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("解析 {file} 失败：{e}"));
            assert_eq!(parsed, expected, "{file} 反序列化不符");
            // 序列化再解析回相同值（roundtrip）。
            let re_serialized = serde_json::to_string(&parsed).unwrap();
            let back: ImeProtocolMessage = serde_json::from_str(&re_serialized).unwrap();
            assert_eq!(back, parsed, "{file} roundtrip 失败");
        }
    }

    #[test]
    fn protocol_type_field_is_camel_case() {
        // A11.5：type 驼峰。
        let msg = ImeProtocolMessage::ClientReady {
            protocol_version: 1,
            process_id: 1,
            thread_id: 2,
        };
        let s = serde_json::to_string(&msg).unwrap();
        assert!(s.contains("\"type\":\"clientReady\""), "{s}");
        assert!(s.contains("\"protocolVersion\":1"), "{s}");
        assert!(s.contains("\"processId\":1"), "{s}");
        assert!(s.contains("\"threadId\":2"), "{s}");
    }

    #[test]
    fn pipe_name_contains_pid_tid() {
        // A11.5：管道名含 pid-tid。
        assert_eq!(
            ime_pipe_name_for_target(1234, 5678),
            "\\\\.\\pipe\\OpenImeCommit-1234-5678"
        );
    }

    #[test]
    fn fallback_decision_matches_spec() {
        // A11.4：Rejected/Failed 回退，Committed 不回退。
        assert!(!should_fallback_after_ime(ImeSubmitStatus::Committed));
        assert!(should_fallback_after_ime(ImeSubmitStatus::Rejected));
        assert!(should_fallback_after_ime(ImeSubmitStatus::Failed));
    }

    #[test]
    fn tsf_supported_only_for_amd64() {
        // A11.10：AMD64 → TSF；I386 / ARM64 / 未知 → R7。
        assert!(tsf_supported_for_machine(Some(0x8664)));
        assert!(!tsf_supported_for_machine(Some(0x014c))); // I386
        assert!(!tsf_supported_for_machine(Some(0xaa64))); // ARM64
        assert!(!tsf_supported_for_machine(None)); // HWND(0) → None
    }

    #[test]
    fn stale_session_id_ignored_by_caller() {
        // stale sessionId 由调用方忽略（继续等匹配 id 或超时）；这里锁协议能解析不同 id。
        let json = r#"{"type":"submitResult","protocolVersion":1,"sessionId":"other","status":"committed","errorCode":null}"#;
        let parsed: ImeProtocolMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            ImeProtocolMessage::SubmitResult {
                protocol_version: 1,
                session_id: "other".into(),
                status: ImeSubmitStatus::Committed,
                error_code: None,
            }
        );
    }

    #[test]
    fn error_code_snake_case_roundtrip() {
        let json = r#"{"type":"submitResult","protocolVersion":1,"sessionId":"s1","status":"failed","errorCode":"too_large"}"#;
        let parsed: ImeProtocolMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            ImeProtocolMessage::SubmitResult {
                protocol_version: 1,
                session_id: "s1".into(),
                status: ImeSubmitStatus::Failed,
                error_code: Some(ImeErrorCode::TooLarge),
            }
        );
        assert_eq!(serde_json::to_string(&parsed).unwrap().contains("too_large"), true);
    }
}
