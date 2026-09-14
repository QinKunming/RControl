# Win7 构建被控端的功能验证: 3444 私有 worker + selftest + 按 PID 清理
$ErrorActionPreference = 'Continue'
$exe = 'F:\AICode\ClaudeCode\RControl\rcontrol\target_win7\release\rcontrol_server.exe'
$cli = 'F:\AICode\ClaudeCode\RControl\rcontrol\target\release\rcontrol_client.exe'

$p = Start-Process $exe -ArgumentList '--worker','--user','--port','3444' -PassThru
Start-Sleep -Seconds 2
$listen = netstat -ano | Select-String ':3444\s.*LISTENING'
if (-not $listen) { Write-Output ("FAIL: no listener on 3444, worker alive=" + [bool](Get-Process -Id $p.Id -ErrorAction SilentlyContinue)); exit 1 }
Write-Output ("WORKER_PID=" + $p.Id)

& $cli --selftest 127.0.0.1:3444 test123
$rc = $LASTEXITCODE
Write-Output ("SELFTEST_EXIT=" + $rc)

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
Write-Output ("WORKER_KILLED, still alive=" + [bool](Get-Process -Id $p.Id -ErrorAction SilentlyContinue))
$u3 = netstat -ano | Select-String ':3333\s.*LISTENING'
Write-Output ("USER_3333_ALIVE=" + [bool]$u3)
exit $rc
