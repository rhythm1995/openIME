#!/usr/bin/env bash
# openIME 打包脚本（macOS）。
#
# 用法：
#   ./scripts/build.sh           # 打包（产出 .app / .dmg）
#   ./scripts/build.sh install   # 打包并安装到 /Applications
#   ./scripts/build.sh run       # 打包 .app 后直接运行（不安装）
#
# 说明：
# - 自动安装前端依赖 + 构建 dist/，再用 tauri build 产出 macOS bundle。
# - 默认不开 sherpa feature（云端百炼即可用）。
#   如需本地离线引擎，设环境变量：WITH_SHERPA=1 ./scripts/build.sh
# - 产物位于 src-tauri/target/release/bundle/ 下。

set -euo pipefail

# ──────────────── 路径与配置 ────────────────
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_TAURI="$ROOT/src-tauri"
DIST="$ROOT/dist"
APP_NAME="openIME"
# cargo workspace 的 target 在工程根；tauri bundle 输出在 target/release/bundle。
BUNDLE_DIR="$ROOT/target/release/bundle"
APP_PATH="$BUNDLE_DIR/macos/$APP_NAME.app"

# 是否启用本地 sherpa-onnx 引擎（默认否，减小体积与编译时间）。
# openime 包声明了转发 feature：sherpa = ["voice-core/sherpa"]。
WITH_SHERPA="${WITH_SHERPA:-0}"
SHERPA_FLAG=""
if [[ "$WITH_SHERPA" == "1" ]]; then
    SHERPA_FLAG="--features sherpa"
    echo "==> 启用本地 sherpa-onnx 引擎"
fi

ACTION="${1:-build}"

cd "$ROOT"

# ──────────────── 前端依赖 ────────────────
echo "==> 安装前端依赖（pnpm）"
if ! command -v pnpm >/dev/null 2>&1; then
    echo "   pnpm 未安装，尝试用 corepack 启用"
    corepack enable || true
    corepack prepare pnpm@latest --activate || true
fi
pnpm install

# ──────────────── 前端构建 ────────────────
echo "==> 构建前端（tsc + vite）"
pnpm build

# ──────────────── Tauri 打包 ────────────────
# ──────────────── 代码签名身份（macOS 权限持久化关键） ────────────────
# macOS 的辅助功能/麦克风授权按「代码签名指定要求」匹配。ad-hoc 签名每次构建
# cdhash 都变，导致授权失效（系统设置里显示已授权但 API 报拒绝）。
# 因此打包时优先使用稳定的签名身份；没有则退回 ad-hoc（CI/全新机器仍可构建）。
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    SIGN_IDENTITY="$(security find-identity -v -p codesigning 2>/dev/null \
        | awk -F'"' '/valid identity|"/ && NF>1 {print $2; exit}')"
    if [[ -n "$SIGN_IDENTITY" ]]; then
        export APPLE_SIGNING_IDENTITY="$SIGN_IDENTITY"
        echo "==> 使用代码签名身份：$APPLE_SIGNING_IDENTITY"
    else
        export APPLE_SIGNING_IDENTITY="-"
        echo "==> 无可用签名身份，使用 ad-hoc 签名（授权将在每次重装后失效）"
    fi
else
    echo "==> 使用环境变量指定的签名身份：$APPLE_SIGNING_IDENTITY"
fi

echo "==> tauri build（release bundle）"
# sherpa 走 openime 包的转发 feature（sherpa = ["voice-core/sherpa"]），
# tauri build 会把 --features 应用到 app 包（src-tauri）。
if [[ -n "$SHERPA_FLAG" ]]; then
    pnpm exec tauri build $SHERPA_FLAG
else
    pnpm exec tauri build
fi

# ──────────────── 产物定位 ────────────────
echo "==> 打包产物"
if [[ -d "$APP_PATH" ]]; then
    echo "   .app : $APP_PATH"
else
    echo "错误：未找到 $APP_PATH" >&2
    exit 1
fi
DMG_PATH="$BUNDLE_DIR/dmg/${APP_NAME}_0.1.0_aarch64.dmg"
if [[ -f "$DMG_PATH" ]]; then
    echo "   .dmg : $DMG_PATH"
fi

# ──────────────── 安装 / 运行 ────────────────
case "$ACTION" in
    install)
        echo "==> 安装到 /Applications"
        rm -rf "/Applications/$APP_NAME.app"
        cp -R "$APP_PATH" "/Applications/"
        echo "   已安装：/Applications/$APP_NAME.app"
        echo "   首次打开可能需在「系统设置 → 隐私与安全性」允许运行；"
        echo "   并在「辅助功能 / 麦克风」授权 openIME。"
        ;;
    run)
        echo "==> 运行 $APP_PATH"
        open "$APP_PATH"
        ;;
    build|"")
        echo "==> 完成。如需安装：./scripts/build.sh install；如需直接运行：./scripts/build.sh run"
        ;;
    *)
        echo "未知动作：$ACTION（可用：build / install / run）" >&2
        exit 1
        ;;
esac
