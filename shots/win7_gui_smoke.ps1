# Win7 exe GUI smoke: start settings window, verify, close
$exe = 'F:\AICode\ClaudeCode\RControl\rcontrol\target_win7\release\rcontrol_server.exe'
$p = Start-Process $exe -PassThru
Start-Sleep -Seconds 3
$proc = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
if (-not $proc) { Write-Output 'FAIL: GUI process died'; exit 1 }
$h = $proc.MainWindowHandle
$t = $proc.MainWindowTitle
Write-Output ("GUI alive, hwnd=" + $h + " title=[" + $t + "]")
$ok = $p.CloseMainWindow()
Start-Sleep -Seconds 2
$alive = [bool](Get-Process -Id $p.Id -ErrorAction SilentlyContinue)
Write-Output ("CloseMainWindow=" + $ok + " exited=" + (-not $alive))
if ($alive) { Stop-Process -Id $p.Id -Force }
if ($h -eq 0) { Write-Output 'WARN: no main window handle'; exit 2 }
exit 0
