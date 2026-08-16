# openIME Windows 移植 / 打包笔记

> 目标：让 openIME 在 Windows 上能编译、能打包出 NSIS 安装包，且**不影响 macOS 现有功能**。
>
> 本文记录本次移植发现的全部问题、根因、修复，以及 Windows 上的打包方式与验证结果。

## 0. 结论（TL;DR）

- ✅ `src-tauri` 薄壳**首次在 Windows 上编译通过**（`cargo check -p openime`：0 error）。
- ✅ `tauri build` 在 Windows 上产出 **NSIS 安装包**：`target/release/bundle/nsis/openIME_0.1.0_x64-setup.exe`（约 9.5 MB）。
- ✅ 新增 Windows 本地打包脚本 `scripts/build-windows.ps1`（对应 macOS 的 `scripts/build.sh`）。
- ✅ 新增 CI Windows 编译检查 job（`tauri-shell-windows`），兜住本次这类回归。
- ✅ **macOS 路径零改动**：`build.rs` / `tauri.conf.json` / `Info.plist` / `platform/macos/*` / `scripts/build.sh` 等均未触碰；所有 Windows 改动用 `cfg(target_os = "windows")` 或 `cfg(target_os = "macos")` 隔离，macOS 行为与日志保持原样。

## 1. 背景：为什么 Windows 打包一直是坏的

项目「macOS 优先」，CI 与本地脚本长期只面向 macOS：

- `scripts/build.sh` 是纯 macOS 脚本（`codesign` / `osascript` / `/Applications`）。
- CI `.github/workflows/ci.yml` 的 `tauri-shell` job **只在 macOS 上**跑 `cargo clippy/check -p openime`。
- CI 的 `core` job 虽在 `windows-latest` 上跑，但只测 `voice-core`，**从不编译 `src-tauri` 薄壳**。
- `release.yml` 的 Windows job 只在打 tag 时跑 `tauri build`，触发频率极低，失败也难发现。

结果：`src-tauri` 里大量 macOS 专属 Tauri API 调用**没有 `cfg` 门控**，在 Windows 上一编译就报 20 个 error；而 CI 从不在 Windows 编译 `src-tauri`，所以这些错误长期无人发现。本次在 Windows 上实跑 `cargo check` 才把它们一次暴露。

## 2. 问题清单与根因

实跑 `cargo check -p openime --no-default-features --features custom-protocol`（关 sherpa 以快速定位），得到 **20 个编译错误**，分三类：

### 2.1 macOS 专属 Tauri API 未做平台门控（最核心）

下列 API 在 Tauri crate 中**仅 macOS 编译时存在**，Windows 上不存在该类型/方法/变体：

| API | 位置 | 说明 |
|---|---|---|
| `tauri::ActivationPolicy` | `lib.rs` / `qa.rs` 多处 | macOS Dock 激活策略（Accessory/Regular） |
| `AppHandle::set_activation_policy` / `App::set_activation_policy` | `lib.rs` ×4、`qa.rs` ×1 | 同上 |
| `tauri::RunEvent::Reopen` | `lib.rs`（`app.run` 闭包） | Dock 图标点击 reopen 事件，macOS 专属 |

根因：薄壳把这些调用直接写在跨平台路径里，没加 `#[cfg(target_os = "macos")]`。在 macOS 上能编（类型存在），在 Windows 上类型不存在 → 编译失败。

### 2.2 `windows` crate 0.58 类型漂移

`src-tauri/src/platform/windows/focus.rs`（前台 exe 捕获 / 还焦）用的是旧版 `windows-rs` 写法，与 `Cargo.toml` 锁定的 `windows = "0.58"` 不匹配：

- **`PWSTR` 导入路径错了**：`use windows::Win32::Foundation::{…, PWSTR}` —— 0.58 里 `Win32::Foundation` **没有** `PWSTR`。`PWSTR` 实际来自 `windows-strings`，由 `windows::core::PWSTR` 再导出，是 `pub struct PWSTR(pub *mut u16)`（结构体，非类型别名）。需 `use windows::core::PWSTR;` 并用 `PWSTR(buf.as_mut_ptr())` 构造。
- **`HWND` 内部类型变了**：0.58 是 `HWND(pub *mut core::ffi::c_void)`（裸指针，非整数）。原代码 `hwnd.0 == 0` / `HWND(0)` 把它当整数比/构造，全部类型不匹配。正确写法：`hwnd.0.is_null()` / `HWND(std::ptr::null_mut())`。

### 2.3 `enigo` 不是 `src-tauri` 的直属依赖

`insert_fallback.rs::windows_ctrl_v`（Windows Ctrl+V 粘贴兜底）是 `#[cfg(target_os = "windows")]` 代码，里头 `use enigo::{Direction, Enigo, Key, Keyboard, Settings};`。但 `enigo` 只在 workspace 依赖里、被 `voice-core` 用作直属依赖，**`src-tauri` 自己没声明**。Rust 规则：只能 `use` 本 crate 的直属依赖；macOS 上这段代码被 `cfg` 排除不编，所以没暴露；Windows 上它要编 → `unresolved crate enigo`。

### 2.4 其余「错误」其实是 feature 门控（非真问题）

- `transcribe_file_full not found` —— 该函数 `#[cfg(feature = "sherpa")]`，关掉 sherpa 自然找不到。开默认 feature（含 sherpa）即恢复。**非本次修复对象**。

## 3. 修复（逐文件）

> 原则：**所有改动要么 `cfg(target_os = "windows")` 隔离、要么 `cfg(target_os = "macos")` 包住原 macOS 逻辑、要么是行为中性的 lint allow**。macOS 编译路径与运行行为保持不变。

### `src-tauri/src/platform/windows/focus.rs`
- `use windows::core::PWSTR;`（从 `Win32::Foundation` 移除 `PWSTR`）。
- `QueryFullProcessImageNameW(..., PWSTR(buf.as_mut_ptr()), ...)` —— 用 0.58 的 `PWSTR` 结构体构造。
- `hwnd.0 == 0` → `hwnd.0.is_null()`；`HWND(0)` → `HWND(std::ptr::null_mut())`（含测试里的假 HWND）。
  > 此文件整体在 `cfg(target_os = "windows")` 模块下，macOS 不参与编译。

### `src-tauri/Cargo.toml`
- 在 `[target.'cfg(target_os = "windows")'.dependencies]` 下新增 `enigo = { workspace = true }`，并注释说明用途（`windows_ctrl_v` 直属依赖）。
  > 仅 Windows target 依赖，**不进 macOS 依赖列表**；macOS 的 `enigo` 仍由 `voice-core` 传递引入，行为不变。

### `src-tauri/src/lib.rs`
把以下 macOS 专属调用包进 `#[cfg(target_os = "macos")] { … }`：
- 主窗口关闭处理里的 `set_activation_policy(Accessory)`（保留原日志文案，macOS 行为/日志逐字不变）。
- 启动期默认策略块（`Accessory` + 可见则切 `Regular`）整块包住。
- `app.run` 闭包里 `RunEvent::Reopen` 分支包住；Windows 分支用 `let _ = (app_handle, event);` 消解未用形参告警（空操作）。
- `show_main_window` / `show_qa_window` 里的 `set_activation_policy(Regular)` 分别包住；其后的 `show` / `set_focus` 等跨平台调用保持原位不动。
  > 在 macOS 上这些 `cfg(target_os = "macos")` 块恒为真，逻辑与改动前完全一致。

### `src-tauri/src/qa.rs`
- QA 关闭时 `set_activation_policy(Accessory)` 包进 `#[cfg(target_os = "macos")]`。

### `src-tauri/src/platform/windows/fn_key.rs`
- 顶部加 `#![allow(dead_code)]`：这些是「对齐 macOS 调用面」的 Windows 桩函数（Fn 监听 / overlay 无激活显示 / AX 选区等），在 Windows 上不被调用，属预期死代码。仅作用于 Windows 模块，macOS 的 `fn_key.rs` 不受影响。

### `src-tauri/src/insert_fallback.rs`
- 给 `clipboard_get_text` / `clipboard_set_text` 加 `#[allow(unused_variables)]`：`app`（及 `owned`）仅在 macOS 主线程调度分支用到，Windows 分支不用 → Windows 上是未用形参。`allow` 只压告警、不改行为，macOS 上该用还是用。

## 4. Windows 上的打包方式

### 4.1 前置工具

| 工具 | 用途 | 备注 |
|---|---|---|
| Rust (MSVC) | 编译 | `winget install Rustlang.Rust.MSVC` 或 rustup；需 MSVC C++ 生成工具 |
| Node.js + pnpm | 前端构建 / tauri CLI | `corepack enable` 启用 pnpm |
| CMake（可选） | `llm` feature（本地 GGUF 润色） | 不装则自动回退到不含 llm 的构建 |
| NSIS | 打安装包 | **无需手装**，`tauri build` 首次会自动下载到本地缓存 |

### 4.2 一键打包

```powershell
# 方式一：npm script
pnpm app:build:win
# 方式二：直接跑脚本
powershell -ExecutionPolicy Bypass -File ./scripts/build-windows.ps1
# 打包后直接运行：
pnpm app:run:win
```

`scripts/build-windows.ps1` 等价于 macOS 的 `build.sh`：装依赖 → `pnpm build`（前端）→ 检测 cmake 决定是否带 `llm` → `pnpm exec tauri build` → 定位并打印 NSIS 产物。仅云端引擎：加 `-NoSherpa`。

### 4.3 手动逐步（便于排障）

```bash
pnpm install
pnpm build                      # 前端 → dist/
pnpm exec tauri build --features custom-protocol,sherpa --bundles nsis
# 产物：target/release/bundle/nsis/openIME_0.1.0_x64-setup.exe
# 主程序：target/release/openIME.exe
```

> 本地无 CMake 时去掉 `llm`（与 macOS `build.sh` 的 cmake 回退一致）。带 `llm` 需 CMake + llama.cpp，CI 的 `windows-latest` 自带 CMake。

## 5. 验证结果（本机实跑）

| 步骤 | 命令 | 结果 |
|---|---|---|
| 薄壳编译（关 sherpa，快速定位） | `cargo check -p openime --no-default-features --features custom-protocol` | 仅剩 `transcribe_file_full`（sherpa 门控，预期） |
| 薄壳编译（默认 = sherpa，生产场景） | `cargo check -p openime` | ✅ 0 error（3 warning，均为先于本次存在的死代码/告警） |
| 完整打包 | `pnpm exec tauri build --features custom-protocol,sherpa --bundles nsis` | ✅ release 编译 4m52s → NSIS 打包成功 |
| 产物 | `ls target/release/bundle/nsis/` | ✅ `openIME_0.1.0_x64-setup.exe`（9 550 074 B） |
| 主程序 | `target/release/openIME.exe` | ✅ 35 MB |

## 6. macOS 安全性核对（务必不影响 mac）

逐项确认 macOS 路径未被波及：

- **未改文件**：`src-tauri/build.rs`（macOS ObjC 编译：`fn_monitor.m` / `app_focus.m`、framework 链接）、`src-tauri/tauri.conf.json`（含 `macOSPrivateApi` / `macOS.signingIdentity`）、`src-tauri/Info.plist`、`src-tauri/src/platform/macos/*`、`scripts/build.sh` / `ensure-signing-identity.sh` / `ci-sign-macos.sh`、`release.yml` 的 macOS job —— `git diff --stat` 对这些路径**无输出**。
- **`cfg` 隔离**：`lib.rs` / `qa.rs` 的改动一律 `#[cfg(target_os = "macos")] { 原逻辑 }` + `#[cfg(not(target_os = "macos"))] { 空操作 }`。在 macOS 上 cfg 恒真 → 原逻辑逐行保留；`RunEvent::Reopen` 处理、各 `set_activation_policy` 调用、关闭处理日志文案均与改动前一致。
- **`enigo` 依赖**：加在 `[target.'cfg(target_os = "windows")'.dependencies]`，不进 macOS 依赖；macOS 的 `enigo` 仍由 `voice-core` 传递引入。
- **lint allow**：`fn_key`（Windows 模块级）/ `clipboard_*` 的 `#[allow(...)]` 只压告警，零行为变更；macOS 上该编该用照旧。
- **CI**：只**新增** `tauri-shell-windows` job，未改 macOS 的 `tauri-shell` job；`release.yml` 未动。

> 仓库中另有一批**先于本次移植就存在**的工作区改动（`voice-core/config.rs`、`insert.rs`、`polish/roles.rs`、`transcribe.rs`、`pnpm-workspace.yaml`、`History.test.tsx` 的 clippy `cargo fix` 风格重构），不属于本次 Windows 移植产物，已原样保留未动。

## 7. CI 变更

`.github/workflows/ci.yml` 新增 `tauri-shell-windows` job（`runs-on: windows-latest`）：
- 装 pnpm/node → `pnpm install --frozen-lockfile` → `pnpm build`（产出 dist）→ **`cargo check -p openime`**（默认 feature 含 sherpa）。

**门禁用 `cargo check` 而非 `cargo clippy -- -D warnings` 的原因**：`openime` 有一批先于本次就存在的 clippy 提示（`unnecessary_cast` / `unnecessary_unwrap` / `needless_mut` / `dead_code` 等，命中跨平台代码，macOS 上同样会报），且 `voice-core` 的 `transcribe_file_full` 触发 `too_many_arguments`。这些是全项目性的 clippy 清理项，与 Windows 移植无关；待统一清理后再把本 job 提升为 `-D warnings` 与 macOS 对齐。`cargo check` 已足以兜住「macOS 专属 API 漏门控」「windows-rs 类型漂移」这类**编译级**回归（正是本次修复的 bug 类）。

## 8. 已知问题 / 后续

1. **Windows 未签名**：NSIS 安装包未做代码签名，SmartScreen 会提示「未知发布者」，首次需「仍要运行」。接入代码签名证书后在 `tauri.conf.json > bundle.windows` 配置 signtool。
2. **clippy `-D warnings` 未在 Windows CI 启用**：见上文第 7 节，待全项目 clippy 清理后对齐。
3. **`llm` feature 需 CMake**：本地无 CMake 时自动回退（与 macOS `build.sh` 一致）；带 `llm` 构建较重。
4. **Windows IME（TSF）尚为协议层**：`src-tauri/src/windows_ime` 目前是纯协议/决策函数（含黄金 fixture 单测），Windows 专属 FFI（命名管道 client / 会话控制）按 `cfg(target_os = "windows")` 后续落地；本次打包不涉及。
5. **首次打包慢**：release profile（`lto=true`、`codegen-units=1`、`opt-level="s"`）+ sherpa-onnx 链接，首次约 5 分钟；NSIS 首次会下载到本地缓存。

## 9. 运行期问题与修复（打包后实测）

打出的包在 Windows 上跑，发现 4 个运行期问题，已定位根因并修复（代码改动均在 `voice-core` / `src-tauri`，`cfg` 隔离，不影响 macOS）。

### 9.1 录音无效 / 按 Fn 无效

**根因**：默认录音快捷键是 `"Fn"`（`config.rs` `Default`）。`apply_hotkey` 遇 `"Fn"` 走 `platform::current::fn_key::install_fn_monitor`——这是 **macOS NSEvent 监听**；Windows 侧 `platform/windows/fn_key.rs` 是 **no-op 桩**（`install_fn_monitor` 空实现）。结果：Windows 上既没注册全局快捷键、也没原生 Fn 监听 → **完全无法触发录音** →「按 Fn 无效」+「录音无效」。

**修复**：
- `voice-core/src/config.rs`：默认快捷键改为平台感知——`default_hotkey()` 在 macOS 返回 `"Fn"`，Windows/Linux 返回可注册组合键 `"Ctrl+Shift+D"`（与 `lib.rs` 的 `DEFAULT_HOTKEY` 一致）。
- `src-tauri/src/lib.rs::apply_hotkey`：`"Fn"` 分支用 `#[cfg(target_os = "macos")]` 只在 macOS 装原生监听；**非 macOS 上 `"Fn"` 回退注册 `DEFAULT_HOTKEY`（`Alt+Shift+D`）**，避免「配了 Fn 却完全无法触发」。已存旧配置仍是 `"Fn"` 的 Windows 用户也能立即拿到可用触发。

**使用**：Windows 下录音用 `Alt+Shift+D`（回退）或在设置页改任意组合键；触发模式 Toggle/Hold 均可（Hold=按住说话，松开停止）。Fn 键是 macOS 专属，Windows 不可用。

### 9.2 ASR 设置页「正在采集本机信息…」卡住

**根因**：`voice-core/src/system.rs::collect_system_info` 用 `sysinfo::System::new_all()`，它 `refresh_processes()` **枚举全部进程**——Windows 上数百进程逐个查询，极慢（数秒~数十秒，体感像卡死）。设置页「本机信息」小条（`get_system_info`）和 `list_local_asr_models`（经 `system_info_ensure`）首采都会命中。

**修复**：改用 `System::new()` + `refresh_memory()` + `refresh_cpu_all()`——只取内存/CPU brand/核数，**不枚举进程**。CPU 列表由 `refresh_cpu_all` 经 `init_if_needed` 懒填充（读处理器信息，快）。另：`sysctl` Apple Silicon 回退用 `#[cfg(target_os = "macos")]` 门控（避免 Windows 上启动不存在的 `sysctl` 进程）；磁盘剩余 `statvfs_free_bytes` 改为跨平台（原 `#[cfg(not(unix))]` 直接返回 0，Windows 拿不到磁盘信息）。

**实测**（本机 i7-13700HX / Win11）：`collect_system_info` 从「卡死级」降到 **1.48s**，且 `disk_free` 从 0 → 正确读到 903GB 可用；CPU brand/核数/内存/OS 均正确。

### 9.3 本地润色模型 Qwen2.5-1.5B 一直「加载中」

**根因**：本地 GGUF 润色（Qwen2.5-1.5B-Instruct Q4_K_M）走 `llm` feature（`llama-cpp-2`），**需 CMake 编译**。本次本地打包未装 CMake，构建用 `--features custom-protocol,sherpa`（**不含 llm**），故 `llm_feature=false`，本地 LLM 推理引擎根本没编进二进制 → 无法加载模型 → 状态停留在「加载中」。

**修复（构建侧）**：装 CMake 后重新打包即可——`scripts/build-windows.ps1` 已自动检测 cmake 并把 feature 升为 `custom-protocol,sherpa,llm`（与 macOS `build.sh` 同策略）：

```powershell
winget install Kitware.CMake     # 或 choco install cmake
pnpm app:build:win               # 检测到 cmake 即带 llm
```

**前端侧（已具备）**：`get_polish_model_status` 返回 `llm_feature: cfg!(feature = "llm")`；`Settings.tsx` 在 `llm_feature=false` 时显示「当前构建未启用本地推理（需重新打包）」提示。要真正用本地 Qwen，必须带 `llm` 重新打包。临时可用云端润色（设置页配 endpoint + API Key）。

> 附带：若未配置「本地模型目录」，`get_polish_model_status` 会返回 `Err("未配置本地模型目录")`；前端 `getPolishModelStatus().catch(()=>{})` 静默吞错会使状态停在「加载中」。先在设置页指定一个本地模型目录即可。

### 9.4 修复涉及文件（运行期）

| 文件 | 改动 |
|---|---|
| `crates/voice-core/src/config.rs` | `default_hotkey()` 平台感知；`Default.hotkey` 用之 |
| `src-tauri/src/lib.rs` | `apply_hotkey`：`"Fn"` 分支 macOS 装原生监听 / 非 macOS 回退注册 `DEFAULT_HOTKEY` |
| `crates/voice-core/src/system.rs` | `new_all()`→`new()+refresh_memory()+refresh_cpu_all()`；`sysctl` 门控 macOS；磁盘跨平台 |

macOS 影响：`default_hotkey()` 在 macOS 仍返回 `"Fn"`；`apply_hotkey` 的 `"Fn"` 分支在 macOS 走原 `install_fn_monitor`（行为不变）；`system.rs` 的 `sysctl` 块在 macOS 仍编译执行；`refresh_cpu_all` 行为与原 `new_all` 中的 CPU 部分一致，只是不再枚举进程——macOS 上本就快，无回归。


---

## 10. Windows 单键录音（CapsLock / best-effort Fn）+ 已知焦点限制（2026-08-14）

### 10.1 背景

用户反馈：Windows 上按 Fn 无反应、设置页「功能测试」一直「等待按键」。根因是 Windows 侧 `fn_key.rs` 的 Fn 监听是 no-op 桩；且硬件层面**绝大多数笔记本键盘的 Fn 由键盘固件/EC 消费，OS 根本收不到**（低阶钩子/raw input 都看不到，仅个别键盘上报厂商扫描码如 `E0 63`）。

### 10.2 实现（`src-tauri/src/platform/windows/fn_monitor.rs`）

- `WH_KEYBOARD_LL` 低阶键盘钩子（专属线程 + GetMessage 泵），纯函数决策 `classify_hook_event` 可单测（吞键 / 边沿 / auto-repeat 去重 / 补发忽略窗口）。
- 单键目标（`fn_policy::parse_watch_key`，跨平台纯函数）：
  - **CapsLock**：Windows 的「Fn 等价单键」，所有键盘可靠可见。Hold/Toggle **都吞键**（否则每次触发翻转大小写锁定）；短按补发一对 CapsLock 恢复原功能（先写 250ms 忽略窗口防自捕获）。
  - **Fn**：best-effort 盯厂商扫描码 `E0 63`，多数键盘永远不触发 → **同时注册兜底组合键 `Ctrl+Shift+D`**。
- 路由（`lib.rs::apply_hotkey` / `effective_record_shortcut`）：Windows 配单键一律「钩子 + 兜底组合键」双通道；配组合键时钩子目标清空（全放行）。
- `fn_policy`：`is_fn_hotkey` 泛化为 `is_single_key`；`fn_tap_can_consume` 支持 CapsLock 两模式吞键。
- 设置页（`Settings.tsx` + zh/en）：Windows 专属快捷键提示 / Fn 警告 / 功能测试提示 / CapsLock 徽标；短按补发开关在 Windows 配 CapsLock 时也显示。

### 10.3 ⚠️ 已知限制：openIME 自有窗口聚焦时单键被屏蔽（Tauri #14770）

实测定位：**openIME 自己的 WebView 窗口在前台时，本进程的 LL 键盘钩子收不到任何事件**（[tauri#14770](https://github.com/tauri-apps/tauri/issues/14770)，rdev 同样中招）；焦点在任何其它应用时完全正常。对真实听写场景（焦点在微信/Word/浏览器里按键）**无影响**。受影响场景（设置页「功能测试」、QA 面板聚焦时按 CapsLock）由兜底组合键 `Ctrl+Shift+D` 覆盖（RegisterHotKey 不受影响）。设置页 hintWin 已写明。后续可选方案：独立子进程持有钩子、或 RegisterRawInputDevices 观测（不可吞键）。

### 10.4 验证记录（本机 Windows 11）

- `cargo test --lib` 全绿（56 通过）；两个真机金丝雀（同进程 SendInput / 跨进程 PowerShell keybd_event 注入 CapsLock）标注 `#[ignore]` 手动运行（交互桌面下不稳定，单跑通过）。
- 真机 e2e：焦点在其它应用时按 CapsLock → `录音单键按下 → 快捷键触发 → toggle started=true`；再按 → 停止 → `恢复前台 app ... true`。overlay「正在聆听」窗口（330×60 HUD）显示路径与像素级验证通过（SW_SHOWNOACTIVATE + 左下角定位）。

---

## 11. 收敛批处理（2026-08-14 晚）：顺手项 + Hold e2e + 新发现的 panic

### 11.1 已完成

| 事项 | 处理 |
|---|---|
| Windows 日志目录丢日志 | `logging.rs::log_dir_from`：HOME 缺失（Explorer/快捷方式/自启启动）回退 `%APPDATA%\com.openime.desktop\logs`（原静默落 %TEMP%）；纯函数 + 3 单测；真机验证：无 HOME 启动实例日志落在 APPDATA |
| 设置页 macOS 文案 | `appBehavior.launchDescWin`（zh/en）+ `Settings.tsx` 平台分流 |
| Clippy 债务清零 | 全 workspace（含 --all-targets）0 warning：自动修复 6 处（cast/unused mut）+ 手工（doc 缩进×9、field-assignment 重构×7、dead_code allow×2、too_many_arguments allow×1、esc unwrap 重构） |
| Windows CI 对齐 | `tauri-shell-windows` 门禁升级为 `clippy -p openime -- -D warnings`（与 macOS job 一致） |
| Hold 模式真机 e2e | 按住 700ms：按下 → +308ms 触发（delay-start 精确）→ `started=true` → 抬起 → 尾音停止 → 还焦成功；caps 位全程不变（吞键生效）；120ms 短按 → `RepostOnly → 补发` 日志 + caps 位恰好翻转一次（原功能保留） |

### 11.2 e2e 新抓到的 panic（已修，跨平台隐患）

Hold 模式 delay-start 计时器在 tokio worker 上直接调 `on_record_hotkey`，子树里 `tokio::sync::RwLock::blocking_read()` panic（"Cannot block the current thread from within a runtime"）。三处修复：

1. `lib.rs trigger_toggle`：`recording.blocking_read()` → `try_read()`（该检查仅决定 HUD 显示，读锁竞争按 false 处理，权威状态在 toggle_recording）。
2. `lib.rs ArmHoldTimer`：`on_record_hotkey` 经 `spawn_blocking` 派发（同步子树整体安全，含 `has_cloud_key` 等所有 blocking 调用）。
3. `qa.rs insert_last_answer / ask_and_stream`：async fn 里的 `config.blocking_read()` → `read().await`（**macOS 同样存在的潜在 panic**，非 Windows 特有）。

### 11.3 剩余（不可自动化 / 需外部条件 / 已决策缓办）

- 代码签名：需购买证书（SmartScreen「未知发布者」）。
- TSF FFI 落地：P3 缓办（enigo 路径已验证可用）。
- #14770 长期方案（子进程持钩子 / raw input 观测）：缓办，真实听写不受影响。
- 开机自启 / keyring 云端 key / NSIS 干净机安装：需人工或额外环境验证。
- 两个真机钩子金丝雀测试 `#[ignore]`：交互桌面下不稳定，手动 `cargo test --lib real_hook -- --ignored`。
- 多显示器 overlay 定位假设主屏原点：与 macOS 同限，未验证不改。

### 11.4 Windows 默认快捷键改为 CapsLock（用户决策，2026-08-14）

- `voice-core/config.rs`：`default_hotkey()` Windows → `"CapsLock"`（Fn 固件消费不再作默认；Linux 仍 Ctrl+Shift+D）；新增 `default_hotkey_mode()`，Windows 默认 **Hold**（CapsLock 配 Toggle 会吞掉全部短按、大小写锁定完全失效；配 Hold 短按可补发恢复原功能）。serde default 同步。
- **修复设置页保存缺口**：`commands.rs::validate_hotkeys` 此前不认 "CapsLock"（保存被拒，此前只能改库生效）——现与 Fn 同样按「仅录音键支持单键」放行，接受 `caps lock` / `CAPS_LOCK` / `caps` 变体并归一化查重。
- `lib.rs DEFAULT_HOTKEY` 注释更新：语义收窄为「兜底组合键」（单键场景 + 解析失败兜底），不再是配置默认值。
- i18n `recordHintWin` 改为「默认 CapsLock（按住说话）」表述。
- 测试：config 平台配对断言 + CapsLock 校验用例（放行/变体/拒绝非录音键）×5；全量 310 绿 / clippy 0 / 前端 18 绿；真机重启确认 `录音快捷键：CapsLock（WH_KEYBOARD_LL 钩子）`。

### 11.5 #14770 兜底落地：Raw Input 观测通道（2026-08-14 深夜）

用户实测反馈「设置页按 CapsLock 无反应」（#14770：自有 WebView 窗口聚焦时 LL 钩子被屏蔽）。落地双通道方案：

- **主通道 LL 钩子**（可吞键）不变；
- **新增 Raw Input 观测通道**：钩子线程创建隐藏窗口 + `RegisterRawInputDevices(键盘, RIDEV_INPUTSINK)`，消息泵补 `DispatchMessageW` 处理 `WM_INPUT` → RAWKEYBOARD → 与钩子共用 `dispatch_key_event`。
- **去重**：`classify_hook_event` 的按下状态机天然去重（钩子先到置位，raw 后到判为 repeat/孤立 up）；补发忽略窗口内的 raw 事件按注入处理（防自捕获）。
- **不吞键的语义**：raw-only 场景原按键直达系统——Hold 按下+抬起双翻转=净零；短按翻转一次=原功能；**RepostOnly 补发按通道跳过**（`LAST_EDGE_FROM_HOOK=false` 时不补发，防双翻转）。
- **真机验证（openIME 主窗口聚焦状态）**：按住 700ms → 303ms delay-start 触发 → `started=true` → 说话 → **识别插入 9 字**（完整闭环）；短按 120ms → 恰好翻转一次 + 「补发跳过」日志。
- 附带收益：QA 面板聚焦时的单键录音同样恢复可用。i18n 提示已去掉「先点其它应用」限制说明。
- 回归：Rust 310 / 前端 18 全绿，clippy 0。

### 11.6 overlay 残留修复：Windows 隐藏路径直调 SW_HIDE（2026-08-14 深夜）

用户反馈：音频错误（找不到输入设备）后左下角 HUD 不消失。根因分析：Windows 侧 overlay 隐藏走 `win.hide()`（经 Tauri 主线程调度；`hide_overlay_only` 更是包在 `run_on_main_sync` 的 **1s 超时**里）——主线程繁忙时（典型：enigo 插入文字后 WebView 处理）调度被延迟/丢弃 → HUD 残留「正在聆听/…」；显示路径是直调 `SW_SHOWNOACTIVATE`（零调度）所以一直可靠，不对称。

- 修复：`fn_key::hide_window_raw(hwnd)`——`ShowWindow(SW_HIDE)` 直调（线程安全，零调度零超时）；`hide_overlay_only` 与 `trigger_toggle` 错误路径的 Windows 分支全部改走它（hwnd 获取失败才降级 `win.hide()`）。
- 测试：`show_without_activating_keeps_foreground` 补「隐藏对称性」断言（show→可见→raw hide→不可见）；全量 310 绿 / clippy 0。
- 真机验证：错误触发后 2s `visible=False`；连续快速触发后 `visible=False`；含一次完整成功录音（开录→停止→插入→还焦）后同样收起。

---

## 12. R11 TSF 输入法完整落地（2026-08-15）：代码全量交付 + Win11 per-user 注册限制

### 12.1 已交付（全部编译进主构建）

**C++ TIP DLL（`src-tauri/windows-ime/`，~950 行，cl.exe /MT /W4 /EHsc /std:c++17）**：
- `ITfTextInputProcessorEx`：Activate 建隐藏消息窗口（激活线程收 WM_APP 提交）+ 管道 server；Deactivate 反向清理
- `ITfEditSession`：GetSelection → SetText → 光标折叠（目标进程内 CommitText）
- `ipc_server.cpp`：`\.\pipe\OpenImeCommit-{pid}-{tid}`（DACL 仅当前用户、FILE_FLAG_FIRST_PIPE_INSTANCE、OVERLAPPED 可中断）、手写最小 JSON parser、clientReady/SubmitText/SubmitResult/Ping
- `registry.cpp`：注册写 HKLM（管理员时）或 HKCU——CLSID InprocServer32 + TIP LanguageProfile（Description+Enable）+ Category\Category\{catid}\{clsid}（Keyboard 类别是枚举硬前提）+ SortOrder\AssemblyItem\0x00000804 装配项（**语言键是 0x%08X 八位十六进制**，%04X 会写到平行错误键）
- `dllmain/class_factory/guids`：COM 导出四件套，GUID 与 Rust 侧 protocol.rs 常量互为镜像
- build.rs 用 `cc::windows_registry` 定位 cl.exe 一次编译+链接（`/DEF` 必须放 `/link` 之后）

**Rust 宿主（`src-tauri/src/windows_ime/`）**：
- `install.rs`：DLL 路径解析（resource/manifest/exe）、`classify_ime_status` 纯决策 + **`system_lists_tip()` 枚举验证**（EnumProfiles 含我们才算 Installed）、`ensure_registered` 自注册
- `ipc.rs`：管道 client——800ms WaitNamedPipe+CreateFile 重试、**GetNamedPipeServerProcessId==目标 pid 校验**（防仿冒）、JSONL 读写 + sessionId 匹配
- `session.rs`：`prepare_session`（快照→Enable→ActivateProfile(FORSESSION)→WM_INPUTLANGCHANGEREQUEST→clientReady）→ `submit`（≤64KiB）→ `restore_session`（`profile::restore_decision`，Drop 幂等兜底）；`tsf_gate` 纯门控
- `insert_fallback.rs::insert_ex`：TSF 分支最前（Committed 即返回；失败按 `tsf_fallback` 回退 R7；Type 策略保持只打字）
- `commands.rs`：`windows_ime_status` / `windows_ime_restore_profile`；设置页「App 行为」卡新增 TSF 状态行 + 开关 + 恢复按钮（i18n zh/en）
- `focus.rs::frontmost_process_info`：pid/tid/machine（IsWow64Process2）

### 12.2 ⚠️ Win11 per-user TIP 限制（真机实测结论）

**枚举（EnumProfiles）与激活（ActivateProfile）只认 HKLM\SOFTWARE\Microsoft\CTF 下的 TIP 注册**。per-user（HKCU）四件套（LanguageProfile/Category/SortOrder/CLSID）全部写齐、重启 ctfmon 后仍不被收录（ActivateProfile 恒 E_FAIL）。msctf 的 `AddLanguageProfile` 即使在提升进程也只写 HKCU——HKLM 路径必须手写注册表（DLL 已实现：管理员 regsvr32 → PickRegistrationRoot 选 HKLM）。

**因此**：
- `windows_tsf_enabled` 默认 false（探测 RegistrationBroken → 零成本回退 R7，每次插入不多等 1ms）
- **激活方式**：以管理员运行一次 `regsvr32 /s "<安装目录>\resources\ime\OpenImeTsf.dll"`（写 HKLM），然后在设置页打开 TSF 开关
- 设计文档 FR-11.2 的「per-user HKCU 无 UAC」假设在 Win11 不成立（OpenLess 旧系统经验的偏差）；NSIS perUser 安装器如需自动启用，需加提升步骤或引导用户手动执行上述命令

### 12.3 验证记录

- 322 Rust 测试全绿（协议黄金 fixtures/门控/状态决策/GUID 往返/ipc 帧语义/insert.rs KC-8）+ clippy 0 + 前端 18 绿
- 真机：注册键全量落盘验证 ✓；canary `real_tsf_commit_into_foreground_app` 双模式断言 ✓（Installed→Committed 上屏 / Broken→NotInstalled 零等待回退）
- 插入链路保护：探测不过 → 走原 enigo/粘贴路径，无回归（现有插入测试全绿）
