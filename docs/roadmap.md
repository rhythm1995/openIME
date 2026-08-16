# openIME 路线图与实现进度

> 本文是需求状态与实现进度的**唯一来源**。
> - 需求清单：保留原始需求描述，状态见各条目。来源：[`competitive-research.md`](./archive/competitive-research.md) 竞品调研 + 产品判断。
> - 设计依据：P1 需求见 [`archive/p1-design.md`](./archive/p1-design.md)，P2 需求见 [`archive/p2-design.md`](./archive/p2-design.md)，本地三件套见 [`local-model-suite.md`](./local-model-suite.md)。
> - 目前已实现全部 P0 / P1 / 大部分 P2 需求，**仅 R11 Windows TSF 原生上屏（C++ DLL + FFI）未完成**。

---

## 图例

- 🔴 **P0**：便宜 + 高价值
- 🟡 **P1**：中等投入，价值明确
- 🟢 **P2**：长期 / 重投入
- ⚪ **不做**：与定位冲突或性价比过低
- ✅ **已实现** / 🔶 **部分实现**

每条字段：价值 / 难度（★ 越少越易）/ 来源 / 描述 / 验收 / 依赖 / 实现 PR。

---

## 🔴 P0 — 便宜高价值

### R1. 单实例锁 ✅ 已实现
- **价值** 中 · **难度** ★ · 来源 H4 / OpenLess
- **描述**：防止两个 openIME 进程同时运行争抢快捷键边沿。
- **实现**：macOS / Linux unix socket 协调；Windows `CreateMutexW` 命名互斥体（`platform/windows/single_instance.rs`）；第二实例唤起已有窗口后自行退出。

### R2. ESC 中断润色 ✅ 已实现
- **价值** 中 · **难度** ★ · 来源 H3 / CapsWriter
- **描述**：润色与 QA 流式中按 ESC 均取消，已输出部分保留。
- **caveat**：本地 GGUF 润色是一次性请求，ESC 主要对云端流式有意义。

---

## 🟡 P1 — 中等投入（PR1–PR6 全部 ✅）

### R3. endpoint SSRF 校验 ✅ 已实现（PR1）
- **价值** 中 · **难度** ★★ · 来源 H1 / OpenLess
- **描述**：自填 endpoint 做 host/IP 校验——拒绝云元数据、CGNAT、link-local；公网强制 https；放行 RFC1918。
- **实现**：`crates/voice-core/src/endpoint.rs` `validate_endpoint` 32 case + `http_client_no_redirect` + `polish_cloud_api_key` Keychain 迁移 + load 坏 URL 清空。

### R4. 翻译模式 ✅ 已实现（PR4）
- **价值** 中 · **难度** ★★ · 来源 G2 / OpenLess
- **描述**：独立快捷键，源语言说、光标出目标语言；可选「先润色再翻译」。
- **实现**：设置 → 翻译（快捷键 / 目标语言 / 先润色再翻译）；`SessionIntent::Translate` + `LlmClient::translate_text`；`polish_and_translate` 哨兵合成；失败回退 L0 + `TranslateFailed` HUD。
- **语言分档**（本地三件套 `f43012f` 后）：纯本地润色模型兼译 = 基础 7 语；启用云端或本地专翻 = 扩展集（约 20 种，含繁中 / 粤语 / 阿拉伯 / 俄等）。

### R5. LLM 前缀角色 ✅ 已实现（PR5）
- **价值** 中高 · **难度** ★★★ · 来源 F3 / CapsWriter
- **描述**：识别结果开头匹配前缀（如「翻译:」「邮件:」）则分流到对应 system prompt / provider。角色 = 带 `match_prefix` 的风格包（同一张 `style_packs` 表）。
- **实现**：`polish/roles.rs` `detect_prefix_role`（最长别名 + `prefix_boundary_ok` + 空正文拒绝，14 测试）；store v4 迁移；内置 `builtin-role-{mail,translate,cmd}`；命中直连 cloud/local（禁止 `PolishRouter`）；`prefix_roles_enabled=true` 时听写 `streaming_insert=false`。

### R6. 划词语音问答 ✅ 已实现（PR6）
- **价值** 中 · **难度** ★★★ · 来源 G1 / OpenLess
- **描述**：独立快捷键打开浮窗，抓选区作上下文，语音提问 → LLM 流式回答，多轮；关窗清空。
- **实现**：独立 `qa` 窗口 + `qa.rs` 状态机（Hidden/Idle/Recording/Transcribing/Streaming）+ 多轮 messages（8 轮 / 8000 字截断）+ ESC 取消 + 复制 / 插入光标 / 刷新选区。

### R7. 粘贴兜底 + 剪贴板恢复 ✅ 已实现（PR2）
- **价值** 中 · **难度** ★★★ · 来源 C2 / OpenLess
- **描述**：enigo 失败时平台粘贴兜底（macOS `Cmd+V` / Windows `Ctrl+V`），粘贴后 750ms 恢复用户原剪贴板（校验「仍是插入文字」才恢复）。
- **实现**：`InsertOutcome` 四态（Typed / Pasted / CopiedFallback / Failed）+ `PendingRestore` + `CLIPBOARD_MU` + `insert_fallback.rs`；`paste_fallback_apps` 命中前台 exe basename 视同 Paste。

### R13. Hotkey 注册中心 + 互斥（PR4 内） ✅ 已实现
- 任意 hotkey 字段变化都重新注册；`parse_code` 扩展标点支持；热键两两不等校验；翻译 / QA 键 Toggle-only。

---

## 🟢 P2 — 长期 / 重投入

### R9. 短按补发原按键 ✅ 已实现（PR2 + PR3）
- **价值** 中高 · **难度** ★★★ · 来源 A2 / CapsWriter
- **描述**：Hold + Fn 模式下按下 < 阈值（默认 300ms）视为误触，取消录音并补发 🌐 原功能。
- **实现**：`fn_policy.rs` `classify_fn_edge` 状态机（ArmHoldTimer → delay-start → RepostOnly）；`this_press_started_recording` 判定；macOS 一对 `kCGEventFlagsChanged` + `REPOST_IGNORE_MS=60` 自捕获过滤 + `set_fn_tap_consume` 原子配置；Toggle 松开不再停（KD-2 修正）。
- **待验**：真机 A9.1（HID flagsChanged 补发是否触发 🌐 切输入法）；TIS 回退 `fn_repost_tis_fallback` 未接（默认 false）。

### R11. Windows IME TSF 集成 🔶 部分实现（PR5 + PR6，纯函数完成）
- **价值** 中 · **难度** ★★★★★ · 来源 C3 / OpenLess
- **描述**：Windows 上注册自有 TSF profile，IME 直接 `CommitText`，比模拟按键稳。
- **已完成**：命名管道协议（`windows_ime/protocol.rs`，4 条黄金 fixture roundtrip）+ profile 快照 + `should_fallback_after_ime` + `ime_pipe_name_for_target` + `InsertOutcome::Committed` + `InsertOpts::from_config`（`tsf = windows && cfg && !streaming`）+ `tsf_supported_for_machine`（仅 AMD64）。
- **待落地**：C++ `OpenImeTsf.dll`（ITfTextInputProcessorEx + ITfEditSession，`/MT`）+ NSIS hooks（HKCU `regsvr32 /s`）+ 命名管道 client/session FFI + `insert_ex` TSF 通路 + 设置页 IME 状态与恢复按钮。详见 [`windows-porting.md`](./windows-porting.md)。
- **过渡**：Windows 当前走 enigo + 粘贴兜底（R7），功能可用，TSF 为增强。

### R12. 本地长音频分段 + 重叠 ✅ 已实现（PR1）
- **价值** 中 · **难度** ★★ · 来源 E1 / CapsWriter
- **描述**：长音频按 60s 分段、相邻 4s 重叠，避免边界丢字。
- **实现**：`segment_ranges` + `stitch_overlap_punct`（k_min=2、标点二次去重）+ `srt_from_segments`（跨段序号连续）+ `transcribe_file_full` 新签名（自建 recognizer，不碰 cache）+ `transcribe_guard` + `cancel_transcribe` + `transcribe://progress` + UI 取消按钮。

### R14. 本地模型三件套 ✅ 已实现（`f43012f`）
- **详见** [`local-model-suite.md`](./local-model-suite.md)。
- **润色**：`qwen3.5-0.8b` / `qwen3.5-2b`（默认）/ `qwen3.5-4b`；加载失败回退 Qwen3；旧 `qwen2.5-1.5b-*` 配置读入时映射到 `qwen3.5-2b`。
- **翻译**：`milmmt-1b`（默认专翻）/ `hy-mt-1.8b`（自选）；兼译（润色模型两步 Light→译）；目标语言分档（基础 7 语 / 扩展集）。
- **ASR**：下架 FireRed；`open_model_directory` + 打开目录按钮。
- **combo 打标**：`compute_combo_tag`（三件套预算 + 估测 TPS）+ `recommend_defaults` 按内存分档写默认。

---

## ⚪ 不做（与定位冲突或性价比低）

- **Voice Agent（语音 → `claude -p` 编码）**：偏离「语音输入法」定位。
- **UDP 广播 / 控制**：GUI 输入法用不上。
- **Python C/S 架构 / `.py` 角色插件**：Rust 单体，动态脚本加载不安全。
- **流式文本合并算法**：openIME 用流式 ASR（天然连续），仅非流式分段文件转录可能用得上，不预先引入。
- **风格包市场 / JSON 导入导出**：冷启动期非刚需。
- **请求期 DNS `lookup_host` + `classify_ip`**：P1 字面 + 禁 redirect 已覆盖主要风险。
- **音素模糊热词**：Rust 无现成库，工程量大。
- **Linux fcitx5**：暂无 Linux 计划。
- **32-bit / ARM64 TSF DLL**：P2 只交 x64。

---

## 全量测试现状

| 层 | 测试 | 数量 |
|---|---|---|
| voice-core（本地可跑） | `cargo test -p voice-core`（lib 332 + 集成 13） | 345 |
| 前端 | `pnpm test`（Settings 37 / App 5 / History 3 + 其他） | 47 |
| 应用壳（Windows CI 跑） | `cargo test -p openime`（含 `platform/windows/*` 与 `windows_ime` FFI） | 84† |
| **合计** |  | **476** |

CI：GitHub Actions 四 job——`core`（三平台矩阵 × fmt+clippy+test）、`tauri-shell`（macOS，clippy -D warnings）、`tauri-shell-windows`（windows-latest，clippy -D warnings + cargo test）、`frontend`（vitest + build）。

---

> 完整设计依据见 [`archive/`](./archive/) 下的 ADR 文档。开发流程见 [`development.md`](./development.md)。