#!/usr/bin/env bash
#
# install-linux.sh — Linux(systemd user)一键启停/自启 API Transmitter。
#
# 用法:
#   ./scripts/install-linux.sh            # 建 venv+装依赖+安装单元并启动
#   ./scripts/install-linux.sh start      # 启动
#   ./scripts/install-linux.sh stop       # 停止
#   ./scripts/install-linux.sh restart    # 重启
#   ./scripts/install-linux.sh status     # 状态
#   ./scripts/install-linux.sh uninstall  # 禁用并移除单元
#
# 环境变量:
#   MODE=gateway(默认,无头网关) | app(原生 App)
#     - gateway: 只跑 HTTP 网关,界面在浏览器/PWA 打开(服务器/无显示器场景);
#     - app    : 原生桌面 App(网关同进程 + 原生窗口),需要 WebKitGTK
#                (Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0 gir1.2-webkit2-4.1)。
#                无显示器时会自动降级为 headless。
#
# 用 user 级 systemd(免 root)。日志: journalctl --user -u <unit> -f
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${MODE:-gateway}"                 # gateway | app
UNIT_NAME="${UNIT_NAME:-$( [[ "$MODE" == "app" ]] && echo "api-transmitter-app" || echo "api-transmitter" )}"
UNIT="$HOME/.config/systemd/user/$UNIT_NAME.service"
CMD="${1:-install}"

ensure_venv() {
  [ -x "$ROOT/.venv/bin/python" ] || { python3 -m venv "$ROOT/.venv"; }
  "$ROOT/.venv/bin/pip" install --quiet -r "$ROOT/requirements.txt"
  if [[ "$MODE" == "app" ]]; then
    # 原生 App 需要 WebKitGTK 绑定;系统库缺失时安装失败也放行(运行时降级 headless)。
    "$ROOT/.venv/bin/pip" install --quiet -r "$ROOT/requirements-app.txt" \
      || echo "⚠️  App 依赖未装全,将降级为 headless(浏览器打开控制台)"
  fi
}

write_unit() {
  mkdir -p "$HOME/.config/systemd/user"
  MODULE="$( [[ "$MODE" == "app" ]] && echo "codebuddy2api.app" || echo "codebuddy2api.converter" )"
  cat > "$UNIT" <<EOF
[Unit]
Description=WorkBuddy 算力转接网关 ($MODE)
After=network-online.target
EOF
  # 原生 App 需要图形会话;无 DISPLAY/WAYLAND_DISPLAY 时不要在 systemd 里硬起。
  if [[ "$MODE" == "app" ]]; then
    cat >> "$UNIT" <<'EOF'
After=graphical-session.target
EOF
  fi
  cat >> "$UNIT" <<EOF

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=PYTHONPATH=$ROOT/src
ExecStart=$ROOT/.venv/bin/python -m $MODULE --desensitize --skip-check
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload
}

act() { systemctl --user "$1" "$UNIT_NAME"; }

case "$CMD" in
  install)
    ensure_venv
    write_unit
    systemctl --user enable --now "$UNIT_NAME"
    echo "✅ 已安装并启动 $UNIT_NAME"
    echo "   模式 : $( [[ "$MODE" == "app" ]] && echo '原生 App(窗口)' || echo '无头网关(浏览器打开)' )"
    echo "   状态 : MODE=$MODE $0 status | 日志: journalctl --user -u $UNIT_NAME -f"
    ;;
  start)  act start;  echo "已启动 $UNIT_NAME" ;;
  stop)   act stop;   echo "已停止 $UNIT_NAME" ;;
  restart) act restart; echo "已重启 $UNIT_NAME" ;;
  status) systemctl --user status "$UNIT_NAME" || true ;;
  uninstall)
    systemctl --user disable --now "$UNIT_NAME" 2>/dev/null || true
    rm -f "$UNIT"
    systemctl --user daemon-reload
    echo "已卸载 $UNIT_NAME"
    ;;
  *) echo "未知参数: $CMD (可用 install/start/stop/restart/status/uninstall)" >&2; exit 2 ;;
esac