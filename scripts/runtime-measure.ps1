#Requires -Version 5.1
param(
    [Parameter(Mandatory=$true)][string]$Animation,
    [int]$SecondsPerPhase=10,
    [string]$OutputDirectory='artifacts/runtime'
)
$ErrorActionPreference='Stop'
if ($SecondsPerPhase -lt 1 -or $SecondsPerPhase -gt 60) { throw 'Phase duration must be 1 to 60 seconds.' }
$root=Split-Path -Parent $PSScriptRoot
$exe=Join-Path $root 'target/release/kova-image.exe'
$file=(Resolve-Path -LiteralPath $Animation).Path
$null=New-Item -ItemType Directory -Force -Path $OutputDirectory
$output=(Resolve-Path -LiteralPath $OutputDirectory).Path
$ready=Join-Path $output ('ready-'+[guid]::NewGuid().ToString('N')+'.json')
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class KovaMeasurementWindow {
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
}
'@
$process=Start-Process -FilePath $exe -ArgumentList @('--measure', ('"'+$ready+'"'), '--', ('"'+$file+'"')) -PassThru -WindowStyle Hidden
$rows=@()
try {
    $deadline=[datetime]::UtcNow.AddSeconds(30)
    while (!(Test-Path -LiteralPath $ready)) {
        if ($process.HasExited -or [datetime]::UtcNow -gt $deadline) { throw 'Viewer did not become ready.' }
        Start-Sleep -Milliseconds 50
    }
    Start-Sleep -Seconds 2
    $process.Refresh()
    $window=$process.MainWindowHandle
    if ($window -eq [IntPtr]::Zero) { throw 'No viewer window found.' }
    foreach ($phase in @('playing','paused','minimized')) {
        if ($phase -eq 'paused') {
            $null=[KovaMeasurementWindow]::PostMessage($window,0x100,[IntPtr]0x20,[IntPtr]0)
            $null=[KovaMeasurementWindow]::PostMessage($window,0x101,[IntPtr]0x20,[IntPtr]0)
        }
        if ($phase -eq 'minimized') {
            # Resume first, then minimize: measure visibility throttling itself.
            $null=[KovaMeasurementWindow]::PostMessage($window,0x100,[IntPtr]0x20,[IntPtr]0)
            $null=[KovaMeasurementWindow]::PostMessage($window,0x101,[IntPtr]0x20,[IntPtr]0)
            $null=[KovaMeasurementWindow]::ShowWindow($window,6)
        }
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        $cpu=$process.TotalProcessorTime.TotalMilliseconds
        $watch=[Diagnostics.Stopwatch]::StartNew()
        Start-Sleep -Seconds $SecondsPerPhase
        $process.Refresh()
        $elapsed=$watch.Elapsed.TotalMilliseconds
        $used=$process.TotalProcessorTime.TotalMilliseconds-$cpu
        $rows += [pscustomobject]@{
            phase=$phase; elapsed_ms=[math]::Round($elapsed,3); cpu_ms=$used
            one_core_percent=[math]::Round(100*$used/$elapsed,3)
            private_bytes=$process.PrivateMemorySize64
            working_set_bytes=$process.WorkingSet64
            peak_working_set_bytes=$process.PeakWorkingSet64
        }
    }
} finally {
    if (!$process.HasExited) {
        $null=$process.CloseMainWindow()
        if (!$process.WaitForExit(3000)) { Stop-Process -Id $process.Id }
    }
}
$rows | Export-Csv -NoTypeInformation -LiteralPath (Join-Path $output 'animation.csv')
$rows | Format-Table
