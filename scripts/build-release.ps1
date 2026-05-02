<#
.SYNOPSIS
    Build wraith-browser (open-source CLI) release archives for all targets.
.DESCRIPTION
    Thin wrapper that delegates to scripts/build-release.sh via Git Bash.
    All real work lives in the bash script — this just locates bash.exe,
    forwards args, and surfaces exit codes.

    Targets: linux x86_64 + linux aarch64 (Docker), windows x86_64 (native),
    macOS x86_64 + aarch64 (skipped here — emits ssh imac commands).

    See scripts/release-targets.md for design + static-linking decisions.
.PARAMETER Version
    Required. Version tag, e.g. v0.1.0. Must match Cargo.toml workspace.package.version.
.PARAMETER Force
    Wipe a non-empty dist/$Version/ before building.
.PARAMETER Publish
    After building, run gh release create against suhteevah/wraith-browser.
    Requires CHANGELOG_FRAGMENT.md at repo root.
.PARAMETER SkipWindows
    Skip the native Windows build (debug only).
.PARAMETER SkipLinux
    Skip the Docker linux builds (debug only).
.EXAMPLE
    .\scripts\build-release.ps1 -Version v0.1.0
.EXAMPLE
    .\scripts\build-release.ps1 -Version v0.1.0 -Force -Publish
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Version,

    [switch]$Force,
    [switch]$Publish,
    [switch]$SkipWindows,
    [switch]$SkipLinux
)

$ErrorActionPreference = "Stop"

# Locate bash.exe — prefer Git for Windows
$bashCandidates = @(
    "C:\Program Files\Git\bin\bash.exe",
    "C:\Program Files\Git\usr\bin\bash.exe",
    "C:\Program Files (x86)\Git\bin\bash.exe"
)
$bash = $null
foreach ($c in $bashCandidates) {
    if (Test-Path $c) { $bash = $c; break }
}
if (-not $bash) {
    $cmd = Get-Command bash.exe -ErrorAction SilentlyContinue
    if ($cmd) { $bash = $cmd.Source }
}
if (-not $bash) {
    throw "bash.exe not found. Install Git for Windows."
}

$repoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$script   = Join-Path $repoRoot "scripts\build-release.sh"
if (-not (Test-Path $script)) {
    throw "Cannot find $script"
}

# Build arg list for the bash script
$bashArgs = @($script, $Version)
if ($Force)        { $bashArgs += "--force" }
if ($Publish)      { $bashArgs += "--publish" }
if ($SkipWindows)  { $bashArgs += "--skip-windows" }
if ($SkipLinux)    { $bashArgs += "--skip-linux" }

Write-Host ""
Write-Host "============================================"
Write-Host "  wraith-browser release build $Version"
Write-Host "  delegating to: $bash"
Write-Host "============================================"
Write-Host ""

# -lc so Git Bash sources MSYS env; Push-Location keeps cwd at repo root for the script's own resolution
Push-Location $repoRoot
try {
    & $bash -lc ((@("bash") + $bashArgs | ForEach-Object { "'$_'" }) -join ' ')
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}

if ($code -ne 0) {
    Write-Host ""
    Write-Host "build-release.sh exited with code $code" -ForegroundColor Red
    exit $code
}
Write-Host ""
Write-Host "build-release.sh completed successfully." -ForegroundColor Green
