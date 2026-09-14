# 列出 rcontrol_client 所有顶层窗口的句柄/类名/标题
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class LW {
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder sb, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
"@
$targets = Get-Process rcontrol_client -ErrorAction SilentlyContinue
if (-not $targets) { Write-Host "no client process"; exit }
foreach ($t in $targets) {
    Write-Host "=== pid $($t.Id) ==="
    $script:pid_ = $t.Id
    $cb = {
        param($h, $lp)
        $p = [uint32]0
        [LW]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
        if ($p -eq $script:pid_) {
            $sb = New-Object System.Text.StringBuilder 256
            [LW]::GetWindowTextW($h, $sb, 256) | Out-Null
            $cn = New-Object System.Text.StringBuilder 256
            [LW]::GetClassNameW($h, $cn, 256) | Out-Null
            $vis = [LW]::IsWindowVisible($h)
            Write-Host ("  hwnd=$h class=$($cn.ToString()) vis=$vis title=[$($sb.ToString())]")
        }
        return $true
    }
    $null = [LW]::EnumWindows($cb, [IntPtr]::Zero)
}
