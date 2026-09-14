﻿# 清单管理 E2E (不依赖截屏): 预置 hosts.txt -> GUI 读取 -> 新增/更新/删除 -> 验证持久化
# 前提: 无 rcontrol_client 在跑; 3444 worker 可选 (双击连接项才需要)
$ErrorActionPreference = 'Continue'
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class DE {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool SetWindowTextW(IntPtr h, string s);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
    // 跨进程 EDIT 文本: SetWindowText/GetWindowText 只动缓存, 必须走消息 (系统自动封送)
    [DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="SendMessageW")] public static extern IntPtr SendSet(IntPtr h, uint msg, IntPtr wp, string lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, EntryPoint="SendMessageW")] public static extern IntPtr SendGet(IntPtr h, uint msg, IntPtr wp, StringBuilder lp);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
"@
$root = "F:\AICode\ClaudeCode\RControl\rcontrol"
$exe = "$root\target\release\rcontrol_client.exe"
$hostsFile = "$root\target\release\hosts.txt"
$results = @()
function Check($name, $ok, $detail) {
    $script:results += ("[{0}] {1}  {2}" -f ($(if ($ok) {'PASS'} else {'FAIL'})), $name, $detail)
}

function Find-Win($pid_, $prefix) {
    $script:found = [IntPtr]::Zero
    $cb = {
        param($h, $lp)
        $p = [uint32]0
        [DE]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
        if ($p -eq $pid_ -and [DE]::IsWindowVisible($h)) {
            $sb = New-Object System.Text.StringBuilder 256
            [DE]::GetWindowTextW($h, $sb, 256) | Out-Null
            if ($sb.ToString().StartsWith($prefix)) { $script:found = [IntPtr]$h; return $false }
        }
        return $true
    }
    $null = [DE]::EnumWindows($cb, [IntPtr]::Zero)
    return $script:found
}
function Get-Text($h) {
    $sb = New-Object System.Text.StringBuilder 256
    [DE]::SendGet($h, 0x000D, [IntPtr]256, $sb) | Out-Null
    return $sb.ToString()
}
function Set-Text($h, $s) {
    [DE]::SendSet($h, 0x000C, [IntPtr]::Zero, $s) | Out-Null
}
# ListView 消息
$LVM_GETITEMCOUNT = 0x1004
$LVM_GETITEMTEXTW = 0x1073
$LVIF_TEXT = 1
# 按控件 ID 点按钮 (WM_COMMAND: wp = 低16位 ID)
function Click-Btn($dlg, $id) {
    [DE]::PostMessageW($dlg, 0x0111, [IntPtr]$id, [IntPtr]::Zero) | Out-Null
}
# 读 ListView 某行某列 (跨进程: LVITEM 结构在目标进程内分配, 本机同位数可直接传指针不可靠 -> 用列数+首列文本仅验证行数)
function Lv-Count($lv) {
    return [DE]::SendMessageW($lv, $LVM_GETITEMCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
}

Stop-Process -Name rcontrol_client -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

# 0) 预置清单
@(
"# RControl test",
"PC-A`t192.168.0.10:3333`tpwd-a`t0",
"PC-B`t192.168.0.11`tpwd-b`t1"
) | Set-Content -Encoding UTF8 $hostsFile

# 1) 启动 GUI
$p = Start-Process $exe -PassThru
Start-Sleep -Seconds 2
$dlg = Find-Win $p.Id "RControl"
Check 'gui-open' ($dlg -ne [IntPtr]::Zero) "hwnd=$dlg"
if ($dlg -eq [IntPtr]::Zero) { $results | Out-String | Write-Host; Stop-Process -Id $p.Id -Force; exit 1 }

$edName = [DE]::GetDlgItem($dlg, 111)
$edAddr = [DE]::GetDlgItem($dlg, 112)
$edPwd  = [DE]::GetDlgItem($dlg, 113)
$chk    = [DE]::GetDlgItem($dlg, 114)
$lv     = [DE]::GetDlgItem($dlg, 110)
$st     = [DE]::GetDlgItem($dlg, 905)
Check 'controls' ($edName -ne [IntPtr]::Zero -and $edAddr -ne [IntPtr]::Zero -and $lv -ne [IntPtr]::Zero -and $st -ne [IntPtr]::Zero) "name=$edName addr=$edAddr lv=$lv st=$st"

# 2) 列表显示 2 行
Start-Sleep -Milliseconds 400
$n = Lv-Count $lv
Check 'list-load' ($n -eq 2) "rows=$n (expect 2)"

# 3) 点击列表第 1 行 (表头~21px 行高20px, y=31 -> item0=PC-A) -> 表单填充
[DE]::PostMessageW($lv, 0x0201, [IntPtr]0x1, [IntPtr](16 -bor (31 -shl 16))) | Out-Null  # WM_LBUTTONDOWN
Start-Sleep -Milliseconds 60
[DE]::PostMessageW($lv, 0x0202, [IntPtr]0x0, [IntPtr](16 -bor (31 -shl 16))) | Out-Null  # WM_LBUTTONUP
Start-Sleep -Milliseconds 500
$tName = Get-Text $edName; $tAddr = Get-Text $edAddr; $tPwd = Get-Text $edPwd
Check 'click-fills-form' ($tName -eq 'PC-A' -and $tAddr -eq '192.168.0.10:3333' -and $tPwd -eq 'pwd-a') "name=[$tName] addr=[$tAddr] pwd=[$tPwd]"

# 4) 新增: 填 PC-C -> 点新增 (ID 2) -> hosts.txt 3 行 + 列表 3 行
Set-Text $edName "PC-C"
Set-Text $edAddr "192.168.0.12"   # 不带端口, 应自动补 :21118
Set-Text $edPwd "pwd-c"
Click-Btn $dlg 2
Start-Sleep -Milliseconds 800
$n = Lv-Count $lv
$lines = (Get-Content $hostsFile -ErrorAction SilentlyContinue | Where-Object { $_ -and -not $_.StartsWith('#') })
$last = $lines | Select-Object -Last 1
Check 'add-row' ($n -eq 3) "rows=$n (expect 3)"
Check 'add-saved' ($lines.Count -eq 3 -and $last -like "PC-C`t192.168.0.12:21118`tpwd-c`t0") "file line=[$last]"

# 5) 更新: 选中第 3 行 (y=90) -> 改名 -> 更新 (ID 3)
[DE]::PostMessageW($lv, 0x0201, [IntPtr]0x1, [IntPtr](16 -bor (70 -shl 16))) | Out-Null
Start-Sleep -Milliseconds 60
[DE]::PostMessageW($lv, 0x0202, [IntPtr]::0, [IntPtr](16 -bor (70 -shl 16))) | Out-Null
Start-Sleep -Milliseconds 500
$t = Get-Text $edName
Check 'select-3rd' ($t -eq 'PC-C') "form name=[$t]"
Set-Text $edName "PC-C2"
Click-Btn $dlg 3
Start-Sleep -Milliseconds 800
$lines = (Get-Content $hostsFile | Where-Object { $_ -and -not $_.StartsWith('#') })
Check 'update-saved' (($lines | Where-Object { $_ -like "PC-C2`t*" }).Count -eq 1 -and $lines.Count -eq 3) "lines=$($lines.Count)"

# 6) 删除: 仍选中第 3 行 -> 点删除 (ID 4) -> 确认框点是 -> 剩 2 行
Click-Btn $dlg 4
Start-Sleep -Milliseconds 700
# 找 MessageBox (#32770) 属于本进程
$mb = Find-Win $p.Id "确认"
if ($mb -eq [IntPtr]::Zero) {
    # 宽限: 再等
    Start-Sleep -Milliseconds 500
    $mb = Find-Win $p.Id "确认"
}
Check 'delete-confirm-box' ($mb -ne [IntPtr]::Zero) "mb=$mb"
if ($mb -ne [IntPtr]::Zero) {
    # IDYES 按钮控件 (id=6) 发 BM_CLICK
    $yes = [DE]::GetDlgItem($mb, 6)
    if ($yes -ne [IntPtr]::Zero) { [DE]::PostMessageW($yes, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null }
    Start-Sleep -Milliseconds 800
}
$lines = (Get-Content $hostsFile | Where-Object { $_ -and -not $_.StartsWith('#') })
$n = Lv-Count $lv
Check 'delete-saved' ($lines.Count -eq 2 -and $n -eq 2) "file=$($lines.Count) rows=$n"

# 7) 校验逻辑: 空名称新增 -> 状态栏提示 (hosts 仍 2 行)
Set-Text $edName ""
Set-Text $edAddr "1.2.3.4"
Click-Btn $dlg 2
Start-Sleep -Milliseconds 600
$sTxt = Get-Text $st
$lines = (Get-Content $hostsFile | Where-Object { $_ -and -not $_.StartsWith('#') })
Check 'validate-empty-name' ($lines.Count -eq 2 -and $sTxt.Length -gt 0) "status=[$sTxt]"

# 8) WM_CLOSE 退出
[DE]::PostMessageW($dlg, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
Start-Sleep -Milliseconds 800
$alive = -not $p.HasExited
Check 'gui-close' (-not $alive) "exited=$(-not $alive)"
if ($alive) { Stop-Process -Id $p.Id -Force }

$results | Out-String | Write-Host
$fail = ($results | Where-Object { $_ -like '[FAIL]*' }).Count
Write-Host ("=== {0} / {1} pass ===" -f ($results.Count - $fail), $results.Count)
if ($fail -gt 0) { exit 1 }
