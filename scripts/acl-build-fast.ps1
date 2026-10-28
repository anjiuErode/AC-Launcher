# Fast verification build: produces a runnable EXE, skips NSIS bundling and signing.
#
# Tier policy (see docs): dev = `tauri dev`; verify = this script; release = acl-build-release.ps1
# Never hand an LTO=off artifact to users as a release.
#
# Usage:
#   .\scripts\acl-build-fast.ps1                 # reuse cached deps, rebuild frontend
#   .\scripts\acl-build-fast.ps1 -SkipFrontend   # Rust-only change: reuse existing dist
#   .\scripts\acl-build-fast.ps1 -Incremental    # enable incremental + high codegen parallelism
#
# Output: target/release/ACL.exe

param(
    [switch]$SkipFrontend,
    [switch]$Incremental
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

# LTO off: fat LTO needs more memory than most dev machines can spare.
$env:CARGO_PROFILE_RELEASE_LTO = 'off'
$env:CARGO_PROFILE_RELEASE_PANIC = 'abort'

if ($Incremental) {
    # Trades binary quality for iteration speed; first run after enabling recompiles everything.
    $env:CARGO_PROFILE_RELEASE_INCREMENTAL = 'true'
    $env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = '256'
}

$configArgs = @('--config', $releaseConfig)

if ($SkipFrontend) {
    # tauri.conf.json runs the frontend build on every invocation; skip it when only Rust changed.
    $skipFile = Join-Path ([System.IO.Path]::GetTempPath()) 'acl-skip-before-build.json'
    '{"build":{"beforeBuildCommand":""}}' | Set-Content -Path $skipFile -Encoding utf8
    $configArgs += @('--config', $skipFile)
}

Write-Host "[acl] fast build (no bundle)" -ForegroundColor Cyan
Push-Location $appDir
# tauri writes progress to stderr; with ErrorActionPreference=Stop that would be
# treated as a terminating NativeCommandError, so relax it around the native call.
$previousEap = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    & $nodeExe $cli build @configArgs --no-bundle
    $buildExit = $LASTEXITCODE
} finally {
    $ErrorActionPreference = $previousEap
    Pop-Location
}
if ($buildExit -ne 0) { throw "tauri build failed with exit code $buildExit" }

$exe = Join-Path $root 'target/release/ACL.exe'
if (-not (Test-Path $exe)) { throw "expected $exe to exist" }

$sizeMb = [math]::Round((Get-Item $exe).Length / 1MB, 1)
Write-Host "[acl] done: $exe ($sizeMb MB)" -ForegroundColor Green
Write-Host "[acl] verification build only - do not ship this artifact" -ForegroundColor Yellow
