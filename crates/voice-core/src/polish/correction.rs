//! L0 规则层纠错：零延迟、纯本地、确定性。
//!
//! 处理：填充词去除 / 标点归一 / 同音字纠错 / 截断检测。
//! 进入条件：总是先过一遍（即使总体 polish 关闭也做最小清理）；调用方见 `pipeline::apply_polish`。
//! 任何规则失败或无可信候选 → 原样返回，不引入新错误。
//! 热词：参与同音纠错（与热词读音相同的片段替换为热词，覆盖 ASR 同音错字）。

use pinyin::ToPinyin;

/// L0 结果。
#[derive(Debug, Clone)]
pub struct L0Result {
    /// 纠错后文本。
    pub text: String,
    /// 是否真的改过（供 L2 gating 用）。
    pub had_correction: bool,
    /// 是否疑似截断（供前端 "…" 提示或 L2 额外关注）。
    pub truncation_flag: bool,
}

// ── 填充词表 ───────────────────────────────────────────────

/// 单字填充词：几乎总是可删（独立成词时）。额/哎 等兼作助词时在句末保留，由 `is_sentence_particle`
/// 判断。
const SINGLE_FILLERS: &[char] = &[
    '嗯', '呃', '额', '唉', '哎', '哦', '噢', '嘛', '呐', '呀', '哇',
];

/// 多字填充词 / 话语连缀。
const MULTI_FILLERS: &[&str] = &[
    "那个",
    "这个",
    "就是",
    "然后",
    "其实",
    "反正",
    "基本上",
    "对吧",
    "是吧",
    "你知道",
    "怎么说呢",
    "那什么",
    "反正就是",
];

/// 句末语气词：紧贴句末时保留（啊/嘛/吧/呢/呀/哦/噢/嘛）。
const SENTENCE_PARTICLES: &[char] = &['啊', '嘛', '吧', '呢', '呀', '哦', '噢', '哪'];

/// 合法叠词白名单：重复出现也不应塌陷（如 慢慢=慢条斯理、哥哥=称谓）。
const LEGAL_REDUPLICATIONS: &[&str] = &[
    "慢慢", "哥哥", "常常", "刚刚", "刚刚", "久久", "清清", "白白", "高高", "低低", "暗暗", "轻轻",
    "缓缓", "悄悄", "静静", "频频", "天天", "年年", "岁岁", "事事", "时时", "处处", "人人", "看看",
    "想想", "说说", "聊聊", "试试", "走走", "听听", "写写", "读读",
];

/// 软分隔：空白 + 中英文标点。用于是否"孤立"的判断。
fn is_soft_sep_char(c: char) -> bool {
    c.is_whitespace() || "。.!！?？,，、；;：:\"'\"'「」『』()（）【】<>".contains(c)
}

/// 常见标点（不含语气词）。
const COMMON_PUNCTS: &[char] = &['。', '，', '、', '；', '：', '！', '？', '.', ',', '!', '?'];

// ── 同音/近音小字典 ──────────────────────────────────────

/// 极小同音映射：键是错误字，值是正确字候选（按出现频度排）。
/// 覆盖项目最常见的英文错字对。后续可扩展到 pycorrector/same_pinyin.txt，并用 `pinyin` crate 动态拉集。
fn homophone_candidates(ch: char) -> Option<&'static [&'static str]> {
    match ch {
        // de/dé/dě/dè 系
        '德' => Some(&["得", "的"]), // 做德很好→做得很好
        // shi 系
        '是' => None, // 过于高频，不做反向纠（误伤大）
        '事' => None,
        // zai 系：最常见混淆
        '载' | '栽' => Some(&["在", "再"]),
        '在' => None, // 在/再 互为同音，不单向纠，靠上下文（见 correct_homophones）
        // 有/又/由/右
        // 其/期/起/奇
        _ => None,
    }
}

/// 专用于"的/得/地"三用混淆的上下文锚点：`地` 后跟动词时，前面的"的"应为"地"。
fn fix_de_usage(chars: &mut Vec<char>, hotwords: &[String]) {
    // 热词命中时，提高阈值：不轻易改"的得地"（避免把热词拆错）。
    let has_de_hotword = hotwords
        .iter()
        .any(|w| w.contains("得") || w.contains("地") || w.contains("的"));
    if has_de_hotword {
        return;
    }
    // 简化：`的地` 误写 → `地` 前有"的"且次字符后是动词类字（通过是否在常见动词后判断太重，
    // 先做最小规则：的+动词根 且原应为地，暂不做；首轮仅做单字填充的"的得地"隔离已足够）
    let _ = chars; // 占位，首轮不做深度的/得/地纠，交 L2
}

// ── 导出主入口 ───────────────────────────────────────────

/// L0 规则层纠错主入口。
///
/// 依次：去填充词首尾修剪 → 重复填充塌陷 → 标点归一 → 同音纠错 → 截断检测。
/// 任何一步无修改则透传；全程 <5ms/句。
pub fn correct_l0(text: &str, hotwords: &[String]) -> L0Result {
    let orig = text.trim();
    if orig.is_empty() {
        return L0Result {
            text: text.to_string(),
            had_correction: false,
            truncation_flag: false,
        };
    }

    let mut cur = orig.to_string();
    let mut had = false;

    // 1) 首尾填充词整段去除。
    let trimmed = trim_fillers(&cur);
    if trimmed != cur {
        had = true;
        cur = trimmed;
        if cur.trim().is_empty() {
            // 去完变空：保留一个词（如用户只说"嗯那个"→保留"那个"）。
            let fallback = orig
                .chars()
                .filter(|c| !SINGLE_FILLERS.contains(c))
                .collect::<String>()
                .trim()
                .to_string();
            if !fallback.is_empty() {
                cur = fallback;
            } else {
                // 全填充词：保留最后一个原词。
                cur = orig
                    .chars()
                    .last()
                    .map(|c| c.to_string())
                    .unwrap_or_default();
            }
        }
    }

    // 2) 句内连续重复塌陷（那个那个→那个、我我我→我），豁免合法叠词。
    let collapsed = collapse_repeated_fillers(&cur);
    if collapsed != cur {
        had = true;
        cur = collapsed;
    }

    // 3) 中间孤立单字填充（被标点/空格夹着）删除。
    let depadded = strip_mid_single_fillers(&cur);
    if depadded != cur {
        had = true;
        cur = depadded;
    }

    // 4) 标点归一。
    let punc = normalize_punct(&cur);
    if punc != cur {
        had = true;
        cur = punc;
    }

    // 5) 同音纠错：热词同音替换（方案A）+ 固定字典高频错字对。
    let hom = correct_homophones(&cur, hotwords);
    if hom != cur {
        had = true;
        cur = hom;
    }

    // 5b) 数字 ITN（B1）：中文数字→阿拉伯（百分之/十位 0-99/纯串）。
    let itn = super::itn::normalize_itn(&cur);
    if itn != cur {
        had = true;
        cur = itn;
    }

    // 6) 截断检测（在去末尾标点之前：用含标点的文本，避免"我觉得。"被误判为截断）。
    let trunc = detect_truncation(&cur);

    // 7) 去末尾标点：单句输入到聊天框，句末标点违和（B4，CapsWriter trash_punc 风格）。
    let stripped = strip_trailing_punct(&cur);
    if stripped != cur {
        had = true;
        cur = stripped;
    }

    L0Result {
        text: cur,
        had_correction: had,
        truncation_flag: trunc,
    }
}

// ── 1) 填充词首尾修剪 ────────────────────────────────────

fn trim_fillers(s: &str) -> String {
    // 多字填充词优先：重复剥离直到不动。
    let mut cur = s.trim().to_string();
    loop {
        let mut next = cur.clone();
        // 前缀多字
        for fw in MULTI_FILLERS {
            if next.starts_with(fw) {
                next = next[fw.len()..].trim_start().to_string();
                break;
            }
        }
        // 前缀单字
        if next
            .chars()
            .next()
            .map(|c| SINGLE_FILLERS.contains(&c))
            .unwrap_or(false)
        {
            let c = next.chars().next().unwrap();
            next = next[c.len_utf8()..].trim_start().to_string();
        }
        // 后缀多字（但句末语气词不剥）
        // 先判后缀语气词：若最后一个字是语气词且前面紧贴标点或句末，则不剥。
        // 简化：后缀多字仅当末尾不是语气词时剥。
        let tail_is_particle = next
            .chars()
            .last()
            .map(|c| SENTENCE_PARTICLES.contains(&c))
            .unwrap_or(false);
        if !tail_is_particle {
            for fw in MULTI_FILLERS {
                if next.ends_with(fw) {
                    next = next[..next.len() - fw.len()].trim_end().to_string();
                    break;
                }
            }
        }
        // 后缀单字（非语气词）
        if next
            .chars()
            .last()
            .map(|c| SINGLE_FILLERS.contains(&c) && !SENTENCE_PARTICLES.contains(&c))
            .unwrap_or(false)
        {
            let c = next.chars().last().unwrap();
            next = next[..next.len() - c.len_utf8()].trim_end().to_string();
        }
        if next == cur {
            break;
        }
        cur = next;
        if cur.trim().is_empty() {
            // 全为填充词：保留最后一个非单字填充（至少给用户留一个实词），避免输出空串
            cur = s
                .trim()
                .chars()
                .filter(|c| !SINGLE_FILLERS.contains(c))
                .collect::<String>()
                .trim()
                .to_string();
            if cur.is_empty() {
                cur = s
                    .trim()
                    .chars()
                    .last()
                    .map(|c| c.to_string())
                    .unwrap_or_default();
            }
            break;
        }
    }
    cur
}

// ── 2) 连续重复塌陷 ──────────────────────────────────────

fn collapse_repeated_fillers(s: &str) -> String {
    let mut out = s.to_string();
    // 多字重复：那个那个→那个、就是就是→就是 ...
    for fw in MULTI_FILLERS {
        let dbl = format!("{fw}{fw}");
        // 循环剥直到不含 dbl
        while out.contains(&dbl) {
            out = out.replace(&dbl, fw);
        }
    }
    // 单字三连以上塌陷：嗯嗯嗯→嗯（但合法叠词 慢慢/哥哥 等豁免）
    // 先把合法叠词占位，避免被误塌。
    let mut placeholders: Vec<(String, String)> = Vec::new();
    let mut tmp = out.clone();
    for (i, w) in LEGAL_REDUPLICATIONS.iter().enumerate() {
        if tmp.contains(w) {
            let ph = format!("\u{FFF0}{i}\u{FFF1}");
            tmp = tmp.replace(w, &ph);
            placeholders.push((ph, w.to_string()));
        }
    }
    // 单字重复：同一字连续 ≥2 且非合法叠词，已被占位保护，剩余的同字连排塌为 1。
    let chars: Vec<char> = tmp.chars().collect();
    let mut dedup: Vec<char> = Vec::new();
    for c in chars {
        if dedup.last() == Some(&c) && SINGLE_FILLERS.contains(&c) {
            // 填充词单字重复时塌陷
            continue;
        }
        dedup.push(c);
    }
    let mut res: String = dedup.into_iter().collect();
    // 三连以上同字（不一定填充词）：aaa→a（非豁免的才塌）
    // 已处理的填充词单字已塌，剩余如 "好好好" 中的"好"三连仍保留（合法感叹），不强制
    for (ph, orig) in placeholders {
        res = res.replace(&ph, &orig);
    }
    res
}

// ── 3) 中间孤立单字填充删除 ──────────────────────────────

fn strip_mid_single_fillers(s: &str) -> String {
    // 简化：把 "嗯" / "呃" 这类若左右皆为软分隔（空格/标点）则删。
    // 对 "啊/嘛" 等语气词不删（句中语气也可能有意）。
    const MID_REMOVABLE: &[char] = &['嗯', '呃', '额', '唉', '哎'];
    let chars: Vec<char> = s.chars().collect();
    let mut keep = vec![true; chars.len()];
    for i in 0..chars.len() {
        let c = chars[i];
        if !MID_REMOVABLE.contains(&c) {
            continue;
        }
        let left_sep = i == 0 || is_soft_sep_char(chars[i - 1]);
        let right_sep = i + 1 >= chars.len() || is_soft_sep_char(chars[i + 1]);
        if left_sep && right_sep {
            keep[i] = false;
        }
        // 也支持 "、嗯、" 或 " 嗯 " 的两侧软分隔删除
        let left_is_sep_or_filler =
            i == 0 || is_soft_sep_char(chars[i - 1]) || MID_REMOVABLE.contains(&chars[i - 1]);
        let _ = left_is_sep_or_filler;
    }
    let res: String = chars
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(c, _)| *c)
        .collect();
    // 去掉因删除产生的多余空白/标点粘连
    res
}

// ── 4) 标点归一 ──────────────────────────────────────────

fn normalize_punct(s: &str) -> String {
    let mut out = s.to_string();
    // 多标点归一
    let reps: &[(&str, &str)] = &[
        ("。。", "。"),
        ("。。。。。", "。"),
        ("，，", "，"),
        ("？？", "？"),
        ("！！", "！"),
        ("!!", "！"),
        ("??", "？"),
        (",,", "，"),
        ("。。。。", "。"),
    ];
    for (from, to) in reps {
        while out.contains(from) {
            out = out.replace(from, to);
        }
    }
    // 循环再归一一次（处理替换后新生的 "。。 "）
    while out.contains("。。") {
        out = out.replace("。。", "。");
    }
    // 孤立单侧引号去除：若只有一个 " 或 ' 且不成对，删
    // 简化：统计引号数，奇数个时删最后一个孤立
    for (open, close) in [('"', '"'), ('\'', '\''), ('「', '」'), ('『', '』')] {
        let o = out.chars().filter(|&c| c == open).count();
        let c = out.chars().filter(|&c2| c2 == close).count();
        if open == close {
            if o % 2 == 1 {
                // 删最后一个孤立引号
                if let Some(pos) = out.rfind(open) {
                    let before = &out[..pos];
                    let after = &out[pos + open.len_utf8()..];
                    out = format!("{before}{after}");
                }
                let _ = c;
            }
        } else if o != c {
            // 非对称引号不同数：去除孤立侧
            if o > c {
                if let Some(pos) = out.rfind(open) {
                    let before = &out[..pos];
                    let after = &out[pos + open.len_utf8()..];
                    out = format!("{before}{after}");
                }
            } else if let Some(pos) = out.rfind(close) {
                let before = &out[..pos];
                let after = &out[pos + close.len_utf8()..];
                out = format!("{before}{after}");
            }
        }
    }
    out
}

// ── 5) 同音纠错（保守小字典）─────────────────────────────

/// 把汉字串转为无声调拼音（非汉字自动跳过）。如"智谱"→"zhipu"。
fn to_pinyin_plain(s: &str) -> String {
    s.to_pinyin()
        .flatten()
        .map(|p| p.plain().to_string())
        .collect()
}

/// 热词同音纠错：按热词字数滑窗取片段，拼音与热词相同（且片段本身不是该热词）
/// 就替换为热词。覆盖 ASR 最常见的"专有名词被识别成同音常用字"（制谱→智谱）。
///
/// - 不分词、不依赖 jieba：按热词字数滑窗，避开分词粒度问题。
/// - 长热词优先（按字数降序匹配，避免短热词截断长匹配）。
/// - 英文/非汉字热词的拼音为空，自动跳过（中英音译不处理）。
fn correct_hotword_homophones(text: &str, hotwords: &[String]) -> String {
    if hotwords.is_empty() {
        return text.to_string();
    }
    let mut hws: Vec<(&str, String, usize)> = hotwords
        .iter()
        .filter_map(|w| {
            let py = to_pinyin_plain(w);
            if py.is_empty() {
                None
            } else {
                Some((w.as_str(), py, w.chars().count()))
            }
        })
        .collect();
    hws.sort_by_key(|(_, _, n)| *n);
    hws.reverse();

    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let mut matched = false;
        for (hw, hw_py, n) in &hws {
            if i + n <= chars.len() {
                let seg: String = chars[i..i + n].iter().collect();
                if &seg == hw {
                    // 已是热词本身，原样保留。
                    out.extend(chars[i..i + n].iter());
                    i += n;
                    matched = true;
                    break;
                }
                let seg_py = to_pinyin_plain(&seg);
                if seg_py == *hw_py {
                    // 同音：替换为热词。
                    out.extend(hw.chars());
                    i += n;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            out.push(chars[i]);
            i += 1;
        }
    }
    out.into_iter().collect()
}

fn correct_homophones(s: &str, hotwords: &[String]) -> String {
    // 1) 热词同音纠错：把与热词读音相同的片段替换为热词。
    let after_hw = correct_hotword_homophones(s, hotwords);

    // 2) 固定字典逐字纠错（通用高频同音，如 德→得）。
    //    含热词的句跳过，避免误改热词内部字。
    let lower = after_hw.to_lowercase();
    let hit_hotword = hotwords.iter().any(|w| lower.contains(&w.to_lowercase()));
    if hit_hotword {
        return after_hw;
    }
    let mut chars: Vec<char> = after_hw.chars().collect();
    let orig_chars = chars.clone();
    let mut changed = false;

    // 逐字扫描：若该字在小字典里，且拼音无调与候选之一同音，则结合二字上下文投票
    for i in 0..chars.len() {
        let ch = orig_chars[i];
        let cands = match homophone_candidates(ch) {
            Some(c) => c,
            None => continue,
        };
        // 热词内部的字不改（已在外层跳过整句，这里二次保险：若该字前后与热词片段相邻也不改）
        // 简化：已热词命中整句跳过，这里不会走到

        // 二字窗口：看左右邻字组成的二字词是否在常用语境下更偏向候选
        // 保守策略：仅在“候选字+右邻”或“左邻+候选字”能组成热词/常用搭配时才改
        // 首轮最小实现：仅处理一个最可信的锚点 —— "德→得" 在"做/觉得/变得/获得"语境
        if ch == '德' {
            // "做德"、"觉得德"、"变得德"、"获得德" 等：德后常跟"很/好/多/不错/很"等
            let left = if i > 0 { orig_chars[i - 1] } else { '\0' };
            let right = if i + 1 < orig_chars.len() {
                orig_chars[i + 1]
            } else {
                '\0'
            };
            // 德 前是 做/觉/变/得/获 等，且后一位是 很/好/多/不/很 等 → 判定为"得"
            let left_is_verb = ['做', '觉', '变', '获', '觉'].contains(&left);
            let right_is_adj = ['很', '好', '多', '不', '很', '太', '挺', '真'].contains(&right);
            if left_is_verb || right_is_adj {
                chars[i] = '得';
                changed = true;
                continue;
            }
        }
        let _ = cands;
    }

    if changed {
        // 同音字典目前极小，未命中 hotword 的句子不做无锚点替换，直接返回
        // 避免引入新错误；已改的"德→得"是锚点可信的
        fix_de_usage(&mut chars, hotwords);
        chars.into_iter().collect()
    } else {
        s.to_string()
    }
}

// ── 7) 去末尾标点 ────────────────────────────────────────

/// 去掉末尾的常见标点（，。 等）：单句输入到聊天框时句末标点违和（B4）。
/// 句中标点保留，仅去末尾连续标点。
fn strip_trailing_punct(s: &str) -> String {
    let t = s.trim_end();
    let mut chars: Vec<char> = t.chars().collect();
    while let Some(&c) = chars.last() {
        if COMMON_PUNCTS.contains(&c) {
            chars.pop();
        } else {
            break;
        }
    }
    chars.into_iter().collect()
}

// ── 7) 截断检测 ──────────────────────────────────────────

/// 开放性收尾词：以这些词结尾的句子疑似被截断（如"我觉得"、"如果"、"然后"）。
/// `detect_truncation` 据此打 flag（在去末尾标点之前检测，故"我觉得。"不会被误判）。
const OPEN_TAILS: &[&str] = &["想", "觉得", "然后", "因为", "如果", "虽然", "但是"];

/// 是否以开放性收尾词结束（trim 后）。
fn is_open_tail(s: &str) -> bool {
    let t = s.trim_end();
    OPEN_TAILS.iter().any(|tail| t.ends_with(tail))
}

fn detect_truncation(s: &str) -> bool {
    let t = s.trim();
    if t.chars().count() < 10 {
        return false;
    }
    let last = t.chars().last().unwrap();
    if COMMON_PUNCTS.contains(&last) || SENTENCE_PARTICLES.contains(&last) {
        return false;
    }
    is_open_tail(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_fillers_head() {
        assert_eq!(trim_fillers("嗯那个你好"), "你好");
        // 前缀「额」是填充词；句首语气词也应剥（语气价值只在句末）
        assert_eq!(trim_fillers("额明天见"), "明天见");
        // 句末语气词保留：明天见啊 不剥
        assert_eq!(trim_fillers("明天见啊"), "明天见啊");
    }

    #[test]
    fn trim_fillers_tail_particle_kept() {
        // 句末语气词保留
        assert_eq!(trim_fillers("明天见啊"), "明天见啊");
        assert_eq!(trim_fillers("是嘛"), "是嘛");
    }

    #[test]
    fn trim_fillers_all_filler_fallback() {
        // 全填充词不删空
        let r = trim_fillers("嗯那个");
        assert!(!r.is_empty());
    }

    #[test]
    fn collapse_repeated() {
        assert_eq!(collapse_repeated_fillers("那个那个你好"), "那个你好");
        assert_eq!(collapse_repeated_fillers("就是就是这样"), "就是这样");
    }

    #[test]
    fn collapse_keeps_legal_reduplication() {
        // 合法叠词不塌陷
        assert_eq!(collapse_repeated_fillers("慢慢来"), "慢慢来");
        assert_eq!(collapse_repeated_fillers("哥哥来了"), "哥哥来了");
    }

    #[test]
    fn normalize_punct_dup() {
        assert_eq!(normalize_punct("你好。。"), "你好。");
        assert_eq!(normalize_punct("你好，，世界"), "你好，世界");
    }

    #[test]
    fn strip_trailing_punct_removes_end_punct() {
        assert_eq!(strip_trailing_punct("你好。"), "你好");
        assert_eq!(strip_trailing_punct("你好。。"), "你好"); // 连续标点都去
        assert_eq!(strip_trailing_punct("你好，世界。"), "你好，世界"); // 仅末尾，句中保留
        assert_eq!(strip_trailing_punct("你好"), "你好"); // 无标点不变
    }

    #[test]
    fn correct_l0_empty_passthrough() {
        let r = correct_l0("  ", &[]);
        assert!(!r.had_correction);
        assert!(!r.truncation_flag);
    }

    #[test]
    fn correct_l0_filler_and_punct() {
        let r = correct_l0("嗯那个你好。。", &[]);
        assert!(r.had_correction);
        // 首尾填充去除 + 多标点归一
        assert!(!r.text.starts_with('嗯'));
        assert!(!r.text.contains("。。"));
    }

    #[test]
    fn correct_l0_truncation_detection() {
        // 短句（<10 字）不标截断。
        let r = correct_l0("我想去", &[]);
        assert!(!r.truncation_flag);
        // 句末"然后"属填充词，会被首尾修剪剥离，故不触发截断；
        // 截断检测的正面用例（"觉得"等非填充收尾词）见 truncation_open_tail_flagged。
        let r2 = correct_l0("我觉得这个方案如果能够再完善一下然后", &[]);
        assert!(!r2.truncation_flag);
    }

    #[test]
    fn correct_l0_hotword_protects() {
        // 热词命中时不同音纠
        let r = correct_l0("做德很好", &["做德".into()]);
        assert_eq!(r.text, "做德很好"); // 热词命中，跳过同音纠
    }

    #[test]
    fn correct_homophone_de() {
        let r = correct_l0("做德很好", &[]);
        assert_eq!(r.text, "做得很好");
    }

    #[test]
    fn dedupe_not_double() {
        // 同一句重复不应在 L0 制造新重复（L0 不引入 A+A）
        let r = correct_l0("你好", &[]);
        assert_eq!(r.text, "你好");
    }

    // ── 补充覆盖（TDD）：中间填充 / 同音保守边界 / 截断 / 标点 ──────────

    #[test]
    fn mid_single_filler_with_punct_removed() {
        // 中间被标点夹住的孤立"嗯"应删（左右皆为软分隔）。
        let r = correct_l0("你好，嗯，世界", &[]);
        assert!(
            !r.text.contains('嗯'),
            "中间孤立填充词应删除，得到 {}",
            r.text
        );
        assert!(r.had_correction);
    }

    #[test]
    fn mid_filler_without_separator_kept() {
        // 无软分隔夹拥时不动"嗯"，避免误删正常语流。
        let r = correct_l0("你好嗯啊", &[]);
        assert!(
            r.text.contains('嗯'),
            "无分隔的中间填充不应误删，得到 {}",
            r.text
        );
    }

    #[test]
    fn homophone_only_de_implemented_others_passthrough() {
        // 同音字典目前仅实现"德→得"；其余同音字（如 载）保守透传，绝不引入新错。
        // 这条测试锁定 L0 的安全边界：未实现的同音字保持原样，且不误报 had_correction。
        let r = correct_l0("下载完成", &[]);
        assert_eq!(r.text, "下载完成");
        assert!(!r.had_correction, "未实现的同音字不应触发 had_correction");
    }

    #[test]
    fn truncation_open_tail_flagged() {
        // 以"觉得"收尾的长句：ensure_sentence_end 让位 → 不补句号 → truncation_flag=true。
        // 此前 ensure_sentence_end 会先补句号把信号掩盖，TDD 驱动修复。
        let r = correct_l0("今天的会议内容比较多我有点觉得", &[]);
        assert!(r.truncation_flag, "开放性收尾应标截断，得到 {}", r.text);
    }

    #[test]
    fn no_truncation_when_punctuated() {
        let r = correct_l0("今天的会议内容比较多。", &[]);
        assert!(!r.truncation_flag);
    }

    #[test]
    fn long_sentence_no_trailing_punct_e2e() {
        // B4：单句输入不补句末标点（CapsWriter trash_punc 风格）。
        let r = correct_l0("今天天气不错但是有点冷", &[]);
        assert!(
            !r.text.ends_with('。'),
            "单句输入不应补句号，得到 {}",
            r.text
        );
    }

    #[test]
    fn punct_english_dup_normalized() {
        let r = correct_l0("你好！！世界", &[]);
        assert!(
            !r.text.contains("！！"),
            "重复感叹号应归一，得到 {}",
            r.text
        );
        assert!(r.had_correction);
    }

    #[test]
    fn collapse_single_filler_repeat_in_sentence() {
        // 句中"嗯嗯"塌陷为单个（合法叠词不受影响）。
        let r = correct_l0("你好嗯嗯世界", &[]);
        let count = r.text.chars().filter(|&c| c == '嗯').count();
        assert_eq!(count, 1, "连续单字填充应塌陷，得到 {}", r.text);
    }

    #[test]
    fn sentence_particle_not_trimmed_e2e() {
        let r = correct_l0("明天见吧", &[]);
        assert_eq!(r.text, "明天见吧");
        assert!(!r.had_correction);
    }

    #[test]
    fn legal_reduplication_kept_e2e() {
        // 合法叠词"慢慢"端到端不被塌陷。
        let r = correct_l0("我想慢慢走过去", &[]);
        assert!(r.text.contains("慢慢"), "合法叠词应保留，得到 {}", r.text);
    }

    // ── 热词同音纠错（方案A）──────────────────────────────

    #[test]
    fn hotword_homophone_corrects_same_pinyin() {
        // 制谱 与 热词 智谱 同音 → 替换。
        assert_eq!(
            correct_hotword_homophones("我在制谱工作", &["智谱".into()]),
            "我在智谱工作"
        );
    }

    #[test]
    fn hotword_homophone_keeps_exact_hotword() {
        // 已是热词本身，不重复纠。
        assert_eq!(
            correct_hotword_homophones("智谱很好", &["智谱".into()]),
            "智谱很好"
        );
    }

    #[test]
    fn hotword_homophone_no_match_unchanged() {
        assert_eq!(
            correct_hotword_homophones("今天天气不错", &["智谱".into()]),
            "今天天气不错"
        );
    }

    #[test]
    fn hotword_homophone_skips_english() {
        // 英文热词拼音为空，不处理（中英音译不纠）。
        assert_eq!(
            correct_hotword_homophones("用 Paraformer", &["Paraformer".into()]),
            "用 Paraformer"
        );
    }

    #[test]
    fn hotword_homophone_longer_first() {
        // 长热词优先：制谱科技 → 智谱科技（不被短热词"智谱"截断）。
        assert_eq!(
            correct_hotword_homophones("我在制谱科技工作", &["智谱".into(), "智谱科技".into()],),
            "我在智谱科技工作"
        );
    }

    #[test]
    fn correct_l0_uses_hotword_homophone() {
        // 端到端：L0 接入热词同音纠错。
        let r = correct_l0("我在制谱工作", &["智谱".into()]);
        assert!(
            r.text.contains("智谱"),
            "L0 应通过热词纠同音，得到 {}",
            r.text
        );
        assert!(r.had_correction);
    }
}
