# Physical capture: check BOTH scrollbar strips of maximized viewer (ASCII only)
Add-Type @"
using System.Runtime.InteropServices;
public class D { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
"@
[void][D]::SetProcessDPIAware()
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
$W = $b.Width; $H = $b.Height
# Vertical strip: x in [W-14, W-1], y=800
$vs = ""
for ($x = ($W - 14); $x -lt $W; $x++) { $c = $bmp.GetPixel($x, 800); $vs += $c.R.ToString() + " " }
Write-Output ("v-strip x3826-3839 y800: " + $vs)
# Horizontal strip: y in [H-48-14, H-48-1] (just above taskbar ~48px), x=1900
$hs = ""
for ($y = ($H - 62); $y -lt ($H - 48); $y++) { $c = $bmp.GetPixel(1900, $y); $hs += $c.R.ToString() + " " }
Write-Output ("h-strip y2098-2111 x1900: " + $hs)
# Scan up from bottom for the first content (non-gray) row to locate bar bottom edge
for ($y = ($H - 100); $y -lt $H; $y++) {
  $c = $bmp.GetPixel(1900, $y)
  $gray = ($c.R -eq $c.G -and $c.G -eq $c.B -and $c.R -ge 40 -and $c.R -le 220)
  if (-not $gray) { Write-Output ("first non-gray row from bottom scan at x1900: y=" + $y); break }
}
$g.Dispose(); $bmp.Dispose()
