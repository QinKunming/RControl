param([string]$Png = "shots/view_1to1.png")
Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Png))
Write-Output "--- row y=300, x=3040..3071 ---"
for ($x = 3040; $x -lt 3072; $x++) {
    $c = $bmp.GetPixel($x, 300)
    Write-Host -NoNewline ("{0}:({1},{2},{3}) " -f $x, $c.R, $c.G, $c.B)
}
Write-Output ""
Write-Output "--- col x=1500, y=1690..1727 ---"
for ($y = 1690; $y -lt 1728; $y++) {
    $c = $bmp.GetPixel(1500, $y)
    Write-Host -NoNewline ("{0}:({1},{2},{3}) " -f $y, $c.R, $c.G, $c.B)
}
Write-Output ""
Write-Output "--- col x=5, y=60..90 (window left border) ---"
for ($y = 60; $y -lt 90; $y++) {
    $c = $bmp.GetPixel(5, $y)
    Write-Host -NoNewline ("{0}:({1},{2},{3}) " -f $y, $c.R, $c.G, $c.B)
}
Write-Output ""
$bmp.Dispose()
