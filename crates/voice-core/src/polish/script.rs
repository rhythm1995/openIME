//! 繁简转换（B6）：基于纯 Rust 的 ferrous-opencc（OpenCC 兼容，词级，字典编译进二进制）。
//! 参考思路：OpenLess `coordinator/llm_pipeline.rs::apply_chinese_script_preference`。

use crate::config::ChineseScriptPreference;
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use std::sync::OnceLock;

static S2T: OnceLock<Option<OpenCC>> = OnceLock::new();
static T2S: OnceLock<Option<OpenCC>> = OnceLock::new();

fn s2t() -> Option<&'static OpenCC> {
    S2T.get_or_init(|| OpenCC::from_config(BuiltinConfig::S2t).ok())
        .as_ref()
}

fn t2s() -> Option<&'static OpenCC> {
    T2S.get_or_init(|| OpenCC::from_config(BuiltinConfig::T2s).ok())
        .as_ref()
}

/// 按偏好做繁简转换。Auto = 不转；转换器初始化失败则原样返回（不阻断）。
pub fn convert_script(text: &str, pref: ChineseScriptPreference) -> String {
    match pref {
        ChineseScriptPreference::Auto => text.to_string(),
        ChineseScriptPreference::Simplified => t2s()
            .map(|c| c.convert(text))
            .unwrap_or_else(|| text.to_string()),
        ChineseScriptPreference::Traditional => s2t()
            .map(|c| c.convert(text))
            .unwrap_or_else(|| text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_noop() {
        assert_eq!(
            convert_script("开放中文转换", ChineseScriptPreference::Auto),
            "开放中文转换"
        );
    }

    #[test]
    fn simplified_to_traditional() {
        let r = convert_script("开放中文转换", ChineseScriptPreference::Traditional);
        assert!(
            r.contains('開') || r.contains('換'),
            "简→繁应转换，得到 {r}"
        );
    }

    #[test]
    fn traditional_to_simplified() {
        let r = convert_script("開放中文轉換", ChineseScriptPreference::Simplified);
        assert_eq!(r, "开放中文转换");
    }
}
