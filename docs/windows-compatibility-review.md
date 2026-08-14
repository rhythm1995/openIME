# openIME Windows 兼容性审查报告

> **审查日期**：2026-08-14
> **审查范围**：全项目源码、CI 配置、构建脚本
> **基线版本**：main @ `85d6f50`（feat(p2): R9/R12/R11）

---

## 1. 总览

openIME 采用 Tauri v2 + React + Rust 架构，以 macOS 为优先平台。经过本轮移植修复，`src-tauri` 薄壳已能在 Windows 上编译通过并打包出 NSIS 安装包。本文档梳理当前仍存在的 Windows 兼容性问题，按严重程度分级，供后续迭代参考。

| 级别 | 数量 | 说明 |
|------|------|------|
| 🔴 高 | 3 | 影响核心功能或存在逻辑矛盾（均已修复 ✅） |
| 🟡 中 | 5 | 功能降级或体验问题（均已修复 ✅） |
| 🟢 低 | 6 | CI/工程/细节问题（4 项已修复、1 项需证书、1 项仅记录） |

> 处理状态详见文末「处理结果汇总」。

---

## 2. 高优先级问题

### 2.1 单实例保护缺失 ✅ 已修复（2026-08-14）

- **位置**：`src-tauri/src/lib.rs:738-746`
- **现象**：Windows 上 `single_instance_check` 是空实现（直接返回 `Ok(())`），用户可启动多个实例，导致端口/快捷键冲突、资源浪费。
- **根因**：macOS 使用 Unix Domain Socket 实现单实例协调，Windows 侧无对应实现。
- **代码现状**：
  ```rust
  /// Windows：暂用简单策略（无单实例协调），返回 Ok 继续。
  /// TODO：用 Windows 命名 Mutex（CreateMutexW）实现真正的单实例。
  #[cfg(not(unix))]
  fn single_instance_check(...) -> Result<(), String> { Ok(()) }
  ```
- **处理**：新增 `src-tauri/src/platform/windows/single_instance.rs`——`CreateMutexW("Local\openIME.single-instance.mutex")` 命名互斥体；`ERROR_ALREADY_EXISTS` 时按 `current_exe` basename 唤起已有实例窗口后返回 Err（调用方退出）；主实例句柄以 usize 存入静态持有到进程结束。`lib.rs` 的 `single_instance_check` 改按 `#[cfg(target_os = "windows")]` 委托。

### 2.2 TSF 输入法集成配置承诺未兑现 ✅ 已修复（2026-08-14）

- **位置**：`crates/voice-core/src/config.rs:281-287`、`crates/voice-core/src/insert.rs:41`、`src-tauri/src/windows_ime/`
- **现象**：
  - `windows_tsf_enabled` 默认为 `true`
  - `InsertOpts::from_config` 在 Windows 上计算 `tsf_enabled = true`
  - 但 `tsf_enabled` **无任何代码消费**——实际文本插入全走 enigo/paste
- **根因**：`windows_ime` 模块目前只有纯协议/决策函数（`protocol.rs`、`profile.rs`），Windows 专属 FFI（命名管道 client / 会话控制）尚未实现。
- **影响**：配置层面承诺了不存在的功能；若用户依赖 TSF 行为预期（如不抢焦点），实际不会生效。
- **处理**：`windows_tsf_enabled` 默认值改为 `false`（serde default + `impl Default` + 测试断言同步），注释注明「FFI 落地后改回默认开启」。UI 本就无该开关渲染，无需改动。

### 2.3 Fn 键回退快捷键与配置默认值不一致 ✅ 已修复（2026-08-14）

- **位置**：`src-tauri/src/lib.rs:33` vs `crates/voice-core/src/config.rs:328-334`
- **现象**：
  - `config.rs` 中非 macOS 默认快捷键为 `"Ctrl+Shift+D"`
  - `lib.rs` 中 `DEFAULT_HOTKEY` 常量为 `"Alt+Shift+D"`
  - 当 Windows 用户配置了 `"Fn"`（合法值），`apply_hotkey` 回退注册的是 `Alt+Shift+D`
- **影响**：用户在设置页看到的默认值（`Ctrl+Shift+D`）与实际注册的快捷键（`Alt+Shift+D`）不同，产生困惑。
- **处理**：`DEFAULT_HOTKEY` 改为平台条件常量（非 macOS = `"Ctrl+Shift+D"` 与 config 对齐；macOS 保持 `"Alt+Shift+D"` 零行为回归）。
  真机测试进一步发现：仅改常量不够——配置为 `"Fn"` 时回退注册的快捷键在 `on_hotkey` 中**无法路由**（路由只认 `parse_shortcut(cfg.hotkey)`，`"Fn"` 解析为 None → 报「未匹配的快捷键」，回退键形同虚设）。新增 `effective_record_shortcut()`：注册与路由统一走「生效快捷键」逻辑（Fn 非 macOS 回退 DEFAULT_HOTKEY；解析失败同样回退），并补单元测试。

---

## 3. 中优先级问题

### 3.1 QA 选中文本功能不可用 ✅ 已修复（2026-08-14，UIAutomation 方案）

- **位置**：`src-tauri/src/platform/windows/fn_key.rs:46-48`
- **现象**：`get_selection()` 返回 `None` 桩实现。QA 面板的"选中文本"入口在 Windows 上永远为空。
- **根因**：macOS 通过 Accessibility API（AXUIElement）获取选中文本，Windows 无等价简单实现。
- **可能方案**：
  - UIAutomation `IUIAutomationTextPattern`（覆盖率高但复杂）✅ **已采用**
  - 剪贴板模拟（Ctrl+C → 读剪贴板 → 还原，有副作用）
  - 标记为"macOS 专属功能"并在 UI 隐藏
- **处理**：新增 `src-tauri/src/platform/windows/uia.rs`——`CoInitializeEx(APARTMENTTHREADED)`（S_OK 才配对 `CoUninitialize`，避免拆掉主线程 WebView2 的 COM）→ `CoCreateInstance(CUIAutomation)` → `GetFocusedElement` → `GetCurrentPatternAs(TextPattern)` → `GetSelection`（windows 0.58 返回类型化的 `IUIAutomationTextRangeArray`）→ 逐段 `GetText` 拼接。`fn_key::get_selection` 委托之；`commands.rs` 的 `get_selection` 命令去掉非 macOS 直返 None 分支。新增 no-panic 冒烟测试。

### 3.2 Overlay 窗口显示时可能抢焦点 ✅ 已修复（2026-08-14）

- **位置**：`src-tauri/src/platform/windows/fn_key.rs`（`show_overlay_preserving_focus` 为空桩）
- **现象**：macOS 通过 ObjC `orderFrontRegardless` + 不激活窗口实现无焦点偷取的 HUD 显示；Windows 走普通 `win.show()`，可能打断用户当前输入。
- **缓解现状**：已设 `set_focusable(false)` + `set_ignore_cursor_events(true)`，但仍可能触发窗口激活动画或焦点转移。
- **处理**：实现 `fn_key::show_window_without_activating`——`ShowWindow(hwnd, SW_SHOWNOACTIVATE)`。`show_overlay` 的 Windows 分支经 `win.hwnd()`（tauri 的 windows 0.61 HWND 经 `.0` 裸指针桥接到本项目 0.58）直调；`hwnd()` 失败降级 `win.show()`。
  真机测试另发现：overlay 的 conf 初始位置 `y: 99999`（屏幕外），Windows 分支原本从不定位——不定位则无激活显示也是显示在屏幕外。已将定位逻辑提升为共享 helper `overlay_target_position()`（macOS 行为不变），Windows 分支显示前 `set_position`（tao 实现带 SWP_NOACTIVATE，不激活）再 SW_SHOWNOACTIVATE。

### 3.3 `activate_by_exe_basename` 可能激活错误窗口 ✅ 已修复（2026-08-14）

- **位置**：`src-tauri/src/platform/windows/focus.rs:80-96`
- **现象**：`EnumWindows` 遍历顶层窗口，匹配目标进程的**第一个**窗口并 `SetForegroundWindow`。该窗口可能是隐藏/最小化的，而非用户正在操作的。
- **影响**：还焦时可能激活错误窗口或唤醒最小化窗口。
- **处理**：枚举回调加 `IsWindowVisible` + `!IsIconic` 过滤；命中后取 `GetLastActivePopup` 优先激活最近活跃的弹出子窗（对话框）。EnumWindows 按 Z-order 自顶向下，首个命中即最靠前窗口。

### 3.4 麦克风权限未检查 HKLM 策略 ✅ 已修复（2026-08-14）

- **位置**：`src-tauri/src/platform/windows/permissions.rs:76-102`
- **现象**：仅读取 `HKCU\...\ConsentStore\microphone`（用户级），不检查 `HKLM`（管理员/组策略级）。
- **影响**：企业环境中管理员通过组策略禁用麦克风时，应用报告"已授权"但录音静默失败。
- **处理**：`consent_value` 泛化为 `consent_value_in(root, subpath)`，`microphone_state()` 同时检查 HKCU 与 `HKLM\SOFTWARE\...\ConsentStore\microphone`（含 `NonPackaged`），任一层级 `Deny` 即 `Denied`。抽出纯函数 `any_deny` 并补单元测试。

### 3.5 `restore_frontmost_focus` 自身识别逻辑失效 ✅ 已修复（2026-08-14）

- **位置**：`src-tauri/src/commands.rs:781`
- **现象**：
  ```rust
  if bid == "com.openime.desktop" { ... }
  ```
  Windows 上 `frontmost` 是 exe 文件名（如 `openime.exe`），永远不等于 macOS bundle ID，此分支死代码。
- **影响**：当焦点在 openIME 自身时，不会走 `set_focus()` 快速路径，而是走 `activate_by_exe_basename`（较慢但功能上可用）。
- **处理**：新增 `is_self_bundle_id(bid)`——macOS 比对 bundle id；Windows 比对 `current_exe()` basename（大小写不敏感），命中走 `main.set_focus()` 快路径。

---

## 4. 低优先级问题

### 4.1 Windows CI 不执行测试 ✅ 已修复（2026-08-14）

- **位置**：`.github/workflows/ci.yml`（`tauri-shell-windows` job）
- **现象**：Windows job 仅跑 `cargo check -p openime`，不跑 `cargo test`。`platform/windows/` 下的单元测试（focus.rs、permissions.rs）在 CI 中从不执行。
- **处理**：`tauri-shell-windows` 追加 `cargo test -p openime`。src-tauri 44 个测试均为纯函数/mock（注册表测试在默认放行场景可通过，UIA 仅 no-panic 冒烟 + 一个 `#[ignore]` 手动功能冒烟），无需 `#[ignore]`。

### 4.2 Windows 安装包未签名 ⏸ 未处理（需证书）

- **位置**：`src-tauri/tauri.conf.json`（无 `bundle.windows` 签名配置）
- **现象**：SmartScreen 提示"未知发布者"，用户需手动"仍要运行"。
- **处理**：无法在代码层面解决——需先申请代码签名证书。接入后在 `tauri.conf.json` 配置 `signCommand`。脚本注释（`build-windows.ps1`）已说明该策略。

### 4.3 磁盘可用空间计算不精确 ✅ 已修复（2026-08-14）

- **位置**：`crates/voice-core/src/system.rs:76-83`
- **现象**：`statvfs_free_bytes` 返回所有磁盘中最大可用空间，忽略 `path` 参数（模型目录所在盘）。多盘 Windows 上可能误导用户认为空间充足。
- **处理**：`statvfs_free_bytes(path)` 改为按 path canonicalize 后与各卷 `mount_point` 做最长前缀匹配（Windows 大小写不敏感，根挂载点 `/` 特判），取匹配卷 `available_space()`；无匹配回退全盘最大值。`collect_system_info` 增加 `disk_path` 参数，薄壳两处调用传 `AppState::model_root()`（模型目录所在卷）。
  真机测试（双盘 C:/D:）抓到并修复：Windows 上 std `canonicalize` 返回 `\\?\` 扩展路径前缀（UNC 为 `\\?\UNC\`），与 sysinfo 挂载点前缀匹配失败导致误取「全盘最大值」；现已剥离该前缀后比较，双卷验证与 OS 报告一致。

### 4.4 `windows` crate 版本升级风险 📌 仅记录（无代码改动）

- **位置**：`src-tauri/src/platform/windows/focus.rs`
- **现象**：0.58 曾发生 `PWSTR` 导入路径变更和 `HWND` 内部类型变更（整数→裸指针）。后续升级可能再次引入破坏性变更。
- **处理**：风险记录在案，升级时优先跑 Windows `cargo check`。本次新增代码的两处跨版本桥接（tauri 0.61 HWND → 本项目 0.58 经 `.0` 裸指针；`windows::core::PCWSTR` 构造）已用注释标注版本差异。

### 4.5 `llm` feature 需 CMake，缺失时静默降级 ✅ 已修复（2026-08-14）

- **位置**：`scripts/build-windows.ps1`
- **现象**：未装 CMake 时构建不带 `llm` feature，本地润色模型无法加载，UI 显示"加载中"。虽有前端提示，但用户可能不理解。
- **处理**：跳过 llm 时的提示加黄色高亮，并追加 `Write-Warning` 明示「本地 GGUF 润色将无法加载，安装 CMake 后重新打包」。（原有提示文案已存在，仅增强醒目度。）

### 4.6 Clippy lint 债务 ✅ 已修复（2026-08-14 晚）

- **现象**：存在 `unnecessary_cast`、`dead_code`、`needless_mut`、`too_many_arguments` 等告警，Windows CI 无法启用 `-D warnings`。
- **处理**：全 workspace（含 `--all-targets`）clippy 清零——自动修复 6 处 + 手工清理（doc 缩进 / field-assignment 重构 / allow 标注等）；`tauri-shell-windows` job 已升级为 `clippy -p openime -- -D warnings`，与 macOS job 对齐。见 [openIME-windows-porting-notes.md](../openIME-windows-porting-notes.md) §11.1。

---

## 5. 已修复问题（本轮移植）

以下问题在本轮 Windows 移植中已修复，记录备查：

| 问题 | 修复方式 |
|------|----------|
| macOS 专属 Tauri API 未门控（20 个编译错误） | `#[cfg(target_os = "macos")]` 包裹 |
| `windows` crate 0.58 `PWSTR` 导入路径错误 | 改为 `windows::core::PWSTR` |
| `HWND` 类型从整数变为裸指针 | `is_null()` / `null_mut()` |
| `enigo` 非 `src-tauri` 直属依赖 | 加入 Windows target dependencies |
| `System::new_all()` 枚举进程导致 Windows 卡死 | 改为 `new()` + 轻量 refresh |
| 默认快捷键不感知平台 | `default_hotkey()` 按平台返回 |
| `sysctl` 在 Windows 上启动子进程失败 | `#[cfg(target_os = "macos")]` 门控 |
| 磁盘信息 Windows 上返回 0 | 改用跨平台 `sysinfo::Disks` |

---

## 6. 建议处理优先级

```
P0（立即）：
  ├── 2.3 统一 DEFAULT_HOTKEY 与 config 默认值（一行改动）✅
  └── 2.2 TSF 配置默认值改为 false（避免误导）✅

P1（近期）：
  ├── 2.1 实现 Windows 单实例（CreateMutexW）✅
  ├── 3.2 Overlay 无激活显示（SW_SHOWNOACTIVATE）✅
  └── 3.3 activate_by_exe_basename 窗口过滤优化 ✅

P2（中期）：
  ├── 3.1 QA 选中文本（UIAutomation 或标记为 macOS 专属）✅（UIAutomation 方案）
  ├── 3.4 HKLM 麦克风策略检查 ✅
  ├── 3.5 restore_frontmost_focus 平台适配 ✅
  └── 4.1 CI 补充 Windows 测试 ✅

P3（远期）：
  ├── TSF FFI 落地（命名管道 client）⏸
  └── 4.2 代码签名 ⏸（需证书）
  （4.6 Clippy 债务清理 ✅ 已完成，Windows CI 已对齐 -D warnings）
```

| 编号 | 问题 | 状态 | 落点 |
|------|------|------|------|
| 2.1 | 单实例保护缺失 | ✅ 已修复 | 新增 `platform/windows/single_instance.rs`（CreateMutexW） |
| 2.2 | TSF 配置承诺未兑现 | ✅ 已修复 | `config.rs` 默认值改 false + 测试断言 |
| 2.3 | Fn 回退快捷键不一致 | ✅ 已修复 | `lib.rs` `DEFAULT_HOTKEY = "Ctrl+Shift+D"` |
| 3.1 | QA 选中文本不可用 | ✅ 已修复 | 新增 `platform/windows/uia.rs`（UI Automation TextPattern） |
| 3.2 | Overlay 抢焦点 | ✅ 已修复 | `fn_key::show_window_without_activating`（SW_SHOWNOACTIVATE） |
| 3.3 | 还焦可能激活错误窗口 | ✅ 已修复 | `focus.rs` 加 IsWindowVisible/!IsIconic/GetLastActivePopup |
| 3.4 | 麦克风未查 HKLM | ✅ 已修复 | `permissions.rs` 双 hive 检查 + `any_deny` 纯函数测试 |
| 3.5 | 自身还焦识别失效 | ✅ 已修复 | `commands.rs::is_self_bundle_id`（current_exe 比对） |
| 4.1 | Windows CI 无测试 | ✅ 已修复 | `ci.yml` 追加 `cargo test -p openime` |
| 4.2 | 安装包未签名 | ⏸ 需证书 | 无代码改动（脚本注释已说明策略） |
| 4.3 | 磁盘空间计算不精确 | ✅ 已修复 | `system.rs` 按卷前缀匹配 + 薄壳传模型目录 |
| 4.4 | windows crate 升级风险 | 📌 仅记录 | 新代码跨版本桥接处已加注释 |
| 4.5 | llm 降级提示不醒目 | ✅ 已修复 | `build-windows.ps1` 黄色警告 + Write-Warning |
| 4.6 | Clippy lint 债务 | ✅ 已修复 | 全 workspace clippy 0 告警，Windows CI 升级 `-D warnings`（porting notes §11.1） |

---

## 7. 架构评价

整体而言，项目的 Windows 适配架构设计合理：

- **`platform/` 模块抽象**：通过 `platform::current` 统一调度，各平台实现隔离清晰
- **`cfg` 门控规范**：macOS 专属 API 已一致使用 `#[cfg(target_os = "macos")]` 包裹
- **CI 覆盖**：`voice-core` 三平台矩阵测试 + Windows 薄壳编译检查已建立
- **构建脚本对称**：`build.sh`（macOS）/ `build-windows.ps1`（Windows）策略对齐

主要差距在于：TSF 原生集成（C++ DLL + FFI）尚未落地。（2026-08-14 `4c0845e` 更新：本文写作时的其余桩实现——Fn/CapsLock 单键监听（LL 钩子 + Raw Input 双通道）、UIA 选中文本、CreateMutexW 单实例——均已实现并在真机验证，见 [openIME-windows-porting-notes.md](../openIME-windows-porting-notes.md)。）

## 8. 处理结果汇总（2026-08-14）

| 编号 | 问题 | 状态 | 落点 |
|------|------|------|------|
| 2.1 | 单实例保护缺失 | ✅ 已修复 | 新增 `platform/windows/single_instance.rs`（CreateMutexW） |
| 2.2 | TSF 配置承诺未兑现 | ✅ 已修复 | `config.rs` 默认值改 false + 测试断言 |
| 2.3 | Fn 回退快捷键不一致 | ✅ 已修复 | `lib.rs` 平台条件 DEFAULT_HOTKEY + `effective_record_shortcut` 统一注册/路由 |
| 3.1 | QA 选中文本不可用 | ✅ 已修复 | 新增 `platform/windows/uia.rs`（UI Automation TextPattern） |
| 3.2 | Overlay 抢焦点 | ✅ 已修复 | `fn_key::show_window_without_activating`（SW_SHOWNOACTIVATE）+ overlay 屏幕外定位修复 |
| 3.3 | 还焦可能激活错误窗口 | ✅ 已修复 | `focus.rs` 加 IsWindowVisible/!IsIconic/GetLastActivePopup |
| 3.4 | 麦克风未查 HKLM | ✅ 已修复 | `permissions.rs` 双 hive 检查 + `any_deny` 纯函数测试 |
| 3.5 | 自身还焦识别失效 | ✅ 已修复 | `commands.rs::is_self_bundle_id`（current_exe 比对） |
| 4.1 | Windows CI 无测试 | ✅ 已修复 | `ci.yml` 追加 `cargo test -p openime` |
| 4.2 | 安装包未签名 | ⏸ 需证书 | 无代码改动（脚本注释已说明策略） |
| 4.3 | 磁盘空间计算不精确 | ✅ 已修复 | `system.rs` 按卷前缀匹配（含 `\\?\` 前缀剥离）+ 薄壳传模型目录 |
| 4.4 | windows crate 升级风险 | 📌 仅记录 | 新代码跨版本桥接处已加注释 |
| 4.5 | llm 降级提示不醒目 | ✅ 已修复 | `build-windows.ps1` 黄色警告 + Write-Warning |
| 4.6 | Clippy lint 债务 | ✅ 已修复 | 全 workspace clippy 0 告警，Windows CI 升级 `-D warnings`（porting notes §11.1） |

## 9. 真机测试记录（2026-08-14，本机 Windows 11，rustc 1.97.1）

### 9.1 自动化测试（纳入 CI / 测试套件）

| 测试 | 验证内容 | 结果 |
|------|----------|------|
| `single_instance::same_process_second_acquire_detects_existing` | 真实内核互斥体：同进程二次获取报已存在 | ✅ |
| `single_instance::second_process_detects_existing` | **跨进程**：父进程持锁，子进程（真实第二进程）检测到已有实例；哨兵输出防「过滤未命中」假阳性 | ✅ |
| `focus::find_activatable_skips_hidden_and_iconic` | 真实窗口：可见/最小化/隐藏三窗口同 exe，必须命中可见且未最小化者；仅剩隐藏/最小化时返回 None | ✅ |
| `fn_key::show_without_activating_keeps_foreground` | 真实窗口 + 真实前台：SW_SHOWNOACTIVATE 显示成功且前台窗口不变 | ✅ |
| `commands::self_bundle_id_detection` | 自身 exe basename（含大小写）识别为自身 | ✅ |
| `system::statvfs_free_bytes_matches_each_mount`（`#[ignore]` 手动） | 每个挂载点必须选中自身卷 | ✅ C:≈811.5GB / D:≈903.9GB 与 OS 一致 |
| `uia::uia_reads_focused_selection`（`#[ignore]` 手动） | 记事本 Ctrl+A 选区直读 | ✅ `Some("helloworld你好")` |

全量：openime **51 passed / 0 failed**，voice-core **247 passed / 0 failed**，voice-core clippy `-D warnings` 0 告警。

### 9.2 真实应用 E2E（`target/debug/openime.exe`，WebView2 + 前端 dist）

| 场景 | 证据 | 结果 |
|------|------|------|
| 启动冒烟 | 主窗口 "openIME" 出现；日志「数据库已打开 / setup 完成」 | ✅ |
| **双实例互斥** | 第二实例 3 秒内自动退出且第一实例存活；日志「单实例检查：已有实例运行，已唤起其窗口，退出本进程」 | ✅ |
| 唤起已有实例 | 第二实例退出后 openIME 主窗口成为前台（activate_by_exe_basename 生效） | ✅ |
| 配置 hotkey="Fn" 的回退 | 日志「当前平台不支持 Fn 键…回退 Ctrl+Shift+D」（与设置页默认值一致，修复 2.3 生效） | ✅ |
| 全局快捷键路由 | 真机按 Ctrl+Shift+D → 日志「录音快捷键触发」（修复前为「未匹配的快捷键」，本次修复 `effective_record_shortcut`） | ✅ |
| 录音流程错误路径 | 本机**无麦克风输入设备** → 「音频错误: 找不到输入设备」，前台焦点保持记事本未被抢，无崩溃 | ✅（环境限制） |
| Overlay 显示/不抢焦点（录音路径） | 受「无麦克风」硬件限制未走到；由 9.1 的 `show_without_activating_keeps_foreground` 真实窗口测试覆盖底层行为 | ⏸ 待有麦克风机验证 |

### 9.3 测试中发现并修复的额外问题

1. **Fn 回退快捷键不可路由**（2.3 的深层 bug）：注册了回退键但 `on_hotkey` 路由不到 → 新增 `effective_record_shortcut()`。
2. **overlay 屏幕外定位**（3.2 的前置 bug）：conf `y:99999` 且 Windows 从不定位 → 共享 `overlay_target_position()` 后 `set_position`（SWP_NOACTIVATE）。
3. **磁盘按卷误匹配**（4.3 实现 bug）：std canonicalize 的 `\\?\` 前缀导致前缀匹配失败回退全盘最大值 → 剥离前缀修复。

### 9.4 macOS 兼容性审计结论

- `show_overlay` macOS 分支未动；cfg 三路互斥（macos / windows / 其它）完备，非 mac/win 保留原 `win.show()`。
- `single_instance_check`：`#[cfg(unix)]` 分支原样；Windows 新分支 + `not(any(unix, windows))` 兜底。
- `DEFAULT_HOTKEY` 平台条件化：macOS 保持 `Alt+Shift+D` 零行为回归（macOS 配置默认 Fn 走原生监听，此常量仅用于解析失败兜底）。
- `effective_record_shortcut`：macOS 配 Fn 返回 None（原生监听路径，与现状一致）；macOS 解析失败回退 Alt+Shift+D（注册即路由，行为改善且无害）。
- `is_self_bundle_id`：macOS 首判 bundle id 不变；`cfg!(windows)` 块全平台可编译（常量折叠）。
- `get_selection` 命令统一走平台函数：macOS 同前；Linux 桩模块已含 `get_selection`，编译面完整。
- `collect_system_info(disk_path)`：跨平台；`cfg!(windows)` 大小写归一化，macOS/Linux 保持原样比较。
- voice-core 改动由三平台 CI 矩阵（clippy -D warnings + test）兜底；`platform/windows/*` 新文件不参与 macOS 编译。
- 本机无法编译 macOS（需 Xcode 框架），macOS 侧由 CI macOS job 验证；共享代码路径已逐条人工核查。

### 9.5 遗留手动验证清单（建议在有麦克风的 Windows 机器上）

1. 完整录音流程：Ctrl+Shift+D → overlay 出现在屏幕左下偏上（x≈16, 高≈屏高-100）且**不抢焦点**；再按一次停止 → overlay 隐藏、焦点回到原应用。
2. QA 面板「选中文本」在 VS Code / Edge / Office 等多类应用中的覆盖率。
3. 多显示器下 overlay 定位（当前实现取 current_monitor）。
