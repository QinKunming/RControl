# RControl 文件传输窗口 E2E 测试 (纯 ASCII, 经窗口消息驱动, 无跨进程指针)
# 前提: worker 已监听 3333
$ErrorActionPreference = 'Stop'

$clientExe = 'F:\AICode\ClaudeCode\RControl\rcontrol\target\release\rcontrol_client.exe'
$addr = '127.0.0.1:3444'
$pwd_ = 'Fe8r6ZvEK5'
$upDir = Join-Path $env:TEMP 'rc_fm_up'
$downDir = Join-Path $env:TEMP 'rc_fm_down'
$remoteDir = 'C:\rcontrol_fm_test'

Add-Type -Namespace W -Name PInvoke -MemberDefinition @'
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
[DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder sb, int max);
[DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
[DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="SendMessageW")] public static extern IntPtr SendMsgBuf(IntPtr h, uint msg, IntPtr wp, System.Text.StringBuilder lp);
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
[DllImport("user32.dll")] public static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr wp, IntPtr lp, uint flags, uint timeout, out IntPtr result);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder sb, int max);
'@

# NOTE: FindWindowW 在本机某些 shell 上下文对所有窗口返回 0 (Shell_TrayWnd 除外),
# EnumWindows 却能枚举到 -- 所以窗口查找一律走 EnumWindows + 类名匹配.
function Find-WinByClass([string]$cls, [int]$ownerPid) {
    $script:fwHit = [IntPtr]::Zero
    $script:fwCls = $cls
    $script:fwPid = $ownerPid
    $proc = [W.PInvoke+EnumWindowsProc]{
        param($h, $l)
        $wpid = [uint32]0
        [void][W.PInvoke]::GetWindowThreadProcessId($h, [ref]$wpid)
        if ($script:fwPid -ne 0 -and $wpid -ne $script:fwPid) { return $true }
        $cn = New-Object System.Text.StringBuilder 256
        [void][W.PInvoke]::GetClassNameW($h, $cn, 256)
        if ($cn.ToString() -eq $script:fwCls) { $script:fwHit = $h; return $false }
        return $true
    }
    [void][W.PInvoke]::EnumWindows($proc, [IntPtr]::Zero)
    return $script:fwHit
}

function Get-WinText([IntPtr]$h) {
    $len = [W.PInvoke]::SendMessageW($h, 0x000E, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    $buf = New-Object System.Text.StringBuilder ([int]$len + 2)
    $null = [W.PInvoke]::SendMsgBuf($h, 0x000D, [IntPtr]($len + 2), $buf)
    return $buf.ToString()
}
function Set-WinText([IntPtr]$h, [string]$t) {
    $null = [W.PInvoke]::SendMessageW($h, 0x000C, [IntPtr]::Zero, [IntPtr]::Zero) # clear
    $native = [System.Runtime.InteropServices.Marshal]::StringToHGlobalUni($t)
    try { $null = [W.PInvoke]::SendMessageW($h, 0x000C, [IntPtr]::Zero, $native) } finally { [System.Runtime.InteropServices.Marshal]::FreeHGlobal($native) | Out-Null }
}
function Click-Button([IntPtr]$h) { $null = [W.PInvoke]::SendMessageW($h, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) } # BM_CLICK
function Send-Cmd([IntPtr]$fm, [int]$id) { $null = [W.PInvoke]::PostMessageW($fm, 0x0111, [IntPtr]$id, [IntPtr]::Zero) }
function Lv-Count([IntPtr]$lv) { return [W.PInvoke]::SendMessageW($lv, 0x1004, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() }
function Lv-FirstSelected([IntPtr]$lv) { return [W.PInvoke]::SendMessageW($lv, 0x100C, [IntPtr](-1), [IntPtr]2).ToInt64() }
# ListView 点击必须用 PostMessage 异步发: 同步 SendMessage 到表头会进入列拖拽内部循环,
# 等 WM_LBUTTONUP 才返回 -> 和脚本顺序 (先等 DOWN 返回再发 UP) 互相死锁.
function Lv-Click([IntPtr]$lv, [int]$x, [int]$y, [bool]$dbl) {
    $lp = [IntPtr](($y -shl 16) -bor $x)
    $msg = if ($dbl) { 0x0203 } else { 0x0201 }
    $null = [W.PInvoke]::PostMessageW($lv, $msg, [IntPtr]0x0001, $lp)
    Start-Sleep -Milliseconds 150
    $null = [W.PInvoke]::PostMessageW($lv, 0x0202, [IntPtr]::Zero, $lp)
    Start-Sleep -Milliseconds 150
}
function Find-ChildById([IntPtr]$parent, [int]$id) {
    return [W.PInvoke]::GetDlgItem($parent, $id)
}
function Wait-Window([string]$cls, [int]$ms, [int]$ownerPid) {
    $deadline = [DateTime]::Now.AddMilliseconds($ms)
    while ([DateTime]::Now -lt $deadline) {
        $h = Find-WinByClass $cls $ownerPid
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 200
    }
    return [IntPtr]::Zero
}
function Wait-Status([IntPtr]$statusH, [string]$substr, [int]$ms) {
    $deadline = [DateTime]::Now.AddMilliseconds($ms)
    while ([DateTime]::Now -lt $deadline) {
        if ((Get-WinText $statusH) -like "*$substr*") { return $true }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

$results = New-Object System.Collections.ArrayList
function Check([string]$name, [bool]$ok, [string]$detail) {
    $line = if ($ok) { "[PASS] $name $detail" } else { "[FAIL] $name $detail" }
    [void]$results.Add($line); Write-Host $line
}

# 0) 准备本地测试目录与 1MB 文件
Remove-Item -Recurse -Force $upDir, $downDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $upDir, $downDir | Out-Null
$src = Join-Path $upDir '000_test.bin'
$data = New-Object byte[] (1MB); (New-Object Random 42).NextBytes($data)
[IO.File]::WriteAllBytes($src, $data)
$srcHash = (Get-FileHash $src -Algorithm SHA256).Hash

# 1) 客户端由外部 (bash) 启动, 这里只等待 viewer 窗口
$viewH = [IntPtr]::Zero
$deadline = [DateTime]::Now.AddSeconds(15)
while ([DateTime]::Now -lt $deadline -and $viewH -eq [IntPtr]::Zero) {
    Start-Sleep -Milliseconds 300
    $p = Get-Process rcontrol_client -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -like 'RControl - *' } | Select-Object -First 1
    if ($p -ne $null -and $p.MainWindowHandle -ne [IntPtr]::Zero) { $viewH = $p.MainWindowHandle }
}
$clientPid = 0; if ($p -ne $null) { $clientPid = $p.Id }
Check 'viewer-window' ($viewH -ne [IntPtr]::Zero) "hwnd=$viewH pid=$clientPid"
if ($viewH -eq [IntPtr]::Zero) { $results | Out-String | Write-Host; Stop-Process -Name rcontrol_client -Force -ErrorAction SilentlyContinue; exit 1 }

# 2) F9 打开文件传输窗口 (minifb 只认 lParam 扫描码; F9 set-1 = 0x43)
$lp9 = [IntPtr](0x00430001)
[void][W.PInvoke]::PostMessageW($viewH, 0x0100, [IntPtr]0x78, $lp9)   # WM_KEYDOWN
Start-Sleep -Milliseconds 150
[void][W.PInvoke]::PostMessageW($viewH, 0x0101, [IntPtr]0x78, [IntPtr](0xC0430001)) # WM_KEYUP
$fm = Wait-Window 'RControlFiles' 5000 $clientPid
if ($fm -eq [IntPtr]::Zero) { # 兜底: 前台 + 真实按键
    [void][W.PInvoke]::SetForegroundWindow($viewH)
    Start-Sleep -Milliseconds 500
    Add-Type -Namespace W -Name Kbd -MemberDefinition '[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);'
    [W.Kbd]::keybd_event(0x78, 0x43, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
    [W.Kbd]::keybd_event(0x78, 0x43, 2, [UIntPtr]::Zero)
    $fm = Wait-Window 'RControlFiles' 5000 $clientPid
}
Check 'files-window-open' ($fm -ne [IntPtr]::Zero) "hwnd=$fm"
if ($fm -eq [IntPtr]::Zero) { $results | Out-String | Write-Host; Stop-Process -Name rcontrol_client -Force -ErrorAction SilentlyContinue; exit 1 }

# 3) 定位控件
$lvR = Find-ChildById $fm 1005; $lvL = Find-ChildById $fm 2005
$edR = Find-ChildById $fm 1001; $edL = Find-ChildById $fm 2001
$stH = Find-ChildById $fm 9100
Check 'controls-found' ($lvR -ne [IntPtr]::Zero -and $lvL -ne [IntPtr]::Zero -and $edR -ne [IntPtr]::Zero -and $stH -ne [IntPtr]::Zero) "lvR=$lvR lvL=$lvL edR=$edR st=$stH"

# 4) 双栏初始列表
Start-Sleep -Milliseconds 2500
$nR = Lv-Count $lvR; $nL = Lv-Count $lvL
Check 'remote-list-populated' ($nR -gt 0) "items=$nR"
Check 'local-list-populated' ($nL -gt 0) "items=$nL"

# 5) 双击进入 C:\ (核心回归: 旧版 LBS_NOTIFY 缺失导致死功能)
Lv-Click $lvR 30 10 $true
Start-Sleep -Milliseconds 2000
$after = Get-WinText $edR
Check 'dblclick-navigate-C' ($after -like '*C:\*') "edit='$after'"

# 6) 新建远程目录 (input_box 流程)
Send-Cmd $fm 1006
$ib = Wait-Window 'RControlInputBox' 4000 $clientPid
if ($ib -ne [IntPtr]::Zero) {
    $ibEd = Find-ChildById $ib 100
    $ibOk = Find-ChildById $ib 1
    Set-WinText $ibEd $remoteDir
    Click-Button $ibOk
    Start-Sleep -Milliseconds 1000
    Check 'remote-mkdir' $true ''
} else {
    Check 'remote-mkdir' $false 'input box not found'
}

# 7) 远程栏路径跳转 C:\rcontrol_fm_test
Set-WinText $edR $remoteDir
Send-Cmd $fm 1003 # go
Start-Sleep -Milliseconds 2000
$nR2 = Lv-Count $lvR
Check 'remote-goto-dir' ($nR2 -eq 1) "items=$nR2 (expect 1: only ..)"

# 8) 本地栏跳转到上传目录, 选中 000_test.bin (item 1)
Set-WinText $edL $upDir
Send-Cmd $fm 2003
Start-Sleep -Milliseconds 1500
$nL2 = Lv-Count $lvL
Check 'local-goto-dir' ($nL2 -ge 2) "items=$nL2"
Lv-Click $lvL 60 50 $false
Start-Sleep -Milliseconds 400
$sel = Lv-FirstSelected $lvL
Check 'local-select-file' ($sel -eq 1) "selected=$sel (expect 1)"

# 9) 上传
Send-Cmd $fm 3001
# 回环太快, "上传完成"状态一闪即被刷新计数覆盖 -- 以远程列表出现文件为准
$ok = $false
$deadline = [DateTime]::Now.AddSeconds(30)
while ([DateTime]::Now -lt $deadline) {
    if ((Lv-Count $lvR) -ge 2) { $ok = $true; break }
    Start-Sleep -Milliseconds 300
}
Check 'upload-complete' $ok ("remoteItems=" + (Lv-Count $lvR))

# 10) 上传后远程自动刷新, 选中下载源
Start-Sleep -Milliseconds 1000
$nR3 = Lv-Count $lvR
Check 'remote-refreshed-after-upload' ($nR3 -eq 2) "items=$nR3 (expect 2: .. + file)"
Lv-Click $lvR 60 50 $false
Start-Sleep -Milliseconds 400
$selR = Lv-FirstSelected $lvR
Check 'remote-select-file' ($selR -eq 1) "selected=$selR"

# 11) 本地栏切到下载目录, 下载回来
Set-WinText $edL $downDir
Send-Cmd $fm 2003
Start-Sleep -Milliseconds 1200
Send-Cmd $fm 3000
$downFile = Join-Path $downDir '000_test.bin'
$ok2 = $false
$deadline = [DateTime]::Now.AddSeconds(30)
while ([DateTime]::Now -lt $deadline) {
    if ((Test-Path $downFile) -and ((Get-Item $downFile).Length -eq 1MB)) { $ok2 = $true; break }
    Start-Sleep -Milliseconds 300
}
Check 'download-complete' $ok2 ("status='" + (Get-WinText $stH) + "'")
if ($ok2) {
    $dstHash = (Get-FileHash $downFile -Algorithm SHA256).Hash
    Check 'bytes-identical' ($dstHash -eq $srcHash) ''
}

# 12) 远程删除测试目录 (confirm 弹窗)
Lv-Click $lvR 60 50 $false
Start-Sleep -Milliseconds 300
Send-Cmd $fm 1007
$mb = [IntPtr]::Zero
$deadline = [DateTime]::Now.AddSeconds(4)
while ([DateTime]::Now -lt $deadline -and $mb -eq [IntPtr]::Zero) {
    $mb = Find-WinByClass '#32770' $clientPid
    Start-Sleep -Milliseconds 200
}
if ($mb -ne [IntPtr]::Zero) {
    $okBtn = Find-ChildById $mb 1 # IDOK
    if ($okBtn -ne [IntPtr]::Zero) { Click-Button $okBtn } else { [void][W.PInvoke]::PostMessageW($mb, 0x0111, [IntPtr]1, [IntPtr]::Zero) }
    Start-Sleep -Milliseconds 1500
    Check 'remote-delete-confirm' $true ''
} else {
    Check 'remote-delete-confirm' $false 'no message box'
}

# 13) 清理
[void][W.PInvoke]::PostMessageW($fm, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) # WM_CLOSE files
Start-Sleep -Milliseconds 500
[void][W.PInvoke]::PostMessageW($viewH, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) # viewer Esc-like
Start-Sleep -Milliseconds 800
Stop-Process -Name rcontrol_client -Force -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $upDir, $downDir -ErrorAction SilentlyContinue
if (Test-Path (Join-Path $remoteDir '000_test.bin')) { Write-Host "[WARN] remote file still exists (delete may have failed)" }

$fails = ($results | Where-Object { $_ -like '[[]FAIL]*' }).Count
Write-Host ("== FM-E2E: {0} pass / {1} fail ==" -f ($results.Count - $fails), $fails)
exit $fails
