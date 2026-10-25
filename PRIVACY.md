# Privacy Policy — ACL (AnjiuCraftLauncher)

Last updated: 2026-10-26

ACL is a desktop Minecraft launcher. This document describes exactly what
network traffic the application generates.

## No telemetry

This build does **not** include analytics, crash reporting, advertising or
usage tracking. The analytics helpers in
`apps/app-frontend/src/helpers/analytics.ts` are compiled to no-ops, so no
event data is sent anywhere.

## Data sent, and only when you trigger it

| Destination | When | What is sent |
| --- | --- | --- |
| `api.modrinth.com` | You sign in, browse, or download a project | Project IDs you request, OAuth token after you approve sign-in |
| `launcher-meta.modrinth.com` | You start the launcher or create an instance | The game / loader version you ask for |
| Microsoft / Mojang services (`*.live.com`, `*.minecraft.net`) | You sign in with a Microsoft account | Standard Minecraft authentication payloads |
| `textures.minecraft.net` | An instance or skin is displayed | The skin texture identifier |

No data is sold, shared with third parties for advertising, or transmitted to
the project maintainers. There is no server operated by this project: the
launcher is a client only.

## Local data

Instances, settings, logs and the GPU preference file (`gpu.json`) are stored
locally in the application data directory and are never uploaded by the
launcher. Log files stay on disk unless you choose to share them.

## Removing your data

Delete the application data directory:

- Windows: `%APPDATA%\com.anjiu.acl`
- macOS: `~/Library/Application Support/com.anjiu.acl`

Uninstalling does not delete this directory automatically.

## Contact

Open an issue at https://github.com/anjiuErode/code/issues

## Code signing

Free code signing provided by [SignPath.io](https://signpath.io), certificate
by [SignPath Foundation](https://signpath.org).
