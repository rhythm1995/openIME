//! 音频采集与格式转换。
//!
//! - [`resample_to_pcm_s16le`]：纯函数，把任意采样率 f32 mono 样本重采样为目标采样率的 LE-i16 字节。可单测。
//! - [`WavFixture`]：从 WAV 文件读出 16k mono 样本，测试时喂给 MockAudioSource。
//! - [`CpalAudioSource`]：真机采集（macOS CoreAudio），薄封装；靠真机手动验证。
//!
//! 一期目标格式：16kHz / mono / LE-i16（百炼 & sherpa-onnx 通用）。

use async_trait::async_trait;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use crate::traits::{AudioFormat, AudioFrame, AudioSource};
use crate::Error;

pub const TARGET: AudioFormat = AudioFormat::PCM_16K_MONO_S16LE;

/// 把 mono f32 样本（范围 [-1,1]，任意采样率）重采样为目标采样率的 LE-i16 PCM。
///
/// `input_sr` 是输入采样率；`chunk` 是一帧的 mono f32 样本。
/// 输出字节可直接作为百炼/sherpa 的音频帧。
pub fn resample_to_pcm_s16le(
    chunk: &[f32],
    input_sr: u32,
    output_sr: u32,
) -> crate::Result<Vec<u8>> {
    if input_sr == 0 || output_sr == 0 {
        return Err(Error::Audio("采样率为 0".into()));
    }
    if chunk.is_empty() {
        return Ok(Vec::new());
    }

    let samples: Vec<f32> = if input_sr == output_sr {
        chunk.to_vec()
    } else {
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };
        let mut resampler = SincFixedIn::<f32>::new(
            output_sr as f64 / input_sr as f64,
            2.0,
            params,
            chunk.len(),
            1,
        )
        .map_err(|e| Error::Audio(format!("创建重采样器失败: {e}")))?;
        let input_frames = vec![chunk.to_vec()];
        let out_frames = resampler
            .process(&input_frames, None)
            .map_err(|e| Error::Audio(format!("重采样失败: {e}")))?;
        out_frames.into_iter().next().unwrap_or_default()
    };

    Ok(f32_mono_to_s16le_bytes(&samples))
}

/// mono f32 [-1,1] → LE-i16 字节。
pub fn f32_mono_to_s16le_bytes(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * 32767.0).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// 从 WAV 文件读出 mono f32 样本（按需重采样到目标采样率）。
/// 测试用：把它切片喂给 MockAudioSource。
pub struct WavFixture {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

impl WavFixture {
    pub fn load(path: &std::path::Path) -> crate::Result<Self> {
        let mut reader = hound::WavReader::open(path)
            .map_err(|e| Error::Audio(format!("打开 WAV 失败: {e}")))?;
        let spec = reader.spec();
        let sr = spec.sample_rate;
        let channels = spec.channels as usize;
        if channels == 0 {
            return Err(Error::Audio("WAV 通道数为 0".into()));
        }

        let i16_samples: Vec<i16> = reader.samples::<i16>().filter_map(|s| s.ok()).collect();

        // 取第一声道，i16 → f32
        let mono: Vec<f32> = i16_samples
            .into_iter()
            .step_by(channels)
            .map(|v| v as f32 / 32768.0)
            .collect();

        Ok(Self {
            sample_rate: sr,
            samples: mono,
        })
    }

    /// 切成固定时长（毫秒）的 AudioFrame 序列（已是目标格式）。
    pub fn frames(&self, frame_ms: u32) -> Vec<AudioFrame> {
        let samples_per_frame = (self.sample_rate as u64 * frame_ms as u64 / 1000) as usize;
        let bytes = resample_to_pcm_s16le(&self.samples, self.sample_rate, TARGET.sample_rate)
            .unwrap_or_default();
        // 每 sample 2 字节
        let bytes_per_frame = samples_per_frame * 2;
        bytes
            .chunks(bytes_per_frame.max(1))
            .map(|c| AudioFrame::new(TARGET, c.to_vec()))
            .collect()
    }
}

/// cpal 真机采集源。
///
/// cpal 的 `Stream` 在 macOS 上非 `Send`，所以采集在专用 OS 线程里运行，
/// 通过 tokio mpsc 把 mono f32 样本传回 async 侧。`AudioSource` 的 async 方法
/// 只做接收 + 重采样，保持 `Send`。
pub struct CpalAudioSource {
    rx: tokio::sync::mpsc::UnboundedReceiver<Vec<f32>>,
    control: CpalControl,
    input_sr: u32,
    output_sr: u32,
}

/// 控制 OS 线程：start/stop。
enum CpalCmd {
    Play,
    Pause,
    Drop,
}
struct CpalControl {
    tx: std::sync::mpsc::Sender<CpalCmd>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for CpalControl {
    fn drop(&mut self) {
        let _ = self.tx.send(CpalCmd::Drop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl CpalAudioSource {
    pub fn new() -> crate::Result<Self> {
        Self::new_with_device(None)
    }

    /// 列出所有可用输入设备名（用于设置页麦克风下拉）。
    pub fn list_input_devices() -> Vec<String> {
        use cpal::traits::{DeviceTrait, HostTrait};
        cpal::default_host()
            .input_devices()
            .map(|ds| ds.filter_map(|d| d.name().ok()).collect())
            .unwrap_or_default()
    }

    /// 按设备名建立采集源；`device` 为 None / 找不到时回退默认输入设备。
    pub fn new_with_device(device: Option<String>) -> crate::Result<Self> {
        use cpal::traits::{DeviceTrait, StreamTrait};
        let host = cpal::default_host();
        let dev = Self::pick_device(&host, device.as_deref())
            .ok_or_else(|| Error::Audio("找不到输入设备".into()))?;
        let supported = dev
            .supported_input_configs()
            .map_err(|e| Error::Audio(format!("查询输入配置失败: {e}")))?
            .next()
            .ok_or_else(|| Error::Audio("无可用输入配置".into()))?;
        let config = supported.with_max_sample_rate().config();
        let input_sr = config.sample_rate.0;
        let channels = config.channels as usize;
        let dev_name = device;

        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<CpalCmd>();
        let (data_tx, data_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();

        let handle = std::thread::Builder::new()
            .name("cpal-capture".into())
            .spawn(move || {
                // 线程内重新获取设备，避免跨线程发送非 Send 的 Device。
                let host = cpal::default_host();
                let dev = match Self::pick_device(&host, dev_name.as_deref()) {
                    Some(d) => d,
                    None => return,
                };
                let cfg = match dev.default_input_config() {
                    Ok(c) => c,
                    Err(_) => return,
                };
                let stream_tx = data_tx;
                let ch = channels.max(1);
                let stream = match dev.build_input_stream(
                    &cfg.config(),
                    move |data: &[f32], _: &_| {
                        let mono: Vec<f32> = data.iter().step_by(ch).copied().collect();
                        let _ = stream_tx.send(mono);
                    },
                    |e| eprintln!("cpal error: {e}"),
                    None,
                ) {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let _ = stream.play();
                // 响应控制命令；收到 Drop 退出，stream drop 自动停止。
                while let Ok(cmd) = cmd_rx.recv() {
                    match cmd {
                        CpalCmd::Play => {
                            let _ = stream.play();
                        }
                        CpalCmd::Pause => {
                            let _ = stream.pause();
                        }
                        CpalCmd::Drop => break,
                    }
                }
            })
            .map_err(|e| Error::Audio(format!("启动采集线程失败: {e}")))?;

        Ok(Self {
            rx: data_rx,
            control: CpalControl {
                tx: cmd_tx,
                handle: Some(handle),
            },
            input_sr,
            output_sr: TARGET.sample_rate,
        })
    }

    /// 优先按名字选设备，否则用默认输入设备。
    fn pick_device(host: &cpal::Host, name: Option<&str>) -> Option<cpal::Device> {
        use cpal::traits::{DeviceTrait, HostTrait};
        if let Some(n) = name {
            if let Ok(mut ds) = host.input_devices() {
                if let Some(d) = ds.find(|d| d.name().map(|x| x == n).unwrap_or(false)) {
                    return Some(d);
                }
            }
        }
        host.default_input_device()
    }

    /// 打开设备采集约 0.6s，返回峰值振幅（0..1），用于「测试」按钮反馈。
    pub async fn test_input_level(device: Option<String>) -> crate::Result<f32> {
        let mut src = Self::new_with_device(device)?;
        src.start().await?;
        let mut peak = 0f32;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(600);
        while std::time::Instant::now() < deadline {
            if let Some(Ok(frame)) = src.next_frame().await {
                // frame 是 s16le 字节，转回 f32 估振幅。
                for chunk in frame.bytes.chunks_exact(2) {
                    let v = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
                    peak = peak.max(v.abs());
                }
            }
        }
        let _ = src.stop().await;
        Ok(peak)
    }
}

#[async_trait]
impl AudioSource for CpalAudioSource {
    async fn start(&mut self) -> crate::Result<()> {
        self.control
            .tx
            .send(CpalCmd::Play)
            .map_err(|_| Error::Audio("采集线程已退出".into()))
    }

    async fn next_frame(&mut self) -> Option<crate::Result<AudioFrame>> {
        let mono = self.rx.recv().await?;
        match resample_to_pcm_s16le(&mono, self.input_sr, self.output_sr) {
            Ok(bytes) => Some(Ok(AudioFrame::new(TARGET, bytes))),
            Err(e) => Some(Err(e)),
        }
    }

    async fn stop(&mut self) -> crate::Result<()> {
        let _ = self.control.tx.send(CpalCmd::Pause);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_to_s16le_clamps_and_converts() {
        let bytes = f32_mono_to_s16le_bytes(&[1.0, -1.0, 0.0, 0.5]);
        // 1.0 → 32767, -1.0 → -32767, 0.0 → 0, 0.5 → 16384
        let s: Vec<i16> = bytes
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(s, vec![32767, -32767, 0, 16384]);
    }

    #[test]
    fn f32_to_s16le_clamps_overflow() {
        let bytes = f32_mono_to_s16le_bytes(&[2.0, -2.0]);
        let s: Vec<i16> = bytes
            .chunks(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(s, vec![32767, -32767]);
    }

    #[test]
    fn resample_passthrough_when_same_rate() {
        let bytes = resample_to_pcm_s16le(&[0.0, 0.5, 1.0], 16000, 16000).unwrap();
        assert_eq!(bytes.len(), 6); // 3 samples * 2 bytes
    }

    #[test]
    fn resample_downsamples_48k_to_16k_proportionally() {
        // 4800 个 48k 样本下采样到 16k，输出应约为输入的 1/3。
        // rubato 的 SincFixedIn 对单次输入有内部滤波延迟，输出略少于理论值，
        // 故只断言比例落在合理区间，不要求精确等于 1600。
        let input: Vec<f32> = (0..4800).map(|i| (i as f32 / 50.0).sin()).collect();
        let bytes = resample_to_pcm_s16le(&input, 48000, 16000).unwrap();
        let n_samples = bytes.len() / 2;
        assert!(n_samples > 0, "输出为空");
        let ratio = n_samples as f64 / input.len() as f64;
        assert!(
            (0.30..=0.36).contains(&ratio),
            "下采样比例 {ratio:.3} 不在 ~1/3 区间，n_samples={n_samples}"
        );
    }

    #[test]
    fn resample_empty_input_returns_empty() {
        let bytes = resample_to_pcm_s16le(&[], 16000, 16000).unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn resample_zero_rate_errors() {
        assert!(resample_to_pcm_s16le(&[0.0], 0, 16000).is_err());
        assert!(resample_to_pcm_s16le(&[0.0], 16000, 0).is_err());
    }

    #[test]
    fn wav_fixture_frames_are_target_format() {
        // 合成一个 16k mono WAV 到临时文件再读回。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut w = hound::WavWriter::create(&path, spec).unwrap();
            for i in 0..1600 {
                // 100ms 音频
                let v = (i as f32 / 10.0).sin() * 0.3 * 32767.0;
                w.write_sample(v as i16).unwrap();
            }
            w.finalize().unwrap();
        }

        let fx = WavFixture::load(&path).unwrap();
        assert_eq!(fx.sample_rate, 16000);
        let frames = fx.frames(20); // 每 20ms
        assert!(
            frames.len() >= 4 && frames.len() <= 6,
            "frames={}",
            frames.len()
        );
        for f in &frames {
            assert_eq!(f.format, TARGET);
            assert_eq!(f.bytes.len() % 2, 0);
        }
    }
}
