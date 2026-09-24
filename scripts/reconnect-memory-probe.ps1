param([int]$Cycles = 1000, [int]$SoakHours = 0)

$ErrorActionPreference = 'Stop'
if ($Cycles -lt 1 -or $Cycles -gt 1000) { throw 'Cycles must be between 1 and 1000' }
if ($SoakHours -lt 0 -or $SoakHours -gt 24) { throw 'SoakHours must be between 0 and 24' }
$binary = Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot '..\target\release\deps') -Filter 'supervisor-*.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (!$binary) { throw 'Build the release supervisor test before running this probe' }
if ($SoakHours -gt 0) {
    $env:TUNNELWARDEN_SOAK_HOURS = "$SoakHours"
    Remove-Item Env:TUNNELWARDEN_RECONNECT_CYCLES -ErrorAction SilentlyContinue
} else {
    $env:TUNNELWARDEN_RECONNECT_CYCLES = "$Cycles"
    Remove-Item Env:TUNNELWARDEN_SOAK_HOURS -ErrorAction SilentlyContinue
}
$output = Join-Path $env:TEMP "tunnelwarden-reconnect-$PID.out"
$errors = Join-Path $env:TEMP "tunnelwarden-reconnect-$PID.err"
$samplePath = Join-Path $env:TEMP "tunnelwarden-resources-$PID.csv"
$test = Start-Process -FilePath $binary.FullName -ArgumentList 'reconnect_keeps_listener_and_restores_forwarded_traffic','--nocapture' -WindowStyle Hidden -PassThru -RedirectStandardOutput $output -RedirectStandardError $errors
try {
    $samples = [System.Collections.Generic.List[object]]::new()
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $interval = if ($SoakHours -gt 0) { 60 } else { 5 }
    $limit = if ($SoakHours -gt 0) { [TimeSpan]::FromHours($SoakHours).Add([TimeSpan]::FromMinutes(10)) } else { [TimeSpan]::FromMinutes(15) }
    while (!$test.HasExited) {
        if ($watch.Elapsed -gt $limit) { throw 'Test exceeded its expected duration' }
        $process = Get-Process -Id $test.Id -ErrorAction SilentlyContinue
        if ($process) {
            $sockets = @(Get-NetTCPConnection -OwningProcess $test.Id -ErrorAction SilentlyContinue).Count
            $samples.Add([pscustomobject]@{
                Seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1)
                WorkingSetMB = [math]::Round($process.WorkingSet64 / 1MB, 1)
                PrivateMB = [math]::Round($process.PrivateMemorySize64 / 1MB, 1)
                Handles = $process.HandleCount
                Threads = $process.Threads.Count
                TcpSockets = $sockets
            })
        }
        Start-Sleep -Seconds $interval
        $test.Refresh()
    }
    $test.WaitForExit()
    $samples | Export-Csv -LiteralPath $samplePath -NoTypeInformation
    if ($SoakHours -gt 0) {
        $samples | Select-Object -First 5 | Format-Table -AutoSize
        $samples | Select-Object -Last 5 | Format-Table -AutoSize
    } else {
        $samples | Format-Table -AutoSize
    }
    Write-Output "Resource samples: $samplePath"
    Get-Content -LiteralPath $output
    Get-Content -LiteralPath $errors
    if ($test.ExitCode -ne 0) { throw "Reconnect stress failed with exit code $($test.ExitCode)" }
} finally {
    if (!$test.HasExited) { $test.Kill(); $test.WaitForExit() }
}
