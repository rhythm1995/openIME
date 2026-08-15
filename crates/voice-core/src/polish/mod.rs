//! 二期文本润色：本地 GGUF / 云端 chat / 路由，与 ASR 双引擎对称。
//! 新增：L0 规则层 `correction`（零延迟本地纠错，不依赖 LLM）。
//! P1 新增：`llm`（LlmClient 翻译/流式）、`roles`（R5 前缀角色检测）。
//! 本地三件套新增：`runtime`（常驻 GGUF 运行时）、`translate_router`（翻译路由）。

mod cloud;
mod correction;
mod itn;
mod llm;
mod local;
mod prompts;
mod punct;
mod roles;
mod router;
mod runtime;
mod sanitize;
mod script;
mod translate_router;

pub use cloud::{parse_polish_translate, BailianChatPolish, CloudPolishProvider};
pub use correction::{correct_l0, L0Result};
pub use itn::normalize_itn;
pub use llm::{parse_sse_line, ChatRequest, LlmClient, PolishTranslate, SseLine, TranslateRequest};
pub use local::{arch_needs_no_think, LocalGgufPolish, LocalGgufTranslate};
pub use prompts::{
    build_local_translate_messages, build_messages, build_polish_translate_messages,
    build_qa_system, build_translate_messages, detect_source_lang, lang_display_name,
    lang_english_name, looks_like_instruction_leak, truncate_selection, wrap_selected_text,
    POLISHED_SOURCE_SENTINEL, TRANSLATION_SENTINEL,
};
pub use punct::full_to_half_punct;
pub use roles::{assistant_combo_words, detect_prefix_role, starts_with_assistant};
pub use router::{PolishRouter, PolishRouterConfig};
pub use runtime::{strip_think, CompletionRequest, GgufRuntime};
pub use sanitize::{dedupe_consecutive_finals, sanitize_polish_output};
pub use script::convert_script;
pub use translate_router::TranslateRouter;
