# Locate viewer window by exe name, print exact client-rect pixels (DPI aware, ASCII)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
  public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref P p);
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
  [StructLayout(LayoutKind.Sequential)] public struct P { public int X, Y; }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
}
"@
[void][W]::SetProcessDPIAware()
Add-Type -AssemblyName System.Drawing
$targets = @{}
foreach ($p in Get-Process rcontrol_client -ErrorAction SilentlyContinue) { $targets[$p.Id] = $p.Id }
$script:hits = @()
$cb = {
  param($h, $l)
  $pid2 = 0
  [void][W]::GetWindowThreadProcessId($h, [ref]$pid2)
  if ($targets.ContainsKey([int]$pid2) -and [W]::IsWindowVisible($h)) {
    $sb = New-Object System.Text.StringBuilder 256
    [void][W]::GetWindowTextW($h, $sb, 256)
    if ($sb.ToString().StartsWith("RControl - ")) { $script:hits += $h }
  }
  return $true
}
[W]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null
foreach ($h in $script:hits) {
  $r = New-Object W+R
  [void][W]::GetClientRect($h, [ref]$r)
  $p = New-Object W+P; $p.X = 0; $p.Y = 0
  [void][W]::ClientToScreen($h, [ref]$p)
  Write-Output ("hwnd=" + $h + " zoomed=" + [W]::IsZoomed($h) + " client=" + ($r.Rt-$r.L) + "x" + ($r.B-$r.T) + " at (" + $p.X + "," + $p.Y + ")")
  $bmp = New-Object System.Drawing.Bitmap 64, 64
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  # sample v-bar: client right edge minus 7
  $vx = $p.X + ($r.Rt-$r.L) - 7
  $vy = $p.Y + 300
  $g.CopyFromScreen($vx-2, $vy, 0, 0, (New-Object System.Drawing.Size 5,5))
  $s = "vbar(x" + $vx + ",y" + $vy + "): "
  for ($i=0; $i -lt 3; $i++) { $c = $bmp.GetPixel(2, $i+1); $s += $c.R.ToString() + "," }
  Write-Output $s
  # sample h-bar: client bottom minus 7
  $hx = $p.X + 1900
  $hy = $p.Y + ($r.B-$r.T) - 7
  $g.CopyFromScreen($hx, $hy-2, 0, 0, (New-Object System.Drawing.Size 5,5))
  $s = "hbar(x" + $hx + ",y" + $hy + "): "
  for ($i=0; $i -lt 3; $i++) { $c = $bmp.GetPixel($i+1, 2); $s += $c.R.ToString() + "," }
  Write-Output $s
  $g.Dispose(); $bmp.Dispose()
}
