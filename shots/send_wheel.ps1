# Post wheel events to the viewer window, then read back title (scroll state)
param([int]$Notches = 3)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Wheel {
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
}
"@
$p = Get-Process rcontrol_client -ErrorAction Stop
$h = $p.MainWindowHandle
# WM_MOUSEWHEEL = 0x020A; wParam = delta << 16; lParam = (y << 16) | x  (screen coords)
for ($i = 0; $i -lt $Notches; $i++) {
    $w = [IntPtr]((120) -shl 16)
    $lparam = [IntPtr](((1000) -shl 16) -bor 1500)
    [Wheel]::PostMessage($h, 0x020A, $w, $lparam) | Out-Null
    Start-Sleep -Milliseconds 150
}
Start-Sleep -Milliseconds 600
Write-Output ("title: " + $p.MainWindowTitle)
