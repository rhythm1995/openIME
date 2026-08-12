//! D3 文件转录：音频文件 → symphonia 解码 → 16k mono 重采样 → sherpa OfflineRecognizer 整段转录。
//! 参考 CapsWriter `file_transcriber.py`（MIT）思路，Rust 实现。

use std::path::Path;

/// 线性插值重采样（f32，src_sr → dst_sr）。轻量，文件转录够用。
fn resample_linear(samples: &[f32], src_sr: u32, dst_sr: u32) -> Vec<f32> {
    if src_sr == dst_sr || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = dst_sr as f64 / src_sr as f64;
    let dst_len = (samples.len() as f64 * ratio) as usize;
    (0..dst_len)
        .map(|i| {
            let src_idx = i as f64 / ratio;
            let idx0 = src_idx.floor() as usize;
            let idx1 = (idx0 + 1).min(samples.len() - 1);
            let frac = (src_idx - idx0 as f64) as f32;
            samples[idx0] * (1.0 - frac) + samples[idx1] * frac
        })
        .collect()
}

/// 用 symphonia 解码音频文件 → f32 mono + 采样率。
pub fn decode_audio_file(path: &Path) -> crate::Result<(Vec<f32>, u32)> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::DecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let file = std::fs::File::open(path)
        .map_err(|e| crate::Error::Store(format!("打开音频文件失败：{e}")))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| crate::Error::Provider(format!("探测音频格式失败：{e}")))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| crate::Error::Provider("音频无默认轨道".into()))?
        .clone();
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| crate::Error::Provider("音频无采样率".into()))?;
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| crate::Error::Provider(format!("创建解码器失败：{e}")))?;
    let track_id = track.id;
    let mut samples = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| crate::Error::Provider(format!("解码帧失败：{e}")))?;
        let nframes = decoded.capacity() as usize;
        let mut buf = SampleBuffer::<f32>::new(nframes as u64, *decoded.spec());
        buf.copy_interleaved_ref(decoded);
        let ch = channels.max(1);
        for chunk in buf.samples().chunks(ch) {
            let mono = chunk.iter().copied().sum::<f32>() / ch as f32;
            samples.push(mono);
        }
    }
    Ok((samples, sample_rate))
}

/// D3：整段转录音频文件 → 文本（sherpa OfflineRecognizer）。
#[cfg(feature = "sherpa")]
pub fn transcribe_file(
    path: &Path,
    model_root: &Path,
    model_id: &str,
    lang: &str,
) -> crate::Result<String> {
    let (samples, sr) = decode_audio_file(path)?;
    let samples_16k = resample_linear(&samples, sr, 16_000);
    if samples_16k.is_empty() {
        return Err(crate::Error::Provider("音频文件为空或解码失败".into()));
    }
    let recognizer =
        crate::providers::sherpa::engine::build_offline_recognizer(model_root, model_id, lang)?;
    Ok(crate::providers::sherpa::engine::transcribe_offline(
        &recognizer,
        &samples_16k,
    ))
}

/// D3：转录音频文件 → (文本, srt)（srt 按标点切分 + 按字数估算时间戳）。
#[cfg(feature = "sherpa")]
pub fn transcribe_file_full(
    path: &Path,
    model_root: &Path,
    model_id: &str,
    lang: &str,
) -> crate::Result<(String, String)> {
    let (samples, sr) = decode_audio_file(path)?;
    let samples_16k = resample_linear(&samples, sr, 16_000);
    if samples_16k.is_empty() {
        return Err(crate::Error::Provider("音频文件为空或解码失败".into()));
    }
    let duration = samples_16k.len() as f64 / 16000.0;
    let recognizer =
        crate::providers::sherpa::engine::build_offline_recognizer(model_root, model_id, lang)?;
    let text =
        crate::providers::sherpa::engine::transcribe_offline(&recognizer, &samples_16k);
    let srt = text_to_srt(&text, duration);
    Ok((text, srt))
}

/// D3：把整段文本按句号/问号/感叹号切分为 srt cue（时间戳按字数占比估算）。
pub fn text_to_srt(text: &str, total_seconds: f64) -> String {
    let sentences: Vec<&str> = text
        .split(['。', '！', '？', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if sentences.is_empty() {
        return String::new();
    }
    let total_chars: usize = sentences.iter().map(|s| s.chars().count()).sum();
    let mut srt = String::new();
    let mut idx = 1;
    let mut t = 0.0;
    for s in &sentences {
        let chars = s.chars().count();
        let dur = if total_chars > 0 {
            total_seconds * chars as f64 / total_chars as f64
        } else {
            total_seconds / sentences.len() as f64
        };
        let start = t;
        let end = t + dur;
        srt.push_str(&format!("{idx}\n"));
        srt.push_str(&format!(
            "{} --> {}\n",
            fmt_srt_time(start),
            fmt_srt_time(end)
        ));
        srt.push_str(&format!("{s}\n\n"));
        idx += 1;
        t = end;
    }
    srt
}

fn fmt_srt_time(secs: f64) -> String {
    let h = secs as u64 / 3600;
    let m = (secs as u64 % 3600) / 60;
    let s = secs as u64 % 60;
    let ms = ((secs - secs.floor()) * 1000.0) as u64;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_passthrough() {
        let s = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_linear(&s, 16000, 16000), s);
    }

    #[test]
    fn resample_downsample() {
        let s: Vec<f32> = (0..48).map(|i| i as f32).collect();
        let r = resample_linear(&s, 48000, 16000);
        assert_eq!(r.len(), 16);
    }

    #[test]
    fn resample_empty() {
        assert!(resample_linear(&[], 48000, 16000).is_empty());
    }

    #[test]
    fn srt_basic() {
        let srt = text_to_srt("你好。世界。", 10.0);
        assert!(srt.contains("1\n"));
        assert!(srt.contains("-->"));
        assert!(srt.contains("你好"));
        assert!(srt.contains("世界"));
    }
}
