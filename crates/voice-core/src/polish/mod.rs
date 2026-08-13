//! 二期文本润色：本地 GGUF / 云端 chat / 路由，与 ASR 双引擎对称。
//! 新增：L0 规则层 `correction`（零延迟本地纠错，不依赖 LLM）。
//! P1 新增：`llm`（LlmClient 翻译/流式）、`roles`（R5 前缀角色检测）。

mod cloud;
mod correction;
mod itn;
mod llm;
mod local;
mod prompts;
mod punct;
mod roles;
mod router;
mod sanitize;
mod script;

pub use cloud::{parse_polish_translate, BailianChatPolish, CloudPolishProvider};
pub use correction::{correct_l0, L0Result};
pub use itn::normalize_itn;
pub use llm::{parse_sse_line, ChatRequest, LlmClient, PolishTranslate, SseLine, TranslateRequest};
pub use local::LocalGgufPolish;
pub use prompts::{
    build_messages, build_polish_translate_messages, build_qa_system, build_translate_messages,
    lang_display_name, truncate_selection, wrap_selected_text, POLISHED_SOURCE_SENTINEL,
    TRANSLATION_SENTINEL,
};
pub use punct::full_to_half_punct;
pub use roles::detect_prefix_role;
pub use router::{PolishRouter, PolishRouterConfig};
pub use sanitize::{dedupe_consecutive_finals, sanitize_polish_output};
pub use script::convert_script;
