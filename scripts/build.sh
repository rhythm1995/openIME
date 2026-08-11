#!/usr/bin/env bash
# openIME 打包脚本（macOS）。
#
# 用法：
#   ./scripts/build.sh           # 打包（产出 .app）
#   ./scripts/build.sh install   # 打包并安装到 /Applications
#   ./scripts/build.sh run       # 打包 .app 后直接运行（不安装）
#   ./scripts/build.sh resign    # 仅对已有 .app 用稳定身份重签（不重新编译）
#
# 签名策略（权限持久化关键）：
#   固定使用钥匙串身份「openIME Local Dev」。
#   禁止静默退回 ad-hoc（ad-hoc 每次 cdhash 变，辅助功能授权必丢）。
#   首次本机构建会自动创建该证书；指纹记录在 scripts/signing-identity.fingerprint。

set -euo pipefail

# ──────────────── 路径与配置 ────────────────
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_TAURI="$ROOT/src-tauri"
APP_NAME="openIME"
# cargo workspace 的 target 在工程根；tauri bundle 输出在 target/release/bundle。
BUNDLE_DIR="$ROOT/target/release/bundle"
APP_PATH="$BUNDLE_DIR/macos/$APP_NAME.app"

# 与 tauri.conf.json > bundle.macOS.signingIdentity、ensure 脚本保持一致
FIXED_IDENTITY="openIME Local Dev"

# 是否启用本地 sherpa-onnx 引擎（默认是；openime default features 已含 sherpa）。
WITH_SHERPA="${WITH_SHERPA:-1}"
SHERPA_FLAG=""
if [[ "$WITH_SHERPA" == "1" ]]; then
    SHERPA_FLAG="--features sherpa"
    echo "==> 启用本地 sherpa-onnx 引擎"
else
    echo "==> 跳过 sherpa（仅云端引擎）；构建时使用 --no-default-features --features custom-protocol"
fi

ACTION="${1:-build}"

cd "$ROOT"

# ──────────────── 固定签名身份 ────────────────
ensure_signing() {
    echo "==> 确保稳定代码签名身份"
    "$ROOT/scripts/ensure-signing-identity.sh"
    # 允许用环境变量覆盖（例如 CI 临时用其它证书），但禁止 "-" ad-hoc
    if [[ -n "${APPLE_SIGNING_IDENTITY:-}" && "${APPLE_SIGNING_IDENTITY}" != "-" ]]; then
        export APPLE_SIGNING_IDENTITY
        echo "==> 使用环境变量签名身份：$APPLE_SIGNING_IDENTITY"
    else
        export APPLE_SIGNING_IDENTITY="$FIXED_IDENTITY"
        echo "==> 使用固定签名身份：$APPLE_SIGNING_IDENTITY"
    fi
    if [[ "${APPLE_SIGNING_IDENTITY}" == "-" ]]; then
        echo "错误：禁止 ad-hoc 签名（-）。权限会在每次重编后失效。" >&2
        echo "请运行 ./scripts/ensure-signing-identity.sh 创建「${FIXED_IDENTITY}」。" >&2
        exit 1
    fi
}

# 对 .app 强制用固定身份深签 + 校验（双保险：tauri 签完再盖一次）
resign_app() {
    local app="${1:-$APP_PATH}"
    if [[ ! -d "$app" ]]; then
        echo "错误：找不到 $app" >&2
        exit 1
    fi
    local id="${APPLE_SIGNING_IDENTITY:-$FIXED_IDENTITY}"
    echo "==> codesign（深签）：$id → $app"
    # hardenedRuntime 在 tauri.conf 里为 false：不要加 --options runtime
    # 否则自签证书下 TCC 麦克风可能不弹窗
    codesign --force --deep --sign "$id" "$app"
    codesign --verify --deep --strict "$app"
    echo "==> 签名校验通过"
    codesign -dvvv "$app" 2>&1 | grep -E '^(Authority|Identifier|Signature|TeamIdentifier|Format)=' || true
    echo "==> 指定要求（应含 certificate root = H\"...\"，跨构建稳定）："
    codesign -d -r- "$app" 2>&1 | sed -n 's/^.*designated => /   /p'
    # 拒绝 ad-hoc
    if codesign -dvvv "$app" 2>&1 | grep -qi 'Signature=adhoc'; then
        echo "错误：仍是 ad-hoc 签名，权限无法持久化" >&2
        exit 1
    fi
}

if [[ "$ACTION" == "resign" ]]; then
    ensure_signing
    resign_app
    echo "==> 完成重签"
    exit 0
fi

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

ensure_signing

# ──────────────── Tauri 打包 ────────────────
# 默认开启 llm feature（本地 GGUF 润色），需系统 cmake + Metal。
# 无 cmake 环境会自动回退到不含 llm 的构建（见下方 fallback）。
BUILD_FEATURES="custom-protocol,sherpa,llm"
if ! command -v cmake >/dev/null 2>&1; then
    echo "==> 未检测到 cmake，回退到不含 llm 的构建（本地润色将不可用）"
    BUILD_FEATURES="custom-protocol,sherpa"
fi

echo "==> tauri build (release bundle, identity=${APPLE_SIGNING_IDENTITY}, features=${BUILD_FEATURES})"
if [[ "$WITH_SHERPA" == "1" ]]; then
    pnpm exec tauri build --features "$BUILD_FEATURES"
else
    pnpm exec tauri build --no-default-features --features "$BUILD_FEATURES"
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

# 强制稳定签名（覆盖 tauri 可能留下的 ad-hoc / 不完整签名）
resign_app "$APP_PATH"

# ──────────────── 安装 / 运行 ────────────────
case "$ACTION" in
    install)
        echo "==> 安装到 /Applications（仅一份）"
        # 退出所有旧实例，避免菜单栏出现两个托盘图标
        pkill -x openime 2>/dev/null || true
        pkill -x openIME 2>/dev/null || true
        osascript -e 'tell application id "com.openime.desktop" to quit' 2>/dev/null || true
        sleep 0.4
        # 清掉 Applications 下所有 openIME 变体（含 "openIME 2.app" 等 Finder 拷贝）
        while IFS= read -r -d '' old; do
            echo "   移除旧副本：$old"
            rm -rf "$old"
        done < <(find /Applications -maxdepth 1 \( -name 'openIME.app' -o -name 'openIME *.app' -o -name 'openime.app' \) -print0 2>/dev/null)
        cp -R "$APP_PATH" "/Applications/$APP_NAME.app"
        resign_app "/Applications/$APP_NAME.app"
        echo "   已安装：/Applications/$APP_NAME.app"
        echo "   说明：工程 target/ 下的 .app 只是构建缓存，不会再往 Applications 装第二份。"
        echo "   首次：系统设置 → 隐私与安全性 → 辅助功能 / 麦克风，授权 openIME。"
        echo "   之后用本脚本重装同一签名身份时，授权应保持有效。"
        ;;
    run)
        echo "==> 运行 $APP_PATH"
        open "$APP_PATH"
        ;;
    build|"")
        echo "==> 完成。"
        echo "   安装：./scripts/build.sh install"
        echo "   运行：./scripts/build.sh run"
        echo "   仅重签：./scripts/build.sh resign"
        ;;
    *)
        echo "未知动作：$ACTION（可用：build / install / run / resign）" >&2
        exit 1
        ;;
esac
