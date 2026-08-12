//! 端到端 pipeline：编排"采集 → 转写 → 插入 → 落库"。
//!
//! 设计：`Pipeline` 持有四个 trait 对象（Arc），`record_once` 跑一次完整录音会话：
//! 1. provider.connect 建立会话；
//! 2. 起 reader 任务消费 deltas：partial 经 `on_partial` 回调（UI 显示），
//!    final 经 inserter 插入前台 App + 落库为 utterance；
//! 3. 主循环：audio.next_frame → session.feed；
//! 4. 录音停止（audio 返回 None 或外部 cancel）→ session.finish → 等 reader 结束。
//!
//! 完全用 mock 可测：FakeAudioSource + FakeAsrProvider + RecordingInserter + InMemoryStore。

use std::sync::Arc;

use futures::StreamExt;
use uuid::Uuid;

use crate::traits::{
    AsrProvider, AudioSource, HistoryStore, PolishMode, PolishRequest, SessionSummary,
    TextInserter, TextPolishProvider, TranscriptKind, UtteranceRecord,
};
use crate::Error;

/// pipeline 的依赖。全部以 Arc<dyn> 注入，便于 mock 与替换。
pub struct PipelineDeps {
    pub provider: Arc<dyn AsrProvider>,
    pub inserter: Arc<dyn TextInserter>,
    pub store: Arc<dyn HistoryStore>,
    /// 二期润色；None 则直通原文。
    pub polish: Option<Arc<dyn TextPolishProvider>>,
}

/// 润色上下文（录音结束插入前使用）。
#[derive(Debug, Clone, Default)]
pub struct PolishContext {
    pub enabled: bool,
    pub mode: PolishMode,
    pub style_prompt: Option<String>,
    pub hotwords: Vec<String>,
    pub timeout_ms: u32,
}

/// partial 增量回调（UI 用）。一期可忽略返回。
pub type PartialCallback = Arc<dyn Fn(&str) + Send + Sync>;

/// 一次录音会话的结果汇总。
#[derive(Debug, Clone, Default)]
pub struct SessionResult {
    pub session_id: String,
    pub utterances: Vec<String>,
}

/// 一次录音会话需要的上下文：引擎/provider/model 元信息（来自当前配置）。
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub engine: String,
    pub provider: String,
    pub model: String,
}

/// pipeline 编排器。`audio` 在每次 record_once 时由调用方提供（便于复用/替换）。
pub struct Pipeline {
    deps: PipelineDeps,
}

impl Pipeline {
    pub fn new(deps: PipelineDeps) -> Self {
        Self { deps }
    }

    /// 跑一次完整录音会话。
    ///
    /// - `audio`：音频源（已 start 或将由本方法 start）。
    /// - `cfg`：provider 配置（传给 connect）。
    /// - `meta`：会话元信息（存入 session）。
    /// - `on_partial`：partial 回调；传 `None` 则忽略 partial。
    /// - `stop_flag`：外部停止标志；置 true 后本方法在当前帧后停止喂音频并 finish。
    ///
    /// 返回会话结果（session_id + 各 final 文本）。文本由内部 `insert_finals` 插入前台。
    pub async fn record_once(
        &self,
        audio: Box<dyn AudioSource>,
        cfg: &crate::ProviderConfig,
        meta: SessionMeta,
        on_partial: Option<PartialCallback>,
        stop_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> crate::Result<SessionResult> {
        let result = self
            .record_and_collect(audio, cfg, meta, on_partial, stop_flag, false)
            .await?;
        self.insert_finals(&result.session_id, &result.utterances)
            .await?;
        Ok(result)
    }

    /// 只录音并收集各 final 文本，**不插入**前台 App。
    ///
    /// 用于需要先恢复前台焦点（如 macOS 上 overlay 抢焦点）再插入的场景：
    /// 调用方拿到结果后应先激活目标 app，再调 [`Self::insert_finals`]。
    pub async fn record_and_collect(
        &self,
        mut audio: Box<dyn AudioSource>,
        cfg: &crate::ProviderConfig,
        meta: SessionMeta,
        on_partial: Option<PartialCallback>,
        stop_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        streaming_insert: bool,
    ) -> crate::Result<SessionResult> {
        let session_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        // 建立会话记录（先建，便于即使中途失败也留痕）。
        self.deps
            .store
            .create_session(&SessionSummary {
                id: session_id.clone(),
                title: meta.model.clone(),
                started_at: now,
                ended_at: None,
                engine: meta.engine.clone(),
                provider: meta.provider.clone(),
                model: meta.model.clone(),
            })
            .await?;

        // 连接 ASR。
        let mut asr = self.deps.provider.connect(cfg).await?;
        // 先取出 deltas 流（'static），reader 任务持有它，不持有 asr。
        let deltas = asr.deltas();

        // reader：消费 deltas，收集 final，partial 走回调。
        // C1 streaming_insert=true 时，partial/final 经 diff_prefix 增量上屏（Unicode 安全）。
        let partial_cb = on_partial;
        let inserter = self.deps.inserter.clone();
        let inserted: std::sync::Arc<std::sync::Mutex<String>> =
            std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let streaming = streaming_insert;
        let reader = tokio::spawn(async move {
            let mut finals: Vec<String> = Vec::new();
            let mut deltas = deltas;
            while let Some(item) = deltas.next().await {
                match item {
                    Ok(d) => match d.kind {
                        TranscriptKind::Partial => {
                            if let Some(cb) = &partial_cb {
                                cb(&d.text);
                            }
                            if streaming {
                                let delta = {
                                    let mut s = inserted.lock().unwrap();
                                    let delta = crate::insert::diff_prefix(&s, &d.text).to_string();
                                    *s = d.text.clone();
                                    delta
                                };
                                if !delta.is_empty() {
                                    let _ = inserter.insert(&delta).await;
                                }
                            }
                        }
                        TranscriptKind::Final => {
                            if streaming {
                                let delta = {
                                    let mut s = inserted.lock().unwrap();
                                    let delta = crate::insert::diff_prefix(&s, &d.text).to_string();
                                    s.clear(); // 句末：下一句从零开始
                                    delta
                                };
                                if !delta.is_empty() {
                                    let _ = inserter.insert(&delta).await;
                                }
                            }
                            finals.push(d.text.clone());
                        }
                    },
                    Err(_) => break,
                }
            }
            finals
        });

        // 主循环：喂音频。检查外部停止标志。
        audio.start().await?;
        loop {
            if let Some(flag) = &stop_flag {
                if flag.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
            }
            match audio.next_frame().await {
                Some(Ok(frame)) => asr.feed(&frame).await?,
                Some(Err(_)) => break,
                None => break,
            }
        }
        audio.stop().await?;
        asr.finish().await?;

        let finals = reader
            .await
            .map_err(|e| Error::Insert(format!("reader 任务 panic: {e}")))?;

        Ok(SessionResult {
            session_id,
            utterances: finals,
        })
    }

    /// 把已收集的 final 文本插入前台 App 并落库。
    ///
    /// 调用方负责：需要在插入前确保目标窗口（前台 App）已获得焦点。
    pub async fn insert_finals(&self, session_id: &str, finals: &[String]) -> crate::Result<()> {
        self.insert_finals_with_polish(session_id, finals, &PolishContext::default())
            .await
    }

    /// 插入前可选润色（二期）。`ctx.enabled=false` 或无 polish 依赖时与 [`insert_finals`] 相同。
    pub async fn insert_finals_with_polish(
        &self,
        session_id: &str,
        finals: &[String],
        ctx: &PolishContext,
    ) -> crate::Result<()> {
        // ASR 有时会连续推两条相同 final；先去重再润色/上屏，避免「同一句输入两次」。
        let finals = crate::polish::dedupe_consecutive_finals(finals);
        let mut last_inserted = String::new();
        for (seq, text) in finals.iter().enumerate() {
            let polished = self.apply_polish(text, ctx).await;
            if polished.is_empty() {
                continue;
            }
            // 上屏级再挡一层：连续两条润色结果相同则只插一次。
            if polished == last_inserted {
                tracing::debug!("跳过与上一条相同的上屏文本");
                continue;
            }
            self.deps.inserter.insert(&polished).await?;
            last_inserted = polished.clone();
            self.deps
                .store
                .save_utterance(&UtteranceRecord {
                    id: Uuid::new_v4().to_string(),
                    session_id: session_id.to_string(),
                    seq: seq as u32,
                    // 落库保存实际上屏文本（润色后）。
                    final_text: polished,
                    audio_path: None,
                    created_at: chrono::Utc::now(),
                })
                .await?;
        }
        Ok(())
    }

    /// C1：流式模式专用——finals 已在录音期间逐字上屏，只去重+落库，不重复插入。
    pub async fn persist_finals(&self, session_id: &str, finals: &[String]) -> crate::Result<()> {
        let finals = crate::polish::dedupe_consecutive_finals(finals);
        let mut last = String::new();
        for (seq, text) in finals.iter().enumerate() {
            let text = text.trim();
            if text.is_empty() || text == last {
                continue;
            }
            self.deps
                .store
                .save_utterance(&UtteranceRecord {
                    id: Uuid::new_v4().to_string(),
                    session_id: session_id.to_string(),
                    seq: seq as u32,
                    final_text: text.to_string(),
                    audio_path: None,
                    created_at: chrono::Utc::now(),
                })
                .await?;
            last = text.to_string();
        }
        Ok(())
    }

    async fn apply_polish(&self, text: &str, ctx: &PolishContext) -> String {
        if text.trim().is_empty() {
            return text.to_string();
        }
        // ── L0 规则层：总是先过一遍（即使总体润色关闭，也做最小清理）；不阻断。
        let l0 = crate::polish::correct_l0(text, &ctx.hotwords);
        if l0.text.trim().is_empty() {
            return l0.text;
        }
        tracing::debug!(
            "L0 规则层：had_correction={} truncation={} 原='{}' 纠后='{}'",
            l0.had_correction,
            l0.truncation_flag,
            text,
            l0.text
        );

        // 若总体润色关闭 / 无 provider / 模式 Off → L0 直出。
        if !ctx.enabled || ctx.mode == PolishMode::Off {
            return l0.text;
        }
        let Some(polish) = &self.deps.polish else {
            return l0.text;
        };

        // ── L2 gating：≤8 字跳过 LLM（过度纠正 + 延迟不值得；调研 6.3）。
        if l0.text.trim().chars().count() <= 8 {
            tracing::debug!("L2 跳过：≤8 字，L0 直出");
            return l0.text;
        }

        // ── L2 LLM 纯校对（失败→ L0 回退，不阻断上屏）。
        let req = PolishRequest {
            text: l0.text.clone(),
            mode: ctx.mode,
            style_prompt: ctx.style_prompt.clone(),
            hotwords: ctx.hotwords.clone(),
            timeout: std::time::Duration::from_millis(ctx.timeout_ms.max(100) as u64),
        };
        match polish.polish(req).await {
            Ok(r) => {
                if r.text.trim().is_empty() {
                    l0.text
                } else {
                    let cleaned = crate::polish::sanitize_polish_output(&l0.text, &r.text);
                    if cleaned != r.text.trim() {
                        tracing::info!(
                            "润色输出已清洗（防重复）：provider={} raw_len={} clean_len={}",
                            r.provider,
                            r.text.len(),
                            cleaned.len()
                        );
                    }
                    cleaned
                }
            }
            Err(e) => {
                tracing::warn!("润色失败，使用 L0 结果：{e}");
                l0.text
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{
        AsrSession, AudioFormat, AudioFrame, PolishRequest, PolishResponse, TextPolishProvider,
        TranscriptDelta,
    };
    use crate::ProviderConfig;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex as StdMutex;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    // ---- fakes ----

    struct FakeAudio {
        frames: VecDeque<AudioFrame>,
    }
    impl FakeAudio {
        fn new(n: usize) -> Self {
            Self {
                frames: (0..n)
                    .map(|_| AudioFrame::new(AudioFormat::PCM_16K_MONO_S16LE, vec![0u8; 640]))
                    .collect(),
            }
        }
    }
    #[async_trait]
    impl AudioSource for FakeAudio {
        async fn start(&mut self) -> crate::Result<()> {
            Ok(())
        }
        async fn next_frame(&mut self) -> Option<crate::Result<AudioFrame>> {
            self.frames.pop_front().map(Ok)
        }
        async fn stop(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }

    struct FakeSession {
        rx: StdMutex<Option<tokio::sync::mpsc::UnboundedReceiver<crate::Result<TranscriptDelta>>>>,
        tx: StdMutex<Option<tokio::sync::mpsc::UnboundedSender<crate::Result<TranscriptDelta>>>>,
    }
    impl AsrSession for FakeSession {
        fn feed(
            &mut self,
            _frame: &AudioFrame,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
        {
            Box::pin(async { Ok(()) })
        }
        fn finish(
            &mut self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
        {
            let tx = self.tx.lock().unwrap().take();
            Box::pin(async move {
                if let Some(tx) = tx {
                    let _ = tx.send(Ok(TranscriptDelta::partial("你好", 0)));
                    let _ = tx.send(Ok(TranscriptDelta::final_("你好世界", 0)));
                    // tx drop：接收流自然结束。
                }
                Ok(())
            })
        }
        fn deltas(
            &mut self,
        ) -> std::pin::Pin<Box<dyn futures::Stream<Item = crate::Result<TranscriptDelta>> + Send>>
        {
            let rx = self.rx.lock().unwrap().take().unwrap();
            Box::pin(UnboundedReceiverStream::new(rx))
        }
    }

    struct FakeProvider;
    #[async_trait]
    impl AsrProvider for FakeProvider {
        async fn connect(&self, _cfg: &ProviderConfig) -> crate::Result<Box<dyn AsrSession>> {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            Ok(Box::new(FakeSession {
                rx: StdMutex::new(Some(rx)),
                tx: StdMutex::new(Some(tx)),
            }))
        }
    }

    #[derive(Default)]
    struct RecInserter {
        out: StdMutex<String>,
    }
    #[async_trait]
    impl TextInserter for RecInserter {
        async fn insert(&self, text: &str) -> crate::Result<()> {
            self.out.lock().unwrap().push_str(text);
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemStore {
        sessions: StdMutex<Vec<SessionSummary>>,
        utterances: StdMutex<Vec<UtteranceRecord>>,
    }
    #[async_trait]
    impl HistoryStore for MemStore {
        async fn create_session(&self, s: &SessionSummary) -> crate::Result<()> {
            self.sessions.lock().unwrap().push(s.clone());
            Ok(())
        }
        async fn save_utterance(&self, u: &UtteranceRecord) -> crate::Result<()> {
            self.utterances.lock().unwrap().push(u.clone());
            Ok(())
        }
        async fn list_sessions(&self) -> crate::Result<Vec<SessionSummary>> {
            Ok(self.sessions.lock().unwrap().clone())
        }
        async fn list_utterances(&self, _sid: &str) -> crate::Result<Vec<UtteranceRecord>> {
            Ok(self.utterances.lock().unwrap().clone())
        }
        async fn delete_session(&self, sid: &str) -> crate::Result<()> {
            self.sessions.lock().unwrap().retain(|s| s.id != sid);
            Ok(())
        }
    }

    fn deps() -> (PipelineDeps, Arc<RecInserter>, Arc<MemStore>) {
        let ins = Arc::new(RecInserter::default());
        let store = Arc::new(MemStore::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: store.clone(),
            polish: None,
        };
        (deps, ins, store)
    }

    #[tokio::test]
    async fn pipeline_inserts_final_and_stores() {
        let (deps, ins, store) = deps();
        let pipe = Pipeline::new(deps);

        let partial_count = Arc::new(StdMutex::new(0u32));
        let pc = partial_count.clone();
        let on_partial: PartialCallback = Arc::new(move |_| {
            *pc.lock().unwrap() += 1;
        });

        let cfg = ProviderConfig {
            kind: crate::ProviderKind::Sherpa,
            base_url: String::new(),
            api_key: String::new(),
            model: "test".into(),
            vocabulary_id: None,
            language: None,
        };
        let meta = SessionMeta {
            engine: "local".into(),
            provider: "fake".into(),
            model: "test".into(),
        };

        let result = pipe
            .record_once(
                Box::new(FakeAudio::new(3)),
                &cfg,
                meta,
                Some(on_partial),
                None,
            )
            .await
            .unwrap();

        assert_eq!(result.utterances, vec!["你好世界"]);
        assert_eq!(*ins.out.lock().unwrap(), "你好世界");
        assert_eq!(*partial_count.lock().unwrap(), 1);
        assert_eq!(store.sessions.lock().unwrap().len(), 1);
        assert_eq!(store.utterances.lock().unwrap().len(), 1);
        assert_eq!(store.utterances.lock().unwrap()[0].final_text, "你好世界");
    }

    #[tokio::test]
    async fn pipeline_creates_session_even_if_no_finals() {
        // 一个不发任何 delta 的 provider。
        struct EmptySession;
        impl AsrSession for EmptySession {
            fn feed(
                &mut self,
                _f: &AudioFrame,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
            {
                Box::pin(async { Ok(()) })
            }
            fn finish(
                &mut self,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>>
            {
                Box::pin(async { Ok(()) })
            }
            fn deltas(
                &mut self,
            ) -> std::pin::Pin<Box<dyn futures::Stream<Item = crate::Result<TranscriptDelta>> + Send>>
            {
                Box::pin(futures::stream::empty())
            }
        }
        struct EmptyProvider;
        #[async_trait]
        impl AsrProvider for EmptyProvider {
            async fn connect(&self, _c: &ProviderConfig) -> crate::Result<Box<dyn AsrSession>> {
                Ok(Box::new(EmptySession))
            }
        }

        let ins = Arc::new(RecInserter::default());
        let store = Arc::new(MemStore::default());
        let pipe = Pipeline::new(PipelineDeps {
            provider: Arc::new(EmptyProvider),
            inserter: ins.clone(),
            store: store.clone(),
            polish: None,
        });

        let cfg = ProviderConfig {
            kind: crate::ProviderKind::Sherpa,
            base_url: String::new(),
            api_key: String::new(),
            model: "test".into(),
            vocabulary_id: None,
            language: None,
        };
        let meta = SessionMeta {
            engine: "local".into(),
            provider: "fake".into(),
            model: "test".into(),
        };
        let r = pipe
            .record_once(Box::new(FakeAudio::new(1)), &cfg, meta, None, None)
            .await
            .unwrap();
        assert!(r.utterances.is_empty());
        assert_eq!(store.sessions.lock().unwrap().len(), 1);
        assert!(ins.out.lock().unwrap().is_empty());
    }

    // ── L0 / L2 / 回退 集成测试（TDD）──────────────────────────

    enum MockBehavior {
        Ok(String),
        Empty,
        Err,
    }

    struct MockPolish {
        calls: Arc<AtomicU32>,
        behavior: MockBehavior,
    }
    impl MockPolish {
        fn new(b: MockBehavior) -> Self {
            Self {
                calls: Arc::new(AtomicU32::new(0)),
                behavior: b,
            }
        }
    }
    #[async_trait]
    impl TextPolishProvider for MockPolish {
        async fn polish(&self, _req: PolishRequest) -> crate::Result<PolishResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.behavior {
                MockBehavior::Ok(t) => Ok(PolishResponse {
                    text: t.clone(),
                    provider: "mock".into(),
                    latency_ms: 1,
                }),
                MockBehavior::Empty => Ok(PolishResponse {
                    text: String::new(),
                    provider: "mock".into(),
                    latency_ms: 1,
                }),
                MockBehavior::Err => Err(crate::Error::Provider("mock fail".into())),
            }
        }
    }

    fn deps_with_polish(
        polish: Arc<dyn TextPolishProvider>,
    ) -> (PipelineDeps, Arc<RecInserter>, Arc<MemStore>) {
        let ins = Arc::new(RecInserter::default());
        let store = Arc::new(MemStore::default());
        let deps = PipelineDeps {
            provider: Arc::new(FakeProvider),
            inserter: ins.clone(),
            store: store.clone(),
            polish: Some(polish),
        };
        (deps, ins, store)
    }

    fn ctx_enabled(mode: PolishMode) -> PolishContext {
        PolishContext {
            enabled: true,
            mode,
            style_prompt: None,
            hotwords: vec![],
            timeout_ms: 1000,
        }
    }

    #[tokio::test]
    async fn l0_cleanup_runs_even_when_polish_disabled() {
        // 总开关关闭 / 无 provider：L0 规则层仍生效（去填充词 + 补句号）。
        let (deps, ins, _store) = deps(); // polish: None
        let pipe = Pipeline::new(deps);
        let ctx = PolishContext {
            enabled: false,
            ..Default::default()
        };
        pipe.insert_finals_with_polish("s1", &["嗯那个今天天气不错".into()], &ctx)
            .await
            .unwrap();
        let out = ins.out.lock().unwrap().clone();
        assert!(
            out.contains("今天天气不错"),
            "L0 应去掉首部填充词，得到 {out}"
        );
        assert!(!out.ends_with('。'), "B4：单句输入不应补句号，得到 {out}");
    }

    #[tokio::test]
    async fn l2_skipped_when_l0_result_short() {
        // L0 结果 ≤8 字 → 不调用 LLM（调研 6.3：过度纠正 + 延迟不值得）。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok("不该出现".into())));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish("s1", &["你好".into()], &ctx_enabled(PolishMode::Light))
            .await
            .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "你好");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "短句(≤8字)不应调用 L2");
    }

    #[tokio::test]
    async fn l2_used_for_long_text_and_inserts_corrected() {
        // 长句 → L2 校对，返回纠正文本 → 上屏纠正后结果。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok(
            "我们下午在会议室见面吧".into(),
        )));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["我们下午在会试室见面吧".into()],
            &ctx_enabled(PolishMode::Light),
        )
        .await
        .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "我们下午在会议室见面吧");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn l2_error_falls_back_to_l0() {
        // L2 失败 → 不阻断，回退 L0 结果上屏。
        let mock = Arc::new(MockPolish::new(MockBehavior::Err));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["我们下午在会试室见面吧".into()],
            &ctx_enabled(PolishMode::Light),
        )
        .await
        .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "我们下午在会试室见面吧");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn l2_empty_output_falls_back_to_l0() {
        // L2 返回空串 → 视同无效，回退 L0。
        let mock = Arc::new(MockPolish::new(MockBehavior::Empty));
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish(
            "s1",
            &["我们下午在会试室见面吧".into()],
            &ctx_enabled(PolishMode::Light),
        )
        .await
        .unwrap();
        assert_eq!(*ins.out.lock().unwrap(), "我们下午在会试室见面吧");
    }

    #[tokio::test]
    async fn empty_final_is_skipped() {
        // 空 final 不应触发 polish，也不上屏。
        let mock = Arc::new(MockPolish::new(MockBehavior::Ok("x".into())));
        let calls = mock.calls.clone();
        let (deps, ins, _store) = deps_with_polish(mock);
        let pipe = Pipeline::new(deps);
        pipe.insert_finals_with_polish("s1", &["".into()], &ctx_enabled(PolishMode::Light))
            .await
            .unwrap();
        assert!(ins.out.lock().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
