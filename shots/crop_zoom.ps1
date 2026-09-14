param([string]$Png = "shots/view_1to1_b.png")
Add-Type -AssemblyName System.Drawing
$src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Png))

function CropZoom($x, $y, $w, $h, $zoom, $out) {
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.DrawImage($src, (New-Object System.Drawing.Rectangle(0, 0, $w, $h)), (New-Object System.Drawing.Rectangle($x, $y, $w, $h)), [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()
    $big = New-Object System.Drawing.Bitmap(($w * $zoom), ($h * $zoom))
    $g2 = [System.Drawing.Graphics]::FromImage($big)
    $g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
    $g2.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
    $g2.DrawImage($bmp, 0, 0, $w * $zoom, $h * $zoom)
    $g2.Dispose()
    $big.Save((Join-Path "shots" $out), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose(); $big.Dispose()
    Write-Output "saved $out"
}

# right edge strip: x 3040..3071 (32 wide), y 79..1710 -> zoom x6
CropZoom 3040 79 32 400 6 "crop_right.png"
# bottom strip: x 1400..1700, y 1700..1727
CropZoom 1400 1700 300 28 6 "crop_bottom.png"
# upper-left dialog area
CropZoom 140 230 640 440 2 "crop_dialog.png"
$src.Dispose()
