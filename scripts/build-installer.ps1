param([string]$Nsis = "")
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$manifest = Join-Path $root "Cargo.toml"
$artifacts = Join-Path $root "artifacts"
New-Item -ItemType Directory -Force $artifacts | Out-Null
Write-Host "Building official plugins..."
& (Join-Path $PSScriptRoot "build-official-plugins.ps1")
if ($LASTEXITCODE -ne 0) { throw "build-official-plugins failed with exit code $LASTEXITCODE" }
Write-Host "Building Kite (release)..."
& cargo build --release --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
if (!$Nsis) {
  $Nsis = @((Get-Command makensis.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue), "C:\Program Files (x86)\NSIS\makensis.exe", "C:\Program Files\NSIS\makensis.exe") | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (!$Nsis) { throw "NSIS not found. Install with: winget install NSIS.NSIS" }
Write-Host "Building NSIS installer..."
& $Nsis (Join-Path $root "installer\kite.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE" }
$exe = Join-Path $root "target\release\kite.exe"
Copy-Item $exe (Join-Path $artifacts "kite.exe") -Force
Write-Host "Created: $(Join-Path $artifacts 'kite-setup.exe')"