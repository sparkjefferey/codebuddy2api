#!/usr/bin/env bash
#
# install-launchagent.sh — macOS 一键启停 API Transmitter(LaunchAgent)。
#
# 用法:
#   ./scripts/install-launchagent.sh            # 安装(写 plist)并启动
#   ./scripts/install-launchagent.sh start      # 启动(需已安装)
#   ./scripts/install-launchagent.sh stop       # 停止(保留配置)
#   ./scripts/install-launchagent.sh status     # 查看状态
#   ./scripts/install-launchagent.sh uninstall  # 卸载并停止
#
# 说明:
#   - 默认装【无头网关】(converter),界面在浏览器/PWA 打开;
#   - 环境变量 MODE=app 装【原生桌面 App】(app.py:网关同进程 + 原生窗口 + 菜单栏);
#   - 安装后随登录自启、日志轮转,意外退出由 launchd 拉起。
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${MODE:-gateway}"                 # gateway(无头) | app(原生 App)
# 原生 App 用独立 label,便于与无头网关并存(两者可同时装)
LABEL="${LABEL:-$( [[ "$MODE" == "app" ]] && echo "com.apitransmitter.gateway.app" || echo "com.apitransmitter.gateway" )}"
PYBIN="$ROOT/.venv/bin/python"
LOG="$ROOT/gateway.log"
CMD="${1:-install}"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"

log()  { echo "$@"; }
run()  { launchctl bootout "gui/$(id -u)" "$PLIST" 2>/dev/null || true; }
loaded(){ launchctl list | grep -q "$LABEL"; }
load() { run; launchctl bootstrap "gui/$(id -u)" "$PLIST" "$@"; }

write_plist() {
  mkdir -p "$HOME/Library/LaunchAgents"
  cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PYTHONPATH</key><string>$ROOT/src</string>
  </dict>
  <key>ProgramArguments</key>
  <array>
    <string>$PYBIN</string>
    <string>-m</string>
    <string>$( [[ "$MODE" == "app" ]] && echo "codebuddy2api.app" || echo "codebuddy2api.converter" )</string>
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
}

case "$CMD" in
  install)
    if [ ! -x "$PYBIN" ]; then
      echo "❌ 找不到 venv($PYBIN),先: uv venv && uv pip install -r requirements.txt" >&2
      exit 1
    fi
    write_plist
    load
    log "✅ 已安装并启动 $LABEL ( $PLIST )"
    log "   模式    : $( [[ "$MODE" == "app" ]] && echo '原生 App(窗口+菜单栏)' || echo '无头网关(浏览器打开)' )"
    log "   状态    : MODE=$MODE $ROOT/scripts/install-launchagent.sh status"
    log "   停止    : MODE=$MODE $ROOT/scripts/install-launchagent.sh stop"
    log "   日志    : $LOG"
    ;;
  start)
    [ -f "$PLIST" ] || { log "未安装,先执行 install"; exit 1; }
    load
    log "✅ 已启动 $LABEL"
    ;;
  stop)
    run
    log "⏹ 已停止 $LABEL(配置保留,可 start 恢复)"
    ;;
  status)
    if loaded; then
      pid=$(launchctl list | awk -v K="$LABEL" '$NF==K{print $1}')
      log "● $LABEL 运行中 (pid=$pid)"
    else
      [ -f "$PLIST" ] && log "○ $LABEL 已安装,未运行(start 启动)" || log "○ $LABEL 未安装(install 安装)"
    fi
    ;;
  uninstall)
    run
    rm -f "$PLIST"
    log "已卸载 $LABEL"
    ;;
  *) log "未知参数: $CMD (可用 install/start/stop/status/uninstall)"; exit 2;;
esac
