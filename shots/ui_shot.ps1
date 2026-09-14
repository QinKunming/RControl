# 截取主控端/被控端 GUI 窗口截图用于排版视觉验证
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class WinEnum {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

function Find-WinByTitle($procId, $prefix) {
    $found = [IntPtr]::Zero
    $cb = {
        param($h, $lp)
        $pid2 = 0
        [WinEnum]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
        if ($pid2 -eq $procId -and [WinEnum]::IsWindowVisible($h)) {
            $sb = New-Object System.Text.StringBuilder 256
            [WinEnum]::GetWindowTextW($h, $sb, 256) | Out-Null
            if ($sb.ToString().StartsWith($prefix)) { $script:found = [IntPtr]$h; return $false }
        }
        return $true
    }
    $null = [WinEnum]::EnumWindows($cb, [IntPtr]::Zero)
    return $script:found
}

function Shot($h, $file) {
    $r = New-Object WinEnum+RECT
    [WinEnum]::GetWindowRect([IntPtr]$h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    [WinEnum]::SetForegroundWindow([IntPtr]$h) | Out-Null
    Start-Sleep -Milliseconds 600
    $bmp = New-Object System.Drawing.Bitmap $w, $ht
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size $w, $ht))
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "saved: $file ($w x $ht)"
}

$root = "F:\AICode\ClaudeCode\RControl\rcontrol"

# ---- 主控端 ----
# 预置示例清单, 展示多台被控端的排版
@"
# RControl 被控端清单 (名称/地址/口令/只读)
本机测试	127.0.0.1:3333	Fe8r6ZvEK5	0
办公室主机	192.168.0.94		0
机房服务器A	192.168.1.10:21118	example-pwd	1
"@ | Set-Content -Encoding UTF8 "$root\target\release\hosts.txt"

$p1 = Start-Process "$root\target\release\rcontrol_client.exe" -PassThru
Start-Sleep -Seconds 2
$h1 = Find-WinByTitle $p1.Id "RControl 主控端"
if ($h1 -ne [IntPtr]::Zero) { Shot $h1 "$root\shots\client_dlg.png" } else { Write-Host "CLIENT WINDOW NOT FOUND" }
Stop-Process -Id $p1.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 500

# ---- 被控端 GUI ----
$p2 = Start-Process "$root\target_srv\release\rcontrol_server.exe" -PassThru
Start-Sleep -Seconds 2
$h2 = Find-WinByTitle $p2.Id "RControl 被控端设置"
if ($h2 -ne [IntPtr]::Zero) { Shot $h2 "$root\shots\server_gui.png" } else { Write-Host "SERVER WINDOW NOT FOUND" }
Stop-Process -Id $p2.Id -Force -ErrorAction SilentlyContinue
Write-Host "done"
