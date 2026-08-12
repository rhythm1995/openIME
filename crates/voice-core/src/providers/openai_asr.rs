//! OpenAI 兼容 ASR provider（REST POST /audio/transcriptions）。
//!
//! 适用于：OpenAI Whisper API、OpenRouter Audio Transcriptions、
//! 阿里云 DashScope（OpenAI 兼容模式）等。

use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::traits::{AsrProvider, AsrSession, AudioFrame, TranscriptDelta};
use crate::ProviderConfig;
use crate::{Error, Result};

pub struct OpenAiAsrProvider;

#[async_trait]
impl AsrProvider for OpenAiAsrProvider {
    async fn connect(&self, cfg: &ProviderConfig) -> Result<Box<dyn AsrSession>> {
        if !matches!(cfg.kind, crate::ProviderKind::OpenAiAsr) {
            return Err(Error::Provider(
                "OpenAiAsrProvider: kind != openai_asr".into(),
            ));
        }
        cfg.validate()?;
        let (dtx, drx) = mpsc::unbounded_channel::<Result<TranscriptDelta>>();
        Ok(Box::new(OpenAiAsrSession {
            samples: Vec::new(),
            cfg: cfg.clone(),
            dtx: Some(dtx),
            deltas_rx: Some(drx),
            finished: false,
        }))
    }
}

struct OpenAiAsrSession {
    samples: Vec<f32>,
    cfg: ProviderConfig,
    dtx: Option<mpsc::UnboundedSender<Result<TranscriptDelta>>>,
    deltas_rx: Option<mpsc::UnboundedReceiver<Result<TranscriptDelta>>>,
    finished: bool,
}

impl AsrSession for OpenAiAsrSession {
    fn feed(
        &mut self,
        frame: &AudioFrame,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        // 同步操作：s16le bytes → f32 samples，无需 async。
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
        let dtx = self.dtx.take().expect("deltas channel sender 已取走");
        Box::pin(async move {
            if samples.is_empty() {
                let _ = dtx.send(Ok(TranscriptDelta::final_("", 0)));
                return Ok(());
            }
            let wav_bytes = pcm_f32_to_wav_pub(&samples, 16000);
            let b64 = {
                use base64::{engine::general_purpose, Engine as _};
                general_purpose::STANDARD.encode(&wav_bytes)
            };
            let url = format!(
                "{}/audio/transcriptions",
                cfg.base_url.trim_end_matches('/')
            );
            let body = json!({
                "model": cfg.model,
                "input_audio": {
                    "data": b64,
                    "format": "wav",
                },
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
                .map_err(|e| Error::Provider(format!("ASR REST 请求失败: {e}")))?;
            let status = resp.status();
            let raw = resp
                .text()
                .await
                .map_err(|e| Error::Provider(format!("读取 ASR 响应失败: {e}")))?;
            if !status.is_success() {
                return Err(Error::Provider(format!(
                    "ASR REST HTTP {status}: {}",
                    raw.chars().take(300).collect::<String>()
                )));
            }
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| Error::Provider(format!("解析 ASR JSON 失败: {e}")))?;
            let text = v["text"].as_str().unwrap_or("").trim().to_string();
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

/// f32 PCM → s16le WAV bytes（16kHz mono）。pub 供 multimodal_asr 复用。
pub fn pcm_f32_to_wav_pub(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_size = samples.len() * 2;
    let mut wav = Vec::with_capacity(44 + data_size);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_size as u32).to_le_bytes());
    for s in samples {
        let clamped = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        wav.extend_from_slice(&clamped.to_le_bytes());
    }
    wav
}

/// 测试连接：发 1 秒静音 WAV → POST → 看 HTTP 200。
pub async fn test_connection(cfg: &ProviderConfig) -> Result<String> {
    cfg.validate()?;
    let silence = vec![0.0f32; 16000];
    let wav_bytes = pcm_f32_to_wav_pub(&silence, 16000);
    let b64 = {
        use base64::{engine::general_purpose, Engine as _};
        general_purpose::STANDARD.encode(&wav_bytes)
    };
    let url = format!(
        "{}/audio/transcriptions",
        cfg.base_url.trim_end_matches('/')
    );
    let body = json!({
        "model": cfg.model,
        "input_audio": { "data": b64, "format": "wav" },
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
    use super::*;

    #[test]
    fn wav_header_correct() {
        let wav = pcm_f32_to_wav_pub(&[0.0, 0.5, -0.5], 16000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 6);
    }
}
