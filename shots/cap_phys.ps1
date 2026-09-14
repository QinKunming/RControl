# Physical-pixel capture of viewer right edge (DPI-aware, ASCII comments only)
Add-Type @"
using System.Runtime.InteropServices;
public class D { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }
"@
[void][D]::SetProcessDPIAware()
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$vw = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Write-Output ("screen bounds: " + $vw.Width + "x" + $vw.Height)
$bmp = New-Object System.Drawing.Bitmap $vw.Width, $vw.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
# Right-edge strip of the maximized viewer: x in [W-14, W-1] is the scrollbar
$W = $vw.Width
$grays = 0; $total = 0; $thumb = 0; $track = 0
for ($y = 200; $y -lt 1500; $y += 7) {
  for ($x = ($W - 14); $x -lt $W; $x++) {
    $c = $bmp.GetPixel($x, $y); $total++
    if ($c.R -eq $c.G -and $c.G -eq $c.B) {
      if ($c.R -ge 120 -and $c.R -le 200) { $thumb++ }
      if ($c.R -ge 45 -and $c.R -le 75) { $track++ }
      if ($c.R -ge 40 -and $c.R -le 220) { $grays++ }
    }
  }
}
Write-Output ("right strip 14px x y200..1500: gray=" + $grays + "/" + $total + " thumbpx=" + $thumb + " trackpx=" + $track)
# Sample line at y=500
$s = ""
for ($x = ($W - 16); $x -lt $W; $x++) { $c = $bmp.GetPixel($x, 500); $s += $c.R.ToString() + "," }
Write-Output ("y=500 tail: " + $s)
# A column in content area (x=W-200) should NOT be scrollbar gray everywhere
$g.Dispose(); $bmp.Dispose()
