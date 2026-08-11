//! 本地 GGUF 润色（Qwen2.5-1.5B-Instruct Q4_K_M 等）。
//!
//! - 开启 `llm` feature：llama.cpp 进程内推理。
//! - 未开启：返回明确错误（与 sherpa stub 同风格）。

use std::path::{Path, PathBuf};
use std::time::Instant;

use async_trait::async_trait;

use crate::traits::{PolishMode, PolishRequest, PolishResponse, TextPolishProvider};
use crate::{Error, Result};

use super::prompts::build_messages;

/// 本地 GGUF 润色。
pub struct LocalGgufPolish {
    pub model_path: PathBuf,
    /// 生成时上下文长度（短改写足够小）。
    pub n_ctx: u32,
    pub n_predict: i32,
}

impl LocalGgufPolish {
    pub fn new(model_path: impl Into<PathBuf>) -> Self {
        Self {
            model_path: model_path.into(),
            n_ctx: 2048,
            n_predict: 128,
        }
    }

    pub fn model_exists(&self) -> bool {
        self.model_path.is_file()
    }
}

#[async_trait]
impl TextPolishProvider for LocalGgufPolish {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse> {
        if req.mode == PolishMode::Off || req.text.trim().is_empty() {
            return Ok(PolishResponse {
                text: req.text,
                provider: "passthrough".into(),
                latency_ms: 0,
            });
        }
        if !self.model_exists() {
            return Err(Error::Provider(format!(
                "本地润色模型未安装：{}",
                self.model_path.display()
            )));
        }

        let path = self.model_path.clone();
        let n_ctx = self.n_ctx;
        let n_predict = self.n_predict;
        let messages = build_messages(
            &req.text,
            req.mode,
            req.persona_prompt.as_deref(),
            &req.hotwords,
        );
        let timeout = req.timeout;

        // llama.cpp 绑定非 async：丢到 blocking 线程，再套超时。
        let fut = tokio::task::spawn_blocking(move || {
            run_gguf_completion(&path, &messages, n_ctx, n_predict)
        });
        let joined = tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| Error::Provider(format!("本地润色超时（{}ms）", timeout.as_millis())))?
            .map_err(|e| Error::Provider(format!("本地润色任务失败: {e}")))?;

        let t0 = Instant::now(); // latency 在 run 内更准；此处兜底
        let (text, ms) = joined?;
        Ok(PolishResponse {
            text,
            provider: "local-gguf".into(),
            latency_ms: if ms > 0 {
                ms
            } else {
                t0.elapsed().as_millis() as u32
            },
        })
    }
}

/// 本地推理入口。
///
/// `llm` feature 开启且系统装有 cmake 时链接 llama.cpp；否则返回引导错误。
/// （当前环境若未装 cmake，默认构建不含 llm，下载 GGUF 仍可用，推理走云端或回退原文。）
fn run_gguf_completion(
    model_path: &Path,
    messages: &[(String, String)],
    n_ctx: u32,
    n_predict: i32,
) -> Result<(String, u32)> {
    #[cfg(feature = "llm")]
    {
        return run_gguf_completion_llama(model_path, messages, n_ctx, n_predict);
    }
    #[cfg(not(feature = "llm"))]
    {
        let _ = (model_path, messages, n_ctx, n_predict);
        Err(Error::Provider(
            "本地润色引擎未启用：请安装 cmake 后以 `--features llm` 编译（llama.cpp + GGUF）。\
             也可配置百炼 api_key 使用云端润色。"
                .into(),
        ))
    }
}

#[cfg(feature = "llm")]
fn run_gguf_completion_llama(
    model_path: &Path,
    messages: &[(String, String)],
    n_ctx: u32,
    n_predict: i32,
) -> Result<(String, u32)> {
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_backend::LlamaBackend;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::params::LlamaModelParams;
    use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
    use llama_cpp_2::sampling::LlamaSampler;
    use std::num::NonZeroU32;

    let t0 = Instant::now();
    let backend = LlamaBackend::init()
        .map_err(|e| Error::Provider(format!("初始化 llama backend 失败: {e}")))?;

    let model_params = LlamaModelParams::default();
    let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
        .map_err(|e| Error::Provider(format!("加载 GGUF 失败: {e}")))?;

    let ctx_params = LlamaContextParams::default().with_n_ctx(Some(
        NonZeroU32::new(n_ctx).unwrap_or(NonZeroU32::new(2048).unwrap()),
    ));
    let mut ctx = model
        .new_context(&backend, ctx_params)
        .map_err(|e| Error::Provider(format!("创建 llama context 失败: {e}")))?;

    let chat_msgs: Vec<LlamaChatMessage> = messages
        .iter()
        .filter_map(|(role, content)| LlamaChatMessage::new(role.clone(), content.clone()).ok())
        .collect();

    // 新版 llama-cpp-2：apply_chat_template 需要显式 &LlamaChatTemplate。
    // 优先用模型内置模板（chat_template），拿不到则回退 chatml（Qwen2.5 Instruct 兼容）。
    let tmpl = model
        .chat_template(None)
        .or_else(|_| LlamaChatTemplate::new("chatml"))
        .map_err(|e| Error::Provider(format!("取 chat_template 失败: {e}")))?;
    let prompt = model
        .apply_chat_template(&tmpl, &chat_msgs, true)
        .map_err(|e| Error::Provider(format!("apply_chat_template 失败: {e}")))?;

    let tokens = model
        .str_to_token(&prompt, AddBos::Always)
        .map_err(|e| Error::Provider(format!("tokenize 失败: {e}")))?;

    let mut batch = LlamaBatch::new(tokens.len() + n_predict.max(1) as usize, 1);
    let last_idx = tokens.len().saturating_sub(1);
    for (i, token) in tokens.into_iter().enumerate() {
        batch
            .add(token, i as i32, &[0], i == last_idx)
            .map_err(|e| Error::Provider(format!("batch.add 失败: {e}")))?;
    }
    ctx.decode(&mut batch)
        .map_err(|e| Error::Provider(format!("prompt decode 失败: {e}")))?;

    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::temp(0.3),
        LlamaSampler::top_p(0.9, 1),
        LlamaSampler::greedy(),
    ]);

    let mut out = String::new();
    let mut n_cur = batch.n_tokens();
    for _ in 0..n_predict.max(1) {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.is_eog_token(token) {
            break;
        }
        let piece = model
            .token_to_str(token, llama_cpp_2::model::Special::Tokenize)
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

    let text = out.trim().to_string();
    if text.is_empty() {
        return Err(Error::Provider("本地润色返回空文本".into()));
    }
    Ok((text, t0.elapsed().as_millis() as u32))
}
