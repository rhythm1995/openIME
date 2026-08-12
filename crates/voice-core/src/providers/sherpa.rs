//! 本地 sherpa-onnx ASR provider（Paraformer-online + Silero VAD，离线）。
//!
//! - 开启 `sherpa` feature：进程内推理。`connect()` 建立 recognizer + VAD，
//!   spawn 一个驱动线程：从音频 channel 收 f32 样本 → VAD 切片 → recognizer 解码 →
//!   产 partial/final → 推 deltas channel。`feed` 推帧、`finish` 关闭音频 channel
//!   使驱动线程自然结束并关闭 deltas 流。
//! - 默认（未开 feature）：返回 `Err`，引导选用云端或开启 feature 重新编译。
//!
//! 模型路径约定（与 model_mgr 配合）：
//!   {model_root}/{model_name}/{encoder.int8.onnx, decoder.int8.onnx, tokens.txt}
//!   {vad_root}/silero_vad.onnx

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use futures::stream::Stream;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::asr_catalog::{
    ASR_MODEL_FIRERED_LARGE, ASR_MODEL_FUNASR_NANO_FP16, ASR_MODEL_FUNASR_NANO_INT8,
    ASR_MODEL_SENSEVOICE,
};
use crate::config::ProviderKind;
use crate::model_download::normalize_asr_model_id;
use crate::traits::{AsrProvider, AsrSession, AudioFrame, TranscriptDelta};
use crate::{Error, ProviderConfig};

/// 流式模型路径（Paraformer 或 Zipformer transducer）。
#[derive(Debug, Clone)]
pub struct SherpaModelPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    /// Zipformer transducer 需要 joiner；Paraformer 可空路径。
    pub joiner: Option<PathBuf>,
    pub tokens: PathBuf,
    pub vad_model: PathBuf,
    pub backend: StreamingBackend,
}

/// 流式后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingBackend {
    Paraformer,
    ZipformerTransducer,
}

/// SenseVoice 离线模型路径。
#[derive(Debug, Clone)]
pub struct SenseVoicePaths {
    pub model: PathBuf,
    pub tokens: PathBuf,
}

impl SenseVoicePaths {
    pub fn from_dirs(model_dir: &Path) -> Self {
        Self {
            model: model_dir.join("model.int8.onnx"),
            tokens: model_dir.join("tokens.txt"),
        }
    }
    pub fn validate(&self) -> crate::Result<()> {
        for (name, p) in [("model", &self.model), ("tokens", &self.tokens)] {
            if !p.exists() {
                return Err(Error::Provider(format!(
                    "SenseVoice 模型文件缺失（{}）：{}",
                    name,
                    p.display()
                )));
            }
        }
        Ok(())
    }
}

/// FireRedASR 离线路径（encoder + decoder + tokens）。
#[derive(Debug, Clone)]
pub struct FireRedAsrPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub tokens: PathBuf,
}

impl FireRedAsrPaths {
    pub fn from_dirs(model_dir: &Path) -> Self {
        Self {
            encoder: model_dir.join("encoder.int8.onnx"),
            decoder: model_dir.join("decoder.int8.onnx"),
            tokens: model_dir.join("tokens.txt"),
        }
    }
    pub fn validate(&self) -> crate::Result<()> {
        for (name, p) in [
            ("encoder", &self.encoder),
            ("decoder", &self.decoder),
            ("tokens", &self.tokens),
        ] {
            if !p.exists() {
                return Err(Error::Provider(format!(
                    "FireRedASR 模型文件缺失（{}）：{}",
                    name,
                    p.display()
                )));
            }
        }
        Ok(())
    }
}

/// 离线后端种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfflineBackend {
    SenseVoice,
    FireRed,
    /// FunASR Nano（encoder+LLM 混合）。variant 0 = int8, 1 = fp16。
    FunAsrNano(FunasrNanoQuant),
}

/// FunASR Nano 量化变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunasrNanoQuant {
    Int8,
    Fp16,
}

impl SherpaModelPaths {
    /// 旧流式 Paraformer 路径。
    pub fn paraformer_from_dirs(model_dir: &Path, vad_dir: &Path) -> Self {
        Self {
            encoder: model_dir.join("encoder.int8.onnx"),
            decoder: model_dir.join("decoder.int8.onnx"),
            joiner: None,
            tokens: model_dir.join("tokens.txt"),
            vad_model: vad_dir.join("silero_vad.onnx"),
            backend: StreamingBackend::Paraformer,
        }
    }

    /// Zipformer 2025 流式 transducer 路径（decoder 为 fp32 decoder.onnx）。
    pub fn zipformer_from_dirs(model_dir: &Path, vad_dir: &Path) -> Self {
        Self {
            encoder: model_dir.join("encoder.int8.onnx"),
            decoder: model_dir.join("decoder.onnx"),
            joiner: Some(model_dir.join("joiner.int8.onnx")),
            tokens: model_dir.join("tokens.txt"),
            vad_model: vad_dir.join("silero_vad.onnx"),
            backend: StreamingBackend::ZipformerTransducer,
        }
    }

    /// 兼容旧调用名。
    pub fn from_dirs(model_dir: &Path, vad_dir: &Path) -> Self {
        Self::paraformer_from_dirs(model_dir, vad_dir)
    }

    /// 校验所有文件存在。
    pub fn validate(&self) -> crate::Result<()> {
        let mut list: Vec<(&str, &PathBuf)> = vec![
            ("encoder", &self.encoder),
            ("decoder", &self.decoder),
            ("tokens", &self.tokens),
            ("vad_model", &self.vad_model),
        ];
        if let Some(ref j) = self.joiner {
            list.push(("joiner", j));
        }
        for (name, p) in list {
            if !p.exists() {
                return Err(Error::Provider(format!(
                    "sherpa 模型文件缺失（{}）：{}",
                    name,
                    p.display()
                )));
            }
        }
        Ok(())
    }
}

pub struct SherpaProvider {
    /// 模型根目录（含各模型子目录）+ VAD 模型目录。
    /// 连接时按 cfg.model 作为子目录名解析完整路径。
    paths_root: Option<(PathBuf, PathBuf)>,
}

impl SherpaProvider {
    /// 创建一个"纯 stub"provider（未配路径），connect 会返回引导错误。
    pub fn new() -> Self {
        Self { paths_root: None }
    }

    /// 创建带模型根目录的 provider。`model_root` 含各模型子目录，
    /// `vad_root` 含 silero_vad.onnx。
    pub fn with_root(model_root: PathBuf, vad_root: PathBuf) -> Self {
        Self {
            paths_root: Some((model_root, vad_root)),
        }
    }

    #[allow(dead_code)]
    fn resolve_paths(&self, model_name: &str) -> crate::Result<SherpaModelPaths> {
        let (model_root, vad_root) = self
            .paths_root
            .as_ref()
            .ok_or_else(|| Error::Config("SherpaProvider 未配置模型根目录".into()))?;
        Ok(SherpaModelPaths::from_dirs(
            &model_root.join(model_name),
            vad_root,
        ))
    }
}

impl Default for SherpaProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AsrProvider for SherpaProvider {
    async fn connect(&self, cfg: &ProviderConfig) -> crate::Result<Box<dyn AsrSession>> {
        if !matches!(cfg.kind, ProviderKind::Sherpa) {
            return Err(Error::Config(format!(
                "SherpaProvider 收到非 sherpa 配置: {:?}",
                cfg.kind
            )));
        }
        cfg.validate()?;
        let (model_root, vad_root) = self
            .paths_root
            .as_ref()
            .ok_or_else(|| Error::Config("SherpaProvider 未配置模型根目录".into()))?;

        // 语言：provider.language > 空→zh（由 sync_local_asr_fields 同步）。
        let lang = cfg
            .language
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("zh");

        // 按 model id 分流：
        // - sensevoice → Offline SenseVoice
        // - firered-large → Offline FireRedASR
        // - 其它目录名 → 旧流式 Paraformer（bilingual 等，encoder+decoder.int8）
        let model_key = cfg
            .model
            .strip_prefix("offline:")
            .unwrap_or(cfg.model.as_str());
        let model_id = normalize_asr_model_id(model_key);

        if cfg.model.starts_with("offline:") || model_id == ASR_MODEL_SENSEVOICE {
            connect_offline_with_paths(model_root, OfflineBackend::SenseVoice, lang).await
        } else if model_id == ASR_MODEL_FIRERED_LARGE {
            connect_offline_with_paths(model_root, OfflineBackend::FireRed, lang).await
        } else if model_id == ASR_MODEL_FUNASR_NANO_INT8 {
            connect_offline_with_paths(
                model_root,
                OfflineBackend::FunAsrNano(FunasrNanoQuant::Int8),
                lang,
            )
            .await
        } else if model_id == ASR_MODEL_FUNASR_NANO_FP16 {
            connect_offline_with_paths(
                model_root,
                OfflineBackend::FunAsrNano(FunasrNanoQuant::Fp16),
                lang,
            )
            .await
        } else {
            let paths =
                SherpaModelPaths::paraformer_from_dirs(&model_root.join(&cfg.model), vad_root);
            connect_with_paths(cfg, &paths).await
        }
    }
}

// ──────────────── feature = "sherpa"：真实推理 ────────────────

#[cfg(feature = "sherpa")]
pub(crate) mod engine {
    use super::*;
    use crate::traits::TranscriptKind;
    use sherpa_onnx::{
        OfflineFireRedAsrModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
        OfflineSenseVoiceModelConfig, OnlineParaformerModelConfig, OnlineRecognizer,
        OnlineRecognizerConfig, OnlineTransducerModelConfig, SileroVadModelConfig, VadModelConfig,
        VoiceActivityDetector,
    };

    /// 构造 recognizer 配置（Paraformer / Zipformer transducer）。
    #[allow(clippy::field_reassign_with_default)]
    pub fn build_recognizer_config(paths: &SherpaModelPaths) -> OnlineRecognizerConfig {
        let mut cfg = OnlineRecognizerConfig::default();
        match paths.backend {
            StreamingBackend::Paraformer => {
                cfg.model_config.paraformer = OnlineParaformerModelConfig {
                    encoder: Some(paths.encoder.to_string_lossy().into_owned()),
                    decoder: Some(paths.decoder.to_string_lossy().into_owned()),
                };
            }
            StreamingBackend::ZipformerTransducer => {
                cfg.model_config.transducer = OnlineTransducerModelConfig {
                    encoder: Some(paths.encoder.to_string_lossy().into_owned()),
                    decoder: Some(paths.decoder.to_string_lossy().into_owned()),
                    joiner: paths
                        .joiner
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned()),
                };
            }
        }
        cfg.model_config.tokens = Some(paths.tokens.to_string_lossy().into_owned());
        cfg.model_config.num_threads = 2;
        cfg.model_config.provider = Some("cpu".into());
        cfg.model_config.debug = false;
        cfg.feat_config.sample_rate = 16_000;
        cfg.feat_config.feature_dim = 80;
        cfg.decoding_method = Some("greedy_search".into());
        cfg.enable_endpoint = true;
        // 端点规则（秒）：尾部静音切句、最大句长。
        cfg.rule1_min_trailing_silence = 2.4;
        cfg.rule2_min_trailing_silence = 1.2;
        cfg.rule3_min_utterance_length = 20.0;
        cfg
    }

    /// 构造 VAD 配置（纯函数，可测）。
    #[allow(clippy::field_reassign_with_default)]
    pub fn build_vad_config(paths: &SherpaModelPaths) -> VadModelConfig {
        let mut silero = SileroVadModelConfig::default();
        silero.model = Some(paths.vad_model.to_string_lossy().into_owned());
        silero.threshold = 0.5;
        silero.min_silence_duration = 0.25;
        silero.min_speech_duration = 0.25;
        silero.max_speech_duration = 20.0;
        silero.window_size = 512;
        VadModelConfig {
            silero_vad: silero,
            ten_vad: Default::default(),
            sample_rate: 16_000,
            num_threads: 1,
            provider: Some("cpu".into()),
            debug: false,
        }
    }

    /// 驱动线程：从 audio_rx 收 f32 样本 → VAD 切片 → recognizer 解码 → 推 delta。
    /// recognizer/VAD 非 Send（含 ONNX runtime 状态），故整个循环跑在专用 OS 线程。
    pub fn run_engine(
        recognizer: OnlineRecognizer,
        vad: VoiceActivityDetector,
        audio_rx: std::sync::mpsc::Receiver<Vec<f32>>,
        deltas_tx: mpsc::UnboundedSender<crate::Result<TranscriptDelta>>,
    ) {
        let stream = recognizer.create_stream();
        let mut sentence_index: u32 = 0;
        const WINDOW: usize = 512; // silero 16kHz 窗口
        let mut buf: Vec<f32> = Vec::new();

        // 每收到一批样本，累积到 VAD 窗口整数倍后送 VAD；VAD 输出语音段送 recognizer。
        while let Ok(samples) = audio_rx.recv() {
            buf.extend_from_slice(&samples);

            // 按 VAD 窗口切片喂 VAD。
            while buf.len() >= WINDOW {
                let window: Vec<f32> = buf.drain(..WINDOW).collect();
                vad.accept_waveform(&window);
                // 取出 VAD 识别的语音段，送 recognizer 解码。
                while let Some(seg) = vad.front() {
                    let speech = seg.samples().to_vec();
                    vad.pop();
                    process_segment(
                        &recognizer,
                        &stream,
                        &speech,
                        &deltas_tx,
                        &mut sentence_index,
                    );
                }
            }

            // 同时对当前缓冲做一次 endpoint 检查（无 VAD 时也可切句）。
            push_partial(&recognizer, &stream, &deltas_tx, sentence_index);
            if recognizer.is_endpoint(&stream) {
                if let Some(r) = recognizer.get_result(&stream) {
                    if !r.text.is_empty() {
                        let _ = deltas_tx.send(Ok(TranscriptDelta::final_(r.text, sentence_index)));
                        sentence_index += 1;
                    }
                }
                recognizer.reset(&stream);
            }
        }

        // 录音结束：flush VAD + 尾部零填充解码剩余。
        vad.flush();
        while let Some(seg) = vad.front() {
            let speech = seg.samples().to_vec();
            vad.pop();
            process_segment(
                &recognizer,
                &stream,
                &speech,
                &deltas_tx,
                &mut sentence_index,
            );
        }
        let tail = vec![0.0f32; 4800]; // 0.3s @16k
        stream.accept_waveform(16_000, &tail);
        stream.input_finished();
        while recognizer.is_ready(&stream) {
            recognizer.decode(&stream);
        }
        if let Some(r) = recognizer.get_result(&stream) {
            if !r.text.is_empty() {
                let _ = deltas_tx.send(Ok(TranscriptDelta::final_(r.text, sentence_index)));
            }
        }
        // deltas_tx drop 后，UnboundedReceiverStream 自然结束。
    }

    fn process_segment(
        recognizer: &OnlineRecognizer,
        stream: &sherpa_onnx::OnlineStream,
        speech: &[f32],
        deltas_tx: &mpsc::UnboundedSender<crate::Result<TranscriptDelta>>,
        sentence_index: &mut u32,
    ) {
        if speech.is_empty() {
            return;
        }
        stream.accept_waveform(16_000, speech);
        while recognizer.is_ready(stream) {
            recognizer.decode(stream);
        }
        push_partial(recognizer, stream, deltas_tx, *sentence_index);
        if recognizer.is_endpoint(stream) {
            if let Some(r) = recognizer.get_result(stream) {
                if !r.text.is_empty() {
                    let _ = deltas_tx.send(Ok(TranscriptDelta::final_(r.text, *sentence_index)));
                    *sentence_index += 1;
                }
            }
            recognizer.reset(stream);
        }
    }

    fn push_partial(
        recognizer: &OnlineRecognizer,
        stream: &sherpa_onnx::OnlineStream,
        deltas_tx: &mpsc::UnboundedSender<crate::Result<TranscriptDelta>>,
        sentence_index: u32,
    ) {
        if let Some(r) = recognizer.get_result(stream) {
            if !r.text.is_empty() {
                let _ = deltas_tx.send(Ok(TranscriptDelta {
                    kind: TranscriptKind::Partial,
                    text: r.text,
                    sentence_index,
                }));
            }
        }
    }

    // newtype 包装：让外部 sherpa-onnx 类型可跨线程 move 到驱动线程。
    // Sherpa 的 recognizer/stream 内部含 ONNX runtime 状态，本身未必 Send；
    // 这里用 newtype + unsafe impl Send 在本 crate 内声明"我们将独占地在单一驱动线程内使用它"。
    struct SendRecognizer(OnlineRecognizer);
    unsafe impl Send for SendRecognizer {}
    struct SendVad(VoiceActivityDetector);
    unsafe impl Send for SendVad {}
    struct SendOfflineRecognizer(OfflineRecognizer);
    unsafe impl Send for SendOfflineRecognizer {}

    /// 建立 recognizer + VAD 并 spawn 驱动线程，返回 SherpaSession。
    pub async fn connect_with_paths(
        cfg: &ProviderConfig,
        paths: &SherpaModelPaths,
    ) -> crate::Result<Box<dyn super::AsrSession>> {
        paths.validate()?;

        let recognizer = {
            let rc = build_recognizer_config(paths);
            OnlineRecognizer::create(&rc)
                .map(SendRecognizer)
                .ok_or_else(|| Error::Provider("创建 OnlineRecognizer 失败".into()))?
        };
        let vad = {
            let vc = build_vad_config(paths);
            VoiceActivityDetector::create(&vc, 30.0)
                .map(SendVad)
                .ok_or_else(|| Error::Provider("创建 VoiceActivityDetector 失败".into()))?
        };

        let (audio_tx, audio_rx) = std::sync::mpsc::channel::<Vec<f32>>();
        let (dtx, drx) = mpsc::unbounded_channel::<crate::Result<TranscriptDelta>>();

        // spawn 驱动 OS 线程：独占 recognizer/vad/stream。
        std::thread::Builder::new()
            .name("sherpa-engine".into())
            .spawn(move || run_engine(recognizer.0, vad.0, audio_rx, dtx))
            .map_err(|e| Error::Provider(format!("启动 sherpa 线程失败: {e}")))?;

        let _ = cfg;
        Ok(Box::new(super::SherpaSession {
            audio_tx: Some(audio_tx),
            deltas_rx: Some(drx),
            finished: false,
        }))
    }

    // ──────────────── 离线模式：SenseVoice / FireRedASR ────────────────

    /// 构造 OfflineRecognizer 配置（SenseVoice）。
    #[allow(clippy::field_reassign_with_default)]
    pub fn build_sensevoice_config(
        sv: &super::SenseVoicePaths,
        language: &str,
    ) -> OfflineRecognizerConfig {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(sv.model.to_string_lossy().into_owned()),
            language: Some(normalize_language(language)),
            use_itn: true,
        };
        cfg.model_config.tokens = Some(sv.tokens.to_string_lossy().into_owned());
        cfg.model_config.num_threads = 2;
        cfg.model_config.provider = Some("cpu".into());
        cfg.model_config.debug = false;
        cfg.feat_config.sample_rate = 16_000;
        cfg.feat_config.feature_dim = 80;
        cfg
    }

    /// 构造 OfflineRecognizer 配置（FireRedASR AED）。
    #[allow(clippy::field_reassign_with_default)]
    pub fn build_firered_config(fr: &super::FireRedAsrPaths) -> OfflineRecognizerConfig {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.fire_red_asr = OfflineFireRedAsrModelConfig {
            encoder: Some(fr.encoder.to_string_lossy().into_owned()),
            decoder: Some(fr.decoder.to_string_lossy().into_owned()),
        };
        cfg.model_config.tokens = Some(fr.tokens.to_string_lossy().into_owned());
        // 大模型略增线程，仍受 CPU 限制。
        cfg.model_config.num_threads = 4;
        cfg.model_config.provider = Some("cpu".into());
        cfg.model_config.debug = false;
        cfg.feat_config.sample_rate = 16_000;
        cfg.feat_config.feature_dim = 80;
        cfg
    }

    fn normalize_language(lang: &str) -> String {
        match lang.trim().to_lowercase().as_str() {
            "zh" | "zh-cn" | "zh_cn" | "中文" => "zh".into(),
            "en" | "英文" => "en".into(),
            "yue" | "粤语" | "cantonese" | "zh-yue" => "yue".into(),
            _ => "auto".into(),
        }
    }

    /// 构造 OfflineRecognizer 配置（FunASR Nano，encoder+LLM 混合）。
    /// embedding/encoder_adaptor/llm 是三个 onnx 文件；tokenizer 指向 Qwen3-0.6B 目录。
    #[allow(clippy::field_reassign_with_default)]
    pub fn build_funasr_nano_config(
        embedding: &Path,
        encoder_adaptor: &Path,
        llm: &Path,
        tokenizer_dir: &Path,
        language: &str,
    ) -> OfflineRecognizerConfig {
        use sherpa_onnx::{OfflineFunASRNanoModelConfig, OfflineModelConfig};
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.funasr_nano = OfflineFunASRNanoModelConfig {
            embedding: Some(embedding.to_string_lossy().into_owned()),
            encoder_adaptor: Some(encoder_adaptor.to_string_lossy().into_owned()),
            llm: Some(llm.to_string_lossy().into_owned()),
            tokenizer: Some(tokenizer_dir.to_string_lossy().into_owned()),
            // itn=1 开启内置逆文本归一化（数字/日期/单位等规范化输出）。
            itn: 1,
            language: Some(normalize_language(language)),
            ..Default::default()
        };
        cfg.model_config.num_threads = 4;
        cfg.model_config.provider = Some("cpu".into());
        cfg.model_config.debug = false;
        cfg.feat_config.sample_rate = 16_000;
        cfg.feat_config.feature_dim = 80;
        let _ = OfflineModelConfig::default(); // 抑制未用 import 警告
        cfg
    }

    // ── D3 文件转录：独立 OfflineRecognizer（不走 session/audio channel）──

    /// 按 model_id 构建 OfflineRecognizer 配置（复用 connect_offline 的路径逻辑）。
    #[cfg(feature = "sherpa")]
    pub(crate) fn build_offline_config(
        model_root: &Path,
        model_id: &str,
        lang: &str,
    ) -> crate::Result<OfflineRecognizerConfig> {
        use crate::asr_catalog::{
            ASR_MODEL_FIRERED_LARGE, ASR_MODEL_FUNASR_NANO_FP16, ASR_MODEL_FUNASR_NANO_INT8,
            ASR_MODEL_SENSEVOICE, FIRERED_LARGE_DIR, FUNASR_NANO_FP16_DIR, FUNASR_NANO_INT8_DIR,
        };
        use crate::model_download::SENSEVOICE_MODEL_NAME;

        let cfg = if model_id == ASR_MODEL_SENSEVOICE {
            let sv = super::SenseVoicePaths::from_dirs(&model_root.join(SENSEVOICE_MODEL_NAME));
            sv.validate()?;
            build_sensevoice_config(&sv, lang)
        } else if model_id == ASR_MODEL_FIRERED_LARGE {
            let fr = super::FireRedAsrPaths::from_dirs(&model_root.join(FIRERED_LARGE_DIR));
            fr.validate()?;
            build_firered_config(&fr)
        } else if model_id == ASR_MODEL_FUNASR_NANO_INT8 || model_id == ASR_MODEL_FUNASR_NANO_FP16 {
            let dir = if model_id == ASR_MODEL_FUNASR_NANO_INT8 {
                FUNASR_NANO_INT8_DIR
            } else {
                FUNASR_NANO_FP16_DIR
            };
            let model_dir = model_root.join(dir);
            let tokenizer_dir = model_dir.join("Qwen3-0.6B");
            let llm_name = if model_id == ASR_MODEL_FUNASR_NANO_INT8 {
                "llm.int8.onnx"
            } else {
                "llm.fp16.onnx"
            };
            for (name, p) in [
                ("embedding", model_dir.join("embedding.int8.onnx")),
                (
                    "encoder_adaptor",
                    model_dir.join("encoder_adaptor.int8.onnx"),
                ),
                ("llm", model_dir.join(llm_name)),
                ("tokenizer_dir", tokenizer_dir.clone()),
            ] {
                if !p.exists() {
                    return Err(Error::Provider(format!(
                        "模型文件缺失（{name}）：{}",
                        p.display()
                    )));
                }
            }
            let mut c = build_funasr_nano_config(
                &model_dir.join("embedding.int8.onnx"),
                &model_dir.join("encoder_adaptor.int8.onnx"),
                &model_dir.join(llm_name),
                &tokenizer_dir,
                lang,
            );
            c.model_config.num_threads = 2;
            c
        } else {
            return Err(Error::Provider(format!("未知离线模型 id：{model_id}")));
        };
        Ok(cfg)
    }

    /// D3：按 model_id 创建 OfflineRecognizer（独立于 session）。
    #[cfg(feature = "sherpa")]
    pub fn build_offline_recognizer(
        model_root: &Path,
        model_id: &str,
        lang: &str,
    ) -> crate::Result<OfflineRecognizer> {
        let cfg = build_offline_config(model_root, model_id, lang)?;
        OfflineRecognizer::create(&cfg)
            .ok_or_else(|| Error::Provider("创建 OfflineRecognizer 失败".into()))
    }

    /// D3：OfflineRecognizer 整段 decode（文件转录）。
    #[cfg(feature = "sherpa")]
    pub fn transcribe_offline(recognizer: &OfflineRecognizer, samples: &[f32]) -> String {
        let stream = recognizer.create_stream();
        stream.accept_waveform(16_000, samples);
        recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_string())
            .unwrap_or_default()
    }

    /// 离线模式：Fn 按下录音→松开时整段送 OfflineRecognizer 解码。
    pub async fn connect_offline_with_paths(
        model_root: &Path,
        backend: super::OfflineBackend,
        language: &str,
    ) -> crate::Result<Box<dyn super::AsrSession>> {
        let lang = language;
        let mut cfg = match backend {
            super::OfflineBackend::SenseVoice => {
                let sv = super::SenseVoicePaths::from_dirs(
                    &model_root.join(crate::model_download::SENSEVOICE_MODEL_NAME),
                );
                sv.validate()?;
                build_sensevoice_config(&sv, lang)
            }
            super::OfflineBackend::FireRed => {
                let fr = super::FireRedAsrPaths::from_dirs(
                    &model_root.join(crate::asr_catalog::FIRERED_LARGE_DIR),
                );
                fr.validate()?;
                build_firered_config(&fr)
            }
            super::OfflineBackend::FunAsrNano(quant) => {
                let dir = match quant {
                    super::FunasrNanoQuant::Int8 => crate::asr_catalog::FUNASR_NANO_INT8_DIR,
                    super::FunasrNanoQuant::Fp16 => crate::asr_catalog::FUNASR_NANO_FP16_DIR,
                };
                let model_dir = model_root.join(dir);
                let tokenizer_dir = model_dir.join("Qwen3-0.6B");
                let llm_name = match quant {
                    super::FunasrNanoQuant::Int8 => "llm.int8.onnx",
                    super::FunasrNanoQuant::Fp16 => "llm.fp16.onnx",
                };
                // 校验文件存在。
                for (name, p) in [
                    ("embedding", model_dir.join("embedding.int8.onnx")),
                    (
                        "encoder_adaptor",
                        model_dir.join("encoder_adaptor.int8.onnx"),
                    ),
                    ("llm", model_dir.join(llm_name)),
                    ("tokenizer_dir", tokenizer_dir.clone()),
                ] {
                    if !p.exists() {
                        return Err(Error::Provider(format!(
                            "FunASR Nano 模型文件缺失（{name}）：{}",
                            p.display()
                        )));
                    }
                }
                build_funasr_nano_config(
                    &model_dir.join("embedding.int8.onnx"),
                    &model_dir.join("encoder_adaptor.int8.onnx"),
                    &model_dir.join(llm_name),
                    &tokenizer_dir,
                    lang,
                )
            }
        };

        let label = match backend {
            super::OfflineBackend::SenseVoice => "SenseVoice",
            super::OfflineBackend::FireRed => "FireRedASR",
            super::OfflineBackend::FunAsrNano(quant) => match quant {
                super::FunasrNanoQuant::Int8 => "FunASR Nano int8",
                super::FunasrNanoQuant::Fp16 => "FunASR Nano fp16",
            },
        };
        // FunASR Nano int8/fp16 在高分模型上容易吃满，降一线程换稳定性（与 FireRed 路径一致）
        if matches!(backend, super::OfflineBackend::FunAsrNano(_)) {
            cfg.model_config.num_threads = 2;
        }

        let recognizer = OfflineRecognizer::create(&cfg)
            .ok_or_else(|| Error::Provider(format!("创建 OfflineRecognizer 失败（{label}）")))?;
        let recognizer = SendOfflineRecognizer(recognizer);

        let (dtx, drx) = mpsc::unbounded_channel::<crate::Result<TranscriptDelta>>();
        let (audio_tx, audio_rx) = std::sync::mpsc::channel::<Vec<f32>>();

        std::thread::Builder::new()
            .name("sherpa-offline".into())
            .spawn(move || {
                let recognizer = recognizer.0;
                let mut all_samples: Vec<f32> = Vec::new();
                while let Ok(samples) = audio_rx.recv() {
                    all_samples.extend_from_slice(&samples);
                }
                if all_samples.is_empty() {
                    let _ = dtx.send(Ok(TranscriptDelta::final_("", 0)));
                    return;
                }
                let stream = recognizer.create_stream();
                stream.accept_waveform(16_000, &all_samples);
                recognizer.decode(&stream);
                if let Some(result) = stream.get_result() {
                    let text = result.text.trim().to_string();
                    let _ = dtx.send(Ok(TranscriptDelta::final_(&text, 0)));
                } else {
                    let _ = dtx.send(Ok(TranscriptDelta::final_("", 0)));
                }
            })
            .map_err(|e| Error::Provider(format!("启动 offline 线程失败: {e}")))?;

        Ok(Box::new(super::SherpaSession {
            audio_tx: Some(audio_tx),
            deltas_rx: Some(drx),
            finished: false,
        }))
    }
}

#[cfg(feature = "sherpa")]
pub use engine::{connect_offline_with_paths, connect_with_paths};

/// 默认 stub（未启用 sherpa feature）。
#[cfg(not(feature = "sherpa"))]
pub async fn connect_with_paths(
    _cfg: &ProviderConfig,
    _paths: &SherpaModelPaths,
) -> crate::Result<Box<dyn AsrSession>> {
    Err(Error::Provider(
        "本地 sherpa-onnx 引擎未启用：请在编译时开启 `sherpa` feature".into(),
    ))
}

#[cfg(not(feature = "sherpa"))]
pub async fn connect_offline_with_paths(
    _model_root: &Path,
    _backend: OfflineBackend,
    _language: &str,
) -> crate::Result<Box<dyn AsrSession>> {
    Err(Error::Provider(
        "本地 sherpa-onnx 引擎未启用：请在编译时开启 `sherpa` feature".into(),
    ))
}

/// 一次 sherpa 转写会话。
pub struct SherpaSession {
    audio_tx: Option<std::sync::mpsc::Sender<Vec<f32>>>,
    deltas_rx: Option<mpsc::UnboundedReceiver<crate::Result<TranscriptDelta>>>,
    finished: bool,
}

impl AsrSession for SherpaSession {
    fn feed(
        &mut self,
        frame: &AudioFrame,
    ) -> Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>> {
        let samples = s16le_to_f32_mono(&frame.bytes, frame.format.channels);
        match self.audio_tx.clone() {
            Some(tx) => Box::pin(async move {
                tx.send(samples)
                    .map_err(|_| Error::Provider("sherpa 驱动线程已退出".into()))
            }),
            None => Box::pin(async { Ok(()) }),
        }
    }

    fn finish(
        &mut self,
    ) -> Pin<Box<dyn std::future::Future<Output = crate::Result<()>> + Send + '_>> {
        Box::pin(async move {
            if !self.finished {
                self.finished = true;
                // 关键：drop 唯一的 audio_tx sender → 驱动线程 recv 返回 Err，
                // 跑收尾解码并关闭 deltas，reader 任务随通道结束。
                // （若不加这步，sender 要等本 session drop 才释放，而本 session 在
                //   reader.await 之后才 drop → 循环等待死锁，录音永不结束。）
                self.audio_tx = None;
            }
            Ok(())
        })
    }

    fn deltas(&mut self) -> Pin<Box<dyn Stream<Item = crate::Result<TranscriptDelta>> + Send>> {
        let rx = self.deltas_rx.take().expect("deltas() 只能调用一次");
        Box::pin(UnboundedReceiverStream::new(rx))
    }
}

impl Drop for SherpaSession {
    fn drop(&mut self) {
        // drop 时关闭 audio_tx（若还未关），驱动线程自然结束。
        // 显式标记，避免编译器警告。
        let _ = self.finished;
    }
}

/// LE-i16 PCM bytes → mono f32（取第一声道，归一化到 [-1,1]）。
pub fn s16le_to_f32_mono(bytes: &[u8], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let frame_bytes = 2 * ch;
    bytes
        .chunks_exact(frame_bytes)
        .map(|frame| {
            let v = i16::from_le_bytes([frame[0], frame[1]]);
            v as f32 / 32768.0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s16le_to_f32_mono_basic() {
        // mono: [0, 16384, -16384] → [0, 0.5, -0.5]
        let bytes: Vec<u8> = [0i16, 16384, -16384]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let f = s16le_to_f32_mono(&bytes, 1);
        assert!((f[0] - 0.0).abs() < 1e-4);
        assert!((f[1] - 0.5).abs() < 1e-3);
        assert!((f[2] + 0.5).abs() < 1e-3);
    }

    #[test]
    fn s16le_to_f32_mono_takes_first_channel() {
        // 2 通道：样本交错 [c0, c1, c0, c1]；取 c0。
        let stereo: Vec<u8> = [100i16, 200, 300, 400]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let f = s16le_to_f32_mono(&stereo, 2);
        assert_eq!(f.len(), 2);
        assert!((f[0] - 100.0 / 32768.0).abs() < 1e-6);
        assert!((f[1] - 300.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn model_paths_validate_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SherpaModelPaths::from_dirs(dir.path(), dir.path());
        let err = paths.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("模型文件缺失"), "got: {msg}");
    }

    #[test]
    fn model_paths_validate_all_present() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "tokens.txt",
            "silero_vad.onnx",
        ] {
            std::fs::write(dir.path().join(name), b"stub").unwrap();
        }
        let paths = SherpaModelPaths::paraformer_from_dirs(dir.path(), dir.path());
        assert!(paths.validate().is_ok());
    }
}

#[cfg(all(test, feature = "sherpa"))]
mod sherpa_engine_tests {
    use super::*;

    #[test]
    fn build_recognizer_config_sets_paraformer_paths() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "encoder.int8.onnx",
            "decoder.int8.onnx",
            "tokens.txt",
            "silero_vad.onnx",
        ] {
            std::fs::write(dir.path().join(name), b"stub").unwrap();
        }
        let paths = SherpaModelPaths::from_dirs(dir.path(), dir.path());
        let cfg = engine::build_recognizer_config(&paths);
        assert!(cfg
            .model_config
            .paraformer
            .encoder
            .as_ref()
            .unwrap()
            .ends_with("encoder.int8.onnx"));
        assert!(cfg
            .model_config
            .tokens
            .as_ref()
            .unwrap()
            .ends_with("tokens.txt"));
        assert_eq!(cfg.feat_config.sample_rate, 16_000);
        assert!(cfg.enable_endpoint);
    }

    #[test]
    fn build_vad_config_sets_silero_model() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("silero_vad.onnx"), b"stub").unwrap();
        let paths = SherpaModelPaths::from_dirs(dir.path(), dir.path());
        let vc = engine::build_vad_config(&paths);
        assert!(vc
            .silero_vad
            .model
            .as_ref()
            .unwrap()
            .ends_with("silero_vad.onnx"));
        assert_eq!(vc.sample_rate, 16_000);
    }
}
