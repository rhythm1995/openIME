//! 润色结果清洗：去掉模型把同一句重复两遍、原文+改写并排等导致「上屏两次」的输出。

/// 清洗模型输出，避免连续输入两遍同一句。
pub fn sanitize_polish_output(original: &str, polished: &str) -> String {
    let o = original.trim();
    let mut p = polished.trim().to_string();
    if p.is_empty() {
        return o.to_string();
    }

    // 去掉常见包裹与前缀。
    p = strip_wrappers(&p);
    p = strip_labels(&p);
    let p = p.trim();
    if p.is_empty() {
        return o.to_string();
    }

    // 1) 整串是 A+A（无分隔或仅空白/换行/标点分隔）。
    if let Some(once) = split_exact_double(p) {
        return once;
    }

    // 2) 原文非空：润色结果 = 原文 + 原文（及中间空白/标点）。
    if !o.is_empty() {
        if let Some(once) = split_prefixed_double(p, o) {
            return once;
        }
        // 3) 润色结果 = 原文 + 改写，且改写又以原文开头 → 取后半改写版。
        if let Some(stripped) = p.strip_prefix(o) {
            let rest = stripped.trim_start_matches(is_soft_sep).trim();
            if !rest.is_empty() && rest != o && !rest.starts_with(o) {
                // 原文紧跟真正改写：取改写部分（更可能是模型「先回显再改写」）。
                // 但若 rest 明显更长且包含 o，仍可能是重复——交给 split_exact_double。
                if rest.chars().count() <= o.chars().count().saturating_mul(3) {
                    return rest.to_string();
                }
            }
            if rest == o {
                return o.to_string();
            }
        }
    }

    // 4) 按句号/问号等切成两句且内容相同 → 只留一句。
    if let Some(once) = dedupe_two_equal_sentences(p) {
        return once;
    }

    p.to_string()
}

/// 连续相同的 final 去重（ASR 有时 endpoint + flush 推两次）。
pub fn dedupe_consecutive_finals(finals: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in finals {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        if out.last().map(|x| x.as_str()) == Some(t) {
            continue;
        }
        // 后一条是前一条的「整段重复拼接」时，丢后一条。
        if let Some(prev) = out.last() {
            if t == format!("{prev}{prev}") || t.starts_with(prev) && t[prev.len()..].trim() == prev
            {
                continue;
            }
            // 前一条已是后一条×2，用短的替换。
            if prev == &format!("{t}{t}") {
                *out.last_mut().unwrap() = t.to_string();
                continue;
            }
        }
        out.push(t.to_string());
    }
    out
}

fn is_soft_sep(c: char) -> bool {
    c.is_whitespace() || "。.!！?？,，、；;：:".contains(c)
}

fn strip_wrappers(s: &str) -> String {
    let mut t = s.trim().to_string();
    // 成对引号
    let pairs = [
        ('"', '"'),
        ('"', '"'),
        ('\'', '\''),
        ('「', '」'),
        ('『', '』'),
    ];
    for (a, b) in pairs {
        if t.starts_with(a) && t.ends_with(b) && t.chars().count() >= 2 {
            let mut chars = t.chars();
            chars.next();
            let mut body: String = chars.collect();
            if body.ends_with(b) {
                body.pop();
            }
            t = body.trim().to_string();
        }
    }
    // markdown code fence
    if t.starts_with("```") {
        let lines: Vec<&str> = t.lines().collect();
        if lines.len() >= 3 && lines.last().is_some_and(|l| l.trim().starts_with("```")) {
            t = lines[1..lines.len() - 1].join("\n").trim().to_string();
        }
    }
    t
}

fn strip_labels(s: &str) -> String {
    let mut t = s.trim();
    for prefix in [
        "改写：",
        "改写:",
        "结果：",
        "结果:",
        "输出：",
        "输出:",
        "正文：",
        "正文:",
        "润色：",
        "润色:",
    ] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim();
        }
    }
    t.to_string()
}

/// 若 s 是完全相同的两半，返回一半。
fn split_exact_double(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    if n < 4 {
        return None;
    }
    // 偶数字符数且对半相等
    if n % 2 == 0 {
        let mid = n / 2;
        if chars[..mid] == chars[mid..] {
            return Some(chars[..mid].iter().collect());
        }
    }
    // 中间夹一个软分隔
    for mid in (n / 2).saturating_sub(2)..=(n / 2).saturating_add(2) {
        if mid == 0 || mid >= n {
            continue;
        }
        let left: String = chars[..mid].iter().collect();
        let right_raw: String = chars[mid..].iter().collect();
        let right = right_raw.trim_start_matches(is_soft_sep);
        if !left.is_empty() && left == right {
            return Some(left);
        }
        // 左去掉尾部分隔再比
        let left_trim = left.trim_end_matches(is_soft_sep);
        if !left_trim.is_empty() && left_trim == right {
            return Some(left_trim.to_string());
        }
    }
    None
}

/// polished = original + original（中间可有软分隔）。
fn split_prefixed_double(polished: &str, original: &str) -> Option<String> {
    if original.is_empty() || !polished.starts_with(original) {
        return None;
    }
    let rest = polished[original.len()..].trim_start_matches(is_soft_sep);
    if rest == original {
        return Some(original.to_string());
    }
    // original 带句末标点 vs 不带
    let o_core = original.trim_end_matches(is_soft_sep);
    let r_core = rest.trim_end_matches(is_soft_sep);
    if !o_core.is_empty() && o_core == r_core {
        return Some(original.to_string());
    }
    None
}

fn dedupe_two_equal_sentences(s: &str) -> Option<String> {
    // 用中文/英文句末切两段（注意多字节标点的 char 边界）。
    let seps = ['。', '！', '？', '!', '?', '\n'];
    for sep in seps {
        if let Some(i) = s.find(sep) {
            let end = i + sep.len_utf8();
            let left = s[..end].trim();
            let right = s[end..].trim();
            if !left.is_empty() && left == right {
                return Some(left.to_string());
            }
            let left_core = s[..i].trim();
            let right_core = right.trim_end_matches(is_soft_sep);
            if !left_core.is_empty() && left_core == right_core {
                return Some(s[..end].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_double_concat() {
        let o = "为什么连时出了两次?";
        let p = format!("{o}{o}");
        assert_eq!(sanitize_polish_output(o, &p), o);
    }

    #[test]
    fn double_with_no_original_still_splits() {
        let p = "你好世界你好世界";
        assert_eq!(sanitize_polish_output("其它", p), "你好世界");
    }

    #[test]
    fn normal_polish_unchanged() {
        let o = "嗯那个你好";
        let p = "你好。";
        assert_eq!(sanitize_polish_output(o, p), "你好。");
    }

    #[test]
    fn strip_label_prefix() {
        assert_eq!(sanitize_polish_output("嗯你好", "改写：你好。"), "你好。");
    }

    #[test]
    fn dedupe_finals_consecutive() {
        let v = vec!["你好".into(), "你好".into(), "世界".into()];
        assert_eq!(
            dedupe_consecutive_finals(&v),
            vec!["你好".to_string(), "世界".to_string()]
        );
    }

    #[test]
    fn dedupe_finals_skips_empty() {
        let v = vec!["  ".into(), "a".into()];
        assert_eq!(dedupe_consecutive_finals(&v), vec!["a".to_string()]);
    }
}
