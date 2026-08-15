# openIME 需求路线图

> 结构化需求清单（保留原始需求描述作存档，状态见各条目）。来源：[competitive-research.md](./competitive-research.md) 的竞品调研 + 产品判断。
> 实现过程与验收记录见 [progress.md](./progress.md)，测试见 [development.md](./development.md) 测试矩阵。目前仅 R11 的 Windows 原生部分未完成。

## 图例

- 🔴 **P0**：便宜 + 高价值，建议马上做
- 🟡 **P1**：中等投入，价值明确
- 🟢 **P2**：长期 / 重投入，视资源排期
- ⚪ **不做**：与定位冲突或性价比过低
- ✅ **已实现** / 🔶 **部分实现**：标注在条目标题

每条字段：价值 / 难度（★ 越少越易）/ 来源 / 描述 / 验收 / 依赖。

---

## 🔴 P0 — 便宜高价值

### R1. 单实例锁 ✅ 已实现
- **状态**：✅ 已实现（macOS/Linux：unix socket 协调，`src-tauri/src/lib.rs` `single_instance_check`；Windows：`platform/windows/single_instance.rs` CreateMutexW 命名互斥体；第二实例均唤起已有窗口后自行退出）。
- **价值** 中 · **难度** ★ · 来源 H4 / OpenLess
- **描述**：防止两个 openIME 进程同时运行争抢快捷键边沿。
- **方案**：接入 `tauri-plugin-single-instance`。
- **验收**：启动第二个实例时激活已有实例并自行退出，不出现双进程并发。
- **依赖**：无。

### R2. ESC 中断润色 ✅ 已实现
- **状态**：✅ 已实现（f3f1324；润色与 QA 流式中按 ESC 均取消，已输出部分保留）。
- **价值** 中 · **难度** ★ · 来源 H3 / CapsWriter
- **描述**：润色（尤其云端流式）跑飞或用户改主意时，按 ESC 立即中断，已输出部分保留。
- **caveat**：本地 GGUF 润色是一次性请求，ESC 主要对云端流式有意义。
- **方案**：流式润色调用加 cancel flag；前端监听 ESC → 通知后端取消。
- **验收**：润色进行中按 ESC，已插入文字保留，不再烧 token。
- **依赖**：润色流式化（部分已有）。

---

## 🟡 P1 — 中等投入

### R3. endpoint SSRF 校验 ✅ 已实现
- **状态**：✅ 已实现（f3f1324 + 054888c；`crates/voice-core/src/endpoint.rs` `validate_endpoint`，保存时校验）。
- **价值** 中 · **难度** ★★ · 来源 H1 / OpenLess
- **描述**：用户自填的 ASR / 润色 endpoint 做 host/IP 校验——拒绝云元数据（`169.254.169.254`）、CGNAT、link-local；公网强制 https；放行 RFC1918 局域网（支持自托管 ollama/Whisper）。
- **方案**：`validate_endpoint` 在 `save_config` 时调用，fail-closed（被拒回退安全默认）。
- **验收**：填 `http://169.254.169.254` 被拒；填 `http://192.168.x.x:1234` 放行；填公网 http 提示改 https。
- **依赖**：无。

### R4. 翻译模式 ✅ 已实现
- **状态**：✅ 已实现（054888c；设置 → 翻译：快捷键 / 目标语言 / 先润色再翻译）。P1 为固定 7 语；本地三件套后续扩展为本地专翻（MiLMMT-46 / HY-MT）+ 目标语言分档（基础 7 语 / 扩展集），见 [progress.md](./progress.md) / [local-model-suite-plan.md](./local-model-suite-plan.md)。
- **价值** 中 · **难度** ★★ · 来源 G2 / OpenLess
- **描述**：独立快捷键，用源语言说、直接插入目标语言；可选「润色 + 翻译」一次调用。
- **方案**：`polish/cloud.rs` 加 `translate_text`；前端加翻译快捷键 + 目标语言选择。
- **验收**：按翻译快捷键说中文，光标出英文；切换目标语言生效。
- **依赖**：云端 LLM key。

### R5. LLM 前缀角色（指令路由）✅ 已实现
- **状态**：✅ 已实现（054888c；`polish/roles.rs` 前缀检测 + store v4 迁移 + 内置 mail/translate/cmd 角色）。
- **价值** 中高 · **难度** ★★★ · 来源 F3 / CapsWriter
- **描述**：识别结果开头匹配前缀（如「翻译:」「邮件:」「命令:」）则分流到对应 system prompt / provider。比全局风格包更灵活——同一会话靠前缀分流。
- **方案**：TOML 角色配置（`name`/`match_prefix`/`system_prompt`/`provider`/`model`）；`polish/router.rs` 扩展「前缀 → 角色」路由。
- **验收**：说「邮件: 明天开会」→ 输出正式邮件体；说「翻译: hello」→ 输出译文。
- **依赖**：与风格包（F1 ✅）理清关系——角色 = 带前缀触发的风格包。

### R6. 划词语音问答（QA 面板）✅ 已实现
- **状态**：✅ 已实现（054888c；独立 `qa` 窗口 + `qa.rs` 状态机 + 多轮上下文 + ESC 取消）。
- **价值** 中 · **难度** ★★★ · 来源 G1 / OpenLess
- **描述**：独立快捷键打开浮窗，抓当前选中文字作上下文，语音提问 → LLM 流式回答，支持多轮。
- **方案**：复用选区注入（F4 ✅）；把悬浮 HUD 扩展为 QA 面板 + 多轮 messages 状态机。
- **验收**：选中一段代码，按 QA 键问「这段什么意思」，浮窗流式回答；关浮窗清空上下文。
- **依赖**：选区注入（✅）、独立 Tauri 窗口。

### R7. 粘贴兜底 + 剪贴板恢复 ✅ 已实现
- **状态**：✅ 已实现（054888c；`insert_fallback.rs` 四态插入 + arboard 剪贴板恢复，约 0.75 秒后还原）。
- **价值** 中 · **难度** ★★★ · 来源 C1 配套 / C2 / OpenLess
- **描述**：对拒收逐字模拟的 app 加 `Cmd+V` 粘贴兜底；粘贴后延时还原用户原剪贴板（校验「剪贴板仍是插入文字」才还原）。
- **caveat**：openIME 现纯 enigo 逐字、**不碰剪贴板，故当前 C2 非痛点**；粘贴兜底是为提升 app 兼容性，配套必须做剪贴板恢复。
- **方案**：`arboard` crate；三态插入（逐字成功 / 粘贴兜底 / 全失败）；OpenLess `insertion.rs` 的 restore_plan 可照搬。
- **验收**：在不接受模拟按键的 app 里文字正确插入；插入后用户原剪贴板内容恢复；用户中途改复制则不覆盖。
- **依赖**：无。

---

## 🟢 P2 — 长期 / 重投入

### R9. 短按补发原按键（Fn 误触恢复）✅ 已实现
- **状态**：✅ 已实现（85d6f50；Hold 模式短按阈值默认 300ms，macOS flagsChanged 补发。真机 🌐 验证与 TIS 回退待接，见 [progress.md](./progress.md) PR3）。
- **价值** 中高 · **难度** ★★★ · 来源 A2 / CapsWriter
- **描述**：按下时间 < 阈值（0.3s）视为误触，取消录音并补发原按键（Fn/🌐 原功能不丢）。
- **caveat**：macOS 上补发 Fn/🌐 键 tricky（NSEvent re-post），需防自捕获（补发的按键不被自己的监听器抓回）。
- **验收**：短按 Fn 不触发录音，且系统 🌐 原功能（如切输入法）正常执行。
- **依赖**：Fn 监听（✅）。

### R11. Windows IME TSF 集成 🔶 部分实现
- **状态**：🔶 部分实现（85d6f50；命名管道协议 / profile 快照 / insert_ex 判定等纯函数已落地并单测。4c0845e 起 Windows 侧已可编译 / 打包 NSIS / 真机运行，插入暂走模拟按键 + 粘贴兜底；C++ TSF DLL 与 Windows FFI 仍待落地，见 [openIME-windows-porting-notes.md](../openIME-windows-porting-notes.md)）。
- **价值** 中 · **难度** ★★★★★ · 来源 C3 / OpenLess
- **描述**：Windows 上注册自己的 TSF 输入法 profile，IME 直接 `CommitText`，比模拟按键稳（不抢焦点、不受目标 app 输入法状态干扰）。
- **caveat**：仅 Windows 版需要；工程量大（C++ TSF DLL + 安装器挂钩）。过渡方案：模拟 `Ctrl+V` + 剪贴板恢复（见 R7）。
- **依赖**：Windows 平台适配。

### R12. 本地长音频分段 + 重叠 ✅ 已实现
- **状态**：✅ 已实现（85d6f50；默认 60s 分段 + 4s 重叠，转录进度显示与取消）。
- **价值** 中 · **难度** ★★ · 来源 E1 / CapsWriter
- **描述**：本地 sherpa 离线模型长音频易截断；按 60s 分段、相邻 4s 重叠，避免边界丢字。
- **方案**：配合文件转录（D3 ✅）的长音频场景。
- **验收**：30 分钟音频本地转录，段间不丢字。
- **依赖**：文件转录（✅）。

---

## ⚪ 不做（与定位冲突或性价比低）

- **Voice Agent（语音 → `claude -p` 编码）**：偏离「语音输入法」定位，依赖外部 CLI + 护栏工程量大。
- **UDP 广播 / 控制**：CapsWriter 为外接硬件留的口子，GUI 输入法用不上。
- **Python C/S 架构 / `.py` 角色插件**：openIME 是 Rust 单体，动态脚本加载不安全不现实（角色改 TOML，见 R5）。
- **流式文本合并算法**：openIME 用流式 ASR（天然连续），仅非流式分段文件转录可能用得上，不预先引入。

---

> 实现时参考 [competitive-research.md](./competitive-research.md) 的「附录：关键文件索引」（精确到竞品源码文件 / 函数）。
