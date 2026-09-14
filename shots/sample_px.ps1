param([string]$Png = "shots/view_1to1.png")
Add-Type -AssemblyName System.Drawing
$bmp = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Png))
$pts = @(
    @(3058, 300),   # v-scrollbar track expected (dark gray ~58,58,58)
    @(3058, 1600),  # v-scrollbar lower track
    @(3065, 300),   # right of scrollbar / outside window
    @(1500, 300),   # middle of image area
    @(100, 70),     # outer title bar
    @(1500, 70),    # outer title bar text zone
    @(55, 128),     # predicted nested title bar left edge (1:1)
    @(1500, 143),   # predicted nested title bar middle
    @(1500, 1690),  # near bottom: h-scrollbar expected (~y 1710)
    @(1500, 1715),  # h-scrollbar track expected
    @(1500, 1725),  # taskbar zone (outside window)
    @(30, 128),
    @(1500, 200)    # image area upper
)
foreach ($p in $pts) {
    $c = $bmp.GetPixel($p[0], $p[1])
    Write-Output ("({0},{1}) = ({2},{3},{4})" -f $p[0], $p[1], $c.R, $c.G, $c.B)
}
$bmp.Dispose()
