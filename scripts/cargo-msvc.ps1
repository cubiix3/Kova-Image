#Requires -Version 5.1
<#
.SYNOPSIS
    Helper that runs a cargo command inside a Visual Studio x64 dev shell.

.DESCRIPTION
    Kova links against the Windows C++ runtime, so cargo needs the LIB/PATH
    environment set by vcvars64.bat. This script discovers a Visual Studio
    install that carries the x64 C++ toolset - including the standalone Build
    Tools - and runs the requested cargo command with the environment
    initialized.

    It avoids calling the system `cmd` command because some environments have a
    Node wrapper at `cmd` that breaks argument parsing.

.EXAMPLE
    .\scripts\cargo-msvc.ps1 test --workspace
    .\scripts\cargo-msvc.ps1 build --release
#>
param(
    [Parameter(Mandatory = $true, ValueFromRemainingArguments = $true)]
    [string[]]$CargoArgs
)

$ErrorActionPreference = "Stop"

function Find-VsVarsBatch {
    # vswhere ships with the installer and is the supported way to locate any
    # instance, whichever edition, channel and directory layout it uses. Ask
    # for the x64 C++ toolset so an install without it is skipped rather than
    # reported as a working build environment.
    $installerRoot = ${env:ProgramFiles(x86)}
    if ($installerRoot) {
        $vswhere = Join-Path $installerRoot "Microsoft Visual Studio\Installer\vswhere.exe"
        if (Test-Path -LiteralPath $vswhere) {
            $installations = & $vswhere -latest -prerelease -products * `
                -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
                -property installationPath
            foreach ($installation in @($installations)) {
                if (-not $installation) { continue }
                $candidate = Join-Path $installation "VC\Auxiliary\Build\vcvars64.bat"
                if (Test-Path -LiteralPath $candidate) {
                    return $candidate
                }
            }
        }
    }

    # Fallback for installs whose vswhere is missing. Visual Studio changed its
    # installation layout over time: classic releases live under a four-digit
    # year folder ("2022"), newer ones under a version-number folder ("18"), and
    # the Build Tools land in the 32-bit Program Files root. Probe every
    # installed root and edition instead of hard-coding a single layout.
    $roots = @()
    foreach ($programFiles in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if (-not $programFiles) { continue }
        $vsDir = Join-Path $programFiles "Microsoft Visual Studio"
        if (Test-Path -LiteralPath $vsDir) {
            $roots += Get-ChildItem -LiteralPath $vsDir -Directory |
                Sort-Object Name -Descending |
                ForEach-Object { $_.FullName }
        }
    }
    foreach ($root in $roots) {
        foreach ($edition in @("Community", "Professional", "Enterprise", "BuildTools", "Preview")) {
            $candidate = Join-Path $root "$edition\VC\Auxiliary\Build\vcvars64.bat"
            if (Test-Path -LiteralPath $candidate) {
                return $candidate
            }
        }
    }
    throw "vcvars64.bat not found. Install Visual Studio or the Visual Studio Build Tools with the Desktop development with C++ workload."
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$vcvars = Find-VsVarsBatch

# Capture environment *before* running vcvars so we can diff it afterwards.
$before = @{}
foreach ($var in [Environment]::GetEnvironmentVariables("Process").Keys) {
    $before[$var] = [Environment]::GetEnvironmentVariable($var, "Process")
}

# Use the legacy Windows command processor directly via its absolute path.
$cmdExe = "$env:SystemRoot\system32\cmd.exe"
$envDump = & $cmdExe /c """$vcvars"" 1>nul 2>nul & set" 2>$null
$after = @{}
foreach ($line in $envDump) {
    if ($line -match "^(\w+)=(.*)$") {
        $after[$matches[1]] = $matches[2]
    }
}

# Apply all new or changed variables to the current PowerShell process.
foreach ($key in $after.Keys) {
    if ($before[$key] -ne $after[$key]) {
        [Environment]::SetEnvironmentVariable($key, $after[$key], "Process")
    }
}

Write-Host "Visual Studio x64 environment loaded from: $vcvars" -ForegroundColor Cyan

# If the user typed `cargo-msvc.ps1 cargo test ...`, drop the leading "cargo".
if ($CargoArgs[0] -eq "cargo") {
    $CargoArgs = $CargoArgs[1..($CargoArgs.Length - 1)]
}

$ErrorActionPreference = "Continue"
& cargo @CargoArgs 2>&1 | ForEach-Object { $_.ToString() }
exit $LASTEXITCODE
