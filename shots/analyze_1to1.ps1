param([string]$Png = "shots/view_1to1.png")
Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Png))
Write-Output ("bitmap {0}x{1}" -f $bmp.Width, $bmp.Height)

function ScanRow($y) {
    $trans = New-Object System.Collections.Generic.List[string]
    $prev = $bmp.GetPixel(0, $y)
    for ($x = 1; $x -lt $bmp.Width; $x++) {
        $c = $bmp.GetPixel($x, $y)
        if ([math]::Abs($c.R - $prev.R) -gt 40 -or [math]::Abs($c.G - $prev.G) -gt 40 -or [math]::Abs($c.B - $prev.B) -gt 40) {
            $trans.Add(("{0}:({1},{2},{3})->({4},{5},{6})" -f $x, $prev.R, $prev.G, $prev.B, $c.R, $c.G, $c.B))
        }
        $prev = $c
    }
    Write-Output ("y=$y : " + ($trans -join "  "))
}

function ScanCol($x) {
    $trans = New-Object System.Collections.Generic.List[string]
    $prev = $bmp.GetPixel($x, 0)
    for ($y = 1; $y -lt $bmp.Height; $y++) {
        $c = $bmp.GetPixel($x, $y)
        if ([math]::Abs($c.R - $prev.R) -gt 40 -or [math]::Abs($c.G - $prev.G) -gt 40 -or [math]::Abs($c.B - $prev.B) -gt 40) {
            $trans.Add(("{0}:({1},{2},{3})->({4},{5},{6})" -f $y, $prev.R, $prev.G, $prev.B, $c.R, $c.G, $c.B))
        }
        $prev = $c
    }
    Write-Output ("x=$x : " + ($trans -join "  "))
}

ScanRow 300
ScanRow 900
ScanCol 100
ScanCol 900
$bmp.Dispose()
