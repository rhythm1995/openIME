//! 润色 prompt 拼装 — Light 仅校对 / Heavy 改写润色通顺。
//! P1 增加：翻译目标语言表（R4）+ 翻译 / 润色+翻译 prompt（R4）+ QA 系统 prompt（R6）。

use crate::traits::PolishMode;

/// R4：润色+翻译哨兵。合成调用里，译文与润色后的原文分别包在这两个标记内；
/// 解析失败 → 回退纯 translate_text。
pub const POLISHED_SOURCE_SENTINEL: &str = "[[OPENIME_POLISHED_SOURCE]]";
pub const TRANSLATION_SENTINEL: &str = "[[OPENIME_TRANSLATION]]";

/// 目标语言短码 → prompt 用名（未知短码原样传入，UI 不下发未知值）。
///
/// 基础 7 语（润色模型兼译可用）+ 扩展集（云端 / 本地专翻可选，见 Settings 目标语言分档）。
/// 乌克兰语（uk）仅 HY-MT 与云端支持；MiLMMT 走 `lang_english_name` 的 None 回退。
pub fn lang_display_name(code: &str) -> &str {
    let lower = code.trim().to_lowercase();
    match lower.as_str() {
        "zh" => "中文",
        "zh-hant" | "zh-tw" => "繁體中文",
        "yue" => "粵語",
        "en" | "en-us" => "English",
        "ja" => "日本語",
        "ko" => "한국어",
        "fr" => "français",
        "de" => "Deutsch",
        "es" => "español",
        "ar" => "العربية",
        "th" => "ไทย",
        "tr" => "Türkçe",
        "ru" => "Русский",
        "pt" | "pt-br" | "pt-pt" => "Português",
        "id" => "Bahasa Indonesia",
        "hi" => "हिन्दी",
        "vi" => "Tiếng Việt",
        "pl" => "Polski",
        "uk" => "Українська",
        "fa" => "فارسی",
        "uz" => "Oʻzbekcha",
        // 未知短码：原样传入（借用输入参数，生命周期安全）。
        _ => code,
    }
}

/// R4：翻译 messages。`target_lang` 已是 prompt 用名（调用方先 `lang_display_name`）。
pub fn build_translate_messages(text: &str, target_lang: &str) -> Vec<(String, String)> {
    vec![
        (
            "system".into(),
            format!(
                "你是语音输入法的翻译助手。把用户语音识别的内容翻译成{target_lang}。\n\
                 要求：\n\
                 1. 只输出译文本身，不要解释、不要加引号或前后缀。\n\
                 2. 保留原意与人称，专有名词可保留原文。\n\
                 3. 翻译结果应自然、符合目标语言习惯。"
            ),
        ),
        ("user".into(), text.to_string()),
    ]
}

/// R4：「先润色再翻译」合成调用 messages：一次调用输出哨兵包裹的两段。
pub fn build_polish_translate_messages(text: &str, target_lang: &str) -> Vec<(String, String)> {
    vec![
        (
            "system".into(),
            format!(
                "你是语音输入法的润色+翻译助手。任务分两步：\n\
                 1. 先把语音识别原文润色通顺（去口头禅、修识别错误、补标点，保留原意）。\n\
                 2. 再把润色后的内容翻译成{target_lang}。\n\
                 输出必须严格使用下面的格式，两段之间不要有任何其他文字：\n\
                 {POLISHED_SOURCE_SENTINEL}\n\
                 润色后的原文\n\
                 {TRANSLATION_SENTINEL}\n\
                 译文\n\
                 不要加解释、引号或 Markdown 标记。"
            ),
        ),
        ("user".into(), text.to_string()),
    ]
}

// ── 本地专翻 prompt（T8）────────────────────────────────

/// 语言短码 → MiLMMT 官方模板用英文名（未知返回 None，调用方回退通用模板）。
///
/// MiLMMT-46 不含乌克兰语（uk）→ None → 通用模板；其余扩展集语种均映射。
pub fn lang_english_name(code: &str) -> Option<&'static str> {
    match code.trim().to_lowercase().as_str() {
        "zh" | "zh-cn" | "zh-hans" => Some("Chinese (Simplified)"),
        "zh-hant" | "zh-tw" => Some("Chinese (Traditional)"),
        "yue" => Some("Cantonese"),
        "en" | "en-us" => Some("English"),
        "ja" => Some("Japanese"),
        "ko" => Some("Korean"),
        "fr" => Some("French"),
        "de" => Some("German"),
        "es" => Some("Spanish"),
        "ar" => Some("Arabic"),
        "th" => Some("Thai"),
        "tr" => Some("Turkish"),
        "ru" => Some("Russian"),
        "pt" | "pt-br" | "pt-pt" => Some("Portuguese"),
        "id" => Some("Indonesian"),
        "hi" => Some("Hindi"),
        "vi" => Some("Vietnamese"),
        "pl" => Some("Polish"),
        "fa" => Some("Persian"),
        "uz" => Some("Uzbek"),
        _ => None,
    }
}

/// 源语判定（T8）：配置语言优先；`auto`/空 用脚本粗分（CJK/Hangul/Kana/Latin）。
pub fn detect_source_lang(text: &str, configured: &str) -> String {
    let cfg = configured.trim().to_lowercase();
    if !cfg.is_empty() && cfg != "auto" {
        return cfg;
    }
    let has = |f: fn(char) -> bool| text.chars().any(f);
    let kana = |c: char| matches!(c as u32, 0x3040..=0x30FF);
    let hangul = |c: char| matches!(c as u32, 0xAC00..=0xD7AF | 0x1100..=0x11FF);
    let cjk = |c: char| matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF);
    if has(kana) {
        "ja".into()
    } else if has(hangul) {
        "ko".into()
    } else if has(cjk) {
        "zh".into()
    } else {
        "en".into()
    }
}

/// 本地专翻 messages：按模型 id 选官方模板；未知/兼译走通用 Instruct。
///
/// - `milmmt-1b`：MiLMMT 官方 `Translate this from {src} to {tgt}`（user-only）。
/// - `hy-mt-1.8b`：HY-MT 中英模板 / 英文模板（user-only，只输出译文）。
/// - 其它（兼译 = 润色模型）：通用 `build_translate_messages`。
pub fn build_local_translate_messages(
    model_id: &str,
    text: &str,
    src_lang: &str,
    target_lang: &str,
) -> Vec<(String, String)> {
    match model_id {
        "milmmt-1b" => match lang_english_name(src_lang) {
            Some(src) => vec![(
                "user".into(),
                format!(
                    "Translate this from {src} to {target_lang}:\n{src}: {text}\n{target_lang}:"
                ),
            )],
            None => build_translate_messages(text, target_lang),
        },
        "hy-mt-1.8b" => {
            // 中文变体（简/繁/粤）目标用中文模板，其余用英文模板（官方双语模板）。
            let is_zh_pair =
                src_lang == "zh" || matches!(target_lang, "中文" | "繁體中文" | "粵語");
            let user = if is_zh_pair {
                format!(
                    "将以下文本翻译为{target_lang}，注意只需要输出翻译后的结果，不要额外解释：\n\n{text}"
                )
            } else {
                format!(
                    "Translate the following text into {target_lang}. Only output the translation without any additional explanation:\n\n{text}"
                )
            };
            vec![("user".into(), user)]
        }
        _ => build_translate_messages(text, target_lang),
    }
}

/// 明显指令泄漏判定（T8 失败语义）：以这些开头的小模型输出视为失败，不上屏。
///
/// `sure` / `当然` 这类确认词后必须紧跟标点（`Sure, …` / `当然，…`）才算泄漏；
/// `Sure thing …` 这类以确认词开头的正常译文不算（防误伤）。
pub fn looks_like_instruction_leak(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    starts_with_word_punct(&t, "sure")
        || starts_with_word_punct(&t, "当然")
        || starts_with_word_punct(&t, "翻译如下")
        || t.starts_with("here is")
        || t.starts_with("here's")
}

/// 前缀词后紧跟标点（或串尾）才算泄漏句式；紧跟空白+正文则不算。
fn starts_with_word_punct(t: &str, word: &str) -> bool {
    let Some(rest) = t.strip_prefix(word) else {
        return false;
    };
    rest.is_empty()
        || rest.starts_with(|c: char| c.is_ascii_punctuation() || "，。！？：；、…—（".contains(c))
}

/// 构造 chat messages：`(role, content)` 列表，兼容 OpenAI / llama chat template。
///
/// - mode=Off 时调用方应在外层跳过；此处仍返回最小透传（不 panic）。
/// - Light：只纠 ASR 错，不改措辞（含 no-op 直通示例抑制过度纠正）。
/// - Heavy：允许改写润色通顺、调整语序，但保留原意、不扩写。
/// - hotwords：专有名词保留写法。
pub fn build_messages(
    text: &str,
    mode: PolishMode,
    hotwords: &[String],
    style_prompt: Option<&str>,
) -> Vec<(String, String)> {
    let mut system = if mode == PolishMode::Heavy {
        // F1：Heavy 模式下，若选中风格包，用其 system_prompt 替代默认 Heavy prompt。
        style_prompt
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| heavy_system().to_string())
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
        let msgs = build_messages("你好", PolishMode::Light, &[], None);
        let sys = &msgs[0].1;
        assert!(sys.contains("纠错") || sys.contains("修正"));
        assert!(!sys.contains("通顺化"));
        assert!(sys.contains("今天天气挺好的"));
    }

    #[test]
    fn light_user_is_correction() {
        let msgs = build_messages("你好", PolishMode::Light, &[], None);
        assert!(msgs[1].1.contains("纠错"));
    }

    #[test]
    fn heavy_is_rewrite_smooth() {
        let msgs = build_messages("你好", PolishMode::Heavy, &[], None);
        let sys = &msgs[0].1;
        assert!(sys.contains("润色") && sys.contains("通顺化"));
        assert!(sys.contains("保留原意"));
        assert!(msgs[1].1.contains("润色"));
    }

    #[test]
    fn hotwords_appended_to_system() {
        let hw = vec!["openIME".into(), "Paraformer".into()];
        let msgs = build_messages("你好", PolishMode::Light, &hw, None);
        assert!(msgs[0].1.contains("专有名词"));
        assert!(msgs[0].1.contains("openIME"));
        assert!(msgs[0].1.contains("Paraformer"));
    }

    #[test]
    fn heavy_uses_style_prompt() {
        // F1：Heavy 模式下，style_prompt 替代默认 Heavy system。
        let msgs = build_messages(
            "你好",
            PolishMode::Heavy,
            &[],
            Some("你是一个 commit message 生成器"),
        );
        assert!(
            msgs[0].1.contains("commit message 生成器"),
            "Heavy 应使用 style_prompt，得到 {}",
            msgs[0].1
        );
    }

    #[test]
    fn off_mode_does_not_panic() {
        let msgs = build_messages("你好", PolishMode::Off, &[], None);
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn lang_display_name_table() {
        assert_eq!(lang_display_name("zh"), "中文");
        assert_eq!(lang_display_name("en"), "English");
        assert_eq!(lang_display_name("en-US"), "English");
        assert_eq!(lang_display_name("ja"), "日本語");
        assert_eq!(lang_display_name("ko"), "한국어");
        assert_eq!(lang_display_name("fr"), "français");
        assert_eq!(lang_display_name("de"), "Deutsch");
        assert_eq!(lang_display_name("es"), "español");
        // 扩展集（云端 / 专翻可选语种）。
        assert_eq!(lang_display_name("zh-hant"), "繁體中文");
        assert_eq!(lang_display_name("zh-TW"), "繁體中文");
        assert_eq!(lang_display_name("yue"), "粵語");
        assert_eq!(lang_display_name("ar"), "العربية");
        assert_eq!(lang_display_name("th"), "ไทย");
        assert_eq!(lang_display_name("tr"), "Türkçe");
        assert_eq!(lang_display_name("ru"), "Русский");
        assert_eq!(lang_display_name("pt"), "Português");
        assert_eq!(lang_display_name("pt-br"), "Português");
        assert_eq!(lang_display_name("pt-PT"), "Português");
        assert_eq!(lang_display_name("id"), "Bahasa Indonesia");
        assert_eq!(lang_display_name("hi"), "हिन्दी");
        assert_eq!(lang_display_name("vi"), "Tiếng Việt");
        assert_eq!(lang_display_name("pl"), "Polski");
        assert_eq!(lang_display_name("uk"), "Українська");
        assert_eq!(lang_display_name("fa"), "فارسی");
        assert_eq!(lang_display_name("uz"), "Oʻzbekcha");
        // 未知短码原样传入（防御；UI 不下发未知值）。
        assert_eq!(lang_display_name("xx"), "xx");
    }

    #[test]
    fn lang_english_name_table() {
        // MiLMMT 模板用英文名：扩展集（除乌克兰语——MiLMMT-46 不含）全映射。
        assert_eq!(lang_english_name("zh"), Some("Chinese (Simplified)"));
        assert_eq!(lang_english_name("zh-hant"), Some("Chinese (Traditional)"));
        assert_eq!(lang_english_name("yue"), Some("Cantonese"));
        assert_eq!(lang_english_name("ar"), Some("Arabic"));
        assert_eq!(lang_english_name("th"), Some("Thai"));
        assert_eq!(lang_english_name("tr"), Some("Turkish"));
        assert_eq!(lang_english_name("ru"), Some("Russian"));
        assert_eq!(lang_english_name("pt-br"), Some("Portuguese"));
        assert_eq!(lang_english_name("pt-pt"), Some("Portuguese"));
        assert_eq!(lang_english_name("id"), Some("Indonesian"));
        assert_eq!(lang_english_name("hi"), Some("Hindi"));
        assert_eq!(lang_english_name("vi"), Some("Vietnamese"));
        assert_eq!(lang_english_name("pl"), Some("Polish"));
        assert_eq!(lang_english_name("fa"), Some("Persian"));
        assert_eq!(lang_english_name("uz"), Some("Uzbek"));
        // MiLMMT-46 不支持乌克兰语 → None → 通用模板回退。
        assert_eq!(lang_english_name("uk"), None);
        assert_eq!(lang_english_name("xx"), None);
    }

    #[test]
    fn translate_messages_contain_target_lang() {
        let msgs = build_translate_messages("明天开会", "English");
        assert!(msgs[0].1.contains("English"));
        assert_eq!(msgs[1].1, "明天开会");
    }

    #[test]
    fn polish_translate_messages_use_sentinels() {
        let msgs = build_polish_translate_messages("那个明天开会", "日本語");
        let sys = &msgs[0].1;
        assert!(sys.contains(POLISHED_SOURCE_SENTINEL));
        assert!(sys.contains(TRANSLATION_SENTINEL));
        assert!(sys.contains("日本語"));
    }

    // ── T8：本地专翻 prompt ──

    #[test]
    fn milmmt_uses_official_template() {
        let msgs = build_local_translate_messages("milmmt-1b", "我爱机器翻译", "zh", "English");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, "user");
        assert!(msgs[0]
            .1
            .contains("Translate this from Chinese (Simplified) to English"));
        assert!(msgs[0].1.ends_with("English:"));
    }

    #[test]
    fn milmmt_unknown_src_falls_back_to_generic() {
        let msgs = build_local_translate_messages("milmmt-1b", "你好", "xx", "English");
        assert!(msgs[0].0 == "system");
    }

    #[test]
    fn hymt_zh_pair_uses_chinese_template() {
        let msgs = build_local_translate_messages("hy-mt-1.8b", "你好", "zh", "English");
        assert!(msgs[0].1.contains("将以下文本翻译为"));
        let msgs = build_local_translate_messages("hy-mt-1.8b", "Hello", "en", "日本語");
        assert!(msgs[0].1.contains("Translate the following text"));
    }

    #[test]
    fn fallback_generic_translate_prompt() {
        // 兼译（润色模型 id）走通用 Instruct。
        let msgs = build_local_translate_messages("qwen3.5-2b", "明天开会", "zh", "English");
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].1.contains("English"));
    }

    #[test]
    fn source_lang_detection_by_script() {
        assert_eq!(detect_source_lang("hello world", "auto"), "en");
        assert_eq!(detect_source_lang("你好世界", "auto"), "zh");
        assert_eq!(detect_source_lang("こんにちは", "auto"), "ja");
        assert_eq!(detect_source_lang("안녕하세요", "auto"), "ko");
        // 配置语言优先于脚本。
        assert_eq!(detect_source_lang("こんにちは", "zh"), "zh");
        assert_eq!(detect_source_lang("你好", ""), "zh");
    }

    #[test]
    fn source_lang_detection_edges() {
        // 空文本 / 空配置 → en（Latin 兜底，不 panic）。
        assert_eq!(detect_source_lang("", "auto"), "en");
        assert_eq!(detect_source_lang("", ""), "en");
        // 假名 + 谚文混排 → ja 优先（kana 判定在前）。
        assert_eq!(detect_source_lang("こんにちは 안녕", "auto"), "ja");
        // 配置带空白/大小写 → 归一后生效。
        assert_eq!(detect_source_lang("hello", " Auto "), "en");
        assert_eq!(detect_source_lang("hello", " ZH "), "zh");
    }

    #[test]
    fn hymt_target_chinese_uses_chinese_template() {
        // 反向翻译（源英 → 目标中文）：target_lang == "中文" 也算中英对，用中文模板。
        let msgs = build_local_translate_messages("hy-mt-1.8b", "Hello", "en", "中文");
        assert!(
            msgs[0].1.contains("将以下文本翻译为中文"),
            "得到 {}",
            msgs[0].1
        );
        assert!(!msgs[0].1.contains("Translate the following"));
    }

    #[test]
    fn hymt_chinese_variants_use_chinese_template() {
        // 繁體中文 / 粵語目标同样走中文模板（混元以中文语料为主）。
        for tgt in ["繁體中文", "粵語"] {
            let msgs = build_local_translate_messages("hy-mt-1.8b", "Hello", "en", tgt);
            assert!(
                msgs[0].1.contains(&format!("将以下文本翻译为{tgt}")),
                "目标 {tgt} 得到 {}",
                msgs[0].1
            );
        }
    }

    #[test]
    fn milmmt_extended_langs_use_official_template() {
        // 扩展集源语：官方模板能拿到英文名（繁中分列 / 巴葡归一）。
        let msgs = build_local_translate_messages("milmmt-1b", "你好", "zh-hant", "Português");
        assert!(msgs[0].1.contains("Chinese (Traditional)"));
        let msgs = build_local_translate_messages("milmmt-1b", "olá", "pt-br", "English");
        assert!(msgs[0]
            .1
            .contains("Translate this from Portuguese to English"));
    }

    #[test]
    fn milmmt_ukrainian_src_falls_back_to_generic() {
        // uk 不在 MiLMMT-46 → 通用模板（云端或 HY-MT 兜底该语种）。
        let msgs = build_local_translate_messages("milmmt-1b", "Привіт", "uk", "English");
        assert_eq!(msgs[0].0, "system");
    }

    #[test]
    fn instruction_leak_detection_covers_common_prefixes() {
        // 大小写不敏感 + here is 变体 + 中文「当然」前缀（小模型高频泄漏）。
        assert!(looks_like_instruction_leak("SURE, the translation is:"));
        assert!(looks_like_instruction_leak("Sure! Here you go:"));
        assert!(looks_like_instruction_leak("Here is the translation:"));
        assert!(looks_like_instruction_leak("  当然，以下是翻译："));
        assert!(looks_like_instruction_leak("翻译如下："));
        // 确认词开头但后接正文的正常译文不算泄漏（防误伤）。
        assert!(!looks_like_instruction_leak("Sure thing 会议改到三点"));
        assert!(!looks_like_instruction_leak("当然没问题我可以做到"));
        assert!(!looks_like_instruction_leak("Hereby signed."));
    }

    #[test]
    fn instruction_leak_detection() {
        assert!(looks_like_instruction_leak(
            "Sure, here is the translation:"
        ));
        assert!(looks_like_instruction_leak("Here's the translation:"));
        assert!(looks_like_instruction_leak("翻译如下："));
        assert!(!looks_like_instruction_leak("We have a meeting tomorrow."));
    }
}
