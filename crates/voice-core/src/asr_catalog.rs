//! 本地 ASR 模型目录：可下载、可选中启用。
//!
//! 当前候选：
//! - `sensevoice`：SenseVoice 离线（轻量多语）
//! - `firered-large`：FireRedASR Large 离线（本地中文高精度）
//! - `funasr-nano-int8`：FunASR Nano int8 离线（encoder+LLM，方言/抗噪强）
//! - `funasr-nano-fp16`：FunASR Nano fp16 离线（同上，fp16 LLM 精度略高）
//!
//! 已移除：`paraformer-trilingual`（流式逐字上屏不稳定）/ `zipformer-zh-2025` / `zipformer-zh-xlarge`。

use serde::Serialize;

use crate::model_download::{LocalModelFile, SENSEVOICE_MODEL_NAME, VAD_DIR};

/// FireRedASR Large 中英 离线安装目录名。
pub const FIRERED_LARGE_DIR: &str = "sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16";

/// FunASR Nano int8 离线安装目录名（encoder+LLM 混合架构）。
pub const FUNASR_NANO_INT8_DIR: &str = "sherpa-onnx-funasr-nano-int8-2025-12-30";

/// FunASR Nano fp16 离线安装目录名（与 int8 共享 embedding/encoder_adaptor，仅 llm 用 fp16）。
pub const FUNASR_NANO_FP16_DIR: &str = "sherpa-onnx-funasr-nano-fp16-2025-12-30";

/// 本地 ASR 模型 id。
pub const ASR_MODEL_SENSEVOICE: &str = "sensevoice";
pub const ASR_MODEL_FIRERED_LARGE: &str = "firered-large";
pub const ASR_MODEL_FUNASR_NANO_INT8: &str = "funasr-nano-int8";
pub const ASR_MODEL_FUNASR_NANO_FP16: &str = "funasr-nano-fp16";

/// 推理后端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrBackend {
    /// Offline SenseVoice（整段解码）。
    OfflineSenseVoice,
    /// Offline FireRedASR AED（整段解码，高精度）。
    OfflineFireRed,
    /// Online Paraformer（流式逐字，FunASR）。
    StreamingParaformer,
    /// Offline FunASR Nano（encoder+LLM 混合，方言/抗噪强）。
    OfflineFunAsrNano,
}

/// 目录中一条本地 ASR 模型。
#[derive(Debug, Clone, Serialize)]
pub struct AsrModelInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// 安装子目录名（相对 model_root）。
    pub dir_name: &'static str,
    pub backend: AsrBackend,
    pub recommended: bool,
    /// 不含 VAD 的模型主体大约字节数（展示用）。
    pub approx_size: u64,
}

/// 全部候选（顺序即设置页展示顺序）。
pub fn asr_model_catalog() -> &'static [AsrModelInfo] {
    &[
        AsrModelInfo {
            id: ASR_MODEL_SENSEVOICE,
            title: "SenseVoice",
            description: "离线整段解码 · 中英日韩粤五语 · 约 240MB · 轻量首选。速度快、峰值内存小（~0.5GB），首装即用；自带标点、多语混说友好。整段解码（非逐字流式），短句即时输入够用。普通话准确率略低于 FireRedASR Large，日常聊天最均衡。",
            dir_name: SENSEVOICE_MODEL_NAME,
            backend: AsrBackend::OfflineSenseVoice,
            recommended: false,
            approx_size: 239_233_841 + 315_894,
        },
        AsrModelInfo {
            id: ASR_MODEL_FIRERED_LARGE,
            title: "FireRedASR Large",
            description: "离线整段解码 · 中英 · 约 1.7GB · 普通话最准。开源 SOTA 档（CER≈3%），适合正式文稿/会议/长句，纠错最少。AED 架构整段解码，更慢、峰值内存 ~2GB+，低配机可能卡顿。仅中英，不支持方言/日韩。追求识别率选它。",
            dir_name: FIRERED_LARGE_DIR,
            backend: AsrBackend::OfflineFireRed,
            recommended: false,
            approx_size: 1_293_430_814 + 445_469_383 + 71_448,
        },
        AsrModelInfo {
            id: ASR_MODEL_FUNASR_NANO_INT8,
            title: "FunASR Nano int8",
            description: "离线 encoder+LLM · 中(7方言)+英+日 · 约 948MB · 方言/抗噪强。int8 量化体积最小；内置 ITN（数字/日期归一化），支持吴/粤/闽/客/赣等 7 方言 + 日文，嘈杂场景稳。注意：int8 偶有重复字小瑕疵，要更高精度选 fp16 版。",
            dir_name: FUNASR_NANO_INT8_DIR,
            backend: AsrBackend::OfflineFunAsrNano,
            recommended: false,
            approx_size: 155_584_380 + 237_792_748 + 600_356_593,
        },
        AsrModelInfo {
            id: ASR_MODEL_FUNASR_NANO_FP16,
            title: "FunASR Nano fp16",
            description: "离线 encoder+LLM · 中(7方言)+英+日 · 约 1.5GB · Apple Silicon 友好。fp16 LLM 精度优于 int8、Metal 加速稳；内置 ITN，方言/抗噪强，支持日文。常说方言/粤语、或要日文、或 M 系列上求精度速度平衡选它。体积比 int8 大。",
            dir_name: FUNASR_NANO_FP16_DIR,
            backend: AsrBackend::OfflineFunAsrNano,
            recommended: false,
            approx_size: 155_584_380 + 237_792_748 + 1_192_981_814,
        },
    ]
}

pub fn asr_model_by_id(id: &str) -> Option<&'static AsrModelInfo> {
    asr_model_catalog().iter().find(|m| m.id == id)
}

/// 各 ASR 模型热词容量上限（经验值：模型越大可利用的偏置词越多）。
/// 超出部分仍存库可见，但导出给引擎时只取前 N 个对识别生效。
pub fn hotword_capacity(model_id: &str) -> usize {
    match model_id {
        ASR_MODEL_SENSEVOICE => 100,
        ASR_MODEL_FUNASR_NANO_INT8 => 200,
        ASR_MODEL_FIRERED_LARGE | ASR_MODEL_FUNASR_NANO_FP16 => 300,
        _ => 100,
    }
}

pub fn default_asr_model_id() -> &'static str {
    // 默认偏轻量：SenseVoice 适合首装（快、带标点）；用户可改选 FireRed 或 FunASR Nano。
    ASR_MODEL_SENSEVOICE
}

/// 某模型安装所需文件（含共享 VAD）。
pub fn asr_model_files(id: &str) -> Vec<LocalModelFile> {
    let mut files = match id {
        ASR_MODEL_SENSEVOICE => sensevoice_files(),
        ASR_MODEL_FIRERED_LARGE => firered_large_files(),
        ASR_MODEL_FUNASR_NANO_INT8 => funasr_nano_files(FunasrNanoVariant::Int8),
        ASR_MODEL_FUNASR_NANO_FP16 => funasr_nano_files(FunasrNanoVariant::Fp16),
        _ => Vec::new(),
    };
    files.push(vad_file());
    files
}

fn sensevoice_files() -> Vec<LocalModelFile> {
    vec![
        LocalModelFile {
            file_name: "model.int8.onnx",
            rel_dir: SENSEVOICE_MODEL_NAME,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx",
            ],
            sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
            size: 239_233_841,
        },
        LocalModelFile {
            file_name: "tokens.txt",
            rel_dir: SENSEVOICE_MODEL_NAME,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt",
            ],
            sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
            size: 315_894,
        },
    ]
}

fn firered_large_files() -> Vec<LocalModelFile> {
    // SHA256 = Git LFS oid（同上，勿用 HTTP ETag）。
    vec![
        LocalModelFile {
            file_name: "encoder.int8.onnx",
            rel_dir: FIRERED_LARGE_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16/resolve/main/encoder.int8.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16/resolve/main/encoder.int8.onnx",
            ],
            sha256: "e60cfef737a0ea324846a64eca8b9dae35898f353f4e34b62ad7e536e2d86add",
            size: 1_293_430_814,
        },
        LocalModelFile {
            file_name: "decoder.int8.onnx",
            rel_dir: FIRERED_LARGE_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16/resolve/main/decoder.int8.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16/resolve/main/decoder.int8.onnx",
            ],
            sha256: "c08b9d0297ed17ad84087085e27a4adedcc4e8b3ef14770369f1665681cc507d",
            size: 445_469_383,
        },
        LocalModelFile {
            file_name: "tokens.txt",
            rel_dir: FIRERED_LARGE_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16/resolve/main/tokens.txt",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16/resolve/main/tokens.txt",
            ],
            sha256: "6907215aeb034f6926b26bf8abfd650f756781622480a2342ec1f29b2072cafe",
            size: 71_448,
        },
    ]
}

/// FunASR Nano 的两个变体：int8（llm.int8.onnx）与 fp16（llm.fp16.onnx）。
/// 两者共享相同的 embedding.int8.onnx 与 encoder_adaptor.int8.onnx 及 Qwen3-0.6B tokenizer。
#[derive(Clone, Copy)]
pub enum FunasrNanoVariant {
    Int8,
    Fp16,
}

impl FunasrNanoVariant {
    fn dir(&self) -> &'static str {
        match self {
            Self::Int8 => FUNASR_NANO_INT8_DIR,
            Self::Fp16 => FUNASR_NANO_FP16_DIR,
        }
    }
    fn llm_file(&self) -> &'static str {
        match self {
            Self::Int8 => "llm.int8.onnx",
            Self::Fp16 => "llm.fp16.onnx",
        }
    }
    fn llm_sha(&self) -> &'static str {
        match self {
            Self::Int8 => "dfbf9aa3be41bccc257587f151e15c63fbe1b549f2b517f5ccd5bdce3bf4322a",
            Self::Fp16 => "2bb5d74cc735a5f1b23163203b5b9528bfac4285ebb89e2f38db7d1fdb30bb2c",
        }
    }
    fn llm_size(&self) -> u64 {
        match self {
            Self::Int8 => 600_356_593,
            Self::Fp16 => 1_192_981_814,
        }
    }
}

/// FunASR Nano 安装文件：3 个 onnx（embedding + encoder_adaptor + llm）+ Qwen3-0.6B tokenizer（3 文件）。
/// tokenizer 放在 {dir}/Qwen3-0.6B/ 子目录下（sherpa-onnx OfflineFunASRNanoModelConfig.tokenizer 指向该目录）。
fn funasr_nano_files(variant: FunasrNanoVariant) -> Vec<LocalModelFile> {
    let dir = variant.dir();
    // embedding / encoder_adaptor 两个文件 int8 与 fp16 仓库完全相同（同一份文件）。
    let hf_emb =
        format!("https://huggingface.co/csukuangfj/{dir}/resolve/main/embedding.int8.onnx");
    let mirror_emb =
        format!("https://hf-mirror.com/csukuangfj/{dir}/resolve/main/embedding.int8.onnx");
    let hf_enc =
        format!("https://huggingface.co/csukuangfj/{dir}/resolve/main/encoder_adaptor.int8.onnx");
    let mirror_enc =
        format!("https://hf-mirror.com/csukuangfj/{dir}/resolve/main/encoder_adaptor.int8.onnx");
    let llm_name = variant.llm_file();
    let hf_llm = format!("https://huggingface.co/csukuangfj/{dir}/resolve/main/{llm_name}");
    let mirror_llm = format!("https://hf-mirror.com/csukuangfj/{dir}/resolve/main/{llm_name}");

    // 注意：LocalModelFile 的 file_name / rel_dir 决定落盘路径。
    // tokenizer 3 文件落在 {dir}/Qwen3-0.6B/ 下，rel_dir 用 "{dir}/Qwen3-0.6B"。
    let tk_dir = format!("{dir}/Qwen3-0.6B");
    let hf_tk =
        |f: &str| format!("https://huggingface.co/csukuangfj/{dir}/resolve/main/Qwen3-0.6B/{f}");
    let mirror_tk =
        |f: &str| format!("https://hf-mirror.com/csukuangfj/{dir}/resolve/main/Qwen3-0.6B/{f}");

    // LocalModelFile.urls 是 &'static [&'static str]，但这里的 URL 是动态拼接的。
    // 为保持 API 兼容，我们 leak 这些 String（模型清单只在启动时构造一次，量很小）。
    let leak_str = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let leak_arr =
        |v: Vec<&'static str>| -> &'static [&'static str] { Box::leak(v.into_boxed_slice()) };

    vec![
        LocalModelFile {
            file_name: "embedding.int8.onnx",
            rel_dir: leak_str(dir.to_string()),
            urls: leak_arr(vec![leak_str(hf_emb), leak_str(mirror_emb)]),
            sha256: "95e61cd0c9c3b9543339a4cf973c95c116815e745ccc1e0285cbd81f76d18644",
            size: 155_584_380,
        },
        LocalModelFile {
            file_name: "encoder_adaptor.int8.onnx",
            rel_dir: dir,
            urls: leak_arr(vec![leak_str(hf_enc), leak_str(mirror_enc)]),
            sha256: "f36dea2e30fbc33b5db1d7a7265cc976c5e5586c77b042d5adb1ad27c72db422",
            size: 237_792_748,
        },
        LocalModelFile {
            file_name: leak_str(llm_name.to_string()),
            rel_dir: dir,
            urls: leak_arr(vec![leak_str(hf_llm), leak_str(mirror_llm)]),
            sha256: variant.llm_sha(),
            size: variant.llm_size(),
        },
        // Qwen3-0.6B tokenizer（3 文件，sherpa-onnx 需要 tokenizer.json + vocab.json + merges.txt）。
        LocalModelFile {
            file_name: "tokenizer.json",
            rel_dir: leak_str(tk_dir.clone()),
            urls: leak_arr(vec![
                leak_str(hf_tk("tokenizer.json")),
                leak_str(mirror_tk("tokenizer.json")),
            ]),
            sha256: "aeb13307a71acd8fe81861d94ad54ab689df773318809eed3cbe794b4492dae4",
            size: 11_422_654,
        },
        LocalModelFile {
            file_name: "vocab.json",
            rel_dir: leak_str(tk_dir.clone()),
            urls: leak_arr(vec![
                leak_str(hf_tk("vocab.json")),
                leak_str(mirror_tk("vocab.json")),
            ]),
            sha256: "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910",
            size: 2_776_833,
        },
        LocalModelFile {
            file_name: "merges.txt",
            rel_dir: leak_str(tk_dir),
            urls: leak_arr(vec![
                leak_str(hf_tk("merges.txt")),
                leak_str(mirror_tk("merges.txt")),
            ]),
            sha256: "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5",
            size: 1_671_853,
        },
    ]
}

fn vad_file() -> LocalModelFile {
    LocalModelFile {
        file_name: "silero_vad.onnx",
        rel_dir: VAD_DIR,
        urls: &[
            "https://huggingface.co/csukuangfj/vad/resolve/main/silero_vad.onnx",
            "https://hf-mirror.com/csukuangfj/vad/resolve/main/silero_vad.onnx",
        ],
        sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
        // 2025 年 HF 上游更新了 silero_vad.onnx（643854 → 1807522 字节）；
        // SHA256 未变。size 必须同步，否则 is_installed() 永远判为未安装 → 死循环。
        size: 1_807_522,
    }
}

/// 模型是否已安装（所需文件齐全且校验通过）。
///
/// 用 lenient 判定（size 或 SHA256 任一匹配即算已装），与 `missing_files_for` 保持一致，
/// 避免硬编码 size 过期导致永远判为未装。
pub fn is_asr_model_installed(model_root: &std::path::Path, id: &str) -> bool {
    asr_model_files(id)
        .into_iter()
        .all(|f| f.is_installed_lenient(model_root))
}
