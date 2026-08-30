#!/usr/bin/env bash
#
# build-release-local.sh — 本地 macOS 打包(不签名/公证，用于快速验证 spec + 冒烟)。
# 用法: ./scripts/build-release-local.sh [版本号, 默认 0.0.0-local]
# 产出: dist/gateway.app(可拖动)、dist/gateway/(便携目录)、gateway.log
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

V="${1:-0.0.0-local}"
V="${V#v}"
echo "$V" > VERSION
echo "◆ 打包版本: $V"

if ! command -v pyinstaller >/dev/null 2>&1; then
  python -m pip install --quiet pyinstaller pyinstaller-hooks-contrib
fi

OS=mac pyinstaller packaging/gateway.spec --clean --noconfirm

echo
echo "✅ 产物:"
echo "   .app     dist/gateway.app"
echo "   便携目录 dist/gateway/"
echo
echo "冒烟测试:"
echo "   dist/gateway/gateway --desensitize --skip-check --log gateway.log &"
echo "   curl -s 127.0.0.1:8787/health | python3 -m json.tool   # 看 version"
echo "   curl -s 127.0.0.1:8787/ | head -c 80                    # 前端应渲染"
echo
echo "清理: rm -rf build dist VERSION gateway.log"