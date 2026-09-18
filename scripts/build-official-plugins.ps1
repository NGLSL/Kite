# Build official Rust plugins and assemble packages under resources/official-plugins.
# Usage: .\scripts\build-official-plugins.ps1 [-Release]
# Note: keep this script ASCII-only. Windows PowerShell 5.1 reads UTF-8 without BOM
# as ANSI, which corrupts non-ASCII comments and breaks parsing.
# Do not parse plugin.json via Get-Content (same encoding pitfall); use the table below.

param(
    [switch]$Release = $true
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$ws = Join-Path $root "official-plugins"
if (-not (Test-Path (Join-Path $ws "Cargo.toml"))) {
    throw "official-plugins workspace not found: $ws"
}

Push-Location $ws
try {
    if ($Release) {
        cargo build --release
        $outDir = Join-Path $ws "target\release"
    } else {
        cargo build
        $outDir = Join-Path $ws "target\debug"
    }
    cargo test
} finally {
    Pop-Location
}

# crateDir|pluginId|exe
$packages = @(
    "calculator|com.kite.calculator|kite-plugin-calculator.exe"
    "window-switcher|com.kite.window-switcher|kite-plugin-window-switcher.exe"
    "devtools|com.kite.devtools|kite-plugin-devtools.exe"
)

$pluginsRoot = Join-Path $root "resources\official-plugins"
if (Test-Path $pluginsRoot) {
    Remove-Item -Recurse -Force $pluginsRoot
}
New-Item -ItemType Directory -Force $pluginsRoot | Out-Null

foreach ($row in $packages) {
    $parts = $row.Split("|")
    if ($parts.Count -ne 3) {
        throw "bad package row: $row"
    }
    $crateDir = $parts[0]
    $pluginId = $parts[1]
    $cmd = $parts[2]
    $manifest = Join-Path $ws "$crateDir\plugin.json"
    if (-not (Test-Path $manifest)) {
        throw "missing plugin.json: $manifest"
    }
    # Read manifest as UTF-8 explicitly; system ANSI codepage corrupts CJK strings.
    $utf8 = New-Object System.Text.UTF8Encoding $false
    $json = [System.IO.File]::ReadAllText($manifest, $utf8) | ConvertFrom-Json
    if ($json.plugin.id -ne $pluginId) {
        throw "plugin.json id mismatch: expected $pluginId, got $($json.plugin.id) in $manifest"
    }
    if ($json.runtime.command -ne $cmd) {
        throw "plugin.json command mismatch in ${manifest}: expected $cmd, got $($json.runtime.command)"
    }
    $src = Join-Path $outDir $cmd
    if (-not (Test-Path $src)) {
        throw "missing build output: $src"
    }
    $destDir = Join-Path $pluginsRoot $pluginId
    New-Item -ItemType Directory -Force $destDir | Out-Null
    Copy-Item -Force $manifest (Join-Path $destDir "plugin.json")
    Copy-Item -Force $src (Join-Path $destDir $cmd)
    Write-Host "packed $pluginId ($cmd)"
}

Write-Host "official plugins ready under $pluginsRoot"
