# ACL 公会账号系统设计（v0.2 规划）

> **已废弃（2026-10-27）。** 身份来源由 Supabase 改为官网「功能页账号」，见 [acl-auth-contract.md](acl-auth-contract.md)。
> Supabase 项目与 Cloudflare Worker 已全部拆除；数据模型后续落在官网 D1（`yzas-db`），
> 表结构沿用本文的字段设计，但 RLS 策略改由官网 Functions 中间件实现。


## 一、目标与边界

**目标**：在保留 Modrinth 登录的前提下，新增「公会账号」体系，承载公会专属内容与成员在线状态。

**不在本期范围**：共享实例托管与分发（依赖对象存储 + 版本管理，暂缓）。

**关键前提**：模组浏览与下载走公开 API，不需要任何登录；Minecraft 进游戏走微软账号，与启动器账号体系无关。因此本期改动不影响核心可用性。

---

## 二、架构

```
┌──────────────┐   PKCE + 深链接    ┌─────────────────────┐
│  ACL 启动器  │ ─────────────────► │ yingzhou1110.top    │
│  (Tauri)     │ ◄───────────────── │ /auth/bridge (静态) │
└──────┬───────┘   acl://auth?code=  └──────────┬──────────┘
       │                                        │ supabase-js
       │  REST + Realtime                       ▼
       └──────────────────────────► ┌─────────────────────┐
                                    │ Supabase            │
                                    │ Auth / Postgres /   │
                                    │ Realtime            │
                                    └─────────────────────┘
```

**登录流程（桌面端标准做法）**

1. 启动器生成 `code_verifier` 与 `code_challenge`，并生成一次性 `state`
2. 打开系统浏览器到 `https://yingzhou1110.top/auth/bridge?challenge=...&state=...`
3. 该页用 supabase-js 完成登录（邮箱密码 / 后续可加 QQ、GitHub OAuth）
4. 登录成功后，页面回调到 `acl://auth/callback?code=<授权码>&state=...`
5. 启动器校验 `state`，用 `code` + `code_verifier` 换取 session（access + refresh token）
6. access token 存系统密钥链（Windows Credential Manager），refresh token 同存；过期自动刷新

> `acl://` 协议需在安装器注册（当前已注册 `modrinth://`，两者并存）。

---

## 三、数据模型（Postgres + RLS）

| 表 | 主要字段 | 说明 |
|---|---|---|
| `profiles` | `id`(=auth.uid) `display_name` `avatar_url` `role` `created_at` | `role` ∈ `member` / `officer` / `admin` |
| `announcements` | `id` `title` `body` `pinned` `published_at` | 公会公告 |
| `guild_packs` | `id` `name` `mc_version` `loader` `url` `sha256` `notes` | 公会整合包，`url` 指向发布直链 |
| `guild_servers` | `id` `name` `host` `port` `description` `whitelist` | 公会服列表 |
| `presence` | `user_id` `instance` `mc_version` `loader` `last_seen` | 在线状态，60 秒心跳 UPSERT |

**RLS 策略（全部表默认拒绝）**

- `profiles`：本人可写，公会成员可读（`role` 非空）
- `announcements` / `guild_packs` / `guild_servers`：`officer` 以上可写，`member` 以上可读
- `presence`：本人可写，`member` 以上可读；查询时过滤 `last_seen > now() - 3min`

---

## 四、启动器改动

### Rust 侧（`packages/app-lib` 新增模块）

| 模块 | 职责 |
|---|---|
| `guild/auth.rs` | PKCE 生成、浏览器唤起、深链接回调监听、token 交换与刷新 |
| `guild/session.rs` | token 存储（Windows 凭据管理器）、自动刷新、登出清理 |
| `guild/api.rs` | 公告 / 整合包 / 服务器 / 在线状态的读写封装 |
| `guild/presence.rs` | 心跳任务：启动时开始，每 60s UPSERT；退出时置离线 |

要求：所有网络失败不得阻塞启动流程，超时 5 秒，错误降级为「公会功能不可用」提示。

### 前端侧

| 位置 | 改动 |
|---|---|
| 账号页 | 保留 Modrinth 登录入口；新增「使用公会账号登录」按钮与登录态卡片 |
| 侧边栏 | 新增「公会」入口（仅公会账号登录后显示） |
| 公会页 | 三个标签：公告 / 整合包 / 服务器；底部显示在线成员头像列表 |
| 设置 → 账号 | 展示两种账号的独立状态与退出按钮 |

**降级规则**：未登录公会账号时，「公会」入口隐藏，不显示报错；Modrinth 相关功能（收藏、合集、共享实例）在只用公会账号时置灰并附提示。

---

## 五、分阶段交付

| 阶段 | 内容 | 预估 |
|---|---|---|
| P1 | Supabase 项目搭建、表结构与 RLS、`/auth/bridge` 静态页、Rust 登录链路 | 2 天 |
| P2 | 公会页：公告 + 整合包 + 服务器列表 | 1.5 天 |
| P3 | 在线状态：心跳上报、Realtime 订阅、成员头像墙 | 1 天 |
| P4 | 公会服一键加入（写入实例 `servers.dat`）、整合包一键安装 | 1 天 |

---

## 六、需要你提供

1. Supabase 项目的 `URL` 与 `anon key`（anon key 可公开，受 RLS 保护）
2. 公会成员名单与角色划分（谁是 officer，谁是 admin）
3. 是否配置自有邮件发信（Supabase 默认邮件有速率限制，可用你已有的阿里云邮件推送接 SMTP）

## 七、风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| Supabase 国内访问速度一般 | 登录与在线状态有延迟 | 可选香港/新加坡 region；或前置自建反代 |
| 第三方托管依赖 | 服务变更需迁移 | 数据模型用标准 Postgres，可平移到自建库 |
| 在线状态被滥用 | 隐私顾虑 | 仅公会成员可见；提供「隐身」开关，默认开启可见 |
| 账号体系两套易混淆 | 用户不知道用哪个 | 账号页明确文案：公会功能用公会账号，收藏合集用 Modrinth 账号 |
