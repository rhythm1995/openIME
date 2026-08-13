# openIME 竞品调研：OpenLess 与 CapsWriter-Offline

> 调研对象
> - [Open-Less/openless](https://github.com/Open-Less/openless)（下称 **OpenLess**，v1.3.6）
> - [HaujetZhao/CapsWriter-Offline](https://github.com/HaujetZhao/CapsWriter-Offline)（下称 **CapsWriter**，v2.5）
>
> 调研方法：用 zread 直接读取两仓库的 README、`core/`、`src-tauri/src/`、`docs/` 下的具体源码，所有引用都精确到文件/函数，方便日后索引。调研日期 2026-08-12。
>
> **📌 落地状态（2026-08-13 更新）**：本文写于 2026-08-12，此后 openIME 二期已实现其中 **A1 / B1 / B2 / B3 / B4 / B5 / B6 / C1 / D1 / D2 / D3 / F1 / F4 / H2** 共 14 项（见 [README](../README.md) 路线图 ✅ 段）；**未实现**的需求已抽到 [roadmap.md](./roadmap.md)。下文 §2 / §3 的「建议清单 / 优先级」保留作历史档案，**当前状态以本段为准**。
>
> openIME 当前能力：Tauri + Rust（`voice-core`）+ React，macOS 优先；本地 sherpa-onnx（SenseVoice / FireRedASR / FunASR-Nano / Zipformer）+ 阿里云百炼流式 ASR；**enigo 逐字插入 + 百炼流式逐字上屏**；SQLite 历史 + 搜索；Fn 全局快捷键（Toggle / Hold PTT）；悬浮 HUD；AI 润色三档（Off / Light / Heavy，L0 规则纠错含 ITN / 热词同音模糊音 / 繁简 / 去句末标点 / 按 app 标点 + L2 LLM，本地 GGUF Qwen2.5-1.5B / 百炼云端三协议）；CSV 热词 + 风格包；选区注入；文件转录 SRT；日记导出；凭据钥匙串；本机性能 → 模型适配标签；界面 i18n（中 / 英）。

---

## 1. 两项目速览

### 1.1 OpenLess —— 同栈竞品，最直接对标

- **定位**：macOS + Windows 开源语音输入，对标 Typeless / Wispr Flow / Superwhisper。核心理念是「用嘴写 AI 提示词」——把零散口语重塑成结构化、可直接粘进 ChatGPT/Claude/Cursor 的文本。
- **技术栈**：**Tauri 2 + Rust + React/TS**，与 openIME 完全同栈。代码在 `openless-all/app/`，后端模块仅依赖 `types.rs`，接线干净。
- **ASR**：云端为主（火山引擎流式、OpenAI Whisper 兼容批量、Apple Speech、智谱 GLM-ASR、MiMo），本地走 vendored 的 [`Open-Less/qwen-asr`](https://github.com/Open-Less/qwen-asr)（C 语言 Qwen3-ASR 0.6B/1.7B），Windows 上还有 Foundry Local Whisper 变体。
- **LLM 润色**：Ark / DeepSeek / OpenAI / 豆包 / Anthropic / **Google Gemini**（原生 generateContent）/ **Codex OAuth** 多 provider，统一 OpenAI 兼容协议。
- **核心亮点**：
  - **风格包市场（Style Pack Marketplace）**——自定义系统提示词的「输出风格」，社区一键安装/发布/点赞。
  - **流式插入**——润色结果逐字符写入光标，带自动粘贴回退。
  - **选区问答面板（QA Panel）**——独立快捷键，对任意 app 中高亮选中的文本做语音问答。
  - **Less Computer / Voice Agent**——语音转写直接当指令交给无头 `claude -p` 跑，带护栏 + 内联审批 + git 快照。
  - 凭据存系统钥匙串（macOS Keychain / Windows 凭据管理器），应用内自动更新 + Beta 通道，5 语言 UI。
- **与 openIME 的异同**：架构理念几乎一致（核心库零 Tauri 依赖、trait 解耦），但 OpenLess 功能广度领先一个身位（市场、QA、翻译、voice agent、流式插入、Windows IME 集成），且云端 provider 矩阵更宽。openIME 的差异化优势在**本地 ASR 模型更丰富**（sherpa-onnx 四个离线模型 + 一键下载多源切换）和**本地 LLM**（GGUF Qwen2.5-1.5B 离线润色），OpenLess 本地 ASR 仅 Qwen3-ASR 一种、LLM 基本走云端。

### 1.2 CapsWriter-Offline —— Windows 离线极致，工程细节深

- **定位**：**Windows 专属、完全离线、低延迟**的语音输入 + 文件转录工具，追求「如臂使指」的硬件级响应、U 盘即插即用、保密机可用。
- **技术栈**：**Python + WebSocket C/S 架构**（`start_server.exe` 跑模型 + `start_client.exe` 跑交互）。客户端用 `pynput` 监听键鼠、`keyboard` 模拟打字；服务端 sherpa-onnx / 自研 GGUF 推理。明确不支持 macOS（`keyboard` 库已弃 macOS）。
- **ASR**：纯本地，引擎矩阵 Paraformer / SenseVoice-Small / Fun-ASR-Nano / Qwen3-ASR，ONNX + GGUF 双格式，DirectML/Vulkan 显卡加速。另有 CT-Transformer 标点、Qwen3-ForcedAligner 强制对齐。
- **核心亮点**：
  - **按住 CapsLock / 鼠标侧键 X2 说话**，松开即上屏；带「按键恢复」逻辑（CapsLock 状态被劫持后自动补发恢复），不破坏用户大小写锁定。
  - **基于音素（拼音）的模糊热词检索**——两阶段 RAG（FastRAG 倒排索引粗筛 + AccuRAG 模糊音精筛），「撒贝你→撒贝宁」「科大迅飞→科大讯飞」自动纠，支持前后鼻音/平翘舌权重。
  - **文件转录**——音视频拖到 exe 即出 `.srt`/`.txt`/`.json`（时间戳），分段+重叠切片。
  - **流式文本合并算法**（`text_merger.py`）——基于文本重叠 + 模糊匹配的鲁棒拼接，不依赖时间戳。
  - **数字 ITN**（「十五六个」→「15~16个」）、繁简转换、按 app 全/半角标点切换。
  - **LLM 角色**（`LLM/` 目录 `.py` 文件，识别结果开头匹配角色名即转交）、**日记归档**（按日期 Markdown + 音频链接）、**录音全程落盘**、UDP 广播/控制。
- **与 openIME 的异同**：技术栈差异最大（Python C/S vs Rust 单体），但它在「按键交互、后处理、热词、文件转录、归档」这些产品维度的工程深度是两家中最强的，很多细节值得借鉴思路（不一定照搬代码）。openIME 的本地 ASR 已对标其模型矩阵，但缺它后处理那一整套（ITN、音素热词、流式合并、标点适配）。

### 1.3 三方能力速查表

| 维度 | openIME | OpenLess | CapsWriter |
|---|---|---|---|
| 技术栈 | Tauri+Rust+React | Tauri+Rust+React | Python C/S |
| 平台 | macOS（优先） | macOS+Windows | Windows only |
| 触发键 | Fn（flagsChanged） | 组合键/CGEventTap | CapsLock/鼠标侧键(pynput) |
| 触发模式 | Toggle | Hold + Toggle + 线控 | Hold + 单击 + 对讲机 |
| 本地 ASR | sherpa-onnx ×4 模型 | Qwen3-ASR(C) | Paraformer/Sense/Fun/Qwen3 |
| 云端 ASR | 百炼流式 | 火山/Whisper/Apple/GLM | 无 |
| 本地 LLM 润色 | GGUF Qwen2.5-1.5B | 无（仅云端） | Ollama（角色） |
| 输入方式 | enigo 逐字 | enigo/CGEvent+粘贴 | keyboard.write/粘贴 |
| 流式上屏 | ❌（已回退） | ✅ 逐字符+回退 | ✅（流式 write） |
| 音素热词 | ❌（CSV 字面） | 字面+历史学习 | ✅ 两阶段 RAG |
| 文件转录 | ❌ | ❌ | ✅ srt/txt/json |
| 日记/归档 | SQLite 历史 | JSON 历史 | ✅ Markdown+音频 |
| 数字 ITN | ❌ | ❌ | ✅ |
| 翻译模式 | ❌ | ✅ | ❌（角色可做） |
| 划词问答 | ❌ | ✅ | ❌ |
| 风格包/市场 | ❌（有 Off/Light/Heavy） | ✅ 社区市场 | ✅（角色 .py） |
| Windows IME 集成 | ❌ | ✅ TSF profile+IPC | ❌ |
| 剪贴板恢复 | ❌ | ✅ | ✅ |
| Voice Agent | ❌ | ✅ claude -p | ❌ |

---

## 2. openIME 可借鉴 / 可做的功能清单

> 每条标注：来源（精确到文件/函数）、对 openIME 的价值、实现难度（按 openIME 现有 Rust+Tauri 架构评估，★越少越易）。

### A. 按键交互

#### A1. 按住说话（push-to-talk）+ Toggle 双模式
- **说明**：除现有「按一下开/再按一下停」外，增加「按住录音、松开停止」模式。长录音场景下松手即停更符合直觉。
- **来源**：OpenLess `openless-all/app/src-tauri/src/coordinator/dictation.rs` 的 `handle_pressed`/`handle_released`——用 `HotkeyMode::{Hold, Toggle}` 枚举分发；`HOTKEY_DEBOUNCE = 250ms` 防连按误触，Toggle 模式还有 `session_cooldown_until` 防三连按第 3 次误触。CapsWriter `config_client.py::ClientConfig.shortcuts` 的 `hold_mode` 字段 + `core/client/shortcut/event_handler.py::handle_keydown/handle_keyup`。
- **价值**：高。是语音输入最核心的交互选择，很多用户就是冲着 PTT 来的。
- **难度**：★★（openIME 已有 `platform/macos/fn_key.rs` 的 flagsChanged 监听，扩成 press/release 边沿 + 模式枚举即可，但 Fn 键的 release 检测要验证）。

#### A2. 短按/长按阈值 + 短按补发原按键
- **说明**：按下时间 < 阈值（如 0.3s）视为误触，取消录音并**异步补发原按键**（CapsLock 大小写状态、鼠标侧键原功能不丢失）。
- **来源**：CapsWriter `core/client/shortcut/shortcut_manager.py::_handle_mouse_keyup` + `event_handler.py::_handle_short_press`（`task.cancel()` 后 `self._pool.submit(self._emulator.emulate_key, key_name)`）；`emulator.py::ShortcutEmulator` 常驻 controller、`_emulating_keys` 集合做**防自捕获**（补发的按键不再被自己的监听器抓回来）。`config_client.py::ClientConfig.threshold = 0.3`。
- **价值**：中高。用 Fn 键时若误触会切走输入法，补发能修复体验。
- **难度**：★★★（macOS 上补发 Fn/🌐 键比较 tricky，需 NSEvent 重新 post；防自捕获要在 `fn_monitor.m` 里加标志位过滤）。

#### A3. 鼠标侧键 / 多键并发触发
- **说明**：支持鼠标 X1/X2 侧键触发录音，方便游戏/绘图场景手不离鼠标。
- **来源**：CapsWriter `shortcut_manager.py::create_mouse_filter`（`WM_XBUTTONDOWN/UP`，`xbutton = (data.mouseData >> 16) & 0xFFFF`）；`shortcut_config.py` 的 `type: 'mouse'`。
- **价值**：中。macOS 上需求弱于 Windows，但部分用户（带侧键鼠标）会要。
- **难度**：★★★（macOS 需 CGEventTap 监听 `kCGEventOtherMouseDown`，且要申请单独权限）。

#### A4. MediaPlayPause（耳机线控）触发
- **说明**：有线耳机的线控按一下开始/停止录音。
- **来源**：OpenLess README「MediaPlayPause 触发，让有线耳机的线控也能开始/停止录音」。
- **价值**：低中。小众但实现成本低。
- **难度**：★★（注册一个媒体键全局快捷键即可）。

### B. 转写后处理（openIME 最该补的短板）

#### B1. 数字 ITN（Inverse Text Normalization）
- **说明**：把口语数字转成书面阿拉伯数字，「十五六个」→「15~16个」、「百分之二十」→「20%」、「二零二六年」→「2026年」。
- **来源**：CapsWriter `core/tools/chinese_itn.py`（README 列为核心特性）；服务端还配 CT-Transformer 标点引擎 `core/server/engines/ct_transformer/punc_engine.py`。
- **价值**：高。这是中文语音输入最影响可用感的细节，openIME 现在直接出「十五六个」会很掉价。
- **难度**：★★（两条路：(a) 接 sherpa-onnx 自带的 ITN（FunASR 系模型支持）；(b) 用规则 + 正则在 `polish/sanitize.rs` 里做一层，CapsWriter 的实现就是规则驱动）。优先 (b)，可控可调。

#### B2. 音素（拼音）模糊热词纠错
- **说明**：热词不只字面匹配，而是把识别结果和热词都转成音素序列，按编辑距离模糊匹配，过阈值即强制替换。「撒贝你→撒贝宁」「科大迅飞→科大讯飞」「东方菜富→东方财富」。
- **来源**：CapsWriter `core/client/hotword/hot_phoneme.py::PhonemeCorrector`——**两阶段 RAG**：`rag_fast_rf.py::FastRAG`（倒排索引 + Numba JIT 粗筛，砍 90% 计算量）→ `algo_phoneme.py` + `algo_calc.py::fuzzy_substring_search_constrained`（带模糊音权重的边界约束模糊搜索）；双阈值 `hot_thresh=0.85`（替换）/ `hot_similar=0.6`（喂给 LLM 当上下文候选）；`manager.py::HotwordManager` 用 watchdog 监视 `hot.txt` 热加载（3s 防抖）。测试用例在 `hot_phoneme.py` 的 `__main__` 里。
- **价值**：高。专有名词（人名、公司名、产品名）是 ASR 重灾区，openIME 的 CSV 字面热词解决不了「音对字不对」。这一套是 CapsWriter 最核心的护城河。
- **难度**：★★★★（最难的一条。需要：拼音转换（`pypinyin` 的 Rust 对等物或自建映射表）+ 编辑距离模糊匹配 + 性能优化。Rust 生态没有现成的中文音素模糊匹配库，要么从 CapsWriter 的 `algo_phoneme.py`/`algo_calc.py` 移植算法，要么先用简单版：拼音首字母 + 声母韵母近似表）。建议先做**简化版**（拼音序列 + Levenshtein，不带倒排索引），跑通再优化。

#### B3. 规则替换（正则 / 等号）
- **说明**：`Claude=Cloud`、正则 `re.sub`，精准强制替换，和音素热词互补。
- **来源**：CapsWriter `core/client/hotword/hot_rule.py::RuleCorrector` + `hot-rule.txt`；openIME 已有 `polish/correction.rs`，思路一致但可对照 CapsWriter 的 `hot-rule.txt` 语法（等号 + 正则双模式）。
- **价值**：中。openIME 已有规则纠错底子，补全语法和 UI 即可。
- **难度**：★（已有基础，扩展现有 `correction.rs`）。

#### B4. 末尾标点去除（trash_punc）
- **说明**：识别结果末尾的「，。」自动去掉（单句输入不需要句末标点）。
- **来源**：CapsWriter `core/client/output/text_output.py::TextOutput.strip_punc`（`re.sub(f"(?<=.)[{Config.trash_punc}]$", "", text)`，`trash_punc='，。,.'`）；流式输出时还会从右向左找首个非 trash 字符做边界（`llm_output_typing.py::stream_write_chunk`）。
- **价值**：高，成本极低。语音输入「你好。」结尾带句号很违和。
- **难度**：★（一行正则，加在 `polish/sanitize.rs`）。

#### B5. 按 app 切换全/半角标点
- **说明**：在微信/Telegram 里自动把全角「，。」转半角「, 」更协调。
- **来源**：CapsWriter `core/tools/punc_converter.py`（`FULL_TO_HALF` 映射 + `should_convert_punctuation(window_title, keywords)` 按 `paste_apps=['WeiXin.exe','Telegram.exe']` 判定）。
- **价值**：中。中文 IM 场景的细节体验。
- **难度**：★（openIME 已能拿到前台 app，加一张 app→标点偏好表即可）。

#### B6. 繁简转换（字形偏好）
- **说明**：用户可强制输出简体或繁体，不受 ASR 原始字形影响。
- **来源**：OpenLess `coordinator/llm_pipeline.rs::apply_chinese_script_preference`（用 OpenCC，`ChineseScriptPreference::{Auto, Simplified, Traditional}`，且流式输出遇到强制字形偏好时**关闭流式**改走一次性转换路径）；CapsWriter `config_client.py::ClientConfig.traditional_convert / traditional_locale`。
- **价值**：中。港澳台用户刚需。
- **难度**：★★（Rust 有 `opencc-rs`/`fast2s2t` crate，集成简单；难点在流式插入时字形转换要缓冲）。

### C. 输入方式

#### C1. 流式逐字插入（带自动回退）—— 重新考虑
- **说明**：润色结果边生成边逐字敲入光标，降低感知延迟；某 app 不接受流式按键时自动回退为一次性粘贴。openIME 之前因「不稳定」回退掉了，但 OpenLess 给出了更稳的实现。
- **来源**：OpenLess `coordinator/llm_pipeline.rs::polish_or_passthrough_streaming`（`StreamingPolishOutcome::{Streamed, UnsupportedFallback, Failed}` 三态）+ `coordinator/dictation.rs` 的 `drain_streaming_insert_deltas_with`/`flush_streaming_insert_buffer_with`（channel + 50ms 批量 flush，保证 Unicode 字符边界完整，见 `append_typed_prefix_keeps_unicode_char_boundaries` 测试）；`unicode_keystroke.rs` 封装了平台 Unicode 按键（macOS CGEvent / Windows SendInput `KEYEVENTF_UNICODE`，16 字符一批、12ms 间隔）。
- **价值**：高。这是「快」的体感核心，openIME 一次性插入在长文本时等待感明显。
- **难度**：★★★（openIME 有 enigo 逐字基础；难点在 Unicode 边界、批量 flush、失败回退三态机。建议照搬 OpenLess 的 `StreamingPolishOutcome` 三态模型和 `append_typed_prefix` 的 char_boundary 处理）。

#### C2. 剪贴板恢复（粘贴后还原用户原剪贴板）
- **说明**：粘贴插入后，延时 750ms 把用户原来的剪贴板内容还原回去，不破坏用户复制的内容。
- **来源**：OpenLess `insertion.rs`——`copy_to_clipboard_with_restore_plan` 先存原值，`schedule_clipboard_restore` 起线程延时恢复，`PENDING_CLIPBOARD_RESTORE` + `restore_id` 保证多次插入只恢复最早的原始值，`should_restore_clipboard` 校验剪贴板仍是刚插入的文字才恢复（用户中途改了就不动）。CapsWriter `text_output.py::_paste_text` 也做了（`Config.restore_clip` + sleep 0.1s）。
- **价值**：高。openIME 现在粘贴会覆盖用户剪贴板，是真实痛点。
- **难度**：★★（`arboard` crate 已可用；关键是 OpenLess 那套「只在剪贴板仍是插入文字时才恢复」的校验，避免覆盖用户中途的复制）。

#### C3. Windows IME TSF 集成（为跨平台预留）
- **说明**：在 Windows 上注册一个自己的 TSF 输入法 profile，通过 IPC 让 IME 直接 `CommitText`，比模拟按键更稳（不抢焦点、不受目标 app 输入法状态干扰）。
- **来源**：OpenLess `src-tauri/src/windows_ime_session.rs::WindowsImeSessionController`（`prepare_session` 存当前输入法 profile → 激活 OpenLess profile → `submit_prepared` 走 IPC → `restore_session` 还原原 profile）+ `windows_ime_ipc.rs`/`windows_ime_protocol.rs`/`windows_ime_profile.rs`/`windows-ime/`（C++ IME DLL，`nsis/openless-ime-hooks.nsh` 做安装挂钩）。失败时 `should_fallback_after_ime_result` 回退到模拟按键。
- **价值**：中（openIME 当前 macOS 优先，暂不需要；但若做 Windows 版，这是比模拟按键更优的路径）。
- **难度**：★★★★★（需要写 C++ TSF IME DLL + 安装器挂钩，工程量大。macOS 不适用）。

#### C4. Linux fcitx5 commit（为跨平台预留）
- **说明**：Linux 上优先走 fcitx5 插件 `CommitText`，失败回退剪贴板粘贴。
- **来源**：OpenLess `insertion.rs::insert_with_fcitx_or_clipboard_fallback` + `linux_fcitx.rs::commit_text` + `openless-all/scripts/inject-fcitx5-plugin.sh` + `linux-fcitx5-plugin/`。
- **价值**：低（openIME 暂无 Linux 计划）。
- **难度**：★★★。

### D. 历史管理 / 归档 / 回放

#### D1. 日记归档（Markdown + 音频回放链接）
- **说明**：每句语音按 `年/月/日.md` 归档，条目格式 `[HH:MM:SS](音频相对路径) 识别文本`，音频文件全程落盘，可在 Markdown 里直接点链接回放。文件头还附了「文件链接 ↔ HTML audio 控件」互转正则 Tip。
- **来源**：CapsWriter `core/client/diary/diary_writer.py::DiaryWriter.write`（`time.localtime` 拆年月日、`makedirs`、相对路径空格转 `%20`）+ `core/client/audio/file_manager.py::AudioFileManager`（`Config.save_audio=True`，文件名取识别结果前 20 字）。
- **价值**：中高。语音输入的「日记/速记」场景（会议、灵感）很需要，且音频可回溯纠错。比纯 SQLite 更人性化。
- **难度**：★★（openIME 有 SQLite 历史，加一个「导出为日记 Markdown」+ 可选音频落盘即可；音频要扩 `audio.rs` 的采集缓存到 wav）。

#### D2. 历史增强（复制 / 重插 / 重新润色 / 搜索）
- **说明**：历史条目支持复制按钮、重新插入到光标、换档重润色、全文搜索。
- **来源**：OpenLess README「路线图」明确列了这些（尚未发布）；前端 `src/pages/History.tsx`。CapsWriter 托盘菜单有「复制结果」（`core/client/manager/tray_manager.py`）。
- **价值**：中高。openIME 的 `History.tsx` 目前偏只读，这些是高频小功能。
- **难度**：★★（前端工作为主，后端加几个 command）。

#### D3. 文件转录（拖拽音视频→字幕）
- **说明**：把音视频文件拖到客户端，输出 `.srt`（字幕）+ `.txt`（按标点切分）+ `.json`（带时间戳 token），用于会议录音、视频上字幕。
- **来源**：CapsWriter `core/client/transcribe/file_transcriber.py::FileTranscriber`——`MediaTool.build_ffmpeg_cmd` 起 ffmpeg 进程按 1 分钟分块（`chunk_size = 16000*4*60`）流式读 → base64 发服务端 → `result_handler.py::ResultHandler.save_results` 落三种格式；`srt_adjuster.py` 调时间戳；`docs/文件转录功能如何使用.md`。配置 `file_seg_duration/overlap`、`file_save_srt/txt/json/merge`。
- **价值**：高。这是 CapsWriter 区别于纯听写工具的最大差异化功能，能直接拓宽 openIME 的使用场景（播客/会议/视频创作者）。openIME 的 ASR 引擎本就支持，缺的是 ffmpeg 解码 + 分段 + srt 封装。
- **难度**：★★★（ffmpeg 调用 + 分段重叠，openIME 的百炼流式/本地 ASR 都能复用；srt/json 格式化是纯字符串处理。难点在长音频的分段合并与时间戳对齐）。

### E. 音频 / 录音细节

#### E1. 分段 + 重叠切片（长录音稳定识别）
- **说明**：麦克风/文件按固定时长分段（60s）发 ASR，相邻段重叠（4s），避免边界丢字。
- **来源**：CapsWriter `config_client.py::ClientConfig.mic_seg_duration=60 / mic_seg_overlap=4 / file_seg_*`；`recorder.py::AudioRecorder.record_and_send` 按 `seg_duration/overlap` 组装 `AudioMessage`。
- **价值**：中。百炼流式本身不限长，但本地 sherpa 离线模型长音频易截断，分段+重叠能稳。
- **难度**：★★（配合 B6 的流式合并算法一起做）。

#### E2. 短录音阈值缓存（防误触空录）
- **说明**：按下后前 `threshold`（0.3s）的音频先缓存不发，松开时若没超过阈值就当误触丢弃。
- **来源**：CapsWriter `recorder.py`——`if task['time'] - self._start_time < Config.threshold: self._cache.append(...); continue`；`event_handler.py::_handle_short_press` 短按 cancel。
- **价值**：中。避免误触产生空会话。
- **难度**：★。

#### E3. 短按取消（< 阈值不录）
- **说明**：见 A2，录音时长 < 阈值则取消、不上屏。
- **来源**：CapsWriter `event_handler.py::handle_keyup`（`if duration < task.threshold: task.cancel()`）。
- **价值**：中。
- **难度**：★（和 A2 同源）。

### F. 配置 / 插件 / 扩展性

#### F1. 风格包（自定义系统提示词）+ 快捷键切换
- **说明**：用户建多个「风格包」（commit message / 客服回复 / 小红书文案 / 正式报告），每个绑一套系统提示词，用快捷键在运行时切换当前生效风格。openIME 现在的 Off/Light/Heavy 三档是固定 prompt，可升级为「用户自定义风格包」。
- **来源**：OpenLess `src-tauri/src/commands/style_packs.rs` + `types.rs::StylePack`（`StylePackKind::{Builtin, User}`，`raw_style_pack_uses_llm` 判断是否走 LLM）+ `coordinator/llm_pipeline.rs` 把 `style_system_prompt` 注入润色；前端 `src/pages/Style.tsx`。
- **价值**：高。这是把「润色」从黑盒三档变成「可编程输出风格」的关键升级，直接提升留存。
- **难度**：★★（openIME 已有 `polish/prompts.rs` 和三档 router，把固定 prompt 表改成用户可增删的 StylePack 表 + 一个切换快捷键即可，后端改动小）。

#### F2. 风格包市场（社区分享 / 一键安装 / 发布）
- **说明**：浏览/搜索/安装/点赞社区风格包，用 GitHub 登录后可发布自己的（经审核）。
- **来源**：OpenLess `src-tauri/src/commands/marketplace.rs` + `commands/github_oauth.rs` + 前端 `src/pages/Marketplace.tsx`、`components/MarketplaceModal.tsx`、`GithubLoginModal.tsx`。README 说明市场由「自有审核后端」提供。
- **价值**：中（产品冷启动期不是刚需，但增长阶段很杀）。
- **难度**：★★★★（需要后端服务 + GitHub OAuth + 审核 workflow，工程和组织成本都高。建议 F1 做完后视用户量再考虑，或先做「风格包导入/导出 JSON 文件」的轻量分享）。

#### F3. LLM 角色（前缀触发，文件即插件）
- **说明**：识别结果开头匹配某角色名（如「翻译 ...」「命令 ...」）就把后续文本交给该角色的 system prompt 处理。角色用 `LLM/` 目录下的 `.py` 文件定义（带 `RoleConfig`：provider/model/temperature/system_prompt/输出模式/是否带历史/是否读选区），**热加载**（文件监视）。
- **来源**：CapsWriter `core/client/llm/llm_handler.py::LLMHandler.detect_role` + `llm_role_detector.py` + `llm_role_loader.py`（动态加载 `LLM/*.py`）+ `llm_role_config.py::RoleConfig`（字段极全：`match`/`process`/`enable_history`/`enable_read_selection`/`output_mode=typing|toast`/toast 样式）；`llm_watcher.py` 监视角色目录热重载（保留历史记录）；预置 `LLM/default.py`/`小助理.py`/`翻译.py`。`docs/角色功能如何使用.md`。
- **价值**：中高。比 F1 的「全局风格切换」更灵活——**同一会话里靠前缀分流**，且每个角色可独立配置 provider/model/历史。和 openIME 已放弃的「人设」不同，这是「指令路由」。
- **难度**：★★★（Rust 里做 `.py` 热加载不现实，但可改成 TOML/JSON 角色配置 + 前缀正则匹配；openIME 的 `polish/router.rs` 已有路由雏形，扩展成「前缀→角色」路由即可）。

#### F4. 选区注入（读当前选中文字作为 LLM 上下文）
- **说明**：润色/问答时把前台 app 当前高亮选中的文字抓出来，作为 LLM 的上下文（「帮我改一下这段」+ 选中那段）。
- **来源**：OpenLess `src-tauri/src/selection.rs::capture_selection`——**三级 fallback**：(1) macOS AX 直读 `AXSelectedText`（不碰剪贴板）；(2) 模拟 Cmd+C + sentinel 哨兵 + 80ms 后读取 + 还原；(3) Linux `wl-paste/xclip/xsel` 读 PRIMARY。超 4000 字截断首 2000+尾 2000。CapsWriter `llm_get_selection.py::get_selected_text`（角色配置 `enable_read_selection`）。
- **价值**：中高。是 QA/改写场景的基础能力，openIME 要做「选中一段让 AI 改」就绕不开。
- **难度**：★★（macOS AX API 直读那条路径最干净，和 openIME 已有的 `platform/macos/app_focus.m` 权限同源；模拟 Cmd+C fallback 可照搬 selection.rs 的 sentinel 哨兵防误判）。

### G. 新交互形态

#### G1. 划词语音问答（QA Panel）
- **说明**：独立快捷键（OpenLess 默认 `Cmd+Shift+;`）打开浮动面板，抓取当前选中文字作为上下文，语音提问→LLM 流式回答，支持多轮（messages 累积，关浮窗清空）。和主听写互不干扰（路由判断 dictation 是否在跑）。
- **来源**：OpenLess `coordinator/qa.rs::QaSessionState`（`phase/panel_visible/pinned/messages`）+ `handle_qa_hotkey_pressed`（toggle 浮窗）+ `handle_qa_option_edge`（Option 键在浮窗内控制录音）+ `coordinator/qa_session.rs`（抓选区→转写→带选区上下文流式问答）；前端 `src/pages/QaPanel.tsx`/`SelectionAsk.tsx`。
- **价值**：中。属于「进阶功能」，但能拉开和纯听写工具的差距。
- **难度**：★★★（依赖 F4 的选区捕获 + 一个独立 Tauri 窗口 + 多轮对话状态机。openIME 的悬浮 HUD 可扩展成此面板）。

#### G2. 翻译模式（独立快捷键）
- **说明**：按住独立快捷键用源语言说，直接插入为目标语言。OpenLess 还做了「润色+翻译」单次调用（两段哨兵标记 `[[OPENLESS_POLISHED_SOURCE]]` / `[[OPENLESS_TRANSLATION]]` 让模型一次输出润色源文+译文）。
- **来源**：OpenLess `coordinator/llm_pipeline.rs::polish_and_translate_or_passthrough` + `build_polish_translate_system_prompt` + `split_polish_translate_output`（解析失败退回专用 `translate_text`）；前端 `src/pages/Translation.tsx`。
- **价值**：中。跨语言工作者有需求。
- **难度**：★★（prompt 工程 + 一个快捷键绑定，openIME 的 `polish/cloud.rs` 可直接加 translate 方法）。

#### G3. Voice Agent（语音→编码 Agent）
- **说明**：语音转写不插入光标，而是作为指令交给无头 `claude -p` 在设定 workdir 跑，结果在聊天浮窗展示；运行前做 git 快照可回滚，带护栏（高风险命令 deny 清单）+ 内联审批（拦截后弹卡，用户 Approve 则放行该风险模式重跑）。
- **来源**：OpenLess `coordinator/dictation_voice_agent.rs::run_voice_agent_transcript` + `run_less_computer_once`（写临时 settings.json 护栏配置，fail-closed）+ `maybe_request_approval`（扫描终局文本命中「denied/权限/被拦」+ 高风险模式 → oneshot 等用户决断 90s）+ `coding_agent/guard.rs`（`default_deny_rules`/`HIGH_RISK_PATTERNS`/`risk_equivalent_patterns` 等价组放行）。
- **价值**：低中（开发者向，和 openIME「输入法」定位稍远，但作为高级模式有想象空间）。
- **难度**：★★★★（依赖外部 `claude` CLI，护栏+审批工程量大）。

### H. 工程质量 / 安全

#### H1. LLM endpoint SSRF 校验
- **说明**：用户自定义的 ASR/LLM endpoint 是 attacker-controlled，配置时做 host/IP 校验：拒绝云元数据（169.254.169.254）、CGNAT、link-local；公网强制 https；**主动放行 RFC1918 局域网 http**（支持自托管 ollama/Whisper）。
- **来源**：OpenLess `coordinator/llm_pipeline.rs::validate_llm_endpoint` + `guard_asr_http_endpoint`（fail-closed，被拒回退安全默认值）。
- **价值**：中。openIME 用户也会填自建 endpoint，防 SSRF 是负责任的做法。
- **难度**：★★（纯 IP 段判断逻辑，Rust 里好写）。

#### H2. 凭据存系统钥匙串
- **说明**：API key 存 macOS Keychain / Windows 凭据管理器，不落明文 JSON。
- **来源**：OpenLess README「凭据」段（`service=com.openless.app`，旧明文 JSON 仅作迁移源读取后删除）。
- **价值**：高。openIME 若存明文配置应迁移。
- **难度**：★★（macOS 用 `security` CLI 或 `keyring` crate）。

#### H3. ESC 中断 LLM 输出
- **说明**：LLM 流式输出时按 ESC 立即中断（停止烧 token），已输出的部分保留。
- **来源**：CapsWriter `core/client/llm/llm_stop_monitor.py::StopMonitor` + `llm_handler.py` 的 `should_stop_check` 回调；`config_client.py::ClientConfig.llm_stop_key='esc'`。
- **价值**：中。LLM 跑飞时能止损。
- **难度**：★（openIME 流式调用加一个 cancel flag 即可）。

#### H4. 单实例锁
- **说明**：防止两个进程争抢同一快捷键边沿。
- **来源**：OpenLess README「单实例锁」。
- **价值**：中。
- **难度**：★（Tauri 有 `tauri-plugin-single-instance`）。

---

## 3. 优先级建议

按「用户价值 × 实现性价比」排序。★=难度（少=易），价值=高/中/低。

### 🔴 高优先级（投入小、体感提升大、契合 openIME 定位）

| # | 功能 | 价值 | 难度 | 理由 |
|---|---|---|---|---|
| 1 | **末尾标点去除 B4** | 高 | ★ | 一行正则，立刻消除「你好。」的违和感，投入产出比最高 |
| 2 | **剪贴板恢复 C2** | 高 | ★★ | 粘贴插入覆盖用户剪贴板是真痛点，OpenLess 的「校验后才恢复」逻辑可直接照搬 |
| 3 | **数字 ITN B1** | 高 | ★★ | 中文语音输入最影响可用感的细节，规则驱动版即可覆盖 80% 场景 |
| 4 | **风格包 F1** | 高 | ★★ | 把固定三档升级为可编程输出风格，openIME 已有 router/prompts 基础，改动小、留存提升大 |
| 5 | **按住说话 PTT A1** | 高 | ★★ | 核心交互选项，openIME 已有 Fn 监听，扩 press/release 边沿即可 |

### 🟡 中优先级（价值高但需一定投入，或价值中等但便宜）

| # | 功能 | 价值 | 难度 | 理由 |
|---|---|---|---|---|
| 6 | **历史增强 D2** | 中高 | ★★ | 复制/重插/搜索是高频小功能，前端为主 |
| 7 | **文件转录 D3** | 高 | ★★★ | 拓宽使用场景（会议/播客/字幕），ASR 引擎已就绪，缺 ffmpeg+srt 封装 |
| 8 | **选区注入 F4** | 中高 | ★★ | macOS AX 直读路径干净，是改写/QA 的基础能力 |
| 9 | **日记归档 D1** | 中高 | ★★ | 速记场景刚需，音频回放可纠错，基于现有 SQLite 扩展 |
| 10 | **流式逐字插入 C1** | 高 | ★★★ | openIME 曾放弃，OpenLess 的三态机+Unicode 边界处理给出了更稳方案，值得重做 |
| 11 | **按 app 切标点 B5** | 中 | ★ | 依赖已能拿前台 app，成本极低 |
| 12 | **繁简转换 B6** | 中 | ★★ | 港澳台用户刚需，opencc-rs 集成简单 |
| 13 | **ESC 中断 H3** | 中 | ★ | 流式调用加 cancel flag，几行代码 |
| 14 | **凭据钥匙串 H2** | 高 | ★★ | 安全责任，macOS keyring crate |
| 15 | **翻译模式 G2** | 中 | ★★ | prompt 工程+快捷键，cloud.rs 扩展 |

### 🟢 低优先级（投入大 / 小众 / 与定位稍远）

| # | 功能 | 价值 | 难度 | 理由 |
|---|---|---|---|---|
| 16 | **音素模糊热词 B2** | 高 | ★★★★ | 价值极高但工程量最大，Rust 无现成库，建议先做拼音+Levenshtein 简化版验证价值再投入完整 RAG |
| 17 | **LLM 角色 F3** | 中高 | ★★★ | 改 TOML 配置 + 前缀路由可实现，但和 F1 风格包定位有重叠，需想清关系 |
| 18 | **划词 QA G1** | 中 | ★★★ | 依赖 F4 + 独立窗口 + 多轮状态机 |
| 19 | **短按补发 A2** | 中高 | ★★★ | macOS 补发 Fn 键 tricky |
| 20 | **风格包市场 F2** | 中 | ★★★★ | 需后端+OAuth+审核，增长期再做；可先做 JSON 导入导出 |
| 21 | **SSRF 校验 H1** | 中 | ★★ | 该做但用户感知低 |
| 22 | **鼠标侧键 A3** | 中 | ★★★ | macOS 需求弱 |
| 23 | **Windows IME C3** | 中 | ★★★★ | 仅 Windows 版需要 |
| 24 | **Voice Agent G3** | 低中 | ★★★★ | 开发者向，偏离输入法定位 |
| 25 | **分段重叠 E1** | 中 | ★★ | 本地模型长音频才需要，配合 D3 做 |
| 26 | **线控触发 A4** | 低中 | ★★ | 小众 |

### 🏆 Top 3 最值得马上做

1. **末尾标点去除 + 数字 ITN（B4 + B1）**——成本最低、用户每次输入都受益，是「好不好用」的体感地基。先把 `polish/sanitize.rs` 扩成一套后处理管线。
2. **剪贴板恢复（C2）**——直接消除 openIME 粘贴插入的最大副作用，照搬 OpenLess `insertion.rs` 的 restore_plan + 校验逻辑即可。
3. **风格包（F1）**——把 Off/Light/Heavy 升级为用户自定义系统提示词包 + 快捷键切换，这是把 openIME 从「听写工具」拉到「AI 写作助手」的最小一步，且 openIME 的 `polish/router.rs`+`prompts.rs` 已有扩展点。

---

## 4. 不建议照搬的部分

### 4.1 Python C/S 架构（CapsWriter 整体）
CapsWriter 的 `start_server.py`/`start_client.py` 双进程 + WebSocket 通信（`core/client/connection/websocket_manager.py`、`core/server/connection/`）是为 Python 场景设计的（模型重、放服务端进程隔离；Win7 老电脑跑不了模型也能跑客户端）。openIME 是 Tauri 单体 + `voice-core` 库内调用，**没有进程隔离需求**，照搬 C/S 只会徒增复杂度。可借鉴的是它的**协议设计**（`core/protocol.py::AudioMessage` 的 `task_id/source/is_final/seg_duration/seg_overlap/context` 字段）和**分段思路**，但实现上保持进程内 trait 调用。

### 4.2 `keyboard` / `pynput` 模拟按键（CapsWriter）
CapsWriter 用 `keyboard.write()`（`core/client/output/text_output.py::_type_text`）和 `pynput` 模拟。这些是 Python 库，且 `keyboard` 库已弃 macOS。openIME 已用 `enigo`（Rust），且 OpenLess 验证了 macOS 用 CGEvent 直接 post、Windows 用 SendInput `KEYEVENTF_UNICODE` 更稳（`insertion.rs::windows_unicode::send_text` 16 字符分批、`unicode_keystroke.rs`）。**保持 enigo + 平台原生 CGEvent/SendInput**，不要换。

### 4.3 `.py` 文件作为角色/插件载体（CapsWriter LLM 角色）
CapsWriter 的角色是 `LLM/小助理.py`、`LLM/翻译.py` 这种 Python 文件，靠 `llm_role_loader.py` 动态 import（带 `RoleConfig` 类）。Rust 里做动态脚本加载既不安全也不现实。**改为 TOML/JSON 角色配置**（`name/match_prefix/system_prompt/provider/model/temperature`），前缀用正则匹配。

### 4.4 TSF IME DLL（OpenLess Windows IME）
`windows_ime_session.rs` + C++ TSF IME + NSIS 安装挂钩（`windows-ime/`、`nsis/openless-ime-hooks.nsh`）是 Windows 专属的重量级工程，openIME 当前 macOS 优先，**暂不建议碰**。等做 Windows 版时再评估，且可先用「模拟 Cmd/Ctrl+V + 剪贴板恢复」过渡。

### 4.5 Voice Agent + 无头 `claude -p`（OpenLess Less Computer）
`dictation_voice_agent.rs` 依赖外部 `claude` CLI、护栏 JSON、git 快照、内联审批，是重度开发者功能，和 openIME「语音输入法」定位偏离。**不建议短期纳入**，除非产品方向明确转向「语音操作系统」。

### 4.6 CapsWriter 的 UDP 广播/控制
`core/client/udp/udp_broadcaster.py`/`udp_control.py`（`udp_broadcast_targets`、外部程序发 START/STOP 命令控制录音）是 CapsWriter 为外接硬件/其他程序留的口子，openIME 作为 GUI 输入法用不上。

### 4.7 流式文本合并算法（CapsWriter `text_merger.py`）—— 视情况
`merge_by_text`（重叠窗口 + 模糊匹配拼接）是 CapsWriter 为**非流式分段 ASR** 设计的（每段独立识别再拼）。openIME 用百炼**流式 ASR**（天然连续），本地 sherpa 若用 streaming 模型也不需要；**只有做文件转录 D3 且本地模型是非流式时**才用得上。不必预先引入。

---

## 附录：关键文件索引（方便日后查阅原仓库）

### OpenLess（`Open-Less/openless`，路径前缀 `openless-all/app/src-tauri/src/`）
- 快捷键状态机：`coordinator/dictation.rs`（handle_pressed/released、HOTKEY_DEBOUNCE）
- 流式润色三态：`coordinator/llm_pipeline.rs`（StreamingPolishOutcome、apply_chinese_script_preference、validate_llm_endpoint）
- 文本插入：`insertion.rs`（剪贴板恢复 restore_plan、CGEvent Cmd+V、Windows SendInput Unicode）
- 选区捕获：`selection.rs`（macOS AX、模拟 Cmd+C sentinel、截断）
- QA 面板：`coordinator/qa.rs`（QaSessionState、open/close panel）
- Voice Agent：`coordinator/dictation_voice_agent.rs`（护栏、审批、git 快照）
- 风格包：`commands/style_packs.rs`、`types.rs::StylePack`
- 市场：`commands/marketplace.rs`、`commands/github_oauth.rs`
- Windows IME：`windows_ime_session.rs`、`windows_ime_ipc.rs`、`windows_ime_profile.rs`、`windows_ime_protocol.rs`
- Linux fcitx5：`linux_fcitx.rs`
- Unicode 按键：`unicode_keystroke.rs`
- 凭据：`commands/credentials.rs`、`persistence.rs::CredentialsVault`
- Coding Agent 护栏：`coding_agent/guard.rs`、`coding_agent/commands.rs`
- 前端：`src/pages/{History,Style,Marketplace,QaPanel,Translation,Vocab}.tsx`、`src/components/Capsule.tsx`

### CapsWriter（`HaujetZhao/CapsWriter-Offline`，路径前缀 `core/`）
- 快捷键：`client/shortcut/shortcut_manager.py`（键盘+鼠标 filter、防自捕获）、`event_handler.py`（短按/长按）、`emulator.py`（补发按键）、`task.py`、`shortcut_config.py`、`key_mapper.py`
- 音素热词：`client/hotword/hot_phoneme.py`（PhonemeCorrector 两阶段 RAG）、`hot_rule.py`、`rag_fast_rf.py`、`rag_accu.py`、`algo_phoneme.py`、`algo_calc.py`、`manager.py`（watchdog 热加载）
- LLM 角色：`client/llm/llm_handler.py`（LLMHandler 协调器）、`llm_role_config.py`（RoleConfig 全字段）、`llm_role_loader.py`、`llm_role_detector.py`、`llm_context.py`（多轮历史）、`llm_stop_monitor.py`（ESC 中断）、`llm_output_typing.py`（流式打字）、`llm_output_toast.py`、`llm_get_selection.py`、`llm_watcher.py`（角色热重载）；预置 `LLM/default.py`/`小助理.py`/`翻译.py`
- 文件转录：`client/transcribe/file_transcriber.py`、`media_tool.py`（ffmpeg）、`result_handler.py`、`srt_adjuster.py`
- 日记：`client/diary/diary_writer.py`、`client/audio/file_manager.py`
- 录音：`client/audio/recorder.py`（分段+阈值缓存）、`stream.py`
- 输出：`client/output/text_output.py`（strip_punc、粘贴/打字）、`result_processor.py`
- 后处理工具：`tools/punc_converter.py`（全半角）、`tools/chinese_itn.py`（数字 ITN）、`tools/format_tools.py`
- 流式合并：`server/merger/text_merger.py`（文本重叠拼接）、`token_merger.py`、`server/engines/ct_transformer/punc_engine.py`（标点）
- 配置：`config_client.py`（ClientConfig 全字段）、`config_server.py`
- 协议：`core/protocol.py`（AudioMessage/RecognitionMessage）
- 引擎：`server/engines/{sensevoice_onnx,paraformer_onnx,fun_asr_gguf,qwen_asr_gguf,ct_transformer,force_aligner_gguf,llama}/`
- 文档：`docs/{热词功能如何使用,角色功能如何使用,文件转录功能如何使用,text_merge_algorithm}.md`
