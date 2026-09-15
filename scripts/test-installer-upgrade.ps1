param(
    [string]$InstallerScript = (Join-Path (Split-Path $PSScriptRoot -Parent) "installer\kite.nsi")
)

$ErrorActionPreference = "Stop"

$source = Get-Content -Raw -LiteralPath $InstallerScript
$failures = [System.Collections.Generic.List[string]]::new()
$oldUninstallerInvocation = [regex]::Match(
    $source,
    '(?m)^\s*ExecWait\s+''"\$OldInstallDir\\uninstall\.exe"(?<arguments>[^'']*)''\s*$'
)

if (-not $oldUninstallerInvocation.Success) {
    $failures.Add("No awaited old-version uninstaller invocation was found in $InstallerScript.")
}
else {
    $arguments = $oldUninstallerInvocation.Groups['arguments'].Value
    if ($arguments -notmatch '(?:^|\s)_\?=\$OldInstallDir(?:\s|$)') {
        $failures.Add("The old-version uninstaller can detach into a temporary process, allowing it to delete newly installed files. Pass _?=`$OldInstallDir so ExecWait covers the real uninstall operation.")
    }
}

if ($source -notmatch 'SectionSetText\s+\$\{SEC_REMOVE_OLD\}\s+""') {
    $failures.Add("The old-version removal option remains visible on a first install even when no old uninstaller exists.")
}

if ($source -notmatch 'IfFileExists\s+"\$OldInstallDir\\uninstall\.exe"\s+old_install_found') {
    $failures.Add("The installer does not verify that the remembered old install directory still contains an uninstaller.")
}

if ($source -notmatch 'StrCpy\s+\$OldInstallDir\s+""') {
    $failures.Add("A stale remembered install directory is not cleared before the install sections run.")
}

if ($failures.Count -gt 0) {
    throw ($failures -join [Environment]::NewLine)
}

Write-Host "Installer upgrade contracts passed."
