//! 本地 GGUF 润色 / 翻译（Qwen3.5 / MiLMMT / HY-MT 等，目录见 `llm_catalog`）。
//!
//! - 推理统一走常驻 [`crate::polish::GgufRuntime`]（不再每次 `load_from_file`）。
//! - [`LocalGgufPolish`]：实现 [`TextPolishProvider`]，听写润色。
//! - [`LocalGgufTranslate`]：实现 [`LlmClient::translate_text`]，本地专翻 / 兼译；
//!   `polish` / `polish_and_translate` / `chat_stream` 返回「不支持」。
//! - 未开启 `llm` feature：运行时返回引导错误（与 sherpa stub 同风格）。

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::polish::prompts::{
    build_local_translate_messages, build_messages, detect_source_lang, lang_display_name,
    looks_like_instruction_leak,
};
use crate::polish::runtime::{CompletionRequest, GgufRuntime};
use crate::traits::{PolishMode, PolishRequest, PolishResponse, TextPolishProvider};
use crate::{Error, Result};

/// 目录 id 是否属于 Qwen3/3.5 系（需关 thinking）。
pub fn arch_needs_no_think(model_id: &str) -> bool {
    model_id.starts_with("qwen3") || model_id.starts_with("qwen2.5")
}

/// 本地 GGUF 润色。
pub struct LocalGgufPolish {
    pub runtime: Arc<GgufRuntime>,
    pub model_path: PathBuf,
    /// 目录 id（取 n_predict / 关 thinking 等参数用）。
    pub model_id: String,
    /// 生成时上下文长度（短改写足够小）。
    pub n_ctx: u32,
    pub n_predict: i32,
}

impl LocalGgufPolish {
    pub fn new(
        runtime: Arc<GgufRuntime>,
        model_path: impl Into<PathBuf>,
        model_id: impl Into<String>,
    ) -> Self {
        let model_id = model_id.into();
        let n_predict = crate::llm_catalog::llm_model_by_id(&model_id)
            .map(|m| m.n_predict)
            .unwrap_or(128);
        Self {
            runtime,
            model_path: model_path.into(),
            model_id,
            n_ctx: 2048,
            n_predict,
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

        let messages = build_messages(
            &req.text,
            req.mode,
            &req.hotwords,
            req.style_prompt.as_deref(),
        );
        let completion = CompletionRequest {
            messages,
            n_ctx: self.n_ctx,
            n_predict: self.n_predict,
            temperature: 0.3,
            no_think: arch_needs_no_think(&self.model_id),
        };
        let (text, ms) = self
            .runtime
            .complete(self.model_path.clone(), completion, req.timeout)
            .await?;
        Ok(PolishResponse {
            text,
            provider: "local-gguf".into(),
            latency_ms: ms,
        })
    }
}

/// 本地 GGUF 翻译（专翻 / 兼译同一实现，prompt 按 `model_id` 选模板）。
pub struct LocalGgufTranslate {
    pub runtime: Arc<GgufRuntime>,
    pub model_path: PathBuf,
    pub model_id: String,
    pub n_ctx: u32,
    pub n_predict: i32,
}

impl LocalGgufTranslate {
    pub fn new(
        runtime: Arc<GgufRuntime>,
        model_path: impl Into<PathBuf>,
        model_id: impl Into<String>,
    ) -> Self {
        let model_id = model_id.into();
        let n_predict = crate::llm_catalog::llm_model_by_id(&model_id)
            .map(|m| m.n_predict)
            .unwrap_or(256);
        Self {
            runtime,
            model_path: model_path.into(),
            model_id,
            n_ctx: 2048,
            n_predict,
        }
    }

    pub fn model_exists(&self) -> bool {
        self.model_path.is_file()
    }
}

#[async_trait]
impl crate::LlmClient for LocalGgufTranslate {
    /// 本地翻译（R4/R5 + 兼译）。专翻模型用官方模板，兼译用通用 Instruct。
    async fn translate_text(&self, req: crate::TranslateRequest) -> Result<String> {
        if req.text.trim().is_empty() {
            return Ok(String::new());
        }
        if !self.model_exists() {
            return Err(Error::Provider(format!(
                "本地翻译模型未安装：{}",
                self.model_path.display()
            )));
        }
        let src = detect_source_lang(&req.text, &req.source_lang);
        let tgt = lang_display_name(&req.target_lang);
        let messages = build_local_translate_messages(&self.model_id, &req.text, &src, tgt);
        let completion = CompletionRequest {
            messages,
            n_ctx: self.n_ctx,
            n_predict: self.n_predict,
            // 翻译抖动比润色更不可接受：接近 greedy。
            temperature: 0.2,
            no_think: arch_needs_no_think(&self.model_id),
        };
        let (text, _ms) = self
            .runtime
            .complete(self.model_path.clone(), completion, req.timeout)
            .await?;
        if looks_like_instruction_leak(&text) {
            return Err(Error::Provider("本地翻译输出疑似指令泄漏，视为失败".into()));
        }
        Ok(text)
    }

    /// 本地不做润色（专用实现只译）。
    async fn polish(&self, _req: PolishRequest) -> Result<PolishResponse> {
        Err(Error::Provider("本地翻译模型不支持润色".into()))
    }

    /// 本地禁止哨兵合成（方案：本地一律两步 Light → 译）。
    async fn polish_and_translate(
        &self,
        _req: crate::TranslateRequest,
    ) -> Result<crate::PolishTranslate> {
        Err(Error::Provider(
            "本地翻译不支持哨兵合成（应先 Light 润色再译）".into(),
        ))
    }

    /// QA 流式保持云端（小模型质量不承诺）。
    async fn chat_stream(&self, _req: crate::ChatRequest) -> Result<String> {
        Err(Error::Provider("本地翻译模型不支持流式问答".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LlmClient;

    #[test]
    fn no_think_arch_heuristic() {
        assert!(arch_needs_no_think("qwen3.5-2b"));
        assert!(arch_needs_no_think("qwen3-1.7b"));
        assert!(!arch_needs_no_think("milmmt-1b"));
        assert!(!arch_needs_no_think("hy-mt-1.8b"));
    }

    #[test]
    fn n_predict_from_catalog() {
        let rt = Arc::new(GgufRuntime::new());
        let p = LocalGgufPolish::new(rt.clone(), "/tmp/x.gguf", "qwen3.5-2b");
        assert_eq!(p.n_predict, 128);
        let t = LocalGgufTranslate::new(rt, "/tmp/y.gguf", "milmmt-1b");
        assert_eq!(t.n_predict, 256);
    }

    #[tokio::test]
    async fn translate_missing_model_errors() {
        let rt = Arc::new(GgufRuntime::new());
        let t = LocalGgufTranslate::new(rt, "/nonexistent/not-installed.gguf", "milmmt-1b");
        let req = crate::TranslateRequest {
            text: "你好".into(),
            target_lang: "English".into(),
            timeout: std::time::Duration::from_secs(1),
            max_tokens: 256,
            source_lang: "auto".into(),
        };
        let err = t.translate_text(req).await.unwrap_err();
        assert!(format!("{err}").contains("未安装"));
    }

    fn polish_req(mode: PolishMode, text: &str) -> PolishRequest {
        PolishRequest {
            text: text.into(),
            mode,
            style_prompt: None,
            hotwords: vec![],
            timeout: std::time::Duration::from_secs(1),
            max_tokens: None,
        }
    }

    #[tokio::test]
    async fn polish_passthrough_on_off_mode_and_empty_text() {
        // Off / 空文本早退：原样返回 + provider=passthrough（pipeline 据此视为未润色）。
        let rt = Arc::new(GgufRuntime::new());
        let p = LocalGgufPolish::new(rt, "/nonexistent/x.gguf", "qwen3.5-2b");
        let r = p.polish(polish_req(PolishMode::Off, "原文")).await.unwrap();
        assert_eq!(r.text, "原文");
        assert_eq!(r.provider, "passthrough");
        let r = p
            .polish(polish_req(PolishMode::Light, "   "))
            .await
            .unwrap();
        assert_eq!(r.provider, "passthrough");
    }

    #[tokio::test]
    async fn polish_missing_model_errors() {
        // 模型文件缺失 → 明确的「未安装」错误（早于任何推理调用）。
        let rt = Arc::new(GgufRuntime::new());
        let p = LocalGgufPolish::new(rt, "/nonexistent/x.gguf", "qwen3.5-2b");
        let err = p
            .polish(polish_req(PolishMode::Light, "这句话足够长会走到模型检查"))
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("未安装"));
    }

    #[tokio::test]
    async fn translate_empty_text_returns_empty_without_model_check() {
        // 空文本早退在模型检查之前：即使模型未装也 Ok("")。
        let rt = Arc::new(GgufRuntime::new());
        let t = LocalGgufTranslate::new(rt, "/nonexistent/y.gguf", "milmmt-1b");
        let req = crate::TranslateRequest {
            text: "   ".into(),
            target_lang: "English".into(),
            timeout: std::time::Duration::from_secs(1),
            max_tokens: 256,
            source_lang: "auto".into(),
        };
        assert_eq!(t.translate_text(req).await.unwrap(), "");
    }

    #[tokio::test]
    async fn translate_handle_rejects_non_translate_operations() {
        // 方案契约：本地专翻只译——polish / 哨兵合成 / 流式问答一律「不支持」。
        let rt = Arc::new(GgufRuntime::new());
        let t = LocalGgufTranslate::new(rt, "/nonexistent/y.gguf", "milmmt-1b");
        let tr = crate::TranslateRequest {
            text: "hi".into(),
            target_lang: "English".into(),
            timeout: std::time::Duration::from_secs(1),
            max_tokens: 64,
            source_lang: "auto".into(),
        };
        let e = t
            .polish(polish_req(PolishMode::Light, "x"))
            .await
            .unwrap_err();
        assert!(format!("{e}").contains("不支持润色"));
        let e = t.polish_and_translate(tr).await.unwrap_err();
        assert!(format!("{e}").contains("哨兵"));
        let e = t
            .chat_stream(crate::ChatRequest {
                messages: vec![("user".into(), "hi".into())],
                timeout: std::time::Duration::from_secs(1),
                max_tokens: 64,
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                gen: 0,
                on_delta: Box::new(|_| {}),
            })
            .await
            .unwrap_err();
        assert!(format!("{e}").contains("流式"));
    }
}
