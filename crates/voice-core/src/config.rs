//! 应用与 provider 配置。

use serde::{Deserialize, Serialize};

use crate::Error;

/// 一期支持的 provider 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// 本地 sherpa-onnx（离线）。
    Sherpa,
    /// 阿里云百炼 Protocol A（流式 WebSocket）。
    Bailian,
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
                if self.base_url.trim().is_empty() {
                    return Err(Error::Config("bailian provider 缺少 base_url".into()));
                }
                let url = self.base_url.trim();
                let valid = url.starts_with("wss://")
                    || url.starts_with("ws://")
                    || url.starts_with("https://")
                    || url.starts_with("http://");
                if !valid {
                    return Err(Error::Config(
                        "base_url 必须以 ws://, wss://, http:// 或 https:// 开头".into(),
                    ));
                }
                if self.api_key.trim().is_empty() {
                    return Err(Error::Config("bailian provider 缺少 api_key".into()));
                }
                if self.model.trim().is_empty() {
                    return Err(Error::Config("bailian provider 缺少 model".into()));
                }
            }
        }
        Ok(())
    }
}

/// 二期文本润色路由策略（与 ASR PreferLocal 对称）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolishPolicy {
    /// 优先本地 GGUF，失败/未装可回退云端（若已配 key）。
    #[default]
    PreferLocal,
    /// 优先云端 chat。
    PreferCloud,
    /// 仅本地。
    LocalOnly,
    /// 仅云端。
    CloudOnly,
    /// 强制关闭润色（等同 polish_enabled=false）。
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
    /// 当前人设 id（None = 仅 Light 润色）。
    #[serde(default)]
    pub active_persona_id: Option<String>,
    /// 单次润色超时（毫秒）。
    #[serde(default = "default_polish_timeout_ms")]
    pub polish_timeout_ms: u32,
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
            }],
            hotkey: "Fn".to_string(),
            mute_other_audio: false,
            launch_at_login: false,
            // local_mode 与默认 ASR（Zipformer 流式）对齐；新逻辑以 local_asr_model 为准。
            local_mode: "realtime".to_string(),
            local_asr_model: crate::asr_catalog::default_asr_model_id().to_string(),
            audio_device: None,
            polish_enabled: false,
            polish_policy: PolishPolicy::PreferLocal,
            polish_local_model: POLISH_DEFAULT_LOCAL_MODEL.to_string(),
            polish_cloud_model: "qwen-turbo".to_string(),
            active_persona_id: None,
            polish_timeout_ms: 800,
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
            | crate::asr_catalog::ASR_MODEL_FIRERED_LARGE => "offline".to_string(),
            _ => "realtime".to_string(),
        };
        if let Some(p) = self.providers.get_mut(self.active_provider) {
            if p.kind == ProviderKind::Sherpa {
                p.model = id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sherpa_only_needs_model() {
        let c = ProviderConfig {
            kind: ProviderKind::Sherpa,
            base_url: String::new(),
            api_key: String::new(),
            model: "paraformer-online".into(),
            vocabulary_id: None,
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
        };
        assert!(c.validate().is_ok());

        let bad = ProviderConfig {
            kind: ProviderKind::Bailian,
            base_url: "ftp://invalid.example.com".into(),
            api_key: "sk-xxx".into(),
            model: "fun-asr-realtime".into(),
            vocabulary_id: None,
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
}
