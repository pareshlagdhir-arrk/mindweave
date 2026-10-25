# Builds Mindweave from source and installs it for the current user
# (Windows). macOS and Linux: scripts/install.sh.
#
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1            the app and the MCP server
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -SkipMcp   the app only
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -SkipApp   the MCP server only
#
# Installs, without administrator rights:
#   %LOCALAPPDATA%\Programs\Mindweave\mindweave.exe and a Start menu entry
#   %USERPROFILE%\.cargo\bin\mindweave-mcp.exe (the MCP server for AI agents)
#
# MINDWEAVE_APP_DIR and MINDWEAVE_BIN_DIR override where the app and the
# MCP server go; MINDWEAVE_NO_SHORTCUT=1 skips the Start menu entry.
[CmdletBinding()]
param(
    [switch]$SkipApp,
    [switch]$SkipMcp
)
$ErrorActionPreference = 'Stop'

function Say($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "Error: $text" -ForegroundColor Red; exit 1 }

if ($SkipApp -and $SkipMcp) { Fail 'Nothing to do: both -SkipApp and -SkipMcp were given.' }

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# --- Prerequisites -----------------------------------------------------

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Fail "Rust is not installed. Install it from https://rustup.rs (choose the default MSVC toolchain), then open a new terminal and run this script again."
}

# The MSVC linker comes with Visual Studio or its Build Tools.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$haveMsvc = $false
if (Test-Path $vswhere) {
    $found = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $haveMsvc = [bool]$found
}
if (-not $haveMsvc) {
    Fail "The Visual Studio C++ build tools are missing. Install 'Build Tools for Visual Studio' with the 'Desktop development with C++' workload: https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022"
}

# --- Build -------------------------------------------------------------

$packages = @()
if (-not $SkipApp) { $packages += @('-p', 'mindweave-desktop') }
if (-not $SkipMcp) { $packages += @('-p', 'mindweave-mcp') }
Say 'Building (release; the first build takes several minutes)'
& cargo build --release --locked @packages
if ($LASTEXITCODE -ne 0) { Fail 'The build failed (see the messages above).' }

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$binDir = if ($env:MINDWEAVE_BIN_DIR) { $env:MINDWEAVE_BIN_DIR } else { Join-Path $cargoHome 'bin' }
$appDir = if ($env:MINDWEAVE_APP_DIR) { $env:MINDWEAVE_APP_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\Mindweave' }

# --- Install the app ---------------------------------------------------

if (-not $SkipApp) {
    Say "Installing $appDir\mindweave.exe"
    New-Item -ItemType Directory -Force -Path $appDir | Out-Null
    Copy-Item 'target\release\mindweave.exe' (Join-Path $appDir 'mindweave.exe') -Force
    Copy-Item 'crates\mindweave-desktop\assets\app-icon.png' (Join-Path $appDir 'app-icon.png') -Force

    if ($env:MINDWEAVE_NO_SHORTCUT -ne '1') {
        $programs = [Environment]::GetFolderPath('Programs')
        $link = Join-Path $programs 'Mindweave.lnk'
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($link)
        $shortcut.TargetPath = Join-Path $appDir 'mindweave.exe'
        $shortcut.WorkingDirectory = $appDir
        $shortcut.Description = 'Mind maps and diagrams'
        $shortcut.Save()
        Say "Added a Start menu entry: $link"
    }
}

# --- Install the MCP server ---------------------------------------------

if (-not $SkipMcp) {
    Say "Installing $binDir\mindweave-mcp.exe"
    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    Copy-Item 'target\release\mindweave-mcp.exe' (Join-Path $binDir 'mindweave-mcp.exe') -Force
}

# --- Done ----------------------------------------------------------------

Write-Host ''
Say "Done (Mindweave $version)"
if (-not $SkipApp) {
    Write-Host "  Open Mindweave from the Start menu, or run: $appDir\mindweave.exe"
    Write-Host '  Windows support is early: shortcuts still use the Windows key instead of Ctrl, and there is no menu bar yet.'
}
if (-not $SkipMcp) {
    Write-Host '  To let an AI agent use Mindweave, register the MCP server once, for example:'
    Write-Host "    claude mcp add --scope user mindweave -- `"$binDir\mindweave-mcp.exe`""
    Write-Host "    codex mcp add mindweave -- `"$binDir\mindweave-mcp.exe`""
    Write-Host '  Other clients: docs\agents.md'
}
