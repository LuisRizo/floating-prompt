<#
  build.ps1 - build the Rust binary and place it where the Claude Code
  plugin system expects it.

  After running this, install via the plugin system from inside Claude Code:

      /plugin marketplace add <absolute-path-to-this-repo>
      /plugin install floating-prompt@floating-prompt-marketplace

  Claude Code wires the 4 hooks (Stop, PreToolUse, Notification,
  PermissionRequest) automatically from hooks.json - no settings.json edit
  needed.

  Usage:
    .\build.ps1           # cargo build --release; copy to hooks\
    .\build.ps1 -SkipBuild   # skip cargo; copy an existing target\release exe
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$root   = $PSScriptRoot
$srcExe = Join-Path $root "target\release\floating-prompt.exe"
$dstDir = Join-Path $root "hooks"
$dstExe = Join-Path $dstDir "floating-prompt.exe"

if (-not $SkipBuild) {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if (-not $cargo) {
        throw "cargo not found on PATH. Install Rust from https://rustup.rs/ and rerun."
    }
    Write-Host "Building floating-prompt (cargo build --release)..." -ForegroundColor Cyan
    Push-Location $root
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed (exit $LASTEXITCODE)"
        }
    } finally { Pop-Location }
}

if (-not (Test-Path $srcExe)) {
    throw "Binary not found at $srcExe. Run without -SkipBuild, or build manually."
}

New-Item -ItemType Directory -Force -Path $dstDir | Out-Null
Copy-Item -Force $srcExe $dstExe

$kb = [Math]::Round((Get-Item $dstExe).Length / 1KB)
Write-Host ""
Write-Host "Built: $dstExe ($kb KB)" -ForegroundColor Green
Write-Host ""
Write-Host "Next, install the plugin from inside Claude Code:" -ForegroundColor Cyan
Write-Host ""
Write-Host "  /plugin marketplace add `"$root`""
Write-Host "  /plugin install floating-prompt@floating-prompt-marketplace"
Write-Host ""
Write-Host "To remove later:  /plugin uninstall floating-prompt@floating-prompt-marketplace"
