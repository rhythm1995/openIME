# openIME Windows 打包脚本（PowerShell）。
# 与 macOS 的 scripts/build.sh 对应；不影响 macOS 流程（macOS 仍用 build.sh）。
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File ./scripts/build-windows.ps1           # 打包（产出 NSIS .exe）
#   powershell -ExecutionPolicy Bypass -File ./scripts/build-windows.ps1 -Run      # 打包后运行
#   powershell -ExecutionPolicy Bypass -File ./scripts/build-windows.ps1 -NoSherpa # 仅云端引擎（关闭本地 sherpa）
#
# 签名策略（内测版）：
# - Windows 未签名。SmartScreen 会提示「未知发布者」，选「仍要运行」。
# - 有代码签名证书后，在 tauri.conf.json > bundle.windows 配置 signtool/证书。
#
# feature 策略（与 build.sh 一致）：
# - 默认启用本地 sherpa-onnx 引擎（openime default features 已含 sherpa）。
# - llm（本地 GGUF 润色）需系统 cmake；检测不到自动回退到不含 llm 的构建。

param(
    [switch]$Run,
    [switch]$NoSherpa
)

$ErrorActionPreference = "Stop"

# ──────────────── 路径与配置 ────────────────
$Root = (Resolve-Path "$PSScriptRoot/..").Path
$AppName = "openIME"
# cargo workspace 的 target 在工程根；tauri bundle 输出在 target/release/bundle。
$BundleDir = Join-Path $Root "target/release/bundle"
$NsisDir = Join-Path $BundleDir "nsis"

Set-Location $Root

# ──────────────── 前端依赖 ────────────────
Write-Host "==> 安装前端依赖（pnpm）" -ForegroundColor Cyan
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Write-Error "未找到 pnpm。请先安装：corepack enable（随 Node 附带）或 npm install -g pnpm。"
}
pnpm install

# ──────────────── 前端构建 ────────────────
Write-Host "==> 构建前端（tsc + vite）" -ForegroundColor Cyan
pnpm build

# ──────────────── feature 组装 ────────────────
if ($NoSherpa) {
    Write-Host "==> 跳过 sherpa（仅云端引擎）" -ForegroundColor Yellow
    $ExtraArgs = @("--no-default-features", "--features", "custom-protocol")
} else {
    Write-Host "==> 启用本地 sherpa-onnx 引擎" -ForegroundColor Yellow
    $Features = "custom-protocol,sherpa"
    # llm（本地 GGUF 润色）需 cmake；无 cmake 则回退。
    if (Get-Command cmake -ErrorAction SilentlyContinue) {
        $Features = "custom-protocol,sherpa,llm"
        Write-Host "==> 检测到 cmake，启用 llm feature（本地 GGUF 润色）"
    } else {
        Write-Host "==> 未检测到 cmake，回退到不含 llm 的构建（本地润色将不可用）" -ForegroundColor Yellow
        Write-Warning "本地 GGUF 润色模型将无法加载（UI 会显示「加载中」）。如需本地润色，请安装 CMake 后重新打包。"
    }
    $ExtraArgs = @("--features", $Features)
}

# ──────────────── Tauri 打包 ────────────────
Write-Host "==> tauri build (release bundle, features=$($ExtraArgs -join ' '))" -ForegroundColor Cyan
Write-Host "    （首次打包会下载 NSIS 到本地缓存；release 编译 + LTO 较慢，请耐心等待）"
& pnpm exec tauri build @ExtraArgs
if ($LASTEXITCODE -ne 0) {
    Write-Error "tauri build 失败（exit $LASTEXITCODE）。"
}

# ──────────────── 产物定位 ────────────────
Write-Host "==> 打包产物" -ForegroundColor Green
if (Test-Path $NsisDir) {
    $installers = Get-ChildItem $NsisDir -Filter *.exe
    if (-not $installers) {
        Write-Error "NSIS 目录存在但未找到 .exe：$NsisDir"
    }
    foreach ($exe in $installers) {
        Write-Host "   安装包 : $($exe.FullName)"
    }
} else {
    Write-Error "未找到 NSIS 产物目录：$NsisDir"
}

# ──────────────── 运行 ────────────────
if ($Run) {
    # 运行安装后的主程序（target/release/openIME.exe），便于冒烟。
    $mainExe = Join-Path $Root "target/release/openIME.exe"
    if (Test-Path $mainExe) {
        Write-Host "==> 运行 $mainExe" -ForegroundColor Cyan
        Start-Process $mainExe
    } else {
        Write-Warning "未找到主程序 $mainExe（仅打包，不运行）。"
    }
}

Write-Host "==> 完成。" -ForegroundColor Green
Write-Host "   安装：双击上述 .exe（首次 SmartScreen 选「仍要运行」）"
Write-Host "   运行：target/release/openIME.exe"
