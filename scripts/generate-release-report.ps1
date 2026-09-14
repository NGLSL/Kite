param(
    [string]$Tag = $env:GITHUB_REF_NAME,
    [string]$Repository = $env:GITHUB_REPOSITORY,
    [string]$RunId = $env:GITHUB_RUN_ID,
    [string]$Workflow = $env:GITHUB_WORKFLOW,
    [string]$RunNumber = $env:GITHUB_RUN_NUMBER
)

$ErrorActionPreference = "Stop"

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
if (Test-Path $notesPath) {
    $curated = Get-Content -Raw $notesPath
} else {
    $curated = @(
        "## 版本说明"
        ""
        "本版本没有提供对应的维护者版本说明，以下自动信息仍然会完整记录构建和提交范围。后续版本请在打 tag 前补充 $(Format-Code $notesPath)。"
    ) -join "`n"
}

$installer = Get-Item "artifacts/kite-setup.exe"
$sha256 = (Get-FileHash $installer.FullName -Algorithm SHA256).Hash
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
    "- 验证步骤：$(Format-Code 'cargo test')、$(Format-Code 'cargo build --release')、NSIS 安装包构建"
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
    "## 安装与升级"
    ""
    "下载 $(Format-Code 'kite-setup.exe') 后运行安装程序。安装器会结束正在运行的旧版、保留用户选择的安装目录，并创建开始菜单和桌面快捷方式。Kite 本身以普通用户权限运行；安装器需要管理员权限用于写入安装目录和卸载信息。"
    ""
    "## Code signing policy"
    ""
    "Kite 正准备申请 SignPath.io 的免费开源项目签名，目前尚未获批。本版本安装包未进行 Authenticode 数字签名。项目角色、签名政策和隐私说明见 [Kite Code signing policy](https://github.com/$Repository/blob/main/CODE_SIGNING_POLICY.md)。"
)

$body = ($curated.TrimEnd() + "`n`n" + ($automatic -join "`n")).Trim() + "`n"
Set-Content -Path "release-body.md" -Value $body -Encoding utf8
Write-Host "Release report written to release-body.md"
