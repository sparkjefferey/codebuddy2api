#!/usr/bin/env bash
#
# install-linux.sh — Linux(systemd user)一键启停/自启 WorkBuddy 算力网关。
#
# 用法:
#   ./scripts/install-linux.sh            # 建 venv+装依赖+安装单元并启动
#   ./scripts/install-linux.sh start      # 启动
#   ./scripts/install-linux.sh stop       # 停止
#   ./scripts/install-linux.sh restart    # 重启
#   ./scripts/install-linux.sh status     # 状态
#   ./scripts/install-linux.sh uninstall  # 禁用并移除单元
#
# 用 user 级 systemd(免 root)。日志: journalctl --user -u wb-gateway -f
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UNIT_NAME="wb-gateway"
UNIT="$HOME/.config/systemd/user/$UNIT_NAME.service"
CMD="${1:-install}"

ensure_venv() {
  [ -x "$ROOT/.venv/bin/python" ] || { python3 -m venv "$ROOT/.venv"; }
  "$ROOT/.venv/bin/pip" install --quiet -r "$ROOT/requirements.txt"
}

write_unit() {
  mkdir -p "$HOME/.config/systemd/user"
  cat > "$UNIT" <<EOF
[Unit]
Description=WorkBuddy 算力转接网关
After=network-online.target

[Service]
Type=simple
WorkingDirectory=$ROOT
Environment=PYTHONPATH=$ROOT/src
ExecStart=$ROOT/.venv/bin/python -m codebuddy2api.converter --desensitize --skip-check
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
    echo "   状态 : $0 status | 日志: journalctl --user -u $UNIT_NAME -f"
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