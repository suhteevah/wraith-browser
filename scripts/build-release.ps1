#Requires -Version 5.1
<#
.SYNOPSIS
    Build wraith-browser release binaries.
.DESCRIPTION
    Builds release binaries for Windows (native) and optionally Linux (via cross).
    Run from the repo root: .\scripts\build-release.ps1
.NOTES
    Prerequisites:
      - Rust toolchain (rustup + cargo)
      - cargo install cross  (optional, for Linux; requires Docker)
    macOS cross-compilation from Windows is not supported.
#>

$ErrorActionPreference = "Stop"

Push-Location (Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path))
try {

# ---------- version ----------
$Version = "0.1.0"
try {
    $meta = cargo metadata --format-version=1 --no-deps 2>$null | ConvertFrom-Json
    if ($meta.packages.Count -gt 0) {
        $Version = $meta.packages[0].version
    }
} catch {
    Write-Host "Could not read version from Cargo.toml, using default: $Version"
}

$Dist = "dist"
if (Test-Path $Dist) { Remove-Item -Recurse -Force $Dist }
New-Item -ItemType Directory -Path $Dist | Out-Null

$Built = @()
$Failed = @()

Write-Host ""
Write-Host "========================================"
Write-Host "  wraith-browser release build v$Version"
Write-Host "========================================"
Write-Host ""

# ==========================================================
#  1. Windows x86_64 -- native build
# ==========================================================
Write-Host ">>> Building for Windows x86_64 (native)..."
$env:RUSTC_WRAPPER = ""
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$WinBin = Join-Path "target" "release" | Join-Path -ChildPath "wraith-browser.exe"
$OutName = "wraith-browser-$Version-windows-x86_64.exe"
if (Test-Path $WinBin) {
    Copy-Item $WinBin (Join-Path $Dist $OutName)
    $Built += $OutName
    Write-Host "    OK: $OutName"
} else {
    $Failed += "windows-x86_64"
    Write-Host "    FAILED: Binary not found at $WinBin"
}
Write-Host ""

# ==========================================================
#  2. Linux x86_64 -- cross-compile via cross
# ==========================================================
$crossCmd = Get-Command cross -ErrorAction SilentlyContinue
if ($crossCmd) {
    Write-Host ">>> Building for Linux x86_64 (via cross)..."
    $env:RUSTC_WRAPPER = ""
    cross build --release --target x86_64-unknown-linux-gnu
    if ($LASTEXITCODE -eq 0) {
        $LinuxBin = Join-Path "target" "x86_64-unknown-linux-gnu" | Join-Path -ChildPath "release" | Join-Path -ChildPath "wraith-browser"
        $OutName = "wraith-browser-$Version-linux-x86_64"
        if (Test-Path $LinuxBin) {
            Copy-Item $LinuxBin (Join-Path $Dist $OutName)
            $Built += $OutName
            Write-Host "    OK: $OutName"
        } else {
            $Failed += "linux-x86_64"
            Write-Host "    FAILED: Binary not found at $LinuxBin"
        }
    } else {
        $Failed += "linux-x86_64"
        Write-Host "    FAILED: cross build returned an error"
    }
} else {
    Write-Host ">>> SKIPPED: Linux x86_64 -- cross not installed."
    Write-Host "   Install with: cargo install cross"
    Write-Host "   Requires Docker Desktop running."
    $Failed += "linux-x86_64 (skipped)"
}
Write-Host ""

# ==========================================================
#  3. macOS -- NOT supported from Windows
# ==========================================================
Write-Host ">>> SKIPPED: macOS -- cross-compilation from Windows is not supported."
Write-Host "   Build on a Mac with: cargo build --release"
Write-Host ""

# ==========================================================
#  4. Generate SHA256 checksums
# ==========================================================
Write-Host ">>> Generating checksums..."
$ChecksumFile = Join-Path $Dist "SHA256SUMS.txt"
$checksumLines = @()

Get-ChildItem (Join-Path $Dist "wraith-browser-*") -ErrorAction SilentlyContinue | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    $checksumLines += "$hash  $($_.Name)"
}

if ($checksumLines.Count -gt 0) {
    $checksumLines | Set-Content -Path $ChecksumFile -Encoding UTF8
    Write-Host "    Wrote $ChecksumFile"
} else {
    Write-Host "    No binaries to checksum."
}
Write-Host ""

# ==========================================================
#  Summary
# ==========================================================
Write-Host "========================================"
Write-Host "  Build Summary"
Write-Host "========================================"
Write-Host ""
Write-Host "  Version:  $Version"
Write-Host ('  Output:   ' + $Dist + '\')
Write-Host ""

if ($Built.Count -gt 0) {
    Write-Host "  Built successfully:"
    foreach ($b in $Built) {
        $filePath = Join-Path $Dist $b
        $size = (Get-Item $filePath).Length
        $sizeMB = [math]::Round($size / 1048576, 1)
        Write-Host ('    - ' + $b + '  (' + $sizeMB + ' MB)')
    }
} else {
    Write-Host "  No binaries were built successfully."
}
Write-Host ""

if ($Failed.Count -gt 0) {
    Write-Host "  Failed / Skipped:"
    foreach ($f in $Failed) {
        Write-Host "    - $f"
    }
}
Write-Host ""
Write-Host "Done."

} finally {
    Pop-Location
}
