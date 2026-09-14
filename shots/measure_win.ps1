# 精确测量 viewer 窗口位置/尺寸 (PS DPI 不感知, 返回逻辑像素, x1.25=物理)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref P p);
  public struct R { public int L, T, Rt, B; }
  public struct P { public int x, y; }
}
"@
$p = Get-Process rcontrol_client -ErrorAction Stop
$h = $p.MainWindowHandle
$wr = New-Object W+R; [W]::GetWindowRect($h, [ref]$wr) | Out-Null
$cr = New-Object W+R; [W]::GetClientRect($h, [ref]$cr) | Out-Null
$o = New-Object W+P; $o.x = 0; $o.y = 0; [W]::ClientToScreen($h, [ref]$o) | Out-Null
Write-Output ("window rect: L={0} T={1} R={2} B={3}  ({4}x{5})" -f $wr.L,$wr.T,$wr.Rt,$wr.B,($wr.Rt-$wr.L),($wr.B-$wr.T))
Write-Output ("client size: {0}x{1}  origin=({2},{3})" -f $cr.Rt,$cr.B,$o.x,$o.y)
Write-Output ("physical estimate: client {0}x{1} @ ({2},{3})" -f ($cr.Rt*1.25),($cr.B*1.25),($o.x*1.25),($o.y*1.25))
