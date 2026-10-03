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
$loci = Join-Path (Split-Path -Parent $root) "Loci"
if (-not (Test-Path (Join-Path $loci "Cargo.toml"))) {
    throw "Loci engine checkout not found: $loci. Clone NGLSL/Loci next to Kite; see docs/LOCI-PLUGIN.md."
}
foreach ($serviceContract in @("src\bin\loci-service.rs", "src\service.rs", "scripts\install-service.ps1")) {
    if (-not (Test-Path -LiteralPath (Join-Path $loci $serviceContract) -PathType Leaf)) {
        throw "Loci checkout lacks service API: missing $serviceContract in $loci. Use a Loci revision containing the Windows service implementation."
    }
}

Push-Location $ws
try {
    if ($Release) {
        cargo +1.99.0 build --release --locked
        if ($LASTEXITCODE -ne 0) { throw "official plugins cargo build failed with exit code $LASTEXITCODE" }
        $outDir = Join-Path $ws "target\release"
    } else {
        cargo +1.99.0 build --locked
        if ($LASTEXITCODE -ne 0) { throw "official plugins cargo build failed with exit code $LASTEXITCODE" }
        $outDir = Join-Path $ws "target\debug"
    }
    cargo +1.99.0 test --locked
    if ($LASTEXITCODE -ne 0) { throw "official plugins cargo test failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

# The system file index runs in a Windows service, independently of the plugins.
$lociManifest = Join-Path $loci "Cargo.toml"
if ($Release) {
    cargo +1.99.0 build --manifest-path $lociManifest --bin loci-service --bin loci --release --locked
    $lociOutDir = Join-Path $loci "target\release"
} else {
    cargo +1.99.0 build --manifest-path $lociManifest --bin loci-service --bin loci --locked
    $lociOutDir = Join-Path $loci "target\debug"
}
if ($LASTEXITCODE -ne 0) { throw "Loci service cargo build failed with exit code $LASTEXITCODE" }
$lociService = Join-Path $lociOutDir "loci-service.exe"
$lociCli = Join-Path $lociOutDir "loci.exe"
$lociInstaller = Join-Path $loci "scripts\install-service.ps1"
foreach ($required in @($lociService, $lociCli, $lociInstaller)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
        throw "missing Loci service package input: $required"
    }
}
$lociResources = Join-Path $root "resources\loci"
New-Item -ItemType Directory -Force $lociResources | Out-Null
Copy-Item -LiteralPath $lociService -Destination (Join-Path $lociResources "loci-service.exe") -Force
Copy-Item -LiteralPath $lociCli -Destination (Join-Path $lociResources "loci.exe") -Force
Copy-Item -LiteralPath $lociInstaller -Destination (Join-Path $lociResources "install-service.ps1") -Force
Write-Host "Loci service ready under $lociResources"

# crateDir|pluginId|exe
$packages = @(
    "calculator|com.kite.calculator|kite-plugin-calculator.exe"
    "window-switcher|com.kite.window-switcher|kite-plugin-window-switcher.exe"
    "devtools|com.kite.devtools|kite-plugin-devtools.exe"
    "loci|com.kite.loci|kite-plugin-loci.exe"
)

$pluginsRoot = Join-Path $root "resources\official-plugins"
if (Test-Path $pluginsRoot) {
    $resolvedPlugins = (Resolve-Path -LiteralPath $pluginsRoot).Path
    $expectedPlugins = [System.IO.Path]::GetFullPath((Join-Path $root "resources\official-plugins"))
    if ($resolvedPlugins -ne $expectedPlugins) {
        throw "Unexpected official plugin output directory: $resolvedPlugins"
    }
    Remove-Item -LiteralPath $resolvedPlugins -Recurse -Force
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
