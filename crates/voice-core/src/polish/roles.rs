//! R5：LLM 前缀角色——识别结果按「最长别名」分流到带 `match_prefix` 的风格包。
//!
//! 本模块是**唯一**检测实现（p1-design R5 规范全文，以代码为准）：
//! - 别名之后必须是串尾或分隔符 `：:，,。. ` 之一（「翻译家在开会」不匹配「翻译」）。
//! - 剥离分隔符后正文为空（只说了「邮件」/「邮件:」）→ 不触发。
//! - 等长冲突取更小 `ord`。
//! - 只剥最左最长一次。

use crate::store::StylePack;

/// 最长别名匹配。返回命中的包与去前缀正文（已 trim）。
pub fn detect_prefix_role<'a>(
    text: &str,
    packs: &'a [StylePack],
) -> Option<(&'a StylePack, String)> {
    let t = text.trim();
    let mut best: Option<(&StylePack, usize)> = None;
    for p in packs {
        let Some(spec) = p.match_prefix.as_deref() else {
            continue;
        };
        for alias in spec.split('|').map(str::trim).filter(|s| !s.is_empty()) {
            if !starts_with_ignore_case(t, alias) {
                continue;
            }
            let rest: String = t.chars().skip(alias.chars().count()).collect();
            if !prefix_boundary_ok(&rest) {
                continue;
            }
            let n = alias.chars().count();
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
    let rest = rest
        .trim_start_matches(['：', ':', '，', ',', '。', '.', ' '])
        .trim();
    if rest.is_empty() {
        return None;
    }
    Some((pack, rest.to_string()))
}

/// 边界检查：别名之后必须是串尾或分隔符。
fn prefix_boundary_ok(rest: &str) -> bool {
    match rest.chars().next() {
        None => true,
        Some(c) => matches!(c, '：' | ':' | '，' | ',' | '。' | '.' | ' '),
    }
}

/// 忽略大小写的 starts_with（按字符，Unicode 安全）。
fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    let mut t = text.chars();
    for pc in prefix.chars() {
        match t.next() {
            Some(tc) if tc.to_lowercase().eq(pc.to_lowercase()) => {}
            _ => return false,
        }
    }
    true
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
    fn chinese_colon_matches() {
        let packs = [pack("mail", "邮件", 0)];
        let (p, rest) = detect_prefix_role("邮件: 明天三点会议室见", &packs).unwrap();
        assert_eq!(p.id, "mail");
        assert_eq!(rest, "明天三点会议室见");
    }

    #[test]
    fn fullwidth_and_space_separators() {
        let packs = [pack("mail", "邮件", 0)];
        // 全角冒号
        let (_, rest) = detect_prefix_role("邮件：明天", &packs).unwrap();
        assert_eq!(rest, "明天");
        // 仅空格
        let (_, rest) = detect_prefix_role("邮件 明天", &packs).unwrap();
        assert_eq!(rest, "明天");
    }

    #[test]
    fn case_insensitive_ascii_alias() {
        let packs = [pack("mail", "mail", 0)];
        let (p, rest) = detect_prefix_role("MAIL: hi there", &packs).unwrap();
        assert_eq!(p.id, "mail");
        assert_eq!(rest, "hi there");
    }

    #[test]
    fn translate_role_matches() {
        let packs = [translate_pack("translate", "翻译", 0)];
        let (p, rest) = detect_prefix_role("翻译: hello", &packs).unwrap();
        assert_eq!(p.id, "translate");
        assert_eq!(rest, "hello");
    }

    #[test]
    fn boundary_prevents_partial_word_match() {
        // 「翻译家在开会」不匹配「翻译」。
        let packs = [translate_pack("translate", "翻译", 0)];
        assert!(detect_prefix_role("翻译家在开会", &packs).is_none());
    }

    #[test]
    fn empty_body_does_not_trigger() {
        let packs = [pack("mail", "邮件", 0)];
        assert!(detect_prefix_role("邮件", &packs).is_none());
        assert!(detect_prefix_role("邮件:", &packs).is_none());
        assert!(detect_prefix_role("邮件：", &packs).is_none());
        assert!(detect_prefix_role(" 邮件: ", &packs).is_none());
    }

    #[test]
    fn longest_alias_wins() {
        // 「写邮件」比「邮件」长 → 命中写邮件。
        let packs = [pack("mail", "邮件", 0), pack("compose", "写邮件", 1)];
        let (p, rest) = detect_prefix_role("写邮件: 你好", &packs).unwrap();
        assert_eq!(p.id, "compose");
        assert_eq!(rest, "你好");
    }

    #[test]
    fn equal_length_conflict_takes_smaller_ord() {
        let packs = [pack("b", "翻译", 5), translate_pack("a", "翻译", 2)];
        let (p, _) = detect_prefix_role("翻译: hi", &packs).unwrap();
        assert_eq!(p.id, "a");
    }

    #[test]
    fn multi_alias_spec_split_by_pipe() {
        let packs = [pack("cmd", "命令|command|指令", 0)];
        for t in ["命令: ls", "command: ls", "指令: ls"] {
            let (p, rest) = detect_prefix_role(t, &packs).unwrap();
            assert_eq!(p.id, "cmd");
            assert_eq!(rest, "ls");
        }
    }

    #[test]
    fn no_prefix_returns_none() {
        let packs = [pack("mail", "邮件", 0)];
        assert!(detect_prefix_role("明天三点会议室见", &packs).is_none());
        assert!(detect_prefix_role("", &packs).is_none());
        assert!(detect_prefix_role("下午邮件会议", &packs).is_none());
    }

    #[test]
    fn packs_without_prefix_never_match() {
        let mut plain = pack("plain", "邮件", 0);
        plain.match_prefix = None;
        let packs = [plain];
        assert!(detect_prefix_role("邮件: 明天", &packs).is_none());
    }

    #[test]
    fn strips_only_leftmost_prefix_once() {
        let packs = [translate_pack("translate", "翻译", 0)];
        let (_, rest) = detect_prefix_role("翻译: 翻译: hi", &packs).unwrap();
        assert_eq!(rest, "翻译: hi");
    }

    #[test]
    fn mixed_separators_all_accepted() {
        let packs = [pack("mail", "邮件", 0)];
        for t in ["邮件，明天", "邮件,明天", "邮件。明天", "邮件.明天"] {
            let (_, rest) = detect_prefix_role(t, &packs).unwrap();
            assert_eq!(rest, "明天");
        }
    }
}
