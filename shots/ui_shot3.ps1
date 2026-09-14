# PrintWindow 直捕 GUI 窗口内容 (不依赖屏幕状态)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class W3 {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

function Find-Win($procId, $prefix) {
    $script:found = [IntPtr]::Zero
    $cb = {
        param($h, $lp)
        $pid2 = [uint32]0
        [W3]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
        if ($pid2 -eq $procId -and [W3]::IsWindowVisible($h)) {
            $sb = New-Object System.Text.StringBuilder 256
            [W3]::GetWindowTextW($h, $sb, 256) | Out-Null
            if ($sb.ToString().StartsWith($prefix)) { $script:found = [IntPtr]$h; return $false }
        }
        return $true
    }
    $null = [W3]::EnumWindows($cb, [IntPtr]::Zero)
    return $script:found
}

function Shot($h, $file) {
    $r = New-Object W3+RECT
    [W3]::GetClientRect([IntPtr]$h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    Write-Host "client: $w x $ht"
    if ($w -le 0 -or $ht -le 0) { Write-Host "BAD RECT"; return }
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $ok = [W3]::PrintWindow([IntPtr]$h, $hdc, 2)
    $g.ReleaseHdc($hdc)
    $g.Dispose()
    if ($ok) {
        $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
        Write-Host "saved: $file"
    } else {
        Write-Host "PRINTWINDOW FAILED"
    }
    $bmp.Dispose()
}

$root = "F:\AICode\ClaudeCode\RControl\rcontrol"

$p1 = Start-Process "$root\target\release\rcontrol_client.exe" -PassThru
Start-Sleep -Seconds 2
$h1 = Find-Win $p1.Id "RControl"
if ($h1 -ne [IntPtr]::Zero) { Shot $h1 "$root\shots\client_dlg.png" } else { Write-Host "CLIENT NOT FOUND" }
Stop-Process -Id $p1.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 600

$p2 = Start-Process "$root\target_srv\release\rcontrol_server.exe" -PassThru
Start-Sleep -Seconds 2
$h2 = Find-Win $p2.Id "RControl"
if ($h2 -ne [IntPtr]::Zero) { Shot $h2 "$root\shots\server_gui.png" } else { Write-Host "SERVER NOT FOUND" }
Stop-Process -Id $p2.Id -Force -ErrorAction SilentlyContinue
Write-Host done
