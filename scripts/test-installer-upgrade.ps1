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

# 官方插件是独立子进程；升级时也必须释放它们持有的文件句柄。
foreach ($pluginProcess in @(
    'kite-plugin-calculator.exe',
    'kite-plugin-window-switcher.exe',
    'kite-plugin-devtools.exe',
    'kite-plugin-loci.exe'
)) {
    if ($source -notmatch [regex]::Escape("/IM $pluginProcess")) {
        $failures.Add("The installer does not stop the official plugin process $pluginProcess before overwriting bundled files.")
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

# Service updates must stop through the ownership-checking script before overwrite.
$serviceSection = [regex]::Match($source, '(?s)Section\s+"Loci[^"\r\n]*"\s+SEC_LOCI_SERVICE(?<body>.*?)SectionEnd')
if (-not $serviceSection.Success) {
    $failures.Add("The installer does not offer the Loci service component.")
} else {
    $serviceBody = $serviceSection.Groups['body'].Value
    $stopOffset = $serviceBody.IndexOf('-StopOnly')
    $copyOffset = $serviceBody.IndexOf('File "..\resources\loci\loci-service.exe"')
    if ($stopOffset -lt 0 -or $copyOffset -lt 0 -or $stopOffset -gt $copyOffset) {
        $failures.Add("The installer must stop the owned Loci service before copying its executable.")
    }
    if ($serviceBody -match 'SectionIn\s+RO') {
        $failures.Add("The Loci service must remain an optional component.")
    }
    if ($serviceBody -notmatch 'ExecWait[^\r\n]+install-service\.ps1[^\r\n]+-ExecutablePath') {
        $failures.Add("The service component must use the shared service lifecycle script.")
    }
    if ([regex]::Matches($serviceBody, 'SetErrorLevel\s+1\s+Abort').Count -lt 2) {
        $failures.Add("Stopping and starting failures must abort the service component with a failure exit code.")
    }
}
$uninstallSection = [regex]::Match($source, '(?s)Section\s+"Uninstall"(?<body>.*?)SectionEnd')
if (-not $uninstallSection.Success -or $uninstallSection.Groups['body'].Value -notmatch 'ExecWait[^\r\n]+install-service\.ps1[^\r\n]+-Uninstall') {
    $failures.Add("Uninstall must remove the owned Loci service through the shared lifecycle script.")
}
if ($source -match '(?i)taskkill[^\r\n]+loci-service\.exe') {
    $failures.Add("The Loci service must stop through SCM, never through taskkill.")
}

if ($failures.Count -gt 0) {
    throw ($failures -join [Environment]::NewLine)
}

Write-Host "Installer upgrade contracts passed."
