//! 云端润色：3 种 LLM 协议统一接入。
//!
//! - OpenAI Chat Completions（/chat/completions）— OpenAI / OpenRouter / 各兼容网关
//! - Anthropic Messages API（/v1/messages）— Claude
//! - OpenAI Responses API（/v1/responses）— OpenAI 新版
//!
//! 统一入口 [`CloudPolishProvider`]，按 `PolishCloudProtocol` 路由。
//! P1 实现 [`LlmClient`]：polish / translate_text / polish_and_translate 覆盖三种协议
//! （复用 `post_json`），`chat_stream`（QA SSE）仅 OpenAI Chat。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::json;

use crate::config::PolishCloudProtocol;
use crate::polish::llm::{
    parse_sse_line, ChatRequest, LlmClient, PolishTranslate, SseLine, TranslateRequest,
};
use crate::polish::prompts::{
    build_messages, build_polish_translate_messages, build_translate_messages,
    POLISHED_SOURCE_SENTINEL, TRANSLATION_SENTINEL,
};
use crate::traits::{PolishMode, PolishRequest, PolishResponse, TextPolishProvider};
use crate::{Error, Result};

/// 云端润色 provider（3 协议统一）。
pub struct CloudPolishProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub protocol: PolishCloudProtocol,
}

impl CloudPolishProvider {
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model: model.into(),
            protocol: PolishCloudProtocol::OpenAiChat,
        }
    }

    pub fn new_with_protocol(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        protocol: PolishCloudProtocol,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
            model: model.into(),
            protocol,
        }
    }
}

#[async_trait]
impl TextPolishProvider for CloudPolishProvider {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse> {
        if req.mode == PolishMode::Off || req.text.trim().is_empty() {
            return Ok(PolishResponse {
                text: req.text,
                provider: "passthrough".into(),
                latency_ms: 0,
            });
        }
        if self.api_key.trim().is_empty() {
            return Err(Error::Config("云端润色缺少 API Key（必填）".into()));
        }

        let t0 = Instant::now();
        let messages = build_messages(
            &req.text,
            req.mode,
            &req.hotwords,
            req.style_prompt.as_deref(),
        );

        let text = match self.protocol {
            PolishCloudProtocol::OpenAiChat => {
                self.polish_openai_chat(&messages, &req, req.max_tokens.unwrap_or(256))
                    .await?
            }
            PolishCloudProtocol::Anthropic => {
                self.polish_anthropic(&messages, &req, req.max_tokens.unwrap_or(256))
                    .await?
            }
            PolishCloudProtocol::OpenAiResponses => {
                self.polish_openai_responses(&messages, &req, req.max_tokens.unwrap_or(256))
                    .await?
            }
        };

        if text.is_empty() {
            return Err(Error::Llm("云端润色返回空文本".into()));
        }

        Ok(PolishResponse {
            text,
            provider: format!("cloud-{:?}", self.protocol).to_lowercase(),
            latency_ms: t0.elapsed().as_millis() as u32,
        })
    }
}

#[async_trait]
impl LlmClient for CloudPolishProvider {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse> {
        TextPolishProvider::polish(self, req).await
    }

    /// R4/R5：翻译。三种协议均走 post_json。
    async fn translate_text(&self, req: TranslateRequest) -> Result<String> {
        if self.api_key.trim().is_empty() {
            return Err(Error::Config("翻译需要云端 API Key（必填）".into()));
        }
        if req.text.trim().is_empty() {
            return Ok(String::new());
        }
        let messages = build_translate_messages(&req.text, &req.target_lang);
        let text = match self.protocol {
            PolishCloudProtocol::OpenAiChat => {
                self.chat_once_openai(&messages, req.timeout, req.max_tokens, 0.3)
                    .await?
            }
            PolishCloudProtocol::Anthropic => {
                self.chat_once_anthropic(&messages, req.timeout, req.max_tokens)
                    .await?
            }
            PolishCloudProtocol::OpenAiResponses => {
                self.chat_once_responses(&messages, req.timeout, req.max_tokens)
                    .await?
            }
        };
        if text.trim().is_empty() {
            return Err(Error::Llm("云端翻译返回空文本".into()));
        }
        Ok(text.trim().to_string())
    }

    /// R4：「先润色再翻译」哨兵合成调用。解析失败 → 回退纯 translate_text（FR-4.4）。
    async fn polish_and_translate(&self, req: TranslateRequest) -> Result<PolishTranslate> {
        if self.api_key.trim().is_empty() {
            return Err(Error::Config("翻译需要云端 API Key（必填）".into()));
        }
        if req.text.trim().is_empty() {
            return Err(Error::Llm("待翻译文本为空".into()));
        }
        let messages = build_polish_translate_messages(&req.text, &req.target_lang);
        let raw = match self.protocol {
            PolishCloudProtocol::OpenAiChat => {
                self.chat_once_openai(&messages, req.timeout, req.max_tokens, 0.3)
                    .await?
            }
            PolishCloudProtocol::Anthropic => {
                self.chat_once_anthropic(&messages, req.timeout, req.max_tokens)
                    .await?
            }
            PolishCloudProtocol::OpenAiResponses => {
                self.chat_once_responses(&messages, req.timeout, req.max_tokens)
                    .await?
            }
        };
        match parse_polish_translate(&raw) {
            Some(pt) => Ok(pt),
            None => {
                tracing::warn!("润色+翻译哨兵解析失败，回退纯翻译");
                Ok(PolishTranslate {
                    polished: req.text.clone(),
                    translation: self.translate_text(req).await?,
                })
            }
        }
    }

    /// R6：QA 流式。仅 OpenAI Chat（SSE）；Anthropic/Responses 返回错误。
    async fn chat_stream(&self, req: ChatRequest) -> Result<String> {
        if self.api_key.trim().is_empty() {
            return Err(Error::Config("问答需要云端 API Key（必填）".into()));
        }
        if self.protocol != PolishCloudProtocol::OpenAiChat {
            return Err(Error::Config(
                "问答流式（SSE）目前仅支持 OpenAI Chat 协议".into(),
            ));
        }
        self.chat_stream_openai(req).await
    }
}

impl CloudPolishProvider {
    /// 协议 1：OpenAI Chat Completions（润色）。
    async fn polish_openai_chat(
        &self,
        messages: &[(String, String)],
        req: &PolishRequest,
        max_tokens: u32,
    ) -> Result<String> {
        let body_msgs: Vec<_> = messages
            .iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect();
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = with_model(
            json!({
                "messages": body_msgs,
                "temperature": 0.3,
                "max_tokens": max_tokens,
            }),
            &self.model,
        );
        let raw = self
            .post_json(&url, &body, AuthType::Bearer, req.timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Llm(format!("解析 OpenAI Chat JSON 失败: {e}")))?;
        Ok(parse_openai_chat_text(&v))
    }

    /// 协议 2：Anthropic Messages API（润色）。
    async fn polish_anthropic(
        &self,
        messages: &[(String, String)],
        req: &PolishRequest,
        max_tokens: u32,
    ) -> Result<String> {
        // Anthropic 格式：system 独立，messages 只有 user/assistant。
        let (system_msg, chat_msgs) = split_system(messages);
        let url = anthropic_messages_url(&self.base_url);
        let body = with_model(
            json!({
                "max_tokens": max_tokens,
                "system": system_msg,
                "messages": chat_msgs,
            }),
            &self.model,
        );
        let raw = self
            .post_json(&url, &body, AuthType::Anthropic, req.timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Llm(format!("解析 Anthropic JSON 失败: {e}")))?;
        Ok(parse_anthropic_text(&v))
    }

    /// 协议 3：OpenAI Responses API（润色）。
    async fn polish_openai_responses(
        &self,
        messages: &[(String, String)],
        req: &PolishRequest,
        max_tokens: u32,
    ) -> Result<String> {
        // Responses API：instructions = system，input = user text。
        let (system_msg, chat_msgs) = split_system(messages);
        let user_text = chat_msgs
            .iter()
            .find(|m| m["role"].as_str() == Some("user"))
            .and_then(|m| m["content"].as_str())
            .unwrap_or("");
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let body = with_model(
            json!({
                "instructions": system_msg,
                "input": user_text,
                "temperature": 0.3,
                "max_output_tokens": max_tokens,
            }),
            &self.model,
        );
        let raw = self
            .post_json(&url, &body, AuthType::Bearer, req.timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Llm(format!("解析 Responses JSON 失败: {e}")))?;
        Ok(parse_responses_text(&v))
    }

    // ── P1：翻译 / QA 的一次性 chat（三协议）──

    /// OpenAI Chat 一次性调用（翻译 / 合成）。
    async fn chat_once_openai(
        &self,
        messages: &[(String, String)],
        timeout: Duration,
        max_tokens: u32,
        temperature: f64,
    ) -> Result<String> {
        let body_msgs: Vec<_> = messages
            .iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect();
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = with_model(
            json!({
                "messages": body_msgs,
                "temperature": temperature,
                "max_tokens": max_tokens,
            }),
            &self.model,
        );
        let raw = self
            .post_json(&url, &body, AuthType::Bearer, timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Llm(format!("解析 OpenAI Chat JSON 失败: {e}")))?;
        Ok(parse_openai_chat_text(&v))
    }

    /// Anthropic 一次性调用（翻译 / 合成）。
    async fn chat_once_anthropic(
        &self,
        messages: &[(String, String)],
        timeout: Duration,
        max_tokens: u32,
    ) -> Result<String> {
        let (system_msg, chat_msgs) = split_system(messages);
        let url = anthropic_messages_url(&self.base_url);
        let body = with_model(
            json!({
                "max_tokens": max_tokens,
                "system": system_msg,
                "messages": chat_msgs,
            }),
            &self.model,
        );
        let raw = self
            .post_json(&url, &body, AuthType::Anthropic, timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Llm(format!("解析 Anthropic JSON 失败: {e}")))?;
        Ok(parse_anthropic_text(&v))
    }

    /// Responses 一次性调用（翻译 / 合成）。
    async fn chat_once_responses(
        &self,
        messages: &[(String, String)],
        timeout: Duration,
        max_tokens: u32,
    ) -> Result<String> {
        let (system_msg, chat_msgs) = split_system(messages);
        let user_text = chat_msgs
            .iter()
            .find(|m| m["role"].as_str() == Some("user"))
            .and_then(|m| m["content"].as_str())
            .unwrap_or("");
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let body = with_model(
            json!({
                "instructions": system_msg,
                "input": user_text,
                "temperature": 0.3,
                "max_output_tokens": max_tokens,
            }),
            &self.model,
        );
        let raw = self
            .post_json(&url, &body, AuthType::Bearer, timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Llm(format!("解析 Responses JSON 失败: {e}")))?;
        Ok(parse_responses_text(&v))
    }

    /// R6：OpenAI Chat SSE 流式。逐行解析，`cancel` 置位即在下一事件点中断，
    /// 每段增量调 `on_delta`；返回完整文本。
    async fn chat_stream_openai(&self, req: ChatRequest) -> Result<String> {
        let body_msgs: Vec<_> = req
            .messages
            .iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect();
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = with_model(
            json!({
                "messages": body_msgs,
                "stream": true,
                "max_tokens": req.max_tokens,
            }),
            &self.model,
        );
        let client = crate::http::http_client_no_redirect(req.timeout);
        let resp = client
            .post(&url)
            .bearer_auth(self.api_key.trim())
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(format!("问答请求失败（POST {url}）: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let raw = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect::<String>();
            return Err(Error::Llm(format!(
                "问答 HTTP {status}（POST {url}）: {raw}"
            )));
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut full = String::new();
        let cancel: Arc<AtomicBool> = req.cancel;
        'outer: while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                tracing::info!("QA 流被取消");
                return Err(Error::Llm("已取消".into()));
            }
            let chunk = chunk.map_err(|e| Error::Llm(format!("读取 SSE 失败: {e}")))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(idx) = buf.find('\n') {
                let line: String = buf.drain(..=idx).collect();
                match parse_sse_line(&line) {
                    SseLine::Delta(d) => {
                        full.push_str(&d);
                        (req.on_delta)(&d);
                    }
                    SseLine::Done => break 'outer,
                    SseLine::Ignore => {}
                }
            }
        }
        Ok(full)
    }

    /// 通用 POST JSON（按 auth 类型设 header）。
    async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        auth: AuthType,
        timeout: std::time::Duration,
    ) -> Result<String> {
        let client = crate::http::http_client_no_redirect(timeout);
        let mut req_builder = client.post(url).json(body);
        req_builder = match auth {
            AuthType::Bearer => req_builder.bearer_auth(self.api_key.trim()),
            AuthType::Anthropic => {
                let (header, value) = anthropic_auth_header(self.api_key.trim());
                req_builder
                    .header(header, value)
                    .header("anthropic-version", "2023-06-01")
            }
        };
        let resp = req_builder
            .send()
            .await
            .map_err(|e| Error::Llm(format!("云端润色请求失败（POST {url}）: {e}")))?;
        let status = resp.status();
        let raw = resp
            .text()
            .await
            .map_err(|e| Error::Llm(format!("读取云端响应失败（POST {url}）: {e}")))?;
        if !status.is_success() {
            return Err(Error::Llm(format!(
                "云端润色 HTTP {status}（POST {url}）: {}",
                raw.chars().take(300).collect::<String>()
            )));
        }
        Ok(raw)
    }
}

enum AuthType {
    Bearer,
    Anthropic,
}

/// 云端模型 ID 可选：留空时不携带 model 字段（由服务端/网关决定）。
fn with_model(mut body: serde_json::Value, model: &str) -> serde_json::Value {
    let m = model.trim();
    if !m.is_empty() {
        body["model"] = serde_json::Value::String(m.to_string());
    }
    body
}

/// Anthropic Messages API 完整 URL：`{base}/v1/messages`。
/// base 已含 `/v1` 时不重复拼接（如用户直接填 `https://api.anthropic.com/v1`）。
/// 兼容 Claude Code 生态（如阿里云 token-plan 的
/// `https://…/apps/anthropic` 需要补 `/v1`）。
fn anthropic_messages_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    }
}

/// Anthropic 鉴权头选择：
/// - `sk-ant-` 前缀 = 官方 Anthropic API Key → `x-api-key`。
/// - 其它（如 `sk-sp-` Spruce 令牌 / Claude Code 网关）→ `Authorization: Bearer`。
fn anthropic_auth_header(api_key: &str) -> (&'static str, String) {
    if api_key.starts_with("sk-ant-") {
        ("x-api-key", api_key.to_string())
    } else {
        ("Authorization", format!("Bearer {api_key}"))
    }
}

/// 从 messages 分离 system 和 user/assistant（Anthropic 需要 system 独立字段）。
fn split_system(messages: &[(String, String)]) -> (String, Vec<serde_json::Value>) {
    let mut system = String::new();
    let mut chat = Vec::new();
    for (role, content) in messages {
        if role == "system" {
            system.push_str(content);
            system.push('\n');
        } else {
            chat.push(json!({ "role": role, "content": content }));
        }
    }
    (system.trim().to_string(), chat)
}

// ── 响应解析（纯函数，可单测）────────────────────────────────

/// OpenAI Chat：choices[0].message.content（string 或 array of text blocks）。
fn parse_openai_chat_text(v: &serde_json::Value) -> String {
    let content = &v["choices"][0]["message"]["content"];
    if let Some(s) = content.as_str() {
        return s.trim().to_string();
    }
    // content 可能是 array（content blocks）。
    content
        .as_array()
        .and_then(|arr| arr.iter().find_map(|b| b["text"].as_str()))
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Anthropic：{ content: [{ type: "text", text: "..." }] }。
fn parse_anthropic_text(v: &serde_json::Value) -> String {
    v["content"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|b| b["type"].as_str() == Some("text"))
                .and_then(|b| b["text"].as_str())
        })
        .unwrap_or("")
        .trim()
        .to_string()
}

/// OpenAI Responses：{ output: [{ content: [{ type: "output_text", text: "..." }] }] }。
fn parse_responses_text(v: &serde_json::Value) -> String {
    v["output"]
        .as_array()
        .and_then(|arr| {
            arr.iter().find_map(|o| {
                o["content"].as_array().and_then(|c| {
                    c.iter()
                        .find(|b| b["type"].as_str() == Some("output_text"))
                        .and_then(|b| b["text"].as_str())
                })
            })
        })
        .unwrap_or("")
        .trim()
        .to_string()
}

/// R4：解析「润色+翻译」哨兵输出。
/// 返回 None 表示格式不完整（调用方回退纯翻译）。
pub fn parse_polish_translate(raw: &str) -> Option<PolishTranslate> {
    let start = raw.find(POLISHED_SOURCE_SENTINEL)?;
    let rest = &raw[start + POLISHED_SOURCE_SENTINEL.len()..];
    let mid = rest.find(TRANSLATION_SENTINEL)?;
    let polished = rest[..mid].trim().to_string();
    let translation = rest[mid + TRANSLATION_SENTINEL.len()..].trim().to_string();
    if polished.is_empty() || translation.is_empty() {
        return None;
    }
    Some(PolishTranslate {
        polished,
        translation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_openai_chat_string_content() {
        let v = json!({"choices": [{"message": {"content": "你好。"}}]});
        assert_eq!(parse_openai_chat_text(&v), "你好。");
    }

    #[test]
    fn parse_openai_chat_array_content() {
        let v = json!({"choices": [{"message": {"content": [{"type": "text", "text": "你好"}]}}]});
        assert_eq!(parse_openai_chat_text(&v), "你好");
    }

    #[test]
    fn parse_anthropic_text_block() {
        let v = json!({"content": [{"type": "text", "text": "你好。"}]});
        assert_eq!(parse_anthropic_text(&v), "你好。");
    }

    #[test]
    fn parse_responses_output_text() {
        let v = json!({"output": [{"content": [{"type": "output_text", "text": "你好"}]}]});
        assert_eq!(parse_responses_text(&v), "你好");
    }

    #[test]
    fn parse_empty_responses() {
        assert_eq!(parse_anthropic_text(&json!({})), "");
        assert_eq!(parse_responses_text(&json!({})), "");
        assert_eq!(parse_openai_chat_text(&json!({})), "");
    }

    #[test]
    fn anthropic_messages_url_appends_v1() {
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        // Claude Code 生态（token-plan 等）：base 不含 /v1，同样补上。
        assert_eq!(
            anthropic_messages_url(
                "https://token-plan.ap-southeast-1.maas.aliyuncs.com/apps/anthropic"
            ),
            "https://token-plan.ap-southeast-1.maas.aliyuncs.com/apps/anthropic/v1/messages"
        );
        // 已含 /v1（含尾部斜杠）不重复拼接。
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com/v1"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.anthropic.com/v1/"),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn anthropic_auth_header_dispatches_by_prefix() {
        // 官方 Anthropic key → x-api-key。
        assert_eq!(
            anthropic_auth_header("sk-ant-api03-xxx"),
            ("x-api-key", "sk-ant-api03-xxx".to_string())
        );
        // Spruce / 网关令牌 → Authorization: Bearer。
        assert_eq!(
            anthropic_auth_header("sk-sp-xxx"),
            ("Authorization", "Bearer sk-sp-xxx".to_string())
        );
    }

    #[test]
    fn parse_polish_translate_sentinel_output() {
        let raw = format!(
            "{POLISHED_SOURCE_SENTINEL}\n我们明天开会。\n{TRANSLATION_SENTINEL}\nWe have a meeting tomorrow."
        );
        let pt = parse_polish_translate(&raw).unwrap();
        assert_eq!(pt.polished, "我们明天开会。");
        assert_eq!(pt.translation, "We have a meeting tomorrow.");
    }

    #[test]
    fn parse_polish_translate_rejects_malformed() {
        assert!(parse_polish_translate("随便一句没有哨兵").is_none());
        // 只有一个哨兵 → 拒绝（回退纯翻译）。
        assert!(parse_polish_translate(&format!("{POLISHED_SOURCE_SENTINEL}\n只有一半")).is_none());
        // 空段 → 拒绝。
        assert!(parse_polish_translate(&format!(
            "{POLISHED_SOURCE_SENTINEL}\n{TRANSLATION_SENTINEL}"
        ))
        .is_none());
    }
}
