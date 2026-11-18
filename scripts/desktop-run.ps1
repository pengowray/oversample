<#
Build and run the Oversample desktop app (Tauri) on Windows.

Usage:
  scripts\desktop-run.ps1 [-Release | -Dev | -NoBuild] [app args...]
  scripts\desktop-run.cmd  (same options; avoids execution-policy errors)

  (no option)  debug build without bundling, then run it (recommended)
  -Release     release build without bundling, then run it
  -Dev         cargo tauri dev
  -NoBuild     run the last built binary (debug, or release with -Release)
  -Help        show this text
  Any other arguments are passed to the app.

Which frontend build is used (from src-tauri\tauri.conf.json):
  - Builds (default and -Release) run beforeBuildCommand,
    "bash scripts/trunk-release.sh", a release WASM build, so bash (Git Bash)
    must be on PATH. Only the native side is debug in the default mode.
  - -Dev runs beforeDevCommand, "trunk serve --port 21259", a debug WASM
    build, which is very slow. Use the default mode to try things.

Google sign-in: src-tauri\build.rs reads the desktop client id and secret
(OVERSAMPLE_GOOGLE_DESKTOP_CLIENT_ID / _SECRET) from the repo's git-ignored
.env at build time. Nothing to export by hand.

Needs: cargo install tauri-cli --version "^2"
#>
[CmdletBinding(PositionalBinding = $false)]
param(
    [switch]$Release,
    [switch]$Dev,
    [switch]$NoBuild,
    [switch]$Help,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$AppArgs
)

$ErrorActionPreference = 'Stop'

if ($Help) {
    (Get-Content $PSCommandPath -Raw) -match '(?s)<#\r?\n(.*?)#>' | Out-Null
    Write-Output $Matches[1]
    exit 0
}

Set-Location (Join-Path $PSScriptRoot '..')
if ($null -eq $AppArgs) { $AppArgs = @() }

function Assert-Tauri {
    try { & cargo tauri --version *> $null; $ok = ($LASTEXITCODE -eq 0) } catch { $ok = $false }
    if (-not $ok) {
        Write-Error 'cargo tauri is not installed. Install it with: cargo install tauri-cli --version "^2"' -ErrorAction Continue
        exit 1
    }
}

if ($Dev) {
    Assert-Tauri
    if ($AppArgs.Count -gt 0) { & cargo tauri dev -- -- @AppArgs } else { & cargo tauri dev }
    exit $LASTEXITCODE
}

$mode = if ($Release) { 'release' } else { 'debug' }
$bin = Join-Path (Get-Location) "target\$mode\oversample-desktop.exe"

if (-not $NoBuild) {
    Assert-Tauri
    if ($Release) {
        & cargo tauri build --no-bundle
    } else {
        & cargo tauri build --debug --no-bundle
    }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

if (-not (Test-Path $bin)) {
    Write-Error "No built app at $bin. Run without -NoBuild first." -ErrorAction Continue
    exit 1
}
& $bin @AppArgs
exit $LASTEXITCODE
