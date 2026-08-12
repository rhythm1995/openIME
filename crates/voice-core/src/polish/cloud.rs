//! 云端润色：3 种 LLM 协议统一接入。
//!
//! - OpenAI Chat Completions（/chat/completions）— 百炼/OpenAI/OpenRouter 等
//! - Anthropic Messages API（/v1/messages）— Claude
//! - OpenAI Responses API（/v1/responses）— OpenAI 新版
//!
//! 统一入口 `CloudPolishProvider`，按 `PolishCloudProtocol` 路由。

use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use crate::config::PolishCloudProtocol;
use crate::traits::{PolishMode, PolishRequest, PolishResponse, TextPolishProvider};
use crate::{Error, Result};

use super::prompts::build_messages;

/// 云端润色 provider（3 协议统一）。
pub struct CloudPolishProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub protocol: PolishCloudProtocol,
}

/// 向后兼容：旧名 BailianChatPolish = CloudPolishProvider(OpenAiChat)。
pub type BailianChatPolish = CloudPolishProvider;

impl BailianChatPolish {
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

    /// 向后兼容。
    pub fn default_chat_base() -> String {
        "https://dashscope.aliyuncs.com/compatible-mode/v1".into()
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
            return Err(Error::Config("云端润色缺少 api_key".into()));
        }

        let t0 = Instant::now();
        let messages = build_messages(
            &req.text,
            req.mode,
            &req.hotwords,
            req.style_prompt.as_deref(),
        );

        let text = match self.protocol {
            PolishCloudProtocol::OpenAiChat => self.polish_openai_chat(&messages, &req).await?,
            PolishCloudProtocol::Anthropic => self.polish_anthropic(&messages, &req).await?,
            PolishCloudProtocol::OpenAiResponses => {
                self.polish_openai_responses(&messages, &req).await?
            }
        };

        if text.is_empty() {
            return Err(Error::Provider("云端润色返回空文本".into()));
        }

        Ok(PolishResponse {
            text,
            provider: format!("cloud-{:?}", self.protocol).to_lowercase(),
            latency_ms: t0.elapsed().as_millis() as u32,
        })
    }
}

impl CloudPolishProvider {
    /// 协议 1：OpenAI Chat Completions。
    async fn polish_openai_chat(
        &self,
        messages: &[(String, String)],
        req: &PolishRequest,
    ) -> Result<String> {
        let body_msgs: Vec<_> = messages
            .iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect();
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "messages": body_msgs,
            "temperature": 0.3,
            "max_tokens": 256,
        });
        let raw = self
            .post_json(&url, &body, AuthType::Bearer, req.timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Provider(format!("解析 OpenAI Chat JSON 失败: {e}")))?;
        Ok(v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string())
    }

    /// 协议 2：Anthropic Messages API。
    async fn polish_anthropic(
        &self,
        messages: &[(String, String)],
        req: &PolishRequest,
    ) -> Result<String> {
        // Anthropic 格式：system 独立，messages 只有 user/assistant。
        let (system_msg, chat_msgs) = split_system(messages);
        let url = format!("{}/messages", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "max_tokens": 256,
            "system": system_msg,
            "messages": chat_msgs,
        });
        let raw = self
            .post_json(&url, &body, AuthType::Anthropic, req.timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Provider(format!("解析 Anthropic JSON 失败: {e}")))?;
        // response: { content: [{ type: "text", text: "..." }] }
        let text = v["content"]
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|b| b["type"].as_str() == Some("text"))
                    .and_then(|b| b["text"].as_str())
            })
            .unwrap_or("")
            .trim()
            .to_string();
        Ok(text)
    }

    /// 协议 3：OpenAI Responses API。
    async fn polish_openai_responses(
        &self,
        messages: &[(String, String)],
        req: &PolishRequest,
    ) -> Result<String> {
        // Responses API：instructions = system，input = user text。
        let (system_msg, chat_msgs) = split_system(messages);
        let user_text = chat_msgs
            .iter()
            .find(|m| m["role"].as_str() == Some("user"))
            .and_then(|m| m["content"].as_str())
            .unwrap_or("");
        let url = format!("{}/responses", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "instructions": system_msg,
            "input": user_text,
            "temperature": 0.3,
            "max_output_tokens": 256,
        });
        let raw = self
            .post_json(&url, &body, AuthType::Bearer, req.timeout)
            .await?;
        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Provider(format!("解析 Responses JSON 失败: {e}")))?;
        // response: { output: [{ content: [{ type: "output_text", text: "..." }] }] }
        let text = v["output"]
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
            .to_string();
        Ok(text)
    }

    /// 通用 POST JSON（按 auth 类型设 header）。
    async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        auth: AuthType,
        timeout: std::time::Duration,
    ) -> Result<String> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Provider(format!("创建 HTTP 客户端失败: {e}")))?;
        let mut req_builder = client.post(url).json(body);
        req_builder = match auth {
            AuthType::Bearer => req_builder.bearer_auth(self.api_key.trim()),
            AuthType::Anthropic => req_builder
                .header("x-api-key", self.api_key.trim())
                .header("anthropic-version", "2023-06-01"),
        };
        let resp = req_builder
            .send()
            .await
            .map_err(|e| Error::Provider(format!("云端润色请求失败: {e}")))?;
        let status = resp.status();
        let raw = resp
            .text()
            .await
            .map_err(|e| Error::Provider(format!("读取云端响应失败: {e}")))?;
        if !status.is_success() {
            return Err(Error::Provider(format!(
                "云端润色 HTTP {status}: {}",
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
