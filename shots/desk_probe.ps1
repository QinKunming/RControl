# 探测当前进程链的桌面环境: OpenInputDesktop + 桌面名 + 会话
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class DP {
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern bool GetUserObjectInformation(IntPtr h, int idx, StringBuilder sb, int n, out int needed);
    [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr h);
    [DllImport("kernel32.dll")] public static extern uint WTSGetActiveConsoleSessionId();
}
"@
$h = [DP]::OpenInputDesktop(0, $false, 0x01FF)
if ($h -eq [IntPtr]::Zero) {
    Write-Host ("OpenInputDesktop FAILED err=" + [Runtime.InteropServices.Marshal]::GetLastWin32Error())
} else {
    $sb = New-Object System.Text.StringBuilder 256
    $n = 0
    [DP]::GetUserObjectInformation($h, 2, $sb, 256, [ref]$n) | Out-Null
    Write-Host ("OpenInputDesktop OK desktop=" + $sb.ToString())
    [DP]::CloseDesktop($h) | Out-Null
}
Write-Host ("activeConsoleSession=" + [DP]::WTSGetActiveConsoleSessionId())
