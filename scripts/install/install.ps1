# scripts/install/install.ps1
#
# Install the latest `temper` CLI binary on Windows x86_64. Usage:
#
#   irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1 | iex
#
# Or to install a specific version:
#
#   $script = irm https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.ps1
#   & ([scriptblock]::Create($script)) -Version v0.1.0
#
# Installs to:
#   $env:LOCALAPPDATA\Programs\temper\
# and appends that directory to the user PATH.

[CmdletBinding()]
param(
    [string]$Version = ""
)

$ErrorActionPreference = 'Stop'

$Repo = "tasker-systems/temper"

if ($PSVersionTable.PSVersion.Major -lt 5) {
    Write-Error "PowerShell 5.1 or later is required. Found: $($PSVersionTable.PSVersion)"
    exit 1
}

if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    Write-Error @"
No prebuilt binary for Windows $($env:PROCESSOR_ARCHITECTURE).

Temper v1 only ships Windows x86_64 binaries. Build from source requires
installing Rust (https://rustup.rs) and running:

  git clone https://github.com/$Repo
  cd temper
  cargo install --path crates/temper-cli --features embed,extract
"@
    exit 1
}

$Target = "x86_64-pc-windows-msvc"

if (-not $Version) {
    try {
        $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
        $Version = $latest.tag_name
    } catch {
        Write-Error "Could not determine latest release: $_"
        exit 1
    }
}

if (-not $Version) {
    Write-Error "Could not determine a version to install."
    exit 1
}

Write-Host "Installing temper $Version ($Target)..."

$Archive = "temper-$Version-$Target.zip"
$UrlBase = "https://github.com/$Repo/releases/download/$Version"
$ArchiveUrl = "$UrlBase/$Archive"
$ShaUrl = "$UrlBase/$Archive.sha256"

$TmpDir = Join-Path $env:TEMP "temper-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $ArchivePath = Join-Path $TmpDir $Archive
    $ShaPath = "$ArchivePath.sha256"

    Write-Host "  Downloading $Archive..."
    Invoke-WebRequest -Uri $ArchiveUrl -OutFile $ArchivePath -UseBasicParsing
    Invoke-WebRequest -Uri $ShaUrl -OutFile $ShaPath -UseBasicParsing

    Write-Host "  Verifying checksum..."
    $expected = (Get-Content $ShaPath -Raw).Trim().Split()[0].ToLowerInvariant()
    $actual = (Get-FileHash $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expected -ne $actual) {
        Write-Error "Checksum mismatch. Expected: $expected, got: $actual"
        exit 1
    }

    $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\temper"

    if (Test-Path $InstallDir) {
        Write-Host "  Removing previous install at $InstallDir..."
        Remove-Item -Recurse -Force $InstallDir
    }

    Write-Host "  Extracting to $InstallDir..."
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Expand-Archive -Path $ArchivePath -DestinationPath $InstallDir -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($userPath -notlike "*$InstallDir*") {
        $newPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host "  Added $InstallDir to user PATH (restart your shell to take effect)"
    } else {
        Write-Host "  User PATH already contains $InstallDir"
    }
} finally {
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Installed temper $Version to $InstallDir"
Write-Host "Run: temper --help"
Write-Host ""
Write-Host "Note: restart your terminal (or sign out and back in) for the PATH"
Write-Host "change to take effect."
