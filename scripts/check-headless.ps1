param([Parameter(Mandatory)][string]$Executable)
$ErrorActionPreference = 'Stop'
# This starts the installed production identifier; use only a disposable runner.
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Headless installation check requires GitHub Actions' }
$binary = (Resolve-Path -LiteralPath $Executable).Path
$process = Start-Process -FilePath $binary -ArgumentList '--autostart' -WindowStyle Hidden -PassThru
try {
    Start-Sleep -Seconds 4
    if ($process.HasExited) { throw "Background application exited: $($process.ExitCode)" }
    $children = @(Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $process.Id })
    if ($children | Where-Object { $_.Name -eq 'msedgewebview2.exe' }) { throw 'Headless startup created WebView2' }
    Write-Host 'PASS: installed background runtime stays alive without a WebView2 child process.'
} finally {
    # No profiles/tunnels exist on this ephemeral runner. Stop our own smoke instance.
    if (-not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
    $process.Dispose()
}
