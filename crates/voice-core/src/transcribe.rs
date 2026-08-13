//! D3 文件转录：音频文件 → symphonia 解码 → 16k mono 重采样 → sherpa OfflineRecognizer 整段转录。
//! 参考 CapsWriter `file_transcriber.py`（MIT）思路，Rust 实现。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// 线性插值重采样（f32，src_sr → dst_sr）。轻量，文件转录够用。
#[allow(dead_code)]
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

/// D3 / R12：转录音频文件 → (全文, srt)。
///
/// - 16 kHz mono 后按 `seg_secs` / `overlap_secs` 切片，同一 OfflineRecognizer 顺序喂切片。
/// - 全文用有界精确前后缀 stitch（`stitch_overlap_punct`），SRT 用未 stitch 的段文本。
/// - 一次 `build_offline_recognizer`（新实例，**不碰** OFFLINE_RECOGNIZER_CACHE），
///   顺序喂切片后 drop。
/// - `cancel`：段间检查，置位返回「转录已取消」。
#[cfg(feature = "sherpa")]
pub fn transcribe_file_full(
    path: &Path,
    model_root: &Path,
    model_id: &str,
    lang: &str,
    seg_secs: u32,
    overlap_secs: u32,
    cancel: Option<&AtomicBool>,
    on_progress: impl FnMut(usize, usize),
) -> crate::Result<(String, String)> {
    let (samples, sr) = decode_audio_file(path)?;
    let samples_16k = resample_linear(&samples, sr, 16_000);
    if samples_16k.is_empty() {
        return Err(crate::Error::Provider("音频文件为空或解码失败".into()));
    }
    let recognizer =
        crate::providers::sherpa::engine::build_offline_recognizer(model_root, model_id, lang)?;
    let result = transcribe_segmented(
        &samples_16k,
        seg_secs,
        overlap_secs,
        |piece| Ok(crate::providers::sherpa::engine::transcribe_offline(&recognizer, piece)),
        cancel,
        on_progress,
    );
    // recognizer 在函数末尾 drop（一次 build、顺序喂切片后 drop，禁止常驻缓存）。
    drop(recognizer);
    result
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

// ── R12：长音频分段 + 重叠 ──

/// 切片区间（采样点索引，[start, end)）。
///
/// - `overlap < 1 || seg <= overlap` → `Err`（与保存期校验一致的中文文案，禁止 `assert!`）。
/// - `n == 0` → 空；`n <= seg` → 单段 `[(0, n)]`。
/// - 最后一段允许短于 `seg`（自然出现），**不**并入上一段。
/// - hop = seg − overlap。
pub fn segment_ranges(n: usize, seg: usize, overlap: usize) -> crate::Result<Vec<(usize, usize)>> {
    if overlap < 1 || seg <= overlap {
        return Err(crate::Error::Config(
            "分段参数非法：须 10≤duration≤180、1≤overlap≤30 且 overlap<duration".into(),
        ));
    }
    if n == 0 {
        return Ok(vec![]);
    }
    if n <= seg {
        return Ok(vec![(0, n)]);
    }
    let hop = seg - overlap;
    let mut out = Vec::new();
    let mut start = 0;
    while start < n {
        let end = (start + seg).min(n);
        out.push((start, end));
        if end == n {
            break;
        }
        start += hop;
    }
    Ok(out)
}

const STITCH_K_MIN: usize = 2;

/// 有界精确前后缀去重：找最长 `k ∈ [k_min, max_chars]` 使 `a` 后缀 == `b` 前缀，
/// 命中返回 `a + b[k..]`；否则 `a + b`。
pub fn stitch_overlap(a: &str, b: &str, max_chars: usize) -> String {
    stitch_overlap_ex(a, b, max_chars, false)
}

/// 带标点重试的 stitch：先精确前后缀；未命中再剥两侧首尾空白与 `，。,．` 去重。
pub fn stitch_overlap_punct(a: &str, b: &str, max_chars: usize) -> String {
    let s = stitch_overlap_ex(a, b, max_chars, false);
    if s.len() == a.len() + b.len() {
        stitch_overlap_ex(a, b, max_chars, true)
    } else {
        s
    }
}

fn is_boundary_punct(c: char) -> bool {
    c.is_whitespace() || matches!(c, '，' | '。' | ',' | '.' | '．')
}

/// 返回 `chars` 末尾连续标点/空白串（按序）。
fn trailing_punct_run(chars: &[char]) -> Vec<char> {
    let mut i = chars.len();
    while i > 0 && is_boundary_punct(chars[i - 1]) {
        i -= 1;
    }
    chars[i..].to_vec()
}

/// 返回 `chars` 开头连续标点/空白串（按序）。
fn leading_punct_run(chars: &[char]) -> Vec<char> {
    let mut i = 0;
    while i < chars.len() && is_boundary_punct(chars[i]) {
        i += 1;
    }
    chars[..i].to_vec()
}

/// 最长公共「a 后缀 == b 前缀」长度，`k ∈ [k_min, max_chars]`；无 → None。
fn longest_common_overlap(a: &[char], b: &[char], k_min: usize, max_chars: usize) -> Option<usize> {
    let max_k = max_chars.min(a.len()).min(b.len());
    if max_k < k_min {
        return None;
    }
    for k in (k_min..=max_k).rev() {
        if a[a.len() - k..] == b[..k] {
            return Some(k);
        }
    }
    None
}

fn stitch_overlap_ex(a: &str, b: &str, max_chars: usize, punct_retry: bool) -> String {
    if a.is_empty() {
        return b.to_string();
    }
    if b.is_empty() {
        return a.to_string();
    }
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    if let Some(k) = longest_common_overlap(&av, &bv, STITCH_K_MIN, max_chars) {
        let cut: String = bv[k..].iter().collect();
        let mut s = String::with_capacity(a.len() + cut.len());
        s.push_str(a);
        s.push_str(&cut);
        return s;
    }
    if punct_retry {
        // 剥掉 a 尾 / b 头的连续标点+空白，只在「标点串」里再找重叠，避免误吃单字。
        let at = trailing_punct_run(&av);
        let bt = leading_punct_run(&bv);
        if let Some(k) = longest_common_overlap(&at, &bt, 1, at.len().min(bt.len())) {
            let cut: String = bv[k..].iter().collect();
            let mut s = String::with_capacity(a.len() + cut.len());
            s.push_str(a);
            s.push_str(&cut);
            return s;
        }
    }
    let mut s = String::with_capacity(a.len() + b.len());
    s.push_str(a);
    s.push_str(b);
    s
}

fn split_sentences(text: &str) -> Vec<&str> {
    text.split(['。', '！', '？', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// 单段 cue 列表（相对段首的 start/end + 文本）。
fn segment_cues(raw: &str, dur: f64) -> Vec<(f64, f64, String)> {
    let sentences = split_sentences(raw);
    if sentences.is_empty() {
        return vec![];
    }
    let total_chars: usize = sentences.iter().map(|s| s.chars().count()).sum();
    let mut out = Vec::new();
    let mut t = 0.0;
    for s in &sentences {
        let chars = s.chars().count();
        let sdur = if total_chars > 0 {
            dur * chars as f64 / total_chars as f64
        } else {
            dur / sentences.len() as f64
        };
        let start = t;
        let end = t + sdur;
        out.push((start, end, s.to_string()));
        t = end;
    }
    out
}

/// SRT 用**未 stitch** 的段文本：每段 `text_to_srt` 等价 cue 后时间戳 `+ t0`；
/// 段 `i > 0` 丢弃 `start < t0 + half_overlap` 的 cue；跨段 cue 序号连续。
pub fn srt_from_segments(segs: &[(f64, f64, String)], half_overlap: f64) -> String {
    let mut srt = String::new();
    let mut idx = 1usize;
    for (i, (t0, dur, raw)) in segs.iter().enumerate() {
        for (start, end, text) in segment_cues(raw, *dur) {
            let abs_start = t0 + start;
            let abs_end = t0 + end;
            if i > 0 && abs_start < t0 + half_overlap {
                continue;
            }
            srt.push_str(&format!("{idx}\n"));
            srt.push_str(&format!(
                "{} --> {}\n",
                fmt_srt_time(abs_start),
                fmt_srt_time(abs_end)
            ));
            srt.push_str(&format!("{text}\n\n"));
            idx += 1;
        }
    }
    srt
}

/// 分段转录（纯逻辑，可 mock `decode` 闭包做零 I/O 测试）。
///
/// 返回 `(stitched 全文, srt)`。SRT 用各段 `raw_text`，不是 stitch 后全文。
pub fn transcribe_segmented<F>(
    samples: &[f32],
    seg_secs: u32,
    overlap_secs: u32,
    mut decode: F,
    cancel: Option<&AtomicBool>,
    mut on_progress: impl FnMut(usize, usize),
) -> crate::Result<(String, String)>
where
    F: FnMut(&[f32]) -> crate::Result<String>,
{
    let seg = (seg_secs as usize) * 16_000;
    let ov = (overlap_secs as usize) * 16_000;
    let ranges = segment_ranges(samples.len(), seg, ov)?;
    let max_chars = ((overlap_secs as usize) * 8).max(8);
    let mut acc = String::new();
    let mut segs: Vec<(f64, f64, String)> = Vec::new(); // t0, dur, raw
    for (i, (s, e)) in ranges.iter().copied().enumerate() {
        if cancel.map(|c| c.load(Ordering::SeqCst)).unwrap_or(false) {
            return Err(crate::Error::Provider("转录已取消".into()));
        }
        let piece = decode(&samples[s..e])?;
        let t0 = s as f64 / 16_000.0;
        let dur = (e - s) as f64 / 16_000.0;
        segs.push((t0, dur, piece.clone()));
        acc = if i == 0 {
            piece
        } else {
            stitch_overlap_punct(&acc, &piece, max_chars)
        };
        on_progress(i + 1, ranges.len());
    }
    let half_ov = overlap_secs as f64 / 2.0;
    Ok((acc, srt_from_segments(&segs, half_ov)))
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

    // ── R12：segment_ranges ──

    #[test]
    fn segment_ranges_basic() {
        let sr = 16_000;
        assert_eq!(segment_ranges(0, 60 * sr, 4 * sr).unwrap(), vec![]);
        // 10s / 60s 都 <= 60s → 单段。
        assert_eq!(
            segment_ranges(10 * sr, 60 * sr, 4 * sr).unwrap(),
            vec![(0, 10 * sr)]
        );
        assert_eq!(
            segment_ranges(60 * sr, 60 * sr, 4 * sr).unwrap(),
            vec![(0, 60 * sr)]
        );
        // 64s → 2 段 [0,60s), [56s,64s)。
        assert_eq!(
            segment_ranges(64 * sr, 60 * sr, 4 * sr).unwrap(),
            vec![(0, 60 * sr), (56 * sr, 64 * sr)]
        );
        // 1800s → hop=56s，段数 1 + ceil((1800-60)/56) = 33。
        assert_eq!(segment_ranges(1800 * sr, 60 * sr, 4 * sr).unwrap().len(), 33);
    }

    #[test]
    fn segment_ranges_rejects_invalid() {
        assert!(segment_ranges(100, 60, 60).is_err());
        assert!(segment_ranges(100, 60, 61).is_err());
        assert!(segment_ranges(100, 60, 0).is_err());
        // 无「并入上一段」分支：短尾段自然出现，长度 > overlap 即可。
        let r = segment_ranges(64 * 16_000, 60 * 16_000, 4 * 16_000).unwrap();
        assert_eq!(r.last(), Some(&(56 * 16_000, 64 * 16_000)));
    }

    // ── R12：stitch_overlap ──

    #[test]
    fn stitch_overlap_matches_prefix_suffix() {
        assert_eq!(stitch_overlap("你好世界", "世界你好", 4), "你好世界你好");
    }

    #[test]
    fn stitch_overlap_no_common_concats() {
        assert_eq!(stitch_overlap("甲乙丙", "丁戊己", 8), "甲乙丙丁戊己");
    }

    #[test]
    fn stitch_overlap_punct_dedups_boundary_punct() {
        // A12.3b：去重句号。
        assert_eq!(stitch_overlap_punct("你好。", "。世界", 8), "你好。世界");
    }

    #[test]
    fn stitch_overlap_single_char_not_eaten() {
        // A12.3b：k_min=2 → 「的」+「的啊」不误吃，直接拼接。
        assert_eq!(stitch_overlap_punct("的", "的啊", 8), "的的啊");
        assert_eq!(stitch_overlap("的", "的啊", 8), "的的啊");
    }

    #[test]
    fn stitch_overlap_empty() {
        assert_eq!(stitch_overlap("", "世界", 8), "世界");
        assert_eq!(stitch_overlap("你好", "", 8), "你好");
        assert_eq!(stitch_overlap("", "", 8), "");
    }

    // ── R12：srt_from_segments ──

    #[test]
    fn srt_from_segments_offsets_and_drops_overlap() {
        let segs = vec![
            (0.0, 60.0, "甲。乙。".to_string()),
            (56.0, 8.0, "丙。丁。".to_string()),
        ];
        let srt = srt_from_segments(&segs, 2.0);
        // 3 cue：甲(0-30) 乙(30-60) 丁(60-64)；丙(56-60) 因 start<58 丢弃。序号连续。
        assert!(srt.contains("1\n"));
        assert!(srt.contains("2\n"));
        assert!(srt.contains("3\n"));
        assert!(!srt.contains("4\n"));
        assert!(srt.contains("甲"));
        assert!(srt.contains("乙"));
        assert!(srt.contains("丁"));
        assert!(!srt.contains("丙"));
    }

    // ── R12：transcribe_segmented（mock decode）──

    #[test]
    fn transcribe_segmented_stitches_full_text() {
        // 5s@16k，seg=2s overlap=1s → 4 段；decode 返回带重叠词，验证 stitch。
        let samples = vec![0.0f32; 80_000];
        let segs = std::cell::Cell::new(0);
        let result = transcribe_segmented(
            &samples,
            2,
            1,
            |_| {
                let i = segs.get() + 1;
                segs.set(i);
                Ok(match i {
                    1 => "X0世界".to_string(),
                    2 => "世界Y1世界".to_string(),
                    3 => "世界Y2世界".to_string(),
                    _ => "世界Y3".to_string(),
                })
            },
            None,
            |_, _| {},
        )
        .unwrap();
        // 相邻段以「世界」重叠 → 精确前后缀去重，全文连续。
        assert_eq!(result.0, "X0世界Y1世界Y2世界Y3");
        assert!(segs.get() == 4);
        assert!(result.1.contains("-->"));
    }

    #[test]
    fn transcribe_segmented_cancel_after_two_segments() {
        let samples = vec![0.0f32; 80_000];
        let cancel = AtomicBool::new(false);
        let calls = std::cell::Cell::new(0);
        let result = transcribe_segmented(
            &samples,
            2,
            1,
            |_| {
                let n = calls.get() + 1;
                calls.set(n);
                if n == 2 {
                    cancel.store(true, Ordering::SeqCst);
                }
                Ok(format!("S{n}"))
            },
            Some(&cancel),
            |_, _| {},
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("已取消"));
        assert_eq!(calls.get(), 2);
    }
}
