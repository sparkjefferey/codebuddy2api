#!/usr/bin/env bash
#
# install.sh — 把 WorkBuddy 网关装上并注册为「用户级 systemd 自启」(免 root)。
# 用法: 在解压出的 gateway/ 目录内运行 ./install.sh ；卸载用 ./install.sh uninstall
#
# 会复制本目录到 ~/.local/opt/api-transmitter ，并 enable --user 服务。
#
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"   # 本压缩包解压目录
APP="$HOME/.local/opt/api-transmitter"
UNIT="$HOME/.config/systemd/user/api-transmitter.service"
UNIT_SRC="$SRC/api-transmitter.service"

if [[ ! -x "$SRC/gateway" ]]; then
  echo "错误: 未找到 $SRC/gateway 。请在解压出的 gateway 目录内运行。" >&2
  exit 1
fi

if [[ "${1:-}" == "uninstall" ]]; then
  systemctl --user stop api-transmitter 2>/dev/null || true
  systemctl --user disable api-transmitter 2>/dev/null || true
  rm -f "$UNIT"
  echo "已停止并移除自启服务。目录 $APP 保留;rm -rf 即可彻底删除。"
  exit 0
fi

mkdir -p "$APP" "$HOME/.config/systemd/user"

# 复制本目录(binary + _internal 数据 + 本脚本)
cp -R "$SRC/." "$APP/"
chmod +x "$APP/gateway" "$APP/install.sh" 2>/dev/null || true

cp -f "$UNIT_SRC" "$UNIT"

systemctl --user daemon-reload
systemctl --user enable --now api-transmitter

sleep 1
if systemctl --user is-active --quiet api-transmitter; then
  echo "✅ 网关已安装并运行: ~/.local/opt/api-transmitter"
  echo "   控制台: http://127.0.0.1:8787"
  echo "   状态  : systemctl --user status api-transmitter"
  echo "   日志  : tail -f $APP/gateway.log"
  echo "   卸载  : $APP/install.sh uninstall"
else
  echo "⚠️  服务未启动，请 systemctl --user status api-transmitter 排查。" >&2
  echo "   可选: systemctl --user enable-linger $USER 以让无登录时也自动拉起。" >&2
  exit 1
fi