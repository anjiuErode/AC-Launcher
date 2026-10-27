# 下一位 Agent 的启动提示词

用法：新会话开始时，把本文件从「## 提示词正文」开始的内容整段发给 Agent。

---

## 提示词正文

你现在接手 **ACL（AnjiuCraftLauncher）**——瀛洲公会自用的 Minecraft 启动器。第一阶段已完成，你要做的是继续推进后续阶段，而不是重新规划。

### 一、先读这三处，不要先动代码

1. `HANDOVER.md`（仓库根目录）——第一阶段全貌：已完成模块、设计决策、代码结构、已解决问题、技术债务、下一阶段待办
2. `docs-acl/acl-auth-contract.md`——启动器与官网的登录契约
3. `docs-acl/guild-api-contract.md`——公会数据接口规格

读完这三份，你就掌握了 90% 的上下文。不要靠通读代码去猜，也不要凭经验假设架构。

### 二、项目事实（不要质疑，除非有证据）

| 项 | 值 |
| --- | --- |
| 代码仓库 | `D:\ACL启动器\code`，fork 自 `modrinth/code`（GPL-3.0） |
| 官网源码 | `D:\WB\Working\网站设计`（React + Vite + Cloudflare Pages Functions + D1） |
| 上游仓库 | 另有独立项目，**不要修改其后端接口**，改动前先与用户确认 |
| 应用标识 | `com.anjiu.acl` |
| 当前版本 | `0.1.2`，已发布 `v0.1.2-alpha` 预发布 |
| 平台 | 仅 Windows 10/11 x64 |
| 代码签名 | 无。SignPath Foundation 已拒绝，原因与后续路径见 `docs-acl/code-signing-decision.md` |
| 身份体系 | 公会账号 = 官网「功能页账号」（`studio_users`，普通成员 `role='user'`），令牌头 `x-studio-token` |
| 已废弃 | Supabase（项目、表、Cloudflare Worker 全部拆除，不要再引入） |

### 三、必须遵守的工作方式

1. **零警告**：`cargo check` 与前端构建必须 0 error、0 代码警告。不要用 `#[allow(dead_code)]` 掩盖，删掉未使用代码或让它真正被使用。
2. **根因优先**：修问题要修到根上。同类问题反复出现会被直接指出。
3. **增量推进**：一次做一块，做完汇报、等确认，不要一次性铺开多个模块。
4. **先本地验证再发布**：界面与逻辑改动用 `pnpm app:dev` 或 `pnpm --filter @modrinth/app build --no-bundle` 本地验证，**未经用户确认不要推送发版**。
5. **不要猜**：涉及"界面上为什么还显示某个东西""用户装的是哪个版本"这类问题，先取证据（进程哈希、源码引用、接口响应），不要按经验推断。已被明确批评过一次。
6. **环境故障立即上报**：遇到沙箱拦截、文件锁、杀软拦截、端口占不掉、`git push` 502 等，重试不超过两次就停下来告诉用户，不要自行反复重试消耗时间。
7. **需要用户在多个方案间选择时，用问答模块让用户点选**，不要在正文里列选项等回复。

### 四、技术约束（踩过的坑，别再踩）

| 约束 | 说明 |
| --- | --- |
| `apps/app-frontend/src/App.vue` 是纯 JS 的 `<script setup>` | 没有 `lang="ts"`，写入 TS 泛型会直接编译失败 |
| `packages/app-lib` 的 `util` 是私有模块 | 新能力必须在 `lib.rs` 重新导出，否则 `theseus::util::*` 不可见 |
| TOML 格式由 `tombi` 检查 | 多元素数组必须竖排，参照 `Cargo.toml` 里 `async-stripe` 的写法 |
| HTTP 客户端是 `LazyLock` 静态量 | 代理设置改动需重启生效，不要试图热更新 |
| 在线状态用 30 秒轮询 | **不要用 WebSocket**，玩家多挂代理，长连接易断 |
| 令牌不进 webview | 只回传 `user_id` 与 `name`，令牌存本机 `guild_session.json` |
| `i18n:coverage` 是构建前置 | 新增文案 key 需同时补 `zh-CN` 与 `en-US` |
| `sqlx` 使用离线查询数据 | 改 `settings` 表结构需重新生成 `.sqlx`，否则 CI 失败 |
| GitHub 网络不稳 | `git push` 可能 502，间隔重试；判断成功以 `git ls-remote origin refs/heads/main` 为准，不要只看退出码 |

### 五、常用命令

```bash
cd D:/ACL启动器/code

pnpm install                                   # 拉取上游新提交后执行一次
pnpm app:dev                                   # 开发模式，界面热更新
pnpm --filter @modrinth/app build --no-bundle  # 本地出 exe，不发布
cargo check -p theseus_gui                     # Rust 检查
```

**本地测试前**：开发模式继承终端代理变量，若 `HTTPS_PROXY` 指向失效端口，应用能开但网络全失败。先 `unset HTTPS_PROXY HTTP_PROXY`。

### 六、当前进度与下一步

已完成：品牌替换、单色主题、简中本地化、网络代理设置、公会账号登录（浏览器授权 + 本机回环）、远程内容清理（新闻 / 广告 / 募捐 / Hosting 入口 / intercom）。

**下一步按这个顺序做**：

1. **P2 公会页**：侧边栏新增「公会」入口，三个标签页（公告 / 整合包 / 服务器），对接
   `GET /api/guild/me` → `announcements` / `servers` → `packs` → `presence`
2. **P3 在线状态**：30 秒轮询上报 + 成员列表 + 隐身开关
3. **工程清理**：删除 `supabase/migrations/` 残留、摘除 Hosting 路由、补 `PRIVACY.md` 的公会令牌说明

**开工前先做的事**：本地跑一次 `pnpm app:dev`，确认能正常登录（点侧边栏登录按钮 → 浏览器授权 → 回到启动器显示名字）。登录不通，后面做公会页没有意义。

### 七、边界

- 不修改官网后端代码，除非用户明确要求；接口变更走契约文档协商
- 不引入第二套身份体系
- 未经确认不发版、不推送触发构建、不做破坏性 git 操作
- 涉及删除资源（数据库、Worker、域名、Release）必须先列出将被删除的对象并获得确认
