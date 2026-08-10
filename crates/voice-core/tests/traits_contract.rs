//! M0 集成测试：验证四个核心 trait 的契约（对象安全、可 mock、可 Arc<dyn> 共享）。
//!
//! 这是整个工程的"地基测试"：只要这些测试通过，后续 M1–M5 的真实现都可以
//! 独立替换、独立单测，而 pipeline 永远只依赖 trait。

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;
use voice_core::{
    AppConfig, AsrProvider, AsrSession, AudioFormat, AudioFrame, AudioSource, Error, HistoryStore,
    ProviderConfig, ProviderKind, Result, SessionSummary, TextInserter, TranscriptDelta,
};

// ──────────────── 用最小 fake 实现验证 trait 契约 ────────────────

/// 喂一组固定 PCM 帧，结束后返回 None。无需麦克风。
struct FakeAudioSource {
    frames: std::collections::VecDeque<AudioFrame>,
}

impl FakeAudioSource {
    fn from_samples(chunks: &[Vec<i16>]) -> Self {
        let frames = chunks
            .iter()
            .map(|c| {
                let bytes: Vec<u8> = c.iter().flat_map(|s| s.to_le_bytes()).collect();
                AudioFrame::new(AudioFormat::PCM_16K_MONO_S16LE, bytes)
            })
            .collect();
        Self { frames }
    }
}

#[async_trait]
impl AudioSource for FakeAudioSource {
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }
    async fn next_frame(&mut self) -> Option<Result<AudioFrame>> {
        self.frames.pop_front().map(Ok)
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

/// 每收到一帧就吐一个 partial，finish 时吐 final 后关闭流。无网络。
struct FakeAsrSession {
    deltas_tx: Option<tokio::sync::mpsc::UnboundedSender<Result<TranscriptDelta>>>,
    deltas_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Result<TranscriptDelta>>>,
    sentence_index: u32,
}

impl FakeAsrSession {
    fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            deltas_tx: Some(tx),
            deltas_rx: Some(rx),
            sentence_index: 0,
        }
    }
}

impl AsrSession for FakeAsrSession {
    fn feed(
        &mut self,
        frame: &AudioFrame,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let _ = frame;
        let tx = self.deltas_tx.clone();
        let idx = self.sentence_index;
        Box::pin(async move {
            if let Some(tx) = tx {
                let _ = tx.send(Ok(TranscriptDelta::partial("你好", idx)));
            }
            Ok(())
        })
    }
    fn finish(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        let tx = self.deltas_tx.take();
        let idx = self.sentence_index;
        Box::pin(async move {
            if let Some(tx) = tx {
                let _ = tx.send(Ok(TranscriptDelta::final_("你好世界", idx)));
                // drop tx：所有发送端归零后，接收流自然结束。
            }
            Ok(())
        })
    }
    fn deltas(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<TranscriptDelta>> + Send>> {
        let rx = self.deltas_rx.take().expect("deltas() 只能调用一次");
        Box::pin(UnboundedReceiverStream::new(rx))
    }
}

struct FakeAsrProvider;
#[async_trait]
impl AsrProvider for FakeAsrProvider {
    async fn connect(&self, _cfg: &ProviderConfig) -> Result<Box<dyn AsrSession>> {
        Ok(Box::new(FakeAsrSession::new()))
    }
}

#[derive(Default)]
struct RecordingInserter {
    inserted: std::sync::Mutex<String>,
}
#[async_trait]
impl TextInserter for RecordingInserter {
    async fn insert(&self, text: &str) -> Result<()> {
        self.inserted.lock().unwrap().push_str(text);
        Ok(())
    }
}

#[derive(Default)]
struct InMemoryStore {
    sessions: std::sync::Mutex<Vec<SessionSummary>>,
}
#[async_trait]
impl HistoryStore for InMemoryStore {
    async fn create_session(&self, session: &SessionSummary) -> Result<()> {
        self.sessions.lock().unwrap().push(session.clone());
        Ok(())
    }
    async fn save_utterance(&self, _u: &voice_core::UtteranceRecord) -> Result<()> {
        Ok(())
    }
    async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        Ok(self.sessions.lock().unwrap().clone())
    }
    async fn list_utterances(&self, _session_id: &str) -> Result<Vec<voice_core::UtteranceRecord>> {
        Ok(vec![])
    }
    async fn delete_session(&self, session_id: &str) -> Result<()> {
        self.sessions.lock().unwrap().retain(|s| s.id != session_id);
        Ok(())
    }
}

// ──────────────── 测试：trait 可被 Arc<dyn> 共享 ────────────────

#[test]
fn traits_are_object_safe() {
    // 只要下面能编译，就证明四个 trait 都是对象安全的。
    let _provider: Arc<dyn AsrProvider> = Arc::new(FakeAsrProvider);
    let _inserter: Arc<dyn TextInserter> = Arc::new(RecordingInserter::default());
    let _store: Arc<dyn HistoryStore> = Arc::new(InMemoryStore::default());
    // AudioSource 含 &mut self，放 Box。
    let _audio: Box<dyn AudioSource> = Box::new(FakeAudioSource::from_samples(&[vec![0; 320]]));
}

// ──────────────── 测试：audio frame 计算 ────────────────

#[test]
fn audio_frame_samples_and_duration() {
    // 320 个 i16 采样 = 640 字节 = 20ms @16kHz。
    let frame = AudioFrame::new(
        AudioFormat::PCM_16K_MONO_S16LE,
        (0..320i16).flat_map(|s| s.to_le_bytes()).collect(),
    );
    assert_eq!(frame.samples(), 320);
    assert_eq!(frame.duration_ms(), 20);
}

// ──────────────── 测试：最小端到端 fake pipeline ────────────────

#[tokio::test]
async fn fake_pipeline_produces_final_and_inserts() {
    let mut audio = FakeAudioSource::from_samples(&[vec![0i16; 320], vec![0i16; 320]]);
    let provider = FakeAsrProvider;
    let cfg = ProviderConfig {
        kind: ProviderKind::Sherpa,
        base_url: String::new(),
        api_key: String::new(),
        model: "fake".into(),
        vocabulary_id: None,
    };
    let mut session = provider.connect(&cfg).await.unwrap();
    let inserter = Arc::new(RecordingInserter::default());

    // 喂帧
    audio.start().await.unwrap();
    while let Some(Ok(frame)) = audio.next_frame().await {
        session.feed(&frame).await.unwrap();
    }
    session.finish().await.unwrap();
    audio.stop().await.unwrap();

    // 收集 deltas，只把 final 插入
    let mut deltas = session.deltas();
    while let Some(Ok(d)) = deltas.next().await {
        if matches!(d.kind, voice_core::TranscriptKind::Final) {
            inserter.insert(&d.text).await.unwrap();
        }
    }

    let inserted = inserter.inserted.lock().unwrap().clone();
    assert_eq!(inserted, "你好世界");
}

// ──────────────── 测试：store CRUD ────────────────

#[tokio::test]
async fn in_memory_store_crud() {
    let store = InMemoryStore::default();
    let now = chrono::Utc::now();
    let s = SessionSummary {
        id: "s1".into(),
        title: "测试会话".into(),
        started_at: now,
        ended_at: None,
        engine: "local".into(),
        provider: "sherpa".into(),
        model: "fake".into(),
    };
    store.create_session(&s).await.unwrap();
    assert_eq!(store.list_sessions().await.unwrap().len(), 1);
    store.delete_session("s1").await.unwrap();
    assert!(store.list_sessions().await.unwrap().is_empty());
}

// ──────────────── 测试：config 默认与校验 ────────────────

#[test]
fn app_config_default_is_usable() {
    let c = AppConfig::default();
    assert!(c.active().is_ok());
}

#[test]
fn provider_validate_reports_missing_fields() {
    let empty_bailian = ProviderConfig {
        kind: ProviderKind::Bailian,
        base_url: String::new(),
        api_key: String::new(),
        model: String::new(),
        vocabulary_id: None,
    };
    let err = empty_bailian.validate().unwrap_err();
    assert!(matches!(err, Error::Config(_)), "got {err:?}");
}
