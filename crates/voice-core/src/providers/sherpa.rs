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

use crate::config::ProviderKind;
use crate::model_download::SENSEVOICE_MODEL_NAME;
use crate::traits::{AsrProvider, AsrSession, AudioFrame, TranscriptDelta};
use crate::{Error, ProviderConfig};

/// 解析模型文件路径。模型目录内默认用 int8 变体（更小、CPU 友好）。
#[derive(Debug, Clone)]
pub struct SherpaModelPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub tokens: PathBuf,
    pub vad_model: PathBuf,
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

impl SherpaModelPaths {
    /// 在 model_name 目录下推断路径。model 目录即 model_mgr 解压后的目标。
    pub fn from_dirs(model_dir: &Path, vad_dir: &Path) -> Self {
        Self {
            encoder: model_dir.join("encoder.int8.onnx"),
            decoder: model_dir.join("decoder.int8.onnx"),
            tokens: model_dir.join("tokens.txt"),
            vad_model: vad_dir.join("silero_vad.onnx"),
        }
    }

    /// 校验所有文件存在。
    pub fn validate(&self) -> crate::Result<()> {
        for (name, p) in [
            ("encoder", &self.encoder),
            ("decoder", &self.decoder),
            ("tokens", &self.tokens),
            ("vad_model", &self.vad_model),
        ] {
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

        // 按 local_mode 分流：offline → SenseVoice 离线解码；realtime → 流式 Paraformer。
        // mode 从 cfg.model 里的特殊前缀解析（pipeline 侧注入）。
        if cfg.model.starts_with("offline:") {
            connect_offline_with_paths(model_root, vad_root).await
        } else {
            let paths = SherpaModelPaths::from_dirs(&model_root.join(&cfg.model), vad_root);
            connect_with_paths(cfg, &paths).await
        }
    }
}

// ──────────────── feature = "sherpa"：真实推理 ────────────────

#[cfg(feature = "sherpa")]
mod engine {
    use super::*;
    use crate::traits::TranscriptKind;
    use sherpa_onnx::{
        OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig,
        OnlineParaformerModelConfig, OnlineRecognizer, OnlineRecognizerConfig,
        SileroVadModelConfig, VadModelConfig, VoiceActivityDetector,
    };

    /// 构造 recognizer 配置（纯函数，可测）。
    #[allow(clippy::field_reassign_with_default)]
    pub fn build_recognizer_config(paths: &SherpaModelPaths) -> OnlineRecognizerConfig {
        let mut cfg = OnlineRecognizerConfig::default();
        cfg.model_config.paraformer = OnlineParaformerModelConfig {
            encoder: Some(paths.encoder.to_string_lossy().into_owned()),
            decoder: Some(paths.decoder.to_string_lossy().into_owned()),
        };
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

    // ──────────────── 离线模式：SenseVoice ────────────────

    /// 构造 OfflineRecognizer 配置（SenseVoice）。
    #[allow(clippy::field_reassign_with_default)]
    pub fn build_offline_config(sv: &super::SenseVoicePaths) -> OfflineRecognizerConfig {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.sense_voice = OfflineSenseVoiceModelConfig {
            model: Some(sv.model.to_string_lossy().into_owned()),
            language: Some("auto".into()),
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

    /// 离线模式：Fn 按下录音→松开时整段送 OfflineRecognizer 解码。
    /// 音频缓冲在 session 内存，finish 时一次性解码推出结果。
    pub async fn connect_offline_with_paths(
        model_root: &Path,
        _vad_root: &Path,
    ) -> crate::Result<Box<dyn super::AsrSession>> {
        let sv_dir = model_root.join(super::SENSEVOICE_MODEL_NAME);
        let sv = super::SenseVoicePaths::from_dirs(&sv_dir);
        sv.validate()?;

        let cfg = build_offline_config(&sv);
        let recognizer = OfflineRecognizer::create(&cfg)
            .ok_or_else(|| Error::Provider("创建 OfflineRecognizer 失败".into()))?;
        let recognizer = SendOfflineRecognizer(recognizer);

        let (dtx, drx) = mpsc::unbounded_channel::<crate::Result<TranscriptDelta>>();
        let (audio_tx, audio_rx) = std::sync::mpsc::channel::<Vec<f32>>();

        // spawn 驱动线程：收完全部音频后一次性解码。
        std::thread::Builder::new()
            .name("sherpa-offline".into())
            .spawn(move || {
                let recognizer = recognizer.0;
                let mut all_samples: Vec<f32> = Vec::new();
                // 收集全部音频。
                while let Ok(samples) = audio_rx.recv() {
                    all_samples.extend_from_slice(&samples);
                }
                // audio_tx drop → recv 返回 Err → 开始解码。
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
    _vad_root: &Path,
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
        let paths = SherpaModelPaths::from_dirs(dir.path(), dir.path());
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
