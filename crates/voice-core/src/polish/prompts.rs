//! 润色 prompt 拼装 — L2 校对模式：只纠 ASR 错，不改措辞。

use crate::traits::PolishMode;

/// 构造 chat messages：`(role, content)` 列表，兼容 OpenAI / llama chat template。
///
/// 约定：
/// - mode=Off 时调用方应在外层跳过，不应走到此函数；但此处仍返回空纠正（不 panic）。
/// - hotwords：追加到 system 的第 6 条（修复此前 5. 重号 bug）。
/// - persona_prompt：仅作"语气参考"，不改"校对"语义。
pub fn build_messages(
    text: &str,
    mode: PolishMode,
    persona_prompt: Option<&str>,
    hotwords: &[String],
) -> Vec<(String, String)> {
    // 校对版 system：严格约束为"只修识别错误"，并用 2 个 no-op 直通示例压住过度纠正
    // （EMNLP 2024：30-50% no-op 示例是抑制 over-correction 最强杠杆）。
    let mut system = String::from(
        "你是中文语音输入法的 ASR 纠错助手。任务：只修正语音识别引入的明显错误。\n\
严格要求：\n\
1. 只修同音字/近音字、漏字、明显乱码；不改写、不润色、不扩写、不改变句长/语序/风格。\n\
2. 没有错误就原样输出，不做任何修改。\n\
3. 只输出纠错后的文本，不解释。\n\
示例：\n\
输入：我们在会试室见 → 输出：我们在会议室见\n\
输入：今天天气挺好的 → 输出：今天天气挺好的\n\
输入：明天下午在会试室见 → 输出：明天下午在会议室见\n\
输入：那个那个我想问骑士的工资 → 输出：我想问一下骑士的工资",
    );

    if !hotwords.is_empty() {
        system.push_str("\n6. 下列专有名词请尽量保留写法：");
        system.push_str(&hotwords.join("、"));
        system.push('。');
    }

    if mode == PolishMode::Persona {
        if let Some(p) = persona_prompt.map(str::trim).filter(|s| !s.is_empty()) {
            // 人设仅作为语气参考，仍只做纠错（不改写）。
            system.push_str("\n\n【人设参考（仅作语气理解，不用于改写）】\n");
            system.push_str(p);
        }
    }

    let user = format!("请纠错以下语音识别原文（只修识别错误，不改措辞）：\n{text}");
    vec![("system".into(), system), ("user".into(), user)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_has_no_persona_block() {
        let msgs = build_messages("你好", PolishMode::Light, Some("正式邮件"), &[]);
        assert_eq!(msgs.len(), 2);
        assert!(!msgs[0].1.contains("人设"));
    }

    #[test]
    fn persona_includes_prompt_and_hotwords() {
        let hw = vec!["openIME".into()];
        let msgs = build_messages("嗯那个 openIME", PolishMode::Persona, Some("口语化"), &hw);
        assert!(msgs[0].1.contains("人设"));
        assert!(msgs[0].1.contains("口语化"));
        assert!(msgs[0].1.contains("openIME"));
    }

    #[test]
    fn prompt_is_correction_not_rewrite() {
        let msgs = build_messages("你好", PolishMode::Light, None, &[]);
        let sys = &msgs[0].1;
        // 校对语义
        assert!(sys.contains("纠错") || sys.contains("修正"));
        // 不得出现"改写上屏"等旧改写流程文案（规则描述里的"不改写"是约束本身，豁免）
        assert!(
            !sys.contains("改写上屏") && !sys.contains("把语音识别原文改写"),
            "校对模式不应包含改写流程文案"
        );
        // no-op 示例至少 1 个
        assert!(sys.contains("今天天气挺好的"));
    }

    #[test]
    fn hotwords_numbering_fixed() {
        let hw = vec!["openIME".into(), "Paraformer".into()];
        let msgs = build_messages("你好", PolishMode::Light, None, &hw);
        assert!(msgs[0].1.contains("6. 下列专有名词"));
        assert!(!msgs[0].1.contains("5. 下列"));
    }
}
