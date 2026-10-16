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
# Plain $args instead of a param block: an advanced-script parameter rejects
# positional arguments under `powershell -File` and drops a literal `--`.
# From an interactive PowerShell prompt, quote it: ... test '--' --nocapture
# The older `-CargoArgs @('build', ...)` form still works: arrays are
# flattened and the leading switch name is dropped.
$CargoArgs = @($args | ForEach-Object { $_ })
if ($CargoArgs.Count -gt 0 -and $CargoArgs[0] -eq "-CargoArgs") {
    $CargoArgs = @($CargoArgs | Select-Object -Skip 1)
}
if ($CargoArgs.Count -eq 0) {
    throw "Usage: cargo-msvc.ps1 <cargo arguments>, for example: cargo-msvc.ps1 test --locked"
}

. "$PSScriptRoot/msvc-env.ps1"

# If the user typed `cargo-msvc.ps1 cargo test ...`, drop the leading "cargo".
if ($CargoArgs[0] -eq "cargo") {
    $CargoArgs = $CargoArgs[1..($CargoArgs.Length - 1)]
}

$ErrorActionPreference = "Continue"
& cargo @CargoArgs 2>&1 | ForEach-Object { $_.ToString() }
exit $LASTEXITCODE
