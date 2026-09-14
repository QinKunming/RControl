# List rcontrol processes and their visible top-level windows
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class EnumWin {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  public struct R { public int L, T, Rt, B; }
  public static List<string> Found = new List<string>();
  public static bool Cb(IntPtr h, IntPtr l) {
    if (IsWindowVisible(h)) {
      uint pid; GetWindowThreadProcessId(h, out pid);
      int len = GetWindowTextLength(h);
      if (len > 0) {
        var sb = new StringBuilder(len + 1);
        GetWindowText(h, sb, sb.Capacity);
        R r; GetWindowRect(h, out r);
        Found.Add(pid + "|" + h + "|" + r.L + "," + r.T + "," + r.Rt + "," + r.B + "|" + sb);
      }
    }
    return true;
  }
  public static void Run() { EnumWindows(Cb, IntPtr.Zero); }
}
"@
[EnumWin]::Run()
Get-Process | Where-Object { $_.Name -like "rcontrol*" } | ForEach-Object {
    Write-Output ("PROC {0} pid={1} title={2}" -f $_.Name, $_.Id, $_.MainWindowTitle)
}
Write-Output "--- windows ---"
[EnumWin]::Found | ForEach-Object {
    $parts = $_ -split '\|'
    $pid = [uint32]$parts[0]
    $proc = (Get-Process -Id $pid -ErrorAction SilentlyContinue).Name
    Write-Output ("{0,-18} pid={1,-7} rect={2,-24} title={3}" -f $proc, $pid, $parts[2], $parts[3])
}
