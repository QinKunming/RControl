# F9 probe: post F9, then report process liveness + all top-level windows of the process
$ErrorActionPreference = 'Stop'
Add-Type -Namespace W -Name P -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc cb, IntPtr l);
public delegate bool EnumWindowsProc(IntPtr h, IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder sb, int max);
[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder sb, int max);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
'@

$p = Get-Process rcontrol_client -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -like 'RControl - *' } | Select-Object -First 1
if ($p -eq $null) { Write-Host "PROBE no-viewer"; exit 1 }
$viewH = $p.MainWindowHandle
Write-Host "PROBE viewer=$viewH pid=$($p.Id)"

# F9 down/up (scan 0x43 in lParam)
[void][W.P]::PostMessageW($viewH, 0x0100, [IntPtr]0x78, [IntPtr](0x00430001))
Start-Sleep -Milliseconds 120
[void][W.P]::PostMessageW($viewH, 0x0101, [IntPtr]0x78, [IntPtr](0xC0430001))

Start-Sleep -Milliseconds 2500

$p2 = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
if ($p2 -eq $null) {
    Write-Host "PROBE process DEAD after F9"
    exit 2
}
Write-Host "PROBE alive mainwindow='$($p2.MainWindowTitle)'"

$proc = [W.P+EnumWindowsProc]{ param($h,$l) $wpid=[uint32]0; [void][W.P]::GetWindowThreadProcessId($h,[ref]$wpid); if($wpid -eq $l.ToInt64()){ $cn=New-Object System.Text.StringBuilder 256; [void][W.P]::GetClassNameW($h,$cn,256); $tt=New-Object System.Text.StringBuilder 256; [void][W.P]::GetWindowTextW($h,$tt,256); $vis=[W.P]::IsWindowVisible($h); Write-Host ("  WIN cls={0} title='{1}' vis={2} hwnd={3}" -f $cn,$tt,$vis,$h) }; $true }
[void][W.P]::EnumWindows($proc, [IntPtr]$p.Id)
Write-Host "PROBE done"
