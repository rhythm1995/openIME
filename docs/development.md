# 开发指南

openIME 的开发、测试、打包、发布与签名细节。**普通用户上手请看 [README](../README.md)。**

文档导航见 [docs/README.md](./README.md)。

## 技术栈

- **后端核心**：Rust 工作区。`crates/voice-core` 是零 Tauri 依赖的纯库（全部逻辑 + trait），`src-tauri` 是 Tauri 薄壳（commands / state / 平台权限）。
- **前端**：React 18 + TypeScript + Vite。样式为原生 CSS + CSS 变量（单文件 `src/styles.css`），状态用 React hooks。
- **GUI 框架**：Tauri v2。

## 架构

核心逻辑全部在 `voice-core`，四个可 mock 的 trait 串成端到端管线：

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
│   │   ├── traits.rs         # AudioSource / AsrProvider / AsrSession / TextInserter / HistoryStore / PolishMode
│   │   ├── config.rs         # AppConfig / ProviderConfig（含 P1/P2 字段与校验）
│   │   ├── store.rs          # SqliteStore + 迁移（v4：前缀角色）
│   │   ├── bailian_proto.rs  # 百炼协议帧（run-task/result-generated 编解码）
│   │   ├── providers/        # bailian.rs / sherpa.rs / openai_asr.rs / multimodal_asr.rs
│   │   ├── polish/           # L0 规则纠错 / ITN / 繁简 / 云端三协议 / 前缀角色 / prompts / runtime（常驻 GGUF）/ translate_router
│   │   ├── pipeline.rs       # 端到端编排（听写 / 翻译 / 角色 / HUD 警告）
│   │   ├── transcribe.rs     # 文件转录（长音频分段 + 重叠拼接 + SRT）
│   │   ├── endpoint.rs       # endpoint SSRF 校验
│   │   ├── audio.rs          # cpal 采集 + rubato 重采样
│   │   ├── insert.rs         # 文本插入（InsertOutcome 四态 / TSF 纯函数）
│   │   ├── model_mgr.rs / model_download.rs  # 模型下载/SHA256/解压/镜像切换
│   │   └── system.rs / http.rs / permissions.rs / asr_catalog.rs / llm_catalog.rs
│   └── tests/                # traits 契约 + 百炼 WS mock + REST ASR mock 集成
├── src-tauri/                # Tauri 薄壳
│   ├── src/                  # commands / state / qa / fn_policy / insert_fallback / windows_ime / credentials / logging / platform
│   └── tauri.conf.json       # 主窗口 + overlay 悬浮窗 + qa 问答窗 + 托盘
└── src/                      # React 18 + TS 前端
    ├── App.tsx               # 设置/历史/Onboarding
    ├── QaPanel.tsx           # 划词问答浮窗
    ├── RecorderOverlay.tsx   # 悬浮窗（实时转写）
    └── components/           # Settings / History / Dictionary
```

## 开发命令

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

Windows 打包（对应 macOS 的 build.sh）：`pnpm app:build:win`（= `scripts/build-windows.ps1`，自动检测 CMake 决定是否带 `llm` feature，产出 NSIS 安装包）。Windows 移植全记录（编译问题、TSF FFI、运行期修复、验证）见 [openIME-windows-porting-notes.md](./openIME-windows-porting-notes.md)。

## 测试覆盖（TDD）

| 层 | 测试 | 数量 |
|---|---|---|
| 百炼协议帧 | run-task 序列化 / result-generated(partial+final) / task-started/finished/failed 反序列化 | 8 |
| ASR providers | 百炼 WS mock 全流程 + task-failed / REST ASR mock / sherpa 识别器缓存 | 11 |
| pipeline | 端到端编排：partial/final 插入落库 / L0 总生效 / L2 跳过与回退 / 翻译分支（含 TranslateRouter 两步 Light）/ 前缀角色 | 37 |
| 润色 | L0 纠错 31 / 前缀角色 13 / prompts 25 / 云端三协议 7 / ITN 7 / LlmClient+SSE 7 / sanitize 6 / router 4 / 繁简 3 / 标点 2 / runtime 7 / translate_router 2 | 134 |
| 本地模型目录 | `llm_catalog`：润色 3 档 + 翻译 2 档 id 闭集 / 未知归一 / fallback 解析 | 12 |
| 文本插入 | InsertOutcome 四态 / diff_prefix 增量 / 剪贴板恢复判定 / TSF 纯函数 | 17 |
| 文件转录 | symphonia 解码 / 线性重采样 / 长音频分段 + 重叠 stitch / srt 切分 | 14 |
| 存储 | SQLite CRUD / 级联 / 时间戳 / 迁移(v4 前缀角色) / 热词批量 / 风格包 CRUD / 日记导出 | 17 |
| endpoint SSRF | validate_endpoint（IMDS / link-local / CGNAT / mapped IPv6 / RFC1918 放行等） | 13 |
| 配置 | AppConfig / ProviderConfig 校验（含 P1/P2 字段 + 本地三件套字段 + 旧 id 迁移） | 19 |
| 系统采集 | 模型适配度标签：轻量/中量/重型 × 内存分档 / 磁盘不足 / Apple Silicon / 三件套 combo 打标 + 推荐器 | 22 |
| 音频 | f32↔s16le / 重采样比例 / WAV fixture / 错误路径 | 15 |
| 模型下载 | SHA256 向量 / 校验 / tar.gz 解压 / 下载与镜像（model_mgr 4 + model_download 6） | 10 |
| 权限 | 状态枚举 / checker / 序列化 | 3 |
| http | no-redirect client | 1 |
| 集成 | 百炼 WS mock 2 + REST ASR mock 5 + trait 契约 6 | 13 |
| 应用壳（src-tauri） | windows_ime 协议 / 粘贴兜底 / commands / qa / fn_policy / credentials / logging 等；84 个测试函数（含 `platform/windows/*` 与 `windows_ime` FFI 专属） | 84† |
| 前端 | Settings（37）/ App / History 等 | 47 |
| **合计** | 本地可跑 392：`cargo test -p voice-core` 345（lib 332 + 集成 13）+ `pnpm test` 47；应用壳 84 测试函数由 Windows CI 跑（† macOS 因 `windows_ime` FFI 门控待 Windows，`cargo check/test -p openime` 本地不过；CI `tauri-shell-windows` 跑） | **476** |

CI：GitHub Actions 四 job——`core`（三平台矩阵 × fmt+clippy+test）、`tauri-shell`（macOS，clippy -D warnings + check）、`tauri-shell-windows`（windows-latest，clippy -D warnings + check + cargo test，兜住 macOS 专属 API 漏门控 / windows-rs 类型漂移）、`frontend`（vitest + build）。

## 多平台发布（GitHub Actions）

推版本 tag 触发 CI 自动构建 **macOS dmg + Windows NSIS 安装包** 并发布
GitHub Release（`.github/workflows/release.yml`；也可在 Actions 页手动 workflow_dispatch）。

发布流程：

```bash
# 1. 同步三处版本号：workspace Cargo.toml / src-tauri/tauri.conf.json / package.json
# 2. 提交到 main
git commit -am "release: 0.1.1"
# 3. 打 tag 并推送（CI 校验 tag 必须等于 tauri.conf.json 的 version）
git tag v0.1.1 && git push origin main --tags
```

产物：`openIME_<版本>_aarch64.dmg`（macOS 内测临时签名）+ `openIME_<版本>_x64-setup.exe`
（Windows 未签名）+ `SHA256SUMS.txt`。

> **内测包安装注意**：
> - macOS：CI 临时自签（非 Developer ID 公证），首次打开需 **右键 → 打开**；
>   辅助功能/麦克风授权在每次重装新包后需重新授予。
> - Windows：未签名，SmartScreen 会提示「未知发布者」，选「仍要运行」。
> - 正式公证（Apple Developer ID）与 Windows 代码签名待接入证书后补。

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
（旧 ad-hoc / 其它路径的条目对新签名无效）。更多排障见 [troubleshooting.md](./troubleshooting.md)。
