//! 百炼 OpenAI 兼容 chat 润色。

use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use crate::traits::{PolishMode, PolishRequest, PolishResponse, TextPolishProvider};
use crate::{Error, Result};

use super::prompts::build_messages;

/// 云端 chat 润色（DashScope / 百炼 OpenAI 兼容接口）。
pub struct BailianChatPolish {
    pub api_key: String,
    /// 如 `https://dashscope.aliyuncs.com/compatible-mode/v1`
    pub base_url: String,
    pub model: String,
}

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
        }
    }

    /// 从 ASR 用的 wss base_url 推导 chat base（失败则用官方兼容地址）。
    pub fn default_chat_base() -> String {
        "https://dashscope.aliyuncs.com/compatible-mode/v1".into()
    }
}

#[async_trait]
impl TextPolishProvider for BailianChatPolish {
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
            req.persona_prompt.as_deref(),
            &req.hotwords,
        );
        let body_msgs: Vec<_> = messages
            .iter()
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect();

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let client = reqwest::Client::builder()
            .timeout(req.timeout)
            .build()
            .map_err(|e| Error::Provider(format!("创建 HTTP 客户端失败: {e}")))?;

        let resp = client
            .post(&url)
            .bearer_auth(self.api_key.trim())
            .json(&json!({
                "model": self.model,
                "messages": body_msgs,
                "temperature": 0.3,
                "max_tokens": 256,
            }))
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
                raw.chars().take(200).collect::<String>()
            )));
        }

        let v: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Provider(format!("解析云端 JSON 失败: {e}")))?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(Error::Provider("云端润色返回空文本".into()));
        }

        Ok(PolishResponse {
            text,
            provider: "bailian-chat".into(),
            latency_ms: t0.elapsed().as_millis() as u32,
        })
    }
}
