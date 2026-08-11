//! 核心抽象：四个 trait 把"采集 → 转写 → 插入 → 存储"解耦，全可 mock。
//!
//! 设计要点：
//! - 所有 trait 都是 `Send + Sync`，可在 tokio 多线程运行时里以 `Arc<dyn _>` 共享。
//! - 音频与转写均以流式语义表达，支持 partial / final 两类增量。
//! - 录音不需要真麦克风即可测试：把 WAV fixture 喂给 `MockAudioSource`。

use async_trait::async_trait;
use core::future::Future;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::Result;

// ───────────────────────── 音频 ─────────────────────────

/// PCM 音频的格式约定。一期固定为百炼 / sherpa-onnx 通用的 16kHz / mono / LE-i16。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl AudioFormat {
    /// 一期唯一支持的源/目标格式。
    pub const PCM_16K_MONO_S16LE: AudioFormat = AudioFormat {
        sample_rate: 16_000,
        channels: 1,
        bits_per_sample: 16,
    };

    /// 每个采样占用的字节数。
    pub fn bytes_per_sample(&self) -> u16 {
        self.bits_per_sample / 8
    }
}

/// 一帧音频数据。`bytes` 是裸 PCM（无 WAV 头），字节序由 format 决定。
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub format: AudioFormat,
    pub bytes: Vec<u8>,
}

impl AudioFrame {
    pub fn new(format: AudioFormat, bytes: Vec<u8>) -> Self {
        Self { format, bytes }
    }

    /// 该帧对应的采样数（每通道）。
    pub fn samples(&self) -> usize {
        let bps = self.format.bytes_per_sample() as usize;
        if bps == 0 {
            return 0;
        }
        self.bytes.len() / (bps * self.format.channels as usize)
    }

    /// 该帧的时长（毫秒）。
    pub fn duration_ms(&self) -> u32 {
        if self.format.sample_rate == 0 {
            return 0;
        }
        ((self.samples() as u64) * 1000 / self.format.sample_rate as u64) as u32
    }
}

/// 音频源抽象。真实现见 [`crate::audio::CpalAudioSource`]；测试用 mock 喂固定帧序列。
#[async_trait]
pub trait AudioSource: Send {
    /// 开始采集。返回后即可调用 [`Self::next_frame`]。
    async fn start(&mut self) -> Result<()>;
    /// 阻塞获取下一帧音频；`None` 表示流结束（录音停止）。
    async fn next_frame(&mut self) -> Option<Result<AudioFrame>>;
    /// 停止采集。
    async fn stop(&mut self) -> Result<()>;
}

// ───────────────────────── 转写 ─────────────────────────

/// 转写增量的类型。partial 是中间结果（会变），final 是一句的定稿。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptKind {
    Partial,
    Final,
}

/// 一条转写增量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptDelta {
    pub kind: TranscriptKind,
    /// 本句当前的文字。partial 会随新帧变化；final 是该句最终文字。
    /// 注意：百炼的 sentence.text 是"单句"语义，不跨句累计，由 pipeline 负责拼接。
    pub text: String,
    /// 句子在本任务中的序号（从 0 起），用于区分不同句子。
    pub sentence_index: u32,
}

impl TranscriptDelta {
    pub fn partial(text: impl Into<String>, sentence_index: u32) -> Self {
        Self {
            kind: TranscriptKind::Partial,
            text: text.into(),
            sentence_index,
        }
    }
    pub fn final_(text: impl Into<String>, sentence_index: u32) -> Self {
        Self {
            kind: TranscriptKind::Final,
            text: text.into(),
            sentence_index,
        }
    }
}

/// 一次转写会话（对应一条 `run-task` / 一次录音）。
///
/// `deltas()` 返回 `'static` 的流（不借 `self`）：内部把后台任务的接收端移出并包装。
/// 这样 stream 可以 move 进独立的 reader 任务，与主循环的 `feed`/`finish` 并发。
/// 实现需保证 deltas 只调用一次（第二次返回空流或 panic）。
pub trait AsrSession: Send {
    /// 推送一帧音频给 provider。
    fn feed(&mut self, frame: &AudioFrame)
        -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
    /// 通知 provider 录音结束（对应百炼 finish-task）。
    fn finish(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
    /// 转写增量流（'static，可 move 进后台任务）。partial / final 都从这里出。
    fn deltas(&mut self) -> Pin<Box<dyn Stream<Item = Result<TranscriptDelta>> + Send>>;
}

/// 通过 [`ProviderConfig`] 建立一次转写会话。对象安全，可放 `Arc<dyn AsrProvider>`。
#[async_trait]
pub trait AsrProvider: Send + Sync {
    async fn connect(&self, cfg: &crate::ProviderConfig) -> Result<Box<dyn AsrSession>>;
}

// ───────────────────────── 文本插入 ─────────────────────────

/// 把转写结果写入前台 App 的光标位置。真实现见 [`crate::insert::EnigoInserter`]。
#[async_trait]
pub trait TextInserter: Send + Sync {
    async fn insert(&self, text: &str) -> Result<()>;
}

// ───────────────────────── 文本润色（二期） ─────────────────────────

/// 润色强度 / 模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolishMode {
    /// 不做任何处理（直通）。
    #[default]
    Off,
    /// 轻量：去口头禅、补标点、纠明显 ASR 错、不改语气。
    Light,
    /// 高度：L0 规则 + L2 改写润色（通顺化、调整语序，保留原意）。
    Heavy,
}

/// 一次润色请求（通常对应一条 ASR final）。
#[derive(Debug, Clone)]
pub struct PolishRequest {
    pub text: String,
    pub mode: PolishMode,
    /// 热词：提示模型保留写法。
    pub hotwords: Vec<String>,
    /// 超时；超时后 router 可回退原文/云端。
    pub timeout: std::time::Duration,
}

/// 润色结果。
#[derive(Debug, Clone)]
pub struct PolishResponse {
    pub text: String,
    /// 实际生效的实现：passthrough / local-gguf / bailian-chat 等。
    pub provider: String,
    pub latency_ms: u32,
}

/// 文本增强：润色 / 人设。与 [`AsrProvider`] 对称，可 mock。
#[async_trait]
pub trait TextPolishProvider: Send + Sync {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse>;
}

// ───────────────────────── 历史存储 ─────────────────────────

/// 一条录音（utterance）。一次会话可含多条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtteranceRecord {
    pub id: String,
    pub session_id: String,
    pub seq: u32,
    pub final_text: String,
    /// 可选：原始音频文件路径（按需保存）。
    pub audio_path: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 一个会话的摘要（不含每条录音）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    pub engine: String,
    pub provider: String,
    pub model: String,
}

/// 历史记录存储。真实现见 [`crate::store::SqliteStore`]。
#[async_trait]
pub trait HistoryStore: Send + Sync {
    async fn create_session(&self, session: &SessionSummary) -> Result<()>;
    async fn save_utterance(&self, utterance: &UtteranceRecord) -> Result<()>;
    async fn list_sessions(&self) -> Result<Vec<SessionSummary>>;
    async fn list_utterances(&self, session_id: &str) -> Result<Vec<UtteranceRecord>>;
    async fn delete_session(&self, session_id: &str) -> Result<()>;
}
