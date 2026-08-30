# workbuddy2api — WorkBuddy 算力转接网关

把 **WorkBuddy / CodeBuddy(腾讯桌面 AI Agent)** 的本机登录态,转成本地 **OpenAI / Anthropic / Responses 三协议 API**,给**任意 agent** 复用:
Claude Code、Codex CLI、Cherry Studio、ZCode、LobeChat、NextChat、Open WebUI、自己的 SDK 客户端……

多账号、配置驱动、带积分查询与每日签到,上游鉴权失败自动故障切换。

> ⚠️ 非官方逆向项目,依赖腾讯私有接口(copilot.tencent.com / www.workbuddy.ai)。
> **仅限个人学习、自有账号、非商用**;上游改版可能停用,需跟进上游参照实现。
> 与腾讯、OpenAI、Anthropic 无官方关联。使用风险自负。

## 架构

```
Claude Code / Codex CLI / Cherry Studio / 任意 SDK
        │  /v1/chat  |  /v1/messages  |  /v1/responses
        ▼
┌──────────────────────────────────────────────────┐
│ converter.py  FastAPI 网关  (默认 127.0.0.1:8787) │
│  多账号池(AccountPool) → 按 头/模型能力 路由      │
│  故障切换 · 首条system补全(global) · 脱敏 · 日志轮转 │
└──────┬─────────────────────────────┬─────────────┘
       ▼                             ▼
copilot.tencent.com        www.workbuddy.ai
   (cn 账号)                  (global 账号,部分模型免费)
```

## 环境准备

前置:本机已安装并**登录** WorkBuddy / CodeBuddy 桌面端。登录态默认位置:

- macOS: `~/Library/Application Support/CodeBuddyExtension/Data/Public/auth/*.info`
- Windows: `%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\*.info`
- Linux: `~/.local/share/CodeBuddyExtension/Data/Public/auth/*.info`

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt   # 或: uv venv && uv pip install -r requirements.txt
```

## 快速开始

```bash
# 从仓库根目录运行(src/ 下的 codebuddy2api 包)
PYTHONPATH=src .venv/bin/python -m codebuddy2api.converter --desensitize --log gateway.log
```

`GET /health`、`GET /v1/models` 通了即OK。启动时会自动扫描 `*.info`,多个登录态并入账号池。

### 账号路由(多账号)

| 方式 | 说明 |
|---|---|
| 默认 | 按**模型能力**路由:某模型只有某账号后端有时自动选它;重名模型走 `default_account`(默认第一个健康账号) |
| `X-WB-Account: <账号名>` 头 | 强制指定账号(health 里看账号名,如 `cn` / `global`) |
| 模型别名 `global/glm-5.2` / `g:glm-5.2` | 强制路由到 global(国际站)账号 |
| 故障切换 | 鉴权失败(401/403/`ClientApiAuthenticationException`)或模型缺失时,自动尝试下一账号 |

国际站(global)后端要求首条消息为 system,网关自动补齐。
**免费模型(以后端定价元数据为准)**: **CN 账号 `hy3`、`hy4-preview` 免费**(积分倍率 `x0.00`,带"限时免费"角标);global(国际站)账号**暂无确认免费的模型**——经大产出实测 glm-5.2/minimax-m3/auto 等全部计费。其余 CN 模型按倍率计费(如 `default` ×2.20、`glm-5.3` ×0.79、`deepseek-v4-flash` ×0.17)。
> ⚠️ 判定免费 **不能**用"小请求的 `usage.credit==0`"——积分小于 0.01 会被舍入成 0(计费模型同样显示 0,思考型模型耗光 max_tokens 无产出也是 0)。只有两样可信:**后端元数据 `credits==0`**,或**大产出且与计费模型对照时 credit 恒 0**。
`/v1/models` 每条带 `x_free`(CN 由元数据;global 当前未知)与 `x_credits`(倍率,未知为 `—`)。

### 端点

| 端点 | 用途 |
|---|---|
| `GET /` | **控制台(浏览器打开)** |
| `GET/PUT /agents` | 下游 agent 连接偏好的读写(持久化到 config.json) |
| `POST /agents/test` | 一键连通测试(极小请求验证账号+模型可用) |
| `POST /ccswitch/register` | 一键注册 CC Switch:生成 `ccswitch://v1/import` deeplink 并唤起 CC Switch(默认取 claude-code 预设) |
| `GET/PUT /settings` | 路由/同步等可管理设置的读写(persist config.json) |
| `GET /v1/models` | 合并的模型目录(global 模型可走 `global/<id>` 别名;带 `x_free`) |
| `POST /v1/chat/completions` | OpenAI 兼容(含原生 tools/tool_calls/SSE) |
| `POST /v1/responses` | Codex CLI 兼容 |
| `POST /v1/messages` (+ `count_tokens`) | Claude Code / CC Switch 兼容 |
| `GET /health` | 每账号状态/后端/token 有效期 |
| `GET /credits` | 每个账号积分余额 |
| `POST /credits/checkin` | 每日签到领积分 |
| `POST /models/reload` | 立即重拉模型目录 |

### 配置(config.json)

优先级:`config.json`(或 `WB_CONFIG` 指定)< CLI 参数。<模板见 `config.example.json`>。

```json
{
  "host": "127.0.0.1", "port": 8787, "api_key": "",
  "auth_dir": "",
  "default_account": "cn",
  "desensitize": true, "no_compact": false,
  "model_sync_interval_hours": 24,
  "model_overrides": { "global": ["auto", "hy4-preview", "glm-5.3", "glm-5.2", "glm-5v-turbo", "minimax-m3"] },
  "free_models": { "cn": ["hy3", "hy4-preview"] },
  "agents": {
    "claude-code": {"name": "Claude Code", "protocol": "anthropic", "enabled": true, "account": "", "model": "hy3"},
    "codex":       {"name": "Codex CLI", "protocol": "responses", "enabled": true, "account": "", "model": "glm-5.3"},
    "openai":      {"name": "OpenAI 兼容客户端", "protocol": "chat", "enabled": true, "account": "", "model": "auto"}
  },
  "log_file": "gateway.log", "log_max_bytes": 10485760, "log_backups": 3
}
```

CLI 参数: `--host --port --api-key --auth-dir --log --desensitize --no-compact --skip-check --config`

### 控制台(用户前端)

启动网关后打开 **<http://127.0.0.1:8787/>**,是一个**运维控制台**,三个页签:

- **① Agent 连接**:管理要接入的下游 agent(Claude Code / Codex / OpenAI 兼容客户端)。每个 agent 可选**账号 + 模型**(免费模型带 ✨),改动自动保存到 `config.json`;可直接**复制连接配置**、一键「测试连通」(发一个极小请求验证真实可用,含积分消耗)。Claude Code 卡片上还有 **「⚡ 注册 CC Switch」**:一键唤起 CC Switch,把「WorkBuddy 算力网关」预填进它的 provider 导入表单,确认后即切换使用。
- **② WorkBuddy 算力 / 模型**:账号卡片(region / 后端 / token 状态 / 积分余额 / 一键签到)、路由默认(默认账号、模型同步周期)、模型清单与定价(搜索 + 仅免费/计费筛选,`×倍率`、`限时免费`角标、上下文)。
- **③ 模型快速测试**:选账号 + 模型发极小请求,验证可用性与实际积分。

前端是单文件、无构建、离线可用,后端直接托管(`GET /`);网关设置(地址 / API Key)存于浏览器 localStorage。

## 接入客户端

**Claude Code(推荐)** —— 用 `scripts/claude-wb.sh` 一键用 WorkBuddy 算力启动 Claude Code:

```bash
./scripts/claude-wb.sh              # 默认模型 glm-5.2(CN 账号)
WB_MODEL=global/glm-5.3 ./scripts/claude-wb.sh   # 国际站免费模型
```

等价手工方式(CC Switch 亦可配置 base_url=`http://127.0.0.1:8787/v1/messages`):

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:8787
export ANTHROPIC_MODEL=glm-5.2
export ANTHROPIC_API_KEY=anything
claude
```

**Codex CLI** —— 合并 `codex-codebuddy.example.toml` 到 `~/.codex/config.toml`(wire_api=`responses`),然后 `codex --profile workbuddy "任务"`。

**Cherry Studio / ZCode 等** —— base_url=`http://127.0.0.1:8787/v1`,模型名填网关支持的模型(如 `glm-5.2` / `global/glm-5.2`)。

### macOS 开机自启(可选)

```bash
./scripts/install-launchagent.sh            # 安装并启动
./scripts/install-launchagent.sh uninstall  # 卸载
```

## 常用命令

```bash
# 健康 / 模型 / 积分
curl http://127.0.0.1:8787/health
curl http://127.0.0.1:8787/v1/models
curl http://127.0.0.1:8787/credits
curl -X POST http://127.0.0.1:8787/credits/checkin

# 聊天(流式)
curl -N http://127.0.0.1:8787/v1/chat/completions -H "Content-Type: application/json" \
  -d '{"model":"glm-5.2","stream":true,"messages":[{"role":"user","content":"你好"}]}'

# 指定账号
curl ... -H "X-WB-Account: global" -d '{"model":"glm-5.2","messages":[...]}'
```

## 测试

```bash
# 单元测试为开发辅助,未随仓库发布;如本地保留,从仓库根运行:
for t in test_accounts test_routing test_billing test_responses_adapter test_anthropic_adapter; do
  PYTHONPATH=src .venv/bin/python $t.py
done
```

`test_accounts/routing/billing` 不访问网络;协议层测试用模拟 SSE。

## 与脱敏相关

后端有内容审核,agent 运行时的"拒绝作恶"合规模板(DoS/exploit/sandbox…)易被误拦。
`--desensitize` 对这些合规高频词插入零宽空格、压缩 harness 提示;`--no-compact` 保留原文只裁运行时元数据。
若仍被拦,查看日志同一请求 ID 下的 REQUEST BODY 定位触发词。

## 常见问题

- **找不到登录文件**:桌面端没登录,或 `--auth-dir` 指向不对。
- **401**:本地 = `--api-key` 不匹配;后端 = token 失效,重开桌面端或等待自动刷新。
- **model service info not found**:该模型在当前账号不存在,换账号(`X-WB-Account`)或换模型;网关会自动故障切换。
- **被"敏感内容"拦截**:开 `--desensitize --log`,看日志排查触发词。

## 模块一览

模块均位于 `src/codebuddy2api/`:

| 文件 | 职责 |
|---|---|
| `converter.py` | FastAPI 网关:路由/故障切换/三协议端点/积分端点 |
| `accounts.py` | 多账号凭据(读取/自动刷新/回写)、region 判定、后端主机 |
| `models_catalog.py` | 按账号动态拉取模型目录(CN),global 乐观尝试+静态回退,别名 |
| `routing.py` | 账号选择链:头 / 别名 / 模型能力 / 默认账号 |
| `billing.py` | 积分余额查询 + 每日签到 |
| `web/index.html` | 控制台前端(Agent 连接 / 算力模型 / 快速测试,单页无构建) |
| `responses_adapter.py` / `responses_projection.py` | Codex Responses 协议与长上下文投影 |
| `anthropic_adapter.py` | Anthropic Messages 协议转换 |
| `desensitize.py` | 内容审核脱敏/运行时提示压缩 |

## 致谢

基于 [HanHan666666/codebuddy2openai](https://github.com/HanHan666666/codebuddy2openai) 演进而来;
鉴权 header 方案对齐社区参照实现 [Sliverkiss/workbuddy2api](https://github.com/Sliverkiss/workbuddy2api)。

## License

[MIT](./LICENSE)