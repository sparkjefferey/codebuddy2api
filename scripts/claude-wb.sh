#!/usr/bin/env bash
#
# claude-wb — 用 WorkBuddy 算力启动 Claude Code(不走 Anthropic 官方计费)。
#
# 前置: 已启动本网关(默认 http://127.0.0.1:8787)。
# 用法: ./scripts/claude-wb.sh [claude 参数...]
#   WB_BASE_URL=...  网关地址(默认 http://127.0.0.1:8787)
#   WB_MODEL=...     模型(默认 glm-5.2;global 免费可用 global/glm-5.3)
#   WB_API_KEY=...   网关设置的 api key(默认 workbuddy 占位)
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BASE="${WB_BASE_URL:-http://127.0.0.1:8787}"
MODEL="${WB_MODEL:-glm-5.2}"
KEY="${WB_API_KEY:-workbuddy}"

# 健康检查,给出友好提示
if ! curl -s -m 2 "$BASE/health" >/dev/null 2>&1; then
  echo "⚠️  网关未运行( $BASE )。先启动:" >&2
  echo "    PYTHONPATH=$ROOT/src $ROOT/.venv/bin/python -m codebuddy2api.converter --desensitize --log $ROOT/gateway.log" >&2
  exit 1
fi

export ANTHROPIC_BASE_URL="$BASE"
export ANTHROPIC_MODEL="$MODEL"
export ANTHROPIC_API_KEY="$KEY"

echo "◆ Claude Code → workbuddy 网关 $BASE | model=$MODEL" >&2
exec claude "$@"