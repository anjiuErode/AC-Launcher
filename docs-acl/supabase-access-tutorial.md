# Supabase 访问加速方案（无服务器、零成本）

> **状态：已于 2026-10-27 全部拆除**（Worker、`guild.yingzhou1110.top`、Supabase 项目均已删除，身份改用官网功能页账号）。以下内容仅作技术参考保留。
>
> 原实施记录：2026-10-26 实施完成。
> 账号 `f4ca5be78064c067450824659e82e523`，区域 `yingzhou1110.top`（id `f70ae4b0a253a2245746381d30ae644c`），
> Worker `supabase-proxy`，自定义域 `guild.yingzhou1110.top`。
> 实测：健康检查 200（GoTrue v2.197.0），错误凭据返回 400 `invalid_credentials`，缺 apikey 返回 401。
> 启动器常量已切换为 `https://guild.yingzhou1110.top`。以下教程保留作为重建步骤参考。


## 背景

实测问题：`hehzfiafjqthetdxekxf.supabase.co` 在国内网络下 DNS 解析超时、直连失败，导致公会账号无法登录。

目标：让启动器改用自有域名访问 Supabase，避开对 `*.supabase.co` 的干扰。

## 方案对比（已核实 2026-10 官方定价）

| 方案 | 成本 | 需要服务器 | 效果 |
| --- | --- | --- | --- |
| Supabase 官方自定义域名 | Pro $25/月 + 域名附加 $10/月 ≈ ¥250/月 | 否 | 零延迟，官方维护 |
| **Cloudflare Worker 反代**（推荐） | ¥0 | **否**（边缘函数） | 增加 20–50ms |
| 自建 VPS 反代 | 服务器成本 | 是 | 需自行维护 |

推荐第二条：免费、不碰服务器、随时可撤。

---

## 一、把域名接入 Cloudflare（免费）

前提：`yingzhou1110.top` 的 DNS 可改（域名在哪个注册商都行，只需能改 NS）。

1. 登录 https://dash.cloudflare.com → `Add a site`
2. 输入 `yingzhou1110.top`，计划选 **Free**
3. Cloudflare 会给出两个 NS 地址，形如 `xxx.ns.cloudflare.com`
4. 到域名注册商后台，把域名的 DNS 服务器改成这两个
5. 回 Cloudflare 点 `Check nameservers`，等待生效（通常 10 分钟–24 小时）

> 注意：改 NS 后，原注册商处的解析记录要**在 Cloudflare 里重新建**（首页、成员页等原有 A/CNAME 记录）。迁移前先导出原记录。

---

## 二、创建 Worker 反代

1. Cloudflare 控制台 → `Workers & Pages` → `Create` → `Worker`
2. 命名 `supabase-proxy` → `Deploy`
3. 点 `Edit code`，把默认代码整段替换为：

```js
const UPSTREAM = "https://hehzfiafjqthetdxekxf.supabase.co";

export default {
  async fetch(request) {
    const incoming = new URL(request.url);
    const target = UPSTREAM + incoming.pathname + incoming.search;

    const headers = new Headers(request.headers);
    headers.set("Host", new URL(UPSTREAM).host);
    // 避免把 Worker 自己的信息带给上游
    headers.delete("cf-connecting-ip");
    headers.delete("cf-ray");

    const hasBody = !["GET", "HEAD"].includes(request.method);

    return fetch(target, {
      method: request.method,
      headers,
      body: hasBody ? request.body : undefined,
      redirect: "manual",
    });
  },
};
```

4. `Deploy`

---

## 三、绑定自有域名

1. 进入该 Worker → `Settings` → `Domains & Routes` → `Add` → `Custom domain`
2. 填 `guild.yingzhou1110.top`（子域名随意，建议 `guild` 或 `auth`）
3. Cloudflare 会自动创建 DNS 记录并签发证书，状态变 `Active` 即生效

> 这一步必须用 Custom domain，不要用 `*.workers.dev` 的免费子域——那个域名在国内常被干扰。

---

## 四、验证

在任意终端：

```bash
curl -s -o /dev/null -w "%{http_code}\n" \
  "https://guild.yingzhou1110.top/auth/v1/health" \
  -H "apikey: <已废弃的 anon key，项目已于 2026-10-27 删除>"
```

预期 `200`（返回 `{"date":"...","description":"GoTrue is a user registration and authentication API"}`）。

若返回 `401`，说明请求到了 Kong 但缺 apikey；若 `000`/超时，说明域名或证书还没生效。

---

## 五、改启动器里的地址

文件：`apps/app/src/guild.rs`

```rust
const SUPABASE_URL: &str = "https://hehzfiafjqthetdxekxf.supabase.co";
```

改为：

```rust
const SUPABASE_URL: &str = "https://guild.yingzhou1110.top";
```

只改这一行，其余代码无需调整（所有请求路径都是相对拼接）。

---

## 六、已知限制

| 限制 | 说明 | 处理 |
| --- | --- | --- |
| Realtime / WebSocket | 经 Worker 转发 WebSocket 需要额外处理 `Upgrade` 握手 | 在线状态改用 30 秒轮询，不用 WebSocket |
| 延迟 | 多一跳，约增加 20–50ms | 登录与列表加载无感 |
| 国内可达性 | Cloudflare 自有域名比 `*.supabase.co` 稳定，但**仍需实测**，不同地区运营商差异大 | 上线前找 2–3 位不同地区的成员测试 |
| 免费额度 | Workers 免费 10 万请求/天 | 公会规模远低于此 |
| 域名迁移风险 | 改 NS 会影响现有站点解析 | 先导出原记录，在 Cloudflare 重建后再切 |

---

## 七、备选：如果 Cloudflare 也不稳定

1. 换用其他边缘平台（Deno Deploy、Vercel）反代，原理相同
2. 若成员普遍能开代理，则保持现状 + 设置页的代理功能（已实现）
3. 兜底方案：把公会功能改为「离线可用」——公告、整合包列表打包进启动器内置 JSON，联网操作全部可选
