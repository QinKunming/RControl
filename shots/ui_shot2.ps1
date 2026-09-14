# 按 GetWindowRect 精确裁剪 GUI 窗口截图 (UTF-8 BOM, PS5.1 中文安全)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class W2 {
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

function Find-Win($procId, $prefix) {
    $script:found = [IntPtr]::Zero
    $cb = {
        param($h, $lp)
        $pid2 = [uint32]0
        [W2]::GetWindowThreadProcessId($h, [ref]$pid2) | Out-Null
        if ($pid2 -eq $procId -and [W2]::IsWindowVisible($h)) {
            $sb = New-Object System.Text.StringBuilder 256
            [W2]::GetWindowTextW($h, $sb, 256) | Out-Null
            if ($sb.ToString().StartsWith($prefix)) { $script:found = [IntPtr]$h; return $false }
        }
        return $true
    }
    $null = [W2]::EnumWindows($cb, [IntPtr]::Zero)
    return $script:found
}

function Shot($h, $file) {
    $r = New-Object W2+RECT
    [W2]::GetWindowRect([IntPtr]$h, [ref]$r) | Out-Null
    Write-Host "rect: $($r.L),$($r.T) - $($r.R),$($r.B)"
    # SetForeground 省略 (窗口新启动已在前景)
    Start-Sleep -Milliseconds 700
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $ok = $false
    for ($i = 0; $i -lt 3 -and -not $ok; $i++) {
        try { $g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size); $ok = $true } catch { Start-Sleep -Milliseconds 400 }
    }
    if (-not $ok) { Write-Host "COPY FAILED"; $bmp.Dispose(); return }
    $g.Dispose()
    # 全屏坐标 -> 裁剪窗口矩形 (PS 非 DPI aware, GetWindowRect 返回逻辑像素, 与截图同域)
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($w -gt 0 -and $ht -gt 0) {
        $rect = New-Object System.Drawing.Rectangle($r.L, $r.T, $w, $ht)
        $crop = $bmp.Clone($rect, $bmp.PixelFormat)
        $crop.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
        $crop.Dispose()
        Write-Host "saved: $file ($w x $ht)"
    }
    $bmp.Dispose()
}

Add-Type -AssemblyName System.Windows.Forms
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
