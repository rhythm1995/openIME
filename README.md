# openIME

跨平台语音输入法（一期：macOS）。常驻菜单栏，按全局快捷键录音 → 实时流式转写 → 在当前光标逐字输入 → 按会话保存历史。

参考 [AutoGLM（小凹）](https://github.com/) 的产品形态，用 Tauri + Rust 重构为开源、可本地化、可测试的实现。

## 引擎

| 引擎 | 说明 | 状态 |
|---|---|---|
| **阿里云百炼 Protocol A** | 流式 WebSocket，模型 `fun-asr-realtime` / `paraformer-realtime-v2`。对齐 [alibabacloud-bailian-speech-demo](https://github.com/aliyun/alibabacloud-bailian-speech-demo) | ✅ 已实现 + WS mock 集成测试 |
| **本地 sherpa-onnx** | 进程内 Paraformer-online + Silero VAD，离线，feature 门控（`--features sherpa`） | ✅ 已实现 + 应用内一键下载模型 |

用户在设置中填写云端 API（workspace_id + api_key + model），或选用本地引擎并一键下载模型。

### 本地模型一键安装

设置页选择 `sherpa-onnx（本地，离线）` 引擎后出现「本地模型」卡片，点击
「下载并安装模型」：流式下载 + 实时进度条 + SHA256 校验 + 断点续传 + 多源故障切换
（HuggingFace 官方 → hf-mirror 国内镜像）。模型为
`csukuangfj/sherpa-onnx-streaming-paraformer-bilingual-zh-en`（int8，约 227MB）
+ Silero VAD（约 0.6MB），安装到 `app_data_dir/models/`，装完即可离线使用。

## 录音快捷键

默认 **Fn（🌐 键）**：按一下开始录音，再按一下停止。Fn 是修饰键，标准全局快捷键
API 无法注册，由原生 NSEvent flagsChanged 监听实现（见
`src-tauri/src/platform/macos/fn_key.rs`）。也可在设置里改为组合键（如
`Alt+Shift+D`），保存后立即生效。

> 注意：若系统设置里「按下 🌐 键」被设为切换输入法/显示表情，按 Fn 时系统动作
> 仍会执行（不影响录音切换）；可在 系统设置 → 键盘 里改为「不执行任何操作」。

## 架构

核心逻辑全部在 `voice-core`（零 Tauri 依赖，纯库），四个可 mock 的 trait 串成端到端管线：

```
AudioSource ──► AsrProvider/AsrSession ──► TextInserter
   (cpal)          (百炼 WS / sherpa)         (enigo)
                        │
                        ▼
                  HistoryStore (SQLite)
```

```
openIME/
├── crates/voice-core/        # 核心库（全部逻辑 + trait）
│   ├── src/
│   │   ├── traits.rs         # AudioSource / AsrProvider / AsrSession / TextInserter / HistoryStore
│   │   ├── config.rs         # AppConfig / ProviderConfig
│   │   ├── store.rs          # SqliteStore + 迁移
│   │   ├── bailian_proto.rs  # 百炼协议帧（run-task/result-generated 编解码）
│   │   ├── providers/        # bailian.rs / sherpa.rs
│   │   ├── audio.rs          # cpal 采集 + rubato 重采样
│   │   ├── insert.rs         # enigo 文本插入
│   │   ├── pipeline.rs       # 端到端编排
│   │   ├── model_mgr.rs      # 模型下载/SHA256/解压
│   │   └── permissions.rs    # 权限模型
│   └── tests/                # traits 契约 + 百炼 WS mock 集成
└── src-tauri/                # Tauri 薄壳
    ├── src/                  # commands + state + platform/macos 权限
    └── tauri.conf.json       # 主窗口 + overlay 悬浮窗 + 托盘
└── src/                      # React 18 + TS 前端
    ├── App.tsx               # 设置/历史/Onboarding
    ├── RecorderOverlay.tsx   # 悬浮窗（实时转写）
    └── components/           # Settings / History / Onboarding
```

## 开发

```bash
# 核心库（快，无 GUI）
cargo test -p voice-core
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# 全量（含 src-tauri 编译）
cargo test --workspace

# 前端
pnpm install
pnpm test          # Vitest + React Testing Library
pnpm build         # tsc + vite

# 启动（需先 pnpm build 出 dist/）
cargo run -p openime
```

本地 sherpa 引擎（默认开启，与设置页「本地引擎推荐」一致）：

```bash
cargo test -p voice-core --features sherpa
# 打包：./scripts/build.sh   # 默认 WITH_SHERPA=1
# 仅云端：WITH_SHERPA=0 ./scripts/build.sh
```

## 启动行为

- **正常启动**（Dock / Spotlight / `open`）：自动显示主面板。
- **开机自启**：设置页打开「开机自启」后，macOS 登录时经 LaunchAgent 以
  `--autostart` 参数启动，应用静默常驻菜单栏、不弹面板。
- 两种模式都会创建托盘（菜单栏）图标，随时可从托盘「设置/历史」打开面板。

## macOS 权限与代码签名（已固定身份）

应用需要两项系统权限：**辅助功能**（把识别文字输入到光标）与**麦克风**（采集语音）。

### 固定签名策略

macOS 按「代码签名指定要求」(designated requirement) 匹配授权：

| 签名方式 | 指定要求 | 重编后授权 |
|----------|----------|------------|
| **ad-hoc（已禁止）** | `cdhash H"每次都变"` | 必丢 |
| **openIME Local Dev（固定）** | `certificate root = H"<证书指纹>"` | 保持 |

本仓库已写死身份名 **`openIME Local Dev`**：

- `src-tauri/tauri.conf.json` → `bundle.macOS.signingIdentity`
- `scripts/build.sh` 构建后强制 `codesign --deep` 再签一遍，并拒绝 ad-hoc
- `scripts/ensure-signing-identity.sh` 本机没有证书时自动创建
- `scripts/signing-identity.fingerprint` 记录期望指纹（公钥，可提交）

```bash
# 一次性：确保本机有稳定证书（已有则跳过）
pnpm sign:ensure
# 或
./scripts/ensure-signing-identity.sh

# 日常打包 / 安装 / 运行（始终带固定签名）
./scripts/build.sh            # 或 pnpm app:build
./scripts/build.sh install    # 或 pnpm app:install → /Applications
./scripts/build.sh run        # 或 pnpm app:run
./scripts/build.sh resign     # 仅重签已有 .app，不重新编译
```

**请用上述脚本产出的 `.app` 做日常使用与授权**（`/Applications/openIME.app` 或
`target/release/bundle/macos/openIME.app`）。`tauri dev` / 裸 `cargo run` 不走这套签名，
辅助功能授权不稳定，调试权限时不要依赖它。

### 首次授权（同一签名下只需一次）

1. `./scripts/build.sh install` 装到 `/Applications`
2. 打开 openIME → 设置 → 系统权限 → 授权麦克风 / 辅助功能  
   （或在系统设置里勾选 **当前这份** openIME）
3. 之后只要仍用 **同一本机证书** 打包安装，一般不必再授

若系统里已有旧 openIME 条目但应用仍显示未授权：删掉旧条目再授权一次
（旧 ad-hoc / 其它路径的条目对新签名无效）。

### 排障

```bash
# 看签名是否固定身份（应为 Authority=openIME Local Dev，绝不能是 Signature=adhoc）
codesign -dvvv /Applications/openIME.app 2>&1 | grep -E 'Authority|Signature|Identifier'
codesign -d -r- /Applications/openIME.app 2>&1
# 期望类似：designated => identifier "com.openime.desktop" and certificate root = H"51ab02..."

# 清掉失效 TCC 后重授（慎用）
tccutil reset Accessibility com.openime.desktop
tccutil reset Microphone com.openime.desktop
```

`Info.plist`（src-tauri/Info.plist，打包时合并）含 `NSMicrophoneUsageDescription`；
缺少该键时系统不弹麦克风授权框。

**注意：`bundle.macOS.hardenedRuntime` 必须为 `false`**。实测：
自签证书 + hardened runtime 时，TCC 对麦克风请求**不弹授权框、直接拒绝**；
关闭 hardened runtime 后弹窗正常。请求还必须在主线程发起（TCC 依赖运行循环弹窗），
见 `request_microphone` 命令实现。

## 日志与排障

内置文件日志模块（`src-tauri/src/logging.rs`），在 Tauri 启动前就初始化，
因此连 setup 阶段的崩溃也能留痕：

- **日志目录**：`~/Library/Application Support/com.openime.desktop/logs/`
- **按天滚动**：`openime-YYYY-MM-DD.log`，自动清理 7 天前的文件。
- **覆盖范围**：
  - Rust 启动/运行日志（setup 各步骤、托盘、快捷键、窗口可见性、录音流程）；
  - **panic 崩溃日志**：全局 panic hook 记录位置、消息与 backtrace；
  - 前端日志：JS 的 `window.onerror` / `unhandledrejection` / `console.error`
    与关键生命周期（挂载、IPC ping、录音切换）经 `frontend_log` 命令落盘。
- **同时镜像到 stderr**：终端运行 `open /Applications/openIME.app` 或直接跑二进制可见。

排障命令：

```bash
# 实时跟踪日志
tail -f ~/Library/Application\ Support/com.openime.desktop/logs/openime-$(date +%F).log

# 系统级崩溃报告（原生 abort/SIGSEGV 等 Rust panic hook 捕获不到的情况）
ls ~/Library/Logs/DiagnosticReports/ | grep -i openime
```

原生崩溃（.ips 文件）无符号时，可结合当次日志的最后几行定位 setup 走到哪一步。

## 测试覆盖（TDD）

| 层 | 测试 | 数量 |
|---|---|---|
| 百炼协议帧 | run-task 序列化 / result-generated(partial+final) / task-started/finished/failed 反序列化 | 8 |
| 百炼 provider | 本地 WS mock server 全流程 + task-failed | 2 |
| 存储 | SQLite CRUD / 级联 / 时间戳 / 迁移 / 文件持久化 | 6 |
| 音频 | f32↔s16le / 重采样比例 / WAV fixture / 错误路径 | 7 |
| 权限 | 状态枚举 / checker / 序列化 | 3 |
| 文本插入 | diff_prefix 增量 / 空串 | 2 |
| pipeline | partial 回调 + final 插入 + 落库 / 空 finals 仍建会话 | 2 |
| model_mgr | SHA256 向量 / 校验 / tar.gz 解压 / 校验失败拒绝 | 4 |
| trait 契约 | 对象安全 / 端到端 fake / config 校验 | 6 |
| 前端 | App(ping/保存/校验失败/Onboarding) + History(空/删除) | 6 |
| **合计** | | **46** |

CI：GitHub Actions 三 job（core 三平台 × fmt+clippy+test / tauri-shell / frontend vitest+build）。

## 路线

- ✅ **M0** 工程骨架 + voice-core + CI
- ✅ **M1** 存储 + Settings/History 前端
- ✅ **M2** 百炼协议层 + WS mock provider
- ✅ **M3** 音频采集 + 重采样 + 权限探测
- ✅ **M4** model_mgr + sherpa provider（OnlineRecognizer + Silero VAD 推理，feature 门控）
- ✅ **M5** enigo 文本插入 + pipeline 端到端
- ✅ **M6** 悬浮窗 / 托盘 / 全局快捷键 / Onboarding / 历史详情
- 🚧 **二期** AI 润色（PreferLocal：Qwen2.5-1.5B GGUF / 百炼 chat）/ 人设 / 热词 · 见 `docs/phase2-local-llm-research.md`
- 🔜 Windows 平台
