# NOTICE

AnjiuCraftLauncher (ACL)

This project is a modified fork of **Theseus / Modrinth App**, part of the
[`modrinth/code`](https://github.com/modrinth/code) monorepo, which is licensed
under the **GNU General Public License v3.0**.

- Upstream: https://github.com/modrinth/code
- This fork: https://github.com/anjiuErode/code
- Original copyright: © Rinth, Inc. and Modrinth contributors

The full source code of this fork, including all modifications listed below,
is published in the repository above, as required by the GPL-3.0.

## Relationship to upstream

This repository is a GitHub fork. The upstream commit history is preserved.
We are not affiliated with, endorsed by, or supported by Rinth, Inc.
"Modrinth" is a trademark of Rinth, Inc.; all Modrinth branding assets that
were shipped in the upstream repository have been removed from this fork in
accordance with `COPYING.md`.

## List of modifications

Relative to upstream `modrinth/code` at the point of forking:

1. **Branding and naming**
   - Product name changed to `ACL`; identifier changed to `com.anjiu.acl`.
   - Application icons replaced (all sizes in `apps/app/icons/`, including
     `icon.ico`, `icon.icns`, `favicon.ico` and the Windows Store logo set).
   - Splash screen wordmark replaced with the project's own emblem.
   - Upstream branding assets removed: `.idea/icon.svg`, `.github/assets/`.
   - User-facing strings "Modrinth App" replaced with "ACL".

2. **Theme**
   - Light and dark palettes changed to a monochrome scheme (white background
     with black text, black background with white text, high contrast).
   - Brand accent colour changed from green to monochrome.
   - Splash screen tint and loading bar gradients changed to neutral greys.

3. **Localisation**
   - Missing `zh-CN` entries filled in for the launcher UI; the launcher now
     defaults to Simplified Chinese when the system locale is Chinese.

4. **New features**
   - Settings → System → Performance: a hardware-acceleration toggle that
     controls the WebView2 GPU flags (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`).
     Stored in `gpu.json` inside the application config directory.

5. **Update mechanism**
   - The Tauri updater feature and the upstream update endpoint
     (`launcher-files.modrinth.com`) are **disabled**. This build does not
     update itself from upstream releases.

## Data transmission

This build does not include telemetry. The analytics helpers in
`apps/app-frontend/src/helpers/analytics.ts` are compiled to no-ops.

Network access is limited to what the user explicitly triggers:

- `api.modrinth.com` — account sign-in, project and version metadata, downloads
- `launcher-meta.modrinth.com` — Minecraft launcher metadata
- Mojang / Microsoft services — Minecraft authentication and skin data

See the "Code signing policy" section in `README.md` for the signing statement.
