#Requires -Version 5.1
<#
.SYNOPSIS
    Builds the per-user Windows installer for Kova Image.

.DESCRIPTION
    Runs the portable packaging script first, then compiles
    packaging\kova-image.iss from that staging folder with Inno Setup. The
    portable script fails closed when a dependency license text is missing, so
    an installer can only be produced from a payload that passed that check.

    Reuse an existing staging folder with -SkipPackage, for example to compile
    the installer again after editing only the Inno Setup script.

.EXAMPLE
    .\scripts\installer.ps1
    .\scripts\installer.ps1 -SkipPackage
#>
param(
    [string]$Destination = "dist",
    [switch]$SkipPackage
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Find-InnoCompiler {
    # Inno Setup installs per user or per machine depending on how it was
    # obtained, so probe the registered install location before the defaults.
    $keys = @(
        "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1",
        "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1",
        "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1"
    )
    foreach ($key in $keys) {
        $location = (Get-ItemProperty -Path $key -ErrorAction SilentlyContinue).InstallLocation
        if ($location) {
            $candidate = Join-Path $location "ISCC.exe"
            if (Test-Path -LiteralPath $candidate) { return $candidate }
        }
    }
    $fallbacks = @(
        (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe")
    )
    foreach ($candidate in $fallbacks) {
        if ($candidate -and (Test-Path -LiteralPath $candidate)) { return $candidate }
    }
    $onPath = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    throw "ISCC.exe not found. Install Inno Setup 6 (winget install JRSoftware.InnoSetup)."
}

if (-not $SkipPackage) {
    & "$PSScriptRoot/package.ps1" -Destination $Destination
}

$destinationRoot = (Resolve-Path -LiteralPath $Destination).Path
$stage = Get-ChildItem -LiteralPath $destinationRoot -Directory |
    Where-Object { $_.Name -like "Kova-Image-*-x64-*" } |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $stage) { throw "No portable staging folder under $destinationRoot. Run scripts\package.ps1 first." }

$executable = Join-Path $stage.FullName "kova-image.exe"
if (-not (Test-Path -LiteralPath $executable)) { throw "$($stage.FullName) has no kova-image.exe." }

# The staging folder name carries the crate version the payload was built from.
if ($stage.Name -notmatch "^Kova-Image-(?<version>[0-9]+\.[0-9]+\.[0-9]+)-x64-") {
    throw "Cannot read a version from $($stage.Name)."
}
$version = $matches['version']

$iscc = Find-InnoCompiler
Write-Host "Inno Setup compiler: $iscc" -ForegroundColor Cyan
Write-Host "Payload: $($stage.FullName)" -ForegroundColor Cyan

& $iscc "/DStageDir=$($stage.FullName)" "/DAppVersion=$version" "/O$destinationRoot" (Join-Path $root "packaging\kova-image.iss")
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed." }

$setup = Join-Path $destinationRoot ("Kova-Image-" + $version + "-x64-setup.exe")
if (-not (Test-Path -LiteralPath $setup)) { throw "The compiler reported success but $setup is missing." }
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $setup).Hash.ToLowerInvariant()
"$hash *$([System.IO.Path]::GetFileName($setup))" | Set-Content -Encoding ascii -LiteralPath ($setup + ".sha256")

Write-Host "Installer: $setup" -ForegroundColor Green
Write-Host "SHA-256:   $hash" -ForegroundColor Green
