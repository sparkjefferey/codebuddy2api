#!/usr/bin/env bash
#
# install-launchagent.sh — 把 WorkBuddy 算力网关注册为 macOS 开机自启(LaunchAgent)。
#
# 用法:
#   ./scripts/install-launchagent.sh           # 安装并立刻启动
#   ./scripts/install-launchagent.sh uninstall # 卸载并停止
#
# 说明: 这是可选项。安装后网关随登录自动启动、日志轮转,
#   意外退出由 launchd 自动拉起。卸载随时可逆。
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LABEL="com.workbuddy.gateway"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
PYBIN="$ROOT/.venv/bin/python"
LOG="$ROOT/gateway.log"

if [ "${1:-}" = "uninstall" ]; then
  launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
  rm -f "$PLIST"
  echo "已卸载 $LABEL"
  exit 0
fi

if [ ! -x "$PYBIN" ]; then
  echo "❌ 找不到 venv($PYBIN),先: uv venv && uv pip install -r requirements.txt" >&2
  exit 1
fi

mkdir -p "$HOME/Library/LaunchAgents"
cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$PYBIN</string>
    <string>$ROOT/converter.py</string>
    <string>--desensitize</string>
    <string>--skip-check</string>
    <string>--log</string>
    <string>$LOG</string>
  </array>
  <key>WorkingDirectory</key><string>$ROOT</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
EOF

launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
echo "✅ 已安装并启动 $LABEL ( $PLIST )"
echo "   查看状态: launchctl list | grep $LABEL"
echo "   日志    : $LOG"
echo "   卸载    : $ROOT/scripts/install-launchagent.sh uninstall"