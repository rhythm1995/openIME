# 排障

常见问题与诊断手段。开发 / 打包 / 发布细节见 [development.md](./development.md)。

## 日志

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

### 排障命令

```bash
# 实时跟踪日志
tail -f ~/Library/Application\ Support/com.openime.desktop/logs/openime-$(date +%F).log

# 系统级崩溃报告（原生 abort/SIGSEGV 等 Rust panic hook 捕获不到的情况）
ls ~/Library/Logs/DiagnosticReports/ | grep -i openime
```

原生崩溃（.ips 文件）无符号时，可结合当次日志的最后几行定位 setup 走到哪一步。

## 签名验证

macOS 按「代码签名指定要求」匹配授权。验证当前安装包是否为固定签名：

```bash
# 看签名是否固定身份（应为 Authority=openIME Local Dev，绝不能是 Signature=adhoc）
codesign -dvvv /Applications/openIME.app 2>&1 | grep -E 'Authority|Signature|Identifier'
codesign -d -r- /Applications/openIME.app 2>&1
# 期望类似：designated => identifier "com.openime.desktop" and certificate root = H"51ab02..."
```

若显示 `Signature=adhoc`：该包是 ad-hoc 签名，重编后授权必丢。请用
`./scripts/build.sh install` 重新打包（强制固定签名 `openIME Local Dev`）。

## 权限（TCC）重置

辅助功能 / 麦克风授权异常（如旧条目残留导致新签名失效）时，可清掉失效 TCC 后重授（慎用）：

```bash
tccutil reset Accessibility com.openime.desktop
tccutil reset Microphone com.openime.desktop
```

然后在 设置 → 系统权限 重新授权。

## 关键注意事项

- **`NSMicrophoneUsageDescription`**：`src-tauri/Info.plist`（打包时合并）必须含此键，
  否则系统不弹麦克风授权框。
- **`hardenedRuntime` 必须为 `false`**：实测自签证书 + hardened runtime 时，TCC 对麦克风
  请求**不弹授权框、直接拒绝**；关闭后弹窗正常。
- **授权请求必须在主线程发起**（TCC 依赖运行循环弹窗），见 `request_microphone` 命令实现。
- `tauri dev` / 裸 `cargo run` 不走固定签名脚本，辅助功能授权不稳定，**调试权限时不要依赖它**。
