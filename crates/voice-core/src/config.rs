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
/// 序列化名与前端 TS 联合类型对齐：`openai_chat` / `anthropic` / `openai_responses`
/// （`snake_case` 会把 `OpenAiChat` 变成 `open_ai_chat`，故显式 rename；
/// alias 兼容早期 snake_case 落库的旧值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolishCloudProtocol {
    /// OpenAI Chat Completions（/chat/completions）。
    #[default]
    #[serde(rename = "openai_chat", alias = "open_ai_chat")]
    OpenAiChat,
    /// Anthropic Messages API（/v1/messages）。
    Anthropic,
    /// OpenAI Responses API（/v1/responses）。
    #[serde(rename = "openai_responses", alias = "open_ai_responses")]
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

/// 翻译路由策略（本地三件套方案）：与润色相反，默认云端优先。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranslatePolicy {
    /// 云 → 专翻 → 兼译 → 原文（有网默认走云）。
    #[default]
    #[serde(rename = "prefer_cloud")]
    PreferCloud,
    /// 专翻 → 兼译 → 云 → 原文（隐私/离线优先）。
    #[serde(rename = "prefer_local")]
    PreferLocal,
}

/// 应用级配置。持久化到 settings 表 / 配置文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// 当前激活的 provider 索引（0 起）。
    pub active_provider: usize,
    /// 用户已配置的 provider 列表（至少 1 个）。
    pub providers: Vec<ProviderConfig>,
    /// 全局录音快捷键。支持 "Fn"（macOS Globe 键，原生监听）、
    /// "CapsLock"（Windows 单键，WH_KEYBOARD_LL 钩子监听，默认）
    /// 或 Tauri/Accelerator 风格组合键如 "Alt+Shift+D"。
    pub hotkey: String,
    /// 快捷键模式（A1）：Hold（按住说话，默认）/ Toggle（切换）。
    /// 全平台默认 Hold：短按误触恢复（R9 补发）在 macOS（flagsChanged 补发）
    /// 与 Windows（CapsLock 补发）均已支持，Hold 是更符合直觉的听写手势。
    #[serde(default = "default_hotkey_mode")]
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
    /// 云端 LLM 模型 ID（可选）。留空时请求不携带 model 字段，由服务端/网关决定。
    #[serde(default)]
    pub polish_cloud_model: String,
    /// 云端润色 LLM 协议（openai_chat / anthropic / openai_responses）。
    #[serde(default)]
    pub polish_cloud_protocol: PolishCloudProtocol,
    /// 云端润色 LLM endpoint（base URL，必填，如 https://api.openai.com/v1）。
    #[serde(default)]
    pub polish_cloud_endpoint: String,
    /// 云端润色 LLM API Key（必填，独立于识别引擎凭据）。
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
    /// 「先润色再翻译」：云端仍哨兵合成；本地 = Light 源语纠错再译（两步，禁哨兵）。
    #[serde(default)]
    pub translate_with_polish: bool,
    // ── 本地三件套：本地翻译 ──
    /// 本地专翻模型 id：`milmmt-1b` | `hy-mt-1.8b` | ""（未选）。
    #[serde(default = "default_translate_local_model")]
    pub translate_local_model: String,
    /// 弱机兼译：专翻装不下/未装时用润色模型兼做翻译（同一颗 Qwen3.5，两步）。
    #[serde(default)]
    pub translate_use_llm_fallback: bool,
    /// 翻译路由策略：PreferCloud（默认）/ PreferLocal。
    #[serde(default)]
    pub translate_policy: TranslatePolicy,

    // ── P1：R5 前缀角色 ──
    /// 识别结果前缀分流到角色（风格包）。开启时听写强制整段插入（关流式上屏）。
    #[serde(default = "default_true")]
    pub prefix_roles_enabled: bool,
    /// R5：助手名称——「助手名+角色别名」组合触发前缀角色（如「小友翻译…」「小友邮件…」）。
    /// 组合词写入热词精准纠错。空串 = 前缀角色不触发。
    #[serde(default = "default_assistant_name")]
    pub assistant_name: String,
    /// i18n：App 界面语言（"zh" / "en"）。前端语言切换时通过 `set_ui_lang` 同步到后端，
    /// 供 Rust 端按界面语言选择角色名 / 别名 / system prompt（语音触发与提示语言一致）。
    #[serde(default = "default_ui_lang")]
    pub ui_lang: String,

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

    // ── P2：R9 短按补发 ──
    /// Fn 短按阈值（ms）：Hold+Fn 按住超过该时长才开录；提前松开只补发 🌐。
    #[serde(default = "default_short_press_ms")]
    pub short_press_ms: u32,
    /// Hold+Fn 短按补发 🌐（Fn/Globe）原按键。默认开。
    #[serde(default = "default_true")]
    pub fn_repost_enabled: bool,
    /// HID 补发后若前台输入源未变，TIS 切下一输入源（默认关）。
    #[serde(default)]
    pub fn_repost_tis_fallback: bool,

    // ── P2：R11 Windows TSF ──
    /// Windows 优先用 TSF CommitText 上屏。FFI（TIP DLL + 命名管道 client）已落地，
    /// 但 Win11 实测枚举/激活只认 HKLM（管理员）注册的 TIP：per-user 安装下探测为
    /// RegistrationBroken → insert_ex 零成本回退 R7。故默认 false；
    /// 以管理员运行一次 `regsvr32 OpenImeTsf.dll` 后可在设置页开启。
    #[serde(default = "default_false")]
    pub windows_tsf_enabled: bool,
    /// TSF 提交失败时回退 P1 R7 粘贴。
    #[serde(default = "default_true")]
    pub windows_tsf_fallback: bool,

    // ── P2：R12 长音频分段 ──
    /// 文件转录切片时长（秒）。
    #[serde(default = "default_file_seg_duration_secs")]
    pub file_seg_duration_secs: u32,
    /// 相邻切片重叠时长（秒），须 < duration 且 >= 1。
    #[serde(default = "default_file_seg_overlap_secs")]
    pub file_seg_overlap_secs: u32,
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
fn default_polish_timeout_ms() -> u32 {
    800
}
fn default_local_language() -> String {
    "zh".into()
}
fn default_translate_target_lang() -> String {
    "en".into()
}
fn default_translate_local_model() -> String {
    "milmmt-1b".into()
}
fn default_assistant_name() -> String {
    "小友".into()
}
fn default_ui_lang() -> String {
    "zh".into()
}
fn default_false() -> bool {
    false
}
fn default_true() -> bool {
    true
}
fn default_short_press_ms() -> u32 {
    300
}
/// 默认录音快捷键（平台配对）：
/// - macOS：`Fn`（Globe 键，原生 NSEvent 监听）。
/// - Windows：`CapsLock`（单键按住说话，WH_KEYBOARD_LL 钩子；短按补发保留原功能）。
///   Fn 在绝大多数 Windows 键盘由固件消费、系统不可见，不再作为默认。
/// - 其它平台：可注册组合键 `Ctrl+Shift+D`（无单键监听实现）。
fn default_hotkey() -> String {
    if cfg!(target_os = "macos") {
        "Fn".to_string()
    } else if cfg!(target_os = "windows") {
        "CapsLock".to_string()
    } else {
        "Ctrl+Shift+D".to_string()
    }
}

/// 默认触发模式：全平台 Hold（按住说话）。短按（< short_press_ms）视为误触，
/// 取消录音并补发原按键功能——macOS Fn 走 flagsChanged 补发、Windows CapsLock
/// 走钩子补发，均不丢系统原功能。
fn default_hotkey_mode() -> HotkeyMode {
    HotkeyMode::Hold
}
fn default_file_seg_duration_secs() -> u32 {
    60
}
fn default_file_seg_overlap_secs() -> u32 {
    4
}

/// 默认本地润色模型（本地三件套冻结目录：配置缺省时先落 2B 均衡档，
/// 推荐器在设置页首次打开后按机型改写为 0.8/2/4）。
pub const POLISH_DEFAULT_LOCAL_MODEL: &str = "qwen3.5-2b";

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
            hotkey: default_hotkey(),
            hotkey_mode: default_hotkey_mode(),
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
            polish_cloud_model: String::new(),
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
            translate_local_model: default_translate_local_model(),
            translate_use_llm_fallback: false,
            translate_policy: TranslatePolicy::PreferCloud,
            prefix_roles_enabled: true,
            assistant_name: default_assistant_name(),
            ui_lang: default_ui_lang(),
            insert_strategy: InsertStrategy::Auto,
            paste_fallback_apps: Vec::new(),
            restore_clipboard: true,
            short_press_ms: 300,
            fn_repost_enabled: true,
            fn_repost_tis_fallback: false,
            windows_tsf_enabled: false,
            windows_tsf_fallback: true,
            file_seg_duration_secs: 60,
            file_seg_overlap_secs: 4,
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

    /// 当前启用的本地润色模型 id（规范化；旧 1.5B 配置映射到 2B 档）。
    pub fn resolved_polish_local_model(&self) -> String {
        crate::llm_catalog::normalize_polish_model_id(&self.polish_local_model).to_string()
    }

    /// 当前启用的本地专翻模型 id（规范化；空 = 未选专翻）。
    pub fn resolved_translate_local_model(&self) -> String {
        let t = self.translate_local_model.trim();
        if t.is_empty() {
            String::new()
        } else {
            t.to_lowercase()
        }
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

    /// 云端 LLM 必填项（协议 / Endpoint / API Key）完整性检查。
    ///
    /// - Endpoint 与 API Key 全空 → `Ok(false)`：未启用云端 LLM（合法）。
    /// - 只填其一 → `Err`：必填项不完整，拒绝使用/保存。
    /// - 都填 → 校验 Endpoint URL，通过返回 `Ok(true)`。
    ///
    /// 协议是枚举恒有值（默认 openai_chat），无需判空。
    pub fn check_cloud_llm(&self) -> crate::Result<bool> {
        let endpoint = self.polish_cloud_endpoint.trim();
        let key = self.polish_cloud_api_key.trim();
        if endpoint.is_empty() && key.is_empty() {
            return Ok(false);
        }
        if endpoint.is_empty() {
            return Err(Error::Config(
                "云端 LLM 配置不完整：Endpoint 为必填项（需与 API Key 同时填写，或两者都清空）"
                    .into(),
            ));
        }
        if key.is_empty() {
            return Err(Error::Config(
                "云端 LLM 配置不完整：API Key 为必填项（需与 Endpoint 同时填写，或两者都清空）"
                    .into(),
            ));
        }
        crate::endpoint::validate_endpoint(endpoint)
            .map_err(|e| Error::Config(format!("云端 LLM Endpoint 校验失败：{e}")))?;
        Ok(true)
    }

    /// P2：保存期校验 P2 新增字段的范围（serde 之外的强约束，失败整单不落盘）。
    /// 范围与 p2-design「配置模型」一致：
    /// - `short_press_ms ∈ [100, 800]`
    /// - `file_seg_duration_secs ∈ [10, 180]`
    /// - `file_seg_overlap_secs ∈ [1, 30]` 且 `< file_seg_duration_secs`
    pub fn validate_p2_fields(&self) -> crate::Result<()> {
        if !(100..=800).contains(&self.short_press_ms) {
            return Err(Error::Config(format!(
                "短按阈值须在 100..=800 之间，当前 {}（默认 300）",
                self.short_press_ms
            )));
        }
        if !(10..=180).contains(&self.file_seg_duration_secs) {
            return Err(Error::Config(format!(
                "分段时长须在 10..=180 之间，当前 {}（默认 60）",
                self.file_seg_duration_secs
            )));
        }
        if !(1..=30).contains(&self.file_seg_overlap_secs) {
            return Err(Error::Config(format!(
                "分段重叠须在 1..=30 之间，当前 {}（默认 4）",
                self.file_seg_overlap_secs
            )));
        }
        if self.file_seg_overlap_secs >= self.file_seg_duration_secs {
            return Err(Error::Config(
                "分段参数非法：须 10≤duration≤180、1≤overlap≤30 且 overlap<duration".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_lang_defaults_to_zh_and_survives_serde() {
        // 默认中文界面；旧持久化 JSON 没有 ui_lang 字段 → serde 回退默认。
        assert_eq!(AppConfig::default().ui_lang, "zh");
        let mut v = serde_json::to_value(AppConfig::default()).unwrap();
        v.as_object_mut().unwrap().remove("ui_lang");
        let old: AppConfig = serde_json::from_value(v).unwrap();
        assert_eq!(old.ui_lang, "zh");
        // 显式写 en 可反序列化。
        let mut v2 = serde_json::to_value(AppConfig::default()).unwrap();
        v2.as_object_mut()
            .unwrap()
            .insert("ui_lang".into(), "en".into());
        let cfg: AppConfig = serde_json::from_value(v2).unwrap();
        assert_eq!(cfg.ui_lang, "en");
    }

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
        assert_eq!(
            serde_json::to_string(&InsertStrategy::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::to_string(&InsertStrategy::Type).unwrap(),
            "\"type\""
        );
        assert_eq!(
            serde_json::to_string(&InsertStrategy::Paste).unwrap(),
            "\"paste\""
        );
    }

    #[test]
    fn p1_fields_have_defaults() {
        // P1 新字段全部 #[serde(default)]：旧 JSON 可反序列化且默认值符合设计。
        let c = AppConfig::default();
        assert_eq!(c.translate_hotkey, None);
        assert_eq!(c.translate_target_lang, "en");
        assert!(!c.translate_with_polish);
        assert!(c.prefix_roles_enabled);
        assert_eq!(c.insert_strategy, InsertStrategy::Auto);
        assert!(c.paste_fallback_apps.is_empty());
        assert!(c.restore_clipboard);
    }

    #[test]
    fn llm_suite_fields_have_defaults() {
        // 本地三件套新字段全部 #[serde(default)]：旧 JSON 可反序列化。
        let c = AppConfig::default();
        assert_eq!(c.translate_local_model, "milmmt-1b");
        assert!(!c.translate_use_llm_fallback);
        assert_eq!(c.translate_policy, TranslatePolicy::PreferCloud);
        assert_eq!(c.polish_local_model, "qwen3.5-2b");
    }

    #[test]
    fn translate_policy_serde_snake_case() {
        let p: TranslatePolicy = serde_json::from_str("\"prefer_cloud\"").unwrap();
        assert_eq!(p, TranslatePolicy::PreferCloud);
        let p: TranslatePolicy = serde_json::from_str("\"prefer_local\"").unwrap();
        assert_eq!(p, TranslatePolicy::PreferLocal);
        assert_eq!(
            serde_json::to_string(&TranslatePolicy::PreferCloud).unwrap(),
            "\"prefer_cloud\""
        );
    }

    #[test]
    fn legacy_polish_model_resolves_to_2b() {
        // T11：旧 polish_local_model=qwen2.5-1.5b-… 读配置时映射到 qwen3.5-2b，不读旧文件。
        let c = AppConfig {
            polish_local_model: "qwen2.5-1.5b-instruct-q4_k_m".into(),
            translate_local_model: "".into(),
            ..Default::default()
        };
        assert_eq!(c.resolved_polish_local_model(), "qwen3.5-2b");
        // 空 translate_local_model = 未选专翻。
        assert_eq!(c.resolved_translate_local_model(), "");
    }

    #[test]
    fn p2_fields_have_defaults() {
        let c = AppConfig::default();
        assert_eq!(c.short_press_ms, 300);
        assert!(c.fn_repost_enabled);
        assert!(!c.fn_repost_tis_fallback);
        // R11：TSF FFI 落地前默认关闭，避免配置层承诺未实现的上屏通道。
        assert!(!c.windows_tsf_enabled);
        assert!(c.windows_tsf_fallback);
        assert_eq!(c.file_seg_duration_secs, 60);
        assert_eq!(c.file_seg_overlap_secs, 4);
    }

    /// 默认快捷键与触发模式按平台配对：Windows 用 CapsLock+Hold（单键按住说话，
    /// 短按补发保留原功能）；macOS 保持 Fn+Toggle；其它平台组合键+Toggle。
    #[test]
    fn hotkey_default_pairs_with_mode_per_platform() {
        let c = AppConfig::default();
        #[cfg(target_os = "macos")]
        assert_eq!(
            (&c.hotkey, c.hotkey_mode),
            (&"Fn".to_string(), HotkeyMode::Hold)
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            (&c.hotkey, c.hotkey_mode),
            (&"CapsLock".to_string(), HotkeyMode::Hold)
        );
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(
            (&c.hotkey, c.hotkey_mode),
            (&"Ctrl+Shift+D".to_string(), HotkeyMode::Hold)
        );
    }

    #[test]
    fn legacy_config_without_p2_fields_deserializes() {
        // P2 字段缺失的旧 JSON 仍可反序列化（全部 serde default）。
        let json = r#"{
            "active_provider": 0,
            "providers": [],
            "hotkey": "Fn",
            "mute_other_audio": false
        }"#;
        let c: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.short_press_ms, 300);
        assert!(c.fn_repost_enabled);
        assert_eq!(c.file_seg_duration_secs, 60);
        assert_eq!(c.file_seg_overlap_secs, 4);
    }

    #[test]
    fn legacy_config_without_llm_suite_fields_deserializes() {
        // 三件套新字段（translate_local_model / fallback / policy）缺失的旧 JSON
        // 仍可反序列化，且默认值与 Default::default() 一致（llm_suite_fields_have_defaults
        // 只测 Default 路径，这里钉住 serde default 路径）。
        let json = r#"{
            "active_provider": 0,
            "providers": [],
            "hotkey": "Fn",
            "mute_other_audio": false
        }"#;
        let c: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.translate_local_model, "milmmt-1b");
        assert!(!c.translate_use_llm_fallback);
        assert_eq!(c.translate_policy, TranslatePolicy::PreferCloud);
        assert_eq!(c.polish_local_model, "qwen3.5-2b");
    }

    #[test]
    fn resolved_translate_local_model_normalizes_input() {
        // trim + 小写归一；未知 id 原样（小写化后）透传，不强制目录内。
        let mut c = AppConfig {
            translate_local_model: " MiLMMT-1B ".into(),
            ..Default::default()
        };
        assert_eq!(c.resolved_translate_local_model(), "milmmt-1b");
        c.translate_local_model = "Hy-MT-1.8B".into();
        assert_eq!(c.resolved_translate_local_model(), "hy-mt-1.8b");
    }

    #[test]
    fn polish_cloud_model_defaults_to_empty() {
        // 云端模型 ID 默认为空（可选；留空时请求不带 model 字段）。
        assert_eq!(AppConfig::default().polish_cloud_model, "");
        let json = r#"{
            "active_provider": 0,
            "providers": [],
            "hotkey": "Fn",
            "mute_other_audio": false
        }"#;
        let c: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(c.polish_cloud_model, "");
    }

    #[test]
    fn polish_cloud_protocol_wire_format_matches_frontend() {
        // 序列化名必须与前端 PolishCloudProtocol 联合类型一致，
        // 否则 save_app_config 反序列化报 unknown variant、整单保存被拒。
        let json = serde_json::to_string(&AppConfig::default()).unwrap();
        assert!(
            json.contains("\"polish_cloud_protocol\":\"openai_chat\""),
            "{json}"
        );
        for (variant, raw) in [
            (PolishCloudProtocol::OpenAiChat, "openai_chat"),
            (PolishCloudProtocol::Anthropic, "anthropic"),
            (PolishCloudProtocol::OpenAiResponses, "openai_responses"),
        ] {
            assert_eq!(
                serde_json::to_value(variant).unwrap(),
                serde_json::json!(raw)
            );
        }
        // 前端实际发送的值能反序列化。
        for raw in ["openai_chat", "anthropic", "openai_responses"] {
            let _: PolishCloudProtocol = serde_json::from_value(serde_json::json!(raw)).unwrap();
        }
        // 兼容早期 snake_case 落库的旧值（open_ai_chat / open_ai_responses）。
        assert_eq!(
            serde_json::from_value::<PolishCloudProtocol>(serde_json::json!("open_ai_chat"))
                .unwrap(),
            PolishCloudProtocol::OpenAiChat
        );
        assert_eq!(
            serde_json::from_value::<PolishCloudProtocol>(serde_json::json!("open_ai_responses"))
                .unwrap(),
            PolishCloudProtocol::OpenAiResponses
        );
    }

    #[test]
    fn check_cloud_llm_requires_endpoint_and_key() {
        // 全空 = 未启用云端 LLM（合法）。
        let c = AppConfig::default();
        assert!(matches!(c.check_cloud_llm(), Ok(false)));
        // 只填 key → 报缺 Endpoint。
        let c = AppConfig {
            polish_cloud_api_key: "sk-123".into(),
            ..AppConfig::default()
        };
        assert!(c.check_cloud_llm().is_err());
        // 只填 endpoint → 报缺 API Key。
        let c = AppConfig {
            polish_cloud_endpoint: "https://api.openai.com/v1".into(),
            ..AppConfig::default()
        };
        assert!(c.check_cloud_llm().is_err());
        // 都填且 URL 合法 → Ok(true)。
        let c = AppConfig {
            polish_cloud_endpoint: "https://api.openai.com/v1".into(),
            polish_cloud_api_key: "sk-123".into(),
            ..AppConfig::default()
        };
        assert!(matches!(c.check_cloud_llm(), Ok(true)));
        // 都填但 URL 非法 → Err。
        let c = AppConfig {
            polish_cloud_endpoint: "ftp://bad.example.com".into(),
            polish_cloud_api_key: "sk-123".into(),
            ..AppConfig::default()
        };
        assert!(c.check_cloud_llm().is_err());
    }

    #[test]
    fn validate_p2_fields_accepts_default() {
        assert!(AppConfig::default().validate_p2_fields().is_ok());
    }

    #[test]
    fn validate_p2_fields_rejects_out_of_range() {
        let c = AppConfig {
            short_press_ms: 50,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_err());
        let c = AppConfig {
            short_press_ms: 900,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_err());

        let c = AppConfig {
            file_seg_duration_secs: 5,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_err());
        let c = AppConfig {
            file_seg_duration_secs: 200,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_err());

        let c = AppConfig {
            file_seg_overlap_secs: 0,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_err());
        let c = AppConfig {
            file_seg_overlap_secs: 40,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_err());
    }

    #[test]
    fn validate_p2_fields_rejects_overlap_not_less_than_duration() {
        let c = AppConfig {
            file_seg_duration_secs: 10,
            file_seg_overlap_secs: 10,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_err());
        let c = AppConfig {
            file_seg_duration_secs: 10,
            file_seg_overlap_secs: 9,
            ..AppConfig::default()
        };
        assert!(c.validate_p2_fields().is_ok());
    }
}
