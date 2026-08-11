//! 本地 ASR 模型目录：可下载、可选中启用。
//!
//! 当前候选：
//! - `firered-large`：FireRedASR Large 离线（本地中文高精度）
//! - `zipformer-zh-xlarge`：流式 Zipformer 中文 xlarge 2025
//! - `zipformer-zh-2025`：流式 Zipformer 中文 large 2025
//! - `sensevoice`：SenseVoice 离线（轻量多语）

use serde::Serialize;

use crate::model_download::{LocalModelFile, SENSEVOICE_MODEL_NAME, VAD_DIR};

/// Zipformer 中文 2025 large 流式 int8 安装目录名。
pub const ZIPFORMER_ZH_2025_DIR: &str = "sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30";

/// Zipformer 中文 2025 xlarge 流式 int8 安装目录名。
pub const ZIPFORMER_ZH_XLARGE_DIR: &str =
    "sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30";

/// FireRedASR Large 中英 离线安装目录名。
pub const FIRERED_LARGE_DIR: &str = "sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16";

/// 本地 ASR 模型 id。
pub const ASR_MODEL_SENSEVOICE: &str = "sensevoice";
pub const ASR_MODEL_ZIPFORMER_ZH_2025: &str = "zipformer-zh-2025";
pub const ASR_MODEL_ZIPFORMER_ZH_XLARGE: &str = "zipformer-zh-xlarge";
pub const ASR_MODEL_FIRERED_LARGE: &str = "firered-large";

/// 推理后端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrBackend {
    /// Offline SenseVoice（整段解码）。
    OfflineSenseVoice,
    /// Offline FireRedASR AED（整段解码，高精度）。
    OfflineFireRed,
    /// Online Zipformer transducer + VAD。
    StreamingZipformer,
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
            id: ASR_MODEL_FIRERED_LARGE,
            title: "FireRedASR Large",
            description: "离线整段解码 · 中英高精度（普通话/部分方言）· 约 1.7GB · 更准但更慢更吃内存，适合追求识别率",
            dir_name: FIRERED_LARGE_DIR,
            backend: AsrBackend::OfflineFireRed,
            recommended: true,
            approx_size: 1_293_430_814 + 445_469_383 + 71_448,
        },
        AsrModelInfo {
            id: ASR_MODEL_ZIPFORMER_ZH_XLARGE,
            title: "Zipformer 中文 xlarge",
            description: "流式 Zipformer xlarge int8 · 中文大模型 · 约 735MB · 比 large 更准，CPU 占用更高",
            dir_name: ZIPFORMER_ZH_XLARGE_DIR,
            backend: AsrBackend::StreamingZipformer,
            recommended: false,
            approx_size: 761_133_737 + 8_533_022 + 1_545_417 + 18_626,
        },
        AsrModelInfo {
            id: ASR_MODEL_ZIPFORMER_ZH_2025,
            title: "Zipformer 中文 2025",
            description: "流式 Zipformer large int8 · 中文 · 约 167MB · 体积与速度折中，精度弱于 xlarge / FireRed",
            dir_name: ZIPFORMER_ZH_2025_DIR,
            backend: AsrBackend::StreamingZipformer,
            recommended: false,
            approx_size: 161_141_793 + 5_165_083 + 1_033_416 + 20_628,
        },
        AsrModelInfo {
            id: ASR_MODEL_SENSEVOICE,
            title: "SenseVoice",
            description: "离线整段解码 · 中英日韩粤 · 约 240MB · 快、省资源，多语混说友好；中文纯识别不一定压过大模型",
            dir_name: SENSEVOICE_MODEL_NAME,
            backend: AsrBackend::OfflineSenseVoice,
            recommended: false,
            approx_size: 239_233_841 + 315_894,
        },
    ]
}

pub fn asr_model_by_id(id: &str) -> Option<&'static AsrModelInfo> {
    asr_model_catalog().iter().find(|m| m.id == id)
}

pub fn default_asr_model_id() -> &'static str {
    // 默认仍偏轻量可选：SenseVoice 更适合首装；用户可改选 FireRed / xlarge。
    ASR_MODEL_SENSEVOICE
}

/// 某模型安装所需文件（含共享 VAD）。
pub fn asr_model_files(id: &str) -> Vec<LocalModelFile> {
    let mut files = match id {
        ASR_MODEL_SENSEVOICE => sensevoice_files(),
        ASR_MODEL_ZIPFORMER_ZH_2025 => zipformer_zh_2025_files(),
        ASR_MODEL_ZIPFORMER_ZH_XLARGE => zipformer_zh_xlarge_files(),
        ASR_MODEL_FIRERED_LARGE => firered_large_files(),
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

fn zipformer_zh_2025_files() -> Vec<LocalModelFile> {
    vec![
        LocalModelFile {
            file_name: "encoder.int8.onnx",
            rel_dir: ZIPFORMER_ZH_2025_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/encoder.int8.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/encoder.int8.onnx",
            ],
            sha256: "5ac51e27981bb4dab01bb9be4958453ba50c3b61c063ddda0eab23fd3671aa4f",
            size: 161_141_793,
        },
        LocalModelFile {
            file_name: "decoder.onnx",
            rel_dir: ZIPFORMER_ZH_2025_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/decoder.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/decoder.onnx",
            ],
            sha256: "06522ad63cec0fdf6809f4e1db9bb4f7d710c34582e3b35db62ac60eccafac7e",
            size: 5_165_083,
        },
        LocalModelFile {
            file_name: "joiner.int8.onnx",
            rel_dir: ZIPFORMER_ZH_2025_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/joiner.int8.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/joiner.int8.onnx",
            ],
            sha256: "b34584dc6f561089e1d747fedebb3765f2caa72c927ef54d7ca55e5ae40a814b",
            size: 1_033_416,
        },
        LocalModelFile {
            file_name: "tokens.txt",
            rel_dir: ZIPFORMER_ZH_2025_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/tokens.txt",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-int8-2025-06-30/resolve/main/tokens.txt",
            ],
            sha256: "6193c7ea1c96d0d9a1e9652789b40d13a8a913b434a5451e93158f5a09fd6652",
            size: 20_628,
        },
    ]
}

fn zipformer_zh_xlarge_files() -> Vec<LocalModelFile> {
    // SHA256 必须用 Git LFS oid（raw 指针里的 sha256:…），不能用 HTTP ETag
    //（HF 现网 ETag 与内容哈希不一致，会导致「下载成功但校验失败」）。
    vec![
        LocalModelFile {
            file_name: "encoder.int8.onnx",
            rel_dir: ZIPFORMER_ZH_XLARGE_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/encoder.int8.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/encoder.int8.onnx",
            ],
            sha256: "f2c543a0330e1ed0bd09c82e4ae7d3f1cbee10a15feca638fcc4f88083a36b8a",
            size: 761_133_737,
        },
        LocalModelFile {
            file_name: "decoder.onnx",
            rel_dir: ZIPFORMER_ZH_XLARGE_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/decoder.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/decoder.onnx",
            ],
            sha256: "8f9c903da2818f207304a3f30b9eeb30028e30398f333c1e95e12c97704173e6",
            size: 8_533_022,
        },
        LocalModelFile {
            file_name: "joiner.int8.onnx",
            rel_dir: ZIPFORMER_ZH_XLARGE_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/joiner.int8.onnx",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/joiner.int8.onnx",
            ],
            sha256: "f76ffce14b6ef80098cfdbce8846896ff68133970abc314eafab632f910df0d7",
            size: 1_545_417,
        },
        LocalModelFile {
            file_name: "tokens.txt",
            rel_dir: ZIPFORMER_ZH_XLARGE_DIR,
            urls: &[
                "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/tokens.txt",
                "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-zipformer-zh-xlarge-int8-2025-06-30/resolve/main/tokens.txt",
            ],
            sha256: "6722bd1585f46f84456b29c3550a343a3cc375b971645773c02ed8e0b4e2405c",
            size: 18_626,
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
