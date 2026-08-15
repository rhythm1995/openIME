# 本地三件套：需求方案 + 技术方案

> 状态：需求已拍板；技术方案按现有 `voice-core` / Tauri 薄壳落地。  
> 日期：2026-08-15  
> 调研底稿：[`local-polish-model-catalog.md`](local-polish-model-catalog.md)、[`local-translate-model-research.md`](local-translate-model-research.md)。

---

# 一、需求方案

## 已拍板

| # | 决定 |
|---|---|
| 1 | 译前润色只用 Light（纠 ASR），不跟听写 Heavy |
| 2 | 小模型兼译一律两步（Light → 译），不做一步合成 |
| 3 | 弱机即使有云 key 也提示兼译；有网默认仍走云 |
| 4 | FireRed 直接下架，不做迁移 |
| 5 | 高配（M4 Pro 48GB 等）默认润色 Qwen3.5-4B |
| 6 | 专翻默认 MiLMMT-1B；HY-MT 1.8B 自选 |
| 7 | 本地模型区要有「打开模型下载目录」按钮 |

## 1. 冻结目录

| 层 | 档 | id | 常驻约 | 默认落点 |
|---|---|---|---|---|
| ASR | 轻 | `sensevoice` | 0.7 GB | 弱机 / 16GB 默认 |
| | 中 | `funasr-nano-int8` | 1.2 GB | 16GB 可选 |
| | 重 | `funasr-nano-fp16` | ~2.0 GB | 高配可选 |
| 润色 | 极速 | `qwen3.5-0.8b` | 0.6 GB | 弱机默认；可兼译 |
| | 均衡 | `qwen3.5-2b` | 1.5 GB | 16GB 默认 |
| | 高质量 | `qwen3.5-4b` | 2.8 GB | **48GB 默认** |
| 翻译 | 默认专翻 | `milmmt-1b` | 1.1 GB | **有预算时默认** |
| | 可选专翻 | `hy-mt-1.8b` | 1.4 GB | 术语/端侧自选 |

不进目录：Qwen2.5-1.5B、Qwen3.5-9B、FireRed。

`qwen35` 若当前 `llama-cpp-2` 加载失败：同档静默落到 Qwen3-0.6B / 1.7B / 4B-Instruct-2507，设置页仍显示 3.5 档名。

## 2. 三件套同驻 + 机型推荐

默认同驻。预算：8GB→2.5 / 16GB→10 / 32GB→24 / 48GB→38。

| 机器 | ASR | 润色 | 翻译 |
|---|---|---|---|
| ≤8GB / 低配 | SenseVoice | 0.8B | 不下专翻，勾兼译；有云也提示 |
| M4 16GB | SenseVoice | 2B | MiLMMT |
| M4 Pro 48GB | Nano int8 或 fp16 | **4B** | **MiLMMT**（HY-MT 绿标自选） |

标签看 `combo = ASR + 这张卡 + 另一层已选` + 估测 TPS。红卡不拦截。默认推荐线：40 token ≤ 1.0s。

## 3. 润译顺序

```
L0 → [translate_with_polish?] Light 源语纠错 → 译（云 / 专翻 / 兼译）→ 上屏译文
```

听写 Light/Heavy 与翻译热键无关。云端仍一次哨兵；本地禁止哨兵、必须两步。Light 失败则跳过，仍译 L0。

## 4. 弱机兼译

专翻装不下或标红 → 默认勾「用润色模型兼做翻译」。同一颗 Qwen3.5，两步。有云也显示这行；默认 PreferCloud。

```
PreferCloud（默认）: 云 → 专翻 → 兼译 → 原文
PreferLocal:         专翻 → 兼译 → 云 → 原文
```

## 5. 本地模型区交互

共用工具条：打开 `{app_data}/models`、显示路径、三件套预算条、重新采集。

## 明确不做

不保留 1.5B；不默认 9B；不引入 CTranslate2/NLLB；不在快捷键间卸载模型；本期不做本机 llama-bench 校准。

---

# 二、技术方案

## T0. 现状缺口

| 现状 | 问题 |
|---|---|
| `asr_catalog` 仍含 FireRed；Settings 前端 fallback 还有 zipformer 死数据 | 与冻结目录不符 |
| 润色写死 `POLISH_GGUF_FILE` = Qwen2.5-1.5B | 无法换 0.8/2/4B |
| `LocalGgufPolish` 每次 `LlamaModel::load_from_file` | 三模型同驻后不可接受 |
| `apply_translate` 只认 `deps.cloud` | 无本地专翻 / 兼译 |
| `LlmClient` 仅云端实现 | 本地要同一 trait |
| `compute_model_tag` 按单卡体积、1.1GB 切重型 | 不能表达三件套 + TPS |
| 无打开目录命令 | 需求 7 |

原则：ASR 下载/打标/设置列表的形状复用；润色与翻译做成第二、第三份 catalog，推理共用一个常驻 GGUF 运行时。

---

## T1. 磁盘布局

```
{app_data}/models/
  vad/silero_vad.onnx
  sherpa-onnx-sense-voice-…/
  sherpa-onnx-funasr-nano-int8-2025-12-30/
  sherpa-onnx-funasr-nano-fp16-2025-12-30/
  llm/
    Qwen3.5-0.8B-Q4_K_M.gguf          # 或回退 Qwen3-0.6B-…
    Qwen3.5-2B-Q4_K_M.gguf
    Qwen3.5-4B-Q4_K_M.gguf
    MiLMMT-46-1B-v1.0-Q4_K_M.gguf
    HY-MT1.5-1.8B-Q4_K_M.gguf
```

下载：现有 `LocalModelFile`（多 URL、SHA256、Range、hf-mirror）。每个 catalog 条目一组 files。旧 `Qwen2.5-1.5B-Instruct-Q4_K_M.gguf` 不再引用；不主动删盘。

---

## T2. Catalog 数据（voice-core）

新增 `crates/voice-core/src/llm_catalog.rs`，形状对齐 `AsrModelInfo`：

```rust
pub struct LlmModelInfo {
    pub id: &'static str,            // qwen3.5-0.8b / milmmt-1b …
    pub kind: LlmKind,               // Polish | Translate
    pub title: &'static str,
    pub description: &'static str,
    pub file_name: &'static str,     // 首选 GGUF 文件名
    pub fallback_id: Option<&'static str>, // qwen3-0.6b 等
    pub approx_size: u64,            // 文件字节
    pub rss_bytes: u64,              // 常驻估算（打标用）
    pub n_predict: i32,              // polish 128/256；translate 256
    pub arch_hint: LlmArch,          // Qwen25 | Qwen3 | Qwen35 | Gemma3 | Hunyuan
}

pub fn polish_catalog() -> &'static [LlmModelInfo];
pub fn translate_catalog() -> &'static [LlmModelInfo];
pub fn llm_files(id: &str) -> Vec<LocalModelFile>; // 含 fallback 解析
```

分辨率：`resolve_llm_id(id) -> ResolvedGguf`：

1. 首选文件存在且 `llm` feature 能加载该 `arch_hint` → 用首选。
2. 否则若有 `fallback_id` 且文件在 → 用回退。
3. 否则「未安装」，下载按钮下首选（或用户已点过回退则下回退）。

启动时做一次「探测加载」（只对已下载的首选，失败记 `arch_unsupported`），避免每次录音试错。

`asr_catalog`：删 `firered-large` 条目与 `OfflineFireRed` 的 catalog 引用。`sherpa.rs` 里 FireRed 连接代码可先留（无 catalog 走不到），避免大删；Settings 前端 fallback 数组删掉 firered/zipformer。

`default_asr_model_id` 仍 `sensevoice`。若配置里残留 `firered-large`：`resolved_local_asr_model()` 归一到 `sensevoice`。

---

## T3. AppConfig / 前端类型

在现有字段上加，全部 `#[serde(default)]`：

```text
polish_local_model          已有；默认改为按机器解析，见 T6
                            合法值：qwen3.5-0.8b | qwen3.5-2b | qwen3.5-4b
translate_local_model       新增；默认 milmmt-1b
                            合法值：milmmt-1b | hy-mt-1.8b | ""（未选）
translate_use_llm_fallback  新增 bool；弱机推荐 true
translate_policy            新增 enum：PreferCloud（默认）| PreferLocal
                            不再拆 LocalDedicated / CloudOnly；
                            没装专翻 + fallback=false + 无云 = 原文
```

`translate_with_polish` 语义改：**本地 = Light 再译**；云端仍哨兵。字段名不改，少迁配置。

`src/types.ts`、`Settings.test.tsx` 的 `defaultConfig` 同步。

默认润色 id **不要写死 4B**：`default_polish_local_model()` 若在 config 反序列化时拿不到 SystemInfo，先落 `qwen3.5-2b`；设置页首次打开 / `get_config` 后由推荐器写成 0.8/2/4（用户手动选过则不再覆盖）。用 settings 表键 `polish_model_user_set=1` 区分。

---

## T4. 常驻 GGUF 运行时（核心）

新模块 `crates/voice-core/src/polish/runtime.rs`（feature `llm`）：

```text
GgufRuntime
  backend: once LlamaBackend
  slots: Mutex<HashMap<PathBuf, LoadedModel>>   // path → model+ctx

LoadedModel
  model: LlamaModel
  ctx:   LlamaContext   // n_ctx=2048
```

- `complete(path, messages, n_predict, timeout) -> String`：已加载则复用；否则 load 后插入 map。
- 三槽上限：润色当前档 + 翻译当前档（兼译则翻译槽空，共用润色 path）。ASR 不进此 map。
- 换档：加载新 path，drop 旧 path。
- chat template：模型内置 → 回退 chatml。
- Qwen3/3.5：**关 thinking**（apply template `enable_thinking=false`；或 system/user 打 `/no_think`；生成后剥 `<think>…</think>`）。
- Qwen3.5：**不加载 mmproj**。
- 采样：polish 维持 temp 0.3；translate greedy / temp≤0.3。
- 调用仍 `spawn_blocking` + `tokio::time::timeout`。
- 无 `llm` feature：与现在一样返回引导错误。

`LocalGgufPolish` 改为持有 `Arc<GgufRuntime>` + path，不再每次 init backend。  
`LocalGgufTranslate` 实现 `LlmClient::translate_text`（`polish` / `chat_stream` 返回不支持）。  
兼译：`LlmClient::translate_text` 指到**同一** polish path。

`AppState`：

```text
gguf_runtime: Arc<GgufRuntime>          // 进程级
local_polish: 从 polish_local_model 解析 path
local_translate: Option<Arc<dyn LlmClient>>  // 专翻已装则 Some；兼译则 clone polish client
```

`invalidate_pipeline` 不卸 GGUF（避免下次录音冷启动）。只在换模型 id 或进程退出时卸。

---

## T5. 翻译路由与 pipeline

新 `polish/translate_router.rs`：

```text
TranslateRouter { policy, cloud, dedicated, llm_fallback, use_llm_fallback }

fn translate(req) -> Result<String>:
  按 policy 试 cloud / dedicated / llm_fallback
  全失败 → Err（pipeline 插 L0 + TranslateFailed）
```

`apply_translate` 改为：

```text
l0 = text
src = l0
if translate_with_polish:
    src = Light polish via deps.local（失败保留 l0，不 abort）
out = deps.translate_router.translate(src)   // 取代只调 cloud
空/失败 → l0 + TranslateFailed
```

前缀 `role_kind=Translate` 走同一 router（不加听写风格包）。

云端 `polish_and_translate` 仅 `policy` 第一跳是云且 `translate_with_polish` 时保留；本地路径永不走哨兵。

`PolishContext` 增加：`translate_policy`、`translate_use_llm_fallback`（薄壳从 config 填）。

超时：译前 Light 用 `polish_timeout_ms`；translate 仍 `llm_timeout()` = max(8000, polish_timeout)。

---

## T6. 推荐器 / 打标

`system.rs` 新增，不改坏现有 ASR `compute_model_tag`（ASR 单卡仍可用；列表展示改用 combo）：

```text
fn os_reserve(total) -> u64
fn combo_budget(sys) -> u64           // total - reserve
fn chip_row(sys) -> TpsRow            // A=M4-16 / B=M5 / C=M4Pro  对不上按内存分桶
fn est_tps(row, polish_or_translate_id) -> f32
fn compute_combo_tag(sys, asr_id, this_id, other_llm_id) -> ModelPerfTag
fn recommend_defaults(sys) -> (asr, polish, translate, use_fallback)
```

判据（与需求一致）：

```
combo = rss(asr)+rss(this)+rss(other)
if combo > budget:            not_recommended
else if tps < 15:             not_recommended
else if tps < 25 or combo>0.85*budget: usable
else:                         suitable
```

`recommend_defaults`：

- ≤8GB 或非 Apple 且 <16GB：sensevoice + 0.8b + translate="" + fallback=true
- 16–31GB Apple：sensevoice + 2b + milmmt + fallback=false
- ≥32GB Apple（含 48GB Pro）：nano-int8 + **4b** + milmmt + fallback=false

TPS 静态表（tok/s，Q4_K_M，关 thinking）：

| id | A M4 16GB | B M5 32GB | C M4 Pro |
|---|---:|---:|---:|
| 0.8B | ~66* | ~85* | ~150* |
| 2B | ~39* | ~50* | ~89* |
| 4B | ~23 | **29** 实测锚 | ~52 |
| milmmt-1b | 按 1.7B 档估 ~29 / 37 / 89 | | |
| hy-mt-1.8b | 略低于 2B | | |

`*` 预测；B 列 4B 及 Qwen3 回退来自 mac-llm-bench。reason 写明「按带宽折算」或「实测锚」。

列表 API：`list_local_polish_models` / `list_local_translate_models` 返回与 `LocalAsrModelEntry` 同形（id/title/desc/size/installed/active/perf_tag/recommended）。`recommended` 在 `suitable` 里按默认档打勾，不是写死在 catalog。

---

## T7. 设置页与 IPC

工具条（识别/润色/翻译三块共用或顶上一份）：

- `open_model_directory`：`create_dir_all(model_root)` + `tauri_plugin_opener::open_path`（插件已在）。返回 path 给 tooltip。
- 小字 `model_root`。
- 「当前三件套约 X / 预算 Y」。
- 已有「重新采集」。

三列卡片：下载 / 启用 / 删除，对齐 ASR。翻译列额外：

- 开关「用润色模型兼做翻译」
- 弱机或专翻标红时默认开，**有云 key 也渲染**，hint：「有网默认走云；离线或不想下载专翻可用润色模型兼译」

`translate_policy` 先不做第三套单选（少 UI）；设置里两个策略足够。云 endpoint 已有。

i18n：`zh.json` / `en.json` 增 openDir、预算条、兼译、三档标题。

---

## T8. Prompt

- 译前 Light：现有 `build_messages(..., Light)`，禁止 style_prompt。
- 专翻：
  - MiLMMT 官方：`Translate this from {src} to {tgt}:\n{src}: {text}\n{tgt}:`
  - HY-MT：中英用「将以下文本翻译为{lang}…只输出译文」；其它方向用英文模板。热词走 HY-MT 术语模板。
- 兼译：现有 `build_translate_messages`（通用 Instruct）。
- 源语：ASR `local_language`，`auto` 则脚本粗分（CJK/Hangul/Kana/Latin）。

---

## T9. 测试

voice-core 单测（不链真实 GGUF）：

- catalog id 闭集；未知 id 归一。
- `firered-large` 配置 → sensevoice。
- `recommend_defaults` 三档内存。
- `compute_combo_tag`：16GB + 4B + fp16 + hy-mt 为黄/红；48GB + 4B + milmmt 为绿。
- `apply_translate`：无 cloud、有 dedicated mock；with_polish 先调 Light 再 translate。
- 兼译：dedicated None + fallback mock。
- 哨兵路径仅 cloud。
- Light 失败仍 translate。

前端：`defaultConfig` 新字段；打开目录按钮 invoke；兼译文案在 AI 页可见。

---

## T10. PR 切片

| PR | 内容 | 风险 |
|---|---|---|
| **P0** | 本文（需求+技术合一） | 无 |
| **P1** | ASR 下架 FireRed + 清 Settings fallback；`open_model_directory` + 打开目录按钮 | 低 |
| **P2** | `llm_catalog` + 下载多 GGUF + SHA256 表；设置三列但推理仍可走旧 1.5B 直到 P3 | 中（URL/SHA 要锁） |
| **P3** | `GgufRuntime` 常驻；`LocalGgufPolish` 改用；换档不每次 load | 高（llm feature / Metal） |
| **P4** | `TranslateRouter` + `apply_translate` 两步 Light；兼译；config 新字段 | 中 |
| **P5** | combo 打标 + 推荐器写默认 + 预算条 + 弱机兼提示 | 低 |

P3 开工前半天烟测：现绑定能否 load `qwen35` / `gemma3` / `hunyuan-dense`。不能则 P2 下载回退档，P3 只跑 Qwen3 + MiLMMT（Gemma3 一般更熟）或先升 `llama-cpp-2`。

---

## T11. 风险

| 风险 | 处理 |
|---|---|
| `qwen35` / `hunyuan-dense` 旧绑定不认 | 回退 Qwen3；HY-MT 延后，默认本就 MiLMMT |
| 三模型 Metal 抢带宽，TPS 低于单卡表 | 标签偏保守；reason 写「与识别/翻译同时驻留」 |
| Gemma / 混元许可 | README 注明；设置卡小字 |
| 0.8B 兼译日韩差 | 文案不承诺；有预算仍推 MiLMMT |
| 旧 `polish_local_model=qwen2.5-1.5b-…` | 读配置时映射到 `qwen3.5-2b`，不读旧文件 |

---

## T12. 关键文件

- 改：`asr_catalog.rs`、`model_download.rs`、`config.rs`、`system.rs`、`pipeline.rs`、`polish/local.rs`、`polish/mod.rs`、`lib.rs`
- 增：`llm_catalog.rs`、`polish/runtime.rs`、`polish/translate_router.rs`
- 薄壳：`commands.rs`、`state.rs`、`lib.rs`
- 前端：`Settings.tsx`、`types.ts`、`ipc.ts`、`en.json`/`zh.json`、对应 test
- 文档：本文即仓库内唯一正本
