# 刷新 dist 四个 exe (源码目录运行)
$src = Split-Path $PSScriptRoot -Parent   # rcontrol/
$dist = Join-Path $src 'dist'
$dirs = Get-ChildItem $dist -Directory
foreach ($d in $dirs) {
    if ($d.Name -match '被控端') {
        if ($d.Name -match 'Win7') {
            Copy-Item (Join-Path $src 'target_win7\release\rcontrol_server.exe') $d.FullName -Force
        } else {
            Copy-Item (Join-Path $src 'target\release\rcontrol_server.exe') $d.FullName -Force
        }
    } else {
        if ($d.Name -match 'Win7') {
            Copy-Item (Join-Path $src 'target_win7\release\rcontrol_client.exe') $d.FullName -Force
        } else {
            Copy-Item (Join-Path $src 'target\release\rcontrol_client.exe') $d.FullName -Force
        }
    }
}
Get-ChildItem $dist -Recurse -Filter *.exe | ForEach-Object {
    '{0}  {1} bytes  {2}' -f $_.FullName, $_.Length, $_.LastWriteTime
}
