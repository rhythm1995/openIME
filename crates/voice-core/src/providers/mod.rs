//! ASR provider 实现集合。
//!
//! - [`bailian`]：阿里云百炼 Protocol A（流式 WebSocket）
//! - [`sherpa`]：本地 sherpa-onnx（feature 门控，M4）
//! - [`RoutingProvider`]：按 ProviderConfig.kind 在上述两者间路由

pub mod bailian;
pub mod multimodal_asr;
pub mod openai_asr;
pub mod sherpa;

use async_trait::async_trait;

use crate::config::ProviderKind;
use crate::traits::{AsrProvider, AsrSession};
use crate::{Error, ProviderConfig};

/// 按 cfg.kind 路由到 bailian 或 sherpa 的复合 provider。
/// sherpa_root 仅在选用 sherpa 时使用（None 时选 sherpa 会报引导错误）。
pub struct RoutingProvider {
    pub sherpa_root: Option<(std::path::PathBuf, std::path::PathBuf)>,
}

#[async_trait]
impl AsrProvider for RoutingProvider {
    async fn connect(&self, cfg: &ProviderConfig) -> crate::Result<Box<dyn AsrSession>> {
        match cfg.kind {
            ProviderKind::Bailian => bailian::BailianProvider.connect(cfg).await,
            ProviderKind::OpenAiAsr => openai_asr::OpenAiAsrProvider.connect(cfg).await,
            ProviderKind::MultimodalAsr => multimodal_asr::MultimodalAsrProvider.connect(cfg).await,
            ProviderKind::Sherpa => match &self.sherpa_root {
                Some((model_root, vad_root)) => {
                    let provider =
                        sherpa::SherpaProvider::with_root(model_root.clone(), vad_root.clone());
                    provider.connect(cfg).await
                }
                None => Err(Error::Provider(
                    "未配置本地模型路径，请在设置中下载模型".into(),
                )),
            },
        }
    }
}
