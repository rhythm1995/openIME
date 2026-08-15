//! R5：LLM 前缀角色——识别结果按「助手名 + 角色别名」组合分流到带 `match_prefix` 的风格包。
//!
//! 触发格式：`助手名+别名+正文`（如「小友翻译我想要走了」→ 翻译「我想要走了」、
//! 「小友邮件: 明天开会」→ 邮件角色）。组合词（小友翻译 / 小友邮件 / …）由启动同步
//! 写入热词表，L0 拼音纠错把 ASR 错写的同音组合（小又翻忆 等）精准纠回——触发锚点
//! 是自定义非常见词，不再依赖 ASR 标点输出。
//!
//! 规则（本模块是唯一检测实现）：
//! - 助手名为空 → 不触发（功能关闭）。
//! - 文本以「助手名+别名」组合开头即候选（大小写不敏感，兼容英文别名「小友mail」）；
//!   剥组合后顺带剥分隔符（`：:，,。. `，兼容「小友翻译：xxx」），剩余即正文。
//! - 剥离后正文为空（只说「小友翻译」/「小友翻译:」）→ 不触发。
//! - 等长冲突取更小 `ord`；只剥最左最长一次。
//! - 句首是助手名但组合未命中（「小友你好」）→ 调用方用 [`starts_with_assistant`]
//!   跳过润色直出原文（润色模型会把句首别名当正文改坏）。

use crate::store::StylePack;

/// 组合前缀匹配：返回命中的包与去组合正文（已 trim）。
///
/// `assistant_name` 来自配置（trim 后使用；空 = 不触发）。
pub fn detect_prefix_role<'a>(
    text: &str,
    assistant_name: &str,
    packs: &'a [StylePack],
) -> Option<(&'a StylePack, String)> {
    let wake = assistant_name.trim();
    if wake.is_empty() {
        return None;
    }
    let t = text.trim();
    let mut best: Option<(&StylePack, usize)> = None;
    for p in packs {
        let Some(spec) = p.match_prefix.as_deref() else {
            continue;
        };
        for alias in spec.split('|').map(str::trim).filter(|s| !s.is_empty()) {
            // 组合前缀 = 助手名 + [分隔符] + 别名（「小优翻译」/「小优，翻译」）。
            // 中间分隔符容错：用户在助手名与角色名之间的小停顿（~0.2s）会让
            // ASR 插入标点/空格，逐字比较会断裂导致不触发。
            let Some(n) = combo_prefix_len(t, wake, alias) else {
                continue;
            };
            let better = match best {
                None => true,
                Some((_, k)) if n > k => true,
                Some((bp, k)) if n == k && p.ord < bp.ord => true,
                _ => false,
            };
            if better {
                best = Some((p, n));
            }
        }
    }
    let (pack, n) = best?;
    let rest: String = t.chars().skip(n).collect();
    // 兼容「小友翻译：xxx」：剥组合后顺带剥分隔符（无分隔符时天然跳过）。
    let rest = rest.trim_start_matches(SEPARATORS).trim();
    if rest.is_empty() {
        return None;
    }
    Some((pack, rest.to_string()))
}

/// 组合前缀的分隔符集合（助手名/别名之间、组合与正文之间）。
const SEPARATORS: [char; 7] = ['：', ':', '，', ',', '。', '.', ' '];

/// 句首是否是助手名（组合未命中时的润色守卫，如「小友你好」）。
pub fn starts_with_assistant(text: &str, assistant_name: &str) -> bool {
    let wake = assistant_name.trim();
    !wake.is_empty() && starts_with_ignore_case(text.trim(), wake)
}

/// `text` 是否以 `wake + [分隔符*] + alias` 开头。命中返回组合消耗的字符数
/// （含中间分隔符），否则 None。
///
/// 字符比较规则（同音容错，吸收 ASR 错字）：
/// - 汉字按**无声调拼音等值**（「小幽」匹配「小优」）——ASR 同音错字恰好
///   落在被剥掉的前缀里，不上屏也不影响触发；
/// - 其余字符（英文别名等）忽略大小写精确匹配。
fn combo_prefix_len(text: &str, wake: &str, alias: &str) -> Option<usize> {
    use pinyin::ToPinyin;
    // 汉字 → 无声调拼音；非汉字 → None（走精确比较）。
    let py = |c: char| c.to_pinyin().map(|p| p.plain().to_string());
    let eq = |t: char, p: char| match (py(t), py(p)) {
        (Some(a), Some(b)) => a == b,
        _ => t.to_lowercase().eq(p.to_lowercase()),
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    // 助手名。
    for wc in wake.chars() {
        match chars.get(i) {
            Some(&tc) if eq(tc, wc) => i += 1,
            _ => return None,
        }
    }
    // 助手名与别名之间的分隔符（停顿产生的标点/空格，可有多个）。
    while matches!(chars.get(i), Some(&c) if SEPARATORS.contains(&c)) {
        i += 1;
    }
    // 别名（别名内部不容分隔——那是 ASR 错字，交给热词纠错）。
    for ac in alias.chars() {
        match chars.get(i) {
            Some(&tc) if eq(tc, ac) => i += 1,
            _ => return None,
        }
    }
    Some(i)
}

/// 忽略大小写的 starts_with（按字符，Unicode 安全）。
fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    combo_prefix_len(text, prefix, "").is_some()
}

/// 组合词列表：助手名 × 各包全部别名（含英文别名；写入热词时由调用方过滤
/// 无拼音的英文组合）。
pub fn assistant_combo_words(assistant_name: &str, packs: &[StylePack]) -> Vec<String> {
    let wake = assistant_name.trim();
    if wake.is_empty() {
        return Vec::new();
    }
    packs
        .iter()
        .filter_map(|p| p.match_prefix.as_deref())
        .flat_map(|spec| spec.split('|').map(str::trim).filter(|s| !s.is_empty()))
        .map(|alias| format!("{wake}{alias}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(id: &str, prefix: &str, ord: i32) -> StylePack {
        StylePack {
            id: id.into(),
            name: id.into(),
            system_prompt: "prompt".into(),
            is_builtin: true,
            ord,
            match_prefix: Some(prefix.into()),
            provider: None,
            model: None,
            role_kind: crate::store::RoleKind::Default,
            output_mode: crate::store::OutputMode::Insert,
        }
    }

    fn translate_pack(id: &str, prefix: &str, ord: i32) -> StylePack {
        let mut p = pack(id, prefix, ord);
        p.role_kind = crate::store::RoleKind::Translate;
        p
    }

    #[test]
    fn combo_triggers_without_punctuation() {
        // 核心场景：连说无标点直接触发（不再依赖 ASR 标点）。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        let (p, rest) = detect_prefix_role("小友翻译我想要走了", "小友", &packs).unwrap();
        assert_eq!(p.id, "tr");
        assert_eq!(rest, "我想要走了");
    }

    #[test]
    fn combo_with_separator_still_works() {
        // 兼容带标点：「小友翻译：xxx」/「小友翻译，xxx」/「小友翻译 xxx」。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        let cases = [
            ("小友翻译: hello", "hello"),
            ("小友翻译：你好", "你好"),
            ("小友翻译，你好", "你好"),
            ("小友翻译 你好", "你好"),
        ];
        for (t, want) in cases {
            let (_, rest) = detect_prefix_role(t, "小友", &packs).unwrap();
            assert_eq!(rest, want, "输入 {t}");
        }
    }

    #[test]
    fn combo_survives_pause_between_wake_and_alias() {
        // 用户在助手名与角色名之间小停顿（~0.2s）：ASR 在中间插入标点/空格
        //（「小优，翻译明天…」/「小优 翻译明天…」），组合仍要触发并正确剥前缀。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        let cases = [
            ("小优 翻译明天我要开会", "明天我要开会"),
            ("小优，翻译明天我要开会", "明天我要开会"),
            ("小优,翻译明天我要开会", "明天我要开会"),
            ("小优 翻译：明天我要开会", "明天我要开会"),
            ("小优，翻译，明天我要开会", "明天我要开会"),
            ("小优。 翻译 明天我要开会", "明天我要开会"),
        ];
        for (t, want) in cases {
            let (p, rest) = detect_prefix_role(t, "小优", &packs).unwrap();
            assert_eq!(p.id, "tr", "输入 {t}");
            assert_eq!(rest, want, "输入 {t}");
        }
        // 别名内部不容分隔（那是 ASR 错字，交给热词纠错路径）。
        assert!(detect_prefix_role("小优翻 译明天开会", "小优", &packs).is_none());
    }

    #[test]
    fn combo_survives_asr_homophone_errors() {
        // ASR 同音错字（小幽/小又 = 小优，翻忆/翻意 = 翻译）：汉字按拼音等值匹配，
        // 错字恰好落在被剥掉的前缀里，不上屏；可与中间标点容错叠加。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        let cases = [
            ("小幽翻译明天我要开会", "明天我要开会"),
            ("小又翻忆明天我要开会", "明天我要开会"),
            ("小幽，翻译明天我要开会", "明天我要开会"),
            ("小优翻意：明天我要开会", "明天我要开会"),
        ];
        for (t, want) in cases {
            let (p, rest) =
                detect_prefix_role(t, "小优", &packs).unwrap_or_else(|| panic!("应触发：{t}"));
            assert_eq!(p.id, "tr");
            assert_eq!(rest, want, "输入 {t}");
        }
        // 非同音错字不触发（交给热词/正文路径）。
        assert!(detect_prefix_role("小张翻译明天我要开会", "小优", &packs).is_none());
        // 句首同音错字也受守卫保护（组合未命中时跳润色直出）。
        assert!(starts_with_assistant("小幽你好", "小优"));
    }

    #[test]
    fn bare_alias_no_longer_triggers() {
        // 旧设计（裸别名+标点）整体移除：「翻译：xxx」「翻译，xxx」不触发。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        for t in [
            "翻译: hello",
            "翻译，你好",
            "翻译我想要走了",
            "翻译家在开会",
        ] {
            assert!(
                detect_prefix_role(t, "小友", &packs).is_none(),
                "{t} 不应触发（须带助手名）"
            );
        }
    }

    #[test]
    fn empty_assistant_name_disables_detection() {
        let packs = [pack("mail", "邮件|mail|写邮件", 0)];
        assert!(detect_prefix_role("小友邮件: 明天", "", &packs).is_none());
        assert!(detect_prefix_role("邮件: 明天", "  ", &packs).is_none());
        assert!(!starts_with_assistant("小友你好", ""));
    }

    #[test]
    fn custom_assistant_name() {
        // 自定义助手名生效。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        let (p, rest) = detect_prefix_role("阿法翻译我想要走了", "阿法", &packs).unwrap();
        assert_eq!(p.id, "tr");
        assert_eq!(rest, "我想要走了");
        // 换名后旧名不再触发。
        assert!(detect_prefix_role("小友翻译我想要走了", "阿法", &packs).is_none());
    }

    #[test]
    fn single_char_alias_combo_triggers() {
        // 单字别名「译」组合成三字词（小友译），安全触发。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        let (_, rest) = detect_prefix_role("小友译我想要走了", "小友", &packs).unwrap();
        assert_eq!(rest, "我想要走了");
    }

    #[test]
    fn english_alias_combo_case_insensitive() {
        let packs = [pack("mail", "邮件|mail|写邮件", 0)];
        let (p, rest) = detect_prefix_role("小友MAIL: hi there", "小友", &packs).unwrap();
        assert_eq!(p.id, "mail");
        assert_eq!(rest, "hi there");
    }

    #[test]
    fn empty_body_does_not_trigger() {
        let packs = [pack("mail", "邮件|mail|写邮件", 0)];
        assert!(detect_prefix_role("小友邮件", "小友", &packs).is_none());
        assert!(detect_prefix_role("小友邮件:", "小友", &packs).is_none());
        assert!(detect_prefix_role("小友邮件：", "小友", &packs).is_none());
        assert!(detect_prefix_role(" 小友邮件: ", "小友", &packs).is_none());
        // 只有助手名没有别名。
        assert!(detect_prefix_role("小友", "小友", &packs).is_none());
    }

    #[test]
    fn longest_combo_wins() {
        // 「小友写邮件」只命中 compose（mail 的「邮件」组合「小友邮件」不匹配开头）。
        let packs = [pack("mail", "邮件|mail", 0), pack("compose", "写邮件", 1)];
        let (p, rest) = detect_prefix_role("小友写邮件: 你好", "小友", &packs).unwrap();
        assert_eq!(p.id, "compose");
        assert_eq!(rest, "你好");
        // 「小友邮件」命中 mail。
        let (p, _) = detect_prefix_role("小友邮件你好", "小友", &packs).unwrap();
        assert_eq!(p.id, "mail");
    }

    #[test]
    fn equal_length_conflict_takes_smaller_ord() {
        let packs = [pack("b", "翻译", 5), translate_pack("a", "翻译", 2)];
        let (p, _) = detect_prefix_role("小友翻译: hi", "小友", &packs).unwrap();
        assert_eq!(p.id, "a");
    }

    #[test]
    fn mid_text_combo_does_not_trigger() {
        // 组合不在句首不触发。
        let packs = [translate_pack("tr", "翻译|translate|译", 0)];
        assert!(detect_prefix_role("请小友翻译这段话", "小友", &packs).is_none());
        assert!(detect_prefix_role("下午小友邮件会议", "小友", &packs).is_none());
    }

    #[test]
    fn assistant_prefix_guard() {
        // 句首是助手名但组合未命中（小友你好）→ 守卫返回 true。
        assert!(starts_with_assistant("小友你好", "小友"));
        assert!(starts_with_assistant(" 小友帮我查天气 ", "小友"));
        assert!(!starts_with_assistant("翻译这段话", "小友"));
    }

    #[test]
    fn combo_words_for_hotwords() {
        // 组合词清单：助手名 × 全部别名（含单字与英文，写热词时再过滤英文）。
        let packs = [
            translate_pack("tr", "翻译|translate|译", 0),
            pack("mail", "邮件|mail|写邮件", 1),
        ];
        let words = assistant_combo_words("小友", &packs);
        assert!(words.contains(&"小友翻译".to_string()));
        assert!(words.contains(&"小友译".to_string()));
        assert!(words.contains(&"小友translate".to_string()));
        assert!(words.contains(&"小友邮件".to_string()));
        assert!(words.contains(&"小友写邮件".to_string()));
        assert!(words.contains(&"小友mail".to_string()));
        // 空助手名 → 空。
        assert!(assistant_combo_words("", &packs).is_empty());
        // 无前缀包不参与。
        let mut plain = pack("plain", "邮件", 0);
        plain.match_prefix = None;
        assert!(assistant_combo_words("小友", &[plain]).is_empty());
    }
}
