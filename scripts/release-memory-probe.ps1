param(
    [int]$Cycles = 30,
    [string]$Executable = (Join-Path $PSScriptRoot '..\target\release\tunnelwarden.exe')
)

$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class TunnelWardenProbe {
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
    public static IntPtr[] Windows(uint pid, bool visibleOnly) {
        var found = new System.Collections.Generic.List<IntPtr>();
        EnumWindows((hwnd, param) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner == pid && (!visibleOnly || IsWindowVisible(hwnd))) found.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        return found.ToArray();
    }
}
'@

function Wait-VisibleWindow([int]$ProcessId, [bool]$Expected) {
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $windows = [TunnelWardenProbe]::Windows([uint32]$ProcessId, $true)
        if (($windows.Count -gt 0) -eq $Expected) { return $windows }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for visible window state $Expected"
}

function Measure-Resources([int]$ProcessId, [int]$Cycle) {
    $process = Get-Process -Id $ProcessId -ErrorAction Stop
    $gpu = $null
    try {
        $gpu = (Get-Counter '\GPU Process Memory(*)\Local Usage' -ErrorAction Stop).CounterSamples |
            Where-Object { $_.InstanceName -match "pid_$ProcessId(?:_|$)" } |
            Measure-Object -Property CookedValue -Sum |
            Select-Object -ExpandProperty Sum
    } catch { }
    [pscustomobject]@{
        Cycle = $Cycle
        WorkingSetMB = [math]::Round($process.WorkingSet64 / 1MB, 1)
        PrivateMB = [math]::Round($process.PrivateMemorySize64 / 1MB, 1)
        Handles = $process.HandleCount
        Threads = $process.Threads.Count
        Windows = [TunnelWardenProbe]::Windows([uint32]$ProcessId, $false).Count
        GpuLocalMB = if ($null -eq $gpu) { $null } else { [math]::Round($gpu / 1MB, 1) }
    }
}

$resolved = (Resolve-Path -LiteralPath $Executable).Path
$primary = Start-Process -FilePath $resolved -PassThru
try {
    Wait-VisibleWindow $primary.Id $true | Out-Null
    Measure-Resources $primary.Id 0
    for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
        $windows = Wait-VisibleWindow $primary.Id $true
        foreach ($window in $windows) {
            [void][TunnelWardenProbe]::PostMessage($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        Wait-VisibleWindow $primary.Id $false | Out-Null
        $second = Start-Process -FilePath $resolved -WindowStyle Hidden -PassThru -Wait
        if ($second.ExitCode -ne 0) { throw "Second instance exited with $($second.ExitCode)" }
        Wait-VisibleWindow $primary.Id $true | Out-Null
        Start-Sleep -Milliseconds 250
        if ($cycle -eq 1 -or $cycle % 5 -eq 0 -or $cycle -eq $Cycles) {
            Measure-Resources $primary.Id $cycle
        }
    }
} finally {
    if (!$primary.HasExited) {
        $primary.Kill()
        $primary.WaitForExit()
    }
}
