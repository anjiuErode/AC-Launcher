# 本地测试流程

目的：日常改动不必发布到 GitHub 才能看到效果。

## 三种循环，按改动类型选

| 改动范围 | 命令 | 首次耗时 | 之后每次 |
| --- | --- | --- | --- |
| 界面（Vue / CSS / 文案） | `pnpm app:dev` | 10–15 分钟（首次编译） | **秒级**，改完自动热更新 |
| Rust（`app-lib`、`app/src`） | `pnpm app:dev` | 同上（共用缓存） | 1–2 分钟，自动重编译并重启窗口 |
| 需要接近正式版的程序 | `pnpm --filter @modrinth/app build --no-bundle` | 约 10 分钟 | 每次全量 |

`--no-bundle` 只产 exe、不打包安装器，产物在 `apps/app/target/release/ACL.exe`，可直接双击运行，不需要发布。

## 开发模式

```bash
cd D:/ACL启动器/code
pnpm app:dev
```

启动过程：

1. 检查 i18n 覆盖率（`i18n:coverage`）
2. 启动前端 dev server（`http://localhost:1420`）
3. 编译 Rust（debug profile）
4. 弹出应用窗口

窗口里改界面代码会立即生效，不用重启；改 Rust 代码会自动重新编译并重启窗口。

关闭：在终端按 `Ctrl+C`。

## 登录流程在开发模式下同样可用

授权页在线上（`yingzhou1110.top`），开发模式与安装版行为一致：

1. 点侧边栏登录按钮 → 浏览器打开授权页
2. 授权完成后回到 `127.0.0.1:<随机端口>`
3. 启动器保存会话并显示名字

开发版与安装版共用同一个应用标识 `com.anjiu.acl`，因此：

- 配置目录相同（`%APPDATA%\com.anjiu.acl`）
- 登录状态互通
- 代理设置互通

如果要测「首次登录」的干净状态，删掉 `%APPDATA%\com.anjiu.acl\guild_session.json` 即可。

## 首次准备

如果报 `node_modules may be out of sync with your lockfile`：

```bash
pnpm install
```

只需要在拉取上游新提交后做一次。

## 常见问题

| 现象 | 原因 | 处理 |
| --- | --- | --- |
| 应用能开但登录/下载全部失败 | 继承了失效的 `HTTPS_PROXY` | 启动前执行 `unset HTTPS_PROXY HTTP_PROXY`，或用设置里的「网络 → 直连」 |
| 端口 1420 被占用 | 上一次 dev 没退干净 | 关掉残留终端，或结束 `node.exe` 进程 |
| 首次编译很慢 | Rust debug 全量编译 | 正常，10–15 分钟，之后靠增量 |
| 改 Rust 后要等很久 | 触碰了 `app-lib` 的底层模块 | 正常，1–2 分钟；只有改 `app/src` 才快 |
| 界面样式与安装版不同 | dev 使用 `localhost:1420`，资源路径不同 | 图标、字体一般一致；布局问题请以正式包为准 |

## 什么时候仍然要发布

- 需要验证安装器行为（NSIS 安装、卸载、快捷方式）
- 需要验证「干净机器」上的首次启动
- 对外分发

其余情况全部用本地循环，不必推送。
