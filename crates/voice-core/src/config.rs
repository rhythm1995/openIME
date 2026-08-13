//! 应用与 provider 配置。

use serde::{Deserialize, Serialize};

use crate::traits::PolishMode;
use crate::Error;

/// 一期支持的 provider 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// 本地 sherpa-onnx（离线）。
    Sherpa,
    /// 阿里云百炼 Protocol A（流式 WebSocket）。
    Bailian,
    /// REST POST /audio/transcriptions（OpenAI Whisper / OpenRouter 兼容）。
    #[serde(rename = "openai_asr")]
    OpenAiAsr,
    /// REST POST multimodal-generation/chat（百炼 Qwen3 ASR 非流式 / OpenAI Chat audio）。
    #[serde(rename = "multimodal_asr")]
    MultimodalAsr,
}

/// 单个 provider 的连接配置。
///
/// - Sherpa：`model` 是模型目录名（如 `sherpa-onnx-streaming-paraformer-bilingual-zh-en`），
///   `base_url` 为空，`api_key` 为空，模型路径由 model_mgr 管理。
/// - Bailian：`base_url` 形如
///   `wss://{workspace_id}.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference`，
///   `api_key` 是 `sk-...`，`model` 如 `fun-asr-realtime` / `paraformer-realtime-v2`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 百炼热词表 ID（可选；二期 UI 再暴露）。
    #[serde(default)]
    pub vocabulary_id: Option<String>,
    /// 本地 ASR 语言提示（仅 sherpa 用）：`zh` / `en` / `yue` / `auto`。
    #[serde(default)]
    pub language: Option<String>,
}

impl ProviderConfig {
    /// 校验配置完备。Sherpa 只需要 model；Bailian 需要 base_url + api_key + model。
    pub fn validate(&self) -> crate::Result<()> {
        match self.kind {
            ProviderKind::Sherpa => {
                if self.model.trim().is_empty() {
                    return Err(Error::Config("sherpa provider 缺少 model".into()));
                }
            }
            ProviderKind::Bailian => {
                let url = self.base_url.trim();
                if url.is_empty() {
                    return Err(Error::Config("bailian provider 缺少 base_url".into()));
                }
                let valid_scheme = url.starts_with("wss://")
                    || url.starts_with("ws://")
                    || url.starts_with("https://")
                    || url.starts_with("http://");
                if !valid_scheme {
                    return Err(Error::Config(
                        "base_url 必须以 ws://, wss://, http:// 或 https:// 开头".into(),
                    ));
                }
                // R3：百炼只校验归一化后的 wss URL（用户常贴 http://…/compatible-mode/v1）。
                let normalized = crate::providers::bailian::normalize_ws_url(url);
                crate::endpoint::validate_endpoint(&normalized)
                    .map_err(|e| Error::Config(format!("base_url 校验失败：{e}")))?;
                if self.api_key.trim().is_empty() {
                    return Err(Error::Config("bailian provider 缺少 api_key".into()));
                }
                if self.model.trim().is_empty() {
                    return Err(Error::Config("bailian provider 缺少 model".into()));
                }
            }
            ProviderKind::OpenAiAsr | ProviderKind::MultimodalAsr => {
                let url = self.base_url.trim();
                if url.is_empty() {
                    return Err(Error::Config(
                        "云端 ASR provider 缺少 base_url（endpoint）".into(),
                    ));
                }
                // R3：REST 校验用户填写的原始 URL（含 scheme + host/IP 分类）。
                crate::endpoint::validate_endpoint(url)
                    .map_err(|e| Error::Config(format!("base_url 校验失败：{e}")))?;
                if self.api_key.trim().is_empty() {
                    return Err(Error::Config("云端 ASR provider 缺少 api_key".into()));
                }
                if self.model.trim().is_empty() {
                    return Err(Error::Config("云端 ASR provider 缺少 model".into()));
                }
            }
        }
        Ok(())
    }
}

/// 繁简偏好（B6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChineseScriptPreference {
    #[default]
    Auto,
    Simplified,
    Traditional,
}

/// 快捷键模式（A1）：Toggle 按一下开/再按一下停；Hold 按住说话、松开停。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyMode {
    #[default]
    Toggle,
    Hold,
}

/// 云端润色 LLM 协议类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolishCloudProtocol {
    /// OpenAI Chat Completions（/chat/completions）。
    #[default]
    OpenAiChat,
    /// Anthropic Messages API（/v1/messages）。
    Anthropic,
    /// OpenAI Responses API（/v1/responses）。
    OpenAiResponses,
}

/// R7：文本插入策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InsertStrategy {
    /// 先 enigo 逐字输入，失败自动回退粘贴（Type-then-Paste）。
    #[default]
    Auto,
    /// 只 enigo 逐字输入，失败即 Failed。
    #[serde(rename = "type")]
    Type,
    /// 只粘贴（剪贴板 + 平台和弦）。
    Paste,
}

/// 润色路由策略。已简化为两态：本地优先（默认）/ 关闭。
/// 旧配置中的 prefer_cloud / local_only / cloud_only 经 serde alias 归一为 PreferLocal。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolishPolicy {
    /// 本地 GGUF 优先，失败/未装自动回退云端。
    #[default]
    #[serde(alias = "prefer_cloud", alias = "local_only", alias = "cloud_only")]
    PreferLocal,
    /// 强制关闭润色（等同 polish_mode=Off）。
    Off,
}

/// 应用级配置。持久化到 settings 表 / 配置文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// 当前激活的 provider 索引（0 起）。
    pub active_provider: usize,
    /// 用户已配置的 provider 列表（至少 1 个）。
    pub providers: Vec<ProviderConfig>,
    /// 全局录音快捷键。支持 "Fn"（macOS Globe 键，原生监听）
    /// 或 Tauri/Accelerator 风格组合键如 "Alt+Shift+D"。
    pub hotkey: String,
    /// 快捷键模式（A1）：Toggle（默认）/ Hold（按住说话）。
    #[serde(default)]
    pub hotkey_mode: HotkeyMode,
    /// 风格包循环切换快捷键（F1，可选，如 Ctrl+Shift+P；None=不启用）。
    #[serde(default)]
    pub style_switch_hotkey: Option<String>,
    /// 录音时是否静音其他应用音频（一期可固定 false）。
    pub mute_other_audio: bool,
    /// 开机自启（macOS Login Items）。开机自启时应用静默常驻菜单栏，不弹面板。
    #[serde(default)]
    pub launch_at_login: bool,
    /// 本地引擎模式（兼容旧配置）："offline" / "realtime"。
    /// 新逻辑以 `local_asr_model` 为准；保存时会与之同步。
    #[serde(default = "default_local_mode")]
    pub local_mode: String,
    /// 当前启用的本地 ASR 模型 id：`zipformer-zh-2025` | `sensevoice`。
    #[serde(default = "default_local_asr_model")]
    pub local_asr_model: String,
    /// 麦克风设备名（None/空 = 系统默认输入设备）。
    #[serde(default)]
    pub audio_device: Option<String>,

    // ── 二期：AI 润色 ──
    /// 总开关。默认 false：渐进开启，避免首启强制下 1GB 模型。
    #[serde(default)]
    pub polish_enabled: bool,
    /// 本地/云端路由策略。默认 PreferLocal。
    #[serde(default)]
    pub polish_policy: PolishPolicy,
    /// 本地 GGUF 模型 id（目录/文件约定，见 model_download）。
    #[serde(default = "default_polish_local_model")]
    pub polish_local_model: String,
    /// 云端 chat 模型名（百炼 OpenAI 兼容），如 qwen-turbo。
    #[serde(default = "default_polish_cloud_model")]
    pub polish_cloud_model: String,
    /// 云端润色 LLM 协议（openai_chat / anthropic / openai_responses）。
    #[serde(default)]
    pub polish_cloud_protocol: PolishCloudProtocol,
    /// 云端润色 LLM endpoint（base URL，如 https://dashscope.aliyuncs.com/compatible-mode/v1）。
    #[serde(default)]
    pub polish_cloud_endpoint: String,
    /// 云端润色 LLM API Key（覆盖 provider api_key，若为空则用 bailian provider 的 key）。
    #[serde(default)]
    pub polish_cloud_api_key: String,
    /// 润色程度：Off（保持原样）/ Light（中度，仅校对）/ Heavy（高度，改写润色）。
    #[serde(default)]
    pub polish_mode: PolishMode,
    /// 当前选中的风格包 id（F1，仅 Heavy 模式生效；None = 用默认 Heavy prompt）。
    #[serde(default)]
    pub active_style_pack_id: Option<String>,
    /// 单次润色超时（毫秒）。
    #[serde(default = "default_polish_timeout_ms")]
    pub polish_timeout_ms: u32,
    /// 半角标点偏好 app 关键字（B5）：前台 app bundle id 含其中任一关键字时，
    /// 上屏文本的全角标点转半角（适合 IM）。空 = 不转换。
    #[serde(default)]
    pub punct_half_width_apps: Vec<String>,
    /// 繁简偏好（B6）：Auto 不转，Simplified/Traditional 强制简/繁。
    #[serde(default)]
    pub chinese_script_preference: ChineseScriptPreference,

    /// 默认识别语言：`zh` / `en` / `yue` / `auto`。默认 `zh`。
    #[serde(default = "default_local_language")]
    pub local_language: String,

    // ── P1：R4 翻译 ──
    /// 翻译快捷键（None = 不注册；P1 仅 Toggle）。
    #[serde(default)]
    pub translate_hotkey: Option<String>,
    /// 翻译目标语言（BCP-47 短码，固定下拉闭集）。默认 "en"。
    #[serde(default = "default_translate_target_lang")]
    pub translate_target_lang: String,
    /// 「先润色再翻译」一次调用（哨兵合成）。
    #[serde(default)]
    pub translate_with_polish: bool,

    // ── P1：R5 前缀角色 ──
    /// 识别结果前缀分流到角色（风格包）。开启时听写强制整段插入（关流式上屏）。
    #[serde(default = "default_true")]
    pub prefix_roles_enabled: bool,

    // ── P1：R6 划词问答 ──
    /// QA 快捷键（None = 不注册；P1 仅 Toggle）。
    #[serde(default)]
    pub qa_hotkey: Option<String>,
    /// 是否把 QA 问答写入历史（sessions/utterances）。
    #[serde(default)]
    pub qa_save_history: bool,

    // ── P1：R7 粘贴兜底 ──
    /// 插入策略：auto / type / paste。
    #[serde(default)]
    pub insert_strategy: InsertStrategy,
    /// 前台 app 标识命中任一条时视同 Paste（应对「Ok 但吞键」）。
    #[serde(default)]
    pub paste_fallback_apps: Vec<String>,
    /// 粘贴后 750ms 恢复原剪贴板（内容仍相等才写回）。
    #[serde(default = "default_true")]
    pub restore_clipboard: bool,
}

fn default_local_mode() -> String {
    "offline".to_string()
}
fn default_local_asr_model() -> String {
    crate::asr_catalog::default_asr_model_id().to_string()
}
fn default_polish_local_model() -> String {
    POLISH_DEFAULT_LOCAL_MODEL.to_string()
}
fn default_polish_cloud_model() -> String {
    "qwen-turbo".to_string()
}
fn default_polish_timeout_ms() -> u32 {
    800
}
fn default_local_language() -> String {
    "zh".into()
}
fn default_translate_target_lang() -> String {
    "en".into()
}
fn default_true() -> bool {
    true
}

/// 默认本地润色模型（Qwen2.5-1.5B-Instruct GGUF Q4_K_M）。
pub const POLISH_DEFAULT_LOCAL_MODEL: &str = "qwen2.5-1.5b-instruct-q4_k_m";

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            active_provider: 0,
            providers: vec![ProviderConfig {
                kind: ProviderKind::Sherpa,
                base_url: String::new(),
                api_key: String::new(),
                model: SHERPA_DEFAULT_MODEL.to_string(),
                vocabulary_id: None,
                language: None,
            }],
            hotkey: "Fn".to_string(),
            hotkey_mode: HotkeyMode::Toggle,
            style_switch_hotkey: None,
            mute_other_audio: false,
            launch_at_login: false,
            // local_mode 与默认 ASR（Zipformer 流式）对齐；新逻辑以 local_asr_model 为准。
            local_mode: "realtime".to_string(),
            local_asr_model: crate::asr_catalog::default_asr_model_id().to_string(),
            audio_device: None,
            local_language: default_local_language(),
            polish_enabled: false,
            polish_policy: PolishPolicy::PreferLocal,
            polish_local_model: POLISH_DEFAULT_LOCAL_MODEL.to_string(),
            polish_cloud_model: "qwen-turbo".to_string(),
            polish_cloud_protocol: PolishCloudProtocol::OpenAiChat,
            polish_cloud_endpoint: String::new(),
            polish_cloud_api_key: String::new(),
            polish_mode: PolishMode::Off,
            active_style_pack_id: None,
            polish_timeout_ms: 800,
            punct_half_width_apps: Vec::new(),
            chinese_script_preference: ChineseScriptPreference::Auto,
            translate_hotkey: None,
            translate_target_lang: default_translate_target_lang(),
            translate_with_polish: false,
            prefix_roles_enabled: true,
            qa_hotkey: None,
            qa_save_history: false,
            insert_strategy: InsertStrategy::Auto,
            paste_fallback_apps: Vec::new(),
            restore_clipboard: true,
        }
    }
}

/// sherpa 默认：与 default_asr_model_id 对齐。
const SHERPA_DEFAULT_MODEL: &str = "sensevoice";

impl AppConfig {
    /// 取当前激活的 provider。
    pub fn active(&self) -> crate::Result<&ProviderConfig> {
        self.providers.get(self.active_provider).ok_or_else(|| {
            Error::Config(format!(
                "active_provider 索引越界: {}",
                self.active_provider
            ))
        })
    }

    /// 当前启用的本地 ASR 模型 id（规范化）。
    pub fn resolved_local_asr_model(&self) -> String {
        let raw = if self.local_asr_model.trim().is_empty() {
            // 兼容仅写了 local_mode 的旧配置
            self.local_mode.as_str()
        } else {
            self.local_asr_model.as_str()
        };
        crate::model_download::normalize_asr_model_id(raw).to_string()
    }

    /// 规范化 local_asr_model，并同步 local_mode / sherpa provider.model。
    pub fn sync_local_asr_fields(&mut self) {
        let id = self.resolved_local_asr_model();
        self.local_asr_model = id.clone();
        // 兼容旧字段：离线整段 ≈ offline，流式 ≈ realtime。
        self.local_mode = match id.as_str() {
            crate::asr_catalog::ASR_MODEL_SENSEVOICE
            | crate::asr_catalog::ASR_MODEL_FIRERED_LARGE
            | crate::asr_catalog::ASR_MODEL_FUNASR_NANO_INT8
            | crate::asr_catalog::ASR_MODEL_FUNASR_NANO_FP16 => "offline".to_string(),
            _ => "realtime".to_string(),
        };
        // 同步语言到 sherpa provider（empty → zh）
        let lang = if self.local_language.trim().is_empty() {
            "zh".into()
        } else {
            self.local_language.trim().to_lowercase()
        };
        self.local_language = lang.clone();
        if let Some(p) = self.providers.get_mut(self.active_provider) {
            if p.kind == ProviderKind::Sherpa {
                p.model = id;
                p.language = Some(lang);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_policy_variants_map_to_prefer_local() {
        // 旧配置里的 prefer_cloud / local_only / cloud_only 归一为 PreferLocal。
        let p: PolishPolicy = serde_json::from_str("\"prefer_cloud\"").unwrap();
        assert_eq!(p, PolishPolicy::PreferLocal);
        let p: PolishPolicy = serde_json::from_str("\"local_only\"").unwrap();
        assert_eq!(p, PolishPolicy::PreferLocal);
        let p: PolishPolicy = serde_json::from_str("\"cloud_only\"").unwrap();
        assert_eq!(p, PolishPolicy::PreferLocal);
    }

    #[test]
    fn provider_kind_serde_roundtrip_snake() {
        // 前端用 snake_case（openai_asr / multimodal_asr），必须能反序列化。
        let k: ProviderKind = serde_json::from_str("\"openai_asr\"").unwrap();
        assert_eq!(k, ProviderKind::OpenAiAsr);
        let m: ProviderKind = serde_json::from_str("\"multimodal_asr\"").unwrap();
        assert_eq!(m, ProviderKind::MultimodalAsr);
        // 序列化也一致（roundtrip）。
        assert_eq!(serde_json::to_string(&k).unwrap(), "\"openai_asr\"");
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"multimodal_asr\"");
    }

    #[test]
    fn sherpa_only_needs_model() {
        let c = ProviderConfig {
            kind: ProviderKind::Sherpa,
            base_url: String::new(),
            api_key: String::new(),
            model: "paraformer-online".into(),
            vocabulary_id: None,
            language: None,
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn bailian_requires_full_config() {
        let c = ProviderConfig {
            kind: ProviderKind::Bailian,
            base_url: "wss://wsid.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference".into(),
            api_key: "sk-xxx".into(),
            model: "fun-asr-realtime".into(),
            vocabulary_id: None,
            language: None,
        };
        assert!(c.validate().is_ok());

        let bad = ProviderConfig {
            kind: ProviderKind::Bailian,
            base_url: "ftp://invalid.example.com".into(),
            api_key: "sk-xxx".into(),
            model: "fun-asr-realtime".into(),
            vocabulary_id: None,
            language: None,
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn app_config_default_has_sherpa() {
        let c = AppConfig::default();
        assert_eq!(c.active_provider, 0);
        assert_eq!(c.providers.len(), 1);
        assert_eq!(c.active().unwrap().kind, ProviderKind::Sherpa);
        assert!(!c.launch_at_login);
        assert_eq!(
            c.resolved_local_asr_model(),
            crate::asr_catalog::ASR_MODEL_SENSEVOICE
        );
    }

    #[test]
    fn legacy_config_without_launch_at_login_deserializes() {
        // 旧版本持久化的配置没有 launch_at_login 字段，反序列化时应默认 false 而非报错。
        let json = r#"{
            "active_provider": 0,
            "providers": [],
            "hotkey": "Alt+Shift+D",
            "mute_other_audio": false
        }"#;
        let c: AppConfig = serde_json::from_str(json).unwrap();
        assert!(!c.launch_at_login);
    }

    #[test]
    fn insert_strategy_serde_snake_case() {
        // R7：serde 用 auto / type / paste（type 是 Rust 保留字，只能靠 rename）。
        let s: InsertStrategy = serde_json::from_str("\"auto\"").unwrap();
        assert_eq!(s, InsertStrategy::Auto);
        let s: InsertStrategy = serde_json::from_str("\"type\"").unwrap();
        assert_eq!(s, InsertStrategy::Type);
        let s: InsertStrategy = serde_json::from_str("\"paste\"").unwrap();
        assert_eq!(s, InsertStrategy::Paste);
        assert_eq!(serde_json::to_string(&InsertStrategy::Auto).unwrap(), "\"auto\"");
        assert_eq!(serde_json::to_string(&InsertStrategy::Type).unwrap(), "\"type\"");
        assert_eq!(serde_json::to_string(&InsertStrategy::Paste).unwrap(), "\"paste\"");
    }

    #[test]
    fn p1_fields_have_defaults() {
        // P1 新字段全部 #[serde(default)]：旧 JSON 可反序列化且默认值符合设计。
        let c = AppConfig::default();
        assert_eq!(c.translate_hotkey, None);
        assert_eq!(c.translate_target_lang, "en");
        assert!(!c.translate_with_polish);
        assert!(c.prefix_roles_enabled);
        assert_eq!(c.qa_hotkey, None);
        assert!(!c.qa_save_history);
        assert_eq!(c.insert_strategy, InsertStrategy::Auto);
        assert!(c.paste_fallback_apps.is_empty());
        assert!(c.restore_clipboard);
    }
}
