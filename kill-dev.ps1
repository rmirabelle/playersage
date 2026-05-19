Get-Process playersage -ErrorAction SilentlyContinue | Stop-Process -Force
$pids = (Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue).OwningProcess
foreach ($procId in $pids) { if ($procId) { Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue } }
Write-Output 'cleaned'
