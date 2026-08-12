//! 润色路由：PreferLocal / PreferCloud / LocalOnly / CloudOnly / Off。

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::config::PolishPolicy;
use crate::traits::{PolishMode, PolishRequest, PolishResponse, TextPolishProvider};
use crate::Result;

/// 路由配置。
#[derive(Debug, Clone)]
pub struct PolishRouterConfig {
    pub policy: PolishPolicy,
    /// 总开关；false 时一律 passthrough。
    pub enabled: bool,
}

/// 复合润色：本地 + 云端，策略镜像 ASR。
pub struct PolishRouter {
    pub cfg: PolishRouterConfig,
    pub local: Option<Arc<dyn TextPolishProvider>>,
    pub cloud: Option<Arc<dyn TextPolishProvider>>,
}

impl PolishRouter {
    pub fn passthrough_only() -> Self {
        Self {
            cfg: PolishRouterConfig {
                policy: PolishPolicy::Off,
                enabled: false,
            },
            local: None,
            cloud: None,
        }
    }
}

#[async_trait]
impl TextPolishProvider for PolishRouter {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse> {
        if !self.cfg.enabled || req.mode == PolishMode::Off || self.cfg.policy == PolishPolicy::Off
        {
            return Ok(PolishResponse {
                text: req.text,
                provider: "passthrough".into(),
                latency_ms: 0,
            });
        }

        let t0 = Instant::now();
        let try_local = async {
            if let Some(p) = &self.local {
                p.polish(req.clone()).await
            } else {
                Err(crate::Error::Provider("本地润色未配置".into()))
            }
        };
        let try_cloud = async {
            if let Some(p) = &self.cloud {
                p.polish(req.clone()).await
            } else {
                Err(crate::Error::Provider("云端润色未配置".into()))
            }
        };

        let result = match self.cfg.policy {
            PolishPolicy::Off => Ok(PolishResponse {
                text: req.text.clone(),
                provider: "passthrough".into(),
                latency_ms: 0,
            }),
            PolishPolicy::LocalOnly => try_local.await,
            PolishPolicy::CloudOnly => try_cloud.await,
            PolishPolicy::PreferLocal => match try_local.await {
                Ok(r) => Ok(r),
                Err(e_local) => match try_cloud.await {
                    Ok(r) => Ok(r),
                    Err(_) => {
                        // 双失败：原文直出，不阻断上屏。
                        tracing::warn!("润色失败，回退原文：{e_local}");
                        Ok(PolishResponse {
                            text: req.text.clone(),
                            provider: "passthrough-fallback".into(),
                            latency_ms: t0.elapsed().as_millis() as u32,
                        })
                    }
                },
            },
            PolishPolicy::PreferCloud => match try_cloud.await {
                Ok(r) => Ok(r),
                Err(e_cloud) => match try_local.await {
                    Ok(r) => Ok(r),
                    Err(_) => {
                        tracing::warn!("润色失败，回退原文：{e_cloud}");
                        Ok(PolishResponse {
                            text: req.text.clone(),
                            provider: "passthrough-fallback".into(),
                            latency_ms: t0.elapsed().as_millis() as u32,
                        })
                    }
                },
            },
        };

        result
    }
}

/// 测试用：固定把文本加上标记。
#[cfg(test)]
pub struct MarkPolish;

#[cfg(test)]
#[async_trait]
impl TextPolishProvider for MarkPolish {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse> {
        Ok(PolishResponse {
            text: format!("【润】{}", req.text),
            provider: "mark".into(),
            latency_ms: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn req(text: &str) -> PolishRequest {
        PolishRequest {
            text: text.into(),
            mode: PolishMode::Light,
            style_prompt: None,
            hotwords: vec![],
            timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn disabled_passthrough() {
        let r = PolishRouter {
            cfg: PolishRouterConfig {
                policy: PolishPolicy::PreferLocal,
                enabled: false,
            },
            local: Some(Arc::new(MarkPolish)),
            cloud: None,
        };
        let out = r.polish(req("你好")).await.unwrap();
        assert_eq!(out.text, "你好");
        assert_eq!(out.provider, "passthrough");
    }

    #[tokio::test]
    async fn prefer_local_uses_local() {
        let r = PolishRouter {
            cfg: PolishRouterConfig {
                policy: PolishPolicy::PreferLocal,
                enabled: true,
            },
            local: Some(Arc::new(MarkPolish)),
            cloud: None,
        };
        let out = r.polish(req("你好")).await.unwrap();
        assert_eq!(out.text, "【润】你好");
    }

    #[tokio::test]
    async fn prefer_local_falls_back_to_original_when_no_backend() {
        let r = PolishRouter {
            cfg: PolishRouterConfig {
                policy: PolishPolicy::PreferLocal,
                enabled: true,
            },
            local: None,
            cloud: None,
        };
        let out = r.polish(req("原文")).await.unwrap();
        assert_eq!(out.text, "原文");
        assert!(out.provider.contains("passthrough"));
    }

    /// 静默降级：本地失败 → 云端也失败 → 原文直出、不返回 Err。
    /// （用户要求：润色失败就原样输出，不要报错。）
    struct FailPolish;
    #[async_trait]
    impl TextPolishProvider for FailPolish {
        async fn polish(&self, req: PolishRequest) -> Result<PolishResponse> {
            let _ = req;
            Err(crate::Error::Provider("mock fail".into()))
        }
    }

    #[tokio::test]
    async fn prefer_local_both_fail_silently_output_original() {
        let r = PolishRouter {
            cfg: PolishRouterConfig {
                policy: PolishPolicy::PreferLocal,
                enabled: true,
            },
            local: Some(Arc::new(FailPolish)),
            cloud: Some(Arc::new(FailPolish)),
        };
        // 关键：返回 Ok（不是 Err），且文本是原文 —— 不阻断上屏、不报错。
        let out = r.polish(req("原样输出")).await.unwrap();
        assert_eq!(out.text, "原样输出");
        assert!(out.provider.contains("passthrough"));
    }
}
