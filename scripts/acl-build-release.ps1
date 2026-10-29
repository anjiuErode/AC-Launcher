# Release build: NSIS installer + updater signature.
#
# Tier policy: verify with acl-build-fast.ps1 first, run this only after the build is accepted.
# Requirements (do not skip):
#   - updater feature enabled       (tauri-release.conf.json)
#   - NSIS installer + .sig produced
#   - real signing key used, and the .sig is checked for existence afterwards
#
# Usage:
#   $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content <key path> -Raw
#   .\scripts\acl-build-release.ps1
#
#   .\scripts\acl-build-release.ps1 -KeyPath D:\ACL启动器\keys\acl-updater.key
#
# Output: target/release/bundle/nsis/ACL_<version>_x64-setup.exe and .sig

param(
    [string]$KeyPath,
    [switch]$SkipFrontend,
    # 明确承认"这不是可发布的正式产物"才允许覆盖 LTO
    [switch]$AllowNonFinalProfile
)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$appDir = Join-Path $root 'apps/app'
$cli = Join-Path $root 'node_modules/@tauri-apps/cli/tauri.js'

# Resolve node explicitly: the binary is not always on PATH (e.g. tool shells).
$nodeExe = $env:ACL_NODE
if (-not $nodeExe) {
    $nodeCmd = Get-Command node -ErrorAction SilentlyContinue
    if ($nodeCmd) { $nodeExe = $nodeCmd.Source }
}
if (-not $nodeExe -or -not (Test-Path $nodeExe)) {
    throw 'node not found; install Node.js or set ACL_NODE to the full path of node.exe'
}
$releaseConfig = Join-Path $appDir 'tauri-release.conf.json'

if ($KeyPath) {
    if (-not (Test-Path $KeyPath)) { throw "signing key not found: $KeyPath" }
    $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Path $KeyPath -Raw
}

if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    throw 'TAURI_SIGNING_PRIVATE_KEY is not set; refusing to build an unsigned release'
}

# Release artifacts keep the real profile (fat LTO). Override only when the machine cannot
# handle it, and say so out loud - such a build is NOT equivalent to a CI release.
if (-not $env:CARGO_PROFILE_RELEASE_LTO) {
    Write-Host '[acl] using the repository release profile (fat LTO)' -ForegroundColor Cyan
} elseif (-not $AllowNonFinalProfile) {
    # 默认硬失败：避免把 LTO=off 的本地产物当成正式包发出去
    throw "CARGO_PROFILE_RELEASE_LTO=$env:CARGO_PROFILE_RELEASE_LTO is set. Refusing to build a release package with a non-final profile. Re-run with -AllowNonFinalProfile if this build must not be published."
} else {
    Write-Host "[acl] WARNING: non-final profile (LTO=$env:CARGO_PROFILE_RELEASE_LTO) - this artifact must NOT be published" -ForegroundColor Red
}

$configArgs = @('--config', $releaseConfig)

if ($SkipFrontend) {
    $skipFile = Join-Path ([System.IO.Path]::GetTempPath()) 'acl-skip-before-build.json'
    '{"build":{"beforeBuildCommand":""}}' | Set-Content -Path $skipFile -Encoding utf8
    $configArgs += @('--config', $skipFile)
}

Write-Host '[acl] release build (NSIS + signature)' -ForegroundColor Cyan
Push-Location $appDir
# tauri writes progress to stderr; with ErrorActionPreference=Stop that would be
# treated as a terminating NativeCommandError, so relax it around the native call.
$previousEap = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    & $nodeExe $cli build @configArgs --bundles nsis
    $buildExit = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousEap
    Pop-Location
}
if ($buildExit -ne 0) { throw "tauri build failed with exit code $buildExit" }

$nsisDir = Join-Path $root 'target/release/bundle/nsis'
$installer = Get-ChildItem -Path $nsisDir -Filter '*_x64-setup.exe' | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $installer) { throw "no installer produced in $nsisDir" }

$sig = "$($installer.FullName).sig"
if (-not (Test-Path $sig)) { throw "signature missing for $($installer.Name) - refusing to call this a release" }

$sizeMb = [math]::Round($installer.Length / 1MB, 1)
Write-Host "[acl] installer: $($installer.FullName) ($sizeMb MB)" -ForegroundColor Green
Write-Host "[acl] signature: $sig" -ForegroundColor Green
Write-Host '[acl] next: upload installer to the updater release, then update latest.json' -ForegroundColor Cyan
