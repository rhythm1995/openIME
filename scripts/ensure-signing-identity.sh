#!/usr/bin/env bash
# 确保本机钥匙串里有稳定的 openIME 代码签名身份。
#
# 为什么需要：
#   macOS 辅助功能 / 麦克风授权按「代码签名指定要求」(designated requirement) 匹配。
#   ad-hoc 签名每次构建 cdhash 都变 → 授权失效。
#   固定同一份自签证书后，DR 变为 certificate root = H"<固定指纹>"，重装/重编仍有效。
#
# 用法：
#   ./scripts/ensure-signing-identity.sh           # 没有则创建，有则校验
#   ./scripts/ensure-signing-identity.sh --print   # 只打印身份名（供其它脚本 source）
#
# 身份名固定为：openIME Local Dev（与 tauri.conf / build.sh 一致）

set -euo pipefail

IDENTITY_NAME="openIME Local Dev"
# 证书指纹（SHA-1 of public cert）写入仓库，便于确认「整台机器是否同一份」
# 注意：这是公钥指纹，不是私钥；私钥只存在本机钥匙串。
FINGERPRINT_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/scripts/signing-identity.fingerprint"

print_only=0
if [[ "${1:-}" == "--print" ]]; then
  print_only=1
fi

has_identity() {
  security find-identity -v -p codesigning 2>/dev/null \
    | grep -F "\"${IDENTITY_NAME}\"" >/dev/null 2>&1
}

current_fingerprint() {
  # identity 行： 1) HASH "openIME Local Dev"
  security find-identity -v -p codesigning 2>/dev/null \
    | awk -v n="$IDENTITY_NAME" -F'"' '$0 ~ n {print $1}' \
    | awk '{print $2}' \
    | head -1 \
    | tr '[:lower:]' '[:upper:]'
}

create_identity() {
  echo "==> 创建稳定代码签名证书：${IDENTITY_NAME}"
  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  # 自签 codesigning 证书，10 年有效；CN 必须与 IDENTITY_NAME 完全一致
  openssl req -x509 -newkey rsa:2048 \
    -keyout "$tmp/key.pem" -out "$tmp/cert.pem" \
    -days 3650 -nodes \
    -subj "/CN=${IDENTITY_NAME}/OU=openIME/O=openIME/C=CN" \
    -addext "extendedKeyUsage=codeSigning" \
    -addext "keyUsage=digitalSignature" \
    >/dev/null 2>&1

  openssl pkcs12 -export -legacy \
    -out "$tmp/id.p12" \
    -inkey "$tmp/key.pem" \
    -in "$tmp/cert.pem" \
    -passout pass:openime-local-dev \
    >/dev/null 2>&1

  local keychain="${HOME}/Library/Keychains/login.keychain-db"
  if [[ ! -f "$keychain" ]]; then
    keychain="${HOME}/Library/Keychains/login.keychain"
  fi

  security import "$tmp/id.p12" \
    -k "$keychain" \
    -P openime-local-dev \
    -T /usr/bin/codesign \
    -T /usr/bin/security \
    >/dev/null

  # 允许 codesign 在无 UI 时使用该私钥
  security set-key-partition-list \
    -S apple-tool:,apple:,codesign: \
    -s -k "" "$keychain" >/dev/null 2>&1 || true

  # 信任该证书用于 Code Signing（用户级 trust settings）
  security add-trusted-cert -p codeSign -r trustRoot \
    -k "$keychain" "$tmp/cert.pem" >/dev/null 2>&1 \
    || security add-trusted-cert -p codeSign \
      -k "$keychain" "$tmp/cert.pem" >/dev/null 2>&1 \
    || true

  if ! has_identity; then
    echo "错误：证书已导入，但 security find-identity 仍找不到 \"${IDENTITY_NAME}\"" >&2
    echo "请打开「钥匙串访问」→ 登录 → 我的证书，确认存在且有私钥。" >&2
    exit 1
  fi
  echo "   已创建并导入：${IDENTITY_NAME}"
}

write_fingerprint_file() {
  local fp
  fp="$(current_fingerprint)"
  if [[ -z "$fp" ]]; then
    echo "错误：无法读取证书指纹" >&2
    exit 1
  fi
  # 仓库内记录「期望指纹」；本机指纹不一致时 build 会警告
  if [[ ! -f "$FINGERPRINT_FILE" ]]; then
    printf '%s\n' "$fp" >"$FINGERPRINT_FILE"
    echo "   已写入指纹文件：scripts/signing-identity.fingerprint"
  fi
}

check_fingerprint() {
  local fp expected
  fp="$(current_fingerprint)"
  if [[ -f "$FINGERPRINT_FILE" ]]; then
    expected="$(tr -d '[:space:]' <"$FINGERPRINT_FILE" | tr '[:lower:]' '[:upper:]')"
    if [[ -n "$expected" && "$fp" != "$expected" ]]; then
      echo "警告：本机「${IDENTITY_NAME}」指纹与仓库记录不一致" >&2
      echo "  本机：$fp" >&2
      echo "  仓库：$expected" >&2
      echo "  授权可能无法在多机构建间共享；本机内重编仍稳定。" >&2
      echo "  若要与仓库对齐：删除钥匙串中的旧证书后重跑本脚本，或更新 fingerprint 文件。" >&2
    fi
  fi
  echo "$fp"
}

# ── main ──
if ! has_identity; then
  if [[ "$print_only" -eq 1 ]]; then
    echo "错误：未找到签名身份 \"${IDENTITY_NAME}\"。请先运行：./scripts/ensure-signing-identity.sh" >&2
    exit 1
  fi
  create_identity
fi

FP="$(check_fingerprint)"
write_fingerprint_file

if [[ "$print_only" -eq 1 ]]; then
  echo "$IDENTITY_NAME"
  exit 0
fi

echo "==> 签名身份就绪"
echo "   名称：${IDENTITY_NAME}"
echo "   指纹：${FP}"
echo "   指定要求形态：certificate root = H\"${FP}\"（跨构建稳定）"
