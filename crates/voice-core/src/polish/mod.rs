//! 二期文本润色：本地 GGUF / 云端 chat / 路由，与 ASR 双引擎对称。
//! 新增：L0 规则层 `correction`（零延迟本地纠错，不依赖 LLM）。

mod cloud;
mod correction;
mod local;
mod prompts;
mod router;
mod sanitize;

pub use cloud::BailianChatPolish;
pub use correction::{correct_l0, L0Result};
pub use local::LocalGgufPolish;
pub use prompts::build_messages;
pub use router::{PolishRouter, PolishRouterConfig};
pub use sanitize::{dedupe_consecutive_finals, sanitize_polish_output};
