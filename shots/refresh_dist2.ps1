# Refresh dist: server from target_test (target/release locked by running worker)
$src = Split-Path $PSScriptRoot -Parent
$dist = Join-Path $src 'dist'
foreach ($d in Get-ChildItem $dist -Directory) {
    if ($d.Name -match '2019') {
        Copy-Item (Join-Path $src 'target_test\release\rcontrol_server.exe') $d.FullName -Force
    } elseif ($d.Name -match 'Win10') {
        Copy-Item (Join-Path $src 'target\release\rcontrol_client.exe') $d.FullName -Force
    } elseif ($d.Name -match 'Win7') {
        Copy-Item (Join-Path $src 'target_win7\release\rcontrol_client.exe') $d.FullName -Force
    }
}
Get-ChildItem $dist -Recurse -Filter *.exe | ForEach-Object {
    '{0}  {1} bytes  {2}' -f $_.FullName, $_.Length, $_.LastWriteTime
}
