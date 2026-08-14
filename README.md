[English](./README.en.md) | **简体中文**

# openIME

开源、本地优先的跨平台**语音输入法**——按一下快捷键说话，识别文字直接输入到光标。

[下载](./releases) · [快速上手](#快速上手) · [用户指南](docs/user-guide.md) · [开发文档](docs/development.md) · [排障](docs/troubleshooting.md) · [路线图](docs/roadmap.md)

> [!NOTE]
> 常驻菜单栏，按全局快捷键（默认 **Fn / 🌐 键**）录音 → 实时流式转写 → 文字逐字输入到当前光标 → 按会话保存历史。

## 特性

- 🔒 **本地优先，隐私至上** —— 本地 `sherpa-onnx` 离线识别，音频不出本机；模型一键下载（SHA256 校验 + 断点续传 + HF→国内镜像故障切换）。
- ☁️ **多引擎可切换** —— 本地 sherpa / 百炼 WebSocket 流式 / OpenAI 兼容 REST / Multimodal REST；引擎地址智能归一（填域名 / OpenAI 兼容地址 / DashScope 地址均可）。
- ✨ **AI 润色三档** —— L0 规则纠错（热词同音 + 模糊音、数字 ITN、繁简转换、去句末标点）+ L2 LLM（本地 Qwen2.5 优先，云端 OpenAI Chat / Anthropic / Responses，双失败原文直出）。
- 📝 **热词词典 & 风格包** —— 自定义术语纠音；自定义 system prompt 一键切换输出风格；前缀角色（「邮件:」「翻译:」开头自动分流处理）。
- 🎬 **文件转录** —— 音频文件 → SRT 字幕；长音频自动分段（时长 / 重叠可调，带进度与取消）。
- 🌐 **语音翻译 & 划词问答** —— 独立快捷键说源语言、光标直出目标语言；选中文字浮窗语音提问，流式回答。
- 🌐 **中英双语界面** —— 左下角一键切换；界面语言与 ASR 识别语言相互独立。
- ⌨️ **全局快捷键** —— 默认 Fn（🌐），支持切换 / 按住说话（PTT）两种模式。
- 🧲 **插入兜底** —— 逐字模拟被目标 App 吞掉时自动改粘贴（Cmd+V）并恢复原剪贴板；Hold 模式短按 Fn 自动补发 🌐 原功能。
- 🍎 **macOS**（Windows 进行中）。



## 快速上手

> 普通用户视角。开发者请看 [开发文档](docs/development.md)。

1. **下载安装**：从 [Releases](./releases) 下载 macOS `.dmg`，拖入 `/Applications`。内测包未公证，首次打开需 **右键 → 打开**。
2. **授权权限**：打开 openIME → 设置 → 系统权限 → 授权 **麦克风** 与 **辅助功能**。
3. **选择引擎**：默认推荐本地 sherpa-onnx（设置 → 识别引擎 → 下载模型）；也可填云端凭据。
4. **说话即打字**：按 **Fn（🌐）** 开始录音，再按一次停止，识别文字输入到当前光标。

> [!TIP]
> 想用英文界面？点左下角 🌐 按钮即可在中 / 英间切换。界面语言**不影响**语音识别语言——识别语言在「识别引擎 → 默认语言」单独设置。



## 识别引擎


| 引擎                  | 说明                                                   | 状态  |
| ------------------- | ---------------------------------------------------- | --- |
| **本地 sherpa-onnx**  | 进程内离线识别，模型一键下载；OfflineRecognizer 常驻缓存（二次录音零加载）       | ✅   |
| **百炼 WebSocket 流式** | Protocol A（run-task / result-generated），逐字上屏         | ✅   |
| **OpenAI 兼容 REST**  | `POST /audio/transcriptions`，兼容 Whisper / OpenRouter | ✅   |
| **Multimodal REST** | `POST /chat/completions`，兼容百炼 Qwen3 ASR 非流式          | ✅   |


云端 LLM 润色支持 3 协议，策略固定为「本地优先，失败 / 未装自动回退云端，双失败原文直出不报错」。

## 架构

核心逻辑全部在 `voice-core`（零 Tauri 依赖，纯库），四个可 mock 的 trait 串成端到端管线：

```
AudioSource ──► AsrProvider/AsrSession ──► TextInserter
   (cpal)          (百炼 WS / sherpa)         (enigo)
                        │
                        ▼
                  HistoryStore (SQLite)
```

完整的目录结构、模块说明与设计文档见 [docs/development.md](docs/development.md)。

## 开发

```bash
pnpm install && pnpm test       # 前端（Vitest + React Testing Library）
cargo test -p voice-core        # 核心库
./scripts/build.sh install      # 打包并安装到 /Applications（固定签名）
```

303 个测试覆盖（voice-core 236 + 集成 13 + 应用壳 36 + 前端 18）。完整开发流程、测试矩阵、CI、签名与发布见 [docs/development.md](docs/development.md)，日志与排障见 [docs/troubleshooting.md](docs/troubleshooting.md)。

## 致谢

- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) —— 本地 ASR 引擎
- [Tauri](https://tauri.app/) · [enigo](https://github.com/enigo-rs/enigo) · [cpal](https://github.com/RustAudio/cpal)
- 产品形态参考 AutoGLM（小凹）

---

本地优先 · 开源 · 可测试