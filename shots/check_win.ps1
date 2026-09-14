# Check viewer window: title, zoomed state, rect (ASCII only)
Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class W {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr h, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out R r);
  public struct R { public int L, T, Rt, B; }
}
"@
$found = @()
$cb = {
  param($h, $l)
  $sb = New-Object System.Text.StringBuilder 256
  [void][W]::GetWindowTextW($h, $sb, 256)
  $t = $sb.ToString()
  if ($t -like "*RControl*" -and [W]::IsWindowVisible($h)) {
    $script:found += [pscustomobject]@{
      Hwnd = $h; Title = $t; Zoomed = [W]::IsZoomed($h)
      Wr = ""; Cr = ""
    }
  }
  return $true
}
[W]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
foreach ($f in $found) {
  $r = New-Object W+R
  [void][W]::GetWindowRect($f.Hwnd, [ref]$r)
  $f.Wr = "$($r.L),$($r.T) - $($r.Rt),$($r.B) ($($r.Rt-$r.L)x$($r.B-$r.T))"
  [void][W]::GetClientRect($f.Hwnd, [ref]$r)
  $f.Cr = "$($r.Rt)x$($r.B)"
  Write-Output ("TITLE: " + $f.Title)
  Write-Output ("  zoomed=" + $f.Zoomed + "  winrect=" + $f.Wr + "  client=" + $f.Cr)
}
