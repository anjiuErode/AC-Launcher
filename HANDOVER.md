# ACL 交接文档 · 第一阶段

> 适用版本：`0.1.2-alpha`（2026-10-27）
> 仓库：https://github.com/anjiuErode/code
> 相关契约与决策记录：`docs-acl/`

---

## 0. 一页速览

ACL（AnjiuCraftLauncher）是瀛洲公会自用的 Minecraft 启动器，基于 Modrinth Theseus 的 GPL-3.0 分支改造，**仅发布 Windows 10/11 x64**。

| 项 | 值 |
| --- | --- |
| 应用标识 | `com.anjiu.acl` |
| 当前版本 | `0.1.2` |
| 发布形式 | GitHub Release 预发布（`v0.1.2-alpha`），安装版 + 便携版 |
| 代码签名 | 无（详见 `docs-acl/code-signing-decision.md`） |
| 构建 | GitHub Actions `acl-build.yml`，约 20–24 分钟 |

三条命令覆盖绝大多数场景：

```bash
pnpm app:dev                                  # 开发模式，界面改动秒级热更新
pnpm --filter @modrinth/app build --no-bundle # 本地出 exe，不发布，约 10 分钟
# 仅在需要验证安装器行为或对外分发时才推送并发版
```

---

## 1. 第一阶段已完成内容

| 模块 | 状态 | 主要位置 |
| --- | --- | --- |
| 品牌替换 | 完成 | `apps/app-frontend/src/assets/branding/`、`index.html`、标题栏 |
| 单色主题 | 完成 | 主题变量与 `App.vue` 主题逻辑 |
| 简体中文本地化 | 完成 | `apps/app-frontend/src/locales/zh-CN/` |
| 网络代理设置 | 完成 | `packages/app-lib/src/util/fetch.rs`、`apps/app/src/main.rs`、`features/settings/system/network-settings.vue` |
| 公会账号登录 | 完成（仅登录） | `apps/app/src/guild.rs`、`apps/app/src/main.rs`、侧边栏与 `features/settings/account/guild-account-settings.vue` |
| 远程内容清理 | 完成 | 新闻面板、广告窗、募捐横幅、Hosting 入口、intercom |
| 自动更新 | 已关闭 | 不连接 Modrinth 更新源 |
| 公会数据展示 | **未开始** | 待接口对接（见第 6 节） |

### 1.1 公会账号登录（本阶段最大的改动）

身份来源不是 Modrinth，也不是 Supabase，而是**官网的功能页账号**（`studio_users`，普通成员 `role='user'`）。

流程：

```
点侧边栏按钮
  → 启动器在 127.0.0.1 随机端口监听，生成 state
  → 打开浏览器 https://yingzhou1110.top/acl/auth?port=<p>&state=<s>&v=1
  → 用户在页面登录 / 确认授权
  → 页面跳回 http://127.0.0.1:<p>/callback?state=..&token=..&user_id=..&name=..
  → 启动器校验 state，保存会话，页面提示可关闭
```

关键实现（`apps/app/src/guild.rs`）：

- 回环监听用 `tokio::net::TcpListener`，超时 300 秒
- `state` 为 32 位十六进制随机串，不匹配直接拒绝
- 支持用户取消（`error=cancelled`）
- 会话存 `%APPDATA%\com.anjiu.acl\guild_session.json`，字段仅 `token` / `user_id` / `name`
- **令牌不进入 webview**，前端只能拿到 `user_id` 与 `name`
- 后续调用官网接口时用请求头 `x-studio-token`

Tauri 命令：`guild_begin_login`、`guild_session`、`guild_sign_out`。

### 1.2 网络代理

三种模式写进 `proxy.json`：

| 模式 | 行为 |
| --- | --- |
| `auto`（默认） | 跟随系统代理，其次环境变量 |
| `off` | 强制直连，`no_proxy()` |
| `manual` | 使用指定地址，回环地址始终绕过 |

设置界面带「测试连接」，并发探测 `api.modrinth.com` 与 `login.live.com`，显示状态码与耗时。

**注意**：改动需重启生效（HTTP 客户端在进程启动时构建一次，是 `LazyLock` 静态量）。

---

## 2. 关键设计决策

| 决策 | 理由 | 代价 / 约束 |
| --- | --- | --- |
| 在 Theseus 上 fork，不重写 | 启动器核心（实例管理、Java 发现、模组解析）复杂且成熟，重写不现实 | 需持续跟进上游；上游的大量无关代码与工作流要清理 |
| 只发 Windows x64 | 目标用户全部是 Windows 玩家，跨平台构建成本高 | 上游的 macOS/Linux 构建与本项目无关 |
| 公会身份用官网账号 | 一套账号打通官网与启动器，避免第二套身份 | 启动器依赖官网可用性；令牌有效期跟随官网（7 天） |
| 浏览器授权 + 本机回环 | RFC 8252 原生应用标准做法；不用自定义协议（Windows 上可被抢注，令牌可被截获） | 需要 5 分钟等待窗口；开发调试时浏览器会弹出 |
| Modrinth 账号降级保留 | 收藏、合集、共享实例仍依赖它 | 用户需理解两套账号；侧边栏只留公会入口，Modrinth 在设置里 |
| 移除远程内容（新闻 / 广告 / intercom） | 内容是 Modrinth 的，与公会无关；广告还会加载第三方脚本 | 上游合并时易冲突，需在每次同步后检查 |
| 不签名 | SignPath Foundation 要求项目具备公开可见度，新 fork 结构上无法满足 | 用户首次运行需点「更多信息 → 仍要运行」；详见 `docs-acl/code-signing-decision.md` |
| 放弃 Supabase，数据落官网 D1 | 国内直连 `*.supabase.co` 不可靠；且会形成两套身份 | 已删除 Supabase 项目与 Cloudflare Worker；表结构改由 D1 承载 |

---

## 3. 代码结构与目录

```
code/
├── apps/
│   ├── app/                  Tauri 2 主程序（Rust）
│   │   ├── src/main.rs       应用入口、命令注册、proxy.json 读写、guild 命令
│   │   ├── src/guild.rs      公会账号：回环授权、会话存储
│   │   ├── Cargo.toml        注意：tokio 需 net/io-util feature
│   │   └── tauri.conf.json   标识符、前后端 dev 联动
│   └── app-frontend/         Vue 3 + Vite 前端
│       ├── src/App.vue       侧边栏、导航、账号按钮、主题与全局状态
│       ├── src/app/shell/sidebar/index.vue   侧边栏装配（新闻/广告/募捐已移除）
│       ├── src/features/settings/system/network-settings.vue
│       ├── src/features/settings/account/guild-account-settings.vue
│       └── src/locales/zh-CN/index.json      简体中文文案
├── packages/
│   ├── app-lib/              启动器核心库（crate `theseus`）
│   │   └── src/util/fetch.rs 全局 HTTP 客户端与代理配置
│   ├── ui/                   共享 UI 组件
│   └── assets/               图标与品牌资产
├── supabase/migrations/      ⚠️ 已废弃，见第 5 节技术债务
├── docs-acl/                 本项目自有的契约与决策记录（非上游内容）
└── .github/workflows/
    ├── acl-build.yml         唯一的构建流水线
    ├── check-generic.yml     tombi / typos / shear
    └── check-rust.yml        cargo check
```

**前端注意**：`apps/app-frontend/src/App.vue` 是 `<script setup>`，**没有 `lang="ts"`**。写入 TypeScript 泛型会直接编译失败，改这个文件前先确认。

**Rust 注意**：`packages/app-lib` 的 `util` 是私有模块。新增能力需在 `packages/app-lib/src/lib.rs` 重新导出，否则 `theseus::util::*` 不可见（当前已导出 `REQWEST_CLIENT`、`ProxyConfig`、`ProxyMode` 等）。

---

## 4. 已解决的主要问题

| 现象 | 根因 | 方案 |
| --- | --- | --- |
| 启动器报「无法连接到身份验证服务器」 | 进程继承了指向**已失效端口**的 `HTTPS_PROXY`，reqwest 走 CONNECT 隧道，`os error 10061` | 新增代理设置，支持 `no_proxy()`；同时暴露系统代理能力 |
| 侧边栏三张广告卡删不掉 | 它们来自 `app/shell/sidebar/promotion.vue`（远程广告窗），与已删除的 `news-sidebar.vue` 是两套组件 | 移除 `promotion.vue` 引用 |
| 前端构建报 `Unexpected token, expected ","` | `App.vue` 是纯 JS 的 `<script setup>`，插入了 TS 泛型 | 改为纯 JS 写法 |
| `check-generic` 失败：`Cargo.toml is not formatted` | tombi 要求多元素数组竖排，单行写法不合规 | 展开为多行 |
| Supabase 国内不可达 | `*.supabase.co` DNS 超时，直连失败 | 先试 Cloudflare Worker 反代（可用但多一层）；最终改身份模型，全部拆除 |
| SignPath Foundation 申请被拒 | 该项目要求「已成立的公开项目」，依赖 star / 外部引用 / 贡献者 | 如实更新 README 与 Release 说明，决定暂不签名 |
| `gh run download` 卡死、`git push` 间歇 502、`python` 被杀软锁 | 环境层面的网络与文件锁问题 | 分别改用 artifacts API 直取、`gh release create --target main`、PowerShell 替代 |

---

## 5. 遗留问题与技术债务

| 事项 | 影响 | 建议 |
| --- | --- | --- |
| `supabase/migrations/0001_init.sql` 仍在仓库 | 与实际架构不符，容易误导后来者 | 删除，或移到 `docs-acl/` 作为历史记录 |
| Hosting 路由与 provider 代码保留 | 仅移除了侧边栏入口，通过旧链接仍可进入 | 下一阶段连路由一起摘除，或替换为「公会服务器」页 |
| `PRIVACY.md` 未提及公会令牌本地存储 | 合规瑕疵：我们在本机存了官网会话令牌 | 补充说明存储位置、用途、清除方式 |
| Modrinth 收藏 / 好友 / 共享实例仍可见 | 登录入口移到了设置，用户可能困惑 | 明确文案，或按需隐藏 |
| 无自动化测试 | 每次改动靠人工验证 | 至少为 `guild.rs` 的回调解析与 `fetch.rs` 的代理选择加单测 |
| `i18n:coverage` 前置检查 | 新增文案若未同步语言包，构建早期会报错 | 新增 key 时同时补 `zh-CN` 与 `en-US` |
| `sqlx` 离线查询数据 | 改数据库 schema 需重新生成 `.sqlx`，否则 CI 失败 | 涉及 `settings` 表结构时特别注意 |
| 无代码签名 | 首次运行有 SmartScreen 提示 | 见 `docs-acl/code-signing-decision.md` 的重新评估条件 |
| 仅 Windows x64 | 无法覆盖其他平台 | 确认需求后再考虑 |
| 上游工作流已禁用 15 个 | 同步上游时新出现的工作流仍会自动启用 | 同步后检查 Actions 页面 |

---

## 6. 下一阶段待办

### 6.1 公会页（P2）

按接口契约 `docs-acl/guild-api-contract.md` 对接，建议顺序：

1. `GET /api/guild/me` —— 先跑通令牌校验
2. `GET /api/guild/announcements` + `GET /api/guild/servers` —— 纯读、无状态
3. `GET /api/guild/packs` —— 带 `sha256` 校验的下载与安装
4. `POST/GET /api/guild/presence` —— 在线状态

界面：侧边栏新增「公会」入口，三个标签页（公告 / 整合包 / 服务器）。

### 6.2 在线状态（P3）

**用 30 秒轮询，不要 WebSocket**。原因：玩家多数挂代理，WebSocket 经代理易断；轮询实现简单、可观测。隐身开关默认开启可见，需在界面提供显式开关。

### 6.3 工程清理

- 删除 `supabase/` 残留
- 摘除 Hosting 路由
- 更新 `PRIVACY.md`（公会令牌）
- 回归 `README.md` 的「与上游的差异」表格

### 6.4 对外分发前的决策

- 签名路径：Microsoft Store（一次性约 $19）优于证书（OV 初期仍拦，EV 贵）
- 更新机制：目前完全关闭，正式分发前需决定是否自建更新源

---

## 7. 环境与常用命令

```bash
# 依赖（拉取上游新提交后执行一次）
pnpm install

# 开发模式：界面热更新，Rust 改动自动重编译
pnpm app:dev

# 本地出 exe（不发布）
pnpm --filter @modrinth/app build --no-bundle
# 产物：apps/app/target/release/ACL.exe

# 类型与格式检查（与 CI 一致）
cargo check -p theseus_gui
pnpm --filter @modrinth/app-frontend exec vue-tsc --noEmit
```

**本地测试注意**：开发模式会继承终端的代理环境变量。若终端里的 `HTTPS_PROXY` 指向已关闭的端口，应用能启动但所有网络请求失败。启动前 `unset HTTPS_PROXY HTTP_PROXY`。

详细说明见 `docs-acl/local-dev.md`。

---

## 8. 接手建议

1. 先读 `docs-acl/acl-auth-contract.md` 和 `docs-acl/guild-api-contract.md`，这两份定义了启动器与官网的边界
2. 本地跑一次 `pnpm app:dev`，确认能登录成功，再动代码
3. 改 `App.vue` 前记住它是纯 JS 的 SFC
4. 任何「界面上还残留 Modrinth 内容」的问题，先按数据源查（`ads.js`、远程接口），不要凭版本号推断
5. 推送会触发 20 分钟构建，日常验证请用本地命令
