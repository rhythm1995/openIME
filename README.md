[English](./README.en.md) | **简体中文**

<p align="center">
  <img src="branding/app-icon.svg" width="108" height="108" alt="openIME logo" />
</p>
<h1 align="center">openIME</h1>
<p align="center">
  开源、本地优先的跨平台<strong>语音输入法</strong>
</p>
<p align="center">
  常驻菜单栏，按一个快捷键说话，识别文字直接输入到光标处——隐私不出本机。
</p>

<p align="center">
  <a href="https://rhythm1995.github.io/openIME/">
    <img src="https://img.shields.io/badge/website-openIME.dev-%235C6AFF" alt="website" />
  </a>
  <a href="https://github.com/rhythm1995/openIME/releases">
    <img src="https://img.shields.io/github/v/release/rhythm1995/openIME?label=Release" alt="release" />
  </a>
  <a href="https://github.com/rhythm1995/openIME/actions/workflows/ci.yml">
    <img src="https://img.shields.io/github/actions/workflow/status/rhythm1995/openIME/ci.yml?branch=main" alt="CI" />
  </a>
  <a href="https://github.com/rhythm1995/openIME">
    <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-blue" alt="platform" />
  </a>
  <a href="https://github.com/rhythm1995/openIME">
    <img src="https://img.shields.io/github/license/rhythm1995/openIME" alt="license" />
  </a>
</p>

<br />

---

## 产品截图

<p align="center">
  <img src="docs/screenshots/history.png" alt="历史页" width="720" />
</p>

<p align="center">
  <img src="docs/screenshots/settings.png" alt="设置页" width="720" />
</p>

<p align="center">
  <img src="docs/screenshots/dictionary.png" alt="词典页" width="720" />
</p>

---

## 功能一览

### 🎤 语音转文字

- **本地优先**：进程内 `sherpa-onnx` 离线识别，音频不出本机；模型一键下载（SHA256 校验 + 断点续传 + HF→国内镜像故障切换）。
- **多引擎可切换**：本地 sherpa / 百炼 WebSocket 流式 / OpenAI 兼容 REST / Multimodal REST；引擎地址智能归一（填域名或任意兼容地址均可）。
- **文件转录**：音频文件 → SRT 字幕；长音频自动分段（时长 / 重叠可调，带进度与取消）。

### ✨ AI 润色与增强

- **L0 规则纠错**：热词同音 / 模糊音、数字 ITN、繁简转换、去句末标点。
- **本地 LLM 润色**：Qwen3.5 按机型分档（0.8B 弱机 / 2B 均衡 / 4B 高配），加载失败自动回退 Qwen3，常驻内存避免冷启动。
- **云端润色兜底**：支持 OpenAI Chat / Anthropic / Responses 三协议；本地优先、失败自动回退云端、双失败原文直出。
- **热词词典 & 风格包**：自定义术语纠音；自定义 system prompt 一键切换输出风格。

### 🌐 翻译 & 问答

- **语音翻译**：独立快捷键，说源语言 → 光标出目标语言。基础 7 语开箱即用；启用云端或本地专翻小模型（MiLMMT-1B / HY-MT 1.8B）解锁约 20 种扩展语言。
- **划词问答**：选中文字后浮窗语音提问，LLM 流式回答，多轮对话。
- **前缀角色**：以「邮件:」「翻译:」「CMD:」开头自动分流到对应风格包 / provider。

### ⚙️ 系统级体验

- **全局快捷键**：默认 Fn（🌐 键）+ 按住说话（PTT）；可切换按键切换模式。
- **粘贴兜底**：逐字模拟被目标 App 吞掉时自动改粘贴（macOS Cmd+V / Windows Ctrl+V）并恢复原剪贴板。
- **短按补发**：Hold 模式短按 Fn 自动补发 🌐 原功能。
- **单实例锁**：防双进程争抢快捷键。
- **中英双语界面**：左下角一键切换；界面语言与识别语言相互独立。

### 🖥️ 平台支持

- 🍎 **macOS**：完整支持（菜单栏 + 悬浮窗 + 系统输入法协作）。
- 🪟 **Windows**：公测（NSIS 打包、CapsLock 单键录音、插入兜底真机可用，TSF 输入法原生上屏 FFI 已交付）。

---

## 本地模型三件套

openIME 的核心差异化能力：ASR、润色、翻译三套模型**同驻常驻**，按机型自动分档。

| 层 | 档位 | 模型 | 常驻 | 默认场景 |
|---|---|---|---|---|
| ASR | 轻 | SenseVoice | ~0.7 GB | 弱机 / 16GB 默认 |
| | 中 | FunASR-Nano int8 | ~1.2 GB | 16GB 可选 |
| | 重 | FunASR-Nano fp16 | ~2.0 GB | 高配可选 |
| 润色 | 极速 | Qwen3.5-0.8B | ~0.6 GB | 弱机默认（可兼译） |
| | 均衡 | Qwen3.5-2B | ~1.5 GB | 16GB 默认 |
| | 高质量 | Qwen3.5-4B | ~2.8 GB | 48GB 默认 |
| 翻译 | 默认 | MiLMMT-1B | ~1.1 GB | 有预算时默认 |
| | 自选 | HY-MT-1.8B | ~1.4 GB | 术语 / 端侧自选 |

> 设置页显示当前三件套预算占用条；弱机即使有云 key 也提示可开启「润色模型兼译」离线翻译。

---

## 快速上手

1. **下载**：从 [Releases](https://github.com/rhythm1995/openIME/releases) 获取 macOS `.dmg` 或 Windows `.exe`，安装。
2. **授权权限**（macOS）：设置 → 系统权限 → 授权**麦克风**与**辅助功能**。
3. **选择引擎**：默认推荐本地 sherpa-onnx（设置 → 识别引擎 → 下载模型）；也可填云端凭据。
4. **说话即打字**：按 **Fn（🌐）** 开始录音，再按一次停止，文字逐字输入到光标处。

> [!TIP]
> 英文界面？点左下角 🌐 按钮在中 / 英间切换。界面语言**不影响**语音识别语言——识别语言在「识别引擎 → 默认语言」单独设置。

---

## 架构

```
┌────────────────────────────────────────────────────────────┐
│            openIME（Tauri v2 · Rust · React 18）           │
├────────────────────────────────────────────────────────────┤
│ UI 层  React 18 · TypeScript · Vite · CSS Variables        │
│ 设置 / 历史  ──►  Tauri Commands  ──►  划词问答            │
│ 悬浮窗 / 菜单栏  ──►  全局快捷键 / 剪贴板  ──►  多轮       │
├────────────────────────────────────────────────────────────┤
│ 桥接层  src-tauri  commands / state / QA / 权限 / TSF      │
├────────────────────────────────────────────────────────────┤
│ 核心层  voice-core（Rust 纯库 · 零 Tauri 依赖）            │
│                                                            │
│ ┌────────┐ ─▸┌────────────────────────────┐                │
│ │ Audio  │─▸│ ASR 引擎                   │                 │
│ │ (cpal) │   │ 本地 sherpa-onnx           │                │
│ └────────┘   │ 云端 百炼 WS · OpenAI      │                │
│             │ Multimodal REST            │                 │
│             └──────────────┬─────────────┘                 │
│                             ▼                              │
│ ┌─────────────────────────────────────┐                    │
│ │ Polish / Translate（本地三件套）    │                    │
│ │ 润色 Qwen3.5（GGUF）· L0 · 云端     │                    │
│ │ 翻译 MiLMMT-1B · 前缀角色路由       │                    │
│ └──────────────────┬──────────────────┘                    │
│                    ▼                                       │
│ ┌─────────────────────────────────────┐                    │
│ │ TextInserter                        │                    │
│ │ ① enigo  ② 粘贴 Cmd+V/Ctrl+V  ③ TSF │                    │
│ └──────────────────┬──────────────────┘                    │
│                    ▼                                       │
│ ┌─────────────────────────────────────┐                    │
│ │ HistoryStore（SQLite v4）           │                    │
│ └─────────────────────────────────────┘                    │
├────────────────────────────────────────────────────────────┤
│ 测试 476 例  voice-core 345 │ 前端 47 │ 应用壳 84          │
└────────────────────────────────────────────────────────────┘
```

- **`voice-core`**：全部业务逻辑 + trait，4 个可 mock 的 trait 串成管线，端到端可单测。
- **`polish/`**：L0 规则 / 云端三协议 / 前缀角色 / 常驻 GGUF 运行时 / 翻译路由。
- **`windows_ime/`**：TSF 命名管道协议 + FFI（纯函数 + 黄金 fixture 单测）。
- 完整目录结构与设计文档见 [docs/development.md](docs/development.md)。

---

## 开发

```bash
pnpm install
pnpm test                  # 前端（Vitest + React Testing Library，47 个）
cargo test -p voice-core   # 核心库（345 个，含 13 个集成测试）
./scripts/build.sh install # 打包并安装到 /Applications
```

| 层 | 测试 | 数量 |
|---|---|---|
| voice-core | `cargo test -p voice-core` | 345 |
| 前端 | `pnpm test` | 47 |
| 应用壳（Windows CI） | `cargo test -p openime` | 84 |
| **合计** | | **476** |

CI：GitHub Actions 四 job——三平台核心矩阵、macOS 应用壳、Windows 应用壳、前端测试与构建。

完整开发流程见 [docs/development.md](docs/development.md)，排障见 [docs/troubleshooting.md](docs/troubleshooting.md)。

---

## 文档

| 文档 | 说明 |
|---|---|
| [user-guide.md](docs/user-guide.md) | 用户使用指南 |
| [development.md](docs/development.md) | 技术栈、架构、开发流程 |
| [troubleshooting.md](docs/troubleshooting.md) | 日志与常见问题 |
| [roadmap.md](docs/roadmap.md) | 需求清单与实现进度 |
| [local-model-suite.md](docs/local-model-suite.md) | 本地三件套需求与技术方案 |
| [windows-porting-notes.md](docs/openIME-windows-porting-notes.md) | Windows 移植与打包 |

---

## 路线图

- ✅ 引擎、悬浮窗、托盘、全局快捷键、引导、历史详情、界面中英双语
- ✅ AI 润色三档、热词词典、风格包 + 前缀角色、翻译模式、划词问答、文件转录（长音频分段）、繁简转换、选区注入、日记导出
- ✅ 单实例锁、ESC 中断润色、endpoint SSRF 校验、粘贴兜底 + 剪贴板恢复、Fn 短按补发
- ✅ 本地模型三件套（ASR + 润色 + 翻译，一键分档下载）、combo 打标、机型推荐
- 🔜 Windows TSF 原生上屏（FFI 已交付，Win11 per-user 需管理员注册 HKLM）

> 完整需求清单与实现状态见 [docs/roadmap.md](docs/roadmap.md)。

---

## 致谢

- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) —— 本地 ASR 引擎
- [Tauri](https://tauri.app/) · [enigo](https://github.com/enigo-rs/enigo) · [cpal](https://github.com/RustAudio/cpal)
- 产品形态参考 [AutoGLM](https://xiaonao.io/)（小凹）

---

<p align="center"><sub>本地优先 · 开源 · 可测试</sub></p>
