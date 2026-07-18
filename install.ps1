# Installs flashmark.exe to %LOCALAPPDATA%\Programs\Flashmark and adds that
# directory to the user PATH, so `flashmark` works from any terminal.
#
# Prefers building from this clone when Rust is available; otherwise downloads
# the latest release binary with the GitHub CLI (the repo is private, so a
# plain web download won't work).

$ErrorActionPreference = 'Stop'

$dest = Join-Path $env:LOCALAPPDATA 'Programs\Flashmark'
$destExe = Join-Path $dest 'flashmark.exe'
New-Item -ItemType Directory -Force $dest | Out-Null

if (Get-Command cargo -ErrorAction SilentlyContinue) {
    Write-Host 'Building release binary...'
    cargo build --release --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    Copy-Item (Join-Path $PSScriptRoot 'target\release\flashmark.exe') $destExe -Force
} elseif (Get-Command gh -ErrorAction SilentlyContinue) {
    Write-Host 'No Rust toolchain found — downloading the latest release via gh...'
    gh release download --repo Azmekk/flashmark --pattern 'flashmark.exe' --dir $dest --clobber
    if ($LASTEXITCODE -ne 0) { throw 'gh release download failed — is there a release yet?' }
} else {
    throw 'Neither cargo nor gh is available. Install Rust (https://rustup.rs) or the GitHub CLI (https://cli.github.com).'
}

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $dest) {
    $newPath = if ([string]::IsNullOrEmpty($userPath)) { $dest } else { "$userPath;$dest" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "Added $dest to your user PATH — open a new terminal for it to take effect."
}

Write-Host "Installed $destExe"
& $destExe --version
