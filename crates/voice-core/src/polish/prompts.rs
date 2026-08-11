//! 润色 prompt 拼装 — Light 仅校对 / Heavy 改写润色通顺。

use crate::traits::PolishMode;

/// 构造 chat messages：`(role, content)` 列表，兼容 OpenAI / llama chat template。
///
/// - mode=Off 时调用方应在外层跳过；此处仍返回最小透传（不 panic）。
/// - Light：只纠 ASR 错，不改措辞（含 no-op 直通示例抑制过度纠正）。
/// - Heavy：允许改写润色通顺、调整语序，但保留原意、不扩写。
/// - hotwords：专有名词保留写法。
pub fn build_messages(text: &str, mode: PolishMode, hotwords: &[String]) -> Vec<(String, String)> {
    let mut system = if mode == PolishMode::Heavy {
        heavy_system().to_string()
    } else {
        light_system().to_string()
    };

    if !hotwords.is_empty() {
        system.push_str("\n\n另外，下列专有名词请尽量保留写法：");
        system.push_str(&hotwords.join("、"));
        system.push('。');
    }

    let user = if mode == PolishMode::Heavy {
        format!("请润色以下语音识别原文（通顺化、保留原意）：\n{text}")
    } else {
        format!("请纠错以下语音识别原文（只修识别错误，不改措辞）：\n{text}")
    };
    vec![("system".into(), system), ("user".into(), user)]
}

/// Light：严格校对（只修识别错误）。
fn light_system() -> &'static str {
    "你是中文语音输入法的 ASR 纠错助手。任务：只修正语音识别引入的明显错误。\n\
严格要求：\n\
1. 只修同音字/近音字、漏字、明显乱码；不改写、不润色、不扩写、不改变句长/语序/风格。\n\
2. 没有错误就原样输出，不做任何修改。\n\
3. 只输出纠错后的文本，不解释。\n\
示例：\n\
输入：我们在会试室见 → 输出：我们在会议室见\n\
输入：今天天气挺好的 → 输出：今天天气挺好的\n\
输入：明天下午在会试室见 → 输出：明天下午在会议室见\n\
输入：那个那个我想问骑士的工资 → 输出：我想问一下骑士的工资"
}

/// Heavy：改写润色通顺（保留原意）。
fn heavy_system() -> &'static str {
    "你是中文语音输入法的润色助手。任务：把口语化的语音识别原文润色成通顺的书面文字。\n\
要求：\n\
1. 修正识别错误、补中文标点、去掉口头禅（嗯/那个/然后/就是 等）。\n\
2. 可以调整语序、通顺化、合并啰嗦表述，但必须保留原意、不增加信息、不扩写。\n\
3. 保持第一人称和原句意图，不要改成第三人称或偏移主题。\n\
4. 只输出润色后的正文，不解释。\n\
示例：\n\
输入：那个那个我想问一下会议的时间 → 输出：我想问一下会议的时间。\n\
输入：今天天气挺好的 → 输出：今天天气挺好的。\n\
输入：就是那个项目的话我觉得如果能够再完善一下就更好了 → 输出：我觉得这个项目如果能再完善一下就更好了。"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_is_correction_strict() {
        let msgs = build_messages("你好", PolishMode::Light, &[]);
        let sys = &msgs[0].1;
        assert!(sys.contains("纠错") || sys.contains("修正"));
        // Light 不含 Heavy 的"通顺化"改写语义
        assert!(!sys.contains("通顺化"));
        // no-op 直通示例（抑制过度纠正）
        assert!(sys.contains("今天天气挺好的"));
    }

    #[test]
    fn light_user_is_correction() {
        let msgs = build_messages("你好", PolishMode::Light, &[]);
        assert!(msgs[1].1.contains("纠错"));
    }

    #[test]
    fn heavy_is_rewrite_smooth() {
        let msgs = build_messages("你好", PolishMode::Heavy, &[]);
        let sys = &msgs[0].1;
        assert!(sys.contains("润色") && sys.contains("通顺化"));
        assert!(sys.contains("保留原意"));
        // user 文案按模式区分
        assert!(msgs[1].1.contains("润色"));
    }

    #[test]
    fn hotwords_appended_to_system() {
        let hw = vec!["openIME".into(), "Paraformer".into()];
        let msgs = build_messages("你好", PolishMode::Light, &hw);
        assert!(msgs[0].1.contains("专有名词"));
        assert!(msgs[0].1.contains("openIME"));
        assert!(msgs[0].1.contains("Paraformer"));
    }

    #[test]
    fn off_mode_does_not_panic() {
        // Off 模式调用方应跳过；此处不应 panic，退回 Light 文案。
        let msgs = build_messages("你好", PolishMode::Off, &[]);
        assert_eq!(msgs.len(), 2);
    }
}
