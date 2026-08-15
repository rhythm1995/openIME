//! P1：LLM 调用面 —— 润色 / 翻译 / 流式问答统一 trait。
//!
//! - [`LlmClient::polish`]：单次润色（max_tokens 按请求传入，默认 256）。
//! - [`LlmClient::translate_text`]：翻译（R4 / R5 翻译角色共用）。
//! - [`LlmClient::polish_and_translate`]：「先润色再翻译」哨兵合成调用（仅 R4）。
//! - [`LlmClient::chat_stream`]：OpenAI Chat SSE 流式（R6 QA；带 cancel）。
//!
//! 云端实现见 [`crate::polish::cloud::CloudPolishProvider`]；三种 `PolishCloudProtocol`
//! 全部复用 `post_json`（SSE 仅 OpenAI Chat）。

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::traits::{PolishRequest, PolishResponse};
use crate::Result;

/// 一次翻译请求（R4 / R5）。
#[derive(Debug, Clone)]
pub struct TranslateRequest {
    pub text: String,
    /// 已是 prompt 用名（如 "English"）；调用方先 `lang_display_name`。
    pub target_lang: String,
    /// 源语言短码（`zh`/`en`/`auto`…）；本地专翻模板用，云端忽略。
    pub source_lang: String,
    pub timeout: Duration,
    pub max_tokens: u32,
}

/// 「先润色再翻译」合成结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolishTranslate {
    pub polished: String,
    pub translation: String,
}

/// 一次流式问答请求（R6）。
pub struct ChatRequest {
    /// (role, content) 消息列表；首轮含选区信封。
    pub messages: Vec<(String, String)>,
    pub timeout: Duration,
    pub max_tokens: u32,
    /// 取消标志：置 true 后流在下个网络事件点尽快中断。
    pub cancel: Arc<AtomicBool>,
    /// 代数：调用方 bump，用于丢弃过期 delta（A6.7）。
    pub gen: u64,
    /// 每收到一段增量调用一次。
    pub on_delta: Box<dyn Fn(&str) + Send>,
}

/// LLM 客户端：polish / translate / chat_stream。
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse>;
    async fn translate_text(&self, req: TranslateRequest) -> Result<String>;
    async fn polish_and_translate(&self, req: TranslateRequest) -> Result<PolishTranslate>;
    async fn chat_stream(&self, req: ChatRequest) -> Result<String>;
}

/// SSE 单行解析结果（纯函数，供 chat_stream 与单测复用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseLine {
    /// 内容增量（choices[0].delta.content / message.content）。
    Delta(String),
    /// 流结束（[DONE] 或 finish_reason 出现）。
    Done,
    /// 其它行（心跳、role、空行等）。
    Ignore,
}

/// 解析一行 SSE 数据（调用方已去掉 `data: ` 前缀、trim 过）。
/// 兼容三种 payload 形态：delta.content（流式）、message.content（一次性）、
/// 以及 qwen 类 reasoning_content 之外的字段直接忽略。
pub fn parse_sse_line(line: &str) -> SseLine {
    let line = line.trim();
    if line.is_empty() {
        return SseLine::Ignore;
    }
    if line.starts_with(':') {
        // SSE 注释 / 心跳。
        return SseLine::Ignore;
    }
    let json_str = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
    if json_str == "[DONE]" {
        return SseLine::Done;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return SseLine::Ignore;
    };
    if v.get("finish_reason").is_some() && v["finish_reason"].is_string() {
        return SseLine::Done;
    }
    if v["choices"][0]["finish_reason"].is_string()
        && v["choices"][0]["finish_reason"].as_str() != Some("")
    {
        return SseLine::Done;
    }
    // 内容增量：优先 delta.content（流式），回退 message.content（部分兼容实现）。
    let delta = v["choices"][0]["delta"]["content"]
        .as_str()
        .or_else(|| v["choices"][0]["message"]["content"].as_str());
    match delta {
        Some(s) if !s.is_empty() => SseLine::Delta(s.to_string()),
        _ => SseLine::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_delta_content() {
        let line = format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": "你好"}}]})
        );
        assert_eq!(parse_sse_line(&line), SseLine::Delta("你好".into()));
    }

    #[test]
    fn parse_done_marker() {
        assert_eq!(parse_sse_line("data: [DONE]"), SseLine::Done);
        assert_eq!(parse_sse_line("data:[DONE]"), SseLine::Done);
    }

    #[test]
    fn parse_finish_reason_done() {
        let line = format!(
            "data: {}",
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
        );
        assert_eq!(parse_sse_line(&line), SseLine::Done);
        // 顶层 finish_reason（部分实现）。
        let line = format!("data: {}", json!({"finish_reason": "stop"}));
        assert_eq!(parse_sse_line(&line), SseLine::Done);
    }

    #[test]
    fn parse_empty_delta_ignored() {
        let line = format!(
            "data: {}",
            json!({"choices": [{"delta": {"role": "assistant"}}]})
        );
        assert_eq!(parse_sse_line(&line), SseLine::Ignore);
        assert_eq!(parse_sse_line(""), SseLine::Ignore);
        assert_eq!(parse_sse_line(": keep-alive"), SseLine::Ignore);
        assert_eq!(parse_sse_line("data: {broken json"), SseLine::Ignore);
    }

    #[test]
    fn parse_message_content_fallback() {
        let line = format!(
            "data: {}",
            json!({"choices": [{"message": {"content": "整段"}}]})
        );
        assert_eq!(parse_sse_line(&line), SseLine::Delta("整段".into()));
    }

    #[test]
    fn multi_delta_fixture_concatenates_full_text() {
        // A6.1b：mock SSE fixture 推两条 delta，拼出全文。
        let l1 = format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": "这段函数"}}]})
        );
        let l2 = format!(
            "data: {}",
            json!({"choices": [{"delta": {"content": "的作用是…"}}]})
        );
        let mut full = String::new();
        for line in [&l1, &l2] {
            if let SseLine::Delta(d) = parse_sse_line(line) {
                full.push_str(&d);
            }
        }
        assert_eq!(full, "这段函数的作用是…");
    }

    #[test]
    fn chat_request_fields_default_constructible() {
        // 仅验证类型可组装（cancel / gen / on_delta 语义在薄壳状态机测试覆盖）。
        let _req = ChatRequest {
            messages: vec![("user".into(), "这段什么意思".into())],
            timeout: Duration::from_secs(60),
            max_tokens: 2048,
            cancel: Arc::new(AtomicBool::new(false)),
            gen: 1,
            on_delta: Box::new(|_| {}),
        };
    }
}
