#!/usr/bin/env bash
#
# build-release-local.sh — 本地打包(不签名/公证,用于快速验证 spec + 冒烟)。
#
# 用法:
#   ./scripts/build-release-local.sh [版本号]
#   TARGET=app ./scripts/build-release-local.sh [版本号]      # 打原生 App
#   TARGET=all ./scripts/build-release-local.sh [版本号]      # 两个都打
#
# 环境变量:
#   TARGET  gateway(默认,无头网关) | app(原生 App) | all
#   OS      mac(默认) | win | linux
#
# 产出:
#   TARGET=gateway → dist/gateway.app(可拖动)、dist/gateway/(便携目录)
#   TARGET=app     → dist/API Transmitter.app、dist/ApiTransmitter/
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

V="${1:-0.0.0-local}"
V="${V#v}"
echo "$V" > VERSION
echo "◆ 打包版本: $V"

TARGET="${TARGET:-gateway}"
OS="${OS:-mac}"
echo "◆ TARGET=$TARGET  OS=$OS"

if ! command -v pyinstaller >/dev/null 2>&1; then
  python -m pip install --quiet pyinstaller pyinstaller-hooks-contrib
fi
# 打 App 时才需要 pywebview(及其平台后端)
if [ "$TARGET" != "gateway" ]; then
  python -m pip install --quiet -r requirements.txt -r requirements-app.txt 2>/dev/null \
    || python -m pip install --quiet -r requirements.txt
fi

build() {
  local t="$1"
  echo
  echo "───────── 构建 TARGET=$t ─────────"
  OS="$OS" TARGET="$t" pyinstaller packaging/gateway.spec --clean --noconfirm
}

case "$TARGET" in
  gateway|app) build "$TARGET" ;;
  all)         build gateway; build app ;;
  *) echo "❌ 未知 TARGET=$TARGET (可用 gateway/app/all)" >&2; exit 2 ;;
esac

echo
echo "✅ 产物:"
if [ "$TARGET" != "app" ]; then
  echo "   gateway .app     dist/gateway.app"
  echo "   gateway 便携目录 dist/gateway/"
fi
if [ "$TARGET" != "gateway" ]; then
  echo "   App     .app     dist/API Transmitter.app"
  echo "   App     便携目录 dist/ApiTransmitter/"
fi
echo
echo "冒烟测试:"
echo "   # 原生 App(开窗口 + 网关同进程)"
echo "   open 'dist/API Transmitter.app'"
echo "   curl -s 127.0.0.1:8787/health | python3 -m json.tool"
echo
echo "清理: rm -rf build dist VERSION gateway.log"
