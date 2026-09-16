param(
    [string]$InstallerScript = (Join-Path (Split-Path $PSScriptRoot -Parent) "installer\kite.nsi")
)

$ErrorActionPreference = "Stop"

$source = Get-Content -Raw -Encoding UTF8 -LiteralPath $InstallerScript
$failures = [System.Collections.Generic.List[string]]::new()

# 覆盖安装＝就地覆盖，安装器不得再调用旧版卸载器。
if ($source -match '\$OldInstallDir') {
    $failures.Add("The installer references `$OldInstallDir again. Upgrades are plain overwrites, so no separate old-version uninstall step may come back.")
}

if ($source -match '(?m)^\s*ExecWait\s+''"\$INSTDIR\\uninstall\.exe"') {
    $failures.Add("The installer runs its own uninstaller before copying files, which leaves a partially removed app when the install is aborted.")
}

if ($source -match 'Section\s+"[^"]*卸载旧版') {
    $failures.Add("The old-version removal component is still offered to the user.")
}

if ($source -match 'SectionSetText\s+\$\{SEC_REMOVE_OLD\}') {
    $failures.Add("The installer still toggles the visibility of the removed old-version removal component.")
}

# 就地覆盖的前提：默认安装目录必须记住上一版的位置。
if ($source -notmatch 'InstallDirRegKey\s+HKLM\s+"Software\\Kite"\s+"InstallLocation"') {
    $failures.Add("The installer does not remember the previous install directory, so an upgrade would install to the default path and leave the old copy behind.")
}

# 覆盖 kite.exe 前必须先结束旧进程，且只结束 Kite 自身。
$closeOld = [regex]::Match(
    $source,
    '(?m)^\s*ExecWait\s+''"\$SYSDIR\\taskkill\.exe"(?<arguments>[^'']*)''\s*$'
)

if (-not $closeOld.Success) {
    $failures.Add("The installer no longer stops the running Kite, so overwriting kite.exe fails while the old build holds the file open.")
}
else {
    $arguments = $closeOld.Groups['arguments'].Value
    if ($arguments -notmatch '/IM\s+kite\.exe') {
        $failures.Add("The installer does not target kite.exe when stopping the running instance.")
    }
    if ($arguments -match '/T(?:\s|$)') {
        $failures.Add("taskkill /T also closes the applications Kite launched. Stop only kite.exe.")
    }
}

if ($source -notmatch 'WriteUninstaller\s+"\$INSTDIR\\uninstall\.exe"') {
    $failures.Add("The installer does not write an uninstaller, so an installed copy could never be removed.")
}

if ($source -notmatch 'WriteRegStr\s+HKLM\s+"Software\\Kite"\s+"InstallLocation"\s+"\$INSTDIR"') {
    $failures.Add("The installer does not record the install location, so the next upgrade cannot reuse it.")
}

if ($source -notmatch 'DeleteRegKey\s+HKLM\s+"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Kite"') {
    $failures.Add("Uninstalling leaves the Kite entry in Programs and Features.")
}

if ($failures.Count -gt 0) {
    throw ($failures -join [Environment]::NewLine)
}

Write-Host "Installer upgrade contracts passed."
