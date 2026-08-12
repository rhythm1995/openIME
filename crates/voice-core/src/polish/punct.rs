//! 标点全角→半角转换（B5）：在微信/Telegram 等 IM 里，半角标点更协调。
//! 参考思路：CapsWriter `tools/punc_converter.py`（MIT）。

/// 把中文全角标点转为半角（仅标点，不动汉字/字母/数字）。
pub fn full_to_half_punct(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '，' => ',',
            '。' => '.',
            '！' => '!',
            '？' => '?',
            '：' => ':',
            '；' => ';',
            '（' => '(',
            '）' => ')',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_full_to_half() {
        assert_eq!(full_to_half_punct("你好，世界。"), "你好,世界.");
        assert_eq!(full_to_half_punct("真的吗？"), "真的吗?");
        assert_eq!(full_to_half_punct("（备注）：；"), "(备注):;");
    }

    #[test]
    fn keeps_non_punct() {
        assert_eq!(full_to_half_punct("Hello世界123"), "Hello世界123");
        assert_eq!(full_to_half_punct(""), "");
    }
}
