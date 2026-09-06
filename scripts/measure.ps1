#Requires -Version 5.1
param(
    [Parameter(Mandatory=$true)][string]$Image,
    [int]$Runs = 5,
    [string]$OutputDirectory = "artifacts/measurements",
    [switch]$Software
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target/release/kova-image.exe"
if (!(Test-Path -LiteralPath $exe)) { throw "Build the release executable first." }
$imagePath = (Resolve-Path -LiteralPath $Image).Path
$null = New-Item -ItemType Directory -Force -Path $OutputDirectory
$outputRoot = (Resolve-Path -LiteralPath $OutputDirectory).Path
if ($Runs -lt 1 -or $Runs -gt 100) { throw "Runs must be between 1 and 100." }
$results = @()
for ($i = 0; $i -lt $Runs; $i++) {
    $measurement = Join-Path $outputRoot ("render-" + [guid]::NewGuid().ToString('N') + ".json")
    $arguments = @('--measure', ('"' + $measurement + '"'))
    if ($Software) { $arguments += '--software' }
    $arguments += @('--', ('"' + $imagePath + '"'))
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        while (!(Test-Path -LiteralPath $measurement)) {
            if ($process.HasExited) { throw "Viewer exited before producing a measurement." }
            if ($watch.Elapsed.TotalSeconds -gt 30) { throw "First-render measurement timed out." }
            Start-Sleep -Milliseconds 20
        }
        $process.Refresh()
        $data = Get-Content -LiteralPath $measurement -Raw | ConvertFrom-Json
        $results += [pscustomobject]@{
            run = $i + 1
            first_render_ms = $data.first_render_ms
            launch_to_observation_ms = [math]::Round($watch.Elapsed.TotalMilliseconds, 3)
            peak_working_set_bytes = $process.PeakWorkingSet64
            private_memory_bytes = $process.PrivateMemorySize64
            software = [bool]$Software
        }
    } finally {
        if (!$process.HasExited) {
            $null = $process.CloseMainWindow()
            if (!$process.WaitForExit(3000)) { Stop-Process -Id $process.Id }
        }
    }
}
$results | Export-Csv -NoTypeInformation -LiteralPath (Join-Path $outputRoot "startup.csv")
$results | Format-Table
