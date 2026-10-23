param(
    [string]$Tag = $env:GITHUB_REF_NAME,
    [string]$Repository = $env:GITHUB_REPOSITORY,
    [string]$RunId = $env:GITHUB_RUN_ID,
    [string]$Workflow = $env:GITHUB_WORKFLOW,
    [string]$RunNumber = $env:GITHUB_RUN_NUMBER,
    [string]$InstallerValidationPath = "artifacts/installer-validation.json"
)

$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true") {
    throw "Formal release reports are generated in GitHub Actions from the validated installer artifact."
}

function Format-Code([string]$Value) {
    return ('`{0}`' -f $Value)
}

if ([string]::IsNullOrWhiteSpace($Tag)) {
    $Tag = (git describe --tags --exact-match 2>$null).Trim()
}
if ([string]::IsNullOrWhiteSpace($Tag)) {
    throw "A release tag is required."
}

$tagCommit = (git rev-list -n 1 $Tag).Trim()
if ($LASTEXITCODE -ne 0) { throw "Unable to resolve release tag $Tag." }
$headCommit = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $headCommit -ne $tagCommit) {
    throw "The checked-out commit must match release tag $Tag."
}
$version = [regex]::Match((Get-Content -Raw -LiteralPath "Cargo.toml"), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ([string]::IsNullOrWhiteSpace($version) -or $Tag -cne "v$version") {
    throw "Release tag $Tag does not match Cargo.toml version $version."
}
$previousTag = @(git tag --sort=-v:refname | Where-Object { $_ -ne $Tag } | Select-Object -First 1)
if ($previousTag.Count -gt 0) {
    $range = "$($previousTag[0])..$Tag"
    $previousLabel = $previousTag[0]
} else {
    $range = $Tag
    $previousLabel = "无（首次发布）"
}

$commitCount = (git rev-list --count $range).Trim()
$commitLines = @(git log $range --pretty=format:'- %s (%h)')
if ($commitLines.Count -eq 0) {
    $commitLines = @("- 本次 tag 没有可列出的提交")
}

$notesPath = "docs/releases/$Tag.md"
if (-not (Test-Path -LiteralPath $notesPath -PathType Leaf)) {
    throw "Maintainer release notes are required: $notesPath"
}
$curated = Get-Content -Raw -Encoding UTF8 -LiteralPath $notesPath

$installer = Get-Item "artifacts/kite-setup.exe"
$sha256 = (Get-FileHash $installer.FullName -Algorithm SHA256).Hash
if (-not (Test-Path -LiteralPath $InstallerValidationPath -PathType Leaf)) {
    throw "Installer validation evidence is required: $InstallerValidationPath"
}
$validation = Get-Content -Raw -Encoding UTF8 -LiteralPath $InstallerValidationPath | ConvertFrom-Json
if ($validation.schemaVersion -ne 1 -or $validation.commit -ne $tagCommit -or $validation.version -cne $version) {
    throw "Installer validation evidence does not match the release commit/version."
}
if ($validation.installerSha256 -ne $sha256 -or [long]$validation.installerBytes -ne $installer.Length) {
    throw "Downloaded installer hash/size does not match the installer that passed validation."
}
foreach ($check in @("cleanInstall", "sameVersionOverwrite", "uninstall", "userDataPreserved", "payloadHashes", "registry", "serviceInstallUpdateUninstall")) {
    if ($validation.checks.$check -isnot [bool] -or $validation.checks.$check -ne $true) {
        throw "Required installer validation did not pass: $check"
    }
}
if ($validation.serviceHostReplaced -isnot [bool] -or $validation.serviceHostReplaced -ne $true -or
    $validation.serviceHostFileIdBeforeOverwrite -notmatch '^0x[0-9a-f]{32}$' -or
    $validation.serviceHostFileIdAfterOverwrite -notmatch '^0x[0-9a-f]{32}$' -or
    $validation.serviceHostFileIdBeforeOverwrite -eq $validation.serviceHostFileIdAfterOverwrite) {
    throw "Index service update evidence must show that the host file identity changed."
}
$fileSearch = $validation.fileSearch
$payloadManifest = Get-Content -Raw -LiteralPath "artifacts/payload-manifest.json" | ConvertFrom-Json
$applicationPayload = @($payloadManifest.payload | Where-Object path -eq "kite.exe")
if ($fileSearch.commit -ne $tagCommit -or $applicationPayload.Count -ne 1 -or
    $fileSearch.installedExeSha256 -ne $applicationPayload[0].sha256 -or
    $payloadManifest.installerSha256 -ne $sha256 -or $payloadManifest.commit -ne $tagCommit) {
    throw "Real file-search validation is not bound to this installed release payload."
}
foreach ($check in @("realNtfsCatchup", "over65536Objects", "durableResume", "sentinelCreateRenameDelete", "feedRotation", "feedReclamation")) {
    if ($fileSearch.checks.$check -isnot [bool] -or $fileSearch.checks.$check -ne $true) {
        throw "Required real file-search validation did not pass: $check"
    }
}
if ([long]$fileSearch.createdSurvivingFiles -le 65536 -or [long]$fileSearch.observedOverlayObjects -le 65536 -or
    [int]$fileSearch.observedFeedSegments -lt 2 -or [long]$fileSearch.feedSegmentBytes -ne 33554432 -or
    $fileSearch.firstSegmentReclaimed -isnot [bool] -or $fileSearch.firstSegmentReclaimed -ne $true -or
    -not $fileSearch.generationBeforeRestart -or $fileSearch.generationBeforeRestart -ne $fileSearch.generationAfterRestart -or
    -not $fileSearch.cursorsBeforeRestart -or -not $fileSearch.loadedCursorsAfterRestart -or
    [long]$fileSearch.resumedFileVolumeCursor -le [long]$fileSearch.originalBuildCursor) {
    throw "Real file-search object, segment or durable recovery evidence is incomplete."
}
foreach ($cursor in $fileSearch.cursorsBeforeRestart) {
    $loaded = @($fileSearch.loadedCursorsAfterRestart | Where-Object volume -eq $cursor.volume)
    if ($loaded.Count -ne 1 -or $loaded[0].journal_id -ne $cursor.journal_id -or [long]$loaded[0].next_usn -lt [long]$cursor.next_usn) {
        throw "Published recovery evidence regressed or changed journal identity."
    }
}
$checksumPath = "artifacts/kite-setup.exe.sha256"
$checksum = (Get-Content -Raw -LiteralPath $checksumPath).Trim()
if ($checksum -notmatch ('(?i)^' + [regex]::Escape($sha256) + '\s+\*?kite-setup\.exe$')) {
    throw "The published checksum sidecar does not match the validated installer."
}
$sizeMiB = [math]::Round($installer.Length / 1MB, 2)
$runUrl = "https://github.com/$Repository/actions/runs/$RunId"
$commitUrl = "https://github.com/$Repository/commit/$tagCommit"

$automatic = @(
    "## 构建与验证"
    ""
    "- 发布 tag：$(Format-Code $Tag)"
    "- tag 提交：[$(Format-Code $tagCommit)]($commitUrl)"
    "- 上一个发布：$(Format-Code $previousLabel)"
    "- 提交数量：$(Format-Code $commitCount)"
    "- 构建环境：GitHub Actions $(Format-Code 'windows-latest')"
    "- 检查与构建：精确 main 提交 CI、$(Format-Code 'cargo test --locked')、$(Format-Code 'cargo build --release --locked')、官方插件与 NSIS 打包"
    "- 独立 Windows runner 安装验收：全新安装、同版本覆盖、卸载、负载哈希、注册表、用户数据保留、按需读取组件注册／更新／卸载"
    "- 真实 NTFS 补读：停机期间创建 $($fileSearch.createdSurvivingFiles) 个存活文件，查询变化层实际 $($fileSearch.observedOverlayObjects) 个对象；新增／改名／删除哨兵查询通过"
    "- 分段与恢复：实际生成 $($fileSearch.observedFeedSegments) 个 32 MiB feed 段，确认首段已回收；同一 generation 重启从已提交游标续读"
    "- 产物核对：发布 job 重新计算安装器大小和 SHA-256，与验收报告及校验和文件一致后才上传"
    "- 验收报告：[installer-validation.json](https://github.com/$Repository/releases/download/$Tag/installer-validation.json)"
    "- 验证边界：同版本覆盖不包含旧版本迁移；未验证人工 UAC 拒绝、桌面交互、强制崩溃或磁盘满恢复"
    "- 构建记录：[$Workflow #$RunNumber]($runUrl)"
    ""
    "## 本次提交范围"
    ""
    ($commitLines -join "`n")
    ""
    "## 下载文件"
    ""
    "| 文件 | 大小 | SHA-256 |"
    "| --- | ---: | --- |"
    "| $(Format-Code 'kite-setup.exe') | $sizeMiB MiB | $(Format-Code $sha256) |"
    ""
    "同时提供 $(Format-Code 'kite-setup.exe.sha256') 和 $(Format-Code 'installer-validation.json')，均由 GitHub Actions 生成并随安装器发布。"
    ""
    "## 安装与升级"
    ""
    "下载 $(Format-Code 'kite-setup.exe') 后运行安装程序。安装器会结束正在运行的旧版、保留用户选择的安装目录，并创建开始菜单和桌面快捷方式。Kite 本身以普通用户权限运行；安装器需要管理员权限用于写入安装目录和卸载信息。"
    ""
    "## 隐私"
    ""
    "Kite 的应用索引、设置、启动历史和诊断日志保存在本机，不含遥测。「检查更新」会请求 GitHub API；下载更新会从 GitHub Releases 拉取安装包。网页搜索查询会发到本机配置的搜索服务。"
)

$body = ($curated.TrimEnd() + "`n`n" + ($automatic -join "`n")).Trim() + "`n"
Set-Content -Path "release-body.md" -Value $body -Encoding utf8
Write-Host "Release report written to release-body.md"
