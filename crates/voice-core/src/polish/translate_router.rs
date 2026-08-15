//! 翻译路由（本地三件套方案 T5）。
//!
//! 与润色的 `PreferLocal` 相反，翻译默认 `PreferCloud`（有网默认走云）。
//!
//! ```text
//! PreferCloud（默认）: 云 → 专翻 → 兼译 → 原文
//! PreferLocal:         专翻 → 兼译 → 云 → 原文
//! ```
//!
//! 全失败返回 Err，由 pipeline 插入 L0 原文 + `TranslateFailed`。
//! 云端哨兵合成（`polish_and_translate`）仅当云端是第一跳时可用；本地一律两步
//! （译前 Light 由 pipeline 完成，本模块不参与）。

use std::sync::Arc;

#[cfg(test)]
use async_trait::async_trait;

use crate::config::TranslatePolicy;
use crate::polish::llm::{LlmClient, PolishTranslate, TranslateRequest};
use crate::{Error, Result};

/// 复合翻译路由：云端 + 本地专翻 + 兼译（润色模型），按策略试。
pub struct TranslateRouter {
    pub policy: TranslatePolicy,
    pub cloud: Option<Arc<dyn LlmClient>>,
    /// 本地专翻（MiLMMT / HY-MT）。
    pub dedicated: Option<Arc<dyn LlmClient>>,
    /// 兼译句柄（润色模型兼做翻译）。
    pub llm_fallback: Option<Arc<dyn LlmClient>>,
    /// 是否允许使用兼译（弱机/专翻未装时由配置开启）。
    pub use_llm_fallback: bool,
}

impl TranslateRouter {
    /// 无任何可用后端（前端「可否开始」检查用）。
    pub fn is_empty(&self) -> bool {
        self.cloud.is_none() && self.dedicated.is_none() && self.fallback().is_none()
    }

    /// 云端是否为第一跳（决定是否可用哨兵合成）。
    pub fn first_hop_is_cloud(&self) -> bool {
        match self.policy {
            TranslatePolicy::PreferCloud => self.cloud.is_some(),
            TranslatePolicy::PreferLocal => {
                self.dedicated.is_none() && self.fallback().is_none() && self.cloud.is_some()
            }
        }
    }

    fn fallback(&self) -> Option<&Arc<dyn LlmClient>> {
        if self.use_llm_fallback {
            self.llm_fallback.as_ref()
        } else {
            None
        }
    }

    /// 按策略排好的后端顺序。
    fn order(&self) -> Vec<Arc<dyn LlmClient>> {
        let mut out = Vec::new();
        match self.policy {
            TranslatePolicy::PreferCloud => {
                if let Some(c) = &self.cloud {
                    out.push(c.clone());
                }
                if let Some(d) = &self.dedicated {
                    out.push(d.clone());
                }
                if let Some(f) = self.fallback() {
                    out.push(f.clone());
                }
            }
            TranslatePolicy::PreferLocal => {
                if let Some(d) = &self.dedicated {
                    out.push(d.clone());
                }
                if let Some(f) = self.fallback() {
                    out.push(f.clone());
                }
                if let Some(c) = &self.cloud {
                    out.push(c.clone());
                }
            }
        }
        out
    }

    /// 按策略依次翻译；全部失败 → Err（pipeline 插 L0 + TranslateFailed）。
    pub async fn translate(&self, req: &TranslateRequest) -> Result<String> {
        let mut last_err: Option<Error> = None;
        for client in self.order() {
            match client.translate_text(req.clone()).await {
                Ok(t) if !t.trim().is_empty() => return Ok(t.trim().to_string()),
                Ok(_) => last_err = Some(Error::Provider("翻译输出为空".into())),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Provider("无可用翻译后端".into())))
    }

    /// 云端哨兵合成（仅当云端是第一跳时调用；本地禁止哨兵）。
    pub async fn polish_and_translate(&self, req: &TranslateRequest) -> Result<PolishTranslate> {
        match &self.cloud {
            Some(c) => c.polish_and_translate(req.clone()).await,
            None => Err(Error::Provider("哨兵合成仅云端可用".into())),
        }
    }
}

/// 测试用：可编程的翻译后端（`ok: None` = 返回 Err）。
#[cfg(test)]
pub struct StubTranslate {
    pub ok: Option<String>,
    pub calls: std::sync::atomic::AtomicU32,
}

#[cfg(test)]
impl StubTranslate {
    pub fn ok(text: &str) -> Self {
        Self {
            ok: Some(text.into()),
            calls: std::sync::atomic::AtomicU32::new(0),
        }
    }
    pub fn err() -> Self {
        Self {
            ok: None,
            calls: std::sync::atomic::AtomicU32::new(0),
        }
    }
}

#[cfg(test)]
#[async_trait]
impl LlmClient for StubTranslate {
    async fn translate_text(&self, _req: TranslateRequest) -> Result<String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match &self.ok {
            Some(t) => Ok(t.clone()),
            None => Err(Error::Provider("stub fail".into())),
        }
    }
    async fn polish(&self, _req: crate::PolishRequest) -> Result<crate::PolishResponse> {
        Err(Error::Provider("stub".into()))
    }
    async fn polish_and_translate(&self, _req: TranslateRequest) -> Result<PolishTranslate> {
        Err(Error::Provider("stub".into()))
    }
    async fn chat_stream(&self, _req: crate::ChatRequest) -> Result<String> {
        Err(Error::Provider("stub".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    fn req() -> TranslateRequest {
        TranslateRequest {
            text: "你好".into(),
            target_lang: "English".into(),
            source_lang: "zh".into(),
            timeout: Duration::from_secs(1),
            max_tokens: 256,
        }
    }

    #[test]
    fn first_hop_heuristics() {
        // 云第一跳：PreferCloud 有云；或 PreferLocal 无任何本地后端只剩云。
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: Some(Arc::new(StubTranslate::ok("c"))),
            dedicated: Some(Arc::new(StubTranslate::ok("d"))),
            llm_fallback: Some(Arc::new(StubTranslate::ok("f"))),
            use_llm_fallback: true,
        };
        assert!(r.first_hop_is_cloud());
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferLocal,
            cloud: Some(Arc::new(StubTranslate::ok("c"))),
            dedicated: None,
            llm_fallback: None,
            use_llm_fallback: false,
        };
        assert!(r.first_hop_is_cloud());
    }

    #[test]
    fn order_prefer_local_skips_cloud_first() {
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferLocal,
            cloud: Some(Arc::new(StubTranslate::ok("c"))),
            dedicated: Some(Arc::new(StubTranslate::ok("d"))),
            llm_fallback: None,
            use_llm_fallback: false,
        };
        assert!(!r.first_hop_is_cloud());
    }

    #[tokio::test]
    async fn prefer_cloud_uses_cloud_first() {
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: Some(Arc::new(StubTranslate::ok("cloud 译文"))),
            dedicated: Some(Arc::new(StubTranslate::err())),
            llm_fallback: None,
            use_llm_fallback: false,
        };
        let out = r.translate(&req()).await.unwrap();
        assert_eq!(out, "cloud 译文");
    }

    #[tokio::test]
    async fn prefer_cloud_falls_to_dedicated_then_fallback() {
        // 云失败 → 专翻成功。
        let dedi = Arc::new(StubTranslate::ok("本地专翻"));
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: Some(Arc::new(StubTranslate::err())),
            dedicated: Some(dedi.clone()),
            llm_fallback: None,
            use_llm_fallback: false,
        };
        assert_eq!(r.translate(&req()).await.unwrap(), "本地专翻");
        assert_eq!(dedi.calls.load(Ordering::SeqCst), 1);

        // 云失败 → 专翻失败 → 兼译兜底。
        let fb = Arc::new(StubTranslate::ok("兼译"));
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: Some(Arc::new(StubTranslate::err())),
            dedicated: Some(Arc::new(StubTranslate::err())),
            llm_fallback: Some(fb),
            use_llm_fallback: true,
        };
        assert_eq!(r.translate(&req()).await.unwrap(), "兼译");
    }

    #[tokio::test]
    async fn empty_translation_skips_to_next_backend() {
        // 专翻小模型空输出（真实故障模式）不当作成功，继续降级到兼译。
        let fb = Arc::new(StubTranslate::ok("兼译译文"));
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: Some(Arc::new(StubTranslate::err())),
            dedicated: Some(Arc::new(StubTranslate::ok("   "))),
            llm_fallback: Some(fb.clone()),
            use_llm_fallback: true,
        };
        assert_eq!(r.translate(&req()).await.unwrap(), "兼译译文");
        assert_eq!(fb.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn all_empty_outputs_report_empty_error() {
        // 所有后端都返回空文本 → Err 文案指向「翻译输出为空」（区分于网络/推理失败）。
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferLocal,
            cloud: Some(Arc::new(StubTranslate::ok(""))),
            dedicated: Some(Arc::new(StubTranslate::ok(""))),
            llm_fallback: None,
            use_llm_fallback: false,
        };
        let err = r.translate(&req()).await.unwrap_err();
        assert!(err.to_string().contains("翻译输出为空"), "got: {err}");
    }

    #[tokio::test]
    async fn whitespace_output_trimmed_on_success() {
        // 成功译文统一 trim（后端可能带前后空白）。
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: Some(Arc::new(StubTranslate::ok("  Hello \n"))),
            dedicated: None,
            llm_fallback: None,
            use_llm_fallback: false,
        };
        assert_eq!(r.translate(&req()).await.unwrap(), "Hello");
    }

    #[tokio::test]
    async fn use_llm_fallback_false_skips_fallback() {
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: Some(Arc::new(StubTranslate::err())),
            dedicated: None,
            llm_fallback: Some(Arc::new(StubTranslate::ok("兼译"))),
            use_llm_fallback: false,
        };
        assert!(r.translate(&req()).await.is_err());
    }

    #[tokio::test]
    async fn all_fail_returns_err() {
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferLocal,
            cloud: Some(Arc::new(StubTranslate::err())),
            dedicated: Some(Arc::new(StubTranslate::err())),
            llm_fallback: Some(Arc::new(StubTranslate::err())),
            use_llm_fallback: true,
        };
        assert!(r.translate(&req()).await.is_err());
    }

    #[tokio::test]
    async fn empty_router_reports_empty() {
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: None,
            dedicated: None,
            llm_fallback: None,
            use_llm_fallback: true,
        };
        assert!(r.is_empty());
        assert!(!r.first_hop_is_cloud());
    }

    #[tokio::test]
    async fn polish_and_translate_only_via_cloud() {
        let r = TranslateRouter {
            policy: TranslatePolicy::PreferCloud,
            cloud: None,
            dedicated: Some(Arc::new(StubTranslate::ok("d"))),
            llm_fallback: None,
            use_llm_fallback: false,
        };
        assert!(r.polish_and_translate(&req()).await.is_err());
    }
}
