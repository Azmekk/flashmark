# Installs flashmark to %LOCALAPPDATA%\Programs\Flashmark and adds that
# directory to the user PATH, so `flashmark` works from any terminal.
#
# One-liner (PowerShell):
#   irm https://raw.githubusercontent.com/Azmekk/flashmark/master/install.ps1 | iex
#
# From a clone, `.\install.ps1` builds from source when Rust is available and
# falls back to downloading the latest release otherwise.

$ErrorActionPreference = 'Stop'

$dest = Join-Path $env:LOCALAPPDATA 'Programs\Flashmark'
$destExe = Join-Path $dest 'flashmark.exe'
New-Item -ItemType Directory -Force $dest | Out-Null

$fromClone = $PSScriptRoot -and (Test-Path (Join-Path $PSScriptRoot 'Cargo.toml'))
if ($fromClone -and (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host 'Building release binary from source...'
    cargo build --release --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    Copy-Item (Join-Path $PSScriptRoot 'target\release\flashmark.exe') $destExe -Force
} else {
    Write-Host 'Downloading the latest flashmark release...'
    Invoke-WebRequest -Uri 'https://github.com/Azmekk/flashmark/releases/latest/download/flashmark.exe' -OutFile $destExe
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $dest) {
    $newPath = if ([string]::IsNullOrEmpty($userPath)) { $dest } else { "$userPath;$dest" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "Added $dest to your user PATH — open a new terminal for it to take effect."
}

Write-Host "Installed $destExe"
& $destExe --version
