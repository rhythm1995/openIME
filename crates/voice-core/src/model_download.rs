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

    /// 已安装且校验通过。
    pub fn is_installed(&self, model_root: &Path) -> bool {
        let dest = self.dest(model_root);
        if !dest.is_file() {
            return false;
        }
        match std::fs::read(&dest) {
            Ok(bytes) => crate::model_mgr::verify_sha256(&bytes, self.sha256),
            Err(_) => false,
        }
    }
}

/// 本地引擎所需全部文件。
/// `mode` = "offline" 时下载 SenseVoice 离线模型 + VAD；
/// `mode` = "realtime" 时下载流式 Paraformer + VAD。
pub fn local_model_files_for(mode: &str) -> Vec<LocalModelFile> {
    let mut files = Vec::new();
    if mode == "offline" {
        // SenseVoice 离线模型（中英日韩粤）。
        files.push(LocalModelFile {
            file_name: "model.int8.onnx",
            rel_dir: SENSEVOICE_MODEL_NAME,
            urls: &[URL_SV_MODEL_HF, URL_SV_MODEL_MIRROR],
            sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51",
            size: 239_233_841,
        });
        files.push(LocalModelFile {
            file_name: "tokens.txt",
            rel_dir: SENSEVOICE_MODEL_NAME,
            urls: &[URL_SV_TOK_HF, URL_SV_TOK_MIRROR],
            sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc",
            size: 315_894,
        });
    } else {
        // 流式 Paraformer（中英）。
        files.push(LocalModelFile {
            file_name: "encoder.int8.onnx",
            rel_dir: SHERPA_MODEL_NAME,
            urls: &[URL_ENC_HF, URL_ENC_MIRROR],
            sha256: "81a70226a8934e6ed92aa1d4fc486b428b5398e2f2619ed4897b7294cab90e9a",
            size: 165_462_184,
        });
        files.push(LocalModelFile {
            file_name: "decoder.int8.onnx",
            rel_dir: SHERPA_MODEL_NAME,
            urls: &[URL_DEC_HF, URL_DEC_MIRROR],
            sha256: "f3cca9f77bb9d93c8fcbfb63ae617b6b1ee96818df3aa3b151c40658fe38594f",
            size: 71_664_561,
        });
        files.push(LocalModelFile {
            file_name: "tokens.txt",
            rel_dir: SHERPA_MODEL_NAME,
            urls: &[URL_TOK_HF, URL_TOK_MIRROR],
            sha256: "59aba8873a2ed1e122c25fee421e25f283b63290efbde85c1f01a853d83cb6e6",
            size: 75_756,
        });
    }
    // VAD（两种模式都需要）。
    files.push(LocalModelFile {
        file_name: "silero_vad.onnx",
        rel_dir: VAD_DIR,
        urls: &[URL_VAD_HF, URL_VAD_MIRROR],
        sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
        size: 643_854,
    });
    files
}

/// 向后兼容：默认返回 offline 模式文件（含流式 + 离线 + VAD）。
pub fn local_model_files() -> Vec<LocalModelFile> {
    // 返回全部可能用到的文件（流式 + 离线 + VAD），确保两种模式都能工作。
    let mut files = local_model_files_for("offline");
    files.extend(
        local_model_files_for("realtime")
            .into_iter()
            .filter(|f| f.rel_dir == SHERPA_MODEL_NAME),
    );
    files
}

/// 尚未安装（或缺失/校验失败）的文件。空 = 本地引擎就绪。
/// `mode` = "offline"/"realtime" 决定检查哪套模型。
pub fn missing_files_for(model_root: &Path, mode: &str) -> Vec<LocalModelFile> {
    local_model_files_for(mode)
        .into_iter()
        .filter(|f| !f.is_installed(model_root))
        .collect()
}

/// 向后兼容。
pub fn missing_files(model_root: &Path) -> Vec<LocalModelFile> {
    local_model_files()
        .into_iter()
        .filter(|f| !f.is_installed(model_root))
        .collect()
}

/// 本地引擎（指定模式）是否已就绪。
pub fn is_local_engine_installed_for(model_root: &Path, mode: &str) -> bool {
    missing_files_for(model_root, mode).is_empty()
}

/// 向后兼容：全部文件就绪。
pub fn is_local_engine_installed(model_root: &Path) -> bool {
    missing_files(model_root).is_empty()
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
/// - 断点续传：.part 文件存在时以 Range 请求续传（源不支持则整文件重下）。
/// - 校验：SHA256 不符则删除文件并报错。
/// - 故障切换：单 URL 失败自动尝试下一个候选源。
pub async fn install_local_engine(
    model_root: &Path,
    mode: &str,
    on_progress: &(impl Fn(DownloadProgress) + Send + Sync),
) -> crate::Result<()> {
    let files = local_model_files_for(mode);
    let file_count = files.len();
    let total_size: u64 = files.iter().map(|f| f.size).sum::<u64>().max(1);

    // 已装文件计入"已下载"，进度从真实基线开始。
    let mut total_downloaded: u64 = files
        .iter()
        .filter(|f| f.is_installed(model_root))
        .map(|f| f.size)
        .sum();

    let client = http_client()?;

    for (i, file) in files.iter().enumerate() {
        if file.is_installed(model_root) {
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
        message: "本地模型安装完成".to_string(),
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
                });
                let bytes = tokio::fs::read(&part)
                    .await
                    .map_err(|e| Error::Io(format!("读取下载文件失败: {e}")))?;
                if !crate::model_mgr::verify_sha256(&bytes, file.sha256) {
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
    fn manifest_has_four_files() {
        // local_model_files() 返回 offline(3) + realtime(3) + VAD(1) = 7
        let files = local_model_files();
        assert!(files.len() >= 4);
        let names: Vec<_> = files.iter().map(|f| f.file_name).collect();
        assert!(names.contains(&"encoder.int8.onnx"));
        assert!(names.contains(&"decoder.int8.onnx"));
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
    fn realtime_mode_has_four_files() {
        let files = local_model_files_for("realtime");
        assert_eq!(files.len(), 4); // encoder + decoder + tokens + silero_vad
        let names: Vec<_> = files.iter().map(|f| f.file_name).collect();
        assert!(names.contains(&"encoder.int8.onnx"));
        assert!(names.contains(&"decoder.int8.onnx"));
    }

    #[test]
    fn dest_paths_match_provider_layout() {
        let root = PathBuf::from("/data/models");
        let files = local_model_files();
        let enc = files
            .iter()
            .find(|f| f.file_name == "encoder.int8.onnx")
            .unwrap();
        assert_eq!(
            enc.dest(&root),
            PathBuf::from(format!(
                "/data/models/{SHERPA_MODEL_NAME}/encoder.int8.onnx"
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
        // 按模式查
        assert_eq!(missing_files_for(dir.path(), "offline").len(), 3);
        assert_eq!(missing_files_for(dir.path(), "realtime").len(), 4);
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
