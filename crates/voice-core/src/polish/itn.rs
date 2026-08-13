//! 数字 ITN（B1）：中文口语数字 → 书面阿拉伯数字。
//!
//! 覆盖：连续数字串（二零二六→2026）、十位 0-99（二十→20、十五→15、二十三→23）、
//! 百分之（百分之二十→20%）。**不覆盖**百千万进位、小数（后续）。
//! 参考思路：CapsWriter `tools/chinese_itn.py`（MIT），用 Rust 规则重写。

/// 把文本里的中文数字段转为阿拉伯数字（含「百分之X」→「X%」）。
pub fn normalize_itn(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        // 百分之 + 数字段 → 数字%
        if chars.get(i..i + 3) == Some(&['百', '分', '之']) {
            let start = i + 3;
            let mut j = start;
            while j < chars.len() && is_cn_digit(chars[j]) {
                j += 1;
            }
            if j > start {
                let seg: String = chars[start..j].iter().collect();
                out.push_str(&seg_to_num(&seg));
                out.push('%');
                i = j;
                continue;
            }
        }
        if is_cn_digit(chars[i]) {
            let start = i;
            while i < chars.len() && is_cn_digit(chars[i]) {
                i += 1;
            }
            let seg: String = chars[start..i].iter().collect();
            // 单字非「十」不转：日常「一/二/三 + 量词」多是不定指（一句话、两条鱼），
            // 转成「1句话」反而不自然。多字段（二十、十五、二零二六、一二三）才转。
            if seg.chars().count() == 1 && seg != "十" {
                out.push_str(&seg);
            } else {
                out.push_str(&seg_to_num(&seg));
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn is_cn_digit(c: char) -> bool {
    matches!(
        c,
        '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '零' | '十'
    )
}

fn cn_digit_val(c: char) -> Option<u32> {
    match c {
        '一' => Some(1),
        '二' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        '零' => Some(0),
        _ => None,
    }
}

/// 中文数字段 → 阿拉伯字符串。含「十」按十位解析；否则逐位（纯串）。
fn seg_to_num(seg: &str) -> String {
    let chars: Vec<char> = seg.chars().collect();
    if let Some(ten_idx) = chars.iter().position(|&c| c == '十') {
        let tens = if ten_idx == 0 {
            1
        } else {
            cn_digit_val(chars[ten_idx - 1]).unwrap_or(1)
        };
        let ones = chars
            .get(ten_idx + 1)
            .and_then(|c| cn_digit_val(*c))
            .unwrap_or(0);
        return (tens * 10 + ones).to_string();
    }
    // 纯串逐位
    let mut s = String::new();
    for c in chars {
        if let Some(d) = cn_digit_val(c) {
            s.push(char::from_digit(d, 10).unwrap());
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_digit_string() {
        assert_eq!(normalize_itn("二零二六年"), "2026年");
        assert_eq!(normalize_itn("编号一二三"), "编号123");
    }

    #[test]
    fn tens_range_0_99() {
        assert_eq!(normalize_itn("二十个"), "20个");
        assert_eq!(normalize_itn("十五"), "15");
        assert_eq!(normalize_itn("二十三"), "23");
        assert_eq!(normalize_itn("十个"), "10个");
    }

    #[test]
    fn percent() {
        assert_eq!(normalize_itn("百分之二十"), "20%");
        assert_eq!(normalize_itn("增长百分之十五"), "增长15%");
    }

    #[test]
    fn no_digit_unchanged() {
        assert_eq!(normalize_itn("你好世界"), "你好世界");
    }

    #[test]
    fn single_digit_kept() {
        // 单字「一/二/三 + 量词」多为不定指，不应转成阿拉伯数字（修「说1句话」bug）。
        assert_eq!(normalize_itn("说一句话"), "说一句话");
        assert_eq!(normalize_itn("两条鱼"), "两条鱼");
        assert_eq!(normalize_itn("有三个人"), "有三个人");
    }

    #[test]
    fn mixed() {
        assert_eq!(normalize_itn("二零二六年二十三个"), "2026年23个");
    }

    #[test]
    fn correct_l0_applies_itn() {
        // 端到端：L0 接入 ITN。
        let r = crate::polish::correct_l0("我有二十个", &[]);
        assert!(r.text.contains("20"), "L0 应做 ITN，得到 {}", r.text);
    }
}
