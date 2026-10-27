# 公会数据接口 · 规格说明

面向：负责官网后端的 Agent
版本：v1
依赖：授权页契约见 [acl-auth-contract.md](acl-auth-contract.md)

## 一、职责划分

| 范围 | 归属 |
| --- | --- |
| `/acl/auth` 授权页 | 官网 Agent |
| `/api/guild/*` 数据接口 + D1 表结构 | 官网 Agent |
| 启动器端调用、界面渲染 | 本项目 |

启动器只消费 HTTP 接口，不直接访问 D1。接口稳定后双方可独立迭代。

## 二、鉴权

所有接口统一校验请求头：

```
x-studio-token: <令牌>
```

- 令牌无效 / 过期 → `401`，响应体 `{ "error": "登录已过期" }`
- 启动器收到 `401` 会清除本地会话并提示重新登录
- 角色：普通成员 `role='user'`，管理员 `role='admin'`；管理员可额外调用写入接口

## 三、响应约定

沿用官网现有风格：

- 成功：直接返回数据对象，HTTP `200`
- 失败：`{ "error": "中文说明" }` 加对应状态码
- 时间统一 ISO 8601 字符串（UTC，带 `Z`）
- 响应头带 `Cache-Control: no-store`（除公告列表可用 `max-age=60`）

## 四、接口清单

### 1. 当前用户

```
GET /api/guild/me
```

```json
{ "user_id": 12, "username": "anjiu", "role": "user", "joined_at": "2026-10-01T00:00:00Z" }
```

用途：启动器确认令牌有效，并展示成员身份。

---

### 2. 公告列表

```
GET /api/guild/announcements?limit=20
```

```json
{
  "items": [
    {
      "id": 3,
      "title": "本周服务器维护",
      "body": "周六 22:00 停机约 1 小时",
      "pinned": true,
      "published_at": "2026-10-25T12:00:00Z",
      "author": "anjiu"
    }
  ]
}
```

规则：置顶在前，其余按 `published_at` 倒序；`limit` 默认 20，上限 50。

---

### 3. 整合包列表

```
GET /api/guild/packs
```

```json
{
  "items": [
    {
      "id": 7,
      "name": "瀛洲整合包 · 一周年",
      "mc_version": "1.21.1",
      "loader": "fabric",
      "description": "公会定制，含光影与优化",
      "url": "https://.../yingzhou-1st.zip",
      "sha256": "9F2A...",
      "size_bytes": 48234496
    }
  ]
}
```

`url` 必须是可直接下载的直链（R2 或 Releases）。启动器安装前会校验 `sha256`，不匹配则中止。

---

### 4. 服务器列表

```
GET /api/guild/servers
```

```json
{
  "items": [
    {
      "id": 2,
      "name": "主世界",
      "host": "play.yingzhou1110.top",
      "port": 25565,
      "description": "生存，正版验证",
      "whitelist_required": true
    }
  ]
}
```

---

### 5. 在线状态上报

```
POST /api/guild/presence
```

请求体：

```json
{ "instance_name": "1.21.1-Fabric", "mc_version": "1.21.1", "loader": "fabric", "visible": true }
```

```json
{ "ok": true }
```

规则：

- 启动器每 30 秒调用一次（**不使用 WebSocket**，便于穿透代理）
- 服务端以 `last_seen = now()` UPSERT，同一用户仅保留一行
- `visible=false` 表示隐身，仍然记录但不对外展示
- 请求体缺失字段时按空处理，不返回 400

---

### 6. 在线成员

```
GET /api/guild/presence
```

```json
{
  "items": [
    { "username": "anjiu", "instance_name": "1.21.1-Fabric", "mc_version": "1.21.1", "last_seen": "2026-10-27T04:39:00Z" }
  ]
}
```

规则：只返回 `visible=1` 且 `last_seen > now() - 3 分钟` 的记录；`username` 不含敏感字段。

## 五、D1 表结构（建议）

```sql
CREATE TABLE IF NOT EXISTS guild_announcements (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0,
  author_id INTEGER,
  published_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS guild_packs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  mc_version TEXT NOT NULL,
  loader TEXT,
  description TEXT,
  url TEXT NOT NULL,
  sha256 TEXT,
  size_bytes INTEGER
);

CREATE TABLE IF NOT EXISTS guild_servers (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER NOT NULL DEFAULT 25565,
  description TEXT,
  whitelist_required INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS guild_presence (
  user_id INTEGER PRIMARY KEY,
  instance_name TEXT,
  mc_version TEXT,
  loader TEXT,
  visible INTEGER NOT NULL DEFAULT 1,
  last_seen TEXT NOT NULL
);
```

写入接口（新增/编辑公告、整合包、服务器）本期可不做，先由后台或 SQL 直接维护；启动器只读。

## 六、边界与注意事项

| 事项 | 要求 |
| --- | --- |
| 分页 | 本期数据量小，`limit` 即可，不需要游标 |
| 频率限制 | `presence` 每用户 30 秒一次，服务端可对 >1 次/10 秒 返回 `429` |
| 数据校验 | `host` 只允许域名或 IPv4，禁止携带端口；`port` 范围 1–65535 |
| 敏感信息 | 任何接口都不得返回密码哈希、会话令牌、邮箱 |
| 失败降级 | 接口异常时启动器界面显示「公会数据暂时不可用」，不阻塞启动与游戏 |

## 七、联调顺序建议

1. 先实现 `GET /api/guild/me`，启动器用它验证令牌有效性
2. 再实现公告与服务器列表（纯读，无状态）
3. 整合包列表（含 `sha256` 校验）
4. 最后 `presence` 上报与查询

每完成一个，告诉我接口已上线，我立刻对接并回报实测结果。
