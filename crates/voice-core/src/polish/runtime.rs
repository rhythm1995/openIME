//! 常驻 GGUF 运行时（本地三件套方案核心）。
//!
//! - 进程级单例：`GgufRuntime` 持有最多两个已加载模型槽（润色档 + 翻译档；
//!   兼译时翻译复用润色 path，翻译槽为空）。
//! - 换档：加载新 path，逐出旧 path；不再每次 `load_from_file`。
//! - 调用仍 `spawn_blocking` + `tokio::time::timeout`（llama.cpp 绑定非 async）。
//! - 无 `llm` feature：与旧版一致返回引导错误（下载仍可用，推理走云端或回退原文）。
//! - Qwen3/3.5：system 末尾打 `/no_think`，生成后剥 `<think>…</think>`（绑定侧
//!   不依赖 template 参数注入，兼容性最好）。

#[cfg(feature = "llm")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::Error;

/// 最多同时驻留的 GGUF 模型数（润色 + 翻译；兼译共用润色槽）。
#[cfg(feature = "llm")]
const MAX_SLOTS: usize = 2;

/// 常驻槽只存模型权重（加载 ~1s 的昂贵部分）。
///
/// `LlamaContext` 在 llama-cpp-2 0.1.x 带生命周期（借用 model），无法与 model
/// 同存一个结构体；每次调用轻量重建 context（n_ctx=2048 的 KV 分配仅数 ms，
/// 且本场景每次都是全新 prompt，KV 无需复用）。
#[cfg(feature = "llm")]
struct LoadedModel {
    model: llama_cpp_2::model::LlamaModel,
}

/// 进程级常驻 GGUF 运行时。
///
/// 内部状态全部 `Arc<Mutex>` 共享（`Clone` 即共享句柄，进程级单例语义）；
/// 加载与解码在同一把锁内串行（单次推理，与「输入法一次一句」的使用方式匹配）。
#[derive(Clone)]
pub struct GgufRuntime {
    #[cfg(feature = "llm")]
    slots: Arc<Mutex<HashMap<PathBuf, LoadedModel>>>,
    /// 加载失败（架构不支持等）的 path 记录，catalog 解析据此换回退档。
    arch_unsupported: Arc<Mutex<HashSet<PathBuf>>>,
    /// 非 `llm` feature 构建下的占位，保证结构体跨 feature 形状稳定。
    #[cfg(not(feature = "llm"))]
    _placeholder: (),
}

impl Default for GgufRuntime {
    fn default() -> Self {
        Self::new()
    }
}

/// 一次补全请求（不携带 path；调用方决定用哪个模型）。
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<(String, String)>,
    /// 生成时上下文长度。
    pub n_ctx: u32,
    pub n_predict: i32,
    /// 采样温度（润色 0.3；翻译 greedy/≤0.3）。
    pub temperature: f32,
    /// Qwen3/3.5 系：system 末尾打 `/no_think` + 输出剥 think。
    pub no_think: bool,
}

impl GgufRuntime {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "llm")]
            slots: Arc::new(Mutex::new(HashMap::new())),
            arch_unsupported: Arc::new(Mutex::new(HashSet::new())),
            #[cfg(not(feature = "llm"))]
            _placeholder: (),
        }
    }

    /// 某 path 是否被记录为「架构不支持」（加载失败过）。
    pub fn arch_unsupported(&self, path: &Path) -> bool {
        self.arch_unsupported
            .lock()
            .map(|s| s.contains(path))
            .unwrap_or(false)
    }

    /// 显式卸载某 path（换档后立即释放，不等逐出策略）。
    pub fn evict(&self, path: &Path) {
        #[cfg(feature = "llm")]
        if let Ok(mut slots) = self.slots.lock() {
            slots.remove(path);
        }
        #[cfg(not(feature = "llm"))]
        let _ = path;
    }

    /// 探测加载（T2 启动期）：只 load 权重验证绑定认不认该架构，不进驻 slots。
    ///
    /// 失败记 `arch_unsupported`（catalog 解析据此换回退档），返回 false。
    /// 无 `llm` feature：不做探测、不记录（推理必然走引导错误，属另一码事）。
    pub fn probe_loadable(&self, model_path: &Path) -> bool {
        #[cfg(feature = "llm")]
        {
            let result = (|| -> crate::Result<()> {
                let backend = llama_cpp_2::llama_backend::LlamaBackend::init()
                    .map_err(|e| Error::Provider(format!("初始化 llama backend 失败: {e}")))?;
                let params = llama_cpp_2::model::params::LlamaModelParams::default();
                llama_cpp_2::model::LlamaModel::load_from_file(&backend, model_path, &params)
                    .map(|_| ())
                    .map_err(|e| Error::Provider(format!("加载 GGUF 失败: {e}")))
            })();
            match result {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("GGUF 探测加载失败（记为架构不支持）：{e}");
                    if let Ok(mut set) = self.arch_unsupported.lock() {
                        set.insert(model_path.to_path_buf());
                    }
                    false
                }
            }
        }
        #[cfg(not(feature = "llm"))]
        {
            let _ = model_path;
            false
        }
    }

    /// 补全入口：已加载则复用，否则加载；`spawn_blocking` + 超时。
    ///
    /// 返回（文本, 耗时 ms）。超时返回 [`Error::Provider`]。
    ///
    /// 超时语义（设计取舍）：timeout 只是放弃**等待**，blocking 线程里的推理仍在
    /// 持有 slots 锁继续跑完——超时后的下一次 `complete` 会先等锁（表现为「超时后
    /// 下一句变慢」）。llama.cpp 绑定非 async，无法中途取消；调用方超时值应覆盖
    /// 最慢机型的单句推理时长，而非依赖取消。
    pub async fn complete(
        &self,
        model_path: PathBuf,
        req: CompletionRequest,
        timeout: Duration,
    ) -> crate::Result<(String, u32)> {
        let fut = tokio::task::spawn_blocking({
            let runtime = self.clone();
            move || runtime.run_blocking(&model_path, req)
        });
        let joined = tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| Error::Provider(format!("本地推理超时（{}ms）", timeout.as_millis())))?
            .map_err(|e| Error::Provider(format!("本地推理任务失败: {e}")))?;
        joined
    }

    #[cfg(feature = "llm")]
    fn run_blocking(
        &self,
        model_path: &Path,
        req: CompletionRequest,
    ) -> crate::Result<(String, u32)> {
        use llama_cpp_2::context::params::LlamaContextParams;
        use llama_cpp_2::llama_backend::LlamaBackend;
        use llama_cpp_2::llama_batch::LlamaBatch;
        use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate};
        use llama_cpp_2::sampling::LlamaSampler;
        use std::num::NonZeroU32;

        let t0 = std::time::Instant::now();
        // backend init 幂等且廉价；模型与上下文本身常驻在 slots。
        let backend = LlamaBackend::init()
            .map_err(|e| Error::Provider(format!("初始化 llama backend 失败: {e}")))?;

        let mut slots = self
            .slots
            .lock()
            .map_err(|_| Error::Provider("GGUF 运行时锁中毒".to_string()))?;

        if !slots.contains_key(model_path) {
            // 逐出策略：槽满时先丢一个「不是本次要加载的」旧模型。
            if slots.len() >= MAX_SLOTS {
                if let Some(victim) =
                    pick_eviction_victim(slots.keys().map(std::path::PathBuf::as_path), model_path)
                {
                    let victim = victim.to_path_buf(); // 克隆解除对 slots 的借用。
                    tracing::info!("GGUF 槽满，逐出 {}", victim.display());
                    slots.remove(&victim);
                }
            }
            let loaded = match load_model(&backend, model_path) {
                Ok(l) => l,
                Err(e) => {
                    // 记下「该 path 加载失败」：catalog 解析据此换回退档，避免每次录音试错。
                    if let Ok(mut set) = self.arch_unsupported.lock() {
                        set.insert(model_path.to_path_buf());
                    }
                    return Err(e);
                }
            };
            tracing::info!("GGUF 加载完成：{}", model_path.display());
            slots.insert(model_path.to_path_buf(), loaded);
        }

        let loaded = slots.get_mut(model_path).expect("刚插入的槽必在");
        let model = &loaded.model;
        // context 每次调用轻量重建（见 LoadedModel 注释）；权重常驻是省时的关键。
        let ctx_params = LlamaContextParams::default().with_n_ctx(Some(
            NonZeroU32::new(req.n_ctx.max(512)).unwrap_or(NonZeroU32::new(2048).unwrap()),
        ));
        let mut ctx = model
            .new_context(&backend, ctx_params)
            .map_err(|e| Error::Provider(format!("创建 llama context 失败: {e}")))?;

        // chat template：优先模型内置，拿不到回退 chatml（Qwen2.5/3 系兼容）。
        let tmpl = model
            .chat_template(None)
            .or_else(|_| LlamaChatTemplate::new("chatml"))
            .map_err(|e| Error::Provider(format!("取 chat_template 失败: {e}")))?;

        let mut msgs: Vec<(String, String)> = req.messages.clone();
        if req.no_think {
            // Qwen3 系：user 消息末尾打 /no_think，抑制思维链（无副作用于其它模板）。
            if let Some(last) = msgs.last_mut() {
                last.1.push_str("\n/no_think");
            }
        }
        let chat_msgs: Vec<LlamaChatMessage> = msgs
            .iter()
            .filter_map(|(role, content)| LlamaChatMessage::new(role.clone(), content.clone()).ok())
            .collect();

        let prompt = model
            .apply_chat_template(&tmpl, &chat_msgs, true)
            .map_err(|e| Error::Provider(format!("apply_chat_template 失败: {e}")))?;

        let tokens = model
            .str_to_token(&prompt, AddBos::Always)
            .map_err(|e| Error::Provider(format!("tokenize 失败: {e}")))?;

        let mut batch = LlamaBatch::new(tokens.len() + req.n_predict.max(1) as usize, 1);
        let last_idx = tokens.len().saturating_sub(1);
        for (i, token) in tokens.into_iter().enumerate() {
            batch
                .add(token, i as i32, &[0], i == last_idx)
                .map_err(|e| Error::Provider(format!("batch.add 失败: {e}")))?;
        }
        ctx.decode(&mut batch)
            .map_err(|e| Error::Provider(format!("prompt decode 失败: {e}")))?;

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::temp(req.temperature),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::greedy(),
        ]);

        let mut out = String::new();
        let mut n_cur = batch.n_tokens();
        // token_to_piece 的流式 UTF-8 解码器（跨 token 边界正确拼接）。
        let mut decoder = encoding_rs::UTF_8.new_decoder();
        for _ in 0..req.n_predict.max(1) {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .unwrap_or_default();
            out.push_str(&piece);
            batch.clear();
            batch
                .add(token, n_cur, &[0], true)
                .map_err(|e| Error::Provider(format!("decode step 失败: {e}")))?;
            n_cur += 1;
            ctx.decode(&mut batch)
                .map_err(|e| Error::Provider(format!("decode 失败: {e}")))?;
        }

        let text = strip_think(&out);
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(Error::Provider("本地推理返回空文本".into()));
        }
        Ok((text, t0.elapsed().as_millis() as u32))
    }

    #[cfg(not(feature = "llm"))]
    fn run_blocking(
        &self,
        _model_path: &Path,
        _req: CompletionRequest,
    ) -> crate::Result<(String, u32)> {
        Err(Error::Provider(
            "本地推理引擎未启用：请安装 cmake 后以 `--features llm` 编译（llama.cpp + GGUF）。\
             也可配置云端 endpoint 使用云端润色/翻译。"
                .into(),
        ))
    }
}

#[cfg(feature = "llm")]
fn load_model(
    backend: &llama_cpp_2::llama_backend::LlamaBackend,
    model_path: &Path,
) -> crate::Result<LoadedModel> {
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::LlamaModel;

    let model_params = LlamaModelParams::default();
    let model = LlamaModel::load_from_file(backend, model_path, &model_params)
        .map_err(|e| Error::Provider(format!("加载 GGUF 失败: {e}")))?;
    Ok(LoadedModel { model })
}

/// 槽满时的逐出受害者选择（纯决策，可单测）：返回一个「不是本次要加载路径」的
/// 已驻留 path；无候选（槽未满由调用方判断 / 全部是 incoming，后者不可能）返回 None。
pub fn pick_eviction_victim<'a>(
    resident: impl IntoIterator<Item = &'a Path>,
    incoming: &Path,
) -> Option<&'a Path> {
    resident.into_iter().find(|k| *k != incoming)
}

/// 剥掉 Qwen3 系思维链块（`<think>…</think>`）与 `response` 模板前缀，全 feature 可用。
///
/// 只剥**前导**块（思维链只会出现在输出开头）；文本中部的 `<think>` 可能是
/// 用户内容（如润色/翻译原文本身含标签），不动。
pub fn strip_think(text: &str) -> String {
    let mut out = text.trim_start();
    // 贪婪剥多段（一段为常，防御嵌套/多次输出）。
    loop {
        if starts_with_ci(out, "response") {
            out = out["response".len()..].trim_start();
            continue;
        }
        if starts_with_ci(out, "<think") {
            if let Some(end) = find_ci_after(out, "<think".len(), "</think>") {
                out = out[end..].trim_start();
                continue;
            }
        }
        break;
    }
    out.trim().to_string()
}

/// ASCII 前缀大小写不敏感比较（字节级，UTF-8 多字节序列不会与 ASCII 混淆）。
fn starts_with_ci(s: &str, prefix: &str) -> bool {
    let (sb, pb) = (s.as_bytes(), prefix.as_bytes());
    sb.len() >= pb.len()
        && sb[..pb.len()]
            .iter()
            .zip(pb)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// 从 `from` 字节起找 `needle`（ASCII、大小写不敏感），返回其**结束**位置。
fn find_ci_after(s: &str, from: usize, needle: &str) -> Option<usize> {
    let (sb, nb) = (s.as_bytes(), needle.as_bytes());
    if nb.is_empty() || from >= sb.len() {
        return None;
    }
    let mut i = from;
    while i + nb.len() <= sb.len() {
        if sb[i..i + nb.len()]
            .iter()
            .zip(nb)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return Some(i + nb.len());
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_think_removes_blocks() {
        assert_eq!(strip_think("<think>嗯，用户要翻译。</think>你好"), "你好");
        assert_eq!(strip_think("你好"), "你好");
        // Qwen3 模板的 `response` 前缀。
        assert_eq!(strip_think(" response今日天气不错"), "今日天气不错");
        // 无闭合标签不剥。
        assert_eq!(strip_think("<think>未闭合"), "<think>未闭合");
    }

    #[test]
    fn strip_think_removes_consecutive_blocks() {
        // 贪婪剥多段：小模型偶发多次输出思维链。
        assert_eq!(
            strip_think("<think>第一段</think><think>第二段</think>你好"),
            "你好"
        );
        // response 前缀 + think 块混合链。
        assert_eq!(strip_think("response<think>推理</think>答案"), "答案");
    }

    #[test]
    fn strip_think_is_case_insensitive() {
        // 大写标签（部分模板输出 <THINK>）也能剥。
        assert_eq!(strip_think("<THINK>hidden</THINK>visible"), "visible");
        assert_eq!(strip_think("Response可见"), "可见");
    }

    #[test]
    fn strip_think_only_strips_leading_blocks() {
        // 文本中部的 think 块不是思维链（可能是用户内容），不剥。
        assert_eq!(
            strip_think("前文<think>中段</think>后文"),
            "前文<think>中段</think>后文"
        );
    }

    #[test]
    fn probe_loadable_without_llm_feature_is_false_and_silent() {
        // 非 llm 构建：探测不做、不记 arch_unsupported（推理引导错误是另一条路径）。
        let rt = GgufRuntime::new();
        let p = PathBuf::from("/nonexistent/model.gguf");
        assert!(!rt.probe_loadable(&p));
        assert!(!rt.arch_unsupported(&p));
    }

    #[tokio::test]
    async fn complete_without_llm_feature_returns_guidance() {
        // 无 llm feature 的构建：推理必须返回引导错误而不是 panic。
        let rt = GgufRuntime::new();
        let req = CompletionRequest {
            messages: vec![("user".into(), "你好".into())],
            n_ctx: 2048,
            n_predict: 16,
            temperature: 0.3,
            no_think: false,
        };
        let res = rt
            .complete(
                PathBuf::from("/nonexistent/model.gguf"),
                req,
                Duration::from_secs(2),
            )
            .await;
        #[cfg(feature = "llm")]
        assert!(res.is_err()); // 无 llm feature 时必然 Err；有 feature 时路径不存在也 Err。
        #[cfg(not(feature = "llm"))]
        {
            let err = res.expect_err("非 llm 构建应报引导错误");
            assert!(format!("{err}").contains("llm"));
        }
    }

    #[test]
    fn eviction_picks_non_incoming_resident() {
        let a = Path::new("/models/a.gguf");
        let b = Path::new("/models/b.gguf");
        // 空槽集合 → 无受害者。
        let empty: [&Path; 0] = [];
        assert!(pick_eviction_victim(empty, a).is_none());
        // resident 含 incoming 自己 + 另一个 → 选另一个（绝不逐出本次要加载的）。
        assert_eq!(pick_eviction_victim([a, b], a), Some(b));
        assert_eq!(pick_eviction_victim([b, a], a), Some(b));
        // 全部是 incoming（实际不可能：加载前 contains_key 已排除）→ None，不误删。
        assert!(pick_eviction_victim([a], a).is_none());
        // 多个非 incoming → 取第一个（与原 slots.keys().find 语义一致）。
        let c = Path::new("/models/c.gguf");
        assert_eq!(pick_eviction_victim([b, c], a), Some(b));
    }

    /// 手动真机验证（默认忽略）：需要 `--features llm` 编译 + 环境变量 `GGUF_MODEL_PATH`
    /// 指向已下载的真实 GGUF。覆盖 probe_loadable 真实加载与 complete 端到端一次补全
    ///（槽缓存复用 / chat template / 采样 / strip_think 全链）。
    ///
    /// 运行：`GGUF_MODEL_PATH=… cargo test -p voice-core --features llm gguf_real -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实 GGUF 模型（GGUF_MODEL_PATH）+ --features llm 编译"]
    #[cfg(feature = "llm")]
    async fn gguf_real_probe_and_complete() {
        let path = PathBuf::from(std::env::var("GGUF_MODEL_PATH").expect("设置 GGUF_MODEL_PATH"));
        let rt = GgufRuntime::new();
        assert!(rt.probe_loadable(&path), "真实模型应可加载");
        let req = CompletionRequest {
            messages: vec![("user".into(), "把这句话原样输出：你好世界".into())],
            n_ctx: 2048,
            n_predict: 64,
            temperature: 0.3,
            no_think: false,
        };
        let (text, ms) = rt
            .complete(path, req, Duration::from_secs(120))
            .await
            .expect("端到端补全应成功");
        assert!(!text.trim().is_empty(), "输出不应为空");
        println!("gguf_real 输出（{ms}ms）：{text}");
    }
}
