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
    /// 本地引擎模式："offline"（Fn按下录音、松开后整段解码，精度高）
    /// 或 "realtime"（流式实时转写）。默认 offline。
    #[serde(default = "default_local_mode")]
    pub local_mode: String,
    /// 麦克风设备名（None/空 = 系统默认输入设备）。
    #[serde(default)]
    pub audio_device: Option<String>,
}

fn default_local_mode() -> String {
    "offline".to_string()
}

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
            local_mode: "offline".to_string(),
            audio_device: None,
        }
    }
}

/// sherpa 默认模型目录名（与 model_download 的 SHERPA_MODEL_NAME 对齐）。
const SHERPA_DEFAULT_MODEL: &str = "sherpa-onnx-streaming-paraformer-bilingual-zh-en";

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
        assert_eq!(c.local_mode, "offline");
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
