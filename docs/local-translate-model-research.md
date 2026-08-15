# 调研：本地专属翻译小模型选型

> 状态：**调研冻结（2026-08-15）**，供设计评审拍板。未改代码。
> 范围：在现有「云端 `translate_text`」之上，植入一台**专属本机翻译小模型**；运行时与下载管线**镜像**已落地的润色模型 `Qwen2.5-1.5B-Instruct Q4_K_M`。
> 对齐：[`docs/phase2-local-llm-research.md`](phase2-local-llm-research.md)（润色 ADR）、[`docs/p1-design.md`](p1-design.md) R4/R5、`polish/local.rs` + `model_download.rs`。

---

## 0. 一句话结论

**可以，而且应该做「专属翻译 GGUF」，不要把现有润色 1.5B 当默认翻译引擎。**

默认推荐锁定两档，都走现有 `llama.cpp` + GGUF + `model_mgr` 设计：

| 角色 | 模型 | 量化 | 体积 | 许可 | 为什么 |
|---|---|---|---|---|---|
| **默认推荐** | **tencent/HY-MT1.5-1.8B** | Q4_K_M | **1.13 GB** | 混元社区许可（可商用，有条款） | 专为端侧实时翻译设计；官方 GGUF；覆盖 openIME 全部 7 语 + 粤语/繁中；支持术语干预（可接热词） |
| **许可更干净的并列首选** | **xiaomi-research/MiLMMT-46-1B-v1.0** | Q4_K_M | **~1.01 GB** | Gemma | 2026-08 刚发布的 1B 专翻 SOTA；46 语；Gemma3 架构 llama.cpp 更熟 |
| **零增量兜底** | 现有 **Qwen2.5-1.5B-Instruct** | Q4_K_M | 已装 986 MB | Apache-2.0 | 不新下模型；只作离线降级，**不承诺质量**（与 P1 一致） |

**不要**默认走 NLLB / OPUS-MT / CTranslate2：要新运行时，和现有润色栈分叉。  
**不要**默认走 Seed-X 7B / Tower+ 7B+ / HY-MT 7B：体积和延迟不适合「说完即上屏」。

---

## 1. 现状与缺口

P1 翻译已经落地，但是**纯云端**：

```
麦克风 → ASR（本地/云）→ L0 规则 → cloud.LlmClient.translate_text
                                      ↘ 失败则插入原文 + TranslateFailed
```

关键代码：`pipeline.rs::apply_translate` 只认 `deps.cloud`；本地 GGUF 被明确排除。

P1 当时的判断（今天仍然成立一半）：

> 本地 1.5B GGUF 做翻译 / QA 的质量承诺（允许作离线降级，默认不走）。
> `prefix` / `Translate` / `QA` **永不**调用 `PolishRouter::polish`（避免 PreferLocal 把「邮件:」喂给 1.5B）。

也就是说：**基础设施已经按「再塞一个 GGUF」预留了形状**，缺的是「翻译专用权重 + 路由」，不是从零做推理。

现成可复用：

| 积木 | 位置 | 翻译侧怎么用 |
|---|---|---|
| `LlmClient::translate_text` | `polish/llm.rs` | 给本地实现一个 `LocalGgufTranslate` 即可 |
| `build_translate_messages` | `polish/prompts.rs` | 通用 Instruct 可复用；专翻模型要换成官方模板 |
| `LocalGgufPolish` + `run_gguf_completion` | `polish/local.rs` | 抽出共享 runner；`n_predict` 从 128 提到 256–512 |
| `polish_model_files` / SHA256 / hf-mirror | `model_download.rs` | 平行加 `translate_model_files()` |
| 设置页模型卡片 | `Settings.tsx` | 复制「本地润色模型」那一块 |

---

## 2. 任务画像（决定「什么模型够用」）

openIME 的翻译不是文档 MT，是**语音短句上屏**：

| 维度 | 约束 | 含义 |
|---|---|---|
| 输入 | ASR final，通常 5–80 字，口语、可能有识别噪声 | 需要扛住 ASR 脏输入；文学/长文档不是主场 |
| 方向 | UI 闭集 `zh / en / ja / ko / fr / de / es`，任意→目标语 | 必须 many-to-one，最好 many-to-many |
| 延迟 | 说完到上屏；润色目标 p95 400–800ms，翻译可略宽到 **≤ 1.2s** | 参数量压在 **≤ 2B Q4** |
| 内存 | 已可能同时驻留 sherpa ASR（0.5–1GB）+ 润色 1.5B（~1.2GB） | 翻译模型峰值建议 **≤ 1.5GB**；**不要和润色同时常驻两个 1B+** |
| 运行时 | Rust / Tauri，已绑 `llama-cpp-2` | **第一优先 GGUF**，不要为翻译再引 CTranslate2 / Python |
| 许可 | 桌面产品、可能分发 | 避开 CC-BY-NC；Apache / Gemma / 混元社区 可谈 |
| 产品叙事 | 已有「本地模型 Qwen2.5-1.5B-Instruct Q4_K_M」 | 翻译卡片应对齐：一键下载、SHA256、hf-mirror、失败回退 |

**结论**：翻译侧要的是 **≤2B 的专翻 decoder-only GGUF**，不是通用聊天，也不是 7B 文档翻译器。

---

## 3. 两条技术路线

```
                    ┌─ A. 通用 Instruct GGUF（复用 / 换代 Qwen）
语音短句 ──翻译──┤
                    └─ B. 专翻 GGUF（HY-MT / MiLMMT / GemmaX2 / Tower+）
```

encoder-decoder（NLLB / OPUS / MADLAD / Bergamot）记为路线 C，**本期不作为默认**。

### 3.1 路线 A：通用 Instruct（Qwen 系）

优点：零新运行时、许可干净、和润色同族、可「一套权重两用」。  
缺点：1–2B 通用模型的翻译是「顺便会」，不是「专门会」。小模型常见失败模式：

1. 指令泄漏（输出 `Sure, here is the translation:`）
2. 乱加引号 / Markdown
3. 日韩方向质量掉得比中英更狠
4. 哨兵合成（`translate_with_polish`）几乎必然解析失败
5. Qwen3 默认 thinking，短句翻译会被 `<think>` 吃掉几百 ms

P1 把 1.5B 排除出翻译默认路径，依据的就是这些。今天的公开评测仍然支持这个判断：Xiaomi 2026-02 的 MiLMMT 论文在 FLORES+ / WMT24++ 上系统测了 Qwen2.5 / Qwen3 小模型——**同尺寸专翻模型明显优于通用 Instruct**；Qwen3-1.7B 比 Qwen2.5-1.5B 好一档，但仍远不是 1B 专翻的对手。

**路线 A 的合理定位：离线兜底，不是默认引擎。**

### 3.2 路线 B：专翻 decoder-only GGUF（推荐）

同一套 llama.cpp，换一组「只干翻译」的权重。2025–2026 这一档突然变密：

| 模型 | 参数 | 语种 | Q4_K_M | 训练配方 | llama.cpp | 许可 |
|---|---|---|---|---|---|---|
| **HY-MT1.5-1.8B** | 1.8B | 33 + 粤/藏/蒙/维 | **1.13 GB（官方仓）** | 腾讯混元，WMT25 冠军蒸馏到端侧 | 官方给了 llama-cli 示例；架构名 `hunyuan-dense` | 混元社区许可 |
| **MiLMMT-46-1B-v1.0** | 1.0B | **46**（含简/繁/粤） | ~1.01 GB（mradermacher） | Gemma3-1B + 143B token CPT + SFT + RL + merge | Gemma3，生态熟 | Gemma |
| GemmaX2-28-2B-v0.2 | ~2.6B 有效 | 28 | ~1.6–1.8 GB | 小米上一代，Gemma2-2B CPT+SFT | 熟 | Gemma |
| Tower+ 2B | ~2.6B 有效 | 22 | ~1.6 GB | Unbabel，翻译+通用 | 熟 | **CC-BY-NC-4.0** |
| Qwen3-1.7B | 1.7B | 100+（通用） | 1.11 GB | 通用 Instruct，官方宣传含翻译 | 极熟 | **Apache-2.0** |

### 3.3 路线 C：经典 NMT（本期不做默认）

| 模型 | 体积 | 质量（中英短句） | 运行时 | 卡点 |
|---|---|---|---|---|
| OPUS-MT / Bergamot | 每方向 20–80 MB | 中英可用，日韩一般 | Marian / CTranslate2 | 7 语双向 ≈ 几十个包；要新引擎 |
| NLLB-200-distilled-600M | CT2 int8 ~1GB，内存 ~2.5–3GB | 中→英尚可，英→中弱 | CTranslate2 | **CC-BY-NC**；新运行时 |
| NLLB-3.3B | 内存 13–16GB | 明显更好 | CTranslate2 | 体积直接出局 |
| MADLAD-400-3B | 全精度很大，Q4 可压到 ~2GB | 中→英较好 | T5 系，llama.cpp 支持一般 | 延迟高；不是「按键即输」 |

2025-11 的独立中英人工盲测（WhyNotHugo）：短技术文本 OPUS / NLLB-600M 都「能用」；文学/隐喻全体翻车；Tower 最好但 7B 太慢。这和 openIME 的短句场景对得上——**经典 NMT 能交差，但专翻小 LLM 已经把这一档质量/体积比吃掉了**。

---

## 4. 短名单深挖

### 4.1 HY-MT1.5-1.8B —— 默认推荐

腾讯 2025-12-30 开源。产品口径几乎是为 openIME 写的：

- 量化后可跑在端侧；官方数字：**约 50 个中文 token 平均 ~180ms**
- 同尺寸号称超过多数商用翻译 API
- **官方 GGUF 仓** `tencent/HY-MT1.5-1.8B-GGUF`，Q4_K_M = 1.13 GB
- 33 语 + 繁中 / 粤语 / 藏 / 蒙 / 维，**覆盖 UI 全部 7 语**
- 原生支持 **术语干预**（直接接 openIME 热词表）、上下文翻译、格式保留
- Prompt 极简，专门要求「只输出译文」——比通用 Instruct 更抗泄漏

中英 prompt（可直接替换 `build_translate_messages`）：

```
将以下文本翻译为{target_language}，注意只需要输出翻译后的结果，不要额外解释：

{source_text}
```

术语干预（`translate_with_polish` 之外的「热词保真」）：

```
参考下面的翻译：
{source_term} 翻译成 {target_term}

将以下文本翻译为{target_language}，注意只需要输出翻译后的结果，不要额外解释：
{source_text}
```

**风险（必须在立项前验证）：**

1. GGUF 架构字段是 `hunyuan-dense`。当前 `llama-cpp-2 = 0.1` 捆绑的 llama.cpp **不一定认**。落地前要用现有 runner 加载一次 Q4_K_M；不行就升级绑定，或把默认改成 4.2。
2. 混元社区许可不是 Apache：桌面分发一般可以，但要法务扫一遍再写进 README。
3. 官方推荐采样 `temp=0.7 / top_p=0.6`。语音短句建议我们改成 **greedy 或 temp≤0.3**，减少抖译。

### 4.2 MiLMMT-46-1B-v1.0 —— 并列首选（许可 / 架构更稳）

小米 2026-08 刚发（论文 arXiv:2608.10812，CPT 论文 arXiv:2602.11961）。Gemma3-1B 连续预训练 143B token → SFT → RL → merge。

为什么值得并列：

- **1.0B / ~1GB**，比 HY-MT 还轻，和现有润色模型几乎同体积
- 46 语，**简中 / 繁中 / 粤语分列**，和 openIME 的 `ChineseScriptPreference` 能对上
- 论文主表：1B 专翻在 WMT24++ / FLORES+ 上压过一批 7B 通用模型和老一代 2B 专翻
- 架构是 Gemma3，llama.cpp 支持面比 `hunyuan-dense` 宽
- Prompt 固定、无 chat 角色，适合 `n_predict` 卡死

```
Translate this from Chinese (Simplified) to English:
Chinese (Simplified): 我爱机器翻译
English:
```

**风险：**

1. 许可是 **Gemma**（不是 Apache）。商用桌面应用通常允许，但有 Google 使用政策。
2. 官方不直接发 GGUF，目前靠 mradermacher 量化。要像润色一样锁 SHA256，并准备 hf-mirror。
3. Gemma3 词表 256k，**同样 20 字中文，tokenize 比 Qwen 略长**。短句上差异不大，但 `n_ctx=2048` 够，不要照抄 32k。
4. 发布极新（数天），社区 rumble 少，需要我们自己做 7 语回归集。

### 4.3 现有 Qwen2.5-1.5B-Instruct —— 零成本兜底

已经在 `POLISH_GGUF_FILE` 里。复用方式：`LocalGgufPolish` 换 `build_translate_messages` + `n_predict=256`。

只适合：

- 用户没下翻译模型、也没配云端 key 时的离线降级
- 中英短句、术语不敏感的场景

不适合：

- 作为设置页「翻译模型」的默认宣传型号
- `translate_with_polish` 哨兵合成（1.5B 几乎解不出 `[[OPENIME_*]]`）
- 日 / 韩 / 德长句

P1 的「不承诺质量」继续有效。

### 4.4 Qwen3-1.7B —— Apache 保底档

如果评审要求「必须 Apache-2.0 + 官方 GGUF + 和润色同族」：

- `unsloth/Qwen3-1.7B-GGUF` Q4_K_M = **1.11 GB**
- 官方明确写了 100+ 语种和 **translation**
- 同尺寸比 Qwen2.5-1.5B 强，但仍是通用模型

硬约束：**推理必须关 thinking**。`enable_thinking=false` 或在 system/user 两端打 `/no_think`。否则 20 字翻译会先吐一段思维链，延迟和「只输出译文」全部炸掉。更稳的是直接用 **Qwen3-1.7B-Instruct-2507**（纯非思考），如果当时 GGUF 齐。

### 4.5 明确不选（默认路径）

| 方案 | 原因 |
|---|---|
| Seed-X-PPO-7B Q4_K_M（4.3 GB） | 质量很好，7B 不是按键即输 |
| HY-MT1.5-7B / MiLMMT-46-4B / 12B | 高质量可选项，默认不装 |
| Tower+ 2B | 质量强，**CC-BY-NC** |
| NLLB-200 全家 | 要 CTranslate2；600M 还是 NC |
| OPUS-MT 多对模型 | 引擎分叉 + 组合爆炸 |
| 复用润色 1.5B 当默认翻译 | 产品会把「本地翻译很蠢」记在 openIME 头上 |
| 外挂 Ollama | 和润色 ADR 同一结论：开发可以，产品默认不行 |

---

## 5. 和「Qwen2.5-1.5B Q4_K_M」设计怎么对齐

目标：用户在设置页看到的交互，和现在的「本地润色模型」卡片**同一套心智**。

```
app_data_dir/models/
  asr/          # 已有
  vad/          # 已有
  llm/
    Qwen2.5-1.5B-Instruct-Q4_K_M.gguf          # 已有，润色
    HY-MT1.5-1.8B-Q4_K_M.gguf                  # 新增，翻译（或 MiLMMT）
```

| 润色（已有） | 翻译（建议） |
|---|---|
| `POLISH_GGUF_FILE` / `POLISH_MODEL_ID` | `TRANSLATE_GGUF_FILE` / `TRANSLATE_MODEL_ID` |
| `install_polish_model` | `install_translate_model`（复用同一套 Range + SHA256 + hf-mirror） |
| `LocalGgufPolish` | `LocalGgufTranslate`（共享 `run_gguf_completion`） |
| `n_ctx=2048, n_predict=128, temp=0.3` | `n_ctx=2048, n_predict=256~512, greedy / temp≤0.3` |
| `PolishRouter` PreferLocal | `TranslateRouter`：**默认 PreferCloud**，LocalOnly / PreferLocal 可选 |
| 设置页一键下载 ~986MB | 设置页一键下载 ~1.1GB |

### 5.1 路由（不要抄润色的 PreferLocal）

翻译的质量预期比润色高——用户能立刻看出来「译错了」。建议默认：

```
TranslatePolicy
  PreferCloud   # 默认：有 key 走云；失败 / 超时 / 无网 → 本地专翻 → 原文
  PreferLocal   # 隐私档：本地专翻优先，失败才云
  LocalOnly
  CloudOnly     # 现状
```

`translate_with_polish=true`：

- **云端**：保持现有哨兵合成
- **本地**：拆成两步——先 `LocalGgufPolish`，再 `LocalGgufTranslate`。**禁止**让 1B 专翻模型解析 `[[OPENIME_TRANSLATION]]`

### 5.2 内存：按「ASR + 润色 + 翻译」三件套算

听写润色和翻译快捷键会来回切，**默认三套权重同时常驻**，不要靠卸载省内存（冷启动比多占 1GB 更伤）。账本和标签算法见 [`local-polish-model-catalog.md`](local-polish-model-catalog.md) §3。

粗算（RSS）：

| 机器 | 三件套预算 | 默认可同时常驻 |
|---|---|---|
| 8GB | ~2.5 GB | SenseVoice + 极速润色；翻译走云 |
| M4 16GB | ~10 GB | SenseVoice/FunASR + 2B 润色 + HY-MT 1.8B |
| M4 Pro 48GB | ~38 GB | 最重 ASR + 9B + HY-MT 也只是延迟问题 |

1. llama context **常驻**（现在 `LocalGgufPolish` 每次 `load_from_file`，翻译绝不能再这么干）
2. 只有 `combo > budget` 才降级：卸翻译或提示改云端，而不是在快捷键之间来回加载
3. 8GB：设置页直接建议翻译用云，别和润色抢

### 5.3 源语言

专翻模型的 prompt 要源语名字。来源优先级：

1. 本次 ASR 的 `language`（sherpa / SenseVoice 已有）
2. 简单脚本检测（CJK vs Latin vs Hangul vs Kana）——几十行，别再挂一个 lid 模型
3. 未知则只给目标语（HY-MT 的 zh↔xx 模板本来就不强制源语）

### 5.4 失败语义（沿用 FR-4.3）

本地超时 / 空输出 / 明显指令泄漏（译文以 `Sure` / `翻译如下` 开头）→ 视为失败 → 云（若允许）→ 原文 + `TranslateFailed`。不要把垃圾译文上屏。

---

## 6. 质量预期（对用户怎么说）

| 场景 | 云端（现状） | 1.5B 润色模型硬上 | HY-MT 1.8B / MiLMMT 1B |
|---|---|---|---|
| 中↔英 20 字口语 | 好 | 能用，偶发泄漏 | **接近可用产品** |
| 中→日 / 中→韩 | 好 | 明显掉 | 专翻明显好于通用 1.5B |
| 专有名词 / 热词 | prompt 碰运气 | 差 | HY-MT 术语模板可接词典 |
| 先润色再译 | 哨兵一次调用 | 基本失败 | 两步本地，质量取决于润色 |
| 无网 | 失败回原文 | 有 | **这才是做本地翻译的理由** |

不要对外承诺「比肩 DeepL / 百炼」。对外一句话：

> 没网也能把刚说的话译到光标处；有网默认仍走云端，译得更稳。

---

## 7. 建议实施切片

| PR | 内容 | 依赖 |
|---|---|---|
| **T0 调研冻结** | 本文；默认模型二选一拍板 | — |
| **T1 共享 runner** | 把 `run_gguf_completion` 从 polish 抽出；**模型常驻**（全局 / OnceLock）；补加载失败测试 | `llm` feature |
| **T2 本地 translate** | `LocalGgufTranslate` 实现 `LlmClient::translate_text`；专用 prompt；`n_predict=256` | T1 |
| **T3 下载卡** | `TRANSLATE_GGUF_*` + SHA256 + hf-mirror；设置页第二张模型卡 | 现有 model_mgr |
| **T4 路由** | `TranslateRouter` + `translate_policy`；pipeline 不再 `cloud-only` | T2 |
| **T5 热词** | HY-MT 术语模板注入 `hotwords`；MiLMMT 则写进 prompt 约束 | T4 |
| **T6 回归** | 固定 40 条短句（7 语×主方向，含 ASR 脏输入、热词、空句） | T4 |

T0 拍板前必须做的 **烟测**（半天）：

1. 现有 `llama-cpp-2` 能否加载 HY-MT Q4_K_M。不能 → 默认改 MiLMMT 或先升 llama.cpp。
2. 同一台 M 系列上，20 字中→英 / 英→中延迟是否 ≤ 800ms（模型已常驻）。
3. 7 语各 3 句，人工看有没有指令泄漏。

---

## 8. 推荐决策（供拍板）

1. **做专属翻译 GGUF**，不要让润色 1.5B 顶默认。
2. **运行时继续 llama.cpp**，下载管线继续 `model_mgr`。镜像润色设计，不引入 CTranslate2。
3. **默认权重**：HY-MT1.5-1.8B Q4_K_M（1.13GB）。若 `hunyuan-dense` 加载失败或许可不过，改 **MiLMMT-46-1B-v1.0 Q4_K_M**。
4. **Apache 保底档**：Qwen3-1.7B（必须关 thinking），写进设置「兼容型号」，不当默认。
5. **策略默认 PreferCloud**，本地是离线 / 隐私 / 云失败的降级。和润色的 PreferLocal 相反，这是有意的。
6. **模型常驻 + 与润色互斥**。现有每次 `load_from_file` 必须先修。
7. **本地不做哨兵合成**。`translate_with_polish` 在本地拆成 polish → translate 两步。

---

## 9. 开放问题

1. 默认 HY-MT 还是 MiLMMT？取决于 T0 烟测的加载结果和许可态度。
2. 翻译模型是否允许用户在设置里换成「我已经下过的润色 1.5B」？建议可以，但打「质量降级」标签。
3. 8GB 机器要不要默认隐藏翻译下载卡？
4. 要不要在第一次按翻译快捷键且无云端 key 时，弹「下载 1.1GB 本地翻译模型」？

---

## 10. 参考

- 仓库内：[phase2-local-llm-research.md](phase2-local-llm-research.md)、[p1-design.md](p1-design.md) §R4、`polish/local.rs`、`model_download.rs`
- [tencent/HY-MT1.5-1.8B-GGUF](https://huggingface.co/tencent/HY-MT1.5-1.8B-GGUF)（Q4_K_M 1.13GB，官方）
- [Tencent-Hunyuan/HY-MT](https://github.com/Tencent-Hunyuan/HY-MT) / [技术报告](https://arxiv.org/html/2512.24092v1)
- [xiaomi-research/MiLMMT-46-1B-v1.0](https://huggingface.co/xiaomi-research/MiLMMT-46-1B-v1.0)
- [Scaling Model and Data for Multilingual MT](https://arxiv.org/html/2602.11961v2)（含 Qwen2.5/Qwen3 小模型 FLORES+/WMT24++ 表）
- [Reference-Free Post-Training … MiLMMT v1.0](https://arxiv.org/abs/2608.10812)
- [Unbabel/Tower-Plus-2B](https://huggingface.co/Unbabel/Tower-Plus-2B)（NC，仅作对照）
- [Qwen3-1.7B-GGUF](https://huggingface.co/Qwen/Qwen3-1.7B-GGUF) / [关 thinking](https://qwenlm.github.io/blog/qwen3/)
- [中英本地模型盲测](https://whynothugo.nl/journal/2025/11/02/translation-models-between-english-and-chinese/)（OPUS / NLLB / MADLAD / Tower）
- [NLLB-200 distilled 600M + CTranslate2](https://huggingface.co/entai2965/nllb-200-distilled-600M-ctranslate2)
- [Firefox Bergamot 端侧蒸馏](https://hacks.mozilla.org/2022/06/training-efficient-neural-network-models-for-firefox-translations/)
