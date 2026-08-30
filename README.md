# workbuddy2api — WorkBuddy 算力转接网关

把 **WorkBuddy / CodeBuddy(腾讯桌面 AI Agent)** 的本机登录态,转成本地 **OpenAI / Anthropic / Responses 三协议 API**,给**任意 agent** 复用:
Claude Code、Codex CLI、Cherry Studio、ZCode、LobeChat、NextChat、Open WebUI、自己的 SDK 客户端……

多账号、配置驱动、带积分查询与每日签到,上游鉴权失败自动故障切换。

**两种运行形态,同一个网关、同一份前端:**

| 形态 | 启动方式 | 界面 | 适合 |
|---|---|---|---|
| **原生桌面 App**(推荐) | `python -m codebuddy2api.app` | 独立原生窗口(WKWebView / WebView2 / WebKitGTK),macOS 菜单栏常驻 | 日常桌面使用 |
| **无头网关 + 浏览器** | `python -m codebuddy2api.converter` | 浏览器打开 `http://127.0.0.1:8787`(可装为 PWA) | 服务器 / SSH / 无显示器 |

> ⚠️ 非官方逆向项目,依赖腾讯私有接口(copilot.tencent.com / www.workbuddy.ai)。
> **仅限个人学习、自有账号、非商用**;上游改版可能停用,需跟进上游参照实现。
> 与腾讯、OpenAI、Anthropic 无官方关联。使用风险自负。

## 目录结构

```
src/codebuddy2api/
  app.py                 原生桌面 App 入口(网关后台线程 + 原生窗口 + macOS 菜单栏)
  converter.py           FastAPI 网关本体:三协议端点 / 路由 / 故障切换 / 积分(亦可单独 headless 运行)
  accounts.py            多账号凭据(读取/自动刷新/回写)、region 判定、后端主机
  models_catalog.py      按账号动态拉取模型目录 + 静态回退 + 别名
  routing.py             账号选择链:头 / 别名 / 模型能力 / 默认账号
  billing.py             积分余额查询 + 每日签到
  responses_adapter.py   Codex Responses 协议转换
  responses_projection.py Responses 长上下文投影
  anthropic_adapter.py   Anthropic Messages 协议转换
  desensitize.py         内容审核脱敏 / 运行时提示压缩
  ccswitch.py            CC Switch deeplink 一键注册
  config.py              配置加载与合并
web/                     前端单页(原生窗口与浏览器共用同一份)
  index.html             控制台:Agent 连接 / 算力模型 / 快速测试
  sw.js, manifest.webmanifest, icons/     PWA 资源(仅浏览器形态用)
scripts/                 各系统一键启停/自启、本地打包、图标生成
packaging/               PyInstaller spec + 各平台安装脚本
```

## 环境准备

前置:本机已安装并**登录** WorkBuddy / CodeBuddy 桌面端。登录态默认位置:

- macOS: `~/Library/Application Support/CodeBuddyExtension/Data/Public/auth/*.info`
- Windows: `%LOCALAPPDATA%\CodeBuddyExtension\Data\Public/auth/*.info`
- Linux: `~/.local/share/CodeBuddyExtension/Data/Public/auth/*.info`

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
# 用原生 App 再追加一份(各平台后端:macOS=pyobjc / Windows=pythonnet / Linux=PyGObject):
.venv/bin/pip install -r requirements-app.txt
```

## 快速开始

### 原生 App(推荐)

```bash
# 源码树运行(从仓库根目录)
PYTHONPATH=src .venv/bin/python -m codebuddy2api.app --desensitize --log gateway.log
```

会打开一个**独立原生窗口**(Dock/任务栏有自己的图标,不是浏览器标签页),网关在同一进程的后台线程运行:

- **macOS**:菜单栏出现常驻图标。关窗只是**隐藏**(网关继续跑),点菜单栏图标恢复;
  菜单里可直接「打开控制台 / 全部账号签到 / 显示配置文件 / 显示日志文件 / 重启网关 / 退出」。
- **Windows / Linux**:关窗即退出。
- 缺失 pywebview 或系统 WebKit 库时,自动降级为 headless 并在 stderr 打印浏览器地址,不会启动失败。

### 无头网关 + 浏览器

```bash
PYTHONPATH=src .venv/bin/python -m codebuddy2api.converter --desensitize --log gateway.log
# 浏览器打开 http://127.0.0.1:8787
```

`GET /health`、`GET /v1/models` 通了即 OK。启动时会自动扫描 `*.info`,多个登录态并入账号池。

### App 专属参数

`app.py` 接受 `converter.py` 的全部参数,另加:

| 参数 | 说明 |
|---|---|
| `--no-window` | 只跑网关,不开窗口(等价 converter,用于服务器/自启) |
| `--window-only` | 只开窗口,连已运行的网关(不重复占端口) |
| `--no-menubar` | 禁用 macOS 菜单栏常驻(关窗即退出) |
| `--debug` | pywebview 调试模式(窗口内可右键「检查元素」) |

## 账号路由(多账号)

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

## 端点

| 端点 | 用途 |
|---|---|
| `GET /` | 控制台页面(原生窗口内嵌它;浏览器形态直接打开) |
| `GET/PUT /agents` | 下游 agent 连接偏好的读写(持久化到 config.json) |
| `POST /agents/test` | 一键连通测试(极小请求验证账号+模型可用) |
| `POST /ccswitch/register` | 一键注册 CC Switch:生成 `ccswitch://v1/import` deeplink 并唤起 CC Switch |
| `GET/PUT /settings` | 路由/同步等可管理设置的读写 |
| `GET /v1/models` | 合并的模型目录(global 模型可走 `global/<id>` 别名;带 `x_free`) |
| `POST /v1/chat/completions` | OpenAI 兼容(含原生 tools/tool_calls/SSE) |
| `POST /v1/responses` | Codex CLI 兼容 |
| `POST /v1/messages` (+ `count_tokens`) | Claude Code / CC Switch 兼容 |
| `GET /health` | 每账号状态/后端/token 有效期/版本号 |
| `GET /credits` | 每个账号积分余额 |
| `POST /credits/checkin` | 每日签到领积分 |
| `POST /models/reload` | 立即重拉模型目录 |

## 配置(config.json)

优先级:`config.json`(或 `WB_CONFIG` 指定)< CLI 参数。<模板见 `templates/config.example.json`>。

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

## 控制台界面

原生窗口与浏览器渲染的是**同一个** `web/index.html`,三个页签:

- **① Agent 连接**:管理要接入的下游 agent(Claude Code / Codex / OpenAI 兼容客户端)。每个 agent 可选**账号 + 模型**(免费模型带 ✨),改动自动保存到 `config.json`;可直接**复制连接配置**、一键「测试连通」(发一个极小请求验证真实可用,含积分消耗)。Claude Code 卡片上还有 **「⚡ 注册 CC Switch」**:一键唤起 CC Switch,把「API Transmitter」预填进它的 provider 导入表单。
- **② WorkBuddy 算力 / 模型**:账号卡片(region / 后端 / token 状态 / 积分余额 / 一键签到)、路由默认(默认账号、模型同步周期)、模型清单与定价(搜索 + 仅免费/计费筛选,`×倍率`、`限时免费`角标、上下文)。
- **③ 模型快速测试**:选账号 + 模型发极小请求,验证可用性与实际积分。

**原生形态下多出来的能力**(网页做不到的):
标题栏可拖拽移动窗口(`?native=1` 时前端启用 `html.native` 样式);`Cmd/Ctrl+W` 隐藏窗口;复制走**系统剪贴板**;菜单栏「显示配置文件 / 显示日志文件」直接在 Finder 里定位。网关地址与 API Key 只在浏览器形态下存 localStorage——原生形态网关就在本进程,直接用同源地址。

## 接入客户端

**Claude Code(推荐)** —— 用 `scripts/claude-wb.sh` 一键用 WorkBuddy 算力启动 Claude Code:

```bash
./scripts/claude-wb.sh              # 默认模型 glm-5.2(CN 账号)
WB_MODEL=global/glm-5.3 ./scripts/claude-wb.sh   # 国际站模型
```

等价手工方式(CC Switch 亦可配置 base_url=`http://127.0.0.1:8787/v1/messages`):

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:8787
export ANTHROPIC_MODEL=glm-5.2
export ANTHROPIC_API_KEY=anything
claude
```

**Codex CLI** —— 合并 `templates/codex-codebuddy.example.toml` 到 `~/.codex/config.toml`(wire_api=`responses`),然后 `codex --profile workbuddy "任务"`。

**Cherry Studio / ZCode 等** —— base_url=`http://127.0.0.1:8787/v1`,模型名填网关支持的模型(如 `glm-5.2` / `global/glm-5.2`)。

## 开机自启 / 一键启停

原生 App 与无头网关用**不同的服务名**,可并存不冲突。

| 系统 | 无头网关(默认) | 原生 App |
|---|---|---|
| **macOS** | `./scripts/install-launchagent.sh [install\|start\|stop\|status\|uninstall]` | `MODE=app ./scripts/install-launchagent.sh ...` |
| **Windows** | `scripts\install-windows.bat` | `set MODE=app && scripts\install-windows.bat` |
| **Linux** | `./scripts/install-linux.sh [install\|start\|stop\|restart\|status\|uninstall]` | `MODE=app ./scripts/install-linux.sh ...` |

停止 / 卸载用对应的 `windows-stop.bat` 或脚本的 `stop` / `uninstall` 参数(Windows 需带同样的 `MODE`)。

macOS 另有系统级方式:把 `API Transmitter.app` 拖进「应用程序」,到
**系统设置 → 通用 → 登录项与扩展** 里添加即可登录自启。

### 浏览器形态装为 PWA(可选)

无头网关的控制台是**可安装的 PWA**:浏览器打开 `http://127.0.0.1:8787/` 后,
Safari/Chrome/Edge → 分享或菜单 →「添加到程序坞 / 安装应用」,就有独立窗口和图标。
原生 App 形态不需要这一步(它本来就是独立 App),页面里也会跳过 Service Worker 注册。
图标由 `scripts/gen-icons.py` 生成(需 Pillow),已随仓库提交在 `web/icons/`。

## 自动发布(打包 / 发行)

打 `vX.Y.Z` 的 git tag,CI(`.github/workflows/release.yml`)自动用 **PyInstaller** 构建多平台包并挂到 GitHub Release。每个平台出**两套**产物:

| 平台 | 原生 App | 无头网关 |
|---|---|---|
| macOS arm64 / x86_64 | `WorkBuddy-Gateway-mac-<arch>-app.dmg`(签名+公证 when certs configured)+ `-app.zip` | `gateway-mac-<arch>.dmg` + `-portable.zip` |
| Windows x64 | `WorkBuddy-Gateway-win-x64-app.zip` | `gateway-win-x64.zip`(含 `install.bat`) |
| Linux x86_64 | `WorkBuddy-Gateway-linux-x64-app.tar.gz` | `gateway-linux-x64.tar.gz`(含 `install.sh` + systemd unit) |

```bash
git tag v0.3.0 && git push origin --tags    # 触发发布
```

- 一个 spec 出两套产物:`TARGET=app|gateway`(`OS=mac|win|linux` 区分平台)。
  App 走 `src/codebuddy2api/app.py`,gateway 走 `converter.py`。
- 前端 `web/` 整个目录(含 icons / manifest / sw.js)打进 bundle;冻结运行时资源从
  `sys._MEIPASS` 解析(`converter._resource_root()`)。
- macOS 免签名时用「右键 → 打开」绕过 Gatekeeper;Windows 未签名会有 SmartScreen 提示(更多信息 → 仍要运行)。
- 本地出包调试: `TARGET=app ./scripts/build-release-local.sh`(不签名/公证,产出 `dist/`)。
- CI 用到的签名/公证 secrets 见 workflow 内注释;未配置时仍能出未公证包。
- 版本号取自 tag(`VERSION` 注入 bundle,`/health` 上报)。`*.info` 登录态在你的本机,不随包、不进 CI。

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

## 与脱敏相关

后端有内容审核,agent 运行时的"拒绝作恶"合规模板(DoS/exploit/sandbox…)易被误拦。
`--desensitize` 对这些合规高频词插入零宽空格、压缩 harness 提示;`--no-compact` 保留原文只裁运行时元数据。
若仍被拦,查看日志同一请求 ID 下的 REQUEST BODY 定位触发词。

## 常见问题

- **找不到登录文件**:桌面端没登录,或 `--auth-dir` 指向不对。
- **401**:本地 = `--api-key` 不匹配;后端 = token 失效,重开桌面端或等待自动刷新。
- **model service info not found**:该模型在当前账号不存在,换账号(`X-WB-Account`)或换模型;网关会自动故障切换。
- **被"敏感内容"拦截**:开 `--desensitize --log`,看日志排查触发词。
- **App 打开后是空白 / 自动降级为 headless**:缺系统 WebKit 库。
  Linux 需 `sudo apt install libwebkit2gtk-4.1-0 gir1.2-webkit2-4.1` 并安装 PyGObject;
  Windows 需 Edge WebView2 Runtime(Win11 自带)。降级时 stderr 会给出浏览器地址,网关本身照常可用。
- **Windows 原生 App 双击一闪即退(无任何报错)**:App 形态默认隐藏控制台(console=False),
  原生窗口后端(Edge WebView2)初始化失败时异常被吞掉。可能原因:
  1) 打包产物缺 WebView2 interop DLL(旧版本 Release 有此问题,见下方「打包说明」);
  2) 目标机没装 **Microsoft Edge WebView2 Runtime**(Win10 / 服务器版可能没有,
     去 `https://developer.microsoft.com/microsoft-edge/webview2/` 装「常青版引导程序」)。
  新版 App 已修复:后端不可用时**弹系统错误框 + 写 `api-transmitter-app.log`**,
  并降级为浏览器控制台(打开 `http://127.0.0.1:8787`),不再静默闪退。
- **端口被占用**:已有网关在跑。App 会提示并复用;换端口用 `--port`。

## 致谢

基于 [HanHan666666/codebuddy2openai](https://github.com/HanHan666666/codebuddy2openai) 演进而来;
鉴权 header 方案对齐社区参照实现 [Sliverkiss/workbuddy2api](https://github.com/Sliverkiss/workbuddy2api)。

## License

[MIT](./LICENSE)
