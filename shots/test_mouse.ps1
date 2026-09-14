param(
    [int]$BufW = 1920,
    [int]$BufH = 1080,
    [int]$ScrW = 3072,
    [int]$ScrH = 1728,
    [int]$WinW = 1000,
    [int]$WinH = 700,
    [switch]$Click
)
# 端到端鼠标映射验证: 把光标放到 viewer 客户区若干点, 读回注入后的屏幕光标位置
# 期望 = 客户区坐标 --(letterbox 反算)--> 帧坐标 --(服务端 frame->screen)--> 屏幕坐标
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class U {
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref P p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out P p);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool rep);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  public struct R { public int L, T, Rt, B; }
  public struct P { public int x, y; }
}
"@
$p = Get-Process rcontrol_client -ErrorAction Stop
$h = $p.MainWindowHandle
[U]::MoveWindow($h, 60, 60, $WinW, $WinH, $true) | Out-Null
Start-Sleep -Milliseconds 800

$cr = New-Object U+R
[U]::GetClientRect($h, [ref]$cr) | Out-Null
$cw = $cr.Rt; $ch = $cr.B
$o = New-Object U+P; $o.x = 0; $o.y = 0
[U]::ClientToScreen($h, [ref]$o) | Out-Null
Write-Output ("client={0}x{1} origin=({2},{3}) buffer={4}x{5} screen={6}x{7}" -f $cw,$ch,$o.x,$o.y,$BufW,$BufH,$ScrW,$ScrH)

# 与 view.rs displayed_rect 相同的整数布局计算
$ba = [double]$BufW / $BufH; $wa = [double]$cw / $ch
if ($ba -gt $wa) {
    $dh = [math]::Floor($cw / $ba); $dw = $cw
    $xo = 0; $yo = [math]::Truncate(($dh - $ch) / -2)
} else {
    $dw = [math]::Floor($ch * $ba); $dh = $ch
    $xo = [math]::Truncate(($dw - $cw) / -2); $yo = 0
}
Write-Output ("display rect: offset=({0},{1}) size={2}x{3}" -f $xo,$yo,$dw,$dh)

$points = @(
    @{fx=0.50; fy=0.50},
    @{fx=0.25; fy=0.25},
    @{fx=0.75; fy=0.75},
    @{fx=0.10; fy=0.90},
    @{fx=0.90; fy=0.10}
)
$bad = 0
foreach ($t in $points) {
    $px = [int]($cw * $t.fx); $py = [int]($ch * $t.fy)
    $sx = $o.x + $px; $sy = $o.y + $py
    [U]::SetCursorPos($sx, $sy) | Out-Null
    Start-Sleep -Milliseconds 500
    $cur = New-Object U+P
    [U]::GetCursorPos([ref]$cur) | Out-Null
    # 期望: 客户区 -> 缓冲 -> 屏幕
    $bx = ($px - $xo) * $BufW / $dw
    $by = ($py - $yo) * $BufH / $dh
    $ex = [math]::Round($bx * $ScrW / $BufW)
    $ey = [math]::Round($by * $ScrH / $BufH)
    $dx = [math]::Abs($cur.x - $ex); $dy = [math]::Abs($cur.y - $ey)
    $ok = ($dx -le 3 -and $dy -le 3)
    if (-not $ok) { $bad++ }
    Write-Output ("client({0,4},{1,4}) -> expect screen({2,4},{3,4}) actual({4,4},{5,4}) d=({6},{7}) {8}" -f $px,$py,$ex,$ey,$cur.x,$cur.y,$dx,$dy,$(if($ok){"OK"}else{"** FAIL **"}))
    if ($Click) {
        [U]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)  # LEFTDOWN
        Start-Sleep -Milliseconds 60
        [U]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)  # LEFTUP
        Start-Sleep -Milliseconds 250
        [U]::GetCursorPos([ref]$cur) | Out-Null
        $dx2 = [math]::Abs($cur.x - $ex); $dy2 = [math]::Abs($cur.y - $ey)
        Write-Output ("  after click: ({0},{1}) d=({2},{3})" -f $cur.x,$cur.y,$dx2,$dy2)
    }
}
Write-Output ("RESULT: {0} fail" -f $bad)
exit $bad
