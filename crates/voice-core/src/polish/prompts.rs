//! 润色 prompt 拼装。

use crate::traits::PolishMode;

/// 构造 chat messages：`(role, content)` 列表，兼容 OpenAI / llama chat template。
pub fn build_messages(
    text: &str,
    mode: PolishMode,
    persona_prompt: Option<&str>,
    hotwords: &[String],
) -> Vec<(String, String)> {
    let mut system = String::from(
        "你是中文语音输入法的后处理助手。任务：把语音识别原文改写成通顺、可直接上屏的文本。\n\
规则：\n\
1. 只输出改写后的正文，不要解释、不要引号包裹、不要前缀（不要「改写：」「结果：」等）。\n\
2. 修正明显 ASR 错误，补中文标点，去掉「嗯/啊/那个/就是」等口头禅。\n\
3. 不改变原意，不扩写成长文，不编造原文没有的信息。\n\
4. 若原文已通顺，可做最小修改或原样返回。\n\
5. 只输出一版结果，禁止把同一句话重复输出两遍，禁止原文和改写并排输出。",
    );

    if !hotwords.is_empty() {
        system.push_str("\n5. 下列专有名词请尽量保留写法：");
        system.push_str(&hotwords.join("、"));
        system.push('。');
    }

    if mode == PolishMode::Persona {
        if let Some(p) = persona_prompt.map(str::trim).filter(|s| !s.is_empty()) {
            system.push_str("\n\n【人设】\n");
            system.push_str(p);
        }
    }

    let user = format!("请处理以下语音识别原文：\n{text}");
    vec![
        ("system".into(), system),
        ("user".into(), user),
    ]
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
}
