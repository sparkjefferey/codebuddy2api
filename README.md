# BuddyAIGateway

**BuddyAIGateway** 是一个本地桌面网关（Tauri v2）：把 CodeBuddy（CN）账号的算力转换为标准 **Anthropic Messages API**，并通过 **CC Switch** 一键接入 Claude Code。

> 非官方逆向项目，依赖腾讯私有接口，仅限个人学习与自有账号使用。上游改版可能随时失效。

## 架构

```text
Claude Code / CC Switch
  └─ http://127.0.0.1:9178   (仅本机)
       └─ Tauri v2 桌面应用
            ├─ Axum HTTP 网关 (Rust)
            │    ├─ POST /v1/messages           Anthropic Messages (SSE 流式)
            │    ├─ POST /v1/messages/count_tokens
            │    ├─ GET  /v1/models             模型目录(含免费/倍率)
            │    ├─ GET  /credits               积分余额
            │    ├─ POST /credits/checkin       每日签到
            │    ├─ POST /models/reload         刷新模型目录
            │    ├─ POST /agents/test           快速连通测试
            │    └─ POST /ccswitch/register     生成 CC Switch 导入链接
            ├─ React + TS + Vite 控制台
            │    总览 / 账号 / 模型 / Claude Code / 活动 / 设置
            └─ 腾讯上游 copilot.tencent.com
```

## 多账号与负载均衡

「账号」页支持导入多个账号（uid 相同视为更新凭据）：

- **轮询负载均衡**：每个请求按顺序在启用账号间均摊，可单独停用/启用任一账号
- **连续 429 自动切换**：单次 429 立即换下一个账号重试；同一账号连续 2 次 429 进入冷却（60s 起逐次翻倍，封顶 30 分钟），冷却期内不参与调度，任一请求成功即恢复
- **Token 自动续期**：临期自动刷新；401/403 先强制刷新重试一次，仍失败才切换账号
- **渠道指纹中和（内置）**：上游风控会扫描请求内容中的官方客户端签名（11128 Illegal API invocation），网关出站前自动在品牌词内插零宽空格绕过，模型语义几乎不受影响；同时为未带 system 的请求补首条 system 消息（workbuddy.ai 后端强制要求）
- 「账号」页实时显示每个账号的冷却/429 状态

## 使用

1. 从 [Releases](../../releases) 下载安装包并安装（Windows `.msi`/`.exe`、macOS `.dmg`、Linux `.deb`/`.AppImage`）
2. 启动应用，在「账号」页粘贴登录态 JSON（桌面端 CodeBuddy 的 `auth/*.info` 内容）
3. 在「Claude Code」页点击 **注册到 CC Switch**，由 CC Switch 完成配置
4. 在 Claude Code 中正常使用

### 手动环境变量接入（不用 CC Switch）

```bash
export ANTHROPIC_BASE_URL=http://127.0.0.1:9178
export ANTHROPIC_API_KEY=<网关设置页显示的 API key>
export ANTHROPIC_MODEL=hy3
claude
```

## 开发

```bash
pnpm install          # 前端依赖
pnpm tauri dev        # 开发模式(热更新)
pnpm tauri build      # 打包
```

Rust 后端测试：

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

## 安全说明

- 网关仅绑定 `127.0.0.1:9178`，不监听局域网
- 首次启动自动生成随机 API key 并强制鉴权
- 登录态凭据保存在本地配置目录（`%APPDATA%/buddyaigateway/config.json`），请勿外传
- 默认日志不记录 Prompt 内容

## License

MIT