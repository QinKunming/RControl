param([string]$Png, [int]$X = 1500, [int]$Y0 = 100, [int]$Y1 = 1700, [int]$Th = 40)
Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Png))
$out = @()
$prev = $bmp.GetPixel($X, $Y0)
for ($y = $Y0 + 1; $y -lt $Y1; $y++) {
    $c = $bmp.GetPixel($X, $y)
    if ([math]::Abs($c.R - $prev.R) -gt $Th -or [math]::Abs($c.G - $prev.G) -gt $Th -or [math]::Abs($c.B - $prev.B) -gt $Th) {
        $out += $y
    }
    $prev = $c
}
$bmp.Dispose()
Write-Output ($out -join ",")
