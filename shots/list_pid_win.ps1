Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
}
"@
$script:rows = @()
$cb = {
  param($h, $l)
  $p = 0
  [void][W]::GetWindowThreadProcessId($h, [ref]$p)
  if ([W]::IsWindowVisible($h)) {
    $r = New-Object W+R
    [void][W]::GetWindowRect($h, [ref]$r)
    if (($r.Rt-$r.L) -gt 400 -and ($r.B-$r.T) -gt 300) {
      $sb = New-Object System.Text.StringBuilder 256
      [void][W]::GetWindowTextW($h, $sb, 256)
      $proc = (Get-Process -Id $p -ErrorAction SilentlyContinue).ProcessName
      $script:rows += ("pid=" + $p + " " + $proc + " rect=" + $r.L + "," + $r.T + "-" + $r.Rt + "," + $r.B + " title=" + $sb.ToString())
    }
  }
  return $true
}
[W]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
$script:rows
