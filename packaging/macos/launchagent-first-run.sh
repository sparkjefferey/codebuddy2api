#!/usr/bin/env bash
#
# launchagent-first-run.sh — WorkBuddy 网关“安装/自启”脚本。
# 双击运行即可：注册 launchd 自启 + 立刻启动 + 打开控制台( http://127.0.0.1:8787 )。
#
# 同时兼容两种形态，自动定位二进制：
#   .app  : .../gateway.app/Contents/Resources/launchagent-first-run.sh
#           → 二进制在 Contents/MacOS/gateway
#   便携  : <解压目录>/launchagent-first-run.sh → 二进制在同目录 gateway
# 卸载  : ./launchagent-first-run.sh uninstall
#
# 适用范围说明：
#   这是【无头网关】产物(gateway.app / 便携 gateway/)的安装入口 —— 它没有界面，
#   需要一个脚本来「注册自启 + 立即启动 + 打开控制台」。
#   【原生 App】产物(API Transmitter.app)不需要本脚本：拖进「应用程序」双击
#   即可运行；要登录自启，在系统设置 → 通用 → 登录项与扩展 里添加，或用仓库的
#   MODE=app ./scripts/install-launchagent.sh 注册。
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LABEL="com.apitransmitter.gateway"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOGDIR="$HOME/Library/Logs/ApiTransmitter"
LOG="$LOGDIR/gateway.log"

# 定位二进制
if [[ -x "$HERE/gateway" ]]; then
  BIN="$HERE/gateway"                       # 便携目录形态
elif [[ -x "$HERE/../../MacOS/gateway" ]]; then
  BIN="$(cd "$HERE/../.." && pwd)/MacOS/gateway"   # .app 形态
else
  echo "❌ 找不到 gateway 可执行文件（$HERE）。" >&2
  echo "   预期：便携目录内同名，或 .app/Contents/MacOS/gateway。" >&2
  exit 1
fi

if [[ "${1:-}" == "uninstall" ]]; then
  launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
  rm -f "$PLIST"
  echo "已卸载 $LABEL（日志保留在 $LOG）"
  exit 0
fi

mkdir -p "$HOME/Library/LaunchAgents" "$LOGDIR"
cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN</string>
    <string>--desensitize</string>
    <string>--skip-check</string>
    <string>--host</string><string>127.0.0.1</string>
    <string>--port</string><string>8787</string>
    <string>--log</string><string>$LOG</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
</dict>
</plist>
EOF

launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"

echo "✅ 已安装并启动 $LABEL"
echo "   二进制: $BIN"
echo "   日志  : $LOG"
echo "   卸载  : $HERE/launchagent-first-run.sh uninstall"

# 前端是浏览器控制台，而非图形窗口；顺带打开。
sleep 1
open "http://127.0.0.1:8787" 2>/dev/null || true