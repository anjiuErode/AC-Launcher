# ACL — AnjiuCraftLauncher

瀛洲公会（YingZhouAssociation）Minecraft 启动器。基于 [Modrinth Theseus](https://github.com/modrinth/code) 的 GPL-3.0 开源分支。

- 下载：[Releases](https://github.com/anjiuErode/code/releases)
- 本仓库是 GitHub fork，保留上游提交历史；所有修改见 [NOTICE.md](NOTICE.md)
- 与 Rinth, Inc. 无隶属关系，未获其背书

## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- Committers and reviewers: anjiuErode
- Approvers: anjiuErode
- This program does not include telemetry, and will not transfer any
  information to other networked systems unless specifically requested by the
  user or the person installing or operating it.

## 与上游的差异

| 类别 | 改动 |
| --- | --- |
| 品牌 | 名称改为 ACL，标识符 `com.anjiu.acl`，全套图标、启动画面与界面文案替换 |
| 主题 | 浅色为白底黑字、深色为黑底白字，强调色改为单色 |
| 本地化 | 补齐简体中文，系统语言为中文时默认使用中文 |
| 功能 | 设置 → 系统 → 性能：硬件加速开关（WebView2 GPU 参数） |
| 更新 | 已关闭上游自动更新，不会从 Modrinth 官方源更新 |
| 合规 | 已移除上游品牌资产（`.idea/icon.svg`、`.github/assets/`） |

## Development

This repository contains several projects. For detailed development information, please refer to their respective guides:

| Project       | Directory              | Guide                                                     |
| ------------- | ---------------------- | --------------------------------------------------------- |
| Website       | `apps/frontend`        | [Guide](https://docs.modrinth.com/contributing/knossos/)  |
| App           | `apps/app`             | [Guide](https://docs.modrinth.com/contributing/theseus/)  |
| API           | `apps/labrinth`        | [Guide](https://docs.modrinth.com/contributing/labrinth/) |
| Documentation | `apps/docs`            | [Guide](https://docs.modrinth.com/contributing/docs/)     |

Shared libraries live in `packages/`.

## Contributing

We welcome contributions! Before submitting any contributions, please read our [contributing guidelines](https://docs.modrinth.com/contributing/getting-started/).

If you plan to fork this repository for your own purposes, please review our [copying guidelines](COPYING.md).

## Security

If you discover a security vulnerability within our codebase, please follow our [responsible disclosure guidelines](https://modrinth.com/legal/security).

## Support

If you need help with the Modrinth web interface or app, please visit our [support page](https://support.modrinth.com). For general inquiries, you can also join our [Discord server](https://discord.modrinth.com).

## License

All packages in this repository are licensed under their respective licenses. See the [copying guidelines](COPYING.md) for details, including restrictions on Modrinth branding.
