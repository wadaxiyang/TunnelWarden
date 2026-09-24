param(
    [int]$Cycles = 1000,
    [int]$SoakHours = 0,
    [string]$LogDirectory = (Join-Path $PSScriptRoot '..\logs')
)

$ErrorActionPreference = 'Stop'
if ($Cycles -lt 1 -or $Cycles -gt 1000) { throw 'Cycles must be between 1 and 1000' }
if ($SoakHours -lt 0 -or $SoakHours -gt 24) { throw 'SoakHours must be between 0 and 24' }
$binary = Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot '..\target\release\deps') -Filter 'supervisor-*.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (!$binary) { throw 'Build the release supervisor test before running this probe' }

$runName = 'reconnect-{0}-{1}-{2}' -f $(if ($SoakHours -gt 0) { "${SoakHours}h" } else { "${Cycles}cycles" }), (Get-Date -Format 'yyyyMMdd-HHmmss'), $PID
$logRoot = (New-Item -ItemType Directory -Path $LogDirectory -Force).FullName
$runDirectory = Join-Path $logRoot $runName
New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
$stdoutPath = Join-Path $runDirectory 'test.stdout.log'
$stderrPath = Join-Path $runDirectory 'test.stderr.log'
$samplePath = Join-Path $runDirectory 'resources.csv'
$statusPath = Join-Path $runDirectory 'status.log'
'Timestamp,Seconds,WorkingSetMB,PrivateMB,Handles,Threads,TcpSockets' | Set-Content -LiteralPath $samplePath -Encoding utf8

function Write-Status([string]$message) {
    $line = '{0} {1}' -f (Get-Date -Format 'yyyy-MM-ddTHH:mm:ssK'), $message
    Add-Content -LiteralPath $statusPath -Value $line -Encoding utf8
    Write-Output $line
}

if ($SoakHours -gt 0) {
    $env:TUNNELWARDEN_SOAK_HOURS = "$SoakHours"
    Remove-Item Env:TUNNELWARDEN_RECONNECT_CYCLES -ErrorAction SilentlyContinue
} else {
    $env:TUNNELWARDEN_RECONNECT_CYCLES = "$Cycles"
    Remove-Item Env:TUNNELWARDEN_SOAK_HOURS -ErrorAction SilentlyContinue
}
Remove-Item Env:TUNNELWARDEN_SOAK_SMOKE -ErrorAction SilentlyContinue
Write-Status "Starting $($binary.FullName); soak hours=$SoakHours; cycles=$Cycles"
Write-Status "Logs: $runDirectory"

$test = $null
$stdoutStream = $null
$stderrStream = $null
$stdoutTask = $null
$stderrTask = $null
$result = 'failed'
try {
    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $binary.FullName
    $startInfo.Arguments = 'reconnect_keeps_listener_and_restores_forwarded_traffic --nocapture'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $test = New-Object System.Diagnostics.Process
    $test.StartInfo = $startInfo
    if (!$test.Start()) { throw 'Could not start release supervisor test' }
    # A one-byte FileStream buffer makes CopyToAsync progress visible to a later reader.
    $stdoutStream = [System.IO.FileStream]::new($stdoutPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read, 1, $true)
    $stderrStream = [System.IO.FileStream]::new($stderrPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write, [System.IO.FileShare]::Read, 1, $true)
    $stdoutTask = $test.StandardOutput.BaseStream.CopyToAsync($stdoutStream)
    $stderrTask = $test.StandardError.BaseStream.CopyToAsync($stderrStream)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $interval = if ($SoakHours -gt 0) { 60 } else { 5 }
    $limit = if ($SoakHours -gt 0) { [TimeSpan]::FromHours($SoakHours).Add([TimeSpan]::FromMinutes(10)) } else { [TimeSpan]::FromMinutes(15) }
    $culture = [System.Globalization.CultureInfo]::InvariantCulture
    while (!$test.HasExited) {
        if ($watch.Elapsed -gt $limit) { throw 'Test exceeded its expected duration' }
        $process = Get-Process -Id $test.Id -ErrorAction SilentlyContinue
        if ($process) {
            $sockets = @(Get-NetTCPConnection -OwningProcess $test.Id -ErrorAction SilentlyContinue).Count
            $row = '{0},{1},{2},{3},{4},{5},{6}' -f (Get-Date -Format 'yyyy-MM-ddTHH:mm:ssK'), ([math]::Round($watch.Elapsed.TotalSeconds, 1).ToString($culture)), ([math]::Round($process.WorkingSet64 / 1MB, 1).ToString($culture)), ([math]::Round($process.PrivateMemorySize64 / 1MB, 1).ToString($culture)), $process.HandleCount, $process.Threads.Count, $sockets
            Add-Content -LiteralPath $samplePath -Value $row -Encoding utf8
        }
        Start-Sleep -Seconds $interval
        $test.Refresh()
    }
    $test.WaitForExit()
    [System.Threading.Tasks.Task]::WaitAll(@($stdoutTask, $stderrTask))
    $exitCode = $test.ExitCode
    Write-Status "Process exited: id=$($test.Id); exit code=$exitCode"
    if ($exitCode -ne 0) { throw "Reconnect stress failed with exit code $exitCode" }
    $result = 'passed'
    Write-Status "Passed after $([math]::Round($watch.Elapsed.TotalSeconds, 1)) seconds"
} catch {
    Write-Status "Failed: $($_.Exception.Message)"
    throw
} finally {
    if ($test -and !$test.HasExited) { $test.Kill(); $test.WaitForExit() }
    try {
        if ($stdoutTask -and $stderrTask) { [System.Threading.Tasks.Task]::WaitAll(@($stdoutTask, $stderrTask)) }
    } finally {
        if ($stdoutStream) { $stdoutStream.Dispose() }
        if ($stderrStream) { $stderrStream.Dispose() }
        if ($test) { $test.Dispose() }
        Write-Status "Finished: $result; stdout=$stdoutPath; stderr=$stderrPath; samples=$samplePath"
    }
}
