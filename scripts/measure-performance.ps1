# Kite 100 次连续唤起 + 驻留资源采样。
# 方法对齐 docs/PERFORMANCE.md：Alt+Space → 窗口可见 → Esc，间隔 20ms。
# 用法：.\scripts\measure-performance.ps1 [-Iterations 100] [-OutCsv path]
#
# 静默 CPU 口径：索引发布（index complete）后还有约 4-5 秒的收尾工作
# （无日志输出，实测 10 秒窗口内可烧掉 ~4.6 秒 CPU）。因此必须等这段收尾
# 结束再取基线，否则 idle_cpu_delta_s 量到的是收尾而不是静默。
param(
    [int]$Iterations = 100,
    [int]$IdleSeconds = 20,
    [int]$GapMs = 20,
    [int]$SettleSeconds = 15,
    [string]$OutCsv = "",
    [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class Win32Perf {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hWnd, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr extra);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr hWnd, EnumWindowsProc cb, IntPtr extra);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr extra);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hWnd, uint msg, IntPtr wParam, IntPtr lParam);
}
'@

function Send-AltSpace {
    # SendInput via .NET SendKeys 语义：Alt+Space 打开系统菜单，这里用 keybd_event 组合
    $sig = @'
using System;
using System.Runtime.InteropServices;
public static class Kb {
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
    public const uint KEYEVENTF_KEYUP = 0x0002;
    public static void AltSpace() {
        keybd_event(0x12, 0, 0, IntPtr.Zero); // VK_MENU
        keybd_event(0x20, 0, 0, IntPtr.Zero); // VK_SPACE
        keybd_event(0x20, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
        keybd_event(0x12, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
    }
    public static void Esc() {
        keybd_event(0x1B, 0, 0, IntPtr.Zero);
        keybd_event(0x1B, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
    }
}
'@
    if (-not ("Kb" -as [type])) { Add-Type $sig }
    [Kb]::AltSpace()
}

function Send-Esc {
    if (-not ("Kb" -as [type])) {
        Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Kb {
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, IntPtr extra);
    public const uint KEYEVENTF_KEYUP = 0x0002;
    public static void AltSpace() {
        keybd_event(0x12, 0, 0, IntPtr.Zero);
        keybd_event(0x20, 0, 0, IntPtr.Zero);
        keybd_event(0x20, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
        keybd_event(0x12, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
    }
    public static void Esc() {
        keybd_event(0x1B, 0, 0, IntPtr.Zero);
        keybd_event(0x1B, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
    }
}
'@
    }
    [Kb]::Esc()
}

function Get-KiteWindowHandle([int]$procId) {
    $found = [IntPtr]::Zero
    $cb = [Win32Perf+EnumWindowsProc]{
        param($h, $extra)
        $pidOut = 0
        [void][Win32Perf]::GetWindowThreadProcessId($h, [ref]$pidOut)
        if ($pidOut -eq $procId) {
            $sb = New-Object System.Text.StringBuilder 256
            [void][Win32Perf]::GetClassName($h, $sb, $sb.Capacity)
            $cls = $sb.ToString()
            # iced/winit window classes vary; title "Kite" is stable
            $title = (Get-Process -Id $procId).MainWindowTitle
            if ($cls -match 'Window|Iced|winit|ApplicationFrame' -or $true) {
                # Prefer process main window
            }
            if ($h -eq (Get-Process -Id $procId).MainWindowHandle) {
                $script:found = $h
                return $false
            }
        }
        return $true
    }
    $script:found = [IntPtr]::Zero
    [void][Win32Perf]::EnumWindows($cb, [IntPtr]::Zero)
    if ($script:found -ne [IntPtr]::Zero) { return $script:found }
    return (Get-Process -Id $procId).MainWindowHandle
}

function Wait-KiteVisible([int]$procId, [int]$timeoutMs) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $h = Get-KiteWindowHandle $procId
        if ($h -ne [IntPtr]::Zero -and [Win32Perf]::IsWindowVisible($h)) {
            return $sw.Elapsed.TotalMilliseconds
        }
        Start-Sleep -Milliseconds 1
    }
    return -1
}

function Wait-KiteHidden([int]$procId, [int]$timeoutMs) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        $h = Get-KiteWindowHandle $procId
        if ($h -eq [IntPtr]::Zero -or -not [Win32Perf]::IsWindowVisible($h)) {
            return $true
        }
        Start-Sleep -Milliseconds 1
    }
    return $false
}

function Get-LogMarker {
    $log = Join-Path $env:APPDATA "com.kite.launcher\kite.log"
    if (Test-Path $log) { (Get-Item $log).Length } else { 0 }
}

function Wait-IndexReady([int]$procId, [int]$timeoutMs, [long]$fromOffset) {
    # 只认本次启动之后新追加的日志：应用从不打印 pid，而 tail 里通常还留着
    # 上一次运行的 index complete，按 tail 判断会立刻"就绪"，把 6 秒多的索引
    # 扫描算进后面的静默窗口（idle_cpu_delta_s 因此被扫描 CPU 污染）。
    $log = Join-Path $env:APPDATA "com.kite.launcher\kite.log"
    $deadline = [DateTime]::UtcNow.AddMilliseconds($timeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path $log) {
            try {
                $fs = [System.IO.File]::Open($log, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
                try {
                    $len = $fs.Length
                    # 日志达到上限后会裁剪重写，偏移失效时从头读。
                    if ($len -lt $fromOffset) { $fromOffset = 0 }
                    if ($len -gt $fromOffset) {
                        [void]$fs.Seek($fromOffset, [System.IO.SeekOrigin]::Begin)
                        $buf = New-Object byte[] ($len - $fromOffset)
                        $read = $fs.Read($buf, 0, $buf.Length)
                        $text = [System.Text.Encoding]::UTF8.GetString($buf, 0, $read)
                        if ($text -match 'index complete n=\d+') { return $true }
                    }
                } finally { $fs.Close() }
            } catch {
                # 读取竞争（写盘/裁剪）不应中断测量，下一轮再试。
            }
        }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

if (-not $ExePath) {
    $ExePath = Join-Path $PSScriptRoot "..\target\release\kite.exe" | Resolve-Path
}
if (-not (Test-Path $ExePath)) {
    throw "Release exe not found: $ExePath (run cargo build --release first)"
}
if (-not $OutCsv) {
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $OutCsv = Join-Path $PSScriptRoot "..\docs\performance-kite-$stamp.csv" | Resolve-Path -ErrorAction SilentlyContinue
    if (-not $OutCsv) {
        $OutCsv = Join-Path (Split-Path $PSScriptRoot -Parent) "docs\performance-kite-$stamp.csv"
    }
}

# Kill existing kite
Get-Process kite -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

# 记录启动前的日志长度：只有这之后新出现的 index complete 才算本次就绪。
$logPath = Join-Path $env:APPDATA "com.kite.launcher\kite.log"
$logLenBefore = 0
if (Test-Path $logPath) { $logLenBefore = (Get-Item $logPath).Length }

Write-Host "Starting $ExePath ..."
$proc = Start-Process -FilePath $ExePath -PassThru
Start-Sleep -Milliseconds 800
if ($proc.HasExited) { throw "Kite exited immediately code=$($proc.ExitCode)" }

Write-Host "Waiting for index ready (pid=$($proc.Id)) ..."
$ready = Wait-IndexReady -procId $proc.Id -timeoutMs 60000 -fromOffset $logLenBefore
if (-not $ready) { Write-Warning "Index-ready marker not observed; continuing after extra wait" ; Start-Sleep -Seconds 3 }

# 索引发布后仍有收尾工作（见文件头说明）：等到它结束，静默样本才有意义。
Write-Host ("Settling {0}s before the idle baseline ..." -f $SettleSeconds)
Start-Sleep -Seconds $SettleSeconds

# Stable idle baseline
$before = Get-Process -Id $proc.Id
$beforeCpu = $before.TotalProcessorTime
Start-Sleep -Seconds $IdleSeconds
$after = Get-Process -Id $proc.Id
$afterCpu = $after.TotalProcessorTime
$cpuDelta = ($afterCpu - $beforeCpu).TotalSeconds
$privateIdle = [math]::Round($after.PrivateMemorySize64 / 1MB, 2)
$workingIdle = [math]::Round($after.WorkingSet64 / 1MB, 2)
Write-Host ("Idle {0}s: CPU +{1:N3}s private={2}MB working={3}MB" -f $IdleSeconds, $cpuDelta, $privateIdle, $workingIdle)

$rows = New-Object System.Collections.Generic.List[object]
for ($i = 1; $i -le $Iterations; $i++) {
    Send-AltSpace
    $openMs = Wait-KiteVisible -procId $proc.Id -timeoutMs 2000
    if ($openMs -lt 0) {
        Write-Warning "iter ${i}: window not visible within 2000ms"
        $openMs = 2000
        Send-Esc
        Start-Sleep -Milliseconds 100
    } else {
        # sample memory while visible
        $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
        if ($p) {
            $private = [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
            $working = [math]::Round($p.WorkingSet64 / 1MB, 2)
        } else {
            $private = 0; $working = 0
        }
        Send-Esc
        [void](Wait-KiteHidden -procId $proc.Id -timeoutMs 500)
    }
    $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
    if ($p) {
        $private = [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
        $working = [math]::Round($p.WorkingSet64 / 1MB, 2)
    } else {
        $private = 0; $working = 0
        Write-Warning "iter ${i}: process died"
        break
    }
    $rows.Add([pscustomobject]@{
        iteration = $i
        open_ms = [math]::Round($openMs, 3)
        private_mb = $private
        working_mb = $working
    })
    if ($i % 10 -eq 0) { Write-Host ("  {0}/{1} last={2}ms private={3}MB" -f $i, $Iterations, $openMs, $private) }
    Start-Sleep -Milliseconds $GapMs
}

# Final idle after loop
$final = Get-Process -Id $proc.Id
$finalPrivate = [math]::Round($final.PrivateMemorySize64 / 1MB, 2)
$finalWorking = [math]::Round($final.WorkingSet64 / 1MB, 2)

$rows | Export-Csv -Path $OutCsv -NoTypeInformation -Encoding UTF8
Write-Host "CSV: $OutCsv"

# Stats
function Get-Stats([double[]]$vals) {
    $sorted = $vals | Sort-Object
    $n = $sorted.Count
    $avg = ($vals | Measure-Object -Average).Average
    $p = {
        param($q)
        $idx = [math]::Ceiling($q * $n) - 1
        if ($idx -lt 0) { $idx = 0 }
        if ($idx -ge $n) { $idx = $n - 1 }
        return $sorted[$idx]
    }
    [pscustomobject]@{
        n = $n
        avg = [math]::Round($avg, 2)
        min = [math]::Round($sorted[0], 2)
        max = [math]::Round($sorted[$n-1], 2)
        p50 = [math]::Round((& $p 0.50), 2)
        p95 = [math]::Round((& $p 0.95), 2)
        p99 = [math]::Round((& $p 0.99), 2)
    }
}

$open = @($rows | ForEach-Object { [double]$_.open_ms })
$priv = @($rows | ForEach-Object { [double]$_.private_mb })
$work = @($rows | ForEach-Object { [double]$_.working_mb })
$openStats = Get-Stats $open
$privStats = Get-Stats $priv
$workStats = Get-Stats $work

$summary = [pscustomobject]@{
    version = (Get-Item $ExePath).VersionInfo.ProductVersion
    exe_bytes = (Get-Item $ExePath).Length
    pid = $proc.Id
    iterations = $openStats.n
    open_avg_ms = $openStats.avg
    open_min_ms = $openStats.min
    open_max_ms = $openStats.max
    open_p50_ms = $openStats.p50
    open_p95_ms = $openStats.p95
    open_p99_ms = $openStats.p99
    private_avg_mb = $privStats.avg
    private_min_mb = $privStats.min
    private_max_mb = $privStats.max
    working_avg_mb = $workStats.avg
    working_min_mb = $workStats.min
    working_max_mb = $workStats.max
    idle_seconds = $IdleSeconds
    settle_seconds = $SettleSeconds
    idle_cpu_delta_s = [math]::Round($cpuDelta, 3)
    idle_private_mb = $privateIdle
    idle_working_mb = $workingIdle
    final_private_mb = $finalPrivate
    final_working_mb = $finalWorking
    csv = $OutCsv
}

$summaryPath = [System.IO.Path]::ChangeExtension($OutCsv, ".summary.json")
$summary | ConvertTo-Json | Set-Content -Path $summaryPath -Encoding UTF8
Write-Host "SUMMARY: $summaryPath"
$summary | Format-List

# Leave process running? Stop for cleanliness
Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
Get-Process kite -ErrorAction SilentlyContinue | Stop-Process -Force
Write-Host "Done. Kite process stopped."
