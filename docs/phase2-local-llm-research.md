# ⚠️ 已归档：二期本地 LLM 选型 ADR

> **本文是 2026-08-11 的本地 LLM 选型调研 ADR，默认模型 Qwen2.5-1.5B-Instruct Q4_K_M。**
> 该选型已被后续「本地三件套」升级取代（Qwen3.5 三档 + MiLMMT/HY-MT 专翻），当前方案见 [local-model-suite.md](./local-model-suite.md)。本文保留作架构决策追溯。


> 状态：**已落地（2026-08）**，保留作架构决策记录（ADR）——默认模型 Qwen2.5-1.5B-Instruct Q4_K_M、llama.cpp 进程内推理、`PreferLocal` 策略均按本文实施。落地进度见文末 §12。  
> 范围：AI 润色 / 人设 / 热词增强（非 Windows 平台）  
> 日期：2026-08-11  
> 对齐一期：ASR 已具备「本地 sherpa + 云端百炼」双引擎与 `model_mgr` 下载管线

---

## 1. 产品目标（二期核心）

一期管线：

```
麦克风 → ASR（本地/云端）→ 原文 → 插入光标
```

二期在 **final 文本之后、插入之前**（可选）增加文本增强：

```
麦克风 → ASR → [可选] TextPolish（本地小模型 / 云端 LLM）→ 插入光标
                      ↑
               人设 prompt + 热词偏好
```

能力分层：

| 能力 | 说明 | 触发 |
|---|---|---|
| **基础润色** | 去口头禅、补标点、纠明显 ASR 错字、通顺化 | 开关 / 每次 final |
| **人设改写** | 按 persona prompt 调整语气（正式/口语/邮件…） | 选用人设 |
| **热词强化** | 已有 `hotwords` 表；ASR 侧 + 润色侧双重利用 | 词典维护 |
| **云端兜底** | 本地不可用/失败/关闭时走百炼 OpenAI 兼容 API | 自动或用户强制 |

**原则（与用户要求一致）**：

1. **逻辑镜像 ASR**：`TextPolishProvider` 抽象 + 本地/云端两种实现。  
2. **优先本地**：默认本地小模型；未下载/加载失败再云端（可配置）。  
3. **可关**：润色关闭时零开销，管线退回一期行为。  
4. **不绑架体验**：润色延迟必须可感知可控（超时 → 原文直出）。

---

## 2. 任务特征 → 模型需求

| 维度 | 约束 | 含义 |
|---|---|---|
| 输入 | ASR final，通常 5–80 字中文短句 | 不需要 32B 级长文模型 |
| 输出 | 改写后的短句 | 指令遵循 > 百科知识 |
| 延迟 | 目标 p95 **≤ 400–800ms**（M 系列） | 参数量压到 **≤ 1.5–2B** 量化档 |
| 内存 | 与 sherpa ASR 共存（已 ~0.5–1GB 模型+runtime） | 润色模型峰值建议 **≤ 1.5GB** |
| 语言 | 中文为主，中英混输 | 优先 Qwen 系 |
| 隐私 | 本地优先 | 权重本地、默认不上云 |
| 集成 | Rust / Tauri | GGUF + llama.cpp 系最稳 |

**结论**：二期不是「通用聊天助手」，是 **短文本指令改写 SLM**。0.5B–1.7B Instruct 足够；3B+ 仅作「高质量可选」。

---

## 3. 候选模型对比

### 3.1 推荐短名单（按优先级）

| 档位 | 模型 | 体量（约） | 量化内存（约） | 中文/指令 | 许可 | 角色 |
|---|---|---|---|---|---|---|
| **默认推荐** | **Qwen2.5-1.5B-Instruct** (GGUF Q4_K_M) | 1.5B | ~1.0–1.2 GB | 强（阿里通义系） | Apache-2.0 | **默认本地模型** |
| 轻量备选 | **Qwen2.5-0.5B-Instruct** (GGUF Q4) | 0.5B | ~0.4–0.5 GB | 可用，润色略糙 | Apache-2.0 | 低配机 / 与大 ASR 共存 |
| 新一代轻量 | **Qwen3-0.6B / 1.7B** Instruct GGUF | 0.6 / 1.7B | ~0.5 / ~1.2 GB | 更新架构，生态成熟中 | Apache-2.0 | 二期后期可升默认 |
| 质量可选 | **Qwen2.5-3B-Instruct** Q4 | 3B | ~2.0–2.2 GB | 更好 | Apache-2.0 | 设置里「高质量」 |
| 边缘备选 | Gemma 4 E2B 等 | ~2–3B eff. | ~2 GB | 中文弱于 Qwen | 需看条款 | 不优先 |
| 云端 | 百炼 `qwen-turbo` / `qwen-flash` 等 | — | 0 本地 | 最强 | 商业 API | 兜底 / 强制云端 |

### 3.2 为何默认 Qwen2.5-1.5B-Instruct

1. **中文短改写**表现稳定，Instruct 版对 system prompt / 人设友好。  
2. **官方 GGUF** 齐全，llama.cpp 一等公民。  
3. **Apache-2.0**，商业/开源产品友好。  
4. Q4_K_M ~1GB，与现有 sherpa 模型（~227MB + ORT）在 16GB 统一内存 Mac 上可共存。  
5. 与云端百炼同属「通义系」，本地/云端风格一致性更好（产品叙事也顺）。

0.5B 作「极速模式」：延迟更低，但纠 ASR 谐音、复杂人设会明显变差，宜作为可选而非默认。

### 3.3 明确不选（一期不引入）

| 方案 | 原因 |
|---|---|
| 7B+ 通用模型 | 延迟与内存不适合「按键即输」 |
| 仅 Python/Ollama 外挂进程 | 分发复杂、生命周期难管；可作为实验，非默认 |
| 纯 Candle 从零 | 生态/量化/中文 chat template 成熟度不如 llama.cpp |
| 专用「标点恢复」小模型单独一条链 | 可用 LLM 一并完成；减少模型种类与下载 |

---

## 4. 推理运行时选型

| 运行时 | 语言 | Apple Silicon | 嵌入进程 | 评价 |
|---|---|---|---|---|
| **llama.cpp**（via `llama-cpp-2` / 自链） | C++/Rust 绑定 | Metal 加速成熟 | 是 | **首选**：GGUF 生态 + 跨 Win/mac 统一 |
| MLX | Python/Swift 为主 | 极致 | Rust 嵌入弱 | Mac 极致性能，跨平台差 |
| Candle | Rust | 可 | 是 | 纯 Rust 优雅，中小模型体验仍追 llama.cpp |
| 外挂 Ollama HTTP | 任意 | 好 | 否 | 开发方便，产品打包不推荐默认 |

**建议**：

- 本地：`llama.cpp` + **GGUF Q4_K_M**，feature 门控（类似 `sherpa`），如 `llm` / `polish-local`。  
- 云端：百炼 **OpenAI 兼容** `chat/completions`（与现有 ASR 的 Protocol A WS 分离，复用同一 api_key/workspace 即可）。  
- 下载：扩展现有 `model_mgr` / `model_download`（SHA256、hf-mirror、进度事件），布局：

```
app_data_dir/models/
  asr/          # 现有 sherpa
  vad/
  llm/          # 新增
    qwen2.5-1.5b-instruct-q4_k_m.gguf
    ...
```

---

## 5. 架构设计（对齐 ASR 双引擎）

### 5.1 新 trait（voice-core）

```rust
/// 文本增强：润色 / 人设改写。与 AsrProvider 对称。
#[async_trait]
pub trait TextPolishProvider: Send + Sync {
    async fn polish(&self, req: PolishRequest) -> Result<PolishResponse>;
}

pub struct PolishRequest {
    pub text: String,                 // ASR final
    pub persona_prompt: Option<String>,
    pub hotwords: Vec<String>,        // 偏置提示，非强制替换
    pub mode: PolishMode,             // Light | Persona | Off
    pub timeout: Duration,
}

pub struct PolishResponse {
    pub text: String,
    pub provider: String,             // "local-qwen" | "bailian-qwen" | "passthrough"
    pub latency_ms: u32,
}
```

### 5.2 复合路由（镜像 `CompositeAsrProvider`）

```
TextPolishRouter
  policy: PreferLocal | PreferCloud | LocalOnly | CloudOnly | Off
  local:  Option<LocalGgufPolish>   // llama.cpp
  cloud:  Option<BailianChatPolish> // OpenAI-compatible
```

默认策略：**PreferLocal**

1. `mode == Off` → passthrough  
2. local 已加载 → 调本地；超时/错误 → 若允许则 cloud，否则原文  
3. local 未安装 → cloud（若已配 key），否则原文 + 设置页提示下载  

### 5.3 接入 pipeline

```
record_and_collect 得到 finals
  → 对每条 final（或整段 join）调用 polish
  → insert_finals(polished)
```

注意：

- **partial 不润色**（避免 UI 抖动与算力浪费）。  
- 离线 ASR 模式（松开 Fn 再解码）与润色更合拍：一次 final 一次 polish。  
- 实时流式 ASR：仅在 sentence final 上 polish。  
- **主线程约束**：润色在 worker 线程；插入前仍走现有还焦逻辑。

### 5.4 配置扩展（`AppConfig`）

```text
polish_enabled: bool                 // 总开关，默认 false（渐进）或 true（产品激进）
polish_policy: PreferLocal | ...     // 默认 PreferLocal
polish_local_model: "qwen2.5-1.5b-instruct-q4_k_m"
polish_cloud_model: "qwen-turbo"     // 百炼 chat
active_persona_id: Option<String>
polish_timeout_ms: u32               // 默认 800
```

人设：用已有 `personas` 表（id/name/prompt/is_builtin/ord/hidden）。  
热词：已有 `hotwords`；润色 prompt 注入「下列专有名词请保留：…」。

### 5.5 与 ASR 引擎的关系（两层双引擎）

```
┌──────────────┐     ┌─────────────────────────────┐
│ ASR layer    │     │ Polish layer                │
│ local sherpa │     │ local GGUF (Qwen 1.5B)      │
│ cloud bailian│     │ cloud bailian chat          │
└──────────────┘     └─────────────────────────────┘
        均可独立选择；默认两者都优先本地
```

用户可组合：本地 ASR + 本地润色（全离线）；本地 ASR + 云端润色；全云端等。

---

## 6. Prompt 与人设（初稿）

**系统 prompt（Light 模式）** 要点：

- 只输出改写后的正文，不要解释  
- 修正 ASR 明显错误、补中文标点、去掉「嗯/那个」  
- 不改变原意，不扩写成长文  
- 保留用户热词列表中的写法  

**人设**：`personas.prompt` 追加为额外 system/user 约束（如「改成商务邮件语气」）。

**安全**：限制 max_tokens（如 128–256），temperature 低（0.2–0.4），防止跑飞。

---

## 7. 非功能与风险

| 风险 | 缓解 |
|---|---|
| 包体积 / 下载 | 模型不进 .app；按需下载；默认 1.5B ~1GB |
| 与 sherpa 内存叠加 | 低配默认 0.5B；可「用完卸载」可选 |
| 润色改错专有名词 | 热词强制保留；diff 过大可回退原文（启发式） |
| 主线程崩溃 | 禁止 AppKit 在 tokio 调（已有教训） |
| 云端 key | 复用 bailian api_key；chat 与 ASR 分 path |
| Feature 膨胀 | `llm` feature 默认开或关与 sherpa 解耦 |

---

## 8. 建议实施切片（PR 序）

| PR | 内容 |
|---|---|
| **P0 调研冻结** | 本文档；默认模型锁定 Qwen2.5-1.5B-Instruct Q4_K_M |
| **P1 trait + passthrough** | `TextPolishProvider`、pipeline 挂钩、开关默认关、单测 |
| **P2 云端 polish** | 百炼 OpenAI 兼容 chat；复用 key；超时与错误回退 |
| **P3 本地 GGUF** | llama.cpp 绑定 + model_mgr 下载 + PreferLocal 路由 |
| **P4 人设 UI** | personas CRUD + 内置 3–5 套 + 设置页 |
| **P5 热词打通** | 润色 prompt 注入；导出 sherpa hotwords 已有能力复用 |
| **P6 体验** | 超时/回退策略、设置页模型状态、可选 0.5B 档 |

---

## 9. 推荐决策（供设计评审确认）

1. **默认本地模型**：`Qwen2.5-1.5B-Instruct` GGUF **Q4_K_M**。  
2. **轻量可选**：`Qwen2.5-0.5B-Instruct` Q4。  
3. **运行时**：进程内 **llama.cpp**（feature `llm`），不用默认依赖 Ollama。  
4. **云端**：百炼 OpenAI 兼容，`qwen-turbo` / `qwen-flash` 级。  
5. **策略默认**：`PreferLocal`，失败/超时回退原文或云端（可配）。  
6. **润色时机**：仅 **final**，不碰 partial。  
7. **下载与校验**：扩展现有 `model_download` 模式，不新造平行体系。

---

## 10. 开放问题（需产品拍板）

1. 润色默认开还是关？（建议：**默认关**，设置一键开，降低首启心智与体积压力）  
2. 超时回退：仅原文，还是自动尝试云端？  
3. 是否在第一次打开润色时引导下载 1.5B（~1GB）？  
4. 人设是否允许用户自定义 system prompt（安全与越狱面）？  
5. 后期是否升默认到 Qwen3-1.7B（需回归延迟）？

---

## 11. 参考链接

- [Qwen2.5-0.5B-Instruct](https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct)  
- [Qwen2.5-1.5B-Instruct-GGUF](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF)  
- [Qwen3 系列开源尺寸表](https://github.com/phlx0/awesome-open-weight-models)  
- [Apple Silicon 本地模型指南](https://apxml.com/posts/best-local-llms-apple-silicon-mac)  
- [阿里云百炼模型列表](https://help.aliyun.com/zh/model-studio/models)  
- 仓库内：`README.md` 二期路线；`store.rs` personas/hotwords 预留；`model_mgr.rs` 一期下载管线  

---

## 12. 落地进度（2026-08-11）

已确认默认模型：**Qwen2.5-1.5B-Instruct GGUF Q4_K_M**（bartowski，~986MB）。

| 项 | 状态 |
|---|---|
| TextPolish trait / PolishMode / Router PreferLocal | ✅ |
| AppConfig 润色字段 + 人设 seed | ✅ |
| pipeline insert_finals_with_polish | ✅ |
| 云端 BailianChatPolish（OpenAI 兼容） | ✅ |
| GGUF 下载 install_polish_model + SHA256 | ✅ |
| LocalGgufPolish + feature `llm`（llama-cpp-2） | ✅ 代码就绪；**需 cmake** 才能默认编进 |
| 设置页「AI 润色」开关/策略/下载/人设 | ✅ |
| 默认构建含 llm | ❌ 本机无 cmake；用 `cargo build -p openime --features llm` |

本地推理启用：

```bash
brew install cmake
cargo build -p openime --features "sherpa,llm,custom-protocol"
# 或 scripts/build.sh 增加 WITH_LLM=1
```
