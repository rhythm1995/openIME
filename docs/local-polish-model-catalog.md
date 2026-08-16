# ⚠️ 已归档：本地润色模型调研

> **本文是 2026-08-15 的润色模型选型调研，已被 [local-model-suite.md](./local-model-suite.md) 吸收。**
> 当前润色模型目录（Qwen3.5 0.8/2/4B 三档 + 回退 Qwen3）与推荐器以 suite plan 为准。


> 状态：**调研冻结（2026-08-15）**，未改代码。  
> 范围：设置页做成和 ASR 一样的可下载目录；**丢掉 Qwen2.5-1.5B**，只上当前代 Qwen3.5（加载失败同档回退 Qwen3）。标签按「ASR + 润色 + 翻译」**三模型同时常驻**打，并用地实测 / 折算 TPS 当推荐依据。  
> 对齐：[`asr_catalog.rs`](../crates/voice-core/src/asr_catalog.rs)、[`system.rs`](../crates/voice-core/src/system.rs)、[`local-translate-model-research.md`](local-translate-model-research.md)。

---

## 0. 原则

1. **给选择，不给唯一答案。** 四张卡：极速 / 均衡 / 高质量 / 旗舰。  
2. **推荐是标签，不是锁死。** 8GB 和 48GB 看到同一张表，绿标落点不同。  
3. **按三模型共存算账。** 用户可以同时开本地 ASR + 本地润色 + 本地翻译。标签看的是 **当前选用的 ASR + 这张润色卡 + 当前选用的翻译卡** 的合计，不是单卡体积。  
4. **输入法看延迟，不看把内存吃满。** 48GB 上 9B 也「装得下」，但 40 字润色如果要 2 秒就不该当默认。  
5. **不保留 Qwen2.5-1.5B。** 直接删，目录只放当前推荐代际。

---

## 1. 上架目录（4 张）

统一 **GGUF Q4_K_M**、llama.cpp、Apache-2.0、关 thinking、`n_ctx=2048`。

| 档 | id | 首选 | 回退（`qwen35` 加载失败） | 文件 | 常驻 RSS 约 |
|---|---|---|---|---|---|
| **极速** | `qwen3.5-0.8b` | Qwen3.5-0.8B | Qwen3-0.6B | 533 / ~500 MB | **0.6 GB** |
| **均衡** | `qwen3.5-2b` | Qwen3.5-2B | Qwen3-1.7B | 1.40 / 1.11 GB | **1.5 GB** |
| **高质量** | `qwen3.5-4b` | Qwen3.5-4B | Qwen3-4B-Instruct-2507 | 2.74 / ~2.5 GB | **2.8 GB** |
| **旗舰** | `qwen3.5-9b` | Qwen3.5-9B | Qwen3-8B | ~5.5 / ~5.0 GB | **5.5 GB** |

RSS 取 mac-llm-bench 在 ctx=4096 下的峰值（我们只用 2048，会略低）：Qwen3-0.6B 0.57GB、1.7B 1.32GB、Qwen3.5-4B 2.74GB、Qwen3.5-9B 5.48GB。

设置页显示档名 + 小字型号。底层按绑定能力选首选或回退。

---

## 2. TPS：三款机器

解码速度（tg，tok/s）决定「说完到上屏」。短句润色大约 **输出 20–40 token**。

### 2.1 机器画像

| 代号 | 机器 | 统一内存带宽 | GPU | 角色 |
|---|---|---|---|---|
| **A** | M4 / 16GB（Air、Mac mini 基配） | **120 GB/s** | 8–10 核 | 低中配常见 |
| **B** | M5 / 32GB（Air 基配） | **153 GB/s** | 10 核 | **唯一有完整 llama-bench 实测的锚点** |
| **C** | M4 Pro / 48GB（14c/20g 高配） | **273 GB/s** | 20 核 | 主力机；带宽与 24GB 版相同，48GB 只多共存余量 |

M4 16GB 与 M4 Pro 48GB **没有**同条件公开 llama-bench。算法：

- **B 列为实测**（[mac-llm-bench](https://github.com/enescingoz/mac-llm-bench) M5 10c/10g/32GB，llama.cpp GGUF Q4_K_M，`--no-think`，tg128）。  
- 旁证锚点：同仓库 **M2 Max 12c/30g/32GB / 400 GB/s** 实测（不进推荐表，只校验折算）。  
- **A、C 为折算**：decode 近似跟带宽走。  
  - A = B × `120/153` ≈ **0.78×**  
  - C：在 B 与 M2 Max 之间按带宽线性插值，`frac = (273−153)/(400−153) ≈ 0.49`；Qwen3.5 在 M5 上内核明显比 M2 Max 新，3.5 档改用 **C ≈ B × 273/153 ≈ 1.78×**（更信新内核）。

### 2.2 生成速度（tok/s，越高越好）

| 模型 | B 实测 M5 32GB | A 折算 M4 16GB | C 折算 M4 Pro 48GB | 备注 |
|---|---:|---:|---:|---|
| Qwen3-0.6B（极速回退） | **92** | ~72 | ~160 | 实测 B / M2 Max 256 |
| **Qwen3.5-0.8B（极速）** | ~85* | ~66* | ~150* | 无现成 bench；夹在 0.6B 与 1.7B 之间，3.5 默认非思考 |
| Qwen3-1.7B（均衡回退） | **37** | ~29 | ~89 | 实测 B / M2 Max 144 |
| **Qwen3.5-2B（均衡）** | ~50* | ~39* | ~89* | 无现成 bench；按 3.5-4B 的 1.7× 体积比、略次线性 |
| Qwen3-4B（高质量回退） | **17** | ~13 | ~40 | 实测 B / M2 Max 65 |
| **Qwen3.5-4B（高质量）** | **29** | ~23 | ~52 | 实测 B；M2 Max 48（旧内核偏慢） |
| Qwen3-8B（旗舰回退） | **9** | ~7 | ~23 | 实测 B / M2 Max 37 |
| **Qwen3.5-9B（旗舰）** | **13** | ~10 | ~23 | 实测 B / M2 Max 30 |

`*` = 预测。其余带粗体的 B 列是 mac-llm-bench 原表数字（四舍五入）。

3.5-4B 在 M5 上 **快过** 同尺寸 Qwen3-4B（29 vs 17），Gated DeltaNet 对短生成有利。旗舰 9B 在 16GB 上约 10 t/s，输入法体感会拖。

### 2.3 换成「40 token 润色要多久」

`latency ≈ 40 / tps`（忽略 prefill；100 token 的 prompt 在这些尺寸上 prefill 通常 < 200ms）。

| 档 | M4 16GB | M5 32GB | M4 Pro 48GB | 输入法体感 |
|---|---|---|---|---|
| 极速 0.8B | ~0.6 s | ~0.5 s | ~0.3 s | 即上屏 |
| 均衡 2B | ~1.0 s | ~0.8 s | ~0.5 s | 默认可接受 |
| 高质量 4B | ~1.7 s | ~1.4 s | ~0.8 s | 16GB 偏慢；Pro 仍顺 |
| 旗舰 9B | ~4.0 s | ~3.1 s | ~1.7 s | 不该当默认 |

推荐器用的延迟预算：**默认档要求估测 ≤ 1.0s / 40 token**（约 ≥ 40 t/s）。「适合」可以放到 ≤ 1.5s；再慢只标「可用但较慢」。

---

## 3. 三模型同时常驻

听写开润色、翻译键走开翻译，两套 GGUF 加一套 sherpa，都该假定**同时在内存里**。`translate_with_polish` 本地还是两步串行，但权重不能靠「用完就卸」省——来回切快捷键会把冷启动打到脸上。

### 3.1 各层 RSS（约）

| 层 | 轻 | 默认 | 重 |
|---|---|---|---|
| ASR | SenseVoice **0.7 GB** | FunASR Nano int8 **1.2 GB** | FireRed / Nano fp16 **2.0–2.5 GB** |
| 润色 | 0.8B **0.6** | 2B **1.5** | 4B **2.8** / 9B **5.5** |
| 翻译 | MiLMMT-1B **1.1** | HY-MT 1.8B **1.4** | （暂不上 7B） |
| 应用+OS 预留 | 16GB 机器 **6 GB** | 32GB **8 GB** | 48GB **10 GB** |

### 3.2 组合峰值（ASR + 润色 + 翻译 + 预留）

默认翻译按 HY-MT 1.4GB。单位 GB。

| ASR \ 润色 | 0.8B | 2B | 4B | 9B |
|---|---|---|---|---|
| SenseVoice 0.7 | 8.8 / 10.8 / 13.8 | 9.7 / 11.7 / 14.7 | 11.0 / 13.0 / 16.0 | 13.7 / 15.7 / 18.7 |
| FunASR int8 1.2 | 9.3 / 11.3 / 14.3 | 10.2 / 12.2 / 15.2 | 11.5 / 13.5 / 16.5 | 14.2 / 16.2 / 19.2 |
| FireRed 2.3 | 10.4 / 12.4 / 15.4 | 11.3 / 13.3 / 16.3 | 12.6 / 14.6 / 17.6 | 15.3 / 17.3 / 20.3 |

格内是「模型合计 + 该档预留」在 16 / 32 / 48 三档预留下的**同一模型合计、不同预留**。更直观的判据是下面的预算：

| 机器总内存 | 留给三模型的预算（总内存 − 预留） | 含义 |
|---|---|---|
| 8GB | ~2.5 GB | 只能 SenseVoice + 极速，翻译建议云端 |
| **16GB M4** | **~10 GB** | SenseVoice/FunASR + 2B + 翻译 轻松；4B 紧；9B 超 |
| 32GB | ~24 GB | 最重组合也过 |
| **48GB M4 Pro** | **~38 GB** | 三模型随便叠，瓶颈是延迟不是容量 |

### 3.3 标签公式（给后续实现）

```
budget = total_mem - os_reserve(total_mem)
combo  = rss(asr) + rss(this_card) + rss(translate_or_0)
tps    = lookup_or_scale(chip, this_card)   // §2 表

if combo > budget:                kind = not_recommended   // 装不下三件套
else if tps < 15:                 kind = not_recommended   // 输入法会明显卡
else if tps < 25 or combo > 0.85*budget: kind = usable
else:                             kind = suitable
```

`recommended`：在 `suitable` 里选 **延迟 ≤ 1.0s 的最大档**，再封顶到均衡（2B）。  
于是：

- M4 16GB + SenseVoice + 本地翻译：0.8B / 2B 绿，推荐 **2B**；4B 黄（~1.7s）；9B 红（超预算或 ~4s）。  
- M5 32GB：到 4B 都绿，推荐仍 **2B**（4B ~1.4s 未进 1.0s 默认线）。  
- M4 Pro 48GB：4B 也绿且 ~0.8s，**可以推荐 4B**；9B 绿但 ~1.7s，只标「适合、稍慢」，不自动勾。  
- 8GB 或未装翻译、ASR 已是 FireRed：推荐掉到 **0.8B**。

用户点红卡不拦截，reason 写清「与当前识别/翻译模型合计约 X GB，本机预算 Y GB」或「估测 40 字约 Z 秒」。

---

## 4. 为什么目录到 9B 为止

| 不进目录 | 原因 |
|---|---|
| Qwen2.5-1.5B | 用户明确不要兼容档；被 2B 取代 |
| Gemma / Phi / Llama | 中文短改写弱 |
| Qwen3.5-27B / 35B-A3B | 48GB 能跑，短句润色收益差，还和 ASR/翻译抢带宽 |
| 带 mmproj 的 VL 包 | 润色纯文本 |

---

## 5. 接入

```
asr_catalog.rs          →  polish_catalog.rs
list_local_asr_models   →  list_local_polish_models（带 combo 后的 perf_tag）
compute_model_tag       →  compute_llm_combo_tag(sys, asr_id, polish_id, translate_id)
tps 表                  →  静态表 + 按 cpu_brand / total_mem 选 A/B/C 行
```

`SystemInfo` 已有 `cpu_brand`、`total_mem`、`is_apple_silicon`。匹配：

- brand 含 `M4` 且含 `Pro`/`Max` → 行 C（48GB 或 24GB 同带宽）  
- brand 含 `M5` 且不含 Pro/Max → 行 B  
- brand 含 `M4` 基配 / 16GB → 行 A  
- 对不上 → 用内存分桶近似（≤16 当 A，≤36 当 B，否则 C），reason 标明「按内存估，非本芯片实测」

硬约束：模型常驻；关 thinking；不加载视觉塔；三件套默认同驻，不要为了省内存在快捷键之间来回 `load_from_file`。

---

## 6. 推荐决策

1. 目录四档：**0.8B / 2B / 4B / 9B**，首选 Qwen3.5，回退 Qwen3。删掉 1.5B。  
2. 标签看 **三模型合计 + 估测 TPS**，不看单卡。  
3. 默认封顶：多数机器 **2B**；M4 Pro 48GB 且 ASR 不是最重档时可以升到 **4B**。9B 永不自动勾。  
4. TPS 先用本文静态表；日后用本机 `llama-bench` 写回覆盖（和 ASR「重新采集」一样）。  
5. 量化 Q4_K_M；下载 HF + hf-mirror + SHA256。

---

## 7. 开放问题

1. 现有 `llama-cpp-2 = 0.1` 能否加载 `qwen35`。不能就先上 Qwen3 回退，目录形状不变。  
2. 要不要在设置页三列（识别 / 润色 / 翻译）顶上显示「当前三件套约 X GB / 预算 Y GB」？建议要，避免用户各选最大再怪崩溃。  
3. 本机跑一轮 40 token 校准是否值得做（30 秒，写进 store）。建议 P2，P1 先静态表。

---

## 8. 参考与数据出处

- 实测 tg128：[mac-llm-bench M5 10c/10g/32GB](https://github.com/enescingoz/mac-llm-bench/blob/main/results/m5/base/speed/README.md)（llama.cpp Q4_K_M，`--no-think`）  
- 折算旁证：[同仓库 M2 Max 12c/30g/32GB](https://github.com/enescingoz/mac-llm-bench/blob/main/results/m2/max/speed/README.md)  
- 带宽：M4 基配 120 GB/s、M5 基配 153 GB/s、M4 Pro 273 GB/s（[Apple 规格](https://support.apple.com/en-us/121553)；24GB 与 48GB 同芯片同带宽）  
- 体积：[Qwen3.5-0.8B](https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF) 533MB、[2B](https://huggingface.co/bartowski/Qwen_Qwen3.5-2B-GGUF) 1.40GB、[4B](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) 2.74GB  
- 翻译侧：[local-translate-model-research.md](local-translate-model-research.md)
