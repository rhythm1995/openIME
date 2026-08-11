//! 本地模型下载与安装（sherpa-onnx 流式 Paraformer 中英双语 + Silero VAD）。
//!
//! 参考 meetily 的模型管理设计：内置模型目录、流式下载进度、SHA256 校验、
//! 断点续传（HTTP Range）、多下载源故障切换（HuggingFace → hf-mirror 国内镜像）。
//!
//! 布局约定（与 sherpa provider 的 SherpaModelPaths 对齐）：
//!   {model_root}/{SHERPA_MODEL_NAME}/{encoder.int8.onnx, decoder.int8.onnx, tokens.txt}
//!   {model_root}/vad/silero_vad.onnx

use std::path::{Path, PathBuf};
use std::time::Instant;

use futures::StreamExt;
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::Error;

/// 流式 Paraformer（中英）模型目录名。
pub const SHERPA_MODEL_NAME: &str = "sherpa-onnx-streaming-paraformer-bilingual-zh-en";

/// SenseVoice 离线模型目录名（中英日韩粤 5 语种）。
pub const SENSEVOICE_MODEL_NAME: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";

/// VAD 文件相对 model_root 的子目录。
pub const VAD_DIR: &str = "vad";

/// 二期本地润色 GGUF 子目录（相对 model_root）。
pub const LLM_DIR: &str = "llm";

/// 默认本地润色模型文件名（Qwen2.5-1.5B-Instruct Q4_K_M）。
pub const POLISH_GGUF_FILE: &str = "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf";

/// 与 AppConfig.polish_local_model 对齐的模型 id。
pub const POLISH_MODEL_ID: &str = "qwen2.5-1.5b-instruct-q4_k_m";

// 润色 GGUF：bartowski 量化（与官方 Qwen GGUF 等价 Q4_K_M）。
const URL_POLISH_GGUF_HF: &str =
    "https://huggingface.co/bartowski/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/Qwen2.5-1.5B-Instruct-Q4_K_M.gguf";
const URL_POLISH_GGUF_MIRROR: &str =
    "https://hf-mirror.com/bartowski/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/Qwen2.5-1.5B-Instruct-Q4_K_M.gguf";

// 流式 Paraformer 下载源。
const URL_ENC_HF: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en/resolve/main/encoder.int8.onnx";
const URL_ENC_MIRROR: &str = "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en/resolve/main/encoder.int8.onnx";
const URL_DEC_HF: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en/resolve/main/decoder.int8.onnx";
const URL_DEC_MIRROR: &str = "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en/resolve/main/decoder.int8.onnx";
const URL_TOK_HF: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en/resolve/main/tokens.txt";
const URL_TOK_MIRROR: &str = "https://hf-mirror.com/csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en/resolve/main/tokens.txt";

// SenseVoice 离线模型下载源。
const URL_SV_MODEL_HF: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx";
const URL_SV_MODEL_MIRROR: &str = "https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx";
const URL_SV_TOK_HF: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt";
const URL_SV_TOK_MIRROR: &str = "https://hf-mirror.com/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt";

const URL_VAD_HF: &str = "https://huggingface.co/csukuangfj/vad/resolve/main/silero_vad.onnx";
const URL_VAD_MIRROR: &str = "https://hf-mirror.com/csukuangfj/vad/resolve/main/silero_vad.onnx";

/// 单个待下载文件：候选 URL（按序尝试）、SHA256、已知大小、安装子目录。
#[derive(Debug, Clone)]
pub struct LocalModelFile {
    pub file_name: &'static str,
    /// 相对 model_root 的安装子目录（模型名目录或 vad）。
    pub rel_dir: &'static str,
    pub urls: &'static [&'static str],
    pub sha256: &'static str,
    pub size: u64,
}

impl LocalModelFile {
    /// 安装目标路径。
    pub fn dest(&self, model_root: &Path) -> PathBuf {
        model_root.join(self.rel_dir).join(self.file_name)
    }

    /// 下载中的临时文件路径。
    fn part_path(&self, model_root: &Path) -> PathBuf {
        model_root
            .join(self.rel_dir)
            .join(format!("{}.part", self.file_name))
    }

    /// 是否视为已安装（**仅检查存在 + 大小**）。
    ///
    /// 打开设置页会频繁调用（每个候选模型一次）；若每次流式算 SHA256，
    /// 会对 200MB～1GB+ 文件扫盘，造成「进主页面卡顿一小段」。
    /// 完整 SHA256 只在**下载落盘后**做（见 download_one）；此处用大小判定即可。
    ///
    /// 注意：size 是硬编码的，上游文件更新后可能过期 → 此时返回 false（误判未装）。
    /// 想避免「size 过期导致死循环下载」应改用 [`Self::is_installed_lenient`]。
    pub fn is_installed(&self, model_root: &Path) -> bool {
        let dest = self.dest(model_root);
        if !dest.is_file() {
            return false;
        }
        match std::fs::metadata(&dest) {
            Ok(meta) => meta.len() == self.size,
            Err(_) => false,
        }
    }

    /// 宽松安装判定：size 匹配 **或** SHA256 匹配即视为已装。
    ///
    /// 比 [`Self::is_installed`] 慢（size 不匹配时要扫盘算 SHA256），只在**非热路径**
    /// （判定缺失文件、跳过下载、安装状态查询）用；设置页候选列表仍用快版 `is_installed`
    /// 做 badge 初判，偶发误判为未装也只是让用户多点一次「下载」，进入安装流程后
    /// 此处会发现其实已装 → 秒过，不会再触发死循环。
    ///
    /// 修复场景：上游文件更新（size 变、SHA256 不变）时，快版会永远判为未装 → 反复下载；
    /// lenient 版靠 SHA256 兜底认出已装文件，打破死循环。
    pub fn is_installed_lenient(&self, model_root: &Path) -> bool {
        if self.is_installed(model_root) {
            return true;
        }
        let dest = self.dest(model_root);
        if !dest.is_file() || self.sha256.is_empty() {
            return false;
        }
        crate::model_mgr::verify_sha256_file(&dest, self.sha256)
    }

    /// 已安装且流式 SHA256 通过（仅用于下载后校验或诊断，勿在 UI 列表热路径调用）。
    pub fn is_installed_verified(&self, model_root: &Path) -> bool {
        if !self.is_installed(model_root) {
            return false;
        }
        if self.sha256.is_empty() {
            return true;
        }
        crate::model_mgr::verify_sha256_file(&self.dest(model_root), self.sha256)
    }
}

/// 本地引擎所需全部文件。
///
/// 优先走 ASR 目录 id（`sensevoice` / `zipformer-zh-2025` 及兼容别名 offline/realtime）；
/// 未知 id 时回退旧 Paraformer 列表（仅兼容历史测试）。
pub fn local_model_files_for(mode: &str) -> Vec<LocalModelFile> {
    let id = normalize_asr_model_id(mode);
    let catalog = crate::asr_catalog::asr_model_files(id);
    if !catalog.is_empty() {
        return catalog;
    }
    // 旧流式 Paraformer 回退（非当前 UI 候选）。
    vec![
        LocalModelFile {
            file_name: "encoder.int8.onnx",
            rel_dir: SHERPA_MODEL_NAME,
            urls: &[URL_ENC_HF, URL_ENC_MIRROR],
            sha256: "81a70226a8934e6ed92aa1d4fc486b428b5398e2f2619ed4897b7294cab90e9a",
            size: 165_462_184,
        },
        LocalModelFile {
            file_name: "decoder.int8.onnx",
            rel_dir: SHERPA_MODEL_NAME,
            urls: &[URL_DEC_HF, URL_DEC_MIRROR],
            sha256: "f3cca9f77bb9d93c8fcbfb63ae617b6b1ee96818df3aa3b151c40658fe38594f",
            size: 71_664_561,
        },
        LocalModelFile {
            file_name: "tokens.txt",
            rel_dir: SHERPA_MODEL_NAME,
            urls: &[URL_TOK_HF, URL_TOK_MIRROR],
            sha256: "59aba8873a2ed1e122c25fee421e25f283b63290efbde85c1f01a853d83cb6e6",
            size: 75_756,
        },
        LocalModelFile {
            file_name: "silero_vad.onnx",
            rel_dir: VAD_DIR,
            urls: &[URL_VAD_HF, URL_VAD_MIRROR],
            sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
            // 与 asr_catalog::vad_file() 保持一致（HF 上游已更新到 1.8MB）。
            size: 1_807_522,
        },
    ]
}

/// 向后兼容：返回目录中全部候选模型文件（去重 VAD）。
pub fn local_model_files() -> Vec<LocalModelFile> {
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for m in crate::asr_catalog::asr_model_catalog() {
        for f in crate::asr_catalog::asr_model_files(m.id) {
            let key = format!("{}/{}", f.rel_dir, f.file_name);
            if seen.insert(key) {
                files.push(f);
            }
        }
    }
    files
}

/// 尚未安装（或缺失/校验失败）的文件。空 = 本地引擎就绪。
/// `mode` 兼容：offline/realtime 或 model id（sensevoice / zipformer-zh-2025）。
///
/// 用 lenient 判定（size 或 SHA256 任一匹配即算已装），避免上游文件更新、
/// 硬编码 size 过期导致永远判为缺失 → 死循环下载。
pub fn missing_files_for(model_root: &Path, mode: &str) -> Vec<LocalModelFile> {
    let id = normalize_asr_model_id(mode);
    let files = crate::asr_catalog::asr_model_files(id);
    if !files.is_empty() {
        return files
            .into_iter()
            .filter(|f| !f.is_installed_lenient(model_root))
            .collect();
    }
    local_model_files_for(mode)
        .into_iter()
        .filter(|f| !f.is_installed_lenient(model_root))
        .collect()
}

/// 向后兼容。
pub fn missing_files(model_root: &Path) -> Vec<LocalModelFile> {
    local_model_files()
        .into_iter()
        .filter(|f| !f.is_installed_lenient(model_root))
        .collect()
}

/// 本地引擎（指定模式/模型 id）是否已就绪。
pub fn is_local_engine_installed_for(model_root: &Path, mode: &str) -> bool {
    missing_files_for(model_root, mode).is_empty()
}

/// 按 catalog id 取安装文件列表（含 VAD）。
pub fn local_model_files_for_id(model_id: &str) -> Vec<LocalModelFile> {
    crate::asr_catalog::asr_model_files(normalize_asr_model_id(model_id))
}

/// 向后兼容：全部文件就绪。
pub fn is_local_engine_installed(model_root: &Path) -> bool {
    missing_files(model_root).is_empty()
}

// ──────────────── 二期：本地润色 GGUF ────────────────

/// 本地润色模型文件清单（当前仅默认 Qwen2.5-1.5B Q4_K_M）。
pub fn polish_model_files() -> Vec<LocalModelFile> {
    vec![LocalModelFile {
        file_name: POLISH_GGUF_FILE,
        rel_dir: LLM_DIR,
        urls: &[URL_POLISH_GGUF_HF, URL_POLISH_GGUF_MIRROR],
        // LFS oid sha256 from HF pointer
        sha256: "1adf0b11065d8ad2e8123ea110d1ec956dab4ab038eab665614adba04b6c3370",
        size: 986_048_768,
    }]
}

/// 润色 GGUF 安装路径。
pub fn polish_model_path(model_root: &Path) -> PathBuf {
    model_root.join(LLM_DIR).join(POLISH_GGUF_FILE)
}

/// 本地润色模型是否已安装且校验通过。
pub fn is_polish_model_installed(model_root: &Path) -> bool {
    polish_model_files()
        .into_iter()
        .all(|f| f.is_installed_lenient(model_root))
}

/// 下载/安装本地润色 GGUF（进度回调与 ASR 模型相同）。
pub async fn install_polish_model(
    model_root: &Path,
    on_progress: &(impl Fn(DownloadProgress) + Send + Sync),
) -> crate::Result<()> {
    // 进度挂到设置页「润色」卡片。
    install_file_list(
        model_root,
        &polish_model_files(),
        "本地润色模型安装完成",
        &|p| on_progress(p.with_target("polish")),
    )
    .await
}

/// 下载进度快照（序列化后推给前端）。
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    /// downloading / verifying / done / error
    pub phase: &'static str,
    pub file_index: usize,
    pub file_count: usize,
    pub file_name: String,
    pub file_downloaded: u64,
    pub file_total: u64,
    pub total_downloaded: u64,
    pub total_size: u64,
    pub speed_bps: u64,
    pub message: String,
    /// 设置页卡片 id：ASR 模型 id 或 `polish`，用于进度条挂到对应卡片。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

impl DownloadProgress {
    /// 给进度打上 target_id（回调包装用）。
    pub fn with_target(mut self, id: impl Into<String>) -> Self {
        self.target_id = Some(id.into());
        self
    }
}

/// 构造下载用 HTTP 客户端。
fn http_client() -> crate::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .connect_timeout(std::time::Duration::from_secs(15))
        .user_agent("openIME/0.1")
        .build()
        .map_err(|e| Error::Io(format!("创建下载客户端失败: {e}")))
}

/// 安装本地引擎：下载全部缺失文件（已装且校验通过的跳过）。
///
/// - `model_id`：目录 id，如 `sensevoice` / `zipformer-zh-2025`；
///   兼容旧值 `offline`→sensevoice、`realtime`→zipformer-zh-2025。
/// - 断点续传 / SHA256 / 多源故障切换同前。
pub async fn install_local_engine(
    model_root: &Path,
    model_id: &str,
    on_progress: &(impl Fn(DownloadProgress) + Send + Sync),
) -> crate::Result<()> {
    let id = normalize_asr_model_id(model_id);
    let files = crate::asr_catalog::asr_model_files(id);
    if files.is_empty() {
        return Err(Error::Config(format!("未知本地 ASR 模型 id：{model_id}")));
    }
    let id_owned = id.to_string();
    install_file_list(
        model_root,
        &files,
        "本地 ASR 模型安装完成",
        &|p| on_progress(p.with_target(&id_owned)),
    )
    .await
}

/// 兼容旧 mode 字符串与新 model id。
pub fn normalize_asr_model_id(id_or_mode: &str) -> &str {
    match id_or_mode {
        "offline" | "sensevoice" => crate::asr_catalog::ASR_MODEL_SENSEVOICE,
        "realtime" | "zipformer-zh-2025" | "zipformer" => {
            crate::asr_catalog::ASR_MODEL_ZIPFORMER_ZH_2025
        }
        "zipformer-zh-xlarge" | "zipformer-xlarge" | "xlarge" => {
            crate::asr_catalog::ASR_MODEL_ZIPFORMER_ZH_XLARGE
        }
        "firered-large" | "firered" | "fire-red" | "fire_red_asr" => {
            crate::asr_catalog::ASR_MODEL_FIRERED_LARGE
        }
        other => other,
    }
}

async fn install_file_list(
    model_root: &Path,
    files: &[LocalModelFile],
    done_msg: &str,
    on_progress: &(impl Fn(DownloadProgress) + Send + Sync),
) -> crate::Result<()> {
    let file_count = files.len();
    let total_size: u64 = files.iter().map(|f| f.size).sum::<u64>().max(1);
    let mut total_downloaded: u64 = files
        .iter()
        .filter(|f| f.is_installed_lenient(model_root))
        .map(|f| f.size)
        .sum();
    let client = http_client()?;

    for (i, file) in files.iter().enumerate() {
        if file.is_installed_lenient(model_root) {
            on_progress(DownloadProgress {
                phase: "downloading",
                file_index: i,
                file_count,
                file_name: file.file_name.to_string(),
                file_downloaded: file.size,
                file_total: file.size,
                total_downloaded,
                total_size,
                speed_bps: 0,
                message: format!("{} 已安装，跳过", file.file_name),
            target_id: None,
            });
            continue;
        }
        download_one(
            &client,
            file,
            model_root,
            i,
            file_count,
            &mut total_downloaded,
            total_size,
            on_progress,
        )
        .await?;
    }

    on_progress(DownloadProgress {
        phase: "done",
        file_index: file_count,
        file_count,
        file_name: String::new(),
        file_downloaded: 0,
        file_total: 0,
        total_downloaded,
        total_size,
        speed_bps: 0,
        message: done_msg.to_string(),
    target_id: None,
    });
    Ok(())
}

/// 下载单个文件（含续传、校验、多源切换）。成功后 total_downloaded 增加 file.size。
#[allow(clippy::too_many_arguments)]
async fn download_one(
    client: &reqwest::Client,
    file: &LocalModelFile,
    model_root: &Path,
    index: usize,
    count: usize,
    total_downloaded: &mut u64,
    total_size: u64,
    on_progress: &(impl Fn(DownloadProgress) + Send + Sync),
) -> crate::Result<()> {
    let dest = file.dest(model_root);
    let part = file.part_path(model_root);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Io(format!("创建目录失败 {}: {e}", parent.display())))?;
    }

    let mut last_err = "无可用下载源".to_string();
    for url in file.urls {
        match download_from_url(
            client,
            file,
            url,
            &part,
            index,
            count,
            total_downloaded,
            total_size,
            on_progress,
        )
        .await
        {
            Ok(()) => {
                // 校验 + 落位。
                on_progress(DownloadProgress {
                    phase: "verifying",
                    file_index: index,
                    file_count: count,
                    file_name: file.file_name.to_string(),
                    file_downloaded: file.size,
                    file_total: file.size,
                    total_downloaded: *total_downloaded,
                    total_size,
                    speed_bps: 0,
                    message: format!("正在校验 {}", file.file_name),
                target_id: None,
                });
                // 大文件用流式校验，避免 1GB+ 模型整读内存。
                let ok = if file.sha256.is_empty() {
                    true
                } else {
                    tokio::task::spawn_blocking({
                        let part = part.clone();
                        let sha = file.sha256;
                        move || crate::model_mgr::verify_sha256_file(&part, sha)
                    })
                    .await
                    .unwrap_or(false)
                };
                if !ok {
                    let _ = tokio::fs::remove_file(&part).await;
                    last_err = format!("{} SHA256 校验失败", file.file_name);
                    continue;
                }
                tokio::fs::rename(&part, &dest)
                    .await
                    .map_err(|e| Error::Io(format!("重命名安装文件失败: {e}")))?;
                *total_downloaded += file.size;
                return Ok(());
            }
            Err(e) => {
                last_err = format!("{url}: {e}");
                // 换源重试；.part 保留以便下一源续传（若支持 Range）。
            }
        }
    }
    Err(Error::Io(format!(
        "下载 {} 失败：{last_err}",
        file.file_name
    )))
}

/// 从单个 URL 下载到 .part（支持 Range 续传），边下边报进度。
#[allow(clippy::too_many_arguments)]
async fn download_from_url(
    client: &reqwest::Client,
    file: &LocalModelFile,
    url: &str,
    part: &Path,
    index: usize,
    count: usize,
    total_downloaded: &mut u64,
    total_size: u64,
    on_progress: &(impl Fn(DownloadProgress) + Send + Sync),
) -> crate::Result<()> {
    // 续传基线：.part 已写字节数。
    let mut start: u64 = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);

    let mut req = client.get(url);
    if start > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={start}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| Error::Io(format!("请求失败: {e}")))?;

    let status = resp.status();
    // 206 = 续传生效；200 = 服务端忽略 Range（从头下）。
    if status == reqwest::StatusCode::OK {
        start = 0;
    } else if status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Error::Io(format!("HTTP {status}")));
    }

    let file_total = resp
        .content_length()
        .map(|cl| cl + start)
        .unwrap_or(file.size);

    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(start == 0)
        .append(start > 0)
        .open(part)
        .await
        .map_err(|e| Error::Io(format!("打开临时文件失败: {e}")))?;

    let mut downloaded = start;
    let mut stream = resp.bytes_stream();
    let mut last_report = Instant::now();
    let mut last_bytes = start;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Io(format!("下载中断: {e}")))?;
        f.write_all(&chunk)
            .await
            .map_err(|e| Error::Io(format!("写盘失败: {e}")))?;
        downloaded += chunk.len() as u64;

        // 限速上报：>200ms 或最后一块。
        let is_last = downloaded >= file_total;
        if last_report.elapsed().as_millis() >= 200 || is_last {
            let elapsed = last_report.elapsed().as_secs_f64().max(1e-3);
            let speed = ((downloaded - last_bytes) as f64 / elapsed) as u64;
            last_report = Instant::now();
            last_bytes = downloaded;
            let delta = downloaded - start;
            on_progress(DownloadProgress {
                phase: "downloading",
                file_index: index,
                file_count: count,
                file_name: file.file_name.to_string(),
                file_downloaded: downloaded,
                file_total,
                total_downloaded: *total_downloaded + delta,
                total_size,
                speed_bps: speed,
                message: format!("正在下载 {}", file.file_name),
            target_id: None,
            });
        }
    }
    f.flush()
        .await
        .map_err(|e| Error::Io(format!("写盘失败: {e}")))?;

    if downloaded != file_total {
        return Err(Error::Io(format!(
            "下载不完整：{downloaded}/{file_total} 字节"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_catalog_files() {
        // sensevoice + zipformer + 共享 VAD
        let files = local_model_files();
        assert!(files.len() >= 7);
        let names: Vec<_> = files.iter().map(|f| f.file_name).collect();
        assert!(names.contains(&"encoder.int8.onnx"));
        assert!(names.contains(&"decoder.onnx"));
        assert!(names.contains(&"joiner.int8.onnx"));
        assert!(names.contains(&"model.int8.onnx"));
        assert!(names.contains(&"tokens.txt"));
        assert!(names.contains(&"silero_vad.onnx"));
    }

    #[test]
    fn offline_mode_has_three_files() {
        let files = local_model_files_for("offline");
        assert_eq!(files.len(), 3); // model.int8.onnx + tokens.txt + silero_vad.onnx
        let names: Vec<_> = files.iter().map(|f| f.file_name).collect();
        assert!(names.contains(&"model.int8.onnx"));
        assert!(names.contains(&"silero_vad.onnx"));
    }

    #[test]
    fn zipformer_mode_has_five_files() {
        // realtime 兼容别名 → zipformer-zh-2025：encoder + decoder + joiner + tokens + vad
        let files = local_model_files_for("realtime");
        assert_eq!(files.len(), 5);
        let names: Vec<_> = files.iter().map(|f| f.file_name).collect();
        assert!(names.contains(&"encoder.int8.onnx"));
        assert!(names.contains(&"decoder.onnx"));
        assert!(names.contains(&"joiner.int8.onnx"));
        assert!(names.contains(&"silero_vad.onnx"));
    }

    #[test]
    fn dest_paths_match_provider_layout() {
        let root = PathBuf::from("/data/models");
        let files = local_model_files_for("zipformer-zh-2025");
        let enc = files
            .iter()
            .find(|f| f.file_name == "encoder.int8.onnx")
            .unwrap();
        assert_eq!(
            enc.dest(&root),
            PathBuf::from(format!(
                "/data/models/{}/encoder.int8.onnx",
                crate::asr_catalog::ZIPFORMER_ZH_2025_DIR
            ))
        );
        let vad = files
            .iter()
            .find(|f| f.file_name == "silero_vad.onnx")
            .unwrap();
        assert_eq!(
            vad.dest(&root),
            PathBuf::from("/data/models/vad/silero_vad.onnx")
        );
    }

    #[test]
    fn missing_files_reports_all_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let n = missing_files(dir.path()).len();
        assert!(n >= 4, "expected >= 4 missing, got {n}");
        assert!(!is_local_engine_installed(dir.path()));
        // 按模型 id / 兼容 mode 查
        assert_eq!(missing_files_for(dir.path(), "offline").len(), 3);
        assert_eq!(missing_files_for(dir.path(), "sensevoice").len(), 3);
        assert_eq!(missing_files_for(dir.path(), "realtime").len(), 5);
        assert_eq!(missing_files_for(dir.path(), "zipformer-zh-2025").len(), 5);
    }

    #[test]
    fn installed_detection_uses_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let files = local_model_files();
        // 写入错误内容 → 校验失败 → 仍算缺失。
        for f in &files {
            let dest = f.dest(root);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::write(&dest, b"wrong content").unwrap();
        }
        assert_eq!(missing_files(root).len(), files.len());
    }
}
