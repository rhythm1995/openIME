//! Multimodal ASR provider（REST POST chat/multimodal-generation）。
//!
//! 适用于：阿里云百炼 Qwen3 ASR 非流式（multimodal-generation/generation）、
//! OpenAI Chat Completions（audio input content block）等。

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::traits::{AsrProvider, AsrSession, AudioFrame, TranscriptDelta};
use crate::ProviderConfig;
use crate::{Error, Result};

pub struct MultimodalAsrProvider;

#[async_trait]
impl AsrProvider for MultimodalAsrProvider {
    async fn connect(&self, cfg: &ProviderConfig) -> Result<Box<dyn AsrSession>> {
        if !matches!(cfg.kind, crate::ProviderKind::MultimodalAsr) {
            return Err(Error::Provider(
                "MultimodalAsrProvider: kind != multimodal_asr".into(),
            ));
        }
        cfg.validate()?;
        let (dtx, drx) = mpsc::unbounded_channel::<Result<TranscriptDelta>>();
        Ok(Box::new(MultimodalAsrSession {
            samples: Vec::new(),
            cfg: cfg.clone(),
            dtx,
            deltas_rx: Some(drx),
            finished: false,
        }))
    }
}

struct MultimodalAsrSession {
    samples: Vec<f32>,
    cfg: ProviderConfig,
    dtx: mpsc::UnboundedSender<Result<TranscriptDelta>>,
    deltas_rx: Option<mpsc::UnboundedReceiver<Result<TranscriptDelta>>>,
    finished: bool,
}

impl AsrSession for MultimodalAsrSession {
    fn feed(
        &mut self,
        frame: &AudioFrame,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        for chunk in frame.bytes.chunks_exact(2) {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            self.samples.push(sample as f32 / 32768.0);
        }
        Box::pin(async { Ok(()) })
    }

    fn finish(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        if self.finished {
            return Box::pin(async { Ok(()) });
        }
        self.finished = true;
        let samples = std::mem::take(&mut self.samples);
        let cfg = self.cfg.clone();
        let dtx = self.dtx.clone();
        Box::pin(async move {
            if samples.is_empty() {
                let _ = dtx.send(Ok(TranscriptDelta::final_("", 0)));
                return Ok(());
            }
            let wav_bytes = crate::providers::openai_asr::pcm_f32_to_wav_pub(&samples, 16000);
            let b64 = {
                use base64::{engine::general_purpose, Engine as _};
                general_purpose::STANDARD.encode(&wav_bytes)
            };
            let data_uri = format!("data:audio/wav;base64,{b64}");
            let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
            let body = json!({
                "model": cfg.model,
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "input_audio",
                        "input_audio": { "data": data_uri }
                    }]
                }],
                "modalities": ["text"],
            });
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| Error::Provider(format!("创建 HTTP 客户端失败: {e}")))?;
            let resp = client
                .post(&url)
                .bearer_auth(cfg.api_key.trim())
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Provider(format!("Multimodal ASR 请求失败: {e}")))?;
            let status = resp.status();
            let raw = resp
                .text()
                .await
                .map_err(|e| Error::Provider(format!("读取响应失败: {e}")))?;
            if !status.is_success() {
                return Err(Error::Provider(format!(
                    "Multimodal ASR HTTP {status}: {}",
                    raw.chars().take(300).collect::<String>()
                )));
            }
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| Error::Provider(format!("解析 JSON 失败: {e}")))?;
            let text = v["choices"][0]["message"]["content"]
                .as_str()
                .or_else(|| {
                    v["choices"][0]["message"]["content"]
                        .as_array()
                        .and_then(|arr| arr.iter().find_map(|b| b["text"].as_str()))
                })
                .or_else(|| v["output"]["text"].as_str())
                .or_else(|| v["output"]["choices"][0]["message"]["content"].as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let _ = dtx.send(Ok(TranscriptDelta::final_(&text, 0)));
            Ok(())
        })
    }

    fn deltas(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<TranscriptDelta>> + Send>> {
        let rx = self
            .deltas_rx
            .take()
            .expect("deltas() called more than once");
        Box::pin(UnboundedReceiverStream::new(rx))
    }
}

/// 测试连接：发 1 秒静音 WAV → POST chat → 看 HTTP 200。
pub async fn test_connection(cfg: &ProviderConfig) -> Result<String> {
    cfg.validate()?;
    let silence = vec![0.0f32; 16000];
    let wav_bytes = crate::providers::openai_asr::pcm_f32_to_wav_pub(&silence, 16000);
    let b64 = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::STANDARD.encode(&wav_bytes)
    };
    let data_uri = format!("data:audio/wav;base64,{b64}");
    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let body = json!({
        "model": cfg.model,
        "messages": [{
            "role": "user",
            "content": [{
                "type": "input_audio",
                "input_audio": { "data": data_uri }
            }]
        }],
        "modalities": ["text"],
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Provider(format!("创建 HTTP 客户端失败: {e}")))?;
    let resp = client
        .post(&url)
        .bearer_auth(cfg.api_key.trim())
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Provider(format!("连接测试失败: {e}")))?;
    let status = resp.status();
    let raw = resp
        .text()
        .await
        .map_err(|e| Error::Provider(format!("读取测试响应失败: {e}")))?;
    if !status.is_success() {
        return Err(Error::Provider(format!(
            "HTTP {status}: {}",
            raw.chars().take(300).collect::<String>()
        )));
    }
    let _: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| Error::Provider(format!("响应不是有效 JSON: {e}")))?;
    Ok(format!("连接成功！模型 {} 已就绪", cfg.model))
}

#[cfg(test)]
mod tests {

    #[test]
    fn wav_via_pub() {
        let wav = crate::providers::openai_asr::pcm_f32_to_wav_pub(&[0.0, 0.5], 16000);
        assert_eq!(&wav[..4], b"RIFF");
    }
}
