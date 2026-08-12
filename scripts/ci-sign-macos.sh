#!/usr/bin/env bash
# CI 用 macOS 一次性自签证书（内测包，非 Developer ID 公证）。
#
# 本机 release 用「openIME Local Dev」固定身份（私钥仅在本机钥匙串）；
# CI 无法复用。此脚本在 CI 上生成一次性自签身份并写入临时 keychain，
# 用后随 runner 销毁。tauri build 通过 --config 覆盖 signingIdentity。
#
# 用法（GitHub Actions macos runner）：
#   source ./scripts/ci-sign-macos.sh     # 设置 CI_SIGNING_IDENTITY 环境变量
#   pnpm tauri build --features ... --config "{\"bundle\":{\"macOS\":{\"signingIdentity\":\"$CI_SIGNING_IDENTITY\"}}}"

set -euo pipefail

IDENTITY_NAME="openIME CI Signing"
KEYCHAIN="openime-ci.keychain"
KEYCHAIN_PASSWORD="$(openssl rand -base64 32)"

echo "==> 创建 CI 临时 keychain：${KEYCHAIN}"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
security set-keychain-settings -lut 21600 "$KEYCHAIN" >/dev/null
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
security list-keychains -d user -s "$KEYCHAIN" "$(security list-keychains -d user | sed 's/^ *//;s/ *$//' | tr '\n' ' ')" >/dev/null 2>&1 || \
  security list-keychains -s "$KEYCHAIN" >/dev/null

echo "==> 生成一次性自签证书：${IDENTITY_NAME}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' RETURN

openssl req -x509 -newkey rsa:2048 \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" \
  -days 7 -nodes \
  -subj "/CN=${IDENTITY_NAME}/OU=openIME CI/O=openIME/C=CN" \
  -addext "extendedKeyUsage=codeSigning" \
  -addext "keyUsage=digitalSignature" \
  >/dev/null 2>&1

openssl pkcs12 -export -legacy \
  -out "$tmp/id.p12" \
  -inkey "$tmp/key.pem" \
  -in "$tmp/cert.pem" \
  -passout pass:openime-ci \
  >/dev/null 2>&1

security import "$tmp/id.p12" \
  -k "$KEYCHAIN" \
  -P openime-ci \
  -T /usr/bin/codesign \
  -T /usr/bin/security \
  >/dev/null

security set-key-partition-list \
  -S apple-tool:,apple:,codesign: \
  -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null

echo "==> 验证身份可用"
security find-identity -v -p codesigning "$KEYCHAIN" 2>/dev/null | grep -F "\"${IDENTITY_NAME}\"" >/dev/null

export CI_SIGNING_IDENTITY="${IDENTITY_NAME}"
echo "==> CI_SIGNING_IDENTITY=${CI_SIGNING_IDENTITY}"
