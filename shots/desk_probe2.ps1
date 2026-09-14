Add-Type @"
using System;using System.Runtime.InteropServices;
public class DP2 {
  [DllImport("user32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern IntPtr OpenDesktopW(string name, uint flags, bool inherit, uint access);
  [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr h);
}
"@
$h = [DP2]::OpenDesktopW("Default", 0, $false, 0x01FF)
if ($h -ne [IntPtr]::Zero) { Write-Host "Default desktop OPEN OK (=> 锁屏中: input 是 Winlogon)"; [DP2]::CloseDesktop($h) | Out-Null } else { Write-Host ("Default FAILED err=" + [Runtime.InteropServices.Marshal]::GetLastWin32Error() + " (=> window station 问题)") }
