//! 本地 LLM 模型目录：润色（Polish）与翻译（Translate）两套 GGUF 目录。
//!
//! 形状对齐 [`crate::asr_catalog::AsrModelInfo`]：可下载、可选中启用、按本机打标签。
//! 冻结目录（`docs/local-model-suite-plan.md`）：
//!
//! - 润色：`qwen3.5-0.8b` / `qwen3.5-2b` / `qwen3.5-4b`（`llm` feature 加载 qwen35
//!   失败时同档回退 Qwen3-0.6B / 1.7B / 4B-Instruct-2507）。
//! - 翻译：`milmmt-1b`（默认专翻）/ `hy-mt-1.8b`（自选专翻）。
//!
//! 不进目录：Qwen2.5-1.5B（被 2B 取代，旧配置读入时映射到 `qwen3.5-2b`）、
//! Qwen3.5-9B、FireRed。所有 GGUF 均为 Q4_K_M；size 与 SHA256 取 HF LFS oid（锁死）。
//!
//! 下载走 [`crate::model_download::LocalModelFile`]（多 URL、SHA256、Range、hf-mirror）。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::model_download::{LocalModelFile, LLM_DIR};

/// LLM 模型用途。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmKind {
    /// 润色（听写 L0 后）。
    Polish,
    /// 翻译（R4/R5 翻译角色）。
    Translate,
}

/// GGUF 架构族（决定 llama.cpp 绑定能否加载 + Qwen3 系是否关 thinking）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmArch {
    Qwen25,
    Qwen3,
    Qwen35,
    Gemma3,
    Hunyuan,
}

/// 目录中一条本地 LLM 模型。
#[derive(Debug, Clone, Serialize)]
pub struct LlmModelInfo {
    pub id: &'static str,
    pub kind: LlmKind,
    pub title: &'static str,
    pub description: &'static str,
    /// 首选 GGUF 文件名（`llm/` 下）。
    pub file_name: &'static str,
    /// 首选加载失败时的同档回退 id（None = 无回退）。
    pub fallback_id: Option<&'static str>,
    /// 文件字节数（下载/已装判定用，锁 HF LFS）。
    pub approx_size: u64,
    /// 常驻内存估算（打标签用，见 system::compute_combo_tag）。
    pub rss_bytes: u64,
    /// 生成 token 上限（润色短改写 128；翻译 256）。
    pub n_predict: i32,
    pub arch_hint: LlmArch,
}

/// 润色目录（顺序即设置页展示顺序）。
pub fn polish_catalog() -> &'static [LlmModelInfo] {
    &[
        LlmModelInfo {
            id: "qwen3.5-0.8b",
            kind: LlmKind::Polish,
            title: "极速 · Qwen3.5-0.8B",
            description: "离线润色 · 约 0.5GB · 弱机/8GB 首选。速度最快、常驻最小，短句改写即时上屏；质量比 2B 略逊。",
            file_name: "Qwen3.5-0.8B-Q4_K_M.gguf",
            fallback_id: Some("qwen3-0.6b"),
            approx_size: 532_517_120,
            rss_bytes: 644_245_094, // ~0.6 GB
            n_predict: 128,
            arch_hint: LlmArch::Qwen35,
        },
        LlmModelInfo {
            id: "qwen3.5-2b",
            kind: LlmKind::Polish,
            title: "均衡 · Qwen3.5-2B",
            description: "离线润色 · 约 1.4GB · 16GB 默认。速度与质量均衡档，40 字润色约 1 秒；多数机器的推荐落点。",
            file_name: "Qwen_Qwen3.5-2B-Q4_K_M.gguf",
            fallback_id: Some("qwen3-1.7b"),
            approx_size: 1_396_198_496,
            rss_bytes: 1_610_612_736, // ~1.5 GB
            n_predict: 128,
            arch_hint: LlmArch::Qwen35,
        },
        LlmModelInfo {
            id: "qwen3.5-4b",
            kind: LlmKind::Polish,
            title: "高质量 · Qwen3.5-4B",
            description: "离线润色 · 约 2.7GB · 48GB/M 系列 Pro 默认。改写得更好、更稳，需要大内存与高带宽；16GB 上偏慢。",
            file_name: "Qwen3.5-4B-Q4_K_M.gguf",
            fallback_id: Some("qwen3-4b-instruct-2507"),
            approx_size: 2_740_937_888,
            rss_bytes: 3_006_477_107, // ~2.8 GB
            n_predict: 256,
            arch_hint: LlmArch::Qwen35,
        },
    ]
}

/// 翻译目录（顺序即设置页展示顺序）。
pub fn translate_catalog() -> &'static [LlmModelInfo] {
    &[
        LlmModelInfo {
            id: "milmmt-1b",
            kind: LlmKind::Translate,
            title: "MiLMMT-1B（默认专翻）",
            description: "离线专翻 · 约 0.8GB · 46 语（含简/繁/粤）。1B 专翻 SOTA，Gemma3 架构生态熟；中英日韩短句接近可用产品。",
            file_name: "MiLMMT-46-1B-v1.0.Q4_K_M.gguf",
            fallback_id: None,
            approx_size: 806_057_408,
            rss_bytes: 1_181_155_328, // ~1.1 GB
            n_predict: 256,
            arch_hint: LlmArch::Gemma3,
        },
        LlmModelInfo {
            id: "hy-mt-1.8b",
            kind: LlmKind::Translate,
            title: "HY-MT1.5-1.8B（自选）",
            description: "离线专翻 · 约 1.1GB · 33 语 + 粤/藏/蒙/维。腾讯混元端侧翻译，原生术语干预（可接热词）；混元社区许可，绿标自选。",
            file_name: "HY-MT1.5-1.8B-Q4_K_M.gguf",
            fallback_id: None,
            approx_size: 1_133_080_512,
            rss_bytes: 1_503_238_553, // ~1.4 GB
            n_predict: 256,
            arch_hint: LlmArch::Hunyuan,
        },
    ]
}

/// 回退档（不进设置页目录，仅在首选加载失败时使用）。
fn fallback_catalog() -> &'static [LlmModelInfo] {
    &[
        LlmModelInfo {
            id: "qwen3-0.6b",
            kind: LlmKind::Polish,
            title: "Qwen3-0.6B（回退）",
            description: "qwen3.5-0.8b 加载失败时的同档回退。",
            file_name: "Qwen3-0.6B-Q4_K_M.gguf",
            fallback_id: None,
            approx_size: 396_705_472,
            rss_bytes: 644_245_094, // ~0.6 GB
            n_predict: 128,
            arch_hint: LlmArch::Qwen3,
        },
        LlmModelInfo {
            id: "qwen3-1.7b",
            kind: LlmKind::Polish,
            title: "Qwen3-1.7B（回退）",
            description: "qwen3.5-2b 加载失败时的同档回退。",
            file_name: "Qwen3-1.7B-Q4_K_M.gguf",
            fallback_id: None,
            approx_size: 1_107_409_472,
            rss_bytes: 1_395_864_371, // ~1.3 GB
            n_predict: 128,
            arch_hint: LlmArch::Qwen3,
        },
        LlmModelInfo {
            id: "qwen3-4b-instruct-2507",
            kind: LlmKind::Polish,
            title: "Qwen3-4B-Instruct-2507（回退）",
            description: "qwen3.5-4b 加载失败时的同档回退（纯非思考版）。",
            file_name: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            fallback_id: None,
            approx_size: 2_497_281_120,
            rss_bytes: 2_791_728_742, // ~2.6 GB
            n_predict: 256,
            arch_hint: LlmArch::Qwen3,
        },
    ]
}

/// 全部已知 id（目录 + 回退）。未知 id 返回 None。
pub fn llm_model_by_id(id: &str) -> Option<&'static LlmModelInfo> {
    polish_catalog()
        .iter()
        .chain(translate_catalog().iter())
        .chain(fallback_catalog().iter())
        .find(|m| m.id == id)
}

/// id 是否属于润色目录（不含回退档/翻译档）。启用润色档的校验用：
/// 防止把翻译/回退档 id 设为润色模型。
pub fn is_polish_catalog_id(id: &str) -> bool {
    polish_catalog().iter().any(|m| m.id == id)
}

/// id 是否属于翻译目录（不含回退档/润色档）。启用专翻的校验用：
/// 防止把润色档 id 设为专翻（UI 卡片列表与运行时行为会失明）。
pub fn is_translate_catalog_id(id: &str) -> bool {
    translate_catalog().iter().any(|m| m.id == id)
}

fn gguf_urls(repo: &'static str, file: &'static str) -> &'static [&'static str] {
    // URL 完全静态拼接（常量编译期折叠），无需 leak。
    match (repo, file) {
        ("unsloth/Qwen3.5-0.8B-GGUF", "Qwen3.5-0.8B-Q4_K_M.gguf") => &[
            "https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/Qwen3.5-0.8B-Q4_K_M.gguf",
            "https://hf-mirror.com/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/Qwen3.5-0.8B-Q4_K_M.gguf",
        ],
        ("bartowski/Qwen_Qwen3.5-2B-GGUF", "Qwen_Qwen3.5-2B-Q4_K_M.gguf") => &[
            "https://huggingface.co/bartowski/Qwen_Qwen3.5-2B-GGUF/resolve/main/Qwen_Qwen3.5-2B-Q4_K_M.gguf",
            "https://hf-mirror.com/bartowski/Qwen_Qwen3.5-2B-GGUF/resolve/main/Qwen_Qwen3.5-2B-Q4_K_M.gguf",
        ],
        ("unsloth/Qwen3.5-4B-GGUF", "Qwen3.5-4B-Q4_K_M.gguf") => &[
            "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf",
            "https://hf-mirror.com/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf",
        ],
        ("unsloth/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q4_K_M.gguf") => &[
            "https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf",
            "https://hf-mirror.com/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf",
        ],
        ("unsloth/Qwen3-1.7B-GGUF", "Qwen3-1.7B-Q4_K_M.gguf") => &[
            "https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
            "https://hf-mirror.com/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
        ],
        ("unsloth/Qwen3-4B-Instruct-2507-GGUF", "Qwen3-4B-Instruct-2507-Q4_K_M.gguf") => &[
            "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            "https://hf-mirror.com/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        ],
        ("mradermacher/MiLMMT-46-1B-v1.0-GGUF", "MiLMMT-46-1B-v1.0.Q4_K_M.gguf") => &[
            "https://huggingface.co/mradermacher/MiLMMT-46-1B-v1.0-GGUF/resolve/main/MiLMMT-46-1B-v1.0.Q4_K_M.gguf",
            "https://hf-mirror.com/mradermacher/MiLMMT-46-1B-v1.0-GGUF/resolve/main/MiLMMT-46-1B-v1.0.Q4_K_M.gguf",
        ],
        ("tencent/HY-MT1.5-1.8B-GGUF", "HY-MT1.5-1.8B-Q4_K_M.gguf") => &[
            "https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q4_K_M.gguf",
            "https://hf-mirror.com/tencent/HY-MT1.5-1.8B-GGUF/resolve/main/HY-MT1.5-1.8B-Q4_K_M.gguf",
        ],
        _ => &[],
    }
}

/// 单条 GGUF 的下载清单（sha256 = HF LFS oid，锁死勿动）。
fn gguf_file(
    repo: &'static str,
    file: &'static str,
    sha256: &'static str,
    size: u64,
) -> LocalModelFile {
    LocalModelFile {
        file_name: file,
        rel_dir: LLM_DIR,
        urls: gguf_urls(repo, file),
        sha256,
        size,
    }
}

/// 目录条目的全部可下载文件（首选 + 回退档，回退档用于首选无法加载时的兜底下载）。
fn files_for(info: &LlmModelInfo) -> Vec<LocalModelFile> {
    let preferred = file_for(info);
    let mut out = vec![preferred];
    if let Some(fb_id) = info.fallback_id {
        if let Some(fb) = llm_model_by_id(fb_id) {
            out.push(file_for(fb));
        }
    }
    out
}

fn file_for(info: &LlmModelInfo) -> LocalModelFile {
    // (repo, file, sha256) 与 URL 表严格配对。
    let (repo, sha) = match info.id {
        "qwen3.5-0.8b" => (
            "unsloth/Qwen3.5-0.8B-GGUF",
            "bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517",
        ),
        "qwen3.5-2b" => (
            "bartowski/Qwen_Qwen3.5-2B-GGUF",
            "57a1085840f497d764a7fc5d346922dbde961efb54cc792ea81d694fd846a1d8",
        ),
        "qwen3.5-4b" => (
            "unsloth/Qwen3.5-4B-GGUF",
            "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
        ),
        "qwen3-0.6b" => (
            "unsloth/Qwen3-0.6B-GGUF",
            "ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a",
        ),
        "qwen3-1.7b" => (
            "unsloth/Qwen3-1.7B-GGUF",
            "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
        ),
        "qwen3-4b-instruct-2507" => (
            "unsloth/Qwen3-4B-Instruct-2507-GGUF",
            "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
        ),
        "milmmt-1b" => (
            "mradermacher/MiLMMT-46-1B-v1.0-GGUF",
            "74d38ba75108d455326e9deeaf9ab01bb266dfa665eae9c4aa84e84485d4fdf9",
        ),
        "hy-mt-1.8b" => (
            "tencent/HY-MT1.5-1.8B-GGUF",
            "4383ac0c3c8e476de98ff979c2a3f069f8c4fb385e7860cf2d28da896cc477c7",
        ),
        _ => ("", ""),
    };
    gguf_file(repo, info.file_name, sha, info.approx_size)
}

/// 某 id 的下载文件清单（含回退档解析）。未知 id 返回空。
pub fn llm_files(id: &str) -> Vec<LocalModelFile> {
    llm_model_by_id(id).map(files_for).unwrap_or_default()
}

/// LLM GGUF 安装路径（首选文件）。
pub fn llm_model_path(model_root: &Path, id: &str) -> PathBuf {
    match llm_model_by_id(id) {
        Some(info) => model_root.join(LLM_DIR).join(info.file_name),
        None => model_root.join(LLM_DIR).join(format!("{id}.gguf")),
    }
}

/// 目录模型是否已可用：首选已装，或首选缺位但回退档已装。
pub fn is_llm_model_installed(model_root: &Path, id: &str) -> bool {
    let Some(info) = llm_model_by_id(id) else {
        return false;
    };
    if file_for(info).is_installed_lenient(model_root) {
        return true;
    }
    match info.fallback_id {
        Some(fb) => llm_model_by_id(fb)
            .map(|f| file_for(f).is_installed_lenient(model_root))
            .unwrap_or(false),
        None => false,
    }
}

/// 解析实际使用的 GGUF：首选可用 → 首选；否则回退档已装 → 回退；否则首选（未安装，供下载）。
///
/// `arch_unsupported`：运行时记录过「加载失败（架构不认）」的 path；命中则跳过该档。
pub fn resolve_llm_id(
    id: &str,
    model_root: &Path,
    arch_unsupported: &dyn Fn(&Path) -> bool,
) -> (String, PathBuf) {
    let info = llm_model_by_id(id);
    let Some(info) = info else {
        return (id.to_string(), llm_model_path(model_root, id));
    };
    let preferred = file_for(info);
    let preferred_path = model_root.join(LLM_DIR).join(preferred.file_name);
    if preferred_path.is_file() && !arch_unsupported(&preferred_path) {
        return (info.id.to_string(), preferred_path);
    }
    if let Some(fb_id) = info.fallback_id {
        if let Some(fb) = llm_model_by_id(fb_id) {
            let fb_path = model_root.join(LLM_DIR).join(fb.file_name);
            if fb_path.is_file() && !arch_unsupported(&fb_path) {
                return (fb.id.to_string(), fb_path);
            }
        }
    }
    (info.id.to_string(), preferred_path)
}

/// 兼容归一：旧配置里的 1.5B id 映射到 2B 档（不读旧文件）。
pub fn normalize_polish_model_id(id: &str) -> &str {
    match id.trim() {
        "" => "qwen3.5-2b",
        other
            if other.eq_ignore_ascii_case("qwen2.5-1.5b")
                || other.eq_ignore_ascii_case("qwen2.5-1.5b-instruct")
                || other.eq_ignore_ascii_case("qwen2.5-1.5b-instruct-q4_k_m")
                || other.eq_ignore_ascii_case("qwen2.5-1.5b-instruct-q4_k_s") =>
        {
            "qwen3.5-2b"
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    fn no_arch(_: &Path) -> bool {
        false
    }

    #[test]
    fn catalogs_are_frozen_closed_sets() {
        let polish_ids: Vec<_> = polish_catalog().iter().map(|m| m.id).collect();
        assert_eq!(polish_ids, vec!["qwen3.5-0.8b", "qwen3.5-2b", "qwen3.5-4b"]);
        let translate_ids: Vec<_> = translate_catalog().iter().map(|m| m.id).collect();
        assert_eq!(translate_ids, vec!["milmmt-1b", "hy-mt-1.8b"]);
        // 1.5B 与 9B 不在目录。
        assert!(polish_catalog().iter().all(|m| !m.id.contains("1.5b")));
        assert!(polish_catalog().iter().all(|m| !m.id.contains("9b")));
    }

    #[test]
    fn unknown_id_has_no_files() {
        assert!(llm_files("no-such-model").is_empty());
        assert!(llm_model_by_id("qwen2.5-1.5b-instruct-q4_k_m").is_none());
    }

    #[test]
    fn catalog_id_membership_is_scoped() {
        // 润色目录：只认三档润色 id；翻译/回退/未知/空串都不认。
        assert!(is_polish_catalog_id("qwen3.5-2b"));
        for id in ["milmmt-1b", "hy-mt-1.8b", "qwen3-1.7b", "", "no-such"] {
            assert!(!is_polish_catalog_id(id), "{id} 不应算润色目录 id");
        }
        // 翻译目录：只认两档专翻；润色/回退/未知/空串都不认。
        assert!(is_translate_catalog_id("milmmt-1b"));
        assert!(is_translate_catalog_id("hy-mt-1.8b"));
        for id in ["qwen3.5-2b", "qwen3-4b-instruct-2507", "", "no-such"] {
            assert!(!is_translate_catalog_id(id), "{id} 不应算翻译目录 id");
        }
    }

    #[test]
    fn every_catalog_entry_has_download_urls_and_sha() {
        for m in polish_catalog().iter().chain(translate_catalog().iter()) {
            let files = llm_files(m.id);
            assert!(!files.is_empty(), "{} 应有下载清单", m.id);
            for f in &files {
                assert!(
                    f.urls.len() >= 2,
                    "{} 应有 HF + hf-mirror 双源",
                    f.file_name
                );
                assert!(f.urls.iter().all(|u| u.contains(f.file_name)));
                assert!(!f.sha256.is_empty(), "{} 必须锁 SHA256", f.file_name);
                assert!(f.size > 100_000_000, "{} 体积异常", f.file_name);
            }
        }
    }

    #[test]
    fn files_include_fallback_when_declared() {
        // 0.8b 声明回退 → 清单含两张。
        let files = llm_files("qwen3.5-0.8b");
        let names: Vec<_> = files.iter().map(|f| f.file_name).collect();
        assert!(names.contains(&"Qwen3.5-0.8B-Q4_K_M.gguf"));
        assert!(names.contains(&"Qwen3-0.6B-Q4_K_M.gguf"));
        // 专翻无回退 → 单文件。
        assert_eq!(llm_files("milmmt-1b").len(), 1);
    }

    #[test]
    fn legacy_polish_id_normalizes_to_2b() {
        assert_eq!(
            normalize_polish_model_id("qwen2.5-1.5b-instruct-q4_k_m"),
            "qwen3.5-2b"
        );
        assert_eq!(normalize_polish_model_id(""), "qwen3.5-2b");
        assert_eq!(normalize_polish_model_id("qwen3.5-4b"), "qwen3.5-4b");
    }

    #[test]
    fn normalize_polish_model_id_covers_all_legacy_aliases() {
        // 迁移契约：四个旧 1.5B 别名 + 大小写不敏感 + 空白容差 + 未知 id 原样透传。
        for alias in [
            "qwen2.5-1.5b",
            "qwen2.5-1.5b-instruct",
            "qwen2.5-1.5b-instruct-q4_k_m",
            "qwen2.5-1.5b-instruct-q4_k_s",
        ] {
            assert_eq!(normalize_polish_model_id(alias), "qwen3.5-2b", "{alias}");
        }
        assert_eq!(
            normalize_polish_model_id(" Qwen2.5-1.5B-Instruct "),
            "qwen3.5-2b"
        );
        assert_eq!(normalize_polish_model_id("custom-model"), "custom-model");
    }

    #[test]
    fn unknown_id_falls_back_to_id_gguf_path_and_uninstalled() {
        // 未知 id（配置被手改/迁移半途）：路径落 {id}.gguf、未安装、resolve 原样返回。
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert_eq!(
            llm_model_path(root, "my-custom-model"),
            root.join(LLM_DIR).join("my-custom-model.gguf")
        );
        assert!(!is_llm_model_installed(root, "my-custom-model"));
        let (id, path) = resolve_llm_id("my-custom-model", root, &no_arch);
        assert_eq!(id, "my-custom-model");
        assert!(path.ends_with("my-custom-model.gguf"));
    }

    #[test]
    fn fallback_manifests_for_2b_and_4b() {
        // 2B/4B 回退档清单：各含首选 + 回退两张 GGUF，双源 + SHA 锁定
        //（配错 SHA 只会在用户下载时才暴露，目录测试提前钉住）。
        for (id, preferred, fb) in [
            (
                "qwen3.5-2b",
                "Qwen_Qwen3.5-2B-Q4_K_M.gguf",
                "Qwen3-1.7B-Q4_K_M.gguf",
            ),
            (
                "qwen3.5-4b",
                "Qwen3.5-4B-Q4_K_M.gguf",
                "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            ),
        ] {
            let files = llm_files(id);
            let names: Vec<_> = files.iter().map(|f| f.file_name).collect();
            assert!(names.contains(&preferred), "{id} 清单缺首选 {preferred}");
            assert!(names.contains(&fb), "{id} 清单缺回退 {fb}");
            for f in files {
                assert!(f.urls.len() >= 2, "{} 应有双源", f.file_name);
                assert!(!f.sha256.is_empty(), "{} 应锁 SHA", f.file_name);
            }
        }
    }

    #[test]
    fn resolve_prefers_installed_then_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // 首选未装 → 落到首选 path（供下载）。
        let (id, path) = resolve_llm_id("qwen3.5-2b", root, &no_arch);
        assert_eq!(id, "qwen3.5-2b");
        assert!(path.ends_with("Qwen_Qwen3.5-2B-Q4_K_M.gguf"));
        // 回退档已装 → 解析到回退。
        let fb_path = llm_model_path(root, "qwen3-1.7b");
        std::fs::create_dir_all(fb_path.parent().unwrap()).unwrap();
        std::fs::write(&fb_path, b"x").unwrap();
        let (id, path) = resolve_llm_id("qwen3.5-2b", root, &no_arch);
        assert_eq!(id, "qwen3-1.7b");
        assert!(path.ends_with("Qwen3-1.7B-Q4_K_M.gguf"));
        // 回退档被标 arch 不支持 → 又回到首选。
        let (_, path) = resolve_llm_id("qwen3.5-2b", root, &|p| p == fb_path.as_path());
        assert!(path.ends_with("Qwen_Qwen3.5-2B-Q4_K_M.gguf"));
    }

    #[test]
    fn installed_accepts_preferred_or_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(!is_llm_model_installed(root, "qwen3.5-2b"));
        // 只放回退档（错误大小 → lenient 判定失败，仍算未装）。
        let fb_path = llm_model_path(root, "qwen3-1.7b");
        std::fs::create_dir_all(fb_path.parent().unwrap()).unwrap();
        std::fs::write(&fb_path, b"x").unwrap();
        assert!(!is_llm_model_installed(root, "qwen3.5-2b"));
    }

    #[test]
    fn rss_bytes_sane() {
        // 常驻估算在 0.5–3GB 区间（打标签数值）。
        for m in polish_catalog()
            .iter()
            .chain(translate_catalog().iter())
            .chain(fallback_catalog().iter())
        {
            assert!(m.rss_bytes >= GB / 2 && m.rss_bytes <= 3 * GB, "{}", m.id);
        }
    }
}
