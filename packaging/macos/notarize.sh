#!/usr/bin/env bash
#
# notarize.sh — 签 macOS .app + 出 .dmg + Apple 公证/stapl e。
# 用法: 在打包 repo 根运行（dist/gateway.app 已由 PyInstaller 生成）。
#   ./packaging/macos/notarize.sh <out-dmg-path>
#
# 环境变量（优先走仓库 Secret 名称，便于 workflow 直接注入）:
#   MACOS_DEV_ID_CERT      签名身份，如 "Developer ID Application: xx (TEAM)"
#   MACOS_CERT_P12         可选：base64 的 Developer ID .p12
#   MACOS_CERT_PASSWORD    p12 口令
#   MACOS_P12_FILE         可选：p12 落盘路径（默认 $RUNNER_TEMP/_dev.p12）
#   APPLE_NOTARIZE_API_KEY    AuthKey_*.p8 内容（base64 或原文）
#   APPLE_NOTARIZE_API_KEY_ID / APPLE_NOTARIZE_TEAM_ID
#
# 未配签名/公证 secrets 时仍出【未公证】的 .dmg（Gatekeeper 右键打开），并明确提示。
#
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP="dist/gateway.app"
OUT="${1:-gateway-mac.dmg}"
VOL="WorkBuddy Gateway"

: "${MACOS_DEV_ID_CERT:=}"

# ---- 1) 导入签名证书(可选) ----
if [[ -n "${MACOS_CERT_P12:-}" ]]; then
  tmp="${RUNNER_TEMP:-/tmp}"
  mkdir -p "$tmp"
  p12="${MACOS_P12_FILE:-"$tmp/_dev.p12"}"
  echo "::group::Import signing cert"
  echo "$MACOS_CERT_P12" | base64 --decode > "$p12"
  KEYCHAIN="$tmp/_signing.keychain-db"
  security create-keychain -p temp "$KEYCHAIN" >/dev/null 2>&1 || true
  security default-keychain -s "$KEYCHAIN"
  security unlock-keychain -p temp "$KEYCHAIN"
  security import "$p12" -P "${MACOS_CERT_PASSWORD:-}" -A -t cert -f pkcs12 -k "$KEYCHAIN"
  security set-key-partition-list -S apple-tool:,apple: -s -k temp "$KEYCHAIN" >/dev/null 2>&1 || true
  echo "::endgroup::"
fi

have_sign=0
if [[ -n "$MACOS_DEV_ID_CERT" && -x "$(command -v codesign)" ]]; then
  have_sign=1
fi

# ---- 2) 签 .app（保留 hardened runtime + timestamp，公证必需） ----
if [[ "$have_sign" == "1" ]]; then
  echo "::group::Codesign .app with hardened runtime"
  # 深层签名 bundle 内所有二进制（PyInstaller 内嵌扩展）。
  codesign --force --deep --options runtime --timestamp --sign "$MACOS_DEV_ID_CERT" "$APP"
  codesign --verify --deep --strict "$APP"
  echo "::endgroup::"
else
  echo "⚠️  MACOS_DEV_ID_CERT 未配置，跳过签名（.dmg 未公证，Gatekeeper 需右键打开）。"
fi

# ---- 3) 出 .dmg ----
# "Resource busy" 是 macOS CI runner 上新装 .app 被 Spotlight/mds 即时索引的已知瞬态，重试即可。
echo "::group::Create .dmg"
dmg_ok=0
for i in 1 2 3; do
  if hdiutil create -volname "$VOL" -srcfolder "$APP" -ov -format UDZO "$OUT" 2>dmg_err.txt; then
    dmg_ok=1
    break
  fi
  echo "⚠️  hdiutil 第 $i 次失败($(tr -d '\r' < dmg_err.txt | tail -1))，重试…"
  sleep 5
done
if [[ "$dmg_ok" != "1" || ! -f "$OUT" ]]; then
  echo "❌ 三次 hdiutil 仍失败，放弃。"
  cat dmg_err.txt; rm -f dmg_err.txt
  exit 1
fi
rm -f dmg_err.txt
echo "::endgroup::"

# ---- 4) 公证(.dmg) + stapl e ----
if [[ -n "${APPLE_NOTARIZE_API_KEY:-}" && -n "${APPLE_NOTARIZE_API_KEY_ID:-}" && -n "${APPLE_NOTARIZE_TEAM_ID:-}" ]]; then
  if [[ "$have_sign" == "0" ]]; then
    echo "⚠️  有公证凭据但无签名身份($MACOS_DEV_ID_CERT)。跳过公证（公证前提是先签名）。"
  else
    echo "::group::Notarize .dmg"
    tmp="${RUNNER_TEMP:-/tmp}"
    mkdir -p "$tmp"
    key="$tmp/AuthKey.p8"
    echo "$APPLE_NOTARIZE_API_KEY" | base64 --decode > "$key" 2>/dev/null \
      || echo "$APPLE_NOTARIZE_API_KEY" > "$key"
    # 必须 --wait：否则后台异步，CI 会把未公证 dmg 上传。
    if ! xcrun notarytool submit "$OUT" \
        --key "$key" \
        --key-id "$APPLE_NOTARIZE_API_KEY_ID" \
        --team-id "$APPLE_NOTARIZE_TEAM_ID" \
        --wait; then
      echo "❌ notarize 失败，按失败产物上传（未 stapl e）。"
      exit 0
    fi
    if xcrun stapler staple "$OUT"; then
      echo "✅ stapl e OK"
    else
      echo "⚠️  stapler 失败（Gatekeeper 仍可右键打开）。"
    fi
    echo "::endgroup::"
  fi
else
  echo "⚠️  未配置公证 secrets，跳过 notarize（Gatekeeper 需右键打开）。"
fi

echo "✅ 产物: $OUT"